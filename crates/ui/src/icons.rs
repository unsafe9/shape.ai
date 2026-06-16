//! The icon registry: a presentation-only map from a command/chrome id to a
//! 24×24 SVG-subset path (absolute `M`/`L`/`C`/`Z`, coords within 0..24),
//! drawn as thin stroke line-art (SF-Symbols feel). This is NOT a scene-core
//! catalog field — icons are pure presentation, keyed off the same id the intent
//! resolver uses, so a toolbar/chrome button looks up its glyph by id without the
//! catalog gaining a presentation column.
//!
//! `emit_icon` (ui-core) scales the 24-box to the icon box, so authoring stays in
//! one fixed grid. Most glyphs are stroked; the registry hands back just the path,
//! and [`icon_in_box`] wraps it in a stroked `Icon` widget centered in a box.

use shape_ui_core::{Icon, Paint, Widget};

/// The default stroke width (in 24-box units) of a registry glyph; `emit_icon`
/// carries it through the scale, so a 24-box `1.8` reads ~1.8px in a 24px icon box.
/// Heavier than a hairline so a `text`-tinted glyph reads CRISP (not a faint thread)
/// on the frosted tray in BOTH themes — the dark chrome read faint at the old 1.6.
const STROKE_W: f64 = 1.8;

/// The side of the square icon box [`icon_in_box`] centers inside its container.
const ICON_BOX: f64 = 24.0;

/// The glyph path for a command/chrome id, in a 24×24 box. `None` when no icon is
/// registered for the id (the caller falls back to a text label).
pub(crate) fn icon_path(id: &str) -> Option<&'static str> {
    REGISTRY.iter().find(|(k, _)| *k == id).map(|(_, d)| *d)
}

/// Wrap the glyph for `id` in a stroked `Icon` widget, centered in the box
/// `(x, y, w, h)` at the fixed 24-unit icon size, painted with `paint` (a theme
/// token). Returns `None` when no glyph is registered, so a caller can fall back to
/// a text label. The widget id is the caller-supplied `widget_id` (the toolbar's
/// `cmd:<id>::icon` etc.) so it never collides with the button body.
pub(crate) fn icon_in_box(
    widget_id: String,
    id: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    paint: Paint,
) -> Option<Widget> {
    let d = icon_path(id)?;
    let ix = x + (w - ICON_BOX) / 2.0;
    let iy = y + (h - ICON_BOX) / 2.0;
    Some(Widget::Icon(Icon {
        id: widget_id,
        x: ix,
        y: iy,
        w: ICON_BOX,
        h: ICON_BOX,
        d: d.to_string(),
        fill: None,
        stroke: Some((paint, STROKE_W)),
    }))
}

/// The id → 24×24 path table. Keys cover every `TOOLBAR_GROUPS` command id plus the
/// three chrome glyph ids (`toggle-theme`, `canvas-new`, `canvas-delete`). Paths
/// use only absolute `M`/`L`/`C`/`Z` with coords in [0,24]; the
/// `icons_cover_every_toolbar_and_chrome_id` test pins both coverage and validity.
const REGISTRY: &[(&str, &str)] = &[
    // --- Tool group ---
    // select-move: NW arrow cursor (classic pointer triangle + tail notch).
    (
        "select-move",
        "M 5 3 L 5 19 L 9 15 L 12 21 L 14 20 L 11 14 L 17 14 Z",
    ),
    // hand-pan: an open hand (palm box with four finger humps + thumb).
    (
        "hand-pan",
        "M 7 12 L 7 7 C 7 6 9 6 9 7 L 9 11 L 9 5 C 9 4 11 4 11 5 L 11 11 L 11 4 C 11 3 13 3 13 4 L 13 11 L 13 6 C 13 5 15 5 15 6 L 15 13 C 15 18 13 21 10 21 C 8 21 7 20 6 18 L 4 14 C 3 13 5 11 6 12 Z",
    ),
    // draw: a pencil (slanted body + nib tip).
    (
        "draw",
        "M 16 3 L 21 8 L 9 20 L 4 21 L 5 16 Z M 14 5 L 19 10",
    ),
    // erase: an eraser block (tilted rounded rectangle) with a wipe line under it.
    (
        "erase",
        "M 4 16 L 12 8 C 13 7 14 7 15 8 L 19 12 C 20 13 20 14 19 15 L 14 20 L 8 20 Z M 4 21 L 20 21",
    ),
    // --- Insert group ---
    // insert-rectangle: a rounded square.
    (
        "insert-rectangle",
        "M 6 4 L 18 4 C 19 4 20 5 20 6 L 20 18 C 20 19 19 20 18 20 L 6 20 C 5 20 4 19 4 18 L 4 6 C 4 5 5 4 6 4 Z",
    ),
    // insert-ellipse: a circle (four kappa cubics, r=9 about center 12,12).
    (
        "insert-ellipse",
        "M 12 3 C 16 3 21 8 21 12 C 21 16 16 21 12 21 C 8 21 3 16 3 12 C 3 8 8 3 12 3 Z",
    ),
    // insert-line: a diagonal line, bottom-left to top-right.
    ("insert-line", "M 4 20 L 20 4"),
    // insert-text: a capital T (top bar + descending stem).
    ("insert-text", "M 5 5 L 19 5 M 12 5 L 12 19"),
    // insert-frame: a frame with corner ticks (crop-mark style).
    (
        "insert-frame",
        "M 7 3 L 7 21 M 17 3 L 17 21 M 3 7 L 21 7 M 3 17 L 21 17",
    ),
    // --- Edit group ---
    // duplicate: two overlapping squares.
    (
        "duplicate",
        "M 4 4 L 14 4 L 14 14 L 4 14 Z M 10 10 L 20 10 L 20 20 L 10 20 Z",
    ),
    // delete: a trash can (lid + handle + bin + two ribs).
    (
        "delete",
        "M 4 6 L 20 6 M 9 6 L 9 4 L 15 4 L 15 6 M 6 6 L 7 20 L 17 20 L 18 6 M 10 9 L 10 17 M 14 9 L 14 17",
    ),
    // group: a dashed bounding box over two squares.
    (
        "group",
        "M 3 3 L 8 3 M 16 3 L 21 3 M 3 21 L 8 21 M 16 21 L 21 21 M 3 3 L 3 8 M 21 3 L 21 8 M 3 16 L 3 21 M 21 16 L 21 21 M 7 7 L 13 7 L 13 13 L 7 13 Z M 11 11 L 17 11 L 17 17 L 11 17 Z",
    ),
    // ungroup: a broken bounding box (gapped corners) over one square.
    (
        "ungroup",
        "M 3 3 L 9 3 M 15 3 L 21 3 M 3 21 L 9 21 M 3 3 L 3 9 M 21 9 L 21 15 M 21 18 L 21 21 L 18 21 M 8 8 L 16 8 L 16 16 L 8 16 Z",
    ),
    // --- History group ---
    // undo: a curved arrow pointing left (arc + arrowhead at the left tip).
    (
        "undo",
        "M 8 8 L 4 12 L 8 16 M 4 12 L 15 12 C 19 12 20 15 20 17 C 20 19 19 20 17 20",
    ),
    // redo: a curved arrow pointing right (mirror of undo).
    (
        "redo",
        "M 16 8 L 20 12 L 16 16 M 20 12 L 9 12 C 5 12 4 15 4 17 C 4 19 5 20 7 20",
    ),
    // --- More group ---
    // open-template-library: a 2×2 grid of rounded cells.
    (
        "open-template-library",
        "M 4 4 L 11 4 L 11 11 L 4 11 Z M 13 4 L 20 4 L 20 11 L 13 11 Z M 4 13 L 11 13 L 11 20 L 4 20 Z M 13 13 L 20 13 L 20 20 L 13 20 Z",
    ),
    // export: a tray (open box) with an up arrow rising out of it.
    (
        "export",
        "M 4 14 L 4 20 L 20 20 L 20 14 M 12 16 L 12 4 M 8 8 L 12 4 L 16 8",
    ),
    // toggle-diagnostics: an activity pulse (ECG zigzag).
    (
        "toggle-diagnostics",
        "M 3 12 L 8 12 L 11 5 L 14 19 L 17 12 L 21 12",
    ),
    // --- View / zoom group ---
    // zoom-out: a magnifier with a minus (lens circle + handle + horizontal bar).
    (
        "zoom-out",
        "M 11 4 C 14 4 18 8 18 11 C 18 14 14 18 11 18 C 8 18 4 14 4 11 C 4 8 8 4 11 4 Z M 16 16 L 21 21 M 8 11 L 14 11",
    ),
    // zoom-in: a magnifier with a plus (lens circle + handle + plus bars).
    (
        "zoom-in",
        "M 11 4 C 14 4 18 8 18 11 C 18 14 14 18 11 18 C 8 18 4 14 4 11 C 4 8 8 4 11 4 Z M 16 16 L 21 21 M 8 11 L 14 11 M 11 8 L 11 14",
    ),
    // zoom-fit: corner-expand arrows inside a frame (four arrows to the corners).
    (
        "zoom-fit",
        "M 4 9 L 4 4 L 9 4 M 15 4 L 20 4 L 20 9 M 20 15 L 20 20 L 15 20 M 9 20 L 4 20 L 4 15 M 4 4 L 9 9 M 20 4 L 15 9 M 20 20 L 15 15 M 4 20 L 9 15",
    ),
    // toggle-fullscreen: 4-corner expand arrows reaching to the box edges.
    (
        "toggle-fullscreen",
        "M 9 4 L 4 4 L 4 9 M 15 4 L 20 4 L 20 9 M 20 15 L 20 20 L 15 20 M 9 20 L 4 20 L 4 15 M 4 4 L 10 10 M 20 4 L 14 10 M 20 20 L 14 14 M 4 20 L 10 14",
    ),
    // --- Chrome glyphs ---
    // toggle-theme: a sun/moon (sun disc with rays + a crescent notch implied by the
    // disc; a single mark reads in both themes). A circle with eight short rays.
    (
        "toggle-theme",
        "M 12 7 C 15 7 17 9 17 12 C 17 15 15 17 12 17 C 9 17 7 15 7 12 C 7 9 9 7 12 7 Z M 12 2 L 12 5 M 12 19 L 12 22 M 2 12 L 5 12 M 19 12 L 22 12 M 5 5 L 7 7 M 17 17 L 19 19 M 19 5 L 17 7 M 7 17 L 5 19",
    ),
    // canvas-new: a plus.
    ("canvas-new", "M 12 4 L 12 20 M 4 12 L 20 12"),
    // canvas-delete: a trash can (same glyph language as `delete`).
    (
        "canvas-delete",
        "M 4 6 L 20 6 M 9 6 L 9 4 L 15 4 L 15 6 M 6 6 L 7 20 L 17 20 L 18 6 M 10 9 L 10 17 M 14 9 L 14 17",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::TOOLBAR_GROUPS;

    /// The chrome glyph ids the registry must cover beyond the toolbar set.
    const CHROME_IDS: &[&str] = &["toggle-theme", "canvas-new", "canvas-delete"];

    /// Parse a registry path the way `emit_icon` does — absolute `M`/`L`/`C`/`Z`,
    /// each command consuming its number arity — and assert every coordinate lands
    /// in [0,24]. Returns `false` on a malformed token, an unknown command, the
    /// wrong arity, or an out-of-box coord, so the coverage test FAILS (not merely
    /// compiles) when a glyph is authored wrong.
    fn path_is_valid_24box(d: &str) -> bool {
        let mut tokens = d.split_whitespace().peekable();
        let mut saw_command = false;
        while let Some(tok) = tokens.next() {
            let arity = match tok {
                "M" | "L" => 2usize,
                "C" => 6,
                "Z" => 0,
                // A bare number with no preceding command, or any other token, is
                // malformed — the emitter would silently drop it, so reject here.
                _ => return false,
            };
            saw_command = true;
            for _ in 0..arity {
                match tokens.next().and_then(|t| t.parse::<f64>().ok()) {
                    Some(v) if (0.0..=24.0).contains(&v) => {}
                    _ => return false,
                }
            }
        }
        saw_command
    }

    #[test]
    fn icons_cover_every_toolbar_and_chrome_id() {
        for id in TOOLBAR_GROUPS.iter().flat_map(|g| g.iter()).chain(CHROME_IDS.iter()) {
            let d = icon_path(id).unwrap_or_else(|| panic!("no icon registered for `{id}`"));
            assert!(
                path_is_valid_24box(d),
                "icon for `{id}` is not a valid M/L/C/Z path with coords in [0,24]: {d}"
            );
        }
    }

    /// A registered id with no glyph (impossible per the coverage test) returns
    /// `None`; an unregistered id also returns `None`. Pins the lookup contract the
    /// placement helper relies on for its text-label fallback.
    #[test]
    fn unregistered_id_has_no_icon() {
        assert!(icon_path("not-a-command").is_none());
    }

    /// The placement helper centers the 24-unit icon box in its container and
    /// stamps the caller's widget id + token paint as a round-cap stroke. FAILS if
    /// the helper stops centering, drops the path, or bakes a non-token paint.
    #[test]
    fn icon_in_box_centers_and_strokes_with_paint() {
        let w = icon_in_box(
            "cmd:undo::icon".to_string(),
            "undo",
            10.0,
            20.0,
            40.0,
            40.0,
            Paint::Token("text".to_string()),
        )
        .expect("undo is registered");
        let Widget::Icon(icon) = w else { panic!("placement helper must emit an Icon") };
        assert_eq!(icon.id, "cmd:undo::icon");
        // Centered: (40-24)/2 = 8 inset on each axis.
        assert_eq!((icon.x, icon.y), (18.0, 28.0));
        assert_eq!((icon.w, icon.h), (ICON_BOX, ICON_BOX));
        assert_eq!(icon.d, icon_path("undo").unwrap());
        assert!(icon.fill.is_none(), "registry glyphs are stroked, not filled");
        match icon.stroke {
            Some((Paint::Token(t), width)) => {
                assert_eq!(t, "text");
                assert_eq!(width, STROKE_W);
            }
            _ => panic!("stroke must be a token paint at the registry width"),
        }
    }

    /// An unregistered id yields no widget, so the caller falls back to its label.
    #[test]
    fn icon_in_box_returns_none_for_unregistered_id() {
        assert!(icon_in_box(
            "x".to_string(),
            "not-a-command",
            0.0,
            0.0,
            24.0,
            24.0,
            Paint::Token("text".to_string()),
        )
        .is_none());
    }
}
