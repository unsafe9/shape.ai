//! The ONE shared placement helper. Both `render` (emit) and `hit` (collect)
//! call `layout_children`, so a drawn box and its hit box can never drift.

use crate::widget::{Axis, Container, CrossAlign, MainAlign, Widget};

/// Intrinsic box size of a widget for the layout cursor. Boxes carry explicit
/// w/h (no text measurement in P2); a Container reports its own w/h.
pub fn measure(w: &Widget) -> (f64, f64) {
    match w {
        Widget::Container(c) => (c.w, c.h),
        Widget::Rect(r) => (r.w, r.h),
        Widget::Icon(i) => (i.w, i.h),
        Widget::Text(t) => (t.w, t.h),
        Widget::Button(b) => (b.w, b.h),
        Widget::Swatch(s) => (s.w, s.h),
        Widget::Toggle(t) => (t.w, t.h),
        Widget::Slider(s) => (s.w, s.h),
        Widget::Segment(s) => (s.w, s.h),
        Widget::TextInput(t) => (t.w, t.h),
    }
}

/// Each child paired with its laid-out TOP-LEFT relative to the container origin
/// (the caller adds the container's absolute origin). The child arm then places
/// AT this origin (it does not re-add its own x/y), so this is the single source
/// of position render and hit share — drawn and hit boxes can never drift.
/// - `Axis::None`: child kept at its own (child.x, child.y) (today's behavior).
/// - `Horizontal`/`Vertical`: cursor starts at `padding.{l,t}`, advances by the
///   child main-extent + the inter-child gap; the cross-offset comes from `align`
///   over `(container cross-size − 2*padding − child cross-extent)`. The gap is the
///   declared `spacing` under `MainAlign::Start`, or the slack split evenly between
///   children under `SpaceBetween` (a label|value row pins to both edges).
pub(crate) fn layout_children(c: &Container) -> Vec<(&Widget, f64, f64)> {
    match c.direction {
        Axis::None => c
            .children
            .iter()
            .map(|child| {
                let (cx, cy) = origin(child);
                (child, cx, cy)
            })
            .collect(),
        Axis::Horizontal => {
            let mut out = Vec::with_capacity(c.children.len());
            let cross_extent = c.h - c.padding.t - c.padding.b;
            let main_extent = c.w - c.padding.l - c.padding.r;
            let gap = main_gap(c, main_extent, |w| measure(w).0);
            let mut cursor = c.padding.l;
            for child in &c.children {
                let (cw, ch) = measure(child);
                let cross = c.padding.t + cross_offset(c.align, cross_extent, ch);
                out.push((child, cursor, cross));
                cursor += cw + gap;
            }
            out
        }
        Axis::Vertical => {
            let mut out = Vec::with_capacity(c.children.len());
            let cross_extent = c.w - c.padding.l - c.padding.r;
            let main_extent = c.h - c.padding.t - c.padding.b;
            let gap = main_gap(c, main_extent, |w| measure(w).1);
            let mut cursor = c.padding.t;
            for child in &c.children {
                let (cw, ch) = measure(child);
                let cross = c.padding.l + cross_offset(c.align, cross_extent, cw);
                out.push((child, cross, cursor));
                cursor += ch + gap;
            }
            out
        }
    }
}

/// The inter-child main-axis gap. `Start` uses the declared `spacing`; `SpaceBetween`
/// spreads the leftover main-axis space (`main_extent − Σ child main-extents`) evenly
/// across the `n−1` gaps, so the first child pins to the leading edge and the last to
/// the trailing edge. With one child (no gap) it falls back to `spacing` (inert).
fn main_gap(c: &Container, main_extent: f64, child_main: impl Fn(&Widget) -> f64) -> f64 {
    match c.main_align {
        MainAlign::Start => c.spacing,
        MainAlign::SpaceBetween => {
            let gaps = c.children.len().saturating_sub(1);
            if gaps == 0 {
                return c.spacing;
            }
            let used: f64 = c.children.iter().map(&child_main).sum();
            ((main_extent - used) / gaps as f64).max(0.0)
        }
    }
}

/// A widget's own declared top-left. `layout_children` uses it for `Axis::None`;
/// `render`/`hit` use it to seed the root origin (the recursion then places each
/// widget AT the offset, never re-adding its own x/y).
pub(crate) fn origin(child: &Widget) -> (f64, f64) {
    match child {
        Widget::Container(c) => (c.x, c.y),
        Widget::Rect(r) => (r.x, r.y),
        Widget::Icon(i) => (i.x, i.y),
        Widget::Text(t) => (t.x, t.y),
        Widget::Button(b) => (b.x, b.y),
        Widget::Swatch(s) => (s.x, s.y),
        Widget::Toggle(t) => (t.x, t.y),
        Widget::Slider(s) => (s.x, s.y),
        Widget::Segment(s) => (s.x, s.y),
        Widget::TextInput(t) => (t.x, t.y),
    }
}

fn cross_offset(align: CrossAlign, cross_extent: f64, child_cross: f64) -> f64 {
    match align {
        CrossAlign::Start => 0.0,
        CrossAlign::Center => (cross_extent - child_cross) / 2.0,
        CrossAlign::End => cross_extent - child_cross,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::{Edges, Rect, RectStyle};

    fn rect(id: &str, w: f64, h: f64) -> Widget {
        Widget::Rect(Rect {
            id: id.to_string(),
            x: 7.0,
            y: 9.0,
            w,
            h,
            style: RectStyle::default(),
            hoverable: false,
        })
    }

    fn container(direction: Axis, align: CrossAlign, children: Vec<Widget>) -> Container {
        Container {
            id: "c".to_string(),
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 40.0,
            direction,
            spacing: 10.0,
            main_align: MainAlign::Start,
            padding: Edges::all(8.0),
            align,
            clip: false,
            children,
        }
    }

    #[test]
    fn horizontal_container_places_children_with_padding_and_spacing() {
        let c = container(
            Axis::Horizontal,
            CrossAlign::Center,
            vec![rect("a", 20.0, 20.0), rect("b", 20.0, 20.0)],
        );
        let laid = layout_children(&c);
        assert_eq!(laid.len(), 2);
        // child[0]: cursor at padding.l=8; cross centered over (40 - 16 - 20)/2 = 2,
        // offset by padding.t=8 ⇒ y = 10.
        let (_, x0, y0) = laid[0];
        assert_eq!(x0, 8.0);
        assert_eq!(y0, 10.0);
        // child[1].x = 8 + 20 + spacing(10) = 38.
        let (_, x1, _) = laid[1];
        assert_eq!(x1, 8.0 + 20.0 + 10.0);

        assert_eq!(measure(&rect("m", 20.0, 30.0)), (20.0, 30.0));
    }

    /// `SpaceBetween` pins the first child to the leading edge and the last to the
    /// trailing edge, splitting the slack into the gap. FAILS if it ever regresses to
    /// the packed `spacing` layout (the value would sit mid-row, not right-pinned).
    #[test]
    fn space_between_pins_children_to_both_main_edges() {
        let mut c = container(
            Axis::Horizontal,
            CrossAlign::Center,
            vec![rect("label", 40.0, 20.0), rect("value", 60.0, 20.0)],
        );
        c.main_align = MainAlign::SpaceBetween;
        // padding 8 each side ⇒ main_extent = 200 - 16 = 184; used = 40 + 60 = 100;
        // single gap = 84. label pins left at padding.l = 8.
        let laid = layout_children(&c);
        let (_, x0, _) = laid[0];
        let (_, x1, _) = laid[1];
        assert_eq!(x0, 8.0);
        // value's RIGHT edge pins to the trailing inner edge: 200 - padding.r 8 = 192.
        assert_eq!(x1 + 60.0, 192.0);
        // the gap is the slack, not the declared spacing(10).
        assert_eq!(x1 - (x0 + 40.0), 84.0);
    }

    #[test]
    fn axis_none_keeps_absolute_positions() {
        let c = container(Axis::None, CrossAlign::Start, vec![rect("a", 20.0, 20.0)]);
        let laid = layout_children(&c);
        // back-compat: returned at the child's own declared (x,y).
        let (_, x, y) = laid[0];
        assert_eq!((x, y), (7.0, 9.0));
        assert_eq!(origin(&rect("z", 1.0, 1.0)), (7.0, 9.0));
    }
}
