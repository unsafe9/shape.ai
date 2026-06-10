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
use shape_scene_core::object::move_together::{BindingGraph, BindingNode};
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
            // W3-G6/#3: build in the PERSISTED theme so the dark canvas-bg survives
            // every re-feed (pan/move/create). The bit is owned by the wrapper
            // (`self.object_theme`, set by `set_object_theme`), not the per-scene
            // renderer that this re-feed throws away — so the clear stays dark with
            // zero extra GPU work (no post-build `set_theme` call needed).
            self.object_theme,
        );
        let counts = ObjectSceneLoadResult {
            objects: scene.objects.len(),
            fill_indices: renderer.fill_index_count() as usize,
            stroke_vertices: renderer.stroke_vertex_count() as usize,
        };
        // FC-04: derive + retain each object's local-space region for hit-test /
        // marquee, then keep the parsed scene as the live-object branch switch.
        self.object_regions = derive_object_regions(&scene);
        // W3-G9/#2: the wire now carries `multiSelect`; mirror it onto the renderer
        // so a later object re-feed (which throws away the per-scene renderer) keeps
        // the highlight set, matching how the legacy `load_scene` retains it.
        self.multi_select = scene.multi_select.clone();
        // W3-G9/#5: precompute the move-together propagation graph ONCE per feed so
        // each drag preview is O(closure), not O(scene).
        self.object_bindings = BindingGraph::build(&binding_nodes(&scene));
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

    /// W2-11 drag zero-rebake: push ONLY the affected objects' instance model
    /// matrices to the GPU. `matrix_json` is a row-major `[[f64;3];3]` CUMULATIVE
    /// world-space DELTA (the same contract as `ObjectTransformDelta.matrix`). The
    /// canonical `object_scene` / `object_regions` are NOT mutated — each affected
    /// object's base transform is read from the untouched scene and `delta * base` is
    /// written straight to its instance buffer (no re-tessellation, no region
    /// re-derive). The RAF `render_frame` loop then draws from the updated instance
    /// buffer on the next tick, so no explicit redraw is needed. No-op if the object
    /// scene is unloaded or the id is absent.
    ///
    /// W3-G9/#5: the affected set is the SameDelta closure of the bindings graph —
    /// the dragged id (or, if it is a `multi_select` member, every member) plus their
    /// children/descendants — so group children and multi members move LIVE during
    /// the drag, each against its own base. O(closure) instance writes, zero rebake.
    #[wasm_bindgen(js_name = setObjectPreviewTransform)]
    pub fn set_object_preview_transform(
        &mut self,
        id: &str,
        matrix_json: &str,
    ) -> Result<(), JsValue> {
        let delta: [[f64; 3]; 3] = serde_json::from_str(matrix_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid preview matrix: {error}")))?;
        let Some(scene) = self.object_scene.as_ref() else {
            return Ok(());
        };
        let write_set = preview_write_set(scene, &self.object_bindings, id);
        // W3-G9/#4: the closure also yields `reproject_followers` — objects whose
        // anchored geometry must reproject through a moved target LIVE. A follower is
        // NOT uniformly transformed (one bound node moves, the rest stay), so the
        // instance-matrix preview above cannot express it; instead each follower's
        // geometry is rewritten with the reprojected node and re-expanded in place.
        let followers = preview_reproject_followers(scene, &self.object_bindings, id);
        let reexpanded = self.reexpand_reprojected_followers(scene, Some(&delta), &followers);
        if let Some(renderer) = self.object_renderer.as_mut() {
            for (target, base) in &write_set {
                renderer.set_preview_transform(&self.queue, target, &delta, base);
            }
            for (follower_id, rebuilt) in &reexpanded {
                renderer.patch_follower_geometry(&self.queue, follower_id, rebuilt);
            }
        }
        Ok(())
    }

    /// W2-11: revert the affected objects' instance matrices to their canonical baked
    /// transforms (`delta = identity`), dropping the live preview. No-op if the
    /// object scene is unloaded or the id is absent.
    ///
    /// W3-G9/#5: symmetric with the preview — reverts the SAME SameDelta closure
    /// (group children + multi members), not just the picked id.
    #[wasm_bindgen(js_name = clearObjectPreview)]
    pub fn clear_object_preview(&mut self, id: &str) -> Result<(), JsValue> {
        let Some(scene) = self.object_scene.as_ref() else {
            return Ok(());
        };
        let write_set = preview_write_set(scene, &self.object_bindings, id);
        // W3-G9/#4: restore each follower's CANONICAL baked geometry (re-expand from
        // the untouched scene object) so a cancelled/failed drag reverts the live
        // reproject too, not just the same-delta instance matrices.
        let followers = preview_reproject_followers(scene, &self.object_bindings, id);
        let restored = self.reexpand_reprojected_followers(scene, None, &followers);
        if let Some(renderer) = self.object_renderer.as_mut() {
            for (target, base) in &write_set {
                renderer.clear_preview_transform(&self.queue, target, base);
            }
            for (follower_id, rebuilt) in &restored {
                renderer.patch_follower_geometry(&self.queue, follower_id, rebuilt);
            }
        }
        Ok(())
    }

    /// W3-G9/#4: re-expand each follower's geometry for an in-place GPU patch. With
    /// `Some(delta)` (the live preview), every node of the follower anchored onto a
    /// moved target is rewritten to track the target's PREVIEWED transform
    /// (`delta * target_base`) before the re-expand. With `None` (the restore on
    /// cancel/commit), the follower's CANONICAL geometry is re-expanded untouched,
    /// so the live reproject is patched back out exactly.
    ///
    /// W3-G13: the pairs are GROUPED by follower first ([`reprojected_follower_geometries`])
    /// — a follower anchored to TWO moved targets accumulates both rewrites into ONE
    /// geometry, then ONE re-expand + ONE patch. O(unique followers), each a single
    /// small object — never a full-scene rebake.
    fn reexpand_reprojected_followers(
        &self,
        scene: &RenderObjectScene,
        delta: Option<&[[f64; 3]; 3]>,
        followers: &[(String, String)],
    ) -> Vec<(String, crate::object_pipeline::FollowerReexpand)> {
        let Some(theme) = self.object_renderer.as_ref().map(|r| r.theme()) else {
            return Vec::new();
        };
        reprojected_follower_geometries(scene, delta, followers)
            .into_iter()
            .filter_map(|(follower_id, geometry_d)| {
                let follower = scene.objects.iter().find(|o| o.id == follower_id)?;
                let mut reprojected = follower.clone();
                reprojected.geometry_d = geometry_d;
                let rebuilt = crate::object_pipeline::reexpand_single_object(
                    &reprojected,
                    theme,
                    scene.camera.clone(),
                );
                Some((follower_id, rebuilt))
            })
            .collect()
    }

}

/// W3-G9/#5: the tiny binding projection of `scene` the move-together graph is
/// built from — each object's id, containment parent, and anchor targets. O(objects)
/// cheap String clones, no geometry/transform copied. Paid once per feed at build
/// time, never per frame.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn binding_nodes(scene: &RenderObjectScene) -> Vec<BindingNode> {
    scene
        .objects
        .iter()
        .map(|o| BindingNode {
            id: o.id.clone(),
            parent: o.parent.clone(),
            anchor_targets: o.anchors.iter().map(|a| a.target.clone()).collect(),
        })
        .collect()
}

/// W3-G9/#5: the propagation ROOTS for a drag of `dragged`. If `dragged` is a
/// member of the scene's `multi_select` set, every member is a root (deduped,
/// canonical order preserved), so the whole selection drives the closure; otherwise
/// just the dragged id — so dragging a non-member is a fresh single drag even when a
/// multi-select exists. Pure (no device/GPU), host-testable under `wgpu-probe`.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn preview_roots(scene: &RenderObjectScene, dragged: &str) -> Vec<String> {
    if !scene.multi_select.iter().any(|id| id == dragged) {
        return vec![dragged.to_string()];
    }
    let mut roots: Vec<String> = Vec::with_capacity(scene.multi_select.len());
    for id in &scene.multi_select {
        if !roots.iter().any(|seen| seen == id) {
            roots.push(id.clone());
        }
    }
    roots
}

/// W3-G9/#5: the `(id, base)` GPU write-set for a drag of `dragged` — the SameDelta
/// closure of the bindings graph from [`preview_roots`], each id paired with its
/// canonical base transform from `scene`. This is the pure half of
/// [`ShapeWebGpuRenderer::set_object_preview_transform`]: every same-delta id gets
/// the same world delta applied against its own base, so group children + multi
/// members move LIVE. Ids absent from `scene.objects` are dropped. Host-testable.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn preview_write_set(
    scene: &RenderObjectScene,
    bindings: &BindingGraph,
    dragged: &str,
) -> Vec<(String, [[f64; 3]; 3])> {
    let roots = preview_roots(scene, dragged);
    let (same_delta, _reproject_followers) = bindings.propagation_closure(&roots);
    same_delta
        .into_iter()
        .filter_map(|id| {
            scene
                .objects
                .iter()
                .find(|o| o.id == id)
                .map(|o| (id, o.transform))
        })
        .collect()
}

/// W3-G9/#4: the `(follower_id, target_id)` reproject pairs for a drag of `dragged`
/// — the Reproject half of the same closure [`preview_write_set`] consumes the
/// SameDelta half of. Each pair names a follower whose anchored geometry must
/// reproject through a moved `target` (a same-delta object). Pure (no device/GPU),
/// host-testable under `wgpu-probe`; the wasm32 preview path turns each pair into an
/// in-place follower vertex patch.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn preview_reproject_followers(
    scene: &RenderObjectScene,
    bindings: &BindingGraph,
    dragged: &str,
) -> Vec<(String, String)> {
    let roots = preview_roots(scene, dragged);
    let (_same_delta, reproject_followers) = bindings.propagation_closure(&roots);
    reproject_followers
}

/// W3-G13: GROUP the `(follower, target)` reproject pairs by FOLLOWER and fold every
/// paired target's node rewrites into ONE cumulative `geometry_d` per follower (the
/// closure dedupes by PAIR, so a follower anchored to two moved targets arrives
/// twice — restarting from the canonical path per pair would stomp the first
/// target's rewrite). Returns one `(follower_id, geometry_d)` per unique follower in
/// first-appearance order; with `delta = None` (the restore path) each unique
/// follower comes back once with its CANONICAL geometry. The reproject math is
/// scene-core's `reproject_geometry_node` — the same the commit path folds with.
/// Pure (no device/GPU), host-testable under `wgpu-probe`; the wasm32
/// `reexpand_reprojected_followers` is a thin re-expand consumer.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn reprojected_follower_geometries(
    scene: &RenderObjectScene,
    delta: Option<&[[f64; 3]; 3]>,
    followers: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (follower_id, target_id) in followers {
        let Some(follower) = scene.objects.iter().find(|o| &o.id == follower_id) else {
            continue;
        };
        // One cumulative entry per follower; the first pair seeds it canonical.
        let entry = match out.iter().position(|(id, _)| id == follower_id) {
            Some(i) => &mut out[i],
            None => {
                out.push((follower_id.clone(), follower.geometry_d.clone()));
                out.last_mut().expect("entry just pushed")
            }
        };
        let Some(delta) = delta else {
            continue;
        };
        let Some(target) = scene.objects.iter().find(|o| &o.id == target_id) else {
            continue;
        };
        for anchor in &follower.anchors {
            if &anchor.target != target_id {
                continue;
            }
            if let Some(rewritten) = shape_scene_core::object::reproject_geometry_node(
                &shape_scene_core::object::Transform3x3 { m: follower.transform },
                &shape_scene_core::object::Transform3x3 { m: target.transform },
                &shape_scene_core::object::Transform3x3 { m: *delta },
                shape_scene_core::object::LocalPoint {
                    x: anchor.at.x.round() as i32,
                    y: anchor.at.y.round() as i32,
                },
                i32::try_from(anchor.node_index).unwrap_or(i32::MAX),
                &entry.1,
            ) {
                entry.1 = rewritten;
            }
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::{binding_nodes, preview_roots, preview_write_set};
    use crate::model::CameraState;
    use crate::render_object::{RenderObject, RenderObjectScene};
    use shape_scene_core::object::move_together::BindingGraph;

    fn scene_with_multi_select(multi_select: Vec<&str>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects: Vec::new(),
            selection: None,
            multi_select: multi_select.into_iter().map(str::to_string).collect(),
        }
    }

    fn object(id: &str, parent: Option<&str>, tx: f64) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            order: "a0".to_string(),
            transform: [[1.0, 0.0, tx], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            geometry_d: "M 0 0 L 8 0 L 8 8 L 0 8 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
        }
    }

    #[test]
    fn dragging_a_member_roots_the_whole_set_in_canonical_order() {
        let scene = scene_with_multi_select(vec!["a", "b", "c"]);
        // The bug returned only ["b"]; the fix roots every member together.
        assert_eq!(preview_roots(&scene, "b"), vec!["a", "b", "c"]);
    }

    #[test]
    fn dragging_a_non_member_roots_only_itself() {
        let scene = scene_with_multi_select(vec!["a", "b", "c"]);
        assert_eq!(preview_roots(&scene, "z"), vec!["z"]);
    }

    #[test]
    fn empty_multi_select_roots_only_the_dragged_id() {
        let scene = scene_with_multi_select(Vec::new());
        assert_eq!(preview_roots(&scene, "a"), vec!["a"]);
    }

    #[test]
    fn write_set_is_the_closure_with_each_objects_own_base() {
        // Frame `a{b}` plus a stand-alone member `d`; multi-select [a, d].
        let mut scene = scene_with_multi_select(vec!["a", "d"]);
        scene.objects = vec![
            object("a", None, 1.0),
            object("b", Some("a"), 2.0),
            object("d", None, 3.0),
        ];
        let bindings = BindingGraph::build(&binding_nodes(&scene));

        // Dragging member `a` writes the SameDelta closure of [a, d]: a, its child b,
        // and d — each paired with its OWN base transform (not the dragged one).
        let write_set = preview_write_set(&scene, &bindings, "a");
        let ids: Vec<&str> = write_set.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "d"]);
        // tx is encoded in column 2 of row 0; confirm per-object base, not shared.
        assert_eq!(write_set[0].1[0][2], 1.0); // a
        assert_eq!(write_set[1].1[0][2], 2.0); // b (child of a)
        assert_eq!(write_set[2].1[0][2], 3.0); // d (other member)
    }

    #[test]
    fn write_set_of_a_lone_drag_is_just_that_subtree() {
        let mut scene = scene_with_multi_select(Vec::new());
        scene.objects = vec![
            object("a", None, 1.0),
            object("b", Some("a"), 2.0),
            object("z", None, 9.0),
        ];
        let bindings = BindingGraph::build(&binding_nodes(&scene));
        let ids: Vec<String> = preview_write_set(&scene, &bindings, "a")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert!(!ids.iter().any(|id| id == "z"));
    }

    fn translate(tx: f64, ty: f64) -> [[f64; 3]; 3] {
        [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]]
    }

    fn anchored_object(
        id: &str,
        parent: Option<&str>,
        transform: [[f64; 3]; 3],
        geometry_d: &str,
        anchors: Vec<crate::render_object::RAnchor>,
    ) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            order: "a0".to_string(),
            transform,
            geometry_d: geometry_d.to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors,
            clip: false,
        }
    }

    /// CROSS-CORE BINDING-GRAPH GUARD. Proves the renderer live-preview CLOSURE half
    /// (`preview_reproject_followers` + `preview_write_set`) drives the SAME pinned
    /// numeric vector scene-core's `anchor_follow::tests::reproject_matches_cross_core_vector`
    /// pins (`"M 0 0 L -664 224"`), closing the gap between "the math matches" and
    /// "the closure actually routes the follower to that math". The renderer now
    /// drives the SAME scene-core math: the closure is `BindingGraph` and the
    /// reproject is `reproject_geometry_node`, so this guard pins the renderer's
    /// real call path against the cross-core vector.
    #[cfg(feature = "wgpu-probe")]
    #[test]
    fn anchor_follower_closure_agrees_with_scene_core_vector() {
        use super::preview_reproject_followers;
        use crate::render_object::{RAnchor, RLocalPoint};
        use shape_scene_core::object::{reproject_geometry_node, LocalPoint, Transform3x3};

        // Target A (the dragged object) and follower B anchored to A. Numbers reuse
        // the pinned cross-core vector so the closure result is hand-checkable.
        let a = anchored_object(
            "a",
            None,
            translate(10.0, 20.0),
            "M 0 0 L 8 0 L 8 8 L 0 8 Z",
            Vec::new(),
        );
        let b = anchored_object(
            "b",
            None,
            translate(100.0, 0.0),
            "M 0 0 L 64 0",
            vec![RAnchor {
                node_index: 1,
                target: "a".to_string(),
                at: RLocalPoint { x: 16.0, y: 8.0 },
            }],
        );
        let scene = RenderObjectScene {
            scene_id: "anchor-follow".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects: vec![a, b],
            selection: None,
            multi_select: Vec::new(),
        };
        let bindings = BindingGraph::build(&binding_nodes(&scene));
        let delta = translate(5.0, 7.0);

        // (1) The closure returns the follower paired with its target: B follows A.
        let followers = preview_reproject_followers(&scene, &bindings, "a");
        assert_eq!(followers, vec![("b".to_string(), "a".to_string())]);

        // (2) GROUP/multiselect SameDelta moves a child of A but NOT the follower B.
        let mut scene2 = scene.clone();
        scene2.objects.push(anchored_object(
            "c",
            Some("a"),
            translate(3.0, 3.0),
            "M 0 0 L 8 0 L 8 8 L 0 8 Z",
            Vec::new(),
        ));
        let bindings2 = BindingGraph::build(&binding_nodes(&scene2));
        let write_set = preview_write_set(&scene2, &bindings2, "a");
        let ids: Vec<&str> = write_set.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
        assert!(!ids.iter().any(|id| *id == "b"));

        // (3) Reprojected follower geometry equals the scene-core pinned vector, via
        // the SAME `reproject_geometry_node` the live preview path now calls.
        let a_base = scene.objects.iter().find(|o| o.id == "a").unwrap().transform;
        let b = scene.objects.iter().find(|o| o.id == "b").unwrap();
        let anchor = &b.anchors[0];
        let d = reproject_geometry_node(
            &Transform3x3 { m: b.transform },
            &Transform3x3 { m: a_base },
            &Transform3x3 { m: delta },
            LocalPoint {
                x: anchor.at.x.round() as i32,
                y: anchor.at.y.round() as i32,
            },
            i32::try_from(anchor.node_index).unwrap_or(i32::MAX),
            &b.geometry_d,
        )
        .expect("addressable, changed");
        assert_eq!(d, "M 0 0 L -664 224");
    }

    /// PRODUCTION WIRE-SERDE GUARD. The closure guard above builds `RenderObject`
    /// structs directly, BYPASSING serde. Production (`scene_feed.rs` load path,
    /// line ~109) instead PARSES the wire JSON the shell sends
    /// (`objectSceneToRenderObjectScene` -> camelCase `nodeIndex`/`geometryD`/
    /// `multiSelect`) and builds the bindings from THAT. A silent serde field
    /// mismatch would drop `anchors` -> empty reproject graph -> no live follow,
    /// invisible to the struct-based guard. This parses the EXACT wire shape, then
    /// asserts the anchor survived and the bindings still route the follower. Its
    /// scene-core twin (`anchor_serializes_with_camelcase_wire_keys`) pins that
    /// scene-core EMITS this casing, so the two cores meet on one wire.
    #[cfg(feature = "wgpu-probe")]
    #[test]
    fn anchored_follower_survives_wire_serde_round_trip() {
        use super::preview_reproject_followers;
        // The exact camelCase wire an anchored line B (node 1 bound to target A)
        // produces via the shell projection.
        let wire = r#"{
          "sceneId": "anchor-follow",
          "camera": { "x": 0.0, "y": 0.0, "zoom": 1.0 },
          "objects": [
            { "id": "a", "order": "a0",
              "transform": [[1,0,10],[0,1,20],[0,0,1]],
              "geometryD": "M 0 0 L 8 0 L 8 8 L 0 8 Z", "anchors": [] },
            { "id": "b", "order": "a1",
              "transform": [[1,0,100],[0,1,0],[0,0,1]],
              "geometryD": "M 0 0 L 64 0",
              "anchors": [ { "nodeIndex": 1, "target": "a", "at": { "x": 16.0, "y": 8.0 } } ] }
          ],
          "multiSelect": []
        }"#;
        let scene: RenderObjectScene =
            serde_json::from_str(wire).expect("wire parses into RenderObjectScene");
        // The anchor SURVIVED the parse — the silent-drop check that the struct guard
        // cannot make (a serde key mismatch would leave this empty).
        let b = scene.objects.iter().find(|o| o.id == "b").expect("b present");
        assert_eq!(b.anchors.len(), 1, "serde dropped `anchors` => empty reproject graph");
        assert_eq!(b.anchors[0].node_index, 1);
        assert_eq!(b.anchors[0].target, "a");
        // Bindings built from the PARSED scene (the scene_feed.rs:109 production path)
        // route B to follow A.
        let bindings = BindingGraph::build(&binding_nodes(&scene));
        let followers = preview_reproject_followers(&scene, &bindings, "a");
        assert_eq!(followers, vec![("b".to_string(), "a".to_string())]);
    }

    /// W3-G13 GROUPING GUARD: the closure dedupes by (follower, target) PAIR, so a
    /// follower anchored to TWO moved targets arrives twice. The grouping must fold
    /// BOTH targets' rewrites into ONE cumulative geometry per follower — RED if it
    /// restarts from the canonical path per pair (the second pair would stomp the
    /// first target's rewrite, exactly the per-pair patch bug).
    #[cfg(feature = "wgpu-probe")]
    #[test]
    fn follower_paired_with_two_moved_targets_accumulates_one_geometry() {
        use super::reprojected_follower_geometries;
        use crate::render_object::{RAnchor, RLocalPoint};
        use shape_scene_core::object::{reproject_geometry_node, LocalPoint, Transform3x3};

        let a = anchored_object("a", None, translate(10.0, 20.0), "M 0 0 L 8 0 L 8 8 L 0 8 Z", Vec::new());
        let b = anchored_object("b", None, translate(30.0, 40.0), "M 0 0 L 8 0 L 8 8 L 0 8 Z", Vec::new());
        let f = anchored_object(
            "f",
            None,
            translate(100.0, 0.0),
            "M 0 0 L 64 0",
            vec![
                RAnchor {
                    node_index: 0,
                    target: "a".to_string(),
                    at: RLocalPoint { x: 16.0, y: 8.0 },
                },
                RAnchor {
                    node_index: 1,
                    target: "b".to_string(),
                    at: RLocalPoint { x: 16.0, y: 8.0 },
                },
            ],
        );
        let scene = RenderObjectScene {
            scene_id: "multi-anchor".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects: vec![a, b, f],
            selection: None,
            multi_select: Vec::new(),
        };
        let delta = translate(5.0, 7.0);
        let pairs = vec![
            ("f".to_string(), "a".to_string()),
            ("f".to_string(), "b".to_string()),
        ];

        let grouped = reprojected_follower_geometries(&scene, Some(&delta), &pairs);
        assert_eq!(grouped.len(), 1, "one entry per follower, not one per pair");
        assert_eq!(grouped[0].0, "f");

        // The oracle: fold the two rewrites by hand through the same scene-core
        // math, each starting from the PREVIOUS pair's result.
        let f_obj = scene.objects.iter().find(|o| o.id == "f").unwrap();
        let mut expected = f_obj.geometry_d.clone();
        for (target_id, anchor_index) in [("a", 0usize), ("b", 1usize)] {
            let target = scene.objects.iter().find(|o| o.id == target_id).unwrap();
            let anchor = &f_obj.anchors[anchor_index];
            expected = reproject_geometry_node(
                &Transform3x3 { m: f_obj.transform },
                &Transform3x3 { m: target.transform },
                &Transform3x3 { m: delta },
                LocalPoint {
                    x: anchor.at.x.round() as i32,
                    y: anchor.at.y.round() as i32,
                },
                i32::try_from(anchor.node_index).unwrap_or(i32::MAX),
                &expected,
            )
            .expect("addressable, changed");
        }
        assert_eq!(grouped[0].1, expected, "BOTH nodes rewritten cumulatively");

        // A canonical-restart bug yields only the LAST pair's rewrite — node 0
        // back at its base. Pin that the cumulative result differs from it.
        let b_obj = scene.objects.iter().find(|o| o.id == "b").unwrap();
        let only_last = reproject_geometry_node(
            &Transform3x3 { m: f_obj.transform },
            &Transform3x3 { m: b_obj.transform },
            &Transform3x3 { m: delta },
            LocalPoint { x: 16, y: 8 },
            1,
            &f_obj.geometry_d,
        )
        .expect("addressable, changed");
        assert_ne!(grouped[0].1, only_last, "node 0's rewrite must survive pair 2");

        // Restore path (`delta = None`): each unique follower comes back ONCE with
        // its canonical geometry, so the snap-back patch also runs once.
        let restored = reprojected_follower_geometries(&scene, None, &pairs);
        assert_eq!(
            restored,
            vec![("f".to_string(), f_obj.geometry_d.clone())]
        );
    }
}
