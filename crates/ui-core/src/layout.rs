//! The ONE shared placement helper. Both `render` (emit) and `hit` (collect)
//! call `layout_children`, so a drawn box and its hit box can never drift.

use crate::widget::{Axis, Container, CrossAlign, Widget};

/// Intrinsic box size of a widget for the layout cursor. Boxes carry explicit
/// w/h (no text measurement in P2); a Container reports its own w/h.
pub(crate) fn measure(w: &Widget) -> (f64, f64) {
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
///   child main-extent + spacing; the cross-offset comes from `align` over
///   `(container cross-size − 2*padding − child cross-extent)`.
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
            let mut cursor = c.padding.l;
            for child in &c.children {
                let (cw, ch) = measure(child);
                let cross = c.padding.t + cross_offset(c.align, cross_extent, ch);
                out.push((child, cursor, cross));
                cursor += cw + c.spacing;
            }
            out
        }
        Axis::Vertical => {
            let mut out = Vec::with_capacity(c.children.len());
            let cross_extent = c.w - c.padding.l - c.padding.r;
            let mut cursor = c.padding.t;
            for child in &c.children {
                let (cw, ch) = measure(child);
                let cross = c.padding.l + cross_offset(c.align, cross_extent, cw);
                out.push((child, cross, cursor));
                cursor += ch + c.spacing;
            }
            out
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
            padding: Edges::all(8.0),
            align,
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
