//! Input state machine, hit-test routing, rollback and debug snapshot
//! (W2-13/S8). Owns the per-batch pointer pipeline (`apply_input_event` and the
//! object-path `apply_object_pointer_event`), the public `inputBatch`/`hitTest`/
//! tool + multi-select setters, the overlay/debug queries, and the batch
//! rollback. `target_arch = "wasm32"` gated.

use shape_renderer_core::hit_test_object::HoverAffordance;
use shape_renderer_core::model::{
    ActiveTool, CameraState, CanvasInputEvent, RenderScenePatch, SceneSelection, WorldPoint,
};
use crate::serde_wasm;
use shape_renderer_core::stats::{
    CoreHitResult, CoreInputBatchResult, CoreMarqueeResult, CoreNearestOutlinePoint,
    CoreOverlayRequest, CoreOverlayTarget, WebGpuDebugSnapshot,
};
use wasm_bindgen::prelude::*;

use super::*;

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl ShapeWebGpuRenderer {
    #[wasm_bindgen(js_name = inputBatch)]
    pub fn input_batch(&mut self, events_json: &str) -> Result<JsValue, JsValue> {
        let events = serde_json::from_str::<Vec<CanvasInputEvent>>(events_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid input batch: {error}")))?;
        let mut patches = Vec::new();
        let mut hit = None;
        let mut overlay = None;
        let mut marquee = None;
        let mut object_out = ObjectInputOut::default();
        let rollback = self.rollback_state();
        for event in events {
            if let Err(error) = self.apply_input_event(
                event,
                &mut patches,
                &mut hit,
                &mut overlay,
                &mut marquee,
                &mut object_out,
            ) {
                self.restore_rollback_state(rollback);
                return Err(error);
            }
        }
        self.write_uniform();
        let selection = self
            .scene
            .as_ref()
            .map(|scene| scene.selection.clone())
            .unwrap_or_default();
        serde_wasm(CoreInputBatchResult {
            camera: self.camera.clone(),
            hit,
            selection,
            patches,
            overlay,
            marquee,
            object_selection: object_out.selection,
            object_transform_delta: object_out.transform_delta,
            object_endpoint_delta: object_out.endpoint_delta,
            object_marquee_ids: object_out.marquee_ids,
            object_double_click: object_out.double_click,
            hover_affordance: object_out
                .hover_affordance
                .unwrap_or(HoverAffordance::Empty)
                .as_str()
                .to_string(),
        })
    }

    #[wasm_bindgen(js_name = overlayRequest)]
    pub fn overlay_request(&self, card_id: &str, field: &str) -> Result<JsValue, JsValue> {
        serde_wasm(self.overlay_request_for_card(card_id, field))
    }

    /// Set the active pointer tool ("select" | "hand"). Equivalent to a
    /// `set-tool` inputBatch event but callable as a one-off (tool toggles in the
    /// shell rarely coincide with a pointer batch). Unknown values are ignored.
    #[wasm_bindgen(js_name = setTool)]
    pub fn set_tool(&mut self, tool: &str) {
        match tool {
            "select" => self.active_tool = ActiveTool::Select,
            "hand" => {
                self.active_tool = ActiveTool::Hand;
                self.input_drag = None;
            }
            _ => {}
        }
    }

    /// Replace the transient multi-select highlight set with `ids` (JSON array of
    /// strings); an empty array clears it. Equivalent to a `set-multi-select`
    /// inputBatch event but callable as a one-off, mirroring `setTool`. The
    /// persisted single-anchor selection is untouched.
    #[wasm_bindgen(js_name = setMultiSelect)]
    pub fn set_multi_select(&mut self, ids_json: &str) -> Result<(), JsValue> {
        let ids = serde_json::from_str::<Vec<String>>(ids_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid multi-select ids: {error}")))?;
        self.set_multi_select_ids(ids);
        Ok(())
    }

    /// W3-G5/#5: flip the live object renderer's light/dark theme bit so the shell's
    /// dark-mode toggle actually reaches the canvas. Forwards to the live
    /// [`ObjectRenderer::set_theme`], which re-resolves the canvas clear color +
    /// token-backed shadow/fill/stroke instance colors against the queue this struct
    /// owns — a zero-rebake uniform/color refresh, no re-tessellation. The next
    /// `renderFrame` (the shell's render loop) presents with the new theme: the clear
    /// color tracks the bit and the rewritten instance colors are already on the GPU.
    /// No-op when no object scene/renderer is live (matches the optional `?.` call in
    /// the shell).
    #[wasm_bindgen(js_name = setObjectTheme)]
    pub fn set_object_theme(&mut self, dark: bool) {
        // W3-G6/#3: persist the bit FIRST so it survives a `load_object_scene`
        // re-feed even if no renderer is live yet — the rebuilt renderer reads it.
        self.object_theme = shape_renderer_core::object_theme::Theme { dark };
        if let Some(renderer) = self.object_renderer.as_mut() {
            renderer.set_theme(&self.queue, dark);
        }
    }


    /// Hit-test a screen-space point without mutating selection or camera (CC4.1).
    /// Returns the picked object (or null) so the shell can show a right-click
    /// context menu. Mirrors `hitTest` in coreContract.ts.
    #[wasm_bindgen(js_name = hitTest)]
    pub fn hit_test(&self, screen_x: f64, screen_y: f64) -> Result<JsValue, JsValue> {
        let hit = self.hit_at_screen(WorldPoint {
            x: screen_x,
            y: screen_y,
        });
        serde_wasm(hit)
    }

    /// FC-08: pure object pick for the right-click context menu. Returns the id of
    /// the top-most object under the screen point (or null) without mutating
    /// selection, camera, or drag state.
    #[wasm_bindgen(js_name = hitTestObject)]
    pub fn hit_test_object_at(&self, screen_x: f64, screen_y: f64) -> Option<String> {
        hit_object_in_regions(
            &self.object_regions,
            &self.camera,
            WorldPoint {
                x: screen_x,
                y: screen_y,
            },
        )
    }

    /// RA3 swept erase: every object crossed by the eraser between two consecutive
    /// SCREEN samples `(prev, curr)` — not just the top-most object at each sample —
    /// so a fast drag that skips between samples still erases everything the segment
    /// passes through. Pure pick: no mutation of selection, camera, or drag state.
    /// Returns the crossed object ids (top-down order) as a JSON array; EN1 (the
    /// shell eraser) authors the delete ops from the returned ids.
    #[wasm_bindgen(js_name = sweptEraseAt)]
    pub fn swept_erase_at(
        &self,
        prev_x: f64,
        prev_y: f64,
        curr_x: f64,
        curr_y: f64,
    ) -> Result<JsValue, JsValue> {
        let ids = swept_erase_in_regions(
            &self.object_regions,
            &self.camera,
            WorldPoint {
                x: prev_x,
                y: prev_y,
            },
            WorldPoint {
                x: curr_x,
                y: curr_y,
            },
        );
        serde_wasm(ids)
    }

    /// W2-06: nearest point on any object outline to a WORLD query point, for shape
    /// drag-create anchor snapping (W2-07).
    ///
    /// COORD SPACE: `world_x`/`world_y` are WORLD coordinates (NOT screen) — W2-07
    /// already has the world point under the cursor. `tol_px` is a screen-pixel
    /// tolerance radius, converted to world via `tol_px / zoom.max(0.025)` (the same
    /// zoom floor `screen_to_world` uses). `exclude_ids_json` is a JSON array of
    /// region ids to skip (W3-G6 #6: the transient create-preview / snap-indicator,
    /// which ride the same feed and would otherwise self-snap under the cursor).
    /// Returns `{ snapped, x, y, targetId }`: on a hit, `snapped = true` with the
    /// nearest WORLD point and the object id; otherwise `snapped = false`,
    /// `x = y = 0`, `targetId = null`.
    #[wasm_bindgen(js_name = nearestOutlinePoint)]
    pub fn nearest_outline_point(
        &self,
        world_x: f64,
        world_y: f64,
        tol_px: f64,
        zoom: f64,
        exclude_ids_json: &str,
    ) -> Result<JsValue, JsValue> {
        let tol_world = tol_px / zoom.max(0.025);
        let exclude = serde_json::from_str::<Vec<String>>(exclude_ids_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid exclude ids: {error}")))?;
        let exclude_refs: Vec<&str> = exclude.iter().map(String::as_str).collect();
        let result = nearest_outline_point(
            &self.object_regions,
            WorldPoint {
                x: world_x,
                y: world_y,
            },
            tol_world,
            &exclude_refs,
        );
        let payload = match result {
            Some((id, x, y)) => CoreNearestOutlinePoint {
                snapped: true,
                x,
                y,
                target_id: Some(id),
            },
            None => CoreNearestOutlinePoint {
                snapped: false,
                x: 0.0,
                y: 0.0,
                target_id: None,
            },
        };
        serde_wasm(payload)
    }

    #[wasm_bindgen(js_name = debugSnapshot)]
    pub fn debug_snapshot(&self) -> Result<JsValue, JsValue> {
        let (selection, selection_world_rect, total_groups, total_cards, total_edges) =
            if let Some(scene) = &self.scene {
                (
                    scene.selection.clone(),
                    selection_world_rect(scene, &scene.selection),
                    scene.groups.len(),
                    scene.cards.len(),
                    scene.edges.len(),
                )
            } else {
                (SceneSelection::Canvas, None, 0, 0, 0)
            };
        let selection_screen_rect = selection_world_rect
            .as_ref()
            .map(|rect| world_rect_to_screen_rect(rect, &self.camera));
        serde_wasm(WebGpuDebugSnapshot {
            camera: self.camera.clone(),
            selection,
            selection_world_rect,
            selection_screen_rect,
            last_hit: self.last_hit.clone(),
            total_groups,
            total_cards,
            total_edges,
            patch_update_count: self.patch_update_count,
            dirty_range_write_count: self.dirty_range_write_count,
            full_buffer_rebuild_count: self.full_buffer_rebuild_count,
        })
    }
}

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
impl ShapeWebGpuRenderer {
    pub(crate) fn rollback_state(&self) -> RendererRollbackState {
        RendererRollbackState {
            scene: self.scene.clone(),
            camera: self.camera.clone(),
            input_drag: self.input_drag.clone(),
            active_tool: self.active_tool,
            multi_select: self.multi_select.clone(),
            last_hit: self.last_hit.clone(),
            text_layout_cache: self.text_layout_cache.clone(),
            counters: self.mutation_counters(),
            object_scene: self.object_scene.clone(),
        }
    }

    pub(crate) fn restore_rollback_state(&mut self, state: RendererRollbackState) {
        self.scene = state.scene;
        self.camera = state.camera;
        self.input_drag = state.input_drag;
        self.active_tool = state.active_tool;
        self.multi_select = state.multi_select;
        self.last_hit = state.last_hit;
        self.object_scene = state.object_scene;
        // W3-G9/#5: the bindings graph is derived from `object_scene`; rebuild it from
        // the restored scene so a rolled-back input batch leaves a consistent graph.
        self.object_bindings = match &self.object_scene {
            Some(scene) => shape_scene_core::object::move_together::BindingGraph::build(
                &super::scene_feed::binding_nodes(scene),
            ),
            None => shape_scene_core::object::move_together::BindingGraph::default(),
        };
        self.text_layout_cache = state.text_layout_cache.clone();
        self.rebuild_vertex_buffer();
        self.text_layout_cache = state.text_layout_cache;
        self.restore_mutation_counters(state.counters);
        self.write_uniform();
    }

    fn mutation_counters(&self) -> MutationCounters {
        MutationCounters {
            patch_update_count: self.patch_update_count,
            dirty_range_write_count: self.dirty_range_write_count,
            full_buffer_rebuild_count: self.full_buffer_rebuild_count,
            vertex_truncation_count: self.vertex_truncation_count,
            truncated_vertex_count: self.truncated_vertex_count,
            edge_capacity_grow_count: self.edge_capacity_grow_count,
            edge_compaction_count: self.edge_compaction_count,
            card_capacity_grow_count: self.card_capacity_grow_count,
            card_compaction_count: self.card_compaction_count,
            group_capacity_grow_count: self.group_capacity_grow_count,
            group_compaction_count: self.group_compaction_count,
        }
    }

    fn restore_mutation_counters(&mut self, counters: MutationCounters) {
        self.patch_update_count = counters.patch_update_count;
        self.dirty_range_write_count = counters.dirty_range_write_count;
        self.full_buffer_rebuild_count = counters.full_buffer_rebuild_count;
        self.vertex_truncation_count = counters.vertex_truncation_count;
        self.truncated_vertex_count = counters.truncated_vertex_count;
        self.edge_capacity_grow_count = counters.edge_capacity_grow_count;
        self.edge_compaction_count = counters.edge_compaction_count;
        self.card_capacity_grow_count = counters.card_capacity_grow_count;
        self.card_compaction_count = counters.card_compaction_count;
        self.group_capacity_grow_count = counters.group_capacity_grow_count;
        self.group_compaction_count = counters.group_compaction_count;
    }


    // Store the transient multi-select set (renderer-held so it survives reloads),
    // mirror it into the scene the draw path reads, and rebuild geometry so every
    // member draws with selection styling. Multi-select changes are discrete user
    // actions (marquee completion, shift-click), so a full rebuild — the same
    // fallback the patch path uses — is acceptable and avoids per-kind slot
    // bookkeeping for a kind-agnostic id set.
    fn set_multi_select_ids(&mut self, ids: Vec<String>) {
        if self.multi_select == ids {
            return;
        }
        self.multi_select = ids;
        let multi_select = self.multi_select.clone();
        let Some(scene) = &mut self.scene else {
            return;
        };
        scene.multi_select = multi_select;
        self.rebuild_vertex_buffer();
    }


    fn apply_input_event(
        &mut self,
        event: CanvasInputEvent,
        patches: &mut Vec<RenderScenePatch>,
        hit: &mut Option<CoreHitResult>,
        overlay: &mut Option<CoreOverlayRequest>,
        marquee: &mut Option<CoreMarqueeResult>,
        object_out: &mut ObjectInputOut,
    ) -> Result<(), JsValue> {
        // FC-07: when an object scene is loaded, pointer events hit-test / drag /
        // marquee against OBJECTS. Non-pointer events (camera, fit-scene, tool) fall
        // through to the shared handlers below so pan/zoom/fit still work.
        if self.object_scene.is_some() {
            match &event {
                CanvasInputEvent::PointerDown { .. }
                | CanvasInputEvent::PointerMove { .. }
                | CanvasInputEvent::PointerUp { .. }
                | CanvasInputEvent::PointerCancel { .. }
                | CanvasInputEvent::DoubleClick { .. } => {
                    return self.apply_object_pointer_event(event, object_out);
                }
                _ => {}
            }
        }
        match event {
            CanvasInputEvent::PointerDown { pointer_id, screen } => {
                // Hand tool always pans; it never hit-tests or mutates selection.
                if self.active_tool == ActiveTool::Hand {
                    self.input_drag = Some(InputDragState::Pan {
                        pointer_id,
                        start: screen,
                        camera: self.camera.clone(),
                    });
                    return Ok(());
                }
                let next_hit = self.hit_at_screen(screen);
                self.last_hit = next_hit.clone();
                *hit = next_hit.clone();
                // Empty hit under the Select tool starts a marquee instead of
                // clearing selection. The shell decides whether/how to clear its
                // own selection from the resulting marquee ids (C1).
                let Some(hit_object) = next_hit.clone() else {
                    let world = screen_to_world(screen, &self.camera);
                    self.input_drag = Some(InputDragState::Marquee {
                        pointer_id,
                        start: world,
                        current: world,
                    });
                    return Ok(());
                };
                self.push_input_patch(
                    RenderScenePatch::Select {
                        selection: selection_from_hit(Some(&hit_object)),
                    },
                    patches,
                )?;
                self.input_drag = match &hit_object {
                    hit if hit.kind == "port" && hit.port.as_deref() == Some("source") => {
                        Some(InputDragState::Edge {
                            pointer_id,
                            source_id: hit.id.clone(),
                        })
                    }
                    hit if hit.kind == "card" || hit.kind == "text" => self
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.cards.iter().find(|card| card.id == hit.id))
                        .map(|card| InputDragState::Card {
                            pointer_id,
                            card_id: card.id.clone(),
                            start: WorldPoint {
                                x: hit.world_x,
                                y: hit.world_y,
                            },
                            start_bounds: card.bounds.clone(),
                        }),
                    hit if hit.kind == "group" => Some(InputDragState::Group {
                        pointer_id,
                        group_id: hit.id.clone(),
                        start: WorldPoint {
                            x: hit.world_x,
                            y: hit.world_y,
                        },
                    }),
                    _ => Some(InputDragState::Pan {
                        pointer_id,
                        start: screen,
                        camera: self.camera.clone(),
                    }),
                };
            }
            CanvasInputEvent::PointerMove { pointer_id, screen } => {
                let Some(drag) = self.input_drag.clone() else {
                    return Ok(());
                };
                match drag {
                    InputDragState::Pan {
                        pointer_id: drag_pointer_id,
                        start,
                        camera,
                    } if drag_pointer_id == pointer_id => {
                        self.camera = CameraState {
                            x: camera.x + screen.x - start.x,
                            y: camera.y + screen.y - start.y,
                            zoom: camera.zoom,
                        };
                    }
                    InputDragState::Group {
                        pointer_id: drag_pointer_id,
                        group_id,
                        start,
                    } if drag_pointer_id == pointer_id => {
                        let world = screen_to_world(screen, &self.camera);
                        self.input_drag = Some(InputDragState::Group {
                            pointer_id,
                            group_id: group_id.clone(),
                            start: world,
                        });
                        self.push_input_patch(
                            RenderScenePatch::MoveGroup {
                                id: group_id,
                                delta: WorldPoint {
                                    x: world.x - start.x,
                                    y: world.y - start.y,
                                },
                            },
                            patches,
                        )?;
                    }
                    InputDragState::Card {
                        pointer_id: drag_pointer_id,
                        card_id,
                        start,
                        start_bounds,
                    } if drag_pointer_id == pointer_id => {
                        let world = screen_to_world(screen, &self.camera);
                        self.push_input_patch(
                            RenderScenePatch::MoveCard {
                                id: card_id,
                                position: WorldPoint {
                                    x: start_bounds.x + world.x - start.x,
                                    y: start_bounds.y + world.y - start.y,
                                },
                            },
                            patches,
                        )?;
                    }
                    InputDragState::Edge {
                        pointer_id: drag_pointer_id,
                        ..
                    } if drag_pointer_id == pointer_id => {}
                    InputDragState::Marquee {
                        pointer_id: drag_pointer_id,
                        start,
                        ..
                    } if drag_pointer_id == pointer_id => {
                        self.input_drag = Some(InputDragState::Marquee {
                            pointer_id,
                            start,
                            current: screen_to_world(screen, &self.camera),
                        });
                    }
                    _ => {}
                }
            }
            CanvasInputEvent::PointerUp {
                pointer_id,
                screen,
                edge_id,
            } => {
                if let Some(InputDragState::Edge {
                    pointer_id: drag_pointer_id,
                    source_id,
                }) = self.input_drag.clone()
                {
                    if drag_pointer_id == pointer_id {
                        let next_hit = self.hit_at_screen(screen);
                        self.last_hit = next_hit.clone();
                        *hit = next_hit.clone();
                        if let Some(target_hit) = next_hit {
                            if target_hit.kind == "port"
                                && target_hit.port.as_deref() == Some("target")
                                && target_hit.id != source_id
                            {
                                let patch = self.scene.as_ref().and_then(|scene| {
                                    let source =
                                        scene.cards.iter().find(|card| card.id == source_id)?;
                                    let target =
                                        scene.cards.iter().find(|card| card.id == target_hit.id)?;
                                    Some(RenderScenePatch::CreateEdge {
                                        group_id: source.group_id.clone(),
                                        source: source.id.clone(),
                                        target: target.id.clone(),
                                        edge_id: edge_id.unwrap_or_else(|| {
                                            format!(
                                                "renderer-edge-{}",
                                                self.patch_update_count.saturating_add(1)
                                            )
                                        }),
                                        label: None,
                                    })
                                });
                                if let Some(patch) = patch {
                                    self.push_input_patch(patch, patches)?;
                                }
                            }
                        }
                    }
                } else if let Some(InputDragState::Marquee {
                    pointer_id: drag_pointer_id,
                    start,
                    ..
                }) = self.input_drag.clone()
                {
                    if drag_pointer_id == pointer_id {
                        let current = screen_to_world(screen, &self.camera);
                        let rect = marquee_rect(start, current);
                        let ids = self
                            .scene
                            .as_ref()
                            .map(|scene| marquee_intersecting_ids(scene, &rect))
                            .unwrap_or_default();
                        *marquee = Some(CoreMarqueeResult { rect, ids });
                    }
                }
                self.input_drag = None;
            }
            CanvasInputEvent::PointerCancel { pointer_id } => {
                if drag_pointer_id(self.input_drag.as_ref()) == Some(pointer_id) {
                    self.input_drag = None;
                }
            }
            CanvasInputEvent::Wheel { screen, delta_y } => {
                self.camera = zoom_camera_at_screen(&self.camera, screen, delta_y);
            }
            CanvasInputEvent::DoubleClick { screen } => {
                let next_hit = self.hit_at_screen(screen);
                self.last_hit = next_hit.clone();
                *hit = next_hit.clone();
                if let Some(hit) = next_hit {
                    if hit.kind == "text" {
                        if let Some(field) = hit.field.as_deref() {
                            *overlay = self.overlay_request_for_card(&hit.id, field);
                        }
                    }
                }
            }
            CanvasInputEvent::FitScene => {
                // FC-09: with an object scene loaded, frame the world-space AABB over
                // all object regions; otherwise fall back to the legacy scene fit.
                if self.object_scene.is_some() {
                    if let Some(bounds) = object_regions_world_bounds(&self.object_regions) {
                        self.camera = fit_camera_to_bounds(&bounds, self.width, self.height);
                    }
                } else if let Some(scene) = &self.scene {
                    self.camera = fit_camera_to_scene(scene, self.width, self.height);
                }
            }
            CanvasInputEvent::FocusBounds {
                bounds,
                screen,
                zoom,
                padding,
                min_zoom,
                max_zoom,
            } => {
                self.camera = focus_camera_to_bounds(
                    &bounds,
                    screen,
                    zoom,
                    padding,
                    min_zoom,
                    max_zoom,
                    self.width,
                    self.height,
                );
            }
            CanvasInputEvent::SetCamera { camera } => {
                self.camera = clamp_camera(camera);
            }
            CanvasInputEvent::SetTool { tool } => {
                self.active_tool = tool;
                // Switching tools mid-gesture abandons any in-flight drag so the
                // new tool starts from a clean pointer state.
                self.input_drag = None;
            }
            CanvasInputEvent::SetMultiSelect { ids } => {
                self.set_multi_select_ids(ids);
            }
            CanvasInputEvent::ContextPick { screen } => {
                // Right-click pick: report the hit without mutating selection or
                // starting a drag, so the shell can open a context menu (CC4.1).
                let next_hit = self.hit_at_screen(screen);
                self.last_hit = next_hit.clone();
                *hit = next_hit;
            }
        }
        Ok(())
    }

    /// FC-07: pointer input against the loaded object scene. Delegates the whole
    /// state machine to the pure [`step_object_pointer`] so it is unit-testable
    /// without a GPU device, then mirrors the resulting selection onto the scene.
    fn apply_object_pointer_event(
        &mut self,
        event: CanvasInputEvent,
        object_out: &mut ObjectInputOut,
    ) -> Result<(), JsValue> {
        // RA2b (D6): a double-click that hits an object is reported as a branched
        // signal — the shell drills in on a container, enters text edit on a leaf.
        // It mutates neither selection nor drag, so it short-circuits the pointer
        // state machine below.
        if let CanvasInputEvent::DoubleClick { screen } = event {
            if let Some(scene) = &self.object_scene {
                object_out.double_click =
                    object_double_click(&self.object_regions, &scene.objects, &self.camera, screen);
            }
            return Ok(());
        }
        let selection = self
            .object_scene
            .as_ref()
            .and_then(|scene| scene.selection.clone());
        step_object_pointer(
            &event,
            &self.object_regions,
            self.active_tool,
            selection.as_deref(),
            &mut self.camera,
            &mut self.input_drag,
            object_out,
        );
        // Mirror the picked selection onto the persisted single-anchor selection so
        // a later draw/debug reads it; the result already carries it for the shell.
        // An empty Select pointer-down (which starts a marquee) clears it, matching
        // the legacy clear-on-empty-click invariant.
        if let Some(id) = &object_out.selection {
            if let Some(scene) = &mut self.object_scene {
                scene.selection = Some(id.clone());
            }
        } else if self.active_tool == ActiveTool::Select
            && matches!(event, CanvasInputEvent::PointerDown { .. })
        {
            if let Some(scene) = &mut self.object_scene {
                scene.selection = None;
            }
        }
        Ok(())
    }

    fn push_input_patch(
        &mut self,
        patch: RenderScenePatch,
        patches: &mut Vec<RenderScenePatch>,
    ) -> Result<(), JsValue> {
        self.apply_render_patch(patch.clone())?;
        patches.push(patch);
        Ok(())
    }

    fn hit_at_screen(&self, screen: WorldPoint) -> Option<CoreHitResult> {
        let scene = self.scene.as_ref()?;
        hit_scene_at_screen(scene, &self.camera, screen)
    }

    fn overlay_request_for_card(&self, card_id: &str, field: &str) -> Option<CoreOverlayRequest> {
        let scene = self.scene.as_ref()?;
        let card = scene.cards.iter().find(|card| card.id == card_id)?;
        let value = match field {
            "title" => card.title.clone(),
            "summary" => card.summary.clone(),
            "detail" => card.detail.clone(),
            _ => return None,
        };
        let style = resolve_shape_style(&scene.styles, &card.style_key);
        let selected = selection_is_node(&scene.selection, &card.id);
        let text_rect = text_field_rect(&card.bounds, &style, field, selected);
        let world_rect = overlay_rect_for_text_field(&text_rect, &style, selected);
        Some(CoreOverlayRequest {
            target: CoreOverlayTarget {
                kind: "card-text".to_string(),
                id: card.id.clone(),
                field: field.to_string(),
            },
            value,
            screen_rect: world_rect_to_screen_rect(&world_rect, &self.camera),
            world_rect,
            style: overlay_style(&self.camera, &style, field, selected),
        })
    }

}
