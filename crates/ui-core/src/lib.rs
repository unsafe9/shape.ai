//! ui-core: a pure peer-producer of `RenderObjectScene` (screen-space px) from a
//! retained widget tree, plus a retained stateful interaction model
//! (`UiRuntime`). renderer-core dep only — no scene-core/op-apply.

mod hit;
mod layout;
mod metric;
mod quant;
mod render;
mod state;
mod theme;
mod widget;

pub use hit::hit;
pub use metric::{GRID, PANEL_RADIUS, ROW_H, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
pub use render::render;
pub use state::{Action, DispatchResult, KeyInput, PointerPhase, UiRuntime};
pub use widget::{
    Axis, Button, Container, CrossAlign, Edges, EditRequest, Icon, MainAlign, Paint, Rect,
    RectStyle, Segment, Slider, Swatch, Text, TextInput, TextPaint, Toggle, VisualState, Widget,
    WidgetId,
};

#[cfg(test)]
mod tests {
    use crate::quant::q;

    #[test]
    fn quant_matches_scene_core_convention() {
        assert_eq!(q(1.0), 8);
        assert_eq!(q(0.0), 0);
        assert_eq!(q(100.0), 800);
        assert_eq!(q(-1.0), -8);
        // 0.0625 px * 8 = 0.5, round half toward +∞ ⇒ 1 (NOT truncation to 0).
        assert_eq!(q(0.0625), 1);
        assert_eq!(q(f64::NAN), 0);
    }
}
