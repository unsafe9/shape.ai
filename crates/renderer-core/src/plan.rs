//! FramePlan IR — the platform-neutral, diffable draw-plan contract between
//! `renderer-core` (what to draw) and `renderer-wgpu` (GPU submission).
//!
//! A [`FramePlan`] is an ordered list of [`PlanEntry`]s (one per object, scene
//! order), each geometry payload addressed by a stable [`ResourceHandle`] keyed
//! `(object id, revision)`. The revision hashes ONLY the geometry-affecting inputs
//! (path, LOD/zoom bucket, stroke/dash/cap/join + fill-kind + text) — never the
//! transform, never the resolved paint COLOR. So a transform-only or style-only
//! change keeps the same handle, and [`diff_plans`] emits a small [`PlanPatch`]
//! instead of re-sending the mesh. Wraps the merged `SceneGeometry` build (which
//! is the geometry store, addressed by per-object `ObjectDraw` ranges) rather than
//! replacing it. Pure CPU, no `wgpu`/`web_sys`.

use crate::model::CameraState;
use crate::object_pipeline::{
    build_scene_geometry_themed, build_scene_geometry_themed_with_text, FillInstance,
    GlyphUvProvider, ObjectDraw, SceneGeometry, ShadowInstance, StrokeInstance, TextInstance,
};
use crate::object_theme::Theme;
use crate::render_object::{RenderObject, RenderObjectScene, RStroke, RText};

/// The fixed draw-pass order, back-to-front (logical z-order). The shadow is
/// composited under fill via the blur pass in `renderer-wgpu`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawPass {
    Shadow,
    Fill,
    Stroke,
    Text,
}

pub fn pass_order() -> [DrawPass; 4] {
    [DrawPass::Shadow, DrawPass::Fill, DrawPass::Stroke, DrawPass::Text]
}

/// A stable address for one object's baked geometry, keyed `(object, revision)`.
/// Same handle => byte-identical geometry (patch instance attributes only); a
/// changed revision => re-send that object's mesh. See [`geometry_revision`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceHandle {
    pub object: String,
    pub revision: u64,
}

/// Per-pass instance attributes: the 3x3 matrix columns (shared by every pass)
/// plus each pass's resolved solid color. Text is matrix-only (color is per-glyph).
/// These are what a transform or style diff updates against an unchanged handle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanInstance {
    pub fill: FillInstance,
    pub stroke: StrokeInstance,
    pub shadow: ShadowInstance,
    pub text: TextInstance,
}

/// One object's contribution to the frame: its geometry handle, per-pass instance
/// attributes, and draw ranges into the shared geometry store.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanEntry {
    pub handle: ResourceHandle,
    pub instance: PlanInstance,
    pub draw: ObjectDraw,
}

/// The diffable draw plan: ordered per-object entries plus the merged geometry
/// store their ranges address. Built by [`build_frame_plan`], diffed by [`diff_plans`].
#[derive(Clone, Debug)]
pub struct FramePlan {
    /// Per-object entries in scene order, index-aligned with the merged buffers.
    pub entries: Vec<PlanEntry>,
    /// The merged buffers; re-sent wholesale only on a [`PlanPatch::Rebuild`], else
    /// a per-object geometry edit patches one entry's slices in place.
    pub geometry: SceneGeometry,
}

impl FramePlan {
    pub fn handles(&self) -> Vec<ResourceHandle> {
        self.entries.iter().map(|e| e.handle.clone()).collect()
    }
}

/// Which instance color slot a [`PlanPatch::StyleUpdate`] rewrites (a theme flip
/// touches all token-backed slots).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleSlot {
    Fill,
    Stroke,
    Shadow,
}

/// A minimal, targeted update against an existing plan. Each variant patches the
/// smallest GPU resource that changed; a change none can express degrades to
/// [`PlanPatch::Rebuild`].
#[derive(Clone, Debug, PartialEq)]
pub enum PlanPatch {
    /// Transform-only: rewrite the shared matrix columns; no mesh, no color. The
    /// drag/pan/move hot path.
    TransformUpdate {
        object: String,
        index: usize,
        columns: [[f32; 3]; 3],
    },
    /// Style-only: rewrite one color slot of one pass's instance; no mesh, no matrix.
    StyleUpdate {
        object: String,
        index: usize,
        slot: StyleSlot,
        color: [f32; 4],
    },
    /// Geometry edit: re-send ONLY this object's slices against its handle's ranges.
    /// Carries the new entry so the consumer can patch in place when the ranges
    /// still fit, else fall back to a rebuild.
    GeometryUpdate {
        object: String,
        index: usize,
        entry: Box<PlanEntry>,
    },
    /// Structural change (objects added/removed/reordered): re-send the whole plan.
    Rebuild,
}

/// Either a set of targeted patches OR a full rebuild. Rebuild is kept distinct
/// (not a patch in the list) so "did the diff avoid a rebuild?" is a clean branch.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanDiff {
    /// Targeted patches to apply in order; empty means the plans are identical.
    Patches(Vec<PlanPatch>),
    Rebuild,
}

/// Build the [`FramePlan`] for `scene` under `theme`. Runs the merged geometry
/// build once, then records each object's handle + instance attributes + ranges.
pub fn build_frame_plan(scene: &RenderObjectScene, theme: Theme) -> FramePlan {
    frame_plan_from(scene, build_scene_geometry_themed(scene, theme))
}

/// As [`build_frame_plan`], with an injected real text shaper + glyph-UV provider
/// (the GPU cutover supplies them) so glyph quads carry real atlas slots. The plan
/// machinery (handles/instances/ranges) is identical — only the text geometry differs.
pub fn build_frame_plan_with_text(
    scene: &RenderObjectScene,
    theme: Theme,
    measure: &dyn Fn(char, f32) -> f32,
    glyph_uv: &GlyphUvProvider<'_>,
) -> FramePlan {
    frame_plan_from(
        scene,
        build_scene_geometry_themed_with_text(scene, theme, measure, glyph_uv),
    )
}

/// Record each object's handle + instance attributes + ranges over an already-built
/// `geometry` (index-aligned with `scene.objects`).
fn frame_plan_from(scene: &RenderObjectScene, geometry: SceneGeometry) -> FramePlan {
    let mut entries = Vec::with_capacity(geometry.draws.len());
    for (i, draw) in geometry.draws.iter().enumerate() {
        // `draws`/`*_instances`/`scene.objects` are all index-aligned in scene order,
        // so entry `i` reads object `i`'s revision inputs and the build's instance `i`.
        let object = &scene.objects[i];
        let handle = ResourceHandle {
            object: draw.id.clone(),
            revision: geometry_revision(object, &scene.camera),
        };
        let instance = PlanInstance {
            fill: geometry.fill_instances[i],
            stroke: geometry.stroke_instances[i],
            shadow: geometry.shadow_instances[i],
            text: geometry.text_instances[i],
        };
        entries.push(PlanEntry {
            handle,
            instance,
            draw: draw.clone(),
        });
    }
    FramePlan { entries, geometry }
}

/// Diff two plans. Same object set in the same order => targeted [`PlanPatch`]es
/// (an identical entry contributes nothing); otherwise [`PlanDiff::Rebuild`]. A
/// transform-only or style-only change can never yield a rebuild here.
pub fn diff_plans(old: &FramePlan, new: &FramePlan) -> PlanDiff {
    if old.entries.len() != new.entries.len() {
        return PlanDiff::Rebuild;
    }
    if old
        .entries
        .iter()
        .zip(&new.entries)
        .any(|(o, n)| o.handle.object != n.handle.object)
    {
        return PlanDiff::Rebuild;
    }

    let mut patches = Vec::new();
    for (index, (old_entry, new_entry)) in old.entries.iter().zip(&new.entries).enumerate() {
        diff_entry(index, old_entry, new_entry, &mut patches);
    }
    PlanDiff::Patches(patches)
}

/// Append the targeted patches taking `old_entry` to `new_entry`. A changed handle
/// revision is a geometry edit and supersedes the instance diff (the re-sent mesh
/// carries the new instance); otherwise emit transform/style updates.
fn diff_entry(
    index: usize,
    old_entry: &PlanEntry,
    new_entry: &PlanEntry,
    patches: &mut Vec<PlanPatch>,
) {
    let object = new_entry.handle.object.clone();

    if old_entry.handle.revision != new_entry.handle.revision {
        patches.push(PlanPatch::GeometryUpdate {
            object,
            index,
            entry: Box::new(new_entry.clone()),
        });
        return;
    }

    let old_i = &old_entry.instance;
    let new_i = &new_entry.instance;

    // Matrix columns are shared across all passes; the fill instance's columns are
    // the canonical matrix (the build writes the same columns into every pass).
    if matrix_columns(&new_i.fill) != matrix_columns(&old_i.fill) {
        patches.push(PlanPatch::TransformUpdate {
            object: object.clone(),
            index,
            columns: matrix_columns(&new_i.fill),
        });
    }

    if new_i.fill.fill != old_i.fill.fill {
        patches.push(PlanPatch::StyleUpdate {
            object: object.clone(),
            index,
            slot: StyleSlot::Fill,
            color: new_i.fill.fill,
        });
    }
    if new_i.stroke.stroke != old_i.stroke.stroke {
        patches.push(PlanPatch::StyleUpdate {
            object: object.clone(),
            index,
            slot: StyleSlot::Stroke,
            color: new_i.stroke.stroke,
        });
    }
    if new_i.shadow.shadow != old_i.shadow.shadow {
        patches.push(PlanPatch::StyleUpdate {
            object,
            index,
            slot: StyleSlot::Shadow,
            color: new_i.shadow.shadow,
        });
    }
}

/// The three matrix columns shared by every pass's instance (the build writes
/// identical `m0,m1,m2` into fill/stroke/shadow/text).
fn matrix_columns(fill: &FillInstance) -> [[f32; 3]; 3] {
    [fill.m0, fill.m1, fill.m2]
}

/// A content hash of every input that changes object `obj`'s tessellated geometry:
/// path, zoom LOD bucket, stroke params (width/dash/cap/join + presence), fill
/// presence, text runs/align. Deliberately EXCLUDES the transform and resolved paint
/// COLORS (instance-level) so a move or recolor keeps the SAME revision — the
/// load-bearing property the whole IR rests on.
pub fn geometry_revision(obj: &RenderObject, camera: &CameraState) -> u64 {
    let mut h = Fnv::new();
    h.bytes(obj.geometry_d.as_bytes());
    // Zoom bucket, not raw zoom: the LOD flattener buckets zoom, so the mesh is
    // identical within a bucket.
    h.u64(crate::curve_lod::zoom_bucket(camera.zoom) as u64);
    hash_stroke(&mut h, obj.stroke.as_ref());
    // Fill presence gates `skip_fill`; the paint kind does not change geometry.
    h.u8(u8::from(obj.fill.is_some()));
    hash_text(&mut h, obj.text.as_ref());
    h.finish()
}

/// Fold the stroke's geometry-affecting params into `h` (presence, width, dash,
/// cap, join). Paint is NOT hashed — stroke COLOR is instance-level.
fn hash_stroke(h: &mut Fnv, stroke: Option<&RStroke>) {
    match stroke {
        None => h.u8(0),
        Some(s) => {
            h.u8(1);
            h.f64(s.width);
            h.u64(s.dash.len() as u64);
            for d in &s.dash {
                h.f64(*d);
            }
            h.u8(stroke_cap_tag(s));
            h.u8(stroke_join_tag(s));
        }
    }
}

fn stroke_cap_tag(s: &RStroke) -> u8 {
    use crate::render_object::RStrokeCap::*;
    match s.cap {
        Butt => 0,
        Round => 1,
        Square => 2,
    }
}

fn stroke_join_tag(s: &RStroke) -> u8 {
    use crate::render_object::RStrokeJoin::*;
    match s.join {
        Miter => 0,
        Bevel => 1,
        Round => 2,
    }
}

/// Fold the text's geometry-affecting content into `h`. Run COLOR is per-glyph
/// geometry (not an instance attribute), so a color-only text edit is a geometry edit.
fn hash_text(h: &mut Fnv, text: Option<&RText>) {
    match text {
        None => h.u8(0),
        Some(t) => {
            h.u8(1);
            h.u64(t.runs.len() as u64);
            for run in &t.runs {
                h.bytes(run.text.as_bytes());
                h.bytes(run.color.as_bytes());
                h.f64(run.size);
                h.u8(u8::from(run.bold));
                h.u8(u8::from(run.italic));
                h.bytes(run.font.as_bytes());
            }
            h.u8(text_align_tag(t));
            h.u8(text_valign_tag(t));
        }
    }
}

fn text_align_tag(t: &RText) -> u8 {
    use crate::render_object::RTextAlign::*;
    match t.align {
        Start => 0,
        Center => 1,
        End => 2,
        Justify => 3,
    }
}

fn text_valign_tag(t: &RText) -> u8 {
    use crate::render_object::RTextValign::*;
    match t.valign {
        Top => 0,
        Middle => 1,
        Bottom => 2,
    }
}

/// FNV-1a 64-bit, deterministic and pure (no RNG/seed/time), so the same object
/// hashes to the same revision across runs and targets. A content fingerprint only;
/// a collision degrades a real geometry edit to "no re-send", which the consumer's
/// range-fit guard still catches.
struct Fnv(u64);

impl Fnv {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Fnv(Self::OFFSET)
    }

    fn u8(&mut self, b: u8) {
        self.0 ^= u64::from(b);
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.u8(b);
        }
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn f64(&mut self, v: f64) {
        // Bit pattern so the fold is exact and total (incl. signed-zero / NaN).
        self.bytes(&v.to_bits().to_le_bytes());
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hit_test_object::{mat3_mul, translate_3x3};
    use crate::model::CameraState;
    use crate::object_pipeline::preview_instance_columns;
    use crate::render_object::{
        RFill, RPaint, RStroke, RStrokeCap, RStrokeJoin, RText, RTextAlign, RTextRun, RTextValign,
    };

    fn identity() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    fn rect_object(id: &str) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            geometry_d: "M0 0 L800 0 L800 800 L0 800 Z".to_string(),
            fill: Some(RFill {
                paint: RPaint::Solid { color: "#ff0000".to_string() },
                opacity: 1.0,
            }),
            stroke: Some(RStroke {
                paint: RPaint::Solid { color: "#00ff00".to_string() },
                width: 4.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            }),
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    /// A rect with token fill + stroke, so its colors re-resolve on a theme flip.
    fn token_rect(id: &str) -> RenderObject {
        let mut obj = rect_object(id);
        obj.fill = Some(RFill {
            paint: RPaint::Token { name: "default-fill".to_string() },
            opacity: 1.0,
        });
        obj.stroke = Some(RStroke {
            paint: RPaint::Token { name: "default-stroke".to_string() },
            width: 4.0,
            opacity: 1.0,
            dash: Vec::new(),
            cap: RStrokeCap::Butt,
            join: RStrokeJoin::Miter,
        });
        obj
    }

    fn text_rect(id: &str) -> RenderObject {
        let mut obj = rect_object(id);
        obj.geometry_d = "M0 0 L1600 0 L1600 800 L0 800 Z".to_string();
        obj.text = Some(RText {
            runs: vec![RTextRun {
                text: "AB".to_string(),
                color: "#111111".to_string(),
                size: 128.0,
                bold: false,
                italic: false,
                font: String::new(),
            }],
            align: RTextAlign::Start,
            valign: RTextValign::Top,
        });
        obj
    }

    fn scene_with(objects: Vec<RenderObject>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "plan".to_string(),
            camera: CameraState { x: 0.0, y: 0.0, zoom: 1.0 },
            objects,
            selection: None,
            multi_select: Vec::new(),
        }
    }

    #[test]
    fn frame_plan_snapshots_one_entry_per_object_in_order() {
        let scene = scene_with(vec![rect_object("a"), text_rect("b"), rect_object("c")]);
        let plan = build_frame_plan(&scene, Theme::light());

        assert_eq!(plan.entries.len(), 3, "one entry per object");
        let ids: Vec<&str> = plan.entries.iter().map(|e| e.handle.object.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "entries in scene order");

        // Each handle revision equals a direct recompute of its geometry revision.
        for (entry, obj) in plan.entries.iter().zip(&scene.objects) {
            assert_eq!(entry.handle.revision, geometry_revision(obj, &scene.camera));
            assert_eq!(entry.draw.id, obj.id, "draw range record is the same object");
        }

        assert_eq!(plan.geometry.draws.len(), 3);
        assert_eq!(
            pass_order(),
            [DrawPass::Shadow, DrawPass::Fill, DrawPass::Stroke, DrawPass::Text]
        );
        assert!(!plan.entries[1].draw.text_range.is_empty(), "text object has glyphs");
        assert!(plan.entries[0].draw.text_range.is_empty(), "plain rect has none");
    }

    #[test]
    fn rebuilding_the_same_scene_diffs_to_no_patches() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")]);
        let a = build_frame_plan(&scene, Theme::light());
        let b = build_frame_plan(&scene, Theme::light());
        assert_eq!(diff_plans(&a, &b), PlanDiff::Patches(Vec::new()));
    }

    #[test]
    fn transform_only_change_yields_a_transform_patch_not_a_rebuild() {
        let base_scene = scene_with(vec![rect_object("a"), rect_object("b")]);
        let old = build_frame_plan(&base_scene, Theme::light());

        let delta = translate_3x3(40.0, -25.0);
        let mut moved_scene = base_scene.clone();
        moved_scene.objects[0].transform = mat3_mul(&delta, &base_scene.objects[0].transform);
        let new = build_frame_plan(&moved_scene, Theme::light());

        assert_eq!(
            old.entries[0].handle, new.entries[0].handle,
            "transform-only move keeps the same geometry handle (no re-tessellation)"
        );

        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("transform-only change must diff to patches, not a rebuild");
        };
        assert_eq!(patches.len(), 1, "only the moved object is patched");
        let expected_columns = {
            let (m0, m1, m2) =
                preview_instance_columns(&delta, &base_scene.objects[0].transform);
            [m0, m1, m2]
        };
        assert_eq!(
            patches[0],
            PlanPatch::TransformUpdate {
                object: "a".to_string(),
                index: 0,
                columns: expected_columns,
            },
            "the patch is a transform update with the composed columns"
        );
        assert!(!patches.iter().any(|p| matches!(
            p,
            PlanPatch::StyleUpdate { .. } | PlanPatch::GeometryUpdate { .. } | PlanPatch::Rebuild
        )));
    }

    #[test]
    fn transform_only_patch_touches_no_geometry_buffers() {
        let base_scene = scene_with(vec![rect_object("a"), rect_object("b")]);
        let old = build_frame_plan(&base_scene, Theme::light());

        let delta = translate_3x3(7.0, 3.0);
        let mut moved = base_scene.clone();
        moved.objects[0].transform = mat3_mul(&delta, &base_scene.objects[0].transform);
        let new = build_frame_plan(&moved, Theme::light());

        // Mesh buffers are byte-identical across a pure move; only the per-object
        // instance matrix columns differ.
        assert_eq!(old.geometry.fill.vertices, new.geometry.fill.vertices);
        assert_eq!(old.geometry.fill.indices, new.geometry.fill.indices);
        assert_eq!(old.geometry.stroke_vertices, new.geometry.stroke_vertices);
        assert_eq!(old.geometry.shadow_vertices, new.geometry.shadow_vertices);
        assert_eq!(old.geometry.text_vertices, new.geometry.text_vertices);

        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("a move must not rebuild");
        };
        assert!(
            !patches.iter().any(|p| matches!(p, PlanPatch::GeometryUpdate { .. })),
            "a transform-only diff carries no geometry payload"
        );
    }

    #[test]
    fn style_only_theme_flip_yields_style_patches_not_a_rebuild() {
        let scene = scene_with(vec![token_rect("a"), token_rect("b")]);
        let light = build_frame_plan(&scene, Theme::light());
        let dark = build_frame_plan(&scene, Theme::dark());

        for (l, d) in light.entries.iter().zip(&dark.entries) {
            assert_eq!(l.handle, d.handle, "theme flip keeps the same geometry handle");
        }

        let PlanDiff::Patches(patches) = diff_plans(&light, &dark) else {
            panic!("a theme flip must diff to patches, not a rebuild");
        };
        assert!(!patches.is_empty(), "a token theme flip changes colors");
        for patch in &patches {
            assert!(
                matches!(patch, PlanPatch::StyleUpdate { .. }),
                "a recolor produces only style updates, got {patch:?}"
            );
        }
        let fill_updates = patches
            .iter()
            .filter(|p| matches!(p, PlanPatch::StyleUpdate { slot: StyleSlot::Fill, .. }))
            .count();
        let shadow_updates = patches
            .iter()
            .filter(|p| matches!(p, PlanPatch::StyleUpdate { slot: StyleSlot::Shadow, .. }))
            .count();
        assert_eq!(fill_updates, 2, "both token fills re-resolve");
        assert_eq!(shadow_updates, 2, "both shadows re-resolve (shadow is always a token)");
    }

    #[test]
    fn raw_hex_recolor_is_a_style_patch_not_geometry() {
        let scene = scene_with(vec![rect_object("a")]);
        let old = build_frame_plan(&scene, Theme::light());

        let mut recolored = scene.clone();
        recolored.objects[0].fill = Some(RFill {
            paint: RPaint::Solid { color: "#0000ff".to_string() },
            opacity: 1.0,
        });
        let new = build_frame_plan(&recolored, Theme::light());

        assert_eq!(old.entries[0].handle, new.entries[0].handle, "recolor keeps the handle");
        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("recolor must not rebuild");
        };
        assert_eq!(
            patches,
            vec![PlanPatch::StyleUpdate {
                object: "a".to_string(),
                index: 0,
                slot: StyleSlot::Fill,
                color: [0.0, 0.0, 1.0, 1.0],
            }]
        );
    }

    #[test]
    fn geometry_edit_patches_only_the_touched_handle() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b"), rect_object("c")]);
        let old = build_frame_plan(&scene, Theme::light());

        let mut edited = scene.clone();
        edited.objects[1].geometry_d = "M0 0 L900 0 L900 900 L0 900 Z".to_string();
        let new = build_frame_plan(&edited, Theme::light());

        assert_eq!(old.entries[0].handle, new.entries[0].handle, "a unchanged");
        assert_ne!(old.entries[1].handle, new.entries[1].handle, "b re-tessellated");
        assert_eq!(old.entries[2].handle, new.entries[2].handle, "c unchanged");

        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("a single geometry edit must not rebuild the whole plan");
        };
        assert_eq!(patches.len(), 1, "only the edited object is patched");
        match &patches[0] {
            PlanPatch::GeometryUpdate { object, index, entry } => {
                assert_eq!(object, "b");
                assert_eq!(*index, 1);
                assert_eq!(entry.handle, new.entries[1].handle);
                assert_eq!(entry.draw, new.entries[1].draw, "carries b's new draw ranges");
            }
            other => panic!("expected a GeometryUpdate for b, got {other:?}"),
        }
    }

    #[test]
    fn geometry_edit_supersedes_a_simultaneous_transform_change() {
        let scene = scene_with(vec![rect_object("a")]);
        let old = build_frame_plan(&scene, Theme::light());

        let mut edited = scene.clone();
        edited.objects[0].geometry_d = "M0 0 L500 0 L500 500 L0 500 Z".to_string();
        edited.objects[0].transform = translate_3x3(10.0, 10.0);
        let new = build_frame_plan(&edited, Theme::light());

        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("must not rebuild");
        };
        assert_eq!(patches.len(), 1, "one patch: the geometry update");
        assert!(matches!(patches[0], PlanPatch::GeometryUpdate { .. }));
    }

    #[test]
    fn structural_change_yields_rebuild() {
        let two = scene_with(vec![rect_object("a"), rect_object("b")]);
        let plan_two = build_frame_plan(&two, Theme::light());

        let three = scene_with(vec![rect_object("a"), rect_object("b"), rect_object("c")]);
        assert_eq!(
            diff_plans(&plan_two, &build_frame_plan(&three, Theme::light())),
            PlanDiff::Rebuild,
            "adding an object rebuilds"
        );

        let one = scene_with(vec![rect_object("a")]);
        assert_eq!(
            diff_plans(&plan_two, &build_frame_plan(&one, Theme::light())),
            PlanDiff::Rebuild,
            "removing an object rebuilds"
        );

        // Reorder rebuilds because a targeted patch addresses by index.
        let swapped = scene_with(vec![rect_object("b"), rect_object("a")]);
        assert_eq!(
            diff_plans(&plan_two, &build_frame_plan(&swapped, Theme::light())),
            PlanDiff::Rebuild,
            "reordering objects rebuilds"
        );
    }

    #[test]
    fn revision_excludes_transform_and_color_includes_geometry_inputs() {
        let camera = CameraState { x: 0.0, y: 0.0, zoom: 1.0 };
        let base = rect_object("o");
        let base_rev = geometry_revision(&base, &camera);

        let mut moved = base.clone();
        moved.transform = translate_3x3(99.0, -99.0);
        assert_eq!(geometry_revision(&moved, &camera), base_rev, "transform excluded");

        let mut recolored = base.clone();
        recolored.fill = Some(RFill {
            paint: RPaint::Solid { color: "#123456".to_string() },
            opacity: 1.0,
        });
        assert_eq!(geometry_revision(&recolored, &camera), base_rev, "fill color excluded");

        let mut restroked = base.clone();
        if let Some(s) = restroked.stroke.as_mut() {
            s.paint = RPaint::Solid { color: "#abcdef".to_string() };
        }
        assert_eq!(geometry_revision(&restroked, &camera), base_rev, "stroke color excluded");

        let mut repathed = base.clone();
        repathed.geometry_d = "M0 0 L400 0 L400 400 L0 400 Z".to_string();
        assert_ne!(geometry_revision(&repathed, &camera), base_rev, "path included");

        let mut rewidened = base.clone();
        if let Some(s) = rewidened.stroke.as_mut() {
            s.width = 12.0;
        }
        assert_ne!(geometry_revision(&rewidened, &camera), base_rev, "stroke width included");

        let mut redashed = base.clone();
        if let Some(s) = redashed.stroke.as_mut() {
            s.dash = vec![4.0, 4.0];
        }
        assert_ne!(geometry_revision(&redashed, &camera), base_rev, "dash included");
    }

    #[test]
    fn revision_tracks_the_zoom_lod_bucket() {
        let obj = rect_object("o");
        let z1 = CameraState { x: 0.0, y: 0.0, zoom: 1.0 };
        let same_bucket = CameraState {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        };
        assert_eq!(
            crate::curve_lod::zoom_bucket(z1.zoom),
            crate::curve_lod::zoom_bucket(same_bucket.zoom)
        );
        assert_eq!(geometry_revision(&obj, &z1), geometry_revision(&obj, &same_bucket));

        let mut other = z1.clone();
        for &z in &[0.1f64, 0.25, 0.5, 2.0, 4.0, 8.0, 16.0] {
            if crate::curve_lod::zoom_bucket(z) != crate::curve_lod::zoom_bucket(z1.zoom) {
                other.zoom = z;
                break;
            }
        }
        assert_ne!(
            crate::curve_lod::zoom_bucket(other.zoom),
            crate::curve_lod::zoom_bucket(z1.zoom),
            "found a different LOD bucket"
        );
        assert_ne!(
            geometry_revision(&obj, &other),
            geometry_revision(&obj, &z1),
            "crossing an LOD bucket changes the revision"
        );
    }
}
