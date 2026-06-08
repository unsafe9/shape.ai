//! Scene load + feed + slot retention (W2-13/S8): the legacy scene path
//! (`load_scene`/`applyPatchBatch`/`apply_render_patch` with incremental
//! dirty/new/grow/compact/clear slot bookkeeping) and the object path
//! (`load_object_scene`/`draw_objects`). `target_arch = "wasm32"` gated.

use std::collections::HashMap;

use crate::model::{
    CameraState, RenderCard, RenderEdge, RenderGroup, RenderScenePatch, SceneSelection,
    SceneSnapshot,
};
use crate::object_pipeline::{ObjectPipeline, ObjectRenderer};
use crate::render_object::RenderObjectScene;
use crate::serde_wasm;
use crate::text::TextBuildStats;
use wasm_bindgen::prelude::*;

use super::*;

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl ShapeWebGpuRenderer {
    #[wasm_bindgen(js_name = loadScene)]
    pub fn load_scene(&mut self, scene_json: &str) -> Result<(), JsValue> {
        let mut scene = serde_json::from_str::<SceneSnapshot>(scene_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid scene snapshot: {error}")))?;
        // The transient multi-select set is shell-owned and not in the wire format,
        // so a reload would otherwise drop it. Re-apply the renderer-held set so the
        // highlight survives scene reloads, mirroring how `active_tool` persists.
        scene.multi_select = self.multi_select.clone();
        self.camera = CameraState {
            x: scene.camera.x,
            y: scene.camera.y,
            zoom: scene.camera.zoom,
        };
        self.vertex_count = 0;
        self.scene = Some(scene);
        self.last_hit = None;
        self.rebuild_vertex_buffer();
        self.write_uniform();
        Ok(())
    }

    #[wasm_bindgen(js_name = applyPatchBatch)]
    pub fn apply_patch_batch(&mut self, patches_json: &str) -> Result<(), JsValue> {
        let patches = serde_json::from_str::<Vec<RenderScenePatch>>(patches_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid render patch batch: {error}")))?;
        let rollback = self.rollback_state();
        for patch in patches {
            if let Err(error) = self.apply_render_patch(patch) {
                self.restore_rollback_state(rollback);
                return Err(error);
            }
        }
        Ok(())
    }


    /// OB-4 object draw entry: parse a [`RenderObjectScene`] and build + upload its
    /// object geometry (fill megabuffer + stroke ribbons + per-object instances)
    /// through an [`ObjectRenderer`] on this renderer's existing device/queue.
    /// The `ObjectPipeline` is built lazily against the live surface format on the
    /// first call. Additive: the legacy `load_scene` 2D path is untouched.
    ///
    /// Returns the built draw counts `{ objects, fillIndices, strokeVertices }` so
    /// the client can confirm the object scene reached the renderer. The live GPU
    /// PASS (recording the object render) is [`Self::draw_objects`].
    #[wasm_bindgen(js_name = loadObjectScene)]
    pub fn load_object_scene(&mut self, scene_json: &str) -> Result<JsValue, JsValue> {
        let scene: RenderObjectScene = serde_json::from_str(scene_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid object scene: {error}")))?;
        if self.object_pipeline.is_none() {
            self.object_pipeline = Some(ObjectPipeline::new(
                &self.device,
                &self.queue,
                self.config.format,
            ));
        }
        let pipeline = self.object_pipeline.as_ref().unwrap();
        let renderer = ObjectRenderer::new(
            &self.device,
            &self.queue,
            pipeline,
            &scene,
            self.width as f32,
            self.height as f32,
            // RB1: load in light mode; AP4 threads the persisted dark/light bit and
            // flips it via `ObjectRenderer::set_theme` (zero-rebake toggle).
            crate::object_theme::Theme::light(),
        );
        let counts = ObjectSceneLoadResult {
            objects: scene.objects.len(),
            fill_indices: renderer.fill_index_count() as usize,
            stroke_vertices: renderer.stroke_vertex_count() as usize,
        };
        // FC-04: derive + retain each object's local-space region for hit-test /
        // marquee, then keep the parsed scene as the live-object branch switch.
        self.object_regions = derive_object_regions(&scene);
        self.object_scene = Some(scene);
        self.object_renderer = Some(renderer);
        serde_wasm(counts)
    }

    /// OB-4 live GPU object PASS: now a thin wrapper over [`Self::render_frame`],
    /// which is the single live frame driver (FC-05). The object pass is recorded
    /// inside `render_frame` against the same acquired surface texture, so a separate
    /// acquire/present here would double-acquire the swapchain. Kept as a harmless
    /// optional wasm export; the client RAF loop calls `render_frame` directly.
    #[wasm_bindgen(js_name = drawObjects)]
    pub fn draw_objects(&mut self) -> Result<(), JsValue> {
        self.render_frame()?;
        Ok(())
    }

    /// W2-11 drag zero-rebake: push ONLY the dragged object's instance model matrix
    /// to the GPU. `matrix_json` is a row-major `[[f64;3];3]` CUMULATIVE world-space
    /// DELTA (the same contract as `ObjectTransformDelta.matrix`). The canonical
    /// `object_scene` / `object_regions` are NOT mutated — the base transform is read
    /// from the untouched scene and `delta * base` is written straight to the
    /// instance buffer (no re-tessellation, no region re-derive). The RAF
    /// `render_frame` loop then draws from the updated instance buffer on the next
    /// tick, so no explicit redraw is needed. No-op if the object scene is unloaded
    /// or the id is absent.
    #[wasm_bindgen(js_name = setObjectPreviewTransform)]
    pub fn set_object_preview_transform(
        &mut self,
        id: &str,
        matrix_json: &str,
    ) -> Result<(), JsValue> {
        let delta: [[f64; 3]; 3] = serde_json::from_str(matrix_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid preview matrix: {error}")))?;
        let Some(base) = self
            .object_scene
            .as_ref()
            .and_then(|scene| scene.objects.iter().find(|o| o.id == id))
            .map(|o| o.transform)
        else {
            return Ok(());
        };
        if let Some(renderer) = self.object_renderer.as_mut() {
            renderer.set_preview_transform(&self.queue, id, &delta, &base);
        }
        Ok(())
    }

    /// W2-11: revert the dragged object's instance matrix to its canonical baked
    /// transform (`delta = identity`), dropping the live preview. No-op if the
    /// object scene is unloaded or the id is absent.
    #[wasm_bindgen(js_name = clearObjectPreview)]
    pub fn clear_object_preview(&mut self, id: &str) -> Result<(), JsValue> {
        let Some(base) = self
            .object_scene
            .as_ref()
            .and_then(|scene| scene.objects.iter().find(|o| o.id == id))
            .map(|o| o.transform)
        else {
            return Ok(());
        };
        if let Some(renderer) = self.object_renderer.as_mut() {
            renderer.clear_preview_transform(&self.queue, id, &base);
        }
        Ok(())
    }

}

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
impl ShapeWebGpuRenderer {
    pub(crate) fn apply_render_patch(&mut self, patch: RenderScenePatch) -> Result<(), JsValue> {
        let mut dirty_card_ids = Vec::new();
        let mut dirty_edge_ids = Vec::new();
        let mut dirty_group_ids = Vec::new();
        let mut created_group_id = None;
        let mut deleted_group_id = None;
        let mut created_card_id = None;
        let mut deleted_card_id = None;
        let mut created_edge_id = None;
        let mut deleted_edge_ids = Vec::new();
        let mut deleted_card_ids = Vec::new();
        let mut rebuild_for_order = false;
        {
            let Some(scene) = &mut self.scene else {
                return Err(JsValue::from_str("No WebGPU scene loaded"));
            };
            match patch {
                RenderScenePatch::CreateGroup { group } => {
                    if group.bounds.width <= 0.0 || group.bounds.height <= 0.0 {
                        return Err(JsValue::from_str("Group bounds must be positive"));
                    }
                    if scene
                        .groups
                        .iter()
                        .any(|candidate| candidate.id == group.id)
                    {
                        return Err(JsValue::from_str(&format!(
                            "Duplicate group id: {}",
                            group.id
                        )));
                    }
                    let group_id = group.id.clone();
                    scene.groups.push(group);
                    scene.selection = SceneSelection::Group {
                        id: group_id.clone(),
                    };
                    created_group_id = Some(group_id);
                }
                RenderScenePatch::DeleteGroup { id } => {
                    let before = scene.groups.len();
                    scene.groups.retain(|group| group.id != id);
                    if scene.groups.len() == before {
                        return Err(JsValue::from_str(&format!("Unknown group id: {id}")));
                    }
                    let mut removed_card_ids = Vec::new();
                    scene.cards.retain(|card| {
                        let keep = card.group_id != id;
                        if !keep {
                            removed_card_ids.push(card.id.clone());
                        }
                        keep
                    });
                    let mut removed_edge_ids = Vec::new();
                    scene.edges.retain(|edge| {
                        let keep = edge.group_id != id
                            && !removed_card_ids
                                .iter()
                                .any(|card_id| card_id == &edge.source)
                            && !removed_card_ids
                                .iter()
                                .any(|card_id| card_id == &edge.target);
                        if !keep {
                            removed_edge_ids.push(edge.id.clone());
                        }
                        keep
                    });
                    if selection_is_group(&scene.selection, &id)
                        || removed_card_ids
                            .iter()
                            .any(|card_id| selection_is_node(&scene.selection, card_id))
                        || removed_edge_ids
                            .iter()
                            .any(|edge_id| selection_is_edge(&scene.selection, edge_id))
                    {
                        scene.selection = SceneSelection::Canvas;
                    }
                    deleted_edge_ids.extend(removed_edge_ids);
                    deleted_card_ids.extend(removed_card_ids);
                    deleted_group_id = Some(id);
                }
                RenderScenePatch::MoveGroup { id, delta } => {
                    let Some(group) = scene.groups.iter_mut().find(|group| group.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown group id: {id}")));
                    };
                    group.bounds.x += delta.x;
                    group.bounds.y += delta.y;
                    dirty_group_ids.push(id.clone());
                    let mut moved_card_ids = Vec::new();
                    for card in scene.cards.iter_mut().filter(|card| card.group_id == id) {
                        card.bounds.x += delta.x;
                        card.bounds.y += delta.y;
                        moved_card_ids.push(card.id.clone());
                    }
                    dirty_card_ids.extend(moved_card_ids.iter().cloned());
                    dirty_edge_ids.extend(
                        scene
                            .edges
                            .iter()
                            .filter(|edge| {
                                edge.group_id == id
                                    || moved_card_ids.iter().any(|card_id| {
                                        card_id == &edge.source || card_id == &edge.target
                                    })
                            })
                            .map(|edge| edge.id.clone()),
                    );
                }
                RenderScenePatch::MoveCard { id, position } => {
                    let Some(card) = scene.cards.iter_mut().find(|card| card.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    };
                    card.bounds.x = position.x;
                    card.bounds.y = position.y;
                    dirty_card_ids.push(id.clone());
                    dirty_edge_ids.extend(
                        scene
                            .edges
                            .iter()
                            .filter(|edge| edge.source == id || edge.target == id)
                            .map(|edge| edge.id.clone()),
                    );
                }
                RenderScenePatch::SetCardZIndex { id, z_index } => {
                    let Some(card) = scene.cards.iter_mut().find(|card| card.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    };
                    card.z_index = z_index;
                    scene.selection = SceneSelection::Node { id };
                    rebuild_for_order = true;
                }
                RenderScenePatch::EditCardText { id, field, value } => {
                    let Some(card) = scene.cards.iter_mut().find(|card| card.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    };
                    match field.as_str() {
                        "title" => card.title = value,
                        "summary" => card.summary = value,
                        "detail" => card.detail = value,
                        _ => {
                            return Err(JsValue::from_str(&format!(
                                "Unsupported text field: {field}"
                            )))
                        }
                    }
                    dirty_card_ids.push(id);
                }
                RenderScenePatch::CreateCard { card } => {
                    if card.bounds.width <= 0.0 || card.bounds.height <= 0.0 {
                        return Err(JsValue::from_str("Card bounds must be positive"));
                    }
                    if !scene.groups.iter().any(|group| group.id == card.group_id) {
                        return Err(JsValue::from_str(&format!(
                            "Unknown group id: {}",
                            card.group_id
                        )));
                    }
                    if scene.cards.iter().any(|candidate| candidate.id == card.id) {
                        return Err(JsValue::from_str(&format!(
                            "Duplicate card id: {}",
                            card.id
                        )));
                    }
                    let card_id = card.id.clone();
                    scene.cards.push(card);
                    scene.selection = SceneSelection::Node {
                        id: card_id.clone(),
                    };
                    created_card_id = Some(card_id);
                }
                RenderScenePatch::DeleteCard { id } => {
                    let before = scene.cards.len();
                    scene.cards.retain(|card| card.id != id);
                    if scene.cards.len() == before {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    }
                    let mut removed_edge_ids = Vec::new();
                    scene.edges.retain(|edge| {
                        let keep = edge.source != id && edge.target != id;
                        if !keep {
                            removed_edge_ids.push(edge.id.clone());
                        }
                        keep
                    });
                    if selection_is_node(&scene.selection, &id)
                        || removed_edge_ids
                            .iter()
                            .any(|edge_id| selection_is_edge(&scene.selection, edge_id))
                    {
                        scene.selection = SceneSelection::Canvas;
                    }
                    deleted_edge_ids.extend(removed_edge_ids);
                    deleted_card_id = Some(id);
                }
                RenderScenePatch::CreateEdge {
                    group_id,
                    source,
                    target,
                    edge_id,
                    label,
                } => {
                    if source == target {
                        return Err(JsValue::from_str("Edge source and target must differ"));
                    }
                    if !scene.cards.iter().any(|card| card.id == source) {
                        return Err(JsValue::from_str(&format!(
                            "Unknown source card id: {source}"
                        )));
                    }
                    if !scene.cards.iter().any(|card| card.id == target) {
                        return Err(JsValue::from_str(&format!(
                            "Unknown target card id: {target}"
                        )));
                    }
                    if !scene.groups.iter().any(|group| group.id == group_id) {
                        return Err(JsValue::from_str(&format!("Unknown group id: {group_id}")));
                    }
                    if scene.edges.iter().any(|edge| edge.id == edge_id) {
                        return Err(JsValue::from_str(&format!("Duplicate edge id: {edge_id}")));
                    }
                    scene.edges.push(RenderEdge {
                        id: edge_id.clone(),
                        group_id,
                        source,
                        target,
                        label: label.unwrap_or_else(|| "relates".to_string()),
                        edge_type: "supports".to_string(),
                        z_index: scene.edges.len() as f64,
                        style_key: "default".to_string(),
                    });
                    created_edge_id = Some(edge_id);
                }
                RenderScenePatch::DeleteEdge { id } => {
                    let before = scene.edges.len();
                    scene.edges.retain(|edge| edge.id != id);
                    if scene.edges.len() == before {
                        return Err(JsValue::from_str(&format!("Unknown edge id: {id}")));
                    }
                    deleted_edge_ids.push(id);
                }
                RenderScenePatch::Select { selection } => {
                    let previous = scene.selection.clone();
                    collect_selection_dirty_ids(
                        &previous,
                        &mut dirty_group_ids,
                        &mut dirty_card_ids,
                        &mut dirty_edge_ids,
                    );
                    collect_selection_dirty_ids(
                        &selection,
                        &mut dirty_group_ids,
                        &mut dirty_card_ids,
                        &mut dirty_edge_ids,
                    );
                    scene.selection = selection;
                }
            }
        }
        self.patch_update_count += 1;
        if rebuild_for_order {
            self.rebuild_vertex_buffer();
            return Ok(());
        }
        let edge_slots_deleted = !deleted_edge_ids.is_empty();
        let card_slots_deleted = deleted_card_id.is_some() || !deleted_card_ids.is_empty();
        for edge_id in deleted_edge_ids {
            if !self.clear_deleted_edge(&edge_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if edge_slots_deleted && self.should_compact_edge_slots() && !self.compact_edge_slots() {
            self.rebuild_vertex_buffer();
            return Ok(());
        }
        for card_id in deleted_card_ids {
            if !self.clear_deleted_card(&card_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(card_id) = deleted_card_id {
            if !self.clear_deleted_card(&card_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if card_slots_deleted && self.should_compact_card_slots() && !self.compact_card_slots() {
            self.rebuild_vertex_buffer();
            return Ok(());
        }
        if let Some(group_id) = deleted_group_id {
            if !self.clear_deleted_group(&group_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
            if self.should_compact_group_slots() && !self.compact_group_slots() {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(group_id) = created_group_id {
            if !self.write_new_group(&group_id)
                && !(self.grow_group_slots() && self.write_new_group(&group_id))
            {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(card_id) = created_card_id {
            if !self.write_new_card(&card_id)
                && !(self.grow_card_slots() && self.write_new_card(&card_id))
            {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(edge_id) = created_edge_id {
            if !self.write_new_edge(&edge_id)
                && !(self.grow_edge_slots() && self.write_new_edge(&edge_id))
            {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        for edge_id in dirty_edge_ids {
            if !self.write_dirty_edge(&edge_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        for group_id in dirty_group_ids {
            if !self.write_dirty_group(&group_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        for card_id in dirty_card_ids {
            if !self.write_dirty_card(&card_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        Ok(())
    }


    pub(crate) fn rebuild_vertex_buffer(&mut self) {
        let mut vertices = Vec::new();
        let mut vertex_ranges = VertexRanges::default();
        let mut text_stats = TextBuildStats::default();
        let mut vertex_fit_stats = VertexFitStats::default();
        if let Some(scene) = &self.scene {
            let mut groups: Vec<&RenderGroup> = scene.groups.iter().collect();
            groups.sort_by(|a, b| a.z_index.total_cmp(&b.z_index));
            for group in groups {
                let offset = vertices.len();
                let (group_vertices, group_text_stats) = build_group_vertices(
                    scene,
                    group,
                    &mut self.text_layout_cache,
                    &mut self.text_engine,
                );
                text_stats.add(group_text_stats);
                let (group_vertices, fit_stats) =
                    fit_vertices_to_slot_with_stats(group_vertices, GROUP_VERTEX_SLOT);
                vertex_fit_stats.add(fit_stats);
                vertices.extend(group_vertices);
                vertex_ranges.groups.insert(
                    group.id.clone(),
                    VertexSlot {
                        offset,
                        capacity: GROUP_VERTEX_SLOT,
                        text_stats: group_text_stats,
                    },
                );
            }
            let group_spare_slots = group_spare_slot_count(scene.groups.len());
            for _ in 0..group_spare_slots {
                let offset = vertices.len();
                vertices.extend(fit_vertices_to_slot(Vec::new(), GROUP_VERTEX_SLOT));
                vertex_ranges.group_free_offsets.push(offset);
            }

            for edge in &scene.edges {
                let offset = vertices.len();
                let (edge_vertices, edge_text_stats) = build_edge_vertices(
                    scene,
                    edge,
                    &mut self.text_layout_cache,
                    &mut self.text_engine,
                );
                text_stats.add(edge_text_stats);
                let (edge_vertices, fit_stats) =
                    fit_vertices_to_slot_with_stats(edge_vertices, EDGE_VERTEX_SLOT);
                vertex_fit_stats.add(fit_stats);
                vertices.extend(edge_vertices);
                vertex_ranges.edges.insert(
                    edge.id.clone(),
                    VertexSlot {
                        offset,
                        capacity: EDGE_VERTEX_SLOT,
                        text_stats: edge_text_stats,
                    },
                );
            }
            let edge_spare_slots = edge_spare_slot_count(scene.edges.len());
            for _ in 0..edge_spare_slots {
                let offset = vertices.len();
                vertices.extend(fit_vertices_to_slot(Vec::new(), EDGE_VERTEX_SLOT));
                vertex_ranges.edge_free_offsets.push(offset);
            }

            let mut cards: Vec<&RenderCard> = scene.cards.iter().collect();
            cards.sort_by(|a, b| a.z_index.total_cmp(&b.z_index));
            for card in cards {
                let offset = vertices.len();
                let (card_vertices, card_text_stats) = build_card_vertices(
                    scene,
                    card,
                    &mut self.text_layout_cache,
                    &mut self.text_engine,
                );
                text_stats.add(card_text_stats);
                let (card_vertices, fit_stats) =
                    fit_vertices_to_slot_with_stats(card_vertices, CARD_VERTEX_SLOT);
                vertex_fit_stats.add(fit_stats);
                vertices.extend(card_vertices);
                vertex_ranges.cards.insert(
                    card.id.clone(),
                    VertexSlot {
                        offset,
                        capacity: CARD_VERTEX_SLOT,
                        text_stats: card_text_stats,
                    },
                );
            }
            let card_spare_slots = card_spare_slot_count(scene.cards.len());
            for _ in 0..card_spare_slots {
                let offset = vertices.len();
                vertices.extend(fit_vertices_to_slot(Vec::new(), CARD_VERTEX_SLOT));
                vertex_ranges.card_free_offsets.push(offset);
            }
        }
        let size = (vertices.len() * std::mem::size_of::<GpuVertex>()).max(4) as u64;
        self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices"),
            size,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        self.vertex_ranges = vertex_ranges;
        self.vertex_count = vertices.len();
        self.text_glyph_count = text_stats.glyph_count;
        self.fallback_text_glyph_count = text_stats.fallback_glyph_count;
        self.cjk_text_glyph_count = text_stats.cjk_glyph_count;
        self.font_fallback_run_count = text_stats.fallback_run_count;
        self.missing_text_glyph_count = text_stats.missing_glyph_count;
        self.text_atlas_overflow_glyph_count = text_stats.atlas_overflow_glyph_count;
        self.text_missing_raster_glyph_count = text_stats.missing_raster_glyph_count;
        self.record_vertex_fit_stats(vertex_fit_stats);
        self.full_buffer_rebuild_count += 1;
        if !vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }

    fn write_dirty_card(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.cards.get(id).copied() else {
            return false;
        };
        let (vertices, text_stats) = {
            let Some(scene) = &self.scene else {
                return false;
            };
            let Some(card) = scene.cards.iter().find(|card| card.id == id) else {
                return false;
            };
            build_card_vertices(
                scene,
                card,
                &mut self.text_layout_cache,
                &mut self.text_engine,
            )
        };
        self.write_slot_vertices(slot, vertices);
        if let Some(slot) = self.vertex_ranges.cards.get_mut(id) {
            self.text_glyph_count = self
                .text_glyph_count
                .saturating_sub(slot.text_stats.glyph_count)
                + text_stats.glyph_count;
            self.fallback_text_glyph_count = self
                .fallback_text_glyph_count
                .saturating_sub(slot.text_stats.fallback_glyph_count)
                + text_stats.fallback_glyph_count;
            self.cjk_text_glyph_count = self
                .cjk_text_glyph_count
                .saturating_sub(slot.text_stats.cjk_glyph_count)
                + text_stats.cjk_glyph_count;
            self.font_fallback_run_count = self
                .font_fallback_run_count
                .saturating_sub(slot.text_stats.fallback_run_count)
                + text_stats.fallback_run_count;
            self.missing_text_glyph_count = self
                .missing_text_glyph_count
                .saturating_sub(slot.text_stats.missing_glyph_count)
                + text_stats.missing_glyph_count;
            self.text_atlas_overflow_glyph_count = self
                .text_atlas_overflow_glyph_count
                .saturating_sub(slot.text_stats.atlas_overflow_glyph_count)
                + text_stats.atlas_overflow_glyph_count;
            self.text_missing_raster_glyph_count = self
                .text_missing_raster_glyph_count
                .saturating_sub(slot.text_stats.missing_raster_glyph_count)
                + text_stats.missing_raster_glyph_count;
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_dirty_group(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.groups.get(id).copied() else {
            return false;
        };
        let (vertices, text_stats) = {
            let Some(scene) = &self.scene else {
                return false;
            };
            let Some(group) = scene.groups.iter().find(|group| group.id == id) else {
                return false;
            };
            build_group_vertices(
                scene,
                group,
                &mut self.text_layout_cache,
                &mut self.text_engine,
            )
        };
        self.write_slot_vertices(slot, vertices);
        if let Some(slot) = self.vertex_ranges.groups.get_mut(id) {
            self.text_glyph_count = self
                .text_glyph_count
                .saturating_sub(slot.text_stats.glyph_count)
                + text_stats.glyph_count;
            self.fallback_text_glyph_count = self
                .fallback_text_glyph_count
                .saturating_sub(slot.text_stats.fallback_glyph_count)
                + text_stats.fallback_glyph_count;
            self.cjk_text_glyph_count = self
                .cjk_text_glyph_count
                .saturating_sub(slot.text_stats.cjk_glyph_count)
                + text_stats.cjk_glyph_count;
            self.font_fallback_run_count = self
                .font_fallback_run_count
                .saturating_sub(slot.text_stats.fallback_run_count)
                + text_stats.fallback_run_count;
            self.missing_text_glyph_count = self
                .missing_text_glyph_count
                .saturating_sub(slot.text_stats.missing_glyph_count)
                + text_stats.missing_glyph_count;
            self.text_atlas_overflow_glyph_count = self
                .text_atlas_overflow_glyph_count
                .saturating_sub(slot.text_stats.atlas_overflow_glyph_count)
                + text_stats.atlas_overflow_glyph_count;
            self.text_missing_raster_glyph_count = self
                .text_missing_raster_glyph_count
                .saturating_sub(slot.text_stats.missing_raster_glyph_count)
                + text_stats.missing_raster_glyph_count;
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_dirty_edge(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.edges.get(id).copied() else {
            return false;
        };
        let (vertices, text_stats) = {
            let Some(scene) = &self.scene else {
                return false;
            };
            let Some(edge) = scene.edges.iter().find(|edge| edge.id == id) else {
                return false;
            };
            build_edge_vertices(
                scene,
                edge,
                &mut self.text_layout_cache,
                &mut self.text_engine,
            )
        };
        self.write_slot_vertices(slot, vertices);
        if let Some(slot) = self.vertex_ranges.edges.get_mut(id) {
            self.text_glyph_count = self
                .text_glyph_count
                .saturating_sub(slot.text_stats.glyph_count)
                + text_stats.glyph_count;
            self.fallback_text_glyph_count = self
                .fallback_text_glyph_count
                .saturating_sub(slot.text_stats.fallback_glyph_count)
                + text_stats.fallback_glyph_count;
            self.cjk_text_glyph_count = self
                .cjk_text_glyph_count
                .saturating_sub(slot.text_stats.cjk_glyph_count)
                + text_stats.cjk_glyph_count;
            self.font_fallback_run_count = self
                .font_fallback_run_count
                .saturating_sub(slot.text_stats.fallback_run_count)
                + text_stats.fallback_run_count;
            self.missing_text_glyph_count = self
                .missing_text_glyph_count
                .saturating_sub(slot.text_stats.missing_glyph_count)
                + text_stats.missing_glyph_count;
            self.text_atlas_overflow_glyph_count = self
                .text_atlas_overflow_glyph_count
                .saturating_sub(slot.text_stats.atlas_overflow_glyph_count)
                + text_stats.atlas_overflow_glyph_count;
            self.text_missing_raster_glyph_count = self
                .text_missing_raster_glyph_count
                .saturating_sub(slot.text_stats.missing_raster_glyph_count)
                + text_stats.missing_raster_glyph_count;
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_new_group(&mut self, id: &str) -> bool {
        let Some(offset) = self.take_append_group_free_offset() else {
            return false;
        };
        let slot = VertexSlot {
            offset,
            capacity: GROUP_VERTEX_SLOT,
            text_stats: TextBuildStats::default(),
        };
        self.vertex_ranges.groups.insert(id.to_string(), slot);
        if !self.write_dirty_group(id) {
            self.vertex_ranges.groups.remove(id);
            self.vertex_ranges.group_free_offsets.push(offset);
            return false;
        }
        true
    }

    fn take_append_group_free_offset(&mut self) -> Option<usize> {
        let used_end = self.group_segment_used_end();
        let mut best_index = None;
        let mut best_offset = usize::MAX;
        for (index, offset) in self
            .vertex_ranges
            .group_free_offsets
            .iter()
            .copied()
            .enumerate()
        {
            if offset >= used_end && offset < best_offset {
                best_index = Some(index);
                best_offset = offset;
            }
        }
        best_index.map(|index| self.vertex_ranges.group_free_offsets.swap_remove(index))
    }

    fn write_new_edge(&mut self, id: &str) -> bool {
        let Some(offset) = self.vertex_ranges.edge_free_offsets.pop() else {
            return false;
        };
        let slot = VertexSlot {
            offset,
            capacity: EDGE_VERTEX_SLOT,
            text_stats: TextBuildStats::default(),
        };
        self.vertex_ranges.edges.insert(id.to_string(), slot);
        if !self.write_dirty_edge(id) {
            self.vertex_ranges.edges.remove(id);
            self.vertex_ranges.edge_free_offsets.push(offset);
            return false;
        }
        true
    }

    fn write_new_card(&mut self, id: &str) -> bool {
        let Some(offset) = self.take_append_card_free_offset() else {
            return false;
        };
        let slot = VertexSlot {
            offset,
            capacity: CARD_VERTEX_SLOT,
            text_stats: TextBuildStats::default(),
        };
        self.vertex_ranges.cards.insert(id.to_string(), slot);
        if !self.write_dirty_card(id) {
            self.vertex_ranges.cards.remove(id);
            self.vertex_ranges.card_free_offsets.push(offset);
            return false;
        }
        true
    }

    fn take_append_card_free_offset(&mut self) -> Option<usize> {
        let used_end = self.card_segment_used_end();
        let mut best_index = None;
        let mut best_offset = usize::MAX;
        for (index, offset) in self
            .vertex_ranges
            .card_free_offsets
            .iter()
            .copied()
            .enumerate()
        {
            if offset >= used_end && offset < best_offset {
                best_index = Some(index);
                best_offset = offset;
            }
        }
        best_index.map(|index| self.vertex_ranges.card_free_offsets.swap_remove(index))
    }

    fn grow_group_slots(&mut self) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let additional_slots = group_spare_slot_count(scene.groups.len());
        let additional_vertices = additional_slots * GROUP_VERTEX_SLOT;
        if additional_vertices == 0 {
            return false;
        }

        let insert_offset = self.group_segment_end();
        let suffix_vertices = self.vertex_count.saturating_sub(insert_offset);
        let new_vertex_count = self.vertex_count + additional_vertices;
        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices grown for group slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU group slot growth encoder"),
            });
        if insert_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                insert_offset as u64 * vertex_size,
            );
        }
        if suffix_vertices > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                insert_offset as u64 * vertex_size,
                &new_buffer,
                (insert_offset + additional_vertices) as u64 * vertex_size,
                suffix_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices = fit_vertices_to_slot(Vec::new(), additional_vertices);
        self.queue.write_buffer(
            &new_buffer,
            insert_offset as u64 * vertex_size,
            bytemuck::cast_slice(&transparent_vertices),
        );

        for slot in self.vertex_ranges.edges.values_mut() {
            slot.offset += additional_vertices;
        }
        for offset in &mut self.vertex_ranges.edge_free_offsets {
            *offset += additional_vertices;
        }
        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset += additional_vertices;
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset += additional_vertices;
        }
        for index in 0..additional_slots {
            self.vertex_ranges
                .group_free_offsets
                .push(insert_offset + index * GROUP_VERTEX_SLOT);
        }
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.group_capacity_grow_count += 1;
        true
    }

    fn should_compact_group_slots(&self) -> bool {
        let used_slots = self.vertex_ranges.groups.len();
        let free_slots = self.vertex_ranges.group_free_offsets.len();
        let target_free_slots = group_spare_slot_count(used_slots);
        free_slots > target_free_slots * 2 && free_slots.saturating_sub(target_free_slots) >= 8
    }

    fn compact_group_slots(&mut self) -> bool {
        let target_free_slots = group_spare_slot_count(self.vertex_ranges.groups.len());
        let old_group_end = self.group_segment_end();
        if old_group_end > self.vertex_count {
            return false;
        }

        let used_groups: Vec<(String, VertexSlot)> = {
            let mut groups: Vec<(String, VertexSlot)> = self
                .vertex_ranges
                .groups
                .iter()
                .map(|(id, slot)| (id.clone(), *slot))
                .collect();
            groups.sort_by_key(|(_, slot)| slot.offset);
            groups
        };
        let compact_group_vertices = (used_groups.len() + target_free_slots) * GROUP_VERTEX_SLOT;
        let new_vertex_count =
            compact_group_vertices + self.vertex_count.saturating_sub(old_group_end);
        if new_vertex_count >= self.vertex_count {
            return false;
        }

        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices compacted for group slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU group slot compaction encoder"),
            });
        let mut compacted_groups = HashMap::new();
        for (index, (id, slot)) in used_groups.into_iter().enumerate() {
            let new_offset = index * GROUP_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                slot.offset as u64 * vertex_size,
                &new_buffer,
                new_offset as u64 * vertex_size,
                GROUP_VERTEX_SLOT as u64 * vertex_size,
            );
            compacted_groups.insert(
                id,
                VertexSlot {
                    offset: new_offset,
                    capacity: slot.capacity,
                    text_stats: slot.text_stats,
                },
            );
        }
        let free_start_offset = compacted_groups.len() * GROUP_VERTEX_SLOT;
        let new_suffix_start = free_start_offset + target_free_slots * GROUP_VERTEX_SLOT;
        let suffix_vertices = self.vertex_count - old_group_end;
        if suffix_vertices > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                old_group_end as u64 * vertex_size,
                &new_buffer,
                new_suffix_start as u64 * vertex_size,
                suffix_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices =
            fit_vertices_to_slot(Vec::new(), target_free_slots * GROUP_VERTEX_SLOT);
        if !transparent_vertices.is_empty() {
            self.queue.write_buffer(
                &new_buffer,
                free_start_offset as u64 * vertex_size,
                bytemuck::cast_slice(&transparent_vertices),
            );
        }

        let suffix_delta = new_suffix_start as isize - old_group_end as isize;
        for slot in self.vertex_ranges.edges.values_mut() {
            slot.offset = slot.offset.saturating_add_signed(suffix_delta);
        }
        for offset in &mut self.vertex_ranges.edge_free_offsets {
            *offset = offset.saturating_add_signed(suffix_delta);
        }
        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset = slot.offset.saturating_add_signed(suffix_delta);
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset = offset.saturating_add_signed(suffix_delta);
        }
        self.vertex_ranges.groups = compacted_groups;
        self.vertex_ranges.group_free_offsets = (0..target_free_slots)
            .map(|index| free_start_offset + index * GROUP_VERTEX_SLOT)
            .collect();
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.group_compaction_count += 1;
        true
    }

    fn grow_edge_slots(&mut self) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let additional_slots = edge_spare_slot_count(scene.edges.len());
        let additional_vertices = additional_slots * EDGE_VERTEX_SLOT;
        if additional_vertices == 0 {
            return false;
        }

        let card_insert_offset = self.card_segment_start();
        let suffix_vertices = self.vertex_count.saturating_sub(card_insert_offset);
        let new_vertex_count = self.vertex_count + additional_vertices;
        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices grown for edge slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU edge slot growth encoder"),
            });
        if card_insert_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                card_insert_offset as u64 * vertex_size,
            );
        }
        if suffix_vertices > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                card_insert_offset as u64 * vertex_size,
                &new_buffer,
                (card_insert_offset + additional_vertices) as u64 * vertex_size,
                suffix_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices = fit_vertices_to_slot(Vec::new(), additional_vertices);
        self.queue.write_buffer(
            &new_buffer,
            card_insert_offset as u64 * vertex_size,
            bytemuck::cast_slice(&transparent_vertices),
        );

        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset += additional_vertices;
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset += additional_vertices;
        }
        for index in 0..additional_slots {
            self.vertex_ranges
                .edge_free_offsets
                .push(card_insert_offset + index * EDGE_VERTEX_SLOT);
        }
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.edge_capacity_grow_count += 1;
        true
    }

    fn should_compact_edge_slots(&self) -> bool {
        let used_slots = self.vertex_ranges.edges.len();
        let free_slots = self.vertex_ranges.edge_free_offsets.len();
        let target_free_slots = edge_spare_slot_count(used_slots);
        free_slots > target_free_slots * 2 && free_slots.saturating_sub(target_free_slots) >= 8
    }

    fn compact_edge_slots(&mut self) -> bool {
        let target_free_slots = edge_spare_slot_count(self.vertex_ranges.edges.len());
        let edge_start_offset = self.group_segment_end();
        let card_start_offset = self.card_segment_start();
        if card_start_offset < edge_start_offset || card_start_offset > self.vertex_count {
            return false;
        }

        let card_vertices = self.vertex_count - card_start_offset;
        let used_edges: Vec<(String, VertexSlot)> = {
            let mut edges: Vec<(String, VertexSlot)> = self
                .vertex_ranges
                .edges
                .iter()
                .map(|(id, slot)| (id.clone(), *slot))
                .collect();
            edges.sort_by_key(|(_, slot)| slot.offset);
            edges
        };
        let compact_edge_vertices = (used_edges.len() + target_free_slots) * EDGE_VERTEX_SLOT;
        let new_vertex_count = edge_start_offset + compact_edge_vertices + card_vertices;
        if new_vertex_count >= self.vertex_count {
            return false;
        }

        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices compacted for edge slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU edge slot compaction encoder"),
            });
        if edge_start_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                edge_start_offset as u64 * vertex_size,
            );
        }
        let mut compacted_edges = HashMap::new();
        for (index, (id, slot)) in used_edges.into_iter().enumerate() {
            let new_offset = edge_start_offset + index * EDGE_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                slot.offset as u64 * vertex_size,
                &new_buffer,
                new_offset as u64 * vertex_size,
                EDGE_VERTEX_SLOT as u64 * vertex_size,
            );
            compacted_edges.insert(
                id,
                VertexSlot {
                    offset: new_offset,
                    capacity: slot.capacity,
                    text_stats: slot.text_stats,
                },
            );
        }
        let free_start_offset = edge_start_offset + compacted_edges.len() * EDGE_VERTEX_SLOT;
        if card_vertices > 0 {
            let new_card_start_offset = free_start_offset + target_free_slots * EDGE_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                card_start_offset as u64 * vertex_size,
                &new_buffer,
                new_card_start_offset as u64 * vertex_size,
                card_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices =
            fit_vertices_to_slot(Vec::new(), target_free_slots * EDGE_VERTEX_SLOT);
        if !transparent_vertices.is_empty() {
            self.queue.write_buffer(
                &new_buffer,
                free_start_offset as u64 * vertex_size,
                bytemuck::cast_slice(&transparent_vertices),
            );
        }

        let new_card_start_offset = free_start_offset + target_free_slots * EDGE_VERTEX_SLOT;
        let card_offset_delta = new_card_start_offset as isize - card_start_offset as isize;
        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset = slot.offset.saturating_add_signed(card_offset_delta);
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset = offset.saturating_add_signed(card_offset_delta);
        }
        self.vertex_ranges.edges = compacted_edges;
        self.vertex_ranges.edge_free_offsets = (0..target_free_slots)
            .map(|index| free_start_offset + index * EDGE_VERTEX_SLOT)
            .collect();
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.edge_compaction_count += 1;
        true
    }

    fn grow_card_slots(&mut self) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let additional_slots = card_spare_slot_count(scene.cards.len());
        let additional_vertices = additional_slots * CARD_VERTEX_SLOT;
        if additional_vertices == 0 {
            return false;
        }

        let old_vertex_count = self.vertex_count;
        let new_vertex_count = old_vertex_count + additional_vertices;
        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices grown for card slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU card slot growth encoder"),
            });
        if old_vertex_count > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                old_vertex_count as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices = fit_vertices_to_slot(Vec::new(), additional_vertices);
        self.queue.write_buffer(
            &new_buffer,
            old_vertex_count as u64 * vertex_size,
            bytemuck::cast_slice(&transparent_vertices),
        );

        for index in 0..additional_slots {
            self.vertex_ranges
                .card_free_offsets
                .push(old_vertex_count + index * CARD_VERTEX_SLOT);
        }
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.card_capacity_grow_count += 1;
        true
    }

    fn should_compact_card_slots(&self) -> bool {
        let used_slots = self.vertex_ranges.cards.len();
        let free_slots = self.vertex_ranges.card_free_offsets.len();
        let target_free_slots = card_spare_slot_count(used_slots);
        free_slots > target_free_slots * 2 && free_slots.saturating_sub(target_free_slots) >= 8
    }

    fn compact_card_slots(&mut self) -> bool {
        let target_free_slots = card_spare_slot_count(self.vertex_ranges.cards.len());
        let card_start_offset = self.card_segment_start();
        if card_start_offset > self.vertex_count {
            return false;
        }

        let used_cards: Vec<(String, VertexSlot)> = {
            let mut cards: Vec<(String, VertexSlot)> = self
                .vertex_ranges
                .cards
                .iter()
                .map(|(id, slot)| (id.clone(), *slot))
                .collect();
            cards.sort_by_key(|(_, slot)| slot.offset);
            cards
        };
        let compact_card_vertices = (used_cards.len() + target_free_slots) * CARD_VERTEX_SLOT;
        let new_vertex_count = card_start_offset + compact_card_vertices;
        if new_vertex_count >= self.vertex_count {
            return false;
        }

        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices compacted for card slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU card slot compaction encoder"),
            });
        if card_start_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                card_start_offset as u64 * vertex_size,
            );
        }
        let mut compacted_cards = HashMap::new();
        for (index, (id, slot)) in used_cards.into_iter().enumerate() {
            let new_offset = card_start_offset + index * CARD_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                slot.offset as u64 * vertex_size,
                &new_buffer,
                new_offset as u64 * vertex_size,
                CARD_VERTEX_SLOT as u64 * vertex_size,
            );
            compacted_cards.insert(
                id,
                VertexSlot {
                    offset: new_offset,
                    capacity: slot.capacity,
                    text_stats: slot.text_stats,
                },
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let free_start_offset = card_start_offset + compacted_cards.len() * CARD_VERTEX_SLOT;
        let transparent_vertices =
            fit_vertices_to_slot(Vec::new(), target_free_slots * CARD_VERTEX_SLOT);
        if !transparent_vertices.is_empty() {
            self.queue.write_buffer(
                &new_buffer,
                free_start_offset as u64 * vertex_size,
                bytemuck::cast_slice(&transparent_vertices),
            );
        }

        self.vertex_ranges.cards = compacted_cards;
        self.vertex_ranges.card_free_offsets = (0..target_free_slots)
            .map(|index| free_start_offset + index * CARD_VERTEX_SLOT)
            .collect();
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.card_compaction_count += 1;
        true
    }

    fn group_segment_end(&self) -> usize {
        self.vertex_ranges
            .groups
            .values()
            .map(|slot| slot.offset + slot.capacity)
            .chain(
                self.vertex_ranges
                    .group_free_offsets
                    .iter()
                    .map(|offset| offset + GROUP_VERTEX_SLOT),
            )
            .max()
            .unwrap_or(0)
    }

    fn group_segment_used_end(&self) -> usize {
        self.vertex_ranges
            .groups
            .values()
            .map(|slot| slot.offset + slot.capacity)
            .max()
            .unwrap_or(0)
    }

    fn card_segment_start(&self) -> usize {
        self.vertex_ranges
            .cards
            .values()
            .map(|slot| slot.offset)
            .chain(self.vertex_ranges.card_free_offsets.iter().copied())
            .min()
            .unwrap_or(self.vertex_count)
    }

    fn card_segment_used_end(&self) -> usize {
        self.vertex_ranges
            .cards
            .values()
            .map(|slot| slot.offset + slot.capacity)
            .max()
            .unwrap_or_else(|| self.card_segment_start())
    }

    fn clear_deleted_edge(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.edges.remove(id) else {
            return false;
        };
        self.write_slot_vertices(slot, Vec::new());
        self.vertex_ranges.edge_free_offsets.push(slot.offset);
        self.text_glyph_count = self
            .text_glyph_count
            .saturating_sub(slot.text_stats.glyph_count);
        self.fallback_text_glyph_count = self
            .fallback_text_glyph_count
            .saturating_sub(slot.text_stats.fallback_glyph_count);
        self.cjk_text_glyph_count = self
            .cjk_text_glyph_count
            .saturating_sub(slot.text_stats.cjk_glyph_count);
        self.font_fallback_run_count = self
            .font_fallback_run_count
            .saturating_sub(slot.text_stats.fallback_run_count);
        self.missing_text_glyph_count = self
            .missing_text_glyph_count
            .saturating_sub(slot.text_stats.missing_glyph_count);
        self.text_atlas_overflow_glyph_count = self
            .text_atlas_overflow_glyph_count
            .saturating_sub(slot.text_stats.atlas_overflow_glyph_count);
        self.text_missing_raster_glyph_count = self
            .text_missing_raster_glyph_count
            .saturating_sub(slot.text_stats.missing_raster_glyph_count);
        self.dirty_range_write_count += 1;
        true
    }

    fn clear_deleted_card(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.cards.remove(id) else {
            return false;
        };
        self.write_slot_vertices(slot, Vec::new());
        self.vertex_ranges.card_free_offsets.push(slot.offset);
        self.text_glyph_count = self
            .text_glyph_count
            .saturating_sub(slot.text_stats.glyph_count);
        self.fallback_text_glyph_count = self
            .fallback_text_glyph_count
            .saturating_sub(slot.text_stats.fallback_glyph_count);
        self.cjk_text_glyph_count = self
            .cjk_text_glyph_count
            .saturating_sub(slot.text_stats.cjk_glyph_count);
        self.font_fallback_run_count = self
            .font_fallback_run_count
            .saturating_sub(slot.text_stats.fallback_run_count);
        self.missing_text_glyph_count = self
            .missing_text_glyph_count
            .saturating_sub(slot.text_stats.missing_glyph_count);
        self.text_atlas_overflow_glyph_count = self
            .text_atlas_overflow_glyph_count
            .saturating_sub(slot.text_stats.atlas_overflow_glyph_count);
        self.text_missing_raster_glyph_count = self
            .text_missing_raster_glyph_count
            .saturating_sub(slot.text_stats.missing_raster_glyph_count);
        self.dirty_range_write_count += 1;
        true
    }

    fn clear_deleted_group(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.groups.remove(id) else {
            return false;
        };
        self.write_slot_vertices(slot, Vec::new());
        self.vertex_ranges.group_free_offsets.push(slot.offset);
        self.text_glyph_count = self
            .text_glyph_count
            .saturating_sub(slot.text_stats.glyph_count);
        self.fallback_text_glyph_count = self
            .fallback_text_glyph_count
            .saturating_sub(slot.text_stats.fallback_glyph_count);
        self.cjk_text_glyph_count = self
            .cjk_text_glyph_count
            .saturating_sub(slot.text_stats.cjk_glyph_count);
        self.font_fallback_run_count = self
            .font_fallback_run_count
            .saturating_sub(slot.text_stats.fallback_run_count);
        self.missing_text_glyph_count = self
            .missing_text_glyph_count
            .saturating_sub(slot.text_stats.missing_glyph_count);
        self.text_atlas_overflow_glyph_count = self
            .text_atlas_overflow_glyph_count
            .saturating_sub(slot.text_stats.atlas_overflow_glyph_count);
        self.text_missing_raster_glyph_count = self
            .text_missing_raster_glyph_count
            .saturating_sub(slot.text_stats.missing_raster_glyph_count);
        self.dirty_range_write_count += 1;
        true
    }

    fn record_vertex_fit_stats(&mut self, stats: VertexFitStats) {
        self.vertex_truncation_count += stats.truncation_count;
        self.truncated_vertex_count += stats.truncated_vertex_count;
    }

    fn write_slot_vertices(&mut self, slot: VertexSlot, vertices: Vec<GpuVertex>) {
        let (vertices, fit_stats) = fit_vertices_to_slot_with_stats(vertices, slot.capacity);
        self.record_vertex_fit_stats(fit_stats);
        let byte_offset = (slot.offset * std::mem::size_of::<GpuVertex>()) as u64;
        self.queue.write_buffer(
            &self.vertex_buffer,
            byte_offset,
            bytemuck::cast_slice(&vertices),
        );
    }

}
