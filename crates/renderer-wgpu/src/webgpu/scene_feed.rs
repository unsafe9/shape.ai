//! Scene load + feed + slot retention: the legacy scene path
//! (`load_scene`/`applyPatchBatch`/`apply_render_patch` with incremental
//! dirty/new/grow/compact/clear slot bookkeeping) and the object path
//! (`load_object_scene`/`draw_objects`). `target_arch = "wasm32"` gated.

use std::collections::HashMap;

use shape_renderer_core::model::{
    CameraState, RenderCard, RenderEdge, RenderGroup, RenderScenePatch, SceneSelection,
    SceneSnapshot,
};
use crate::object_pipeline::{ObjectPipeline, ObjectRenderer};
use shape_renderer_core::render_object::{RenderObject, RenderObjectScene};
use crate::serde_wasm;
use shape_scene_core::object::move_together::{BindingGraph, BindingNode};
use shape_renderer_core::text::TextBuildStats;
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
        // The multi-select set is shell-owned and not in the wire format; re-apply the
        // renderer-held set so the highlight survives reloads.
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


    /// Parse a [`RenderObjectScene`] and build + upload its geometry through an
    /// [`ObjectRenderer`] on this renderer's device/queue (the `ObjectPipeline` is
    /// built lazily against the live surface format on the first call). Returns the
    /// built draw counts; the live GPU PASS is [`Self::draw_objects`].
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
        // When a compatible renderer exists, DIFF the freshly-built plan and apply
        // targeted patches instead of reconstructing — O(changed objects), not
        // O(scene). A structural / unfittable / theme-divergent diff falls back to a
        // full `ObjectRenderer::new`. The diff base must be the persisted theme, so we
        // reconstruct on any divergence.
        let theme_matches = self
            .object_renderer
            .as_ref()
            .map(|r| r.theme().dark == self.object_theme.dark)
            .unwrap_or(false);
        let patched = if theme_matches {
            self.object_renderer
                .as_mut()
                .map(|renderer| renderer.apply_plan_diff(&self.queue, &scene))
                .filter(|stats| !stats.needs_rebuild)
        } else {
            None
        };
        if let Some(stats) = patched {
            self.object_patch_count += stats.patch_count;
        } else {
            // First feed / structural / theme-divergent / unfittable update: rebuild
            // in the PERSISTED theme so the dark canvas-bg survives the re-feed (the
            // bit is owned by the wrapper, not the per-scene renderer this throws away).
            let pipeline = self.object_pipeline.as_ref().unwrap();
            self.object_renderer = Some(ObjectRenderer::new(
                &self.device,
                &self.queue,
                pipeline,
                &scene,
                self.width as f32,
                self.height as f32,
                self.object_theme,
            ));
            self.object_rebuild_count += 1;
        }
        let renderer = self.object_renderer.as_ref().unwrap();
        let counts = ObjectSceneLoadResult {
            objects: scene.objects.len(),
            fill_indices: renderer.fill_index_count() as usize,
            stroke_vertices: renderer.stroke_vertex_count() as usize,
        };
        // Derive + retain each object's local-space region for hit-test / marquee.
        self.object_regions = derive_object_regions(&scene);
        // Mirror `multiSelect` onto the renderer so a later re-feed keeps the highlight.
        self.multi_select = scene.multi_select.clone();
        // Precompute the move-together propagation graph ONCE per feed so each drag
        // preview is O(closure), not O(scene).
        self.object_bindings = BindingGraph::build(&binding_nodes(&scene));
        // The re-feed rebuilt every baked geometry, so no live chord deform survives.
        self.preview_deformed.clear();
        self.endpoint_preview = None;
        self.object_scene = Some(scene);
        serde_wasm(counts)
    }

    /// A thin wrapper over [`Self::render_frame`] (the single live frame driver),
    /// since the object pass is recorded there against the same acquired surface
    /// texture — a separate acquire here would double-acquire the swapchain. The
    /// client RAF loop calls `render_frame` directly.
    #[wasm_bindgen(js_name = drawObjects)]
    pub fn draw_objects(&mut self) -> Result<(), JsValue> {
        self.render_frame()?;
        Ok(())
    }

    /// Zero-rebake drag: push ONLY the affected objects' instance matrices to the GPU.
    /// `matrix_json` is a row-major `[[f64;3];3]` CUMULATIVE world-space DELTA. The
    /// canonical scene / regions are NOT mutated — `delta * base` is written straight
    /// to each instance buffer (no re-tessellation), and the RAF loop draws it next
    /// tick. No-op if the scene is unloaded or the id is absent.
    ///
    /// The affected set is the SameDelta closure of the bindings graph (the dragged
    /// id, or every `multi_select` member, plus their descendants), each against its
    /// own base — O(closure) writes.
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
        // The closure also yields `reproject_followers` — objects whose anchored
        // geometry must reproject through a moved target LIVE. A follower is NOT
        // uniformly transformed, so the instance-matrix preview cannot express it;
        // instead each follower's geometry is rewritten and re-expanded in place.
        let followers = preview_reproject_followers(scene, &self.object_bindings, id);
        let follower_patches = self.reexpand_reprojected_followers(scene, Some(&delta), &followers);
        // Route each moved member through the endpoint decision table: pure-translate
        // / closed-class / legacy keeps the 0-rebake matrix write; an open-class member
        // under a non-translate delta (or with a pinned endpoint) chord-deforms (its
        // matrix stays at BASE, its geometry rides the reexpand+patch path).
        let moved_ids: Vec<String> = write_set.iter().map(|(id, _)| id.clone()).collect();
        let mut matrix_writes: Vec<(String, [[f64; 3]; 3])> = Vec::new();
        let mut deform_ids: Vec<String> = Vec::new();
        let mut patch_pairs: Vec<(String, String)> = Vec::new();
        for (target, base) in write_set {
            match route_preview_member(scene, &moved_ids, &target, &delta) {
                PreviewMemberRoute::Deform { geometry_d } => {
                    deform_ids.push(target.clone());
                    patch_pairs.push((target, geometry_d));
                }
                route => {
                    // A member leaving the deform route mid-gesture snaps its geometry
                    // back to canonical; one never deformed adds nothing, so the
                    // pure-translate hot path stays patch-free.
                    if self.preview_deformed.contains(&target) {
                        if let Some(object) = scene.objects.iter().find(|o| o.id == target) {
                            patch_pairs.push((target.clone(), object.geometry_d.clone()));
                        }
                    }
                    if matches!(route, PreviewMemberRoute::Matrix) {
                        matrix_writes.push((target, base));
                    }
                }
            }
        }
        let member_patches = self.reexpand_geometries(scene, patch_pairs);
        for moved in &moved_ids {
            self.preview_deformed.remove(moved);
        }
        for deformed in deform_ids {
            self.preview_deformed.insert(deformed);
        }
        if let Some(renderer) = self.object_renderer.as_mut() {
            for (target, base) in &matrix_writes {
                renderer.set_preview_transform(&self.queue, target, &delta, base);
            }
            for (patched_id, rebuilt) in follower_patches.iter().chain(member_patches.iter()) {
                renderer.patch_follower_geometry(&self.queue, patched_id, rebuilt);
            }
        }
        Ok(())
    }

    /// Revert the affected objects' instance matrices to their canonical baked
    /// transforms (`delta = identity`), dropping the live preview — the SAME SameDelta
    /// closure the preview wrote, not just the picked id. No-op when unloaded or absent.
    #[wasm_bindgen(js_name = clearObjectPreview)]
    pub fn clear_object_preview(&mut self, id: &str) -> Result<(), JsValue> {
        // Every geometry a live chord deform patched snaps back to canonical with the
        // same restore pass the followers get.
        let deformed = std::mem::take(&mut self.preview_deformed);
        self.endpoint_preview = None;
        let Some(scene) = self.object_scene.as_ref() else {
            return Ok(());
        };
        let write_set = preview_write_set(scene, &self.object_bindings, id);
        // Restore each follower's CANONICAL geometry so a cancelled drag reverts the
        // live reproject too, not just the same-delta matrices.
        let followers = preview_reproject_followers(scene, &self.object_bindings, id);
        let restored = self.reexpand_reprojected_followers(scene, None, &followers);
        let restore_pairs: Vec<(String, String)> = deformed
            .iter()
            .filter_map(|deformed_id| {
                scene
                    .objects
                    .iter()
                    .find(|o| &o.id == deformed_id)
                    .map(|o| (deformed_id.clone(), o.geometry_d.clone()))
            })
            .collect();
        let member_restores = self.reexpand_geometries(scene, restore_pairs);
        if let Some(renderer) = self.object_renderer.as_mut() {
            for (target, base) in &write_set {
                renderer.clear_preview_transform(&self.queue, target, base);
            }
            for (patched_id, rebuilt) in restored.iter().chain(member_restores.iter()) {
                renderer.patch_follower_geometry(&self.queue, patched_id, rebuilt);
            }
        }
        Ok(())
    }

    /// Endpoint-drag LIVE path: chord-deform ONE open-class object so its dragged
    /// endpoint (`node_index`, geometry PAIR space: 0 | last) lands on the live
    /// pointer WORLD position, then re-expand + in-place patch that single object. The
    /// deform is the SAME `endpoint_release_ops` the release commit runs, so live and
    /// committed bytes cannot drift. No-op when unloaded / unknown / not an endpoint.
    #[wasm_bindgen(js_name = setObjectEndpointPreview)]
    pub fn set_object_endpoint_preview(
        &mut self,
        id: &str,
        node_index: i32,
        world_x: f64,
        world_y: f64,
    ) -> Result<(), JsValue> {
        let Some(scene) = self.object_scene.as_ref() else {
            return Ok(());
        };
        let Some(new_d) = endpoint_preview_geometry(scene, id, node_index, (world_x, world_y))
        else {
            return Ok(());
        };
        let patches = self.reexpand_geometries(scene, vec![(id.to_string(), new_d)]);
        self.preview_deformed.insert(id.to_string());
        self.endpoint_preview = Some((
            id.to_string(),
            node_index,
            shape_renderer_core::model::WorldPoint { x: world_x, y: world_y },
        ));
        if let Some(renderer) = self.object_renderer.as_mut() {
            for (patched_id, rebuilt) in &patches {
                renderer.patch_follower_geometry(&self.queue, patched_id, rebuilt);
            }
        }
        Ok(())
    }

    /// Symmetric clear: re-expand the object's CANONICAL geometry back in, dropping
    /// the live endpoint deform. No-op when no endpoint preview patched this id.
    #[wasm_bindgen(js_name = clearObjectEndpointPreview)]
    pub fn clear_object_endpoint_preview(&mut self, id: &str) -> Result<(), JsValue> {
        self.endpoint_preview = None;
        if !self.preview_deformed.remove(id) {
            return Ok(());
        }
        let Some(scene) = self.object_scene.as_ref() else {
            return Ok(());
        };
        let Some(object) = scene.objects.iter().find(|o| o.id == id) else {
            return Ok(());
        };
        let patches =
            self.reexpand_geometries(scene, vec![(id.to_string(), object.geometry_d.clone())]);
        if let Some(renderer) = self.object_renderer.as_mut() {
            for (patched_id, rebuilt) in &patches {
                renderer.patch_follower_geometry(&self.queue, patched_id, rebuilt);
            }
        }
        Ok(())
    }

    /// Re-expand each follower's geometry for an in-place GPU patch. With
    /// `Some(delta)`, every node anchored onto a moved target is rewritten to track
    /// the target's PREVIEWED transform; with `None`, the CANONICAL geometry is
    /// re-expanded untouched (the restore). Pairs are grouped by follower
    /// ([`reprojected_follower_geometries`]) so two moved targets accumulate into ONE
    /// geometry + ONE patch.
    fn reexpand_reprojected_followers(
        &self,
        scene: &RenderObjectScene,
        delta: Option<&[[f64; 3]; 3]>,
        followers: &[(String, String)],
    ) -> Vec<(String, crate::object_pipeline::FollowerReexpand)> {
        self.reexpand_geometries(scene, reprojected_follower_geometries(scene, delta, followers))
    }

    /// The shared re-expand consumer: each `(id, geometry_d)` pair is the scene object
    /// rebuilt with that geometry through `reexpand_single_object` (live theme +
    /// camera), ready for an in-place `patch_follower_geometry`. The one patch pipeline
    /// for the follower reproject, member chord deforms, and endpoint preview.
    fn reexpand_geometries(
        &self,
        scene: &RenderObjectScene,
        pairs: Vec<(String, String)>,
    ) -> Vec<(String, crate::object_pipeline::FollowerReexpand)> {
        let Some(theme) = self.object_renderer.as_ref().map(|r| r.theme()) else {
            return Vec::new();
        };
        pairs
            .into_iter()
            .filter_map(|(id, geometry_d)| {
                let object = scene.objects.iter().find(|o| o.id == id)?;
                let mut reshaped = object.clone();
                reshaped.geometry_d = geometry_d;
                let rebuilt = crate::object_pipeline::reexpand_single_object(
                    &reshaped,
                    theme,
                    scene.camera.clone(),
                );
                Some((id, rebuilt))
            })
            .collect()
    }

}

/// The binding projection the move-together graph is built from — each object's id,
/// parent, and anchor targets. O(objects) String clones, paid once per feed.
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

/// The propagation ROOTS for a drag of `dragged`: every `multi_select` member if
/// `dragged` is one (deduped, canonical order), else just the dragged id. Pure.
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

/// The `(id, base)` GPU write-set for a drag of `dragged`: the SameDelta closure
/// from [`preview_roots`], each id paired with its canonical base. The pure half of
/// [`ShapeWebGpuRenderer::set_object_preview_transform`] — same delta against each
/// own base. Ids absent from `scene.objects` are dropped.
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

/// The `(follower_id, target_id)` reproject pairs for a drag of `dragged` — the
/// Reproject half of the closure [`preview_write_set`] consumes the SameDelta half
/// of. Each pair names a follower whose anchored geometry must reproject through a
/// moved `target`. Pure; the wasm32 path turns each into an in-place vertex patch.
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

/// GROUP the `(follower, target)` reproject pairs by FOLLOWER and fold each paired
/// target's node rewrites into ONE cumulative `geometry_d` per follower (the closure
/// dedupes by PAIR, so a follower anchored to two moved targets arrives twice;
/// restarting per pair would stomp the first rewrite). One entry per follower in
/// first-appearance order; with `delta = None` each comes back with its CANONICAL
/// geometry.
///
/// An open-class follower whose anchors all bind ENDPOINTS deforms as ONE chord
/// ([`open_follower_chord_d`]) — the same route the commit's `anchor_follow_ops`
/// takes, so the live and committed bytes stay equivalent. Everything else keeps the
/// node-splice fold via `reproject_geometry_node`. Pure.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn reprojected_follower_geometries(
    scene: &RenderObjectScene,
    delta: Option<&[[f64; 3]; 3]>,
    followers: &[(String, String)],
) -> Vec<(String, String)> {
    // Group by follower, first-appearance order.
    let mut grouped: Vec<(&str, Vec<&str>)> = Vec::new();
    for (follower_id, target_id) in followers {
        match grouped.iter_mut().find(|(id, _)| *id == follower_id.as_str()) {
            Some((_, targets)) => targets.push(target_id.as_str()),
            None => grouped.push((follower_id.as_str(), vec![target_id.as_str()])),
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for (follower_id, target_ids) in grouped {
        let Some(follower) = scene.objects.iter().find(|o| o.id == follower_id) else {
            continue;
        };
        let Some(delta) = delta else {
            out.push((follower_id.to_string(), follower.geometry_d.clone()));
            continue;
        };
        let d = open_follower_chord_d(scene, follower, &target_ids, delta)
            .unwrap_or_else(|| spliced_follower_d(scene, follower, &target_ids, delta));
        out.push((follower_id.to_string(), d));
    }
    out
}

/// The chord follow for ONE open-class endpoint-anchored follower, mirroring the
/// `endpoint_deform` branch of `anchor_follow_ops` with the same scene-core
/// functions: each anchored ENDPOINT moves to its reprojected position, the
/// un-anchored endpoint pins, and the silhouette rides the chord. `None` when the
/// follower is not an open-class endpoint case (the caller keeps the legacy splice).
#[cfg(feature = "wgpu-probe")]
fn open_follower_chord_d(
    scene: &RenderObjectScene,
    follower: &RenderObject,
    target_ids: &[&str],
    delta: &[[f64; 3]; 3],
) -> Option<String> {
    use shape_scene_core::object::{deform_open_path, is_open_class_d, local_nodes};
    let base = &follower.geometry_d;
    let pairs = local_nodes(base);
    if pairs.len() < 2 || !is_open_class_d(base) {
        return None;
    }
    let last = pairs.len() - 1;
    if follower.anchors.iter().any(|a| a.node_index != 0 && a.node_index != last) {
        return None;
    }
    let mut new_start = pairs[0];
    let mut new_end = pairs[last];
    for anchor in &follower.anchors {
        if !target_ids.contains(&anchor.target.as_str()) {
            continue;
        }
        // Splice the endpoint on the BASE path, then read the pair back. A `None`
        // splice is a no-op (the endpoint already sits there, which the seeds carry).
        let point = reprojected_node_pair(scene, follower, anchor, delta);
        if anchor.node_index == 0 {
            new_start = point.unwrap_or(new_start);
        } else {
            new_end = point.unwrap_or(new_end);
        }
    }
    deform_open_path(base, new_start, new_end)
}

/// One anchored node's reprojected pair-space position under `delta`, read back from
/// a `reproject_geometry_node` splice of the CANONICAL path (byte-identical to the
/// commit). `None` when the rewrite is a no-op.
#[cfg(feature = "wgpu-probe")]
fn reprojected_node_pair(
    scene: &RenderObjectScene,
    follower: &RenderObject,
    anchor: &shape_renderer_core::render_object::RAnchor,
    delta: &[[f64; 3]; 3],
) -> Option<(f64, f64)> {
    use shape_scene_core::object::{local_nodes, reproject_geometry_node, LocalPoint, Transform3x3};
    let target = scene.objects.iter().find(|o| o.id == anchor.target)?;
    let rewritten = reproject_geometry_node(
        &Transform3x3 { m: follower.transform },
        &Transform3x3 { m: target.transform },
        &Transform3x3 { m: *delta },
        LocalPoint {
            x: anchor.at.x.round() as i32,
            y: anchor.at.y.round() as i32,
        },
        i32::try_from(anchor.node_index).unwrap_or(i32::MAX),
        &follower.geometry_d,
    )?;
    local_nodes(&rewritten).get(anchor.node_index).copied()
}

/// The node-splice fold: every moved-target anchor's node rewrites cumulatively
/// through `reproject_geometry_node` (closed-class / multi-subpath / interior-node).
#[cfg(feature = "wgpu-probe")]
fn spliced_follower_d(
    scene: &RenderObjectScene,
    follower: &RenderObject,
    target_ids: &[&str],
    delta: &[[f64; 3]; 3],
) -> String {
    use shape_scene_core::object::{reproject_geometry_node, LocalPoint, Transform3x3};
    let mut d = follower.geometry_d.clone();
    for target_id in target_ids {
        let Some(target) = scene.objects.iter().find(|o| &o.id == target_id) else {
            continue;
        };
        for anchor in &follower.anchors {
            if anchor.target != *target_id {
                continue;
            }
            if let Some(rewritten) = reproject_geometry_node(
                &Transform3x3 { m: follower.transform },
                &Transform3x3 { m: target.transform },
                &Transform3x3 { m: *delta },
                LocalPoint {
                    x: anchor.at.x.round() as i32,
                    y: anchor.at.y.round() as i32,
                },
                i32::try_from(anchor.node_index).unwrap_or(i32::MAX),
                &d,
            ) {
                d = rewritten;
            }
        }
    }
    d
}

/// How ONE moved member previews live — the renderer-side mirror of scene-core's
/// `cascade::push_member_op` decision table.
#[cfg(feature = "wgpu-probe")]
pub(crate) enum PreviewMemberRoute {
    /// The 0-rebake instance-matrix write (pure-translate, closed-class, legacy).
    Matrix,
    /// Both endpoints pinned by unmoved anchor targets — write nothing.
    Pinned,
    /// Chord-deform the geometry to this path-string and patch it; the instance
    /// matrix stays at BASE.
    Deform { geometry_d: String },
}

/// Route one SameDelta-closure member of a live preview (`moved_ids` = the closure).
/// Mirrors `push_member_op` branch for branch so the live frame and commit ops route
/// identically. Pure.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn route_preview_member(
    scene: &RenderObjectScene,
    moved_ids: &[String],
    id: &str,
    delta: &[[f64; 3]; 3],
) -> PreviewMemberRoute {
    use shape_scene_core::object::{
        deform_open_path, is_open_class_d, is_pure_translate, open_endpoint_pins,
        route_open_endpoints, Anchor, EndpointRoute, LocalPoint, Transform3x3,
    };
    let Some(object) = scene.objects.iter().find(|o| o.id == id) else {
        return PreviewMemberRoute::Matrix;
    };
    let delta_t = Transform3x3 { m: *delta };
    // Fast path: an unanchored pure-translate member keeps the 0-rebake write without
    // parsing geometry (the common drag).
    if object.anchors.is_empty() && is_pure_translate(&delta_t) {
        return PreviewMemberRoute::Matrix;
    }
    let d = &object.geometry_d;
    if !is_open_class_d(d) {
        return PreviewMemberRoute::Matrix;
    }
    let anchors: Vec<Anchor> = object
        .anchors
        .iter()
        .map(|a| Anchor {
            node_index: i32::try_from(a.node_index).unwrap_or(i32::MAX),
            target: a.target.clone(),
            at: LocalPoint {
                x: a.at.x.round() as i32,
                y: a.at.y.round() as i32,
            },
        })
        .collect();
    let target_moved = |target: &str| moved_ids.iter().any(|moved| moved == target);
    let Some((start_pinned, end_pinned)) = open_endpoint_pins(d, &anchors, target_moved) else {
        // An interior-node anchor rides the whole transform.
        return PreviewMemberRoute::Matrix;
    };
    match route_open_endpoints(
        d,
        &Transform3x3 { m: object.transform },
        &delta_t,
        start_pinned,
        end_pinned,
    ) {
        Some(EndpointRoute::Pinned) => PreviewMemberRoute::Pinned,
        Some(EndpointRoute::Deform { new_start, new_end }) => {
            match deform_open_path(d, new_start, new_end) {
                Some(geometry_d) => PreviewMemberRoute::Deform { geometry_d },
                None => PreviewMemberRoute::Matrix,
            }
        }
        Some(EndpointRoute::Translate) | None => PreviewMemberRoute::Matrix,
    }
}

/// The live endpoint-drag geometry for `id` — the EditGeometry path-string
/// `endpoint_release_ops` (the release COMMIT function, run here against a one-object
/// scene with no snap) authors for moving `node_index` to the WORLD point. A no-op
/// rewrite returns the canonical geometry. `None` when the id is unknown or not an
/// open-class endpoint. Pure.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn endpoint_preview_geometry(
    scene: &RenderObjectScene,
    id: &str,
    node_index: i32,
    world: (f64, f64),
) -> Option<String> {
    use shape_scene_core::object::{
        endpoint_release_ops, is_open_class_d, local_nodes, FillRule, Geometry, Object, ObjectOp,
        ObjectScene, ObjectSelection, Transform3x3,
    };
    let object = scene.objects.iter().find(|o| o.id == id)?;
    let d = &object.geometry_d;
    if !is_open_class_d(d) {
        return None;
    }
    let pairs = local_nodes(d);
    if pairs.len() < 2 {
        return None;
    }
    let last = i32::try_from(pairs.len() - 1).ok()?;
    if node_index != 0 && node_index != last {
        return None;
    }
    let mut core = Object::new(
        &object.id,
        &object.order,
        Geometry {
            path_string: d.clone(),
            fill_rule: FillRule::EvenOdd,
            subpaths: Vec::new(),
        },
    );
    core.transform = Transform3x3 { m: object.transform };
    let core_scene = ObjectScene {
        scene_version: 1,
        objects: vec![core],
        tags: Vec::new(),
        selection: ObjectSelection::Canvas,
        updated_at: String::new(),
    };
    let deformed = endpoint_release_ops(&core_scene, id, node_index, world, None)
        .into_iter()
        .find_map(|op| match op {
            ObjectOp::EditGeometry { geometry, .. } => Some(geometry.path_string),
            _ => None,
        });
    Some(deformed.unwrap_or_else(|| d.clone()))
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
    use shape_renderer_core::model::CameraState;
    use shape_renderer_core::render_object::{RenderObject, RenderObjectScene};
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
        anchors: Vec<shape_renderer_core::render_object::RAnchor>,
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

    /// The renderer's live-preview closure (`preview_reproject_followers` +
    /// `preview_write_set`) must drive the SAME pinned vector scene-core's
    /// `reproject_matches_cross_core_vector` pins (`"M 0 0 L -664 224"`), via the same
    /// `BindingGraph` + `reproject_geometry_node` call path.
    #[cfg(feature = "wgpu-probe")]
    #[test]
    fn anchor_follower_closure_agrees_with_scene_core_vector() {
        use super::preview_reproject_followers;
        use shape_renderer_core::render_object::{RAnchor, RLocalPoint};
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

    /// Production parses the camelCase wire (`nodeIndex`/`geometryD`/`multiSelect`),
    /// not structs; a silent serde field mismatch would drop `anchors` -> empty
    /// reproject graph -> no live follow, invisible to the struct guard above. Parses
    /// the EXACT wire shape and asserts the anchor survived + still routes the follower.
    #[cfg(feature = "wgpu-probe")]
    #[test]
    fn anchored_follower_survives_wire_serde_round_trip() {
        use super::preview_reproject_followers;
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
        let b = scene.objects.iter().find(|o| o.id == "b").expect("b present");
        assert_eq!(b.anchors.len(), 1, "serde dropped `anchors` => empty reproject graph");
        assert_eq!(b.anchors[0].node_index, 1);
        assert_eq!(b.anchors[0].target, "a");
        let bindings = BindingGraph::build(&binding_nodes(&scene));
        let followers = preview_reproject_followers(&scene, &bindings, "a");
        assert_eq!(followers, vec![("b".to_string(), "a".to_string())]);
    }

    /// A follower anchored to TWO moved targets arrives twice (the closure dedupes by
    /// PAIR); the grouping must fold BOTH rewrites into ONE cumulative geometry, not
    /// restart from canonical per pair (which would stomp the first rewrite).
    #[cfg(feature = "wgpu-probe")]
    #[test]
    fn follower_paired_with_two_moved_targets_accumulates_one_geometry() {
        use super::reprojected_follower_geometries;
        use shape_renderer_core::render_object::{RAnchor, RLocalPoint};
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

    // ----- live endpoint routing ------------------

    use super::{endpoint_preview_geometry, route_preview_member, PreviewMemberRoute};
    use shape_renderer_core::render_object::{RAnchor, RLocalPoint, RStroke, RStrokeCap, RStrokeJoin};

    fn feed_scene(objects: Vec<RenderObject>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "v3-endpoints".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects,
            selection: None,
            multi_select: Vec::new(),
        }
    }

    fn rot90() -> [[f64; 3]; 3] {
        [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
    }

    fn endpoint_anchor(node_index: usize, target: &str) -> RAnchor {
        RAnchor {
            node_index,
            target: target.to_string(),
            at: RLocalPoint { x: 0.0, y: 0.0 },
        }
    }

    /// Each row mirrors scene-core's commit `push_member_op` decision table.
    #[test]
    fn route_preview_member_follows_the_endpoint_decision_table() {
        let ids = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let open = |anchors: Vec<RAnchor>| {
            anchored_object("l", None, translate(0.0, 0.0), "M 0 0 L 320 0", anchors)
        };

        // Closed-class: any delta keeps the instance-matrix write.
        let rect = anchored_object("l", None, translate(0.0, 0.0), "M 0 0 L 8 0 L 8 8 L 0 8 Z", Vec::new());
        let scene = feed_scene(vec![rect]);
        assert!(matches!(
            route_preview_member(&scene, &ids(&["l"]), "l", &rot90()),
            PreviewMemberRoute::Matrix
        ));

        // Unanchored open member, pure translate: 0-rebake matrix.
        let scene = feed_scene(vec![open(Vec::new())]);
        assert!(matches!(
            route_preview_member(&scene, &ids(&["l"]), "l", &translate(10.0, 5.0)),
            PreviewMemberRoute::Matrix
        ));

        // Unanchored open member, NON-translate delta: a 90° rotation deforms the
        // (0,0)->(40,0)px chord to (0,0)->(0,40)px.
        let PreviewMemberRoute::Deform { geometry_d } =
            route_preview_member(&scene, &ids(&["l"]), "l", &rot90())
        else {
            panic!("a rotated open member must chord-deform");
        };
        assert_eq!(geometry_d, "M 0 0 L 0 320");

        // Pinned start: the free end takes the body delta, the anchored end (target
        // not in the moved set) holds its glue point.
        let scene = feed_scene(vec![open(vec![endpoint_anchor(0, "t")])]);
        let PreviewMemberRoute::Deform { geometry_d } =
            route_preview_member(&scene, &ids(&["l"]), "l", &translate(10.0, 0.0))
        else {
            panic!("a one-anchor body drag must rubber-band the free end");
        };
        assert_eq!(geometry_d, "M 0 0 L 400 0");

        // The anchor's target moved in the SAME batch: the delta cancels, so the pure
        // translate keeps the SetTransform fast path.
        assert!(matches!(
            route_preview_member(&scene, &ids(&["l", "t"]), "l", &translate(10.0, 0.0)),
            PreviewMemberRoute::Matrix
        ));

        // Both endpoints pinned by unmoved targets: nothing moves.
        let scene = feed_scene(vec![open(vec![
            endpoint_anchor(0, "t"),
            endpoint_anchor(1, "u"),
        ])]);
        assert!(matches!(
            route_preview_member(&scene, &ids(&["l"]), "l", &translate(10.0, 0.0)),
            PreviewMemberRoute::Pinned
        ));

        // An interior-node anchor rides the whole transform.
        let wire = anchored_object(
            "l",
            None,
            translate(0.0, 0.0),
            "M 0 0 L 320 0 L 640 0",
            vec![endpoint_anchor(1, "t")],
        );
        let scene = feed_scene(vec![wire]);
        assert!(matches!(
            route_preview_member(&scene, &ids(&["l"]), "l", &translate(10.0, 0.0)),
            PreviewMemberRoute::Matrix
        ));
    }

    /// On a group rotate, the open member's LIVE deform bytes must equal the
    /// `edit-geometry` the commit cascade authors, and re-expanding that geometry must
    /// equal a FULL rebake of the committed scene.
    #[test]
    fn open_member_group_rotate_live_matches_scene_core_commit_and_rebake() {
        use crate::object_pipeline::{
            build_scene_geometry_themed, follower_patch_plan, reexpand_single_object,
        };
        use shape_renderer_core::object_theme::Theme;
        use shape_scene_core::object::{
            cascade_multi_transform_ops, FillRule, Geometry, Object, ObjectOp, ObjectScene,
            ObjectSelection, Transform3x3,
        };

        let stroke = RStroke {
            paint: shape_renderer_core::render_object::RPaint::Solid {
                color: "#00ff00".to_string(),
            },
            width: 4.0,
            opacity: 1.0,
            dash: Vec::new(),
            cap: RStrokeCap::Butt,
            join: RStrokeJoin::Miter,
        };
        let rect = anchored_object("r", None, translate(0.0, 0.0), "M 0 0 L 160 0 L 160 160 L 0 160 Z", Vec::new());
        let mut line = anchored_object("l", None, translate(100.0, 0.0), "M 0 0 L 320 0", Vec::new());
        line.stroke = Some(stroke);
        let scene = feed_scene(vec![rect, line]);
        let delta = rot90();

        // LIVE: the multi-rotate routes the open member to a chord deform.
        let moved = vec!["r".to_string(), "l".to_string()];
        let PreviewMemberRoute::Deform { geometry_d: live_d } =
            route_preview_member(&scene, &moved, "l", &delta)
        else {
            panic!("the open member of a rotated group must chord-deform");
        };

        // COMMIT: the scene-core cascade for the same roots + delta.
        let core_object = |o: &RenderObject| {
            let mut core = Object::new(
                &o.id,
                &o.order,
                Geometry {
                    path_string: o.geometry_d.clone(),
                    fill_rule: FillRule::EvenOdd,
                    subpaths: Vec::new(),
                },
            );
            core.transform = Transform3x3 { m: o.transform };
            core
        };
        let core_scene = ObjectScene {
            scene_version: 1,
            objects: scene.objects.iter().map(core_object).collect(),
            tags: Vec::new(),
            selection: ObjectSelection::Canvas,
            updated_at: String::new(),
        };
        let ops = cascade_multi_transform_ops(&core_scene, &moved, &Transform3x3 { m: delta });
        // The closed member keeps SetTransform; the open member commits ONE
        // edit-geometry and NO set-transform.
        assert!(ops
            .iter()
            .any(|op| matches!(op, ObjectOp::SetTransform { id, .. } if id == "r")));
        assert!(!ops
            .iter()
            .any(|op| matches!(op, ObjectOp::SetTransform { id, .. } if id == "l")));
        let commit_d = ops
            .iter()
            .find_map(|op| match op {
                ObjectOp::EditGeometry { id, geometry } if id == "l" => {
                    Some(geometry.path_string.clone())
                }
                _ => None,
            })
            .expect("the open member commits an edit-geometry");
        assert_eq!(live_d, commit_d, "live deform bytes == committed deform bytes");

        // REBAKE PARITY: the live patch (reexpand of live_d over the canonical
        // ranges) equals the full rebake of the committed scene.
        let canonical_build = build_scene_geometry_themed(&scene, Theme::light());
        let mut deformed_scene = scene.clone();
        deformed_scene.objects[1].geometry_d = commit_d;
        let full = build_scene_geometry_themed(&deformed_scene, Theme::light());
        let rebuilt = reexpand_single_object(
            &deformed_scene.objects[1],
            Theme::light(),
            scene.camera.clone(),
        );
        let plan = follower_patch_plan(&canonical_build.draws[1], &rebuilt)
            .expect("a node-count-preserving deform is size-safe");
        assert_eq!(
            plan.stroke_vertex_byte_offset,
            canonical_build.draws[1].stroke_range.start as u64
                * std::mem::size_of::<crate::object_pipeline::StrokeVertex>() as u64
        );
        let full_stroke = &full.stroke_vertices[full.draws[1].stroke_range.start as usize
            ..full.draws[1].stroke_range.end as usize];
        assert!(!full_stroke.is_empty(), "the line has a stroke ribbon");
        assert_eq!(rebuilt.stroke_vertices, full_stroke, "live patch == full rebake");
        let canonical_stroke = &canonical_build.stroke_vertices
            [canonical_build.draws[1].stroke_range.start as usize
                ..canonical_build.draws[1].stroke_range.end as usize];
        assert_ne!(
            rebuilt.stroke_vertices, canonical_stroke,
            "the rotate really moved the ribbon"
        );
    }

    /// An open-class follower whose anchored ENDPOINT follows a moved target deforms
    /// the WHOLE chord (the interior node rides along), and the bytes must equal
    /// `anchor_follow_ops` for the same move.
    #[test]
    fn open_follower_live_chord_matches_anchor_follow_commit() {
        use super::reprojected_follower_geometries;
        use shape_scene_core::object::{
            anchor_follow_ops, Anchor, FillRule, Geometry, LocalPoint, Object, ObjectOp,
            ObjectScene, ObjectSelection, Transform3x3,
        };

        // Target at (200,0); 3-node follower with its END anchored to the target's
        // origin; the target moves +160px x.
        let target = anchored_object("t", None, translate(200.0, 0.0), "M 0 0 L 0 0", Vec::new());
        let follower = anchored_object(
            "f",
            None,
            translate(0.0, 0.0),
            "M 0 0 L 800 0 L 1600 0",
            vec![endpoint_anchor(2, "t")],
        );
        let scene = feed_scene(vec![target, follower]);
        let delta = translate(160.0, 0.0);

        let live = reprojected_follower_geometries(
            &scene,
            Some(&delta),
            &[("f".to_string(), "t".to_string())],
        );
        assert_eq!(live.len(), 1);
        assert_eq!(
            live[0],
            ("f".to_string(), "M 0 0 L 1440 0 L 2880 0".to_string()),
            "the interior node rides the chord (the old splice left it at 800 — the spike)"
        );

        // The commit oracle: anchor_follow_ops for the same move authors the SAME d.
        let core_object = |o: &RenderObject, anchors: Vec<Anchor>| {
            let mut core = Object::new(
                &o.id,
                &o.order,
                Geometry {
                    path_string: o.geometry_d.clone(),
                    fill_rule: FillRule::EvenOdd,
                    subpaths: Vec::new(),
                },
            );
            core.transform = Transform3x3 { m: o.transform };
            core.anchors = anchors;
            core
        };
        let core_scene = ObjectScene {
            scene_version: 1,
            objects: vec![
                core_object(&scene.objects[0], Vec::new()),
                core_object(
                    &scene.objects[1],
                    vec![Anchor {
                        node_index: 2,
                        target: "t".to_string(),
                        at: LocalPoint { x: 0, y: 0 },
                    }],
                ),
            ],
            tags: Vec::new(),
            selection: ObjectSelection::Canvas,
            updated_at: String::new(),
        };
        let ops = anchor_follow_ops(
            &core_scene,
            &[ObjectOp::SetTransform {
                id: "t".to_string(),
                transform: Transform3x3::translate(360.0, 0.0),
            }],
        );
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected the follow edit-geometry");
        };
        assert_eq!(id, "f");
        assert_eq!(live[0].1, geometry.path_string, "live follower bytes == commit bytes");

        // An INTERIOR-node anchor keeps the splice: only the bound node rewrites, the
        // endpoints stay.
        let mut interior_scene = scene.clone();
        interior_scene.objects[1].anchors = vec![endpoint_anchor(1, "t")];
        let live_interior = reprojected_follower_geometries(
            &interior_scene,
            Some(&delta),
            &[("f".to_string(), "t".to_string())],
        );
        assert_eq!(live_interior[0].1, "M 0 0 L 2880 0 L 1600 0");
    }

    /// The endpoint-drag live geometry comes from the SAME `endpoint_release_ops` the
    /// release runs; non-endpoint targets have no surface.
    #[test]
    fn endpoint_preview_geometry_matches_endpoint_release_ops() {
        let edge = anchored_object("e", None, translate(0.0, 0.0), "M 0 0 L 800 0", Vec::new());
        let rect = anchored_object("r", None, translate(0.0, 0.0), "M 0 0 L 8 0 L 8 8 L 0 8 Z", Vec::new());
        let wire = anchored_object("w", None, translate(0.0, 0.0), "M 0 0 L 80 0 L 160 0", Vec::new());
        let scene = feed_scene(vec![edge, rect, wire]);

        // Node 1 of the 100px edge to world (150,10).
        assert_eq!(
            endpoint_preview_geometry(&scene, "e", 1, (150.0, 10.0)),
            Some("M 0 0 L 1200 80".to_string())
        );
        // Dragged back onto the start point: a no-op rewrite restores canonical.
        assert_eq!(
            endpoint_preview_geometry(&scene, "e", 1, (100.0, 0.0)),
            Some("M 0 0 L 800 0".to_string())
        );
        // Closed-class, interior nodes, and unknown ids have no endpoint surface.
        assert!(endpoint_preview_geometry(&scene, "r", 1, (0.0, 0.0)).is_none());
        assert!(endpoint_preview_geometry(&scene, "w", 1, (50.0, 0.0)).is_none());
        assert!(endpoint_preview_geometry(&scene, "ghost", 0, (0.0, 0.0)).is_none());
    }

    /// Re-expanding the endpoint-preview geometry fills the canonical baked ranges
    /// exactly (size-safe patch) and equals a FULL rebake of the deformed scene.
    #[test]
    fn endpoint_preview_patch_matches_a_full_rebake() {
        use crate::object_pipeline::{
            build_scene_geometry_themed, follower_patch_plan, reexpand_single_object,
        };
        use shape_renderer_core::object_theme::Theme;

        let mut edge = anchored_object("e", None, translate(0.0, 0.0), "M 0 0 L 800 0", Vec::new());
        edge.stroke = Some(RStroke {
            paint: shape_renderer_core::render_object::RPaint::Solid {
                color: "#00ff00".to_string(),
            },
            width: 4.0,
            opacity: 1.0,
            dash: Vec::new(),
            cap: RStrokeCap::Butt,
            join: RStrokeJoin::Miter,
        });
        let scene = feed_scene(vec![edge]);
        let canonical_build = build_scene_geometry_themed(&scene, Theme::light());

        let deformed_d =
            endpoint_preview_geometry(&scene, "e", 1, (150.0, 10.0)).expect("deform");
        let mut deformed_scene = scene.clone();
        deformed_scene.objects[0].geometry_d = deformed_d;
        let rebuilt = reexpand_single_object(
            &deformed_scene.objects[0],
            Theme::light(),
            scene.camera.clone(),
        );
        let plan = follower_patch_plan(&canonical_build.draws[0], &rebuilt)
            .expect("an endpoint deform preserves the node count: size-safe");
        assert_eq!(plan.fill_index_rebase, canonical_build.draws[0].fill_vertex_range.start);

        let full = build_scene_geometry_themed(&deformed_scene, Theme::light());
        let full_stroke = &full.stroke_vertices[full.draws[0].stroke_range.start as usize
            ..full.draws[0].stroke_range.end as usize];
        assert!(!full_stroke.is_empty());
        assert_eq!(rebuilt.stroke_vertices, full_stroke, "endpoint patch == full rebake");
    }
}
