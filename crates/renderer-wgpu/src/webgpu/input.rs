//! Input state machine, hit-test routing, rollback, and debug snapshot. Owns the
//! per-batch pointer pipeline, the public `inputBatch`/`hitTest`/tool + multi-select
//! setters, the overlay/debug queries, and the batch rollback. `target_arch =
//! "wasm32"` gated.

use shape_renderer_core::hit_test_object::HoverAffordance;
use shape_renderer_core::model::{
    ActiveTool, CameraState, CanvasInputEvent, RenderScenePatch, SceneSelection, WorldPoint,
};
use shape_renderer_core::render_object::RenderObject;
use crate::serde_wasm;
use shape_renderer_core::stats::{
    CoreHitResult, CoreInputBatchResult, CoreMarqueeResult, CoreNearestOutlinePoint,
    CoreOverlayRequest, CoreOverlayTarget, WebGpuDebugSnapshot,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use super::*;

/// The neutral key the shell forwards as JSON. Mirrors `shape_ui_core::KeyInput`;
/// `text` is the inserted printable char(s), absent for control keys; `ctrl`/`meta`/
/// `alt` carry the OS modifier state so the core can tell a shortcut chord from a
/// bare control key (a chord must fall through, not be swallowed by a focused field).
/// The modifier flags default to false so an older shell payload still deserializes.
#[cfg(feature = "wgpu-probe")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireKeyInput {
    key: String,
    text: Option<String>,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    meta: bool,
    #[serde(default)]
    alt: bool,
}

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

    /// Set the active pointer tool ("select" | "hand"), callable as a one-off
    /// (equivalent to a `set-tool` inputBatch event). Unknown values are ignored.
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

    /// Set the coarse-rotate modifier (e.g. Shift held), callable as a one-off
    /// (equivalent to a `set-coarse-rotate` inputBatch event). While active, a
    /// rotate-handle drag snaps its swept delta to fixed increments.
    #[wasm_bindgen(js_name = setCoarseRotate)]
    pub fn set_coarse_rotate(&mut self, active: bool) {
        self.coarse_rotate = active;
    }

    /// Set the active drill-in container scope (or `None` to exit), callable as a
    /// one-off (equivalent to a `set-active-container` inputBatch event). The shell
    /// forwards the canonical container id; the core resolves the scoped child pick.
    #[wasm_bindgen(js_name = setActiveContainer)]
    pub fn set_active_container(&mut self, id: Option<String>) {
        self.active_container = id;
    }

    /// Replace the transient multi-select highlight set with `ids` (an empty array
    /// clears it), callable as a one-off. The persisted single-anchor selection is
    /// untouched.
    #[wasm_bindgen(js_name = setMultiSelect)]
    pub fn set_multi_select(&mut self, ids_json: &str) -> Result<(), JsValue> {
        let ids = serde_json::from_str::<Vec<String>>(ids_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid multi-select ids: {error}")))?;
        self.set_multi_select_ids(ids);
        Ok(())
    }

    /// Flip the live object renderer's light/dark theme bit, forwarding to
    /// [`ObjectRenderer::set_theme`] (a zero-rebake color refresh). The next
    /// `renderFrame` presents with the new theme. No-op when no object renderer is
    /// live.
    #[wasm_bindgen(js_name = setObjectTheme)]
    pub fn set_object_theme(&mut self, dark: bool) {
        // Persist the bit FIRST so it survives a `load_object_scene` re-feed even if
        // no renderer is live yet.
        self.object_theme = shape_renderer_core::object_theme::Theme { dark };
        if let Some(renderer) = self.object_renderer.as_mut() {
            renderer.set_theme(&self.queue, dark);
        }
        // The screen-space UI renderer tracks the same theme bit so its Token paints
        // (surface/text/selection-ring) recolor with the canvas — zero rebake.
        if let Some(ui) = self.ui_renderer.as_mut() {
            ui.set_theme(&self.queue, dark);
        }
        // The runtime owns the UI scene: UI TEXT is a FIXED hex (not a token), so a
        // theme flip must re-resolve text via the runtime + re-feed. Fills/strokes stay
        // zero-rebake tokens (handled above); only the text scene re-feeds here.
        if self.ui_runtime.as_mut().map(|rt| rt.set_theme(dark)).unwrap_or(false) {
            self.refeed_ui_runtime();
        }
    }


    /// Hit-test a screen-space point without mutating selection or camera, returning
    /// the picked object (or null) for a right-click context menu.
    #[wasm_bindgen(js_name = hitTest)]
    pub fn hit_test(&self, screen_x: f64, screen_y: f64) -> Result<JsValue, JsValue> {
        let hit = self.hit_at_screen(WorldPoint {
            x: screen_x,
            y: screen_y,
        });
        serde_wasm(hit)
    }

    /// Project a WORLD point to SCREEN space through the LIVE core camera, so the
    /// shell never recomputes the transform from a mirrored `CameraState`. Exact
    /// inverse of [`Self::screen_to_world`] (same clamped zoom). Returns `{ x, y }`.
    #[wasm_bindgen(js_name = worldToScreen)]
    pub fn world_to_screen(&self, world_x: f64, world_y: f64) -> Result<JsValue, JsValue> {
        let screen = world_to_screen(
            WorldPoint {
                x: world_x,
                y: world_y,
            },
            &self.camera,
        );
        serde_wasm(screen)
    }

    /// Un-project a SCREEN point to WORLD space through the LIVE core camera. Exact
    /// inverse of [`Self::world_to_screen`] (same clamped zoom). Returns `{ x, y }`.
    #[wasm_bindgen(js_name = screenToWorld)]
    pub fn screen_to_world(&self, screen_x: f64, screen_y: f64) -> Result<JsValue, JsValue> {
        let world = screen_to_world(
            WorldPoint {
                x: screen_x,
                y: screen_y,
            },
            &self.camera,
        );
        serde_wasm(world)
    }

    /// The id of the top-most object under the screen point (or null), without
    /// mutating selection, camera, or drag state.
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

    /// The id of the top-most UI widget under the SCREEN point (or null). The UI scene
    /// is screen-space, so this picks against the IDENTITY camera (world == screen),
    /// NOT the live world camera. The shell forwards raw canvas-local px and consumes
    /// the core-returned id; no shell-side pick decision.
    #[wasm_bindgen(js_name = hitUi)]
    pub fn hit_ui(&self, screen_x: f64, screen_y: f64) -> Option<String> {
        hit_object_in_regions(
            &self.ui_regions,
            &UI_IDENTITY_CAMERA,
            WorldPoint {
                x: screen_x,
                y: screen_y,
            },
        )
    }

    /// Drive the ui-core runtime with a pointer phase (`"down"|"move"|"up"|"cancel"`)
    /// at SCREEN px. The runtime decides everything (hit, slider value, actuation);
    /// on `dirty` the renderer re-renders the runtime + re-feeds the UI scene through
    /// the same `feed_ui_scene` path (so `ui_regions` track the laid-out boxes), and
    /// the shell only forwards + lets the RAF redraw. Returns
    /// `{ consumed, sceneChanged, actions, edit }`. A no-op without a seeded runtime.
    #[wasm_bindgen(js_name = uiPointer)]
    pub fn ui_pointer(&mut self, phase: &str, screen_x: f64, screen_y: f64) -> Result<JsValue, JsValue> {
        let phase = match phase {
            "down" => shape_ui_core::PointerPhase::Down,
            "move" => shape_ui_core::PointerPhase::Move,
            "up" => shape_ui_core::PointerPhase::Up,
            "cancel" => shape_ui_core::PointerPhase::Cancel,
            other => return Err(JsValue::from_str(&format!("unknown UI pointer phase: {other}"))),
        };
        let Some(runtime) = self.ui_runtime.as_mut() else {
            return serde_wasm(crate::webgpu::ui::CoreUiDispatchResult::from_dispatch(
                &shape_ui_core::DispatchResult::default(),
            ));
        };
        let result = runtime.dispatch_pointer(phase, (screen_x, screen_y));
        if result.dirty {
            self.refeed_ui_runtime();
        }
        serde_wasm(self.dispatch_result(&result))
    }

    /// Forward a neutral key (`KeyInput` JSON: `{ key, text }`) to the focused UI
    /// widget. The runtime decides whether it owns the key (only when a TextInput is
    /// focused); on `dirty` the renderer re-feeds. Returns the same dispatch shape as
    /// [`Self::ui_pointer`]. A no-op without a seeded runtime.
    #[wasm_bindgen(js_name = uiKey)]
    pub fn ui_key(&mut self, key_json: &str) -> Result<JsValue, JsValue> {
        let key: WireKeyInput = serde_json::from_str(key_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid UI key: {error}")))?;
        let Some(runtime) = self.ui_runtime.as_mut() else {
            return serde_wasm(crate::webgpu::ui::CoreUiDispatchResult::from_dispatch(
                &shape_ui_core::DispatchResult::default(),
            ));
        };
        let result = runtime.dispatch_key(&shape_ui_core::KeyInput {
            key: key.key,
            text: key.text,
            ctrl: key.ctrl,
            meta: key.meta,
            alt: key.alt,
        });
        if result.dirty {
            self.refeed_ui_runtime();
        }
        serde_wasm(self.dispatch_result(&result))
    }

    /// Commit ONE finished string from the shell's IME surface into the focused UI
    /// field, blur it, and emit the final `TextChanged`. The OS surface owns the
    /// composition; this lands its single committed value (correct even when CJK
    /// composition deleted/replaced in place, where a suffix diff would not be). On
    /// `dirty` the renderer re-feeds. Same dispatch shape as [`Self::ui_key`].
    #[wasm_bindgen(js_name = uiCommitText)]
    pub fn ui_commit_text(&mut self, value: String) -> Result<JsValue, JsValue> {
        let Some(runtime) = self.ui_runtime.as_mut() else {
            return serde_wasm(crate::webgpu::ui::CoreUiDispatchResult::from_dispatch(
                &shape_ui_core::DispatchResult::default(),
            ));
        };
        let result = runtime.commit_text(value);
        if result.dirty {
            self.refeed_ui_runtime();
        }
        serde_wasm(self.dispatch_result(&result))
    }

    /// True when a ui-core widget owns text focus. The window arbiter ORs this into
    /// its `typing` predicate so a focused ui-core TextInput suppresses the catalog
    /// dispatcher exactly like a DOM input.
    #[wasm_bindgen(js_name = uiHasFocus)]
    pub fn ui_has_focus(&self) -> bool {
        self.ui_runtime.as_ref().map(|rt| rt.has_text_focus()).unwrap_or(false)
    }

    /// Seed the renderer-owned UI runtime from the ui-core demo widget tree at the
    /// current viewport + theme, then feed its first render. After this the runtime
    /// OWNS the UI scene — `uiPointer`/`uiKey` re-feed it on a state change, replacing
    /// the static `loadUiScene` feed. The widget tree still lives in `ui-core`
    /// (`demo_ui_tree`) until P4 built-in UIs replace it. Re-seedable (rebuilds the
    /// runtime, dropping prior interaction state) on a viewport change.
    #[wasm_bindgen(js_name = initUiRuntime)]
    pub fn init_ui_runtime(&mut self, viewport_w: f64, viewport_h: f64, _theme_dark: bool) {
        // Seed the runtime from the PERSISTED canvas theme, NOT a hardcoded light seed:
        // the shell applies the real theme via `set_object_theme` as soon as the host is
        // wired, and that may land before this seed. Adopting `self.object_theme` here
        // means a theme that already landed is kept, never clobbered back to light —
        // which otherwise paints light UI fills under a dark canvas until the next theme
        // change. When nothing set the theme yet, `object_theme` is still its light
        // default, so the demo seeds light exactly as before.
        let dark = self.object_theme.dark;
        let runtime =
            shape_ui_core::UiRuntime::new(crate::demo_ui_tree(), (viewport_w, viewport_h), dark);
        self.ui_runtime = Some(runtime);
        self.refeed_ui_runtime();
    }

    /// Feed the built-in UI model (toolbar + inspector) from the shell. `model_json`
    /// carries the shell-owned UI state (theme/viewport/active-tool/create-kind) and
    /// the dynamic inspector view (from scene-core's `objectInspectorView`); the
    /// command catalog is constant in-core, so it is not carried. The renderer
    /// composes `shape_ui::build_root` from it and seeds (or re-trees, preserving
    /// interaction caches so an in-progress field edit / dragged value survives the
    /// re-derive) the runtime, then re-feeds. A model change that only flips a
    /// toggle/recolors rides the existing partial-patch path, NOT a re-tessellation.
    /// After this, `uiPointer`/`uiKey` resolve fired actions into typed intents.
    #[wasm_bindgen(js_name = setUiModel)]
    pub fn set_ui_model(&mut self, model_json: &str) -> Result<(), JsValue> {
        let model: crate::webgpu::ui::UiModelInput = serde_json::from_str(model_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid UI model: {error}")))?;
        // Keep the persisted theme bit in lock-step so a later `setObjectTheme` diff
        // doesn't double-flip the freshly-fed runtime.
        self.object_theme = shape_renderer_core::object_theme::Theme { dark: model.theme_dark };
        let tree = model.build_tree();
        match self.ui_runtime.as_mut() {
            // A live runtime keeps its interaction caches across the re-derive (a
            // focused field / dragged value survives). Sync theme/viewport too.
            Some(runtime) => {
                runtime.set_tree(tree);
                runtime.set_viewport((model.viewport[0], model.viewport[1]));
                runtime.set_theme(model.theme_dark);
            }
            None => {
                self.ui_runtime = Some(shape_ui_core::UiRuntime::new(
                    tree,
                    (model.viewport[0], model.viewport[1]),
                    model.theme_dark,
                ));
            }
        }
        self.ui_model = Some(model);
        self.refeed_ui_runtime();
        Ok(())
    }

    /// Swept erase: every object crossed by the eraser between two consecutive SCREEN
    /// samples `(prev, curr)`, so a fast drag that skips between samples still erases
    /// everything the segment passes through. Pure pick; returns the crossed ids
    /// (top-down) as JSON.
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

    /// Nearest point on any object outline to a WORLD query point, for drag-create
    /// anchor snapping. `world_x`/`world_y` are WORLD coords; `tol_px` is a screen-
    /// pixel tolerance converted to world via `tol_px / zoom.max(0.025)`;
    /// `exclude_ids_json` skips transient preview regions that would self-snap.
    /// Returns `{ snapped, x, y, targetId }`.
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
            coarse_rotate: self.coarse_rotate,
            active_container: self.active_container.clone(),
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
        self.coarse_rotate = state.coarse_rotate;
        self.active_container = state.active_container;
        self.multi_select = state.multi_select;
        self.last_hit = state.last_hit;
        self.object_scene = state.object_scene;
        // The bindings graph is derived from `object_scene`; rebuild it so a
        // rolled-back batch leaves a consistent graph.
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

    /// Re-render the seeded UI runtime and re-feed its scene through the shared
    /// `feed_ui_scene` path, so a state-change dispatch refreshes the GPU geometry +
    /// `ui_regions`. Rendering returns an owned scene, so the immutable runtime borrow
    /// is dropped before the `&mut self` feed. A no-op without a seeded runtime.
    fn refeed_ui_runtime(&mut self) {
        let Some(scene) = self.ui_runtime.as_ref().map(|rt| rt.render()) else {
            return;
        };
        self.feed_ui_scene(&scene);
    }

    /// Wrap a dispatch into the wire shape, resolving fired actions to typed intents
    /// when a UI model is set (the built-in UI binding); on the demo path (no model)
    /// it forwards actions only. The pure mapping/resolution lives in `webgpu::ui`.
    fn dispatch_result(
        &self,
        result: &shape_ui_core::DispatchResult,
    ) -> crate::webgpu::ui::CoreUiDispatchResult {
        match &self.ui_model {
            Some(model) => {
                crate::webgpu::ui::CoreUiDispatchResult::from_dispatch_resolved(result, model)
            }
            None => crate::webgpu::ui::CoreUiDispatchResult::from_dispatch(result),
        }
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


    // Store the multi-select set (renderer-held so it survives reloads), mirror it
    // into the scene the draw path reads, and rebuild geometry so every member draws
    // selected. A full rebuild is acceptable for these discrete user actions and
    // avoids per-kind slot bookkeeping for a kind-agnostic id set.
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
        // When an object scene is loaded, pointer events hit-test / drag / marquee
        // against OBJECTS; non-pointer events fall through to the shared handlers.
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
                // Empty hit under Select starts a marquee instead of clearing; the
                // shell decides how to clear from the resulting marquee ids.
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
                // With an object scene, frame the world AABB over all regions;
                // otherwise fall back to the legacy scene fit.
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
            CanvasInputEvent::SetCoarseRotate { active } => {
                // Mode bit, not drag-clearing: toggling it mid-rotate engages snapping
                // on the next move without abandoning the in-flight gesture.
                self.coarse_rotate = active;
            }
            CanvasInputEvent::SetMultiSelect { ids } => {
                self.set_multi_select_ids(ids);
            }
            CanvasInputEvent::SetActiveContainer { id } => {
                self.active_container = id;
            }
            CanvasInputEvent::ContextPick { screen } => {
                // Right-click pick: report the hit without mutating selection or
                // starting a drag, so the shell can open a context menu.
                let next_hit = self.hit_at_screen(screen);
                self.last_hit = next_hit.clone();
                *hit = next_hit;
            }
        }
        Ok(())
    }

    /// Pointer input against the loaded object scene. Delegates the state machine to
    /// the pure [`step_object_pointer`] (unit-testable without a device), then mirrors
    /// the resulting selection onto the scene.
    fn apply_object_pointer_event(
        &mut self,
        event: CanvasInputEvent,
        object_out: &mut ObjectInputOut,
    ) -> Result<(), JsValue> {
        // A double-click that hits an object is a branched signal (drill-in on a
        // container, text edit on a leaf); it mutates neither selection nor drag, so
        // it short-circuits the state machine below.
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
        // Direct-field borrows of disjoint fields: the scoped pick reads
        // `object_scene.objects` + `active_container` while camera/input_drag are
        // mutated. The scope is a forwarded token the core resolves against.
        let objects: &[RenderObject] = self
            .object_scene
            .as_ref()
            .map(|scene| scene.objects.as_slice())
            .unwrap_or(&[]);
        step_object_pointer(
            &event,
            &self.object_regions,
            objects,
            self.active_tool,
            self.coarse_rotate,
            self.active_container.as_deref(),
            selection.as_deref(),
            &mut self.camera,
            &mut self.input_drag,
            object_out,
        );
        // Mirror the picked selection onto the persisted single-anchor selection. An
        // empty Select pointer-down (which starts a marquee) clears it, matching the
        // clear-on-empty-click invariant.
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
            // A scoped pick that resolved to None is a click OUTSIDE the active
            // container, so exit the drill-in scope (the core owns the exit rule).
            self.active_container = None;
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
