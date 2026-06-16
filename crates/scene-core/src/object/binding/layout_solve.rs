//! Thin auto-layout solve. A children-group [`Object`] with `layout:
//! Some(Layout)` arranges objects whose `parent == group.id` along a main axis
//! ([`LayoutAxis::Horizontal`] => x, [`LayoutAxis::Vertical`] => y) by the single
//! `spacing` (both edge inset and inter-child gap), cross-aligned per
//! [`CrossAlign`] and main-aligned per [`MainAlign`]. The result is derived, not
//! stored: a fresh `(ObjectId, Transform3x3)` per child for the caller to apply
//! at draw time — stored geometry/transforms are never mutated (zero-rebake).
//!
//! Pure: no IO/time/rng. Inputs are object-local quantized i32; the packing math
//! runs in logical px (divide by [`GEOMETRY_QUANTUM_PER_PX`]) because a transform
//! operates in logical px.
//!
//! Lanes shape the line breaking: [`Lanes::Count { value: 1 }`] is one line (a
//! list); `Count { value: N }` is an N-track grid (children block-distributed
//! into N cross-axis tracks, each track packed along main in child order); `Fill`
//! wraps as many children per line as fit the container's resolved main extent
//! (one line when the container hugs and so has no main constraint).
//!
//! Per-child [`Sizing`] picks each axis's packed extent: `Hug` = the child's
//! rotated-AABB (OBB) extent, `Fixed { value }` = that quantized size, `Fill` =
//! an equal share of the line's leftover main space after the non-Fill children.
//! A rotated child packs by the extent of its ROTATED local AABB, not the
//! axis-aligned local AABB.
//!
//! A `Fill`/`Fixed` (main) or `Fixed`/`Stretch`/`Fill` (cross) child bakes a real
//! container-axis scale into its derived transform so its OBB extent matches its
//! resolved slot on that axis — still zero-rebake (only the transform changes;
//! geometry is never re-tessellated). Text/complex-path children visually distort
//! under this non-uniform scale (accepted follow-up). The cross band a Stretch/Fill
//! child grows to excludes those same children, so a stretching child never chases
//! the band it inflates.
//!
//! Honesty caveats: [`MainAlign::Center`]/`End`/`SpaceBetween` need free main space;
//! a container with empty (default) geometry has no derivable frame, so it falls
//! back to `Start`.

use crate::object::anchor_follow::{affine_of, apply_affine};
use crate::object::model::{
    AxisSizing, CrossAlign, Layout, LayoutAxis, Lanes, MainAlign, Object, ObjectId, ObjectScene,
    Sizing, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::region::OutlineDeriver;

/// Quantized-units -> logical px.
fn to_px(q: i32) -> f64 {
    f64::from(q) / f64::from(GEOMETRY_QUANTUM_PER_PX)
}

/// One child's packing box, all in logical px. `main`/`cross` are the resolved
/// extents (after [`Sizing`]); `obb_main`/`obb_cross` are the intrinsic
/// rotated-AABB extents used for `Hug` and as the lower bound a `Fill` child can
/// never shrink below. `offset_*` re-seat the child so its OBB min-corner (not its
/// local origin) lands at the placed position.
struct ChildBox {
    id: ObjectId,
    main: f64,
    cross: f64,
    obb_main: f64,
    obb_cross: f64,
    offset_main: f64,
    offset_cross: f64,
    main_fill: bool,
    cross_fill: bool,
    /// The child's transform with translation zeroed — its rotation/scale, kept so
    /// the derived placement preserves orientation and only replaces translation.
    linear: Transform3x3,
}

/// A transform's linear part (rotation/scale/skew), translation set to zero.
fn linear_only(t: &Transform3x3) -> Transform3x3 {
    let mut m = t.m;
    m[0][2] = 0.0;
    m[1][2] = 0.0;
    Transform3x3 { m }
}

/// The rotated-AABB (OBB) extent of a child in WORLD-aligned main/cross px: the
/// child's local-AABB corners carried through its transform's linear part, then
/// min/max'd on each axis. `(main_lo, main_hi, cross_lo, cross_hi)` are relative
/// to where the transform maps the local origin, so a translate that lands the
/// origin at `p` puts the OBB min-corner at `p + (main_lo, cross_lo)`.
fn obb_extent(
    obj: &Object,
    axis: LayoutAxis,
    aabb_px: (f64, f64, f64, f64),
) -> (f64, f64, f64, f64) {
    let (min_x, min_y, max_x, max_y) = aabb_px;
    let a = affine_of(&obj.transform);
    // Linear part only: subtract the translation so the corners are relative to
    // the origin's image (the translate is re-added by the solver as placement).
    let (ox, oy) = apply_affine(&a, 0.0, 0.0);
    let corners = [
        (min_x, min_y),
        (max_x, min_y),
        (max_x, max_y),
        (min_x, max_y),
    ];
    let mut x_lo = f64::INFINITY;
    let mut x_hi = f64::NEG_INFINITY;
    let mut y_lo = f64::INFINITY;
    let mut y_hi = f64::NEG_INFINITY;
    for (lx, ly) in corners {
        let (wx, wy) = apply_affine(&a, lx, ly);
        let (rx, ry) = (wx - ox, wy - oy);
        x_lo = x_lo.min(rx);
        x_hi = x_hi.max(rx);
        y_lo = y_lo.min(ry);
        y_hi = y_hi.max(ry);
    }
    match axis {
        LayoutAxis::Horizontal => (x_lo, x_hi, y_lo, y_hi),
        LayoutAxis::Vertical => (y_lo, y_hi, x_lo, x_hi),
    }
}

fn sizing_of(obj: &Object) -> Sizing {
    obj.sizing
        .unwrap_or(Sizing { w: AxisSizing::Hug, h: AxisSizing::Hug })
}

/// Resolve one axis's `AxisSizing` to a packed extent. `Hug` => the intrinsic OBB
/// extent; `Fixed` => the quantized value in px; `Fill` => `intrinsic` for now,
/// flagged so the placement step can grow it to its slot (main = the line's leftover
/// share, cross = the line band).
fn resolve_extent(s: AxisSizing, intrinsic: f64) -> (f64, bool) {
    match s {
        AxisSizing::Hug => (intrinsic, false),
        AxisSizing::Fixed { value } => (to_px(value).max(0.0), false),
        AxisSizing::Fill => (intrinsic, true),
    }
}

/// Derived transform per child (objects whose `parent == group_id`), in child
/// paint order (fractional `order`, ties broken by id). Empty when the group is
/// missing, has no `layout`, or no children. Children with no derivable region
/// are skipped — they cannot occupy main-axis space.
pub fn solve_layout(
    scene: &ObjectScene,
    group_id: &str,
    deriver: &impl OutlineDeriver,
) -> Vec<(ObjectId, Transform3x3)> {
    let Some(group) = scene.get(group_id) else {
        return Vec::new();
    };
    let Some(layout) = group.layout.as_ref() else {
        return Vec::new();
    };

    let axis = layout.axis;
    let spacing = to_px(layout.spacing);

    // Gather children with a derivable region, in canonical paint order, each
    // resolved to its packing box (OBB extents + Sizing).
    let mut children: Vec<(&Object, ChildBox)> = scene
        .objects
        .iter()
        .filter(|o| o.parent.as_deref() == Some(group_id))
        .filter_map(|o| {
            let region = deriver.derive_region(&o.geometry, 1).ok()?;
            let b = region.bounds;
            let aabb_px = (to_px(b.min_x), to_px(b.min_y), to_px(b.max_x), to_px(b.max_y));
            let (m_lo, m_hi, c_lo, c_hi) = obb_extent(o, axis, aabb_px);
            let obb_main = m_hi - m_lo;
            let obb_cross = c_hi - c_lo;
            let s = sizing_of(o);
            let (s_main, s_cross) = match axis {
                LayoutAxis::Horizontal => (s.w, s.h),
                LayoutAxis::Vertical => (s.h, s.w),
            };
            let (main, main_fill) = resolve_extent(s_main, obb_main);
            let (cross, cross_fill) = resolve_extent(s_cross, obb_cross);
            Some((
                o,
                ChildBox {
                    id: o.id.clone(),
                    main,
                    cross,
                    obb_main,
                    obb_cross,
                    offset_main: m_lo,
                    offset_cross: c_lo,
                    main_fill,
                    cross_fill,
                    linear: linear_only(&o.transform),
                },
            ))
        })
        .collect();
    children.sort_by(|(a, ab), (b, bb)| a.order.cmp(&b.order).then_with(|| ab.id.cmp(&bb.id)));

    if children.is_empty() {
        return Vec::new();
    }
    let boxes: Vec<ChildBox> = children.into_iter().map(|(_, b)| b).collect();

    // Resolve the container's main-axis content extent. `Fixed` pins it; `Hug`
    // (and `Fill`, which has no parent to fill against here) derives it later from
    // the packed content. A finite pin enables `Fill`-wrap and main-align free
    // space; a hug yields `None` (single-line Fill, Start-only main align).
    let container_sizing = sizing_of(group);
    let container_main_sizing = match axis {
        LayoutAxis::Horizontal => container_sizing.w,
        LayoutAxis::Vertical => container_sizing.h,
    };
    let pinned_main: Option<f64> = match container_main_sizing {
        AxisSizing::Fixed { value } => Some((to_px(value) - 2.0 * spacing).max(0.0)),
        // Hug/Fill derive a pin from the container's OWN drawn geometry main extent,
        // so Center/End/SpaceBetween and Fill-wrap have a real free-space frame. An
        // empty (default) container geometry yields no region => None (Start-only).
        AxisSizing::Hug | AxisSizing::Fill => deriver
            .derive_region(&group.geometry, 1)
            .ok()
            .map(|region| {
                let b = region.bounds;
                let aabb_px = (to_px(b.min_x), to_px(b.min_y), to_px(b.max_x), to_px(b.max_y));
                let (gm_lo, gm_hi, _, _) = obb_extent(group, axis, aabb_px);
                ((gm_hi - gm_lo) - 2.0 * spacing).max(0.0)
            }),
    };

    let lines = break_into_lines(&boxes, layout, pinned_main, spacing);

    place_lines(&boxes, &lines, layout, pinned_main, spacing)
}

/// Each line is the half-open child-index range `[start, end)` (children are
/// already in pack order, so a line is contiguous).
fn break_into_lines(
    boxes: &[ChildBox],
    layout: &Layout,
    pinned_main: Option<f64>,
    spacing: f64,
) -> Vec<(usize, usize)> {
    match layout.lanes {
        Lanes::Count { value } => {
            let tracks = value.max(1) as usize;
            if tracks <= 1 {
                return vec![(0, boxes.len())];
            }
            // Block-distribute: track 0 gets the first `per` children, etc., so
            // `Count { value: 1 }` (one track) is a list and the count is the
            // number of cross-axis tracks.
            let per = boxes.len().div_ceil(tracks);
            let mut lines = Vec::new();
            let mut start = 0;
            while start < boxes.len() {
                let end = (start + per).min(boxes.len());
                lines.push((start, end));
                start = end;
            }
            lines
        }
        Lanes::Fill => {
            // Wrap as many per line as fit the pinned main extent; no constraint
            // (hug) => one line.
            let Some(limit) = pinned_main else {
                return vec![(0, boxes.len())];
            };
            let mut lines = Vec::new();
            let mut start = 0;
            let mut used = 0.0;
            let mut i = 0;
            while i < boxes.len() {
                let w = boxes[i].main;
                let add = if i == start { w } else { spacing + w };
                if i > start && used + add > limit {
                    lines.push((start, i));
                    start = i;
                    used = w;
                } else {
                    used += add;
                }
                i += 1;
            }
            lines.push((start, boxes.len()));
            lines
        }
    }
}

/// Place every child: resolve `Fill` main extents per line against the line's
/// leftover space, then lay each line along main (with main-align) and stack the
/// lines along cross (each line's band sized to its tallest cross extent), cross-
/// aligning each child within its line band.
fn place_lines(
    boxes: &[ChildBox],
    lines: &[(usize, usize)],
    layout: &Layout,
    pinned_main: Option<f64>,
    spacing: f64,
) -> Vec<(ObjectId, Transform3x3)> {
    // Resolved main extent per child (Fill children grown to share leftover).
    let mut main_size: Vec<f64> = boxes.iter().map(|b| b.main).collect();

    // The line that defines the content main extent when hugging = the widest
    // line's packed main length (non-Fill extents + gaps).
    let mut content_main: f64 = 0.0;
    for &(start, end) in lines {
        let n = end - start;
        if n == 0 {
            continue;
        }
        let gaps = spacing * (n.saturating_sub(1) as f64);
        let fixed: f64 = boxes[start..end].iter().filter(|b| !b.main_fill).map(|b| b.main).sum();
        content_main = content_main.max(fixed + gaps);
    }

    // The main extent each line lays out against: the pin when given, else the
    // hugged content extent.
    let line_main = pinned_main.unwrap_or(content_main);

    // Grow Fill children to share each line's leftover main space.
    for &(start, end) in lines {
        let n = end - start;
        if n == 0 {
            continue;
        }
        let fill_count = boxes[start..end].iter().filter(|b| b.main_fill).count();
        if fill_count == 0 {
            continue;
        }
        let gaps = spacing * (n.saturating_sub(1) as f64);
        let fixed: f64 = boxes[start..end].iter().filter(|b| !b.main_fill).map(|b| b.main).sum();
        let leftover = (line_main - fixed - gaps).max(0.0);
        let share = leftover / fill_count as f64;
        for (b, slot) in boxes[start..end].iter().zip(main_size[start..end].iter_mut()) {
            if b.main_fill {
                // A Fill child never shrinks below its intrinsic OBB extent.
                *slot = share.max(b.obb_main);
            }
        }
    }

    let mut out: Vec<(ObjectId, Transform3x3)> = Vec::with_capacity(boxes.len());
    let mut cross_cursor = spacing;
    for &(start, end) in lines {
        let n = end - start;
        if n == 0 {
            continue;
        }
        let used: f64 = main_size[start..end].iter().sum::<f64>()
            + spacing * (n.saturating_sub(1) as f64);
        let free = (line_main - used).max(0.0);
        // Main alignment distributes the line's leftover; falls back to Start when
        // the container hugs (no pin) or there is no free space.
        let (mut main_cursor, between) = match layout.align.main {
            MainAlign::Start => (spacing, spacing),
            MainAlign::Center => (spacing + free / 2.0, spacing),
            MainAlign::End => (spacing + free, spacing),
            MainAlign::SpaceBetween => {
                if pinned_main.is_some() && n > 1 && free > 0.0 {
                    (spacing, spacing + free / (n - 1) as f64)
                } else {
                    (spacing, spacing)
                }
            }
        };

        // The line's cross band. A Stretch/Fill-cross child grows TO this band, so it
        // must be derived from the others (Hug=intrinsic, Fixed=value); else the band
        // would chase the child it inflates. An all-stretch/fill line has no such
        // child, so fall back to the tallest intrinsic OBB (no infinite inflation).
        let stretches = |b: &ChildBox| layout.align.cross == CrossAlign::Stretch || b.cross_fill;
        let band_cross = {
            let banded = boxes[start..end]
                .iter()
                .filter(|b| !stretches(b))
                .map(|b| b.cross)
                .fold(0.0_f64, f64::max);
            if banded > 0.0 {
                banded
            } else {
                boxes[start..end].iter().map(|b| b.obb_cross).fold(0.0_f64, f64::max)
            }
        };

        for i in start..end {
            let child = &boxes[i];
            // Stretch/Fill-cross grow to the band; everything else keeps its resolved
            // cross extent. Alignment free space measures against the REAL placed span.
            let cross_target = if stretches(child) { band_cross } else { child.cross };
            let free_cross = (band_cross - cross_target).max(0.0);
            let cross_within = match layout.align.cross {
                CrossAlign::Start | CrossAlign::Stretch => 0.0,
                CrossAlign::Center => free_cross / 2.0,
                CrossAlign::End => free_cross,
            };
            let main_pos = main_cursor;
            let cross_pos = cross_cursor + cross_within;

            // Bake a real container-axis scale on BOTH axes so the child's OBB extent
            // becomes its resolved slot (Hug keeps k=1 / S=I). Build S in the CONTAINER
            // frame and LEFT-multiply the child linear, so a rotated child's OBB extent
            // in container space scales (not its local axis).
            let resized = (child.main_fill || (main_size[i] - child.obb_main).abs() > 1e-9)
                && child.obb_main > 1e-9;
            let k = if resized { main_size[i] / child.obb_main } else { 1.0 };
            let resized_cross = (cross_target - child.obb_cross).abs() > 1e-9 && child.obb_cross > 1e-9;
            let k_cross = if resized_cross { cross_target / child.obb_cross } else { 1.0 };
            let scale = match layout.axis {
                LayoutAxis::Horizontal => {
                    Transform3x3 { m: [[k, 0.0, 0.0], [0.0, k_cross, 0.0], [0.0, 0.0, 1.0]] }
                }
                LayoutAxis::Vertical => {
                    Transform3x3 { m: [[k_cross, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, 1.0]] }
                }
            };
            let scaled_linear = scale.mul(&child.linear);

            // The child's OBB min-corner must land at (main_pos, cross_pos); the scaled
            // linear maps the local origin to `origin + offset*k` on each axis, so
            // subtract the scaled OBB offset to seat the content (not the origin).
            let main_origin = main_pos - child.offset_main * k;
            let cross_origin = cross_pos - child.offset_cross * k_cross;
            let (tx, ty) = match layout.axis {
                LayoutAxis::Horizontal => (main_origin, cross_origin),
                LayoutAxis::Vertical => (cross_origin, main_origin),
            };

            // Preserve the child's rotation/scale (its transform's linear part) under
            // the layout scale; only the translation is replaced by the placement.
            let placed = Transform3x3::translate(tx, ty).mul(&scaled_linear);
            out.push((child.id.clone(), placed));
            main_cursor += main_size[i] + between;
        }
        cross_cursor += band_cross + spacing;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{
        Align, FillRule, Geometry, Lanes, Layout, MainAlign, Object, PathNode, SubPath,
    };
    use crate::object::region::StubOutlineDeriver;

    fn layout(axis: LayoutAxis, spacing: i32, cross: CrossAlign) -> Layout {
        Layout {
            axis,
            lanes: Lanes::Count { value: 1 },
            spacing,
            align: Align { main: MainAlign::Start, cross },
        }
    }

    fn rect(w: i32, h: i32) -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(w, 0),
                    PathNode::corner(w, h),
                    PathNode::corner(0, h),
                ],
            }],
            FillRule::EvenOdd,
        )
    }

    fn child(id: &str, order: &str, parent: &str, geom: Geometry) -> Object {
        let mut o = Object::new(id, order, geom);
        o.parent = Some(parent.into());
        o
    }

    fn translate_of(t: &Transform3x3) -> (f64, f64) {
        (t.m[0][2], t.m[1][2])
    }

    fn group_with_layout(id: &str, layout: Layout) -> Object {
        let mut g = Object::new(id, "g0", Geometry::default());
        g.layout = Some(layout);
        g
    }

    /// A layout container with a REAL geometry rect (so align/Fill have a derivable
    /// free-space frame) and a Hug/Hug sizing (no Fixed pin — the pin is geometry-derived).
    fn hug_group_with_geometry(id: &str, layout: Layout, w: i32, h: i32) -> Object {
        let mut g = Object::new(id, "g0", rect(w, h));
        g.layout = Some(layout);
        g
    }

    // --- list / single-track packing + spacing-as-uniform-inset --------------

    #[test]
    fn horizontal_packs_children_by_width_plus_spacing() {
        // 3 rects (80x40q = 10x5px), spacing 16q = 2px. spacing is BOTH the inset
        // (x starts at 2) AND the inter-child gap: x by width(10) + spacing(2).
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 16, CrossAlign::Start)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));
        scene.objects.push(child("c", "a2", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert_eq!(out.len(), 3);

        let xs: Vec<f64> = out.iter().map(|(_, t)| translate_of(t).0).collect();
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
        assert_eq!(out[2].0, "c");
        assert!((xs[0] - 2.0).abs() < 1e-9, "first x (inset) = {}", xs[0]);
        assert!((xs[1] - 14.0).abs() < 1e-9, "second x = {}", xs[1]);
        assert!((xs[2] - 26.0).abs() < 1e-9, "third x = {}", xs[2]);
        // Cross inset also rides spacing: every child sits 2px down.
        for (_, t) in &out {
            assert!((translate_of(t).1 - 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn spacing_offsets_both_inset_and_gap() {
        // spacing 8q = 1px: inset places the first child at (1,1); the gap adds 1px
        // between width(10) packs => second x = 1 + 10 + 1 = 12.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 8, CrossAlign::Start)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let (x0, y0) = translate_of(&out[0].1);
        let (x1, _) = translate_of(&out[1].1);
        assert!((x0 - 1.0).abs() < 1e-9, "inset start x = {x0}");
        assert!((y0 - 1.0).abs() < 1e-9, "inset cross y = {y0}");
        assert!((x1 - 12.0).abs() < 1e-9, "second x = {x1}");
    }

    #[test]
    fn vertical_packs_children_by_height_plus_spacing() {
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Vertical, 16, CrossAlign::Start)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let (x0, y0) = translate_of(&out[0].1);
        let (x1, y1) = translate_of(&out[1].1);
        assert!((y0 - 2.0).abs() < 1e-9, "first y (inset) = {y0}");
        assert!((y1 - 9.0).abs() < 1e-9, "second y = {y1}");
        assert!((x0 - 2.0).abs() < 1e-9 && (x1 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn missing_group_or_layout_yields_empty() {
        let scene = ObjectScene::default();
        assert!(solve_layout(&scene, "nope", &StubOutlineDeriver).is_empty());

        let mut scene2 = ObjectScene::default();
        scene2.objects.push(Object::new("grp", "g0", rect(80, 40)));
        scene2.objects.push(child("a", "a0", "grp", rect(80, 40)));
        assert!(solve_layout(&scene2, "grp", &StubOutlineDeriver).is_empty());
    }

    #[test]
    fn children_ordered_by_fractional_order() {
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        scene.objects.push(child("c", "a2", "grp", rect(80, 40)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let ids: Vec<String> = solve_layout(&scene, "grp", &StubOutlineDeriver)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    // --- cross alignment -----------------------------------------------------

    #[test]
    fn center_align_centers_smaller_child_on_cross_axis() {
        // Tall child (40q=5px) + short (20q=2.5px); spacing 0, center offset
        // (5-2.5)/2 = 1.25px.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Center)));
        scene.objects.push(child("tall", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("short", "a1", "grp", rect(80, 20)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let tall_y = translate_of(&out[0].1).1;
        let short_y = translate_of(&out[1].1).1;
        assert!(tall_y.abs() < 1e-9, "tall stays at band top: {tall_y}");
        assert!((short_y - 1.25).abs() < 1e-9, "short centered: {short_y}");
    }

    #[test]
    fn end_align_bottom_aligns_smaller_child_on_cross_axis() {
        // Band cross = 5px (tall), short = 2.5px; End offset = 5 - 2.5 = 2.5px.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::End)));
        scene.objects.push(child("tall", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("short", "a1", "grp", rect(80, 20)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).1.abs() < 1e-9, "tall at band top");
        assert!((translate_of(&out[1].1).1 - 2.5).abs() < 1e-9, "short end-aligned");
    }

    #[test]
    fn stretch_cross_child_grows_to_band_cross() {
        // Stretch bakes a real cross scale: the short child's placed OBB cross span
        // grows to the band (5px), the tall child keeps its 5px (k_cross=1). The band
        // excludes the stretching children, so it never chases what it inflates — here
        // both are Stretch, so it falls back to max intrinsic (5px).
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Stretch)));
        scene.objects.push(child("tall", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("short", "a1", "grp", rect(80, 20)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let tall = placed_obb_cross_span(&out[0].1, LayoutAxis::Horizontal, 80, 40);
        let short = placed_obb_cross_span(&out[1].1, LayoutAxis::Horizontal, 80, 20);
        assert!((tall - 5.0).abs() < 1e-9, "tall stays 5px, got {tall}");
        assert!((short - 5.0).abs() < 1e-9, "short stretches to band 5px, got {short}");
    }

    #[test]
    fn all_stretch_line_falls_back_to_max_intrinsic_no_inflation() {
        // Two Stretch children of equal intrinsic cross: with no non-stretch child to
        // band, the band falls back to max intrinsic, so both keep their intrinsic span
        // (k_cross=1) — never an infinite/NaN inflation.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Stretch)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        for (id, _) in &out {
            let span = placed_obb_cross_span(
                &out.iter().find(|(i, _)| i == id).unwrap().1,
                LayoutAxis::Horizontal,
                80,
                40,
            );
            assert!((span - 5.0).abs() < 1e-9, "{id} keeps intrinsic 5px, got {span}");
        }
    }

    // --- main alignment ------------------------------------------------------

    fn pinned_layout(
        axis: LayoutAxis,
        spacing: i32,
        main: MainAlign,
        container_main_q: i32,
    ) -> (Object, AxisSizing) {
        let mut g = group_with_layout(
            "grp",
            Layout {
                axis,
                lanes: Lanes::Count { value: 1 },
                spacing,
                align: Align { main, cross: CrossAlign::Start },
            },
        );
        let fixed = AxisSizing::Fixed { value: container_main_q };
        g.sizing = Some(match axis {
            LayoutAxis::Horizontal => Sizing { w: fixed, h: AxisSizing::Hug },
            LayoutAxis::Vertical => Sizing { h: fixed, w: AxisSizing::Hug },
        });
        (g, fixed)
    }

    #[test]
    fn main_center_align_uses_pinned_container_free_space() {
        // Container fixed main = 200q (25px), inner = 25 - 2*spacing(0) = 25px.
        // One 10px child => free = 15px, Center => start at 15/2 = 7.5px.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::Center, 200);
        scene.objects.push(g);
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!((translate_of(&out[0].1).0 - 7.5).abs() < 1e-9, "x = {}", translate_of(&out[0].1).0);
    }

    #[test]
    fn main_end_align_packs_against_pinned_far_edge() {
        // 25px inner, one 10px child, End => start at free(15)+inset(0) = 15px.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::End, 200);
        scene.objects.push(g);
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!((translate_of(&out[0].1).0 - 15.0).abs() < 1e-9);
    }

    #[test]
    fn main_space_between_distributes_gaps() {
        // 25px inner, two 10px children => used = 20, free = 5 split into 1 gap.
        // first at 0, second at 10 + 5 = 15.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::SpaceBetween, 200);
        scene.objects.push(g);
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9, "first at inset");
        assert!((translate_of(&out[1].1).0 - 15.0).abs() < 1e-9, "second at far edge minus width");
    }

    #[test]
    fn main_space_between_falls_back_to_start_when_hugging() {
        // No container pin (Hug) => no free space => SpaceBetween packs like Start.
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                axis: LayoutAxis::Horizontal,
                lanes: Lanes::Count { value: 1 },
                spacing: 0,
                align: Align { main: MainAlign::SpaceBetween, cross: CrossAlign::Start },
            },
        ));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9);
        assert!((translate_of(&out[1].1).0 - 10.0).abs() < 1e-9, "packed tight, no distribution");
    }

    // --- align against geometry-derived free space (FIX 2) -------------------

    fn hug_aligned_group(axis: LayoutAxis, spacing: i32, main: MainAlign, w: i32, h: i32) -> Object {
        hug_group_with_geometry(
            "grp",
            Layout {
                axis,
                lanes: Lanes::Count { value: 1 },
                spacing,
                align: Align { main, cross: CrossAlign::Start },
            },
            w,
            h,
        )
    }

    #[test]
    fn main_center_align_uses_geometry_derived_free_space() {
        // HUG container (no Fixed sizing) whose OWN geometry is a 200q (25px) wide
        // rect; one 10px child. Free = 25 - 10 = 15, Center => start at 15/2 = 7.5.
        let mut scene = ObjectScene::default();
        scene.objects.push(hug_aligned_group(LayoutAxis::Horizontal, 0, MainAlign::Center, 200, 40));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!((translate_of(&out[0].1).0 - 7.5).abs() < 1e-9, "x = {}", translate_of(&out[0].1).0);
    }

    #[test]
    fn main_end_align_uses_geometry_derived_free_space() {
        // Same 25px geometry frame, one 10px child, End => start at free(15) = 15px.
        let mut scene = ObjectScene::default();
        scene.objects.push(hug_aligned_group(LayoutAxis::Horizontal, 0, MainAlign::End, 200, 40));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!((translate_of(&out[0].1).0 - 15.0).abs() < 1e-9, "x = {}", translate_of(&out[0].1).0);
    }

    #[test]
    fn main_space_between_uses_geometry_derived_free_space() {
        // 25px geometry frame, two 10px children => used 20, free 5 in one gap.
        // first at 0, second at 10 + 5 = 15.
        let mut scene = ObjectScene::default();
        scene.objects.push(hug_aligned_group(LayoutAxis::Horizontal, 0, MainAlign::SpaceBetween, 200, 40));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9, "first at inset");
        assert!((translate_of(&out[1].1).0 - 15.0).abs() < 1e-9, "second at far edge minus width");
    }

    #[test]
    fn empty_geometry_container_align_falls_back_to_start() {
        // A container with Geometry::default() (empty) has no derivable frame, so
        // Center collapses to Start exactly like the Hug-no-geometry case.
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                axis: LayoutAxis::Horizontal,
                lanes: Lanes::Count { value: 1 },
                spacing: 0,
                align: Align { main: MainAlign::Center, cross: CrossAlign::Start },
            },
        ));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9, "empty-geometry container stays Start");
    }

    // --- per-object sizing ---------------------------------------------------

    fn sized_child(id: &str, order: &str, parent: &str, geom: Geometry, sizing: Sizing) -> Object {
        let mut o = child(id, order, parent, geom);
        o.sizing = Some(sizing);
        o
    }

    #[test]
    fn fixed_sizing_overrides_content_extent_for_packing() {
        // Child A intrinsic 10px but Fixed main = 40q = 5px; B packs after A's
        // fixed 5px (+spacing 0) => B at x = 5.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Fixed { value: 40 }, h: AxisSizing::Hug },
        ));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9, "a at inset 0");
        assert!((translate_of(&out[1].1).0 - 5.0).abs() < 1e-9, "b after a's fixed 5px");
    }

    #[test]
    fn fill_child_grows_to_share_pinned_leftover() {
        // Pinned inner 25px, A is Fill, B is Hug(10px), spacing 0. Leftover for A =
        // 25 - 10 = 15px, so A occupies 15px and B starts at x = 15.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::Start, 200);
        scene.objects.push(g);
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Fill, h: AxisSizing::Hug },
        ));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9, "a at inset");
        assert!((translate_of(&out[1].1).0 - 15.0).abs() < 1e-9, "b after grown a (15px)");
    }

    #[test]
    fn fill_children_split_leftover_equally() {
        // Pinned inner 30px, two Fill children, spacing 0 => each gets 15px.
        // Second starts at x = 15.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::Start, 240);
        scene.objects.push(g);
        let fill = Sizing { w: AxisSizing::Fill, h: AxisSizing::Hug };
        scene.objects.push(sized_child("a", "a0", "grp", rect(80, 40), fill));
        scene.objects.push(sized_child("b", "a1", "grp", rect(80, 40), fill));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert!(translate_of(&out[0].1).0.abs() < 1e-9);
        assert!((translate_of(&out[1].1).0 - 15.0).abs() < 1e-9, "each Fill shares 15px");
    }

    // --- child main-axis transform-scale resize (FIX 4b) ---------------------

    /// The OBB main span (along the layout axis) of a child rect placed under `placed`.
    /// Measures the REAL transformed geometry, so a baked scale (not just a translate)
    /// is observable. `(w, h)` are the rect's quantized local size.
    fn placed_obb_main_span(placed: &Transform3x3, axis: LayoutAxis, w: i32, h: i32) -> f64 {
        let mut probe = Object::new("probe", "a0", rect(w, h));
        probe.transform = *placed;
        let aabb_px = (0.0, 0.0, to_px(w), to_px(h));
        let (m_lo, m_hi, _, _) = obb_extent(&probe, axis, aabb_px);
        m_hi - m_lo
    }

    /// The OBB cross span (perpendicular to the layout axis) of a child rect placed
    /// under `placed` — the counterpart to [`placed_obb_main_span`] so a baked cross
    /// scale is observable on the real transformed geometry.
    fn placed_obb_cross_span(placed: &Transform3x3, axis: LayoutAxis, w: i32, h: i32) -> f64 {
        let mut probe = Object::new("probe", "a0", rect(w, h));
        probe.transform = *placed;
        let aabb_px = (0.0, 0.0, to_px(w), to_px(h));
        let (_, _, c_lo, c_hi) = obb_extent(&probe, axis, aabb_px);
        c_hi - c_lo
    }

    #[test]
    fn fill_child_stretches_to_its_resolved_main_extent() {
        // Pinned inner 25px, A Fill + B Hug(10px), spacing 0: A's slot = 25 - 10 = 15.
        // A's PLACED OBB main span must be 15 (a real scale), not its intrinsic 10.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::Start, 200);
        scene.objects.push(g);
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Fill, h: AxisSizing::Hug },
        ));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let a = &out.iter().find(|(id, _)| id == "a").unwrap().1;
        let span = placed_obb_main_span(a, LayoutAxis::Horizontal, 80, 40);
        assert!((span - 15.0).abs() < 1e-9, "Fill child stretches to 15px, got {span}");
    }

    #[test]
    fn fixed_child_pins_its_resolved_main_extent() {
        // A Fixed{120q = 15px} child of intrinsic 10px: its PLACED OBB main span must
        // be the pinned 15px (scaled up), proving Fixed bakes a real scale.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Fixed { value: 120 }, h: AxisSizing::Hug },
        ));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let span = placed_obb_main_span(&out[0].1, LayoutAxis::Horizontal, 80, 40);
        assert!((span - 15.0).abs() < 1e-9, "Fixed child pins to 15px, got {span}");
    }

    #[test]
    fn rotated_fill_child_scales_along_the_container_main_axis() {
        // A 90°-rotated Fill child: scaling must be ALONG THE CONTAINER MAIN AXIS, so
        // its OBB main span hits the resolved share (15px) — not its rotated-local axis.
        // Pinned 25px container, Fill A + Hug 10px B; A's share = 15px.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::Start, 200);
        scene.objects.push(g);
        let mut a = sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Fill, h: AxisSizing::Hug },
        );
        a.transform = rotate_z(std::f64::consts::FRAC_PI_2);
        scene.objects.push(a);
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let placed = &out.iter().find(|(id, _)| id == "a").unwrap().1;
        // The rotated rect's intrinsic main extent is 5px (its 5px-tall side rotated
        // into the horizontal axis); the Fill grows it to 15px along container main.
        let span = placed_obb_main_span(placed, LayoutAxis::Horizontal, 80, 40);
        assert!(
            (span - 15.0).abs() < 1e-6,
            "rotated Fill child's container-main OBB span = 15px, got {span}"
        );
    }

    #[test]
    fn hug_child_keeps_intrinsic_extent_no_scale() {
        // A Hug child must NOT be scaled: its placed OBB main span stays its intrinsic
        // 10px (k=1, S=I) — the resize path is gated to Fill/Fixed only.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let span = placed_obb_main_span(&out[0].1, LayoutAxis::Horizontal, 80, 40);
        assert!((span - 10.0).abs() < 1e-9, "Hug child stays 10px, got {span}");
    }

    #[test]
    fn resize_is_zero_rebake_geometry_byte_identical() {
        // Baking the main-axis scale must touch only the transform: the child's stored
        // Geometry/path_string is byte-identical before and after solve.
        let mut scene = ObjectScene::default();
        let (g, _) = pinned_layout(LayoutAxis::Horizontal, 0, MainAlign::Start, 200);
        scene.objects.push(g);
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Fill, h: AxisSizing::Hug },
        ));
        let before = scene.get("a").unwrap().geometry.clone();

        let _ = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let after = scene.get("a").unwrap().geometry.clone();
        assert_eq!(before.path_string, after.path_string, "geometry path_string unchanged");
        assert_eq!(before, after, "geometry byte-identical (transform-only resize)");
    }

    // --- child CROSS-axis transform-scale resize -----------------------------

    #[test]
    fn fixed_cross_child_scales_to_its_resolved_cross_extent() {
        // Horizontal axis (cross = Y). A 10x5px rect with Fixed-cross 120q (15px): its
        // placed OBB cross span must be the resolved 15px, not its intrinsic 5px.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Hug, h: AxisSizing::Fixed { value: 120 } },
        ));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let span = placed_obb_cross_span(&out[0].1, LayoutAxis::Horizontal, 80, 40);
        assert!((span - 15.0).abs() < 1e-9, "Fixed-cross child scales to 15px, got {span}");
    }

    #[test]
    fn rotated_fixed_cross_child_scales_along_container_cross_axis() {
        // A 90°-rotated 10x5px rect: rotation maps its 10px local width into the
        // container cross axis (Y), so its intrinsic cross is 10px. Fixed-cross 160q
        // (20px) => the placed cross span hits 20px ALONG THE CONTAINER axis, proving
        // the scale lands in container space, not the rotated-local axis.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        let mut a = sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Hug, h: AxisSizing::Fixed { value: 160 } },
        );
        a.transform = rotate_z(std::f64::consts::FRAC_PI_2);
        scene.objects.push(a);

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let span = placed_obb_cross_span(&out[0].1, LayoutAxis::Horizontal, 80, 40);
        assert!(
            (span - 20.0).abs() < 1e-6,
            "rotated Fixed-cross child's container-cross OBB span = 20px, got {span}"
        );
    }

    #[test]
    fn fixed_cross_child_seat_and_alignment_correct() {
        // Center-aligned cross: a tall Hug child (5px) sets the band, a Fixed-cross
        // child scaled to 2.5px (20q) centers in it. Its placed OBB cross MIN must be
        // cross_cursor(spacing 0) + (band 5 - target 2.5)/2 = 1.25px — proving both
        // the offset_cross*k_cross seating AND free_cross-uses-cross_target.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Center)));
        scene.objects.push(child("tall", "a0", "grp", rect(80, 40)));
        scene.objects.push(sized_child(
            "short",
            "a1",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Hug, h: AxisSizing::Fixed { value: 20 } },
        ));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let short = &out.iter().find(|(id, _)| id == "short").unwrap().1;
        // The placed OBB min-corner cross coord = the transform translation plus the
        // scaled offset; offset_cross is 0 for an origin-anchored rect, so the min is
        // the translation. Measure it directly.
        let mut probe = Object::new("probe", "a0", rect(80, 40));
        probe.transform = *short;
        let aabb_px = (0.0, 0.0, to_px(80), to_px(40));
        let (_, _, c_lo, _) = obb_extent(&probe, LayoutAxis::Horizontal, aabb_px);
        let cross_min = short.m[1][2] + c_lo;
        assert!((cross_min - 1.25).abs() < 1e-9, "centered scaled cross min = 1.25px, got {cross_min}");
        let span = placed_obb_cross_span(short, LayoutAxis::Horizontal, 80, 40);
        assert!((span - 2.5).abs() < 1e-9, "short scaled to 2.5px, got {span}");
    }

    #[test]
    fn cross_resize_is_zero_rebake_geometry_byte_identical() {
        // Baking the cross scale must touch only the transform: the child's stored
        // Geometry is byte-identical before and after solve.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        scene.objects.push(sized_child(
            "a",
            "a0",
            "grp",
            rect(80, 40),
            Sizing { w: AxisSizing::Hug, h: AxisSizing::Fixed { value: 120 } },
        ));
        let before = scene.get("a").unwrap().geometry.clone();

        let _ = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let after = scene.get("a").unwrap().geometry.clone();
        assert_eq!(before.path_string, after.path_string, "geometry path_string unchanged");
        assert_eq!(before, after, "geometry byte-identical (transform-only cross resize)");
    }

    // --- lanes: grid + wrap --------------------------------------------------

    #[test]
    fn count_two_tracks_block_distributes_into_grid() {
        // Count { value: 2 } => 2 cross-axis tracks. 4 children block-distribute
        // 2-per-track: track 0 = {a,b} (cross y = 0), track 1 = {c,d} (next band).
        let mut scene = ObjectScene::default();
        let mut g = group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start));
        if let Some(l) = g.layout.as_mut() {
            l.lanes = Lanes::Count { value: 2 };
        }
        scene.objects.push(g);
        for (id, order) in [("a", "a0"), ("b", "a1"), ("c", "a2"), ("d", "a3")] {
            scene.objects.push(child(id, order, "grp", rect(80, 40)));
        }

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let pos: std::collections::HashMap<&str, (f64, f64)> =
            out.iter().map(|(id, t)| (id.as_str(), translate_of(t))).collect();
        // Track 0: a at x=0, b at x=10, both y=0.
        assert!(pos["a"].0.abs() < 1e-9 && pos["a"].1.abs() < 1e-9);
        assert!((pos["b"].0 - 10.0).abs() < 1e-9 && pos["b"].1.abs() < 1e-9);
        // Track 1: c at x=0, d at x=10, both y = band(5px).
        assert!(pos["c"].0.abs() < 1e-9 && (pos["c"].1 - 5.0).abs() < 1e-9);
        assert!((pos["d"].0 - 10.0).abs() < 1e-9 && (pos["d"].1 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn fill_lanes_wrap_when_line_exceeds_container_main() {
        // Pinned inner main = 22px (176q), each child 10px, spacing 0. Two fit
        // (20 <= 22); the third wraps. So a,b on line 0 (y=0), c on line 1 (y=5).
        let mut scene = ObjectScene::default();
        let mut g = group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start));
        if let Some(l) = g.layout.as_mut() {
            l.lanes = Lanes::Fill;
        }
        g.sizing = Some(Sizing { w: AxisSizing::Fixed { value: 176 }, h: AxisSizing::Hug });
        scene.objects.push(g);
        for (id, order) in [("a", "a0"), ("b", "a1"), ("c", "a2")] {
            scene.objects.push(child(id, order, "grp", rect(80, 40)));
        }

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let pos: std::collections::HashMap<&str, (f64, f64)> =
            out.iter().map(|(id, t)| (id.as_str(), translate_of(t))).collect();
        assert!(pos["a"].1.abs() < 1e-9 && pos["b"].1.abs() < 1e-9, "a,b on first line");
        assert!((pos["b"].0 - 10.0).abs() < 1e-9, "b packs after a");
        assert!((pos["c"].1 - 5.0).abs() < 1e-9, "c wrapped to second line");
        assert!(pos["c"].0.abs() < 1e-9, "c restarts at line inset");
    }

    #[test]
    fn fill_lanes_single_line_when_container_hugs() {
        // No pin => Fill wrap has no constraint => one line (all on y=0).
        let mut scene = ObjectScene::default();
        let mut g = group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start));
        if let Some(l) = g.layout.as_mut() {
            l.lanes = Lanes::Fill;
        }
        scene.objects.push(g);
        for (id, order) in [("a", "a0"), ("b", "a1"), ("c", "a2")] {
            scene.objects.push(child(id, order, "grp", rect(80, 40)));
        }

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        for (_, t) in &out {
            assert!(translate_of(t).1.abs() < 1e-9, "all on a single line");
        }
    }

    // --- rotated child OBB packing -------------------------------------------

    fn rotate_z(rad: f64) -> Transform3x3 {
        let (s, c) = rad.sin_cos();
        Transform3x3 { m: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]] }
    }

    #[test]
    fn rotated_child_packs_by_its_rotated_aabb_extent() {
        // A 10x5px rect rotated 90°: its rotated AABB is 5px wide × 10px tall, so on
        // a horizontal axis it occupies 5px of main (not its 10px local width). The
        // next child packs after 5px, proving OBB (not local-AABB) packing.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        let mut rotated = child("r", "a0", "grp", rect(80, 40));
        rotated.transform = rotate_z(std::f64::consts::FRAC_PI_2);
        scene.objects.push(rotated);
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let bx = out.iter().find(|(id, _)| id == "b").unwrap().1;
        assert!(
            (translate_of(&bx).0 - 5.0).abs() < 1e-6,
            "b must pack after the rotated child's 5px main extent, got {}",
            translate_of(&bx).0
        );
    }

    #[test]
    fn rotated_child_preserves_its_linear_transform_in_the_result() {
        // The derived transform keeps the child's rotation/scale; only translation
        // is replaced by the placement.
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 0, CrossAlign::Start)));
        let mut rotated = child("r", "a0", "grp", rect(80, 40));
        let rot = rotate_z(std::f64::consts::FRAC_PI_2);
        rotated.transform = rot;
        scene.objects.push(rotated);

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let m = out[0].1.m;
        // Linear 2x2 == the rotation; only the translation column differs.
        assert!((m[0][0] - rot.m[0][0]).abs() < 1e-12 && (m[0][1] - rot.m[0][1]).abs() < 1e-12);
        assert!((m[1][0] - rot.m[1][0]).abs() < 1e-12 && (m[1][1] - rot.m[1][1]).abs() < 1e-12);
    }

    #[test]
    fn deterministic_resolve() {
        let mut scene = ObjectScene::default();
        scene
            .objects
            .push(group_with_layout("grp", layout(LayoutAxis::Horizontal, 16, CrossAlign::Center)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 20)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let again = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert_eq!(out, again, "auto-layout solve must be deterministic");
    }
}
