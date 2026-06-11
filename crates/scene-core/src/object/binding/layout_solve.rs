//! OB3.A1 — thin auto-layout solve (D3 tier-3).
//!
//! A children-group [`Object`] carrying `layout: Some(Layout)` arranges the
//! objects whose `parent == group.id` along a main axis (row => x, column => y),
//! separated by `gap`, offset by `padding`, and aligned on the cross axis per
//! [`LayoutAlign`]. The result is **derived, not stored**: this returns a fresh
//! `(ObjectId, Transform3x3)` per child for the caller to apply at draw time; it
//! never mutates the scene's stored geometry or transforms (P4 zero-rebake — a
//! layout position is a transform, geometry is untouched).
//!
//! Pure: no IO/time/rng. Inputs are object-local quantized i32 (D2); main- and
//! cross-axis offsets are converted to logical px (divide by
//! [`GEOMETRY_QUANTUM_PER_PX`]) for the [`Transform3x3`] translate, since a
//! transform operates in logical px (not quantized units).
//!
//! Sizing note (kept thin): only [`LayoutSizing::Hug`] is solved precisely —
//! the group packs tight to its children's content (the group has no stored box
//! size in this slice, so there is nothing to hug *to* but the content itself).
//! [`LayoutSizing::Fixed`] and [`LayoutSizing::Fill`] both need a group box that
//! this slice does not carry, so they are treated identically to `Hug` (content
//! packing); growing children to fill a fixed/parent box is deferred. Likewise
//! [`LayoutAlign::Stretch`] would resize a child's cross extent, which this
//! transform-only slice cannot express, so it falls back to `Start` alignment.

use crate::object::model::{
    LayoutAlign, LayoutDirection, ObjectId, ObjectScene, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::region::{LocalBounds, OutlineDeriver};

/// Quantized-units -> logical px. Exact for the values used here (small integer
/// extents over a power-of-two quantum), and never lossy in a width sense.
fn to_px(q: i32) -> f64 {
    f64::from(q) / f64::from(GEOMETRY_QUANTUM_PER_PX)
}

/// One child's derived box, in quantized units. Width/height are the region
/// AABB extents; `min_x`/`min_y` are the AABB origin so a translate can place
/// the *content* (not the local origin) at a target offset.
struct ChildBox {
    id: ObjectId,
    order: String,
    width: i32,
    height: i32,
    min_x: i32,
    min_y: i32,
}

impl ChildBox {
    fn from_bounds(id: ObjectId, order: String, b: LocalBounds) -> Self {
        ChildBox {
            id,
            order,
            width: b.max_x - b.min_x,
            height: b.max_y - b.min_y,
            min_x: b.min_x,
            min_y: b.min_y,
        }
    }

    /// Extent (quantized) along the main axis for the given direction.
    fn main_extent(&self, dir: LayoutDirection) -> i32 {
        match dir {
            LayoutDirection::Row => self.width,
            LayoutDirection::Column => self.height,
        }
    }

    /// Extent (quantized) along the cross axis for the given direction.
    fn cross_extent(&self, dir: LayoutDirection) -> i32 {
        match dir {
            LayoutDirection::Row => self.height,
            LayoutDirection::Column => self.width,
        }
    }
}

/// Solve the auto-layout for the group `group_id`, returning the derived
/// transform for each child (objects whose `parent == group_id`), in child
/// paint order (fractional `order`, ties broken by id for determinism).
///
/// Returns an empty vec when the group is missing, has no `layout`, or has no
/// children. Children whose geometry yields no derivable region (empty /
/// degenerate) are skipped — they cannot occupy main-axis space.
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

    // Gather children with a derivable region, in canonical paint order.
    let mut children: Vec<ChildBox> = scene
        .objects
        .iter()
        .filter(|o| o.parent.as_deref() == Some(group_id))
        .filter_map(|o| {
            deriver
                .derive_region(&o.geometry, 1)
                .ok()
                .map(|r| ChildBox::from_bounds(o.id.clone(), o.order.clone(), r.bounds))
        })
        .collect();
    children.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));

    if children.is_empty() {
        return Vec::new();
    }

    let dir = layout.direction;
    let padding_px = to_px(layout.padding);
    let gap_px = to_px(layout.gap);

    // Cross-axis span (quantized) = the largest child cross-extent. Used to
    // center/end-align each child within the packed band.
    let cross_span = children.iter().map(|c| c.cross_extent(dir)).max().unwrap_or(0);

    let mut out: Vec<(ObjectId, Transform3x3)> = Vec::with_capacity(children.len());
    let mut main_cursor = padding_px;
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            main_cursor += gap_px;
        }

        // Cross offset within the band, per alignment. Stretch falls back to
        // Start (a transform cannot resize the child's cross extent).
        let free_cross = to_px(cross_span - child.cross_extent(dir));
        let cross_offset = padding_px
            + match layout.align {
                LayoutAlign::Start | LayoutAlign::Stretch => 0.0,
                LayoutAlign::Center => free_cross / 2.0,
                LayoutAlign::End => free_cross,
            };

        // The child's content origin sits at `bounds.min` in its local space;
        // subtract it so the *content* lands at the cursor/offset, not the
        // local origin.
        let (tx, ty) = match dir {
            LayoutDirection::Row => (main_cursor - to_px(child.min_x), cross_offset - to_px(child.min_y)),
            LayoutDirection::Column => {
                (cross_offset - to_px(child.min_x), main_cursor - to_px(child.min_y))
            }
        };

        out.push((child.id.clone(), Transform3x3::translate(tx, ty)));
        main_cursor += to_px(child.main_extent(dir));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, Layout, LayoutSizing, Object, PathNode, SubPath};
    use crate::object::region::StubOutlineDeriver;

    /// A closed unit rect spanning (0,0)-(w,h) in quantized units, origin at 0.
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

    /// Translate components in logical px from a pure-translate transform.
    fn translate_of(t: &Transform3x3) -> (f64, f64) {
        (t.m[0][2], t.m[1][2])
    }

    fn group_with_layout(id: &str, layout: Layout) -> Object {
        let mut g = Object::new(id, "g0", Geometry::default());
        g.layout = Some(layout);
        g
    }

    #[test]
    fn row_packs_children_by_width_plus_gap() {
        // 3 unit-rects (80x40 quantized = 10x5 px), gap 16 quantized = 2 px,
        // padding 0. Row => children spaced along x by width(10px)+gap(2px).
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                direction: LayoutDirection::Row,
                gap: 16,
                padding: 0,
                align: LayoutAlign::Start,
                sizing: LayoutSizing::Hug,
            },
        ));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));
        scene.objects.push(child("c", "a2", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        assert_eq!(out.len(), 3);

        // x translations increase, spaced by width(10) + gap(2) = 12 px.
        let xs: Vec<f64> = out.iter().map(|(_, t)| translate_of(t).0).collect();
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
        assert_eq!(out[2].0, "c");
        assert!((xs[0] - 0.0).abs() < 1e-9, "first x = {}", xs[0]);
        assert!((xs[1] - 12.0).abs() < 1e-9, "second x = {}", xs[1]);
        assert!((xs[2] - 24.0).abs() < 1e-9, "third x = {}", xs[2]);
        // Row Start align: every child y stays at 0.
        for (_, t) in &out {
            assert!((translate_of(t).1).abs() < 1e-9);
        }
    }

    #[test]
    fn padding_offsets_main_axis_start() {
        // padding 8 quantized = 1 px; first child's main offset is the padding.
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                direction: LayoutDirection::Row,
                gap: 0,
                padding: 8,
                align: LayoutAlign::Start,
                sizing: LayoutSizing::Hug,
            },
        ));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let (x0, y0) = translate_of(&out[0].1);
        let (x1, _) = translate_of(&out[1].1);
        assert!((x0 - 1.0).abs() < 1e-9, "padded start x = {x0}");
        // Cross axis is also offset by padding (Start band origin = padding).
        assert!((y0 - 1.0).abs() < 1e-9, "padded cross y = {y0}");
        // Second child: padding(1) + width(10) = 11 px.
        assert!((x1 - 11.0).abs() < 1e-9, "second x = {x1}");
    }

    #[test]
    fn column_packs_children_by_height_plus_gap() {
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                direction: LayoutDirection::Column,
                gap: 16,
                padding: 0,
                align: LayoutAlign::Start,
                sizing: LayoutSizing::Hug,
            },
        ));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        // Column => y increases by height(5px) + gap(2px) = 7 px; x stays 0.
        let (x0, y0) = translate_of(&out[0].1);
        let (x1, y1) = translate_of(&out[1].1);
        assert!((y0 - 0.0).abs() < 1e-9);
        assert!((y1 - 7.0).abs() < 1e-9, "second y = {y1}");
        assert!(x0.abs() < 1e-9 && x1.abs() < 1e-9);
    }

    #[test]
    fn center_align_centers_smaller_child_on_cross_axis() {
        // Row with a tall child (height 40q=5px) and a short one (height 20q=2.5px).
        // Center align puts the short child's cross offset at (5-2.5)/2 = 1.25 px.
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                direction: LayoutDirection::Row,
                gap: 0,
                padding: 0,
                align: LayoutAlign::Center,
                sizing: LayoutSizing::Hug,
            },
        ));
        scene.objects.push(child("tall", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("short", "a1", "grp", rect(80, 20)));

        let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
        let tall_y = translate_of(&out[0].1).1;
        let short_y = translate_of(&out[1].1).1;
        assert!(tall_y.abs() < 1e-9, "tall stays at band top: {tall_y}");
        assert!((short_y - 1.25).abs() < 1e-9, "short centered: {short_y}");
    }

    #[test]
    fn missing_group_or_layout_yields_empty() {
        let scene = ObjectScene::default();
        assert!(solve_layout(&scene, "nope", &StubOutlineDeriver).is_empty());

        let mut scene2 = ObjectScene::default();
        // group object without a layout => empty.
        scene2.objects.push(Object::new("grp", "g0", rect(80, 40)));
        scene2.objects.push(child("a", "a0", "grp", rect(80, 40)));
        assert!(solve_layout(&scene2, "grp", &StubOutlineDeriver).is_empty());
    }

    #[test]
    fn children_ordered_by_fractional_order() {
        // Insertion order is reversed vs. fractional order; output follows order.
        let mut scene = ObjectScene::default();
        scene.objects.push(group_with_layout(
            "grp",
            Layout {
                direction: LayoutDirection::Row,
                gap: 0,
                padding: 0,
                align: LayoutAlign::Start,
                sizing: LayoutSizing::Hug,
            },
        ));
        scene.objects.push(child("c", "a2", "grp", rect(80, 40)));
        scene.objects.push(child("a", "a0", "grp", rect(80, 40)));
        scene.objects.push(child("b", "a1", "grp", rect(80, 40)));

        let ids: Vec<String> = solve_layout(&scene, "grp", &StubOutlineDeriver)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }
}
