//! `hit(tree, screen_pt) -> Option<WidgetId>` — top-most-first containment pick in
//! SCREEN px (the same coords `render` places widgets at). This is the
//! host-testable falsifiable surface for the pick logic; the renderer-side `hitUi`
//! (which uses `derive_object_regions` + identity camera) is the GPU-path mirror.

use crate::layout::{layout_children, origin};
use crate::widget::{Widget, WidgetId};

/// Returns the top-most hittable widget id at `screen_pt`, or None. Hittable =
/// the OWNER box of an interactive widget (Rect, Button, Swatch, Toggle, Slider,
/// Segment, TextInput) — never a synthetic `::part`; Text is non-interactive; a
/// Container is non-hittable (only its children). Later in tree order draws on
/// top, so it wins (mirrors `hit_object_in_regions` reverse iteration).
pub fn hit(tree: &Widget, screen_pt: (f64, f64)) -> Option<WidgetId> {
    let mut hits: Vec<(WidgetId, f64, f64, f64, f64)> = Vec::new();
    let (ox, oy) = origin(tree);
    collect(tree, ox, oy, &mut hits);
    let (px, py) = screen_pt;
    hits.iter()
        .rev()
        .find(|(_, sx, sy, w, h)| px >= *sx && px < *sx + *w && py >= *sy && py < *sy + *h)
        .map(|(id, ..)| id.clone())
}

/// The RESOLVED screen box `(off_x, off_y, w, h)` of `id` — the same accumulated
/// origin `hit`/`collect` place the widget at, NOT its declared `x/y`. Slider/segment
/// value math must read this so a widget nested in an offset/flex container maps
/// pt.x→value against where it actually renders. Top-most (last-drawn) wins.
pub(crate) fn resolved_box(tree: &Widget, id: &WidgetId) -> Option<(f64, f64, f64, f64)> {
    let mut hits: Vec<(WidgetId, f64, f64, f64, f64)> = Vec::new();
    let (ox, oy) = origin(tree);
    collect(tree, ox, oy, &mut hits);
    hits.iter()
        .rev()
        .find(|(hid, ..)| hid == id)
        .map(|(_, sx, sy, w, h)| (*sx, *sy, *w, *h))
}

fn collect(widget: &Widget, off_x: f64, off_y: f64, out: &mut Vec<(WidgetId, f64, f64, f64, f64)>) {
    // `off_x/off_y` is the widget's resolved screen origin (the caller added any
    // container/cursor offset). An arm records its box AT (off_x, off_y), never
    // re-adding its own x/y — mirrors the render `emit` contract exactly.
    match widget {
        Widget::Container(c) => {
            for (child, cx, cy) in layout_children(c) {
                collect(child, off_x + cx, off_y + cy, out);
            }
        }
        Widget::Rect(r) => out.push((r.id.clone(), off_x, off_y, r.w, r.h)),
        Widget::Button(b) => out.push((b.id.clone(), off_x, off_y, b.w, b.h)),
        Widget::Swatch(s) => out.push((s.id.clone(), off_x, off_y, s.w, s.h)),
        Widget::Toggle(t) => out.push((t.id.clone(), off_x, off_y, t.w, t.h)),
        Widget::Slider(s) => out.push((s.id.clone(), off_x, off_y, s.w, s.h)),
        Widget::Segment(s) => out.push((s.id.clone(), off_x, off_y, s.w, s.h)),
        Widget::TextInput(t) => out.push((t.id.clone(), off_x, off_y, t.w, t.h)),
        // Icon and Text are presentation only — the control body underneath is the
        // hit target, so neither contributes its own pick box.
        Widget::Icon(_) | Widget::Text(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::{
        Axis, Button, Container, CrossAlign, Edges, MainAlign, Paint, Rect, RectStyle, Text,
        TextPaint,
    };

    fn absolute(id: &str, x: f64, y: f64, children: Vec<Widget>) -> Container {
        Container {
            id: id.to_string(),
            x,
            y,
            w: 0.0,
            h: 0.0,
            direction: Axis::None,
            spacing: 0.0,
            main_align: MainAlign::Start,
            padding: Edges::all(0.0),
            align: CrossAlign::Start,
            children,
        }
    }

    fn button(id: &str, x: f64, y: f64, w: f64, h: f64) -> Widget {
        Widget::Button(Button {
            id: id.to_string(),
            x,
            y,
            w,
            h,
            label: "L".to_string(),
            style: RectStyle {
                fill: Some(Paint::Token("surface".to_string())),
                stroke: None,
                corner_radius: 12.0,
                opacity: 1.0,
            },
            label_size_px: 16.0,
            label_color: TextPaint::Hex("#ffffff".to_string()),
        })
    }

    #[test]
    fn hit_returns_top_most_button_body_id_in_screen_space() {
        let tree = button("ui-proof-button", 24.0, 24.0, 140.0, 40.0);
        assert_eq!(
            hit(&tree, (30.0, 30.0)),
            Some("ui-proof-button".to_string())
        );
        assert_eq!(hit(&tree, (10.0, 10.0)), None);

        let overlap = Widget::Container(absolute(
            "root",
            0.0,
            0.0,
            vec![
                button("under", 0.0, 0.0, 100.0, 100.0),
                button("over", 0.0, 0.0, 100.0, 100.0),
            ],
        ));
        assert_eq!(hit(&overlap, (50.0, 50.0)), Some("over".to_string()));

        let with_text = Widget::Container(absolute(
            "panel",
            0.0,
            0.0,
            vec![
                Widget::Text(Text {
                    id: "the-text".to_string(),
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                    label: "hi".to_string(),
                    size_px: 16.0,
                    color: TextPaint::Hex("#000000".to_string()),
                    align_center: true,
                }),
                button("the-button", 0.0, 0.0, 100.0, 100.0),
            ],
        ));
        let pick = hit(&with_text, (50.0, 50.0));
        assert_eq!(pick, Some("the-button".to_string()));
        assert!(pick.as_deref() != Some("the-button::label"));
        assert!(pick.as_deref() != Some("panel"));
    }

    #[test]
    fn hit_respects_container_offset() {
        let tree = Widget::Container(absolute(
            "panel",
            100.0,
            100.0,
            vec![Widget::Rect(Rect {
                id: "child".to_string(),
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 40.0,
                style: RectStyle::default(),
                hoverable: false,
            })],
        ));
        assert_eq!(hit(&tree, (110.0, 110.0)), Some("child".to_string()));
        assert_eq!(hit(&tree, (10.0, 10.0)), None);
    }
}
