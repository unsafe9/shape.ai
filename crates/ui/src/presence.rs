//! Peer presence cursors. Each live peer renders a cursor glyph + a label pill at an
//! already-projected SCREEN coordinate. World→screen projection stays SHELL-side (it
//! needs the live core camera, per the thin-shell/pure-core split) — the shell feeds
//! screen coords and drops any peer the camera can't project; this layer only lays
//! out the glyph at that coord.
//!
//! Peer colors are per-user LITERALS, not theme tokens, so the cursor glyph and pill
//! paint with `Paint::Solid`/`TextPaint::Hex` (a theme flip does not recolor a peer).

use serde::Deserialize;
use shape_ui_core::{
    Axis, Container, CrossAlign, Edges, Paint, Rect, RectStyle, Text, TextPaint, Widget,
};

/// One peer cursor at a projected screen point. `color` is the peer's literal
/// `#rrggbb`; `label` is a short human-ish id.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerCursor {
    pub user_id: String,
    /// Already-projected screen px (the shell projects through the live camera).
    pub screen: [f64; 2],
    pub color: String,
    pub label: String,
}

const GLYPH: f64 = 18.0;
const PILL_H: f64 = 16.0;
const PILL_PAD: f64 = 6.0;
/// Approximate px-per-char for the label-pill width — the layout only needs a sane
/// pill box; the renderer lays out the actual glyphs inside it.
const CHAR_W: f64 = 6.5;

/// Build the presence layer: one absolute container per peer, each carrying a cursor
/// glyph + a label pill. Non-interactive (no `cmd:`/`insp:` id is emitted).
pub(crate) fn build(peers: &[PeerCursor]) -> Widget {
    let children: Vec<Widget> = peers.iter().map(cursor).collect();
    Widget::Container(Container {
        id: "presence".to_string(),
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

/// One peer: a pointer glyph (an MSDF text glyph in the peer's literal color) + a
/// label pill below it, anchored at the projected screen point.
fn cursor(peer: &PeerCursor) -> Widget {
    let [sx, sy] = peer.screen;
    let pill_w = PILL_PAD * 2.0 + peer.label.chars().count() as f64 * CHAR_W;
    Widget::Container(Container {
        id: format!("presence::{}", peer.user_id),
        x: sx,
        y: sy,
        w: GLYPH.max(pill_w),
        h: GLYPH + PILL_H + 2.0,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![
            // The cursor glyph: a Text-as-glyph in the peer's literal color.
            Widget::Text(Text {
                id: format!("presence::{}::glyph", peer.user_id),
                x: 0.0,
                y: 0.0,
                w: GLYPH,
                h: GLYPH,
                label: "◤".to_string(),
                size_px: GLYPH,
                color: TextPaint::Hex(peer.color.clone()),
                align_center: false,
            }),
            // The label pill: a peer-colored rounded rect body + the (white) label.
            Widget::Rect(Rect {
                id: format!("presence::{}::pill-bg", peer.user_id),
                x: GLYPH * 0.5,
                y: GLYPH,
                w: pill_w,
                h: PILL_H,
                style: RectStyle {
                    fill: Some(Paint::Solid(peer.color.clone())),
                    stroke: None,
                    corner_radius: PILL_H / 2.0,
                    opacity: 1.0,
                },
                hoverable: false,
            }),
            Widget::Text(Text {
                id: format!("presence::{}::label", peer.user_id),
                x: GLYPH * 0.5 + PILL_PAD,
                y: GLYPH,
                w: pill_w - PILL_PAD * 2.0,
                h: PILL_H,
                label: peer.label.clone(),
                size_px: 11.0,
                color: TextPaint::Hex("#ffffff".to_string()),
                align_center: false,
            }),
        ],
    })
}
