//! FramePlan IR — the platform-neutral, DIFFABLE draw-plan contract between
//! `renderer-core` (what to draw) and `renderer-wgpu` (GPU submission).
//!
//! The IR is plain data. A [`FramePlan`] is an ordered list of [`PlanEntry`]s
//! (one per object, in scene order) split across the fixed [`pass_order`] of
//! draw passes, where every per-object geometry payload is addressed by a stable
//! [`ResourceHandle`] keyed `(object id, revision)`. The revision is a content
//! hash of ONLY the geometry-affecting inputs (path, LOD/zoom bucket, the
//! stroke/dash/cap/join + fill-kind + text that change tessellation) — never the
//! transform and never the resolved paint COLOR. So a transform-only or
//! style-only change keeps the SAME handle, and [`diff_plans`] can prove the
//! geometry is unchanged and emit a small [`PlanPatch`] (a transform / color
//! write against the existing handle) instead of re-sending the mesh.
//!
//! ## Why this wraps `SceneGeometry` rather than replacing it
//!
//! [`crate::object_pipeline::build_scene_geometry`] already merges every object's
//! fill/stroke/shadow/text into shared buffers with per-object ranges, and that
//! build is the proven, heavily-tested core. The IR is a thin formalization on
//! top: [`build_frame_plan`] runs that build once and records, per object, the
//! handle + the instance attributes (matrix columns + per-pass color) + the focus
//! flag. The merged geometry buffers ARE the geometry store, addressed by the
//! per-object [`crate::object_pipeline::ObjectDraw`] ranges the build already
//! produces; the handle is the cache key those ranges hang off.
//!
//! Everything here is pure CPU and host-testable with no GPU and no `wgpu`/`web_sys`.

use crate::model::CameraState;
use crate::object_pipeline::{
    build_scene_geometry_themed, FillInstance, ObjectDraw, SceneGeometry, ShadowInstance,
    StrokeInstance, TextInstance,
};
use crate::object_theme::Theme;
use crate::render_object::{RenderObject, RenderObjectScene, RStroke, RText};

/// The fixed draw-pass order the object pipeline records, back-to-front: the
/// drop-shadow underlay, then fill, then the stroke ribbon, then text glyphs.
/// The IR pins this order so the consumer never has to re-derive it and a diff
/// can reason pass-by-pass. (The shadow is composited UNDER fill via the blur
/// pass in `renderer-wgpu`; the order here is the logical z-order, which is what
/// a diff cares about.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawPass {
    Shadow,
    Fill,
    Stroke,
    Text,
}

/// The canonical pass order. A `FramePlan` is interpreted pass-by-pass in this
/// order; each [`PlanEntry`] contributes its per-pass instance to each pass.
pub fn pass_order() -> [DrawPass; 4] {
    [DrawPass::Shadow, DrawPass::Fill, DrawPass::Stroke, DrawPass::Text]
}

/// A stable address for one object's baked geometry, keyed `(object, revision)`.
///
/// The `revision` is a content hash of every input that changes the object's
/// TESSELLATED geometry (its fill mesh, stroke ribbon, shadow silhouette, glyph
/// quads) — see [`geometry_revision`]. Two plans whose entry for the same id
/// carry the SAME handle have byte-identical geometry, so the consumer can keep
/// the geometry it already uploaded and only patch instance attributes. A changed
/// revision means the mesh must be re-sent for that one object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceHandle {
    pub object: String,
    pub revision: u64,
}

/// The per-pass instance attributes for one object: the 3x3 projective matrix
/// columns (shared by every pass) plus each pass's resolved solid color. Text
/// carries no instance color (color is per-glyph in the geometry), so it is
/// matrix-only. These are the small, frequently-changing values a transform or
/// style diff updates against an unchanged handle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanInstance {
    pub fill: FillInstance,
    pub stroke: StrokeInstance,
    pub shadow: ShadowInstance,
    pub text: TextInstance,
}

/// One object's contribution to the frame: its geometry handle, its per-pass
/// instance attributes, the per-object draw ranges into the shared geometry
/// store, and whether it draws a focus ring. Token names back the theme-flip
/// re-resolve path; they are diff inputs only for completeness (a token rename
/// without a color change is still a style update).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanEntry {
    pub handle: ResourceHandle,
    pub instance: PlanInstance,
    pub draw: ObjectDraw,
}

/// The platform-neutral, diffable draw plan: the ordered per-object entries and
/// the merged geometry store ([`SceneGeometry`]) the entries' ranges address.
/// Produced by [`build_frame_plan`]; diffed by [`diff_plans`].
#[derive(Clone, Debug)]
pub struct FramePlan {
    /// Per-object entries in scene (draw) order, index-aligned with the merged
    /// instance buffers inside `geometry`.
    pub entries: Vec<PlanEntry>,
    /// The merged fill/stroke/shadow/text buffers — the geometry store the
    /// entries' [`ObjectDraw`] ranges index into. Re-sent wholesale only on a
    /// [`PlanPatch::Rebuild`]; a per-object geometry edit patches one entry's
    /// slices in place.
    pub geometry: SceneGeometry,
}

impl FramePlan {
    /// The ordered handle list (one per entry), for cache-key bookkeeping in the
    /// consumer.
    pub fn handles(&self) -> Vec<ResourceHandle> {
        self.entries.iter().map(|e| e.handle.clone()).collect()
    }
}

/// Which instance color slot a [`PlanPatch::StyleUpdate`] rewrites. The matrix
/// region is shared across passes and rewritten by a transform update; a style
/// update touches only the color slot of one pass's instance (or all token-backed
/// slots on a theme flip — the consumer re-resolves those, but the diff still
/// localizes the write to the touched entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleSlot {
    Fill,
    Stroke,
    Shadow,
}

/// A minimal, targeted update against an EXISTING plan — the whole point of the
/// IR. Each variant patches the smallest GPU resource that changed; none re-sends
/// unchanged geometry. A change the diff cannot express as one of these targeted
/// patches degrades to [`PlanPatch::Rebuild`].
#[derive(Clone, Debug, PartialEq)]
pub enum PlanPatch {
    /// Transform-only: rewrite object `object`'s shared matrix columns in every
    /// per-pass instance buffer. No mesh touched, no color touched. This is the
    /// drag/pan/move hot path.
    TransformUpdate {
        object: String,
        index: usize,
        columns: [[f32; 3]; 3],
    },
    /// Style-only: rewrite one color slot of one pass's instance for `object`.
    /// No mesh touched, no matrix touched.
    StyleUpdate {
        object: String,
        index: usize,
        slot: StyleSlot,
        color: [f32; 4],
    },
    /// Geometry edit: object `object`'s revision changed, so its baked mesh must
    /// be re-sent — but ONLY this object's slices, against its existing handle's
    /// ranges. Carries the new entry (new handle + instance + draw ranges) so the
    /// consumer can patch in place when the ranges still fit, or fall back to a
    /// rebuild when they don't.
    GeometryUpdate {
        object: String,
        index: usize,
        entry: Box<PlanEntry>,
    },
    /// The structure changed in a way no targeted patch covers — objects added,
    /// removed, or reordered. The consumer re-sends the whole plan.
    Rebuild,
}

/// The result of diffing two plans: either a set of targeted patches OR a single
/// full rebuild. Keeping rebuild as a distinct shape (not a patch in the list)
/// makes "did the diff avoid a rebuild?" a one-line assertion in the golden tests
/// and a clean branch in the consumer.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanDiff {
    /// Apply these targeted patches in order against the existing plan. Empty
    /// means the two plans are identical (nothing to upload).
    Patches(Vec<PlanPatch>),
    /// The plans differ structurally; re-send `new` wholesale.
    Rebuild,
}

/// Build the [`FramePlan`] for `scene` under `theme` and `camera`'s zoom (no
/// device needed). Runs [`build_scene_geometry_themed`] once — the single
/// tessellation entry — then records each object's handle + instance attributes
/// + draw ranges. The geometry store IS the merged build output; the entries are
/// the diffable surface over it.
pub fn build_frame_plan(scene: &RenderObjectScene, theme: Theme) -> FramePlan {
    let geometry = build_scene_geometry_themed(scene, theme);
    let mut entries = Vec::with_capacity(geometry.draws.len());
    for (i, draw) in geometry.draws.iter().enumerate() {
        // The build loop produces `draws`, `fill_instances`, ... all index-aligned
        // in scene order (pinned by `draws_index_aligns_with_both_instance_buffers`),
        // and `scene.objects[i].id == draws[i].id`. So entry `i` reads object `i`'s
        // geometry-revision inputs and the build's instance `i`.
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

/// Diff two plans into a [`PlanDiff`]. The contract:
///
/// - Same object set in the same order (every entry's `handle.object` matches
///   position-for-position): emit a list of targeted [`PlanPatch`]es —
///   `TransformUpdate` for a moved matrix, `StyleUpdate` for a recolored slot,
///   `GeometryUpdate` for a changed revision (touching ONLY that object). An
///   identical entry contributes nothing.
/// - Otherwise (an object added / removed / reordered): [`PlanDiff::Rebuild`].
///
/// Pure and total: a transform-only or style-only change can NEVER yield a
/// rebuild here, which is what the golden tests pin.
pub fn diff_plans(old: &FramePlan, new: &FramePlan) -> PlanDiff {
    if old.entries.len() != new.entries.len() {
        return PlanDiff::Rebuild;
    }
    // The object identity sequence must match position-for-position; a reorder or
    // an add/remove is a structural change no targeted patch covers.
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

/// Append the targeted patches that take `old_entry` to `new_entry` for object
/// `index`. A changed handle revision is a geometry edit (the mesh changed), so
/// it supersedes the instance-attribute diff — re-sending the mesh carries the
/// new instance with it. Otherwise the geometry is proven unchanged, and we emit
/// a transform update if the matrix moved and a style update per color slot that
/// changed.
fn diff_entry(
    index: usize,
    old_entry: &PlanEntry,
    new_entry: &PlanEntry,
    patches: &mut Vec<PlanPatch>,
) {
    let object = new_entry.handle.object.clone();

    // A changed revision means the baked mesh changed: re-send THIS object's
    // geometry (with its new instance + ranges) and nothing else.
    if old_entry.handle.revision != new_entry.handle.revision {
        patches.push(PlanPatch::GeometryUpdate {
            object,
            index,
            entry: Box::new(new_entry.clone()),
        });
        return;
    }

    // Geometry is unchanged (same handle). Diff the small instance attributes.
    let old_i = &old_entry.instance;
    let new_i = &new_entry.instance;

    // Matrix columns are shared across all four passes, so one transform update
    // rewrites them everywhere. Compare the fill instance's columns as the
    // canonical matrix (the build writes the same columns into every pass).
    if matrix_columns(&new_i.fill) != matrix_columns(&old_i.fill) {
        patches.push(PlanPatch::TransformUpdate {
            object: object.clone(),
            index,
            columns: matrix_columns(&new_i.fill),
        });
    }

    // Color slots are per-pass; emit one style update per slot whose color moved.
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

// ---------------------------------------------------------------------------
// Geometry revision
// ---------------------------------------------------------------------------

/// A content hash of every input that changes object `obj`'s TESSELLATED geometry
/// under `camera`: the path string, the zoom LOD bucket (the flattener's flatness
/// is bucketed, so two zooms in the same bucket tessellate identically), the
/// stroke parameters that change the ribbon (width/dash/cap/join) and whether the
/// object has a stroke at all, the fill KIND that gates `skip_fill`, and the text
/// runs/align that change the glyph quads.
///
/// Deliberately EXCLUDED: the object transform and the resolved paint COLORS.
/// Those are instance-level — a move or a recolor must keep the SAME revision so
/// the diff proves the geometry is unchanged and never re-sends the mesh. This is
/// the load-bearing property the whole IR rests on.
pub fn geometry_revision(obj: &RenderObject, camera: &CameraState) -> u64 {
    let mut h = Fnv::new();
    h.bytes(obj.geometry_d.as_bytes());
    // The zoom bucket, not the raw zoom: the LOD flattener buckets zoom, so the
    // mesh is identical within a bucket (matches `flatten_object_subpaths`).
    h.u64(crate::curve_lod::zoom_bucket(camera.zoom) as u64);
    hash_stroke(&mut h, obj.stroke.as_ref());
    // Fill KIND gates `skip_fill` (an open path with no fill is not tessellated)
    // and Solid-vs-token-vs-gradient does not change geometry, but PRESENCE does.
    h.u8(u8::from(obj.fill.is_some()));
    hash_text(&mut h, obj.text.as_ref());
    h.finish()
}

/// Fold the stroke's geometry-affecting parameters into `h`: presence, width,
/// dash pattern, cap, join. The paint is NOT hashed — stroke COLOR is instance-
/// level, not geometry.
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

/// Fold the text's geometry-affecting content into `h`: each run's text, size,
/// bold/italic/font (all change glyph layout / quad positions) and the block
/// align/valign. Run COLOR is per-glyph geometry data, so it DOES change the baked
/// quads — include it (unlike the instance colors, text color is not an instance
/// attribute the diff can patch, so a color-only text edit is a geometry edit).
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

// ---------------------------------------------------------------------------
// FNV-1a — a tiny, deterministic, alloc-free 64-bit hash for the revision.
// ---------------------------------------------------------------------------

/// FNV-1a 64-bit. Deterministic and pure (no RNG/seed/time — CLAUDE.md pure-core
/// rule), so the same object always hashes to the same revision across runs and
/// targets. Used only as a content fingerprint for the geometry handle; collision
/// risk is irrelevant to correctness because a collision degrades a true geometry
/// edit to "no re-send", which the consumer's range-fit guard would still catch on
/// the in-place patch — but for the diff's purpose the chance is negligible.
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
        // Hash the bit pattern so the fold is exact and total (incl. signed-zero /
        // NaN), matching how the geometry build consumes the raw f64.
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
        }
    }

    /// A rect whose fill + stroke are theme TOKENS, so its instance colors
    /// re-resolve on a theme flip (the style-update fixture).
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

    // ---- FramePlan snapshot ------------------------------------------------

    /// SNAPSHOT GOLDEN: the plan for a representative fixture scene has one entry
    /// per object, in scene order, each carrying a `(object, revision)` handle, the
    /// per-pass instance attributes, and the per-object draw ranges. The pass order
    /// is the fixed back-to-front shadow/fill/stroke/text. FAILS if the builder
    /// drops an object, reorders, or stops index-aligning entries with the scene.
    #[test]
    fn frame_plan_snapshots_one_entry_per_object_in_order() {
        let scene = scene_with(vec![rect_object("a"), text_rect("b"), rect_object("c")]);
        let plan = build_frame_plan(&scene, Theme::light());

        assert_eq!(plan.entries.len(), 3, "one entry per object");
        let ids: Vec<&str> = plan.entries.iter().map(|e| e.handle.object.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "entries in scene order");

        // Each entry's handle revision equals a direct recompute of its object's
        // geometry revision — the handle is the geometry fingerprint, nothing else.
        for (entry, obj) in plan.entries.iter().zip(&scene.objects) {
            assert_eq!(entry.handle.revision, geometry_revision(obj, &scene.camera));
            assert_eq!(entry.draw.id, obj.id, "draw range record is the same object");
        }

        // The geometry store is the merged build output the ranges address.
        assert_eq!(plan.geometry.draws.len(), 3);
        // Pass order is the pinned z-order the consumer records.
        assert_eq!(
            pass_order(),
            [DrawPass::Shadow, DrawPass::Fill, DrawPass::Stroke, DrawPass::Text]
        );
        // The text object (b) carries glyph quads; the plain rects do not.
        assert!(!plan.entries[1].draw.text_range.is_empty(), "text object has glyphs");
        assert!(plan.entries[0].draw.text_range.is_empty(), "plain rect has none");
    }

    /// An identical re-build of the same scene diffs to ZERO patches: nothing to
    /// upload. This is the "stable frame" baseline — a redraw with no edit must not
    /// produce spurious writes.
    #[test]
    fn rebuilding_the_same_scene_diffs_to_no_patches() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")]);
        let a = build_frame_plan(&scene, Theme::light());
        let b = build_frame_plan(&scene, Theme::light());
        assert_eq!(diff_plans(&a, &b), PlanDiff::Patches(Vec::new()));
    }

    // ---- Transform-only -> TransformUpdate (never Rebuild) -----------------

    /// THE DIFFABILITY GOLDEN (transform-only): moving ONE object's transform
    /// yields a single `TransformUpdate` patch against the UNCHANGED handle — never
    /// a rebuild, never a geometry re-send, never a style write. The patched
    /// columns equal `compose(delta, base)` — the exact same packing the instance
    /// preview write pushes. FAILS if a move re-tessellates (handle revision drifts)
    /// or the diff falls back to rebuild.
    #[test]
    fn transform_only_change_yields_a_transform_patch_not_a_rebuild() {
        let base_scene = scene_with(vec![rect_object("a"), rect_object("b")]);
        let old = build_frame_plan(&base_scene, Theme::light());

        // Move object "a" by a translate delta; "b" untouched.
        let delta = translate_3x3(40.0, -25.0);
        let mut moved_scene = base_scene.clone();
        moved_scene.objects[0].transform = mat3_mul(&delta, &base_scene.objects[0].transform);
        let new = build_frame_plan(&moved_scene, Theme::light());

        // (1) The moved object's geometry handle is UNCHANGED — a move never
        // re-tessellates, so the diff can prove the mesh is identical.
        assert_eq!(
            old.entries[0].handle, new.entries[0].handle,
            "transform-only move keeps the same geometry handle (no re-tessellation)"
        );

        // (2) The diff is targeted patches, NOT a rebuild.
        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("transform-only change must diff to patches, not a rebuild");
        };
        // (3) Exactly one patch, and it is a TransformUpdate for "a".
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
        // (4) NOT a style or geometry patch — those would mean we touched a color
        // slot or re-sent a mesh, which a move must never do.
        assert!(!patches.iter().any(|p| matches!(
            p,
            PlanPatch::StyleUpdate { .. } | PlanPatch::GeometryUpdate { .. } | PlanPatch::Rebuild
        )));
    }

    /// PERF BAR (transform-only): the patch path carries no mesh. The moved
    /// object's `GeometryUpdate` is never emitted, and the unchanged object's
    /// merged geometry slices are byte-identical between the two plans — so applying
    /// a transform diff touches zero mesh bytes. FAILS if a move starts copying
    /// geometry (a frame-time copy of unchanged geometry the IR must not add).
    #[test]
    fn transform_only_patch_touches_no_geometry_buffers() {
        let base_scene = scene_with(vec![rect_object("a"), rect_object("b")]);
        let old = build_frame_plan(&base_scene, Theme::light());

        let delta = translate_3x3(7.0, 3.0);
        let mut moved = base_scene.clone();
        moved.objects[0].transform = mat3_mul(&delta, &base_scene.objects[0].transform);
        let new = build_frame_plan(&moved, Theme::light());

        // The merged mesh buffers are byte-identical across a pure move: fill
        // vertices/indices, stroke ribbon, shadow silhouette, glyph quads all stay
        // put. Only the per-object instance matrix columns differ.
        assert_eq!(old.geometry.fill.vertices, new.geometry.fill.vertices);
        assert_eq!(old.geometry.fill.indices, new.geometry.fill.indices);
        assert_eq!(old.geometry.stroke_vertices, new.geometry.stroke_vertices);
        assert_eq!(old.geometry.shadow_vertices, new.geometry.shadow_vertices);
        assert_eq!(old.geometry.text_vertices, new.geometry.text_vertices);

        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("a move must not rebuild");
        };
        // No GeometryUpdate at all: the diff never asks the consumer to re-upload a
        // mesh for a transform-only frame.
        assert!(
            !patches.iter().any(|p| matches!(p, PlanPatch::GeometryUpdate { .. })),
            "a transform-only diff carries no geometry payload"
        );
    }

    // ---- Style-only -> StyleUpdate (never Rebuild) -------------------------

    /// THE DIFFABILITY GOLDEN (style-only): a theme flip recolors token-backed
    /// instance colors with NO geometry change. The diff yields only `StyleUpdate`
    /// patches (fill/stroke/shadow color slots) against unchanged handles — never a
    /// transform update, never a geometry re-send, never a rebuild. FAILS if a
    /// recolor re-tessellates or rebuilds.
    #[test]
    fn style_only_theme_flip_yields_style_patches_not_a_rebuild() {
        let scene = scene_with(vec![token_rect("a"), token_rect("b")]);
        let light = build_frame_plan(&scene, Theme::light());
        let dark = build_frame_plan(&scene, Theme::dark());

        // (1) Handles are theme-invariant: the geometry is byte-identical across a
        // flip, so every object's handle is unchanged.
        for (l, d) in light.entries.iter().zip(&dark.entries) {
            assert_eq!(l.handle, d.handle, "theme flip keeps the same geometry handle");
        }

        let PlanDiff::Patches(patches) = diff_plans(&light, &dark) else {
            panic!("a theme flip must diff to patches, not a rebuild");
        };
        assert!(!patches.is_empty(), "a token theme flip changes colors");
        // (2) EVERY patch is a StyleUpdate — no transform, no geometry, no rebuild.
        for patch in &patches {
            assert!(
                matches!(patch, PlanPatch::StyleUpdate { .. }),
                "a recolor produces only style updates, got {patch:?}"
            );
        }
        // (3) Both objects' fill + stroke + shadow slots flip (token fill/stroke and
        // the always-token shadow), so each object contributes three style updates.
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

    /// A raw-hex object recolored by editing its paint hex (NOT a theme token) is
    /// still a STYLE change, not geometry: same handle, a single fill `StyleUpdate`.
    /// Pins that the revision excludes resolved color, so any recolor is a style
    /// patch.
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

    // ---- Geometry edit -> GeometryUpdate of ONLY the touched handle --------

    /// THE DIFFABILITY GOLDEN (geometry edit): editing ONE object's path changes
    /// only that object's handle revision, so the diff emits a single
    /// `GeometryUpdate` for that object and NOTHING for its unchanged neighbours.
    /// FAILS if a geometry edit fans out to other objects or rebuilds the whole
    /// plan.
    #[test]
    fn geometry_edit_patches_only_the_touched_handle() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b"), rect_object("c")]);
        let old = build_frame_plan(&scene, Theme::light());

        // Edit ONLY object "b"'s path (move a node — a real tessellation change).
        let mut edited = scene.clone();
        edited.objects[1].geometry_d = "M0 0 L900 0 L900 900 L0 900 Z".to_string();
        let new = build_frame_plan(&edited, Theme::light());

        // (1) Only "b"'s handle revision changed; "a" and "c" are byte-stable.
        assert_eq!(old.entries[0].handle, new.entries[0].handle, "a unchanged");
        assert_ne!(old.entries[1].handle, new.entries[1].handle, "b re-tessellated");
        assert_eq!(old.entries[2].handle, new.entries[2].handle, "c unchanged");

        let PlanDiff::Patches(patches) = diff_plans(&old, &new) else {
            panic!("a single geometry edit must not rebuild the whole plan");
        };
        // (2) Exactly one patch, a GeometryUpdate for "b" carrying b's new entry.
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

    /// A geometry edit that ALSO moves the object emits ONLY the GeometryUpdate —
    /// the mesh re-send carries the new matrix with it, so no separate
    /// TransformUpdate is needed (and a double-write would be wrong). Pins that a
    /// revision change supersedes the instance diff.
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

    // ---- Structural change -> Rebuild --------------------------------------

    /// A structural change — adding, removing, or reordering objects — yields a
    /// `Rebuild`. The targeted patch vocabulary cannot express a changed object set
    /// (slot identity moves), so the diff cleanly falls back. FAILS if the diff
    /// silently produces patches against a mismatched object set (a corruption).
    #[test]
    fn structural_change_yields_rebuild() {
        let two = scene_with(vec![rect_object("a"), rect_object("b")]);
        let plan_two = build_frame_plan(&two, Theme::light());

        // Add an object: count changes -> rebuild.
        let three = scene_with(vec![rect_object("a"), rect_object("b"), rect_object("c")]);
        assert_eq!(
            diff_plans(&plan_two, &build_frame_plan(&three, Theme::light())),
            PlanDiff::Rebuild,
            "adding an object rebuilds"
        );

        // Remove an object -> rebuild.
        let one = scene_with(vec![rect_object("a")]);
        assert_eq!(
            diff_plans(&plan_two, &build_frame_plan(&one, Theme::light())),
            PlanDiff::Rebuild,
            "removing an object rebuilds"
        );

        // Reorder (same set, swapped order) -> rebuild, because slot/index identity
        // moves and a targeted patch addresses by index.
        let swapped = scene_with(vec![rect_object("b"), rect_object("a")]);
        assert_eq!(
            diff_plans(&plan_two, &build_frame_plan(&swapped, Theme::light())),
            PlanDiff::Rebuild,
            "reordering objects rebuilds"
        );
    }

    // ---- Revision properties (the load-bearing invariant) ------------------

    /// The geometry revision EXCLUDES the transform and the resolved color, and
    /// INCLUDES the path, stroke width, dash, and text. This is the exact contract
    /// the diffability rests on — pin each half so a future field added to the
    /// revision (or dropped from it) is caught here.
    #[test]
    fn revision_excludes_transform_and_color_includes_geometry_inputs() {
        let camera = CameraState { x: 0.0, y: 0.0, zoom: 1.0 };
        let base = rect_object("o");
        let base_rev = geometry_revision(&base, &camera);

        // Transform change: SAME revision (instance-level, not geometry).
        let mut moved = base.clone();
        moved.transform = translate_3x3(99.0, -99.0);
        assert_eq!(geometry_revision(&moved, &camera), base_rev, "transform excluded");

        // Fill color change (raw hex): SAME revision.
        let mut recolored = base.clone();
        recolored.fill = Some(RFill {
            paint: RPaint::Solid { color: "#123456".to_string() },
            opacity: 1.0,
        });
        assert_eq!(geometry_revision(&recolored, &camera), base_rev, "fill color excluded");

        // Stroke color change: SAME revision (paint not hashed).
        let mut restroked = base.clone();
        if let Some(s) = restroked.stroke.as_mut() {
            s.paint = RPaint::Solid { color: "#abcdef".to_string() };
        }
        assert_eq!(geometry_revision(&restroked, &camera), base_rev, "stroke color excluded");

        // Path change: DIFFERENT revision.
        let mut repathed = base.clone();
        repathed.geometry_d = "M0 0 L400 0 L400 400 L0 400 Z".to_string();
        assert_ne!(geometry_revision(&repathed, &camera), base_rev, "path included");

        // Stroke width change: DIFFERENT revision (changes the ribbon).
        let mut rewidened = base.clone();
        if let Some(s) = rewidened.stroke.as_mut() {
            s.width = 12.0;
        }
        assert_ne!(geometry_revision(&rewidened, &camera), base_rev, "stroke width included");

        // Dash change: DIFFERENT revision (changes the CPU dash split).
        let mut redashed = base.clone();
        if let Some(s) = redashed.stroke.as_mut() {
            s.dash = vec![4.0, 4.0];
        }
        assert_ne!(geometry_revision(&redashed, &camera), base_rev, "dash included");
    }

    /// The revision is bucketed by zoom, matching the LOD flattener: two zooms in
    /// the SAME bucket give the same revision (the mesh is identical), and crossing
    /// a bucket boundary changes it. Pins that a pan/zoom WITHIN a bucket keeps the
    /// handle (so it diffs as no geometry change).
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

        // Find a zoom in a different bucket and confirm the revision moves with it.
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
