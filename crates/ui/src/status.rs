//! The transient status chrome: the status strip (a busy spinner + a status line,
//! shown when the app is busy or non-`Ready`) and the toast pill. Both are
//! non-interactive. The auto-dismiss TIMER for a toast stays SHELL-side (the cores
//! are time-free) — the shell sets/clears `model.toast`; this layer only renders the
//! current value. Every paint is a theme token, so a theme flip recolors with no
//! rebake.

use shape_ui_core::{
    Axis, Container, CrossAlign, Edges, Paint, Rect, RectStyle, Text, TextPaint, Widget,
};

use crate::UiModel;

const STRIP_H: f64 = 28.0;
const MARGIN: f64 = 16.0;
const SPINNER: f64 = 14.0;
const TOAST_H: f64 = 30.0;
const CHAR_W: f64 = 6.5;
const PAD: f64 = 12.0;

/// Build the status chrome, or `None` when there is nothing transient to show (steady
/// `Ready`, not busy, no toast). Returns ONE absolute container holding whichever of
/// the strip / toast are active.
pub(crate) fn build(model: &UiModel) -> Option<Widget> {
    let strip = strip(model);
    let toast = toast(model);
    if strip.is_none() && toast.is_none() {
        return None;
    }
    Some(Widget::Container(Container {
        id: "status".to_string(),
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: strip.into_iter().chain(toast).collect(),
    }))
}

/// The bottom-left status strip — shown when busy OR the status line is non-`Ready`.
/// A busy state prepends a spinner glyph (a static dot; the spin animation is a shell
/// concern, the cores being time-free).
fn strip(model: &UiModel) -> Option<Widget> {
    let status = model.status.unwrap_or("Ready");
    if !model.busy && status == "Ready" {
        return None;
    }
    let (_, vh) = model.viewport;
    let label_w = status.chars().count() as f64 * CHAR_W;
    let strip_w = PAD * 2.0 + if model.busy { SPINNER + 6.0 } else { 0.0 } + label_w;
    let x = MARGIN;
    let y = vh - STRIP_H - MARGIN;

    // A frosted `material` pill with a `hairline` border — the macOS status capsule.
    let mut children = vec![Widget::Rect(Rect {
        id: "status::strip-bg".to_string(),
        x: 0.0,
        y: 0.0,
        w: strip_w,
        h: STRIP_H,
        style: RectStyle {
            fill: Some(Paint::Token("material".to_string())),
            stroke: Some((Paint::Token("hairline".to_string()), 1.0)),
            corner_radius: STRIP_H / 2.0,
            opacity: 1.0,
        },
        hoverable: false,
    })];
    let mut text_x = PAD;
    if model.busy {
        // A spinner placeholder glyph — a small muted dot. The spin is shell-driven.
        children.push(Widget::Rect(Rect {
            id: "status::spinner".to_string(),
            x: PAD,
            y: (STRIP_H - SPINNER) / 2.0,
            w: SPINNER,
            h: SPINNER,
            style: RectStyle {
                fill: Some(Paint::Token("selection-ring".to_string())),
                stroke: None,
                corner_radius: SPINNER / 2.0,
                opacity: 1.0,
            },
            hoverable: false,
        }));
        text_x += SPINNER + 6.0;
    }
    children.push(Widget::Text(Text {
        id: "status::label".to_string(),
        x: text_x,
        y: 0.0,
        w: label_w,
        h: STRIP_H,
        label: status.to_string(),
        size_px: 12.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    }));

    Some(Widget::Container(Container {
        id: "status::strip".to_string(),
        x,
        y,
        w: strip_w,
        h: STRIP_H,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    }))
}

const DIAG_W: f64 = 240.0;
const DIAG_ROW_H: f64 = 20.0;
const DIAG_PAD: f64 = 10.0;

/// The diagnostics panel — five read-only rows (state / detail / objects / frame-ms /
/// camera), each a pre-formatted display string the shell computes (the cores hold no
/// frame timing). Anchored top-right, below where the inspector would sit. `None`
/// when no diagnostics were fed even though the toggle is on.
pub(crate) fn diagnostics(model: &UiModel) -> Option<Widget> {
    let d = model.diagnostics?;
    let rows = [
        ("State", d.state.as_str()),
        ("Detail", d.detail.as_str()),
        ("Objects", d.objects.as_str()),
        ("Frame ms", d.frame_ms.as_str()),
        ("Camera", d.camera.as_str()),
    ];
    let panel_h = DIAG_PAD * 2.0 + rows.len() as f64 * DIAG_ROW_H;
    let (vw, _) = model.viewport;
    let x = (vw - DIAG_W - MARGIN).max(0.0);
    let y = MARGIN;

    // The macOS-material panel: soft-shadow underlay + a frosted `material` body with
    // a `hairline` border (radius 12), then the read-only rows on top.
    let mut children = Vec::new();
    crate::composites::material_panel(&mut children, "diagnostics", DIAG_W, panel_h, 12.0);
    for (i, (label, value)) in rows.iter().enumerate() {
        children.push(Widget::Text(Text {
            id: format!("diagnostics::{}", label.to_lowercase().replace(' ', "-")),
            x: DIAG_PAD,
            y: DIAG_PAD + i as f64 * DIAG_ROW_H,
            w: DIAG_W - DIAG_PAD * 2.0,
            h: DIAG_ROW_H,
            label: format!("{label}: {value}"),
            size_px: 11.0,
            // Diagnostic readouts are muted captions (`text-secondary`).
            color: TextPaint::Token("text-secondary".to_string()),
            align_center: false,
        }));
    }

    Some(Widget::Container(Container {
        id: "diagnostics".to_string(),
        x,
        y,
        w: DIAG_W,
        h: panel_h,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    }))
}

/// The bottom-center toast pill — present iff `model.toast` is set. The shell owns the
/// auto-dismiss timer; this just renders the current message.
fn toast(model: &UiModel) -> Option<Widget> {
    let message = model.toast?;
    let (vw, vh) = model.viewport;
    let toast_w = PAD * 2.0 + message.chars().count() as f64 * CHAR_W;
    let x = ((vw - toast_w) / 2.0).max(0.0);
    // Above where the bottom toolbar sits.
    let y = vh - TOAST_H - MARGIN - 72.0;

    Some(Widget::Container(Container {
        id: "status::toast".to_string(),
        x,
        y: y.max(0.0),
        w: toast_w,
        h: TOAST_H,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![
            // A muted `surface-muted` pill with a `hairline` border — the macOS toast.
            Widget::Rect(Rect {
                id: "status::toast-bg".to_string(),
                x: 0.0,
                y: 0.0,
                w: toast_w,
                h: TOAST_H,
                style: RectStyle {
                    fill: Some(Paint::Token("surface-muted".to_string())),
                    stroke: Some((Paint::Token("hairline".to_string()), 1.0)),
                    corner_radius: TOAST_H / 2.0,
                    opacity: 1.0,
                },
                hoverable: false,
            }),
            Widget::Text(Text {
                id: "status::toast-label".to_string(),
                x: 0.0,
                y: 0.0,
                w: toast_w,
                h: TOAST_H,
                label: message.to_string(),
                size_px: 12.0,
                color: TextPaint::Token("text".to_string()),
                align_center: true,
            }),
        ],
    }))
}
