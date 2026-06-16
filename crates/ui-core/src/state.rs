//! The retained interaction MODEL. The widget tree is the pure VIEW; `UiState`
//! holds interaction state and per-id value caches keyed by `WidgetId`, so
//! re-deriving the tree (P4) never wipes a slider value or focus. Widget structs
//! are NEVER mutated in place — `render()` projects the tree through the caches.
//!
//! All decisions (hit, slider value from pt.x, button down/up actuation, toggle
//! flip, segment cell, backstop text edit) live here — the shell forwards events
//! and renders, computing nothing.

use std::collections::BTreeMap;

use shape_renderer_core::render_object::RenderObjectScene;

use crate::hit::{hit, resolved_box};
use crate::widget::{EditRequest, Segment, TextInput, Toggle, Widget, WidgetId};

/// A neutral key the shell forwards (NEVER an `event.key` literal branch in the
/// shell — the shell forwards the neutral key and ui-core decides). `key` is e.g.
/// `"Backspace"`; `text` is the inserted printable char(s), if any. `ctrl`/`meta`/
/// `alt` carry the OS modifier state so the core can tell a shortcut chord (Cmd+Z,
/// Ctrl+C) from a bare control key — a chord must NOT be consumed by a focused field,
/// so undo/copy/paste/select-all still reach the catalog/browser.
#[derive(Clone, Debug)]
pub struct KeyInput {
    pub key: String,
    pub text: Option<String>,
    pub ctrl: bool,
    pub meta: bool,
    pub alt: bool,
}

/// The semantic outcome of a dispatch, emitted for the app to consume. The shell
/// forwards these opaquely; it computes nothing from them.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Pressed(WidgetId),
    ToggleChanged { id: WidgetId, on: bool },
    SliderChanged { id: WidgetId, value: f64 },
    SegmentChanged { id: WidgetId, index: usize },
    TextChanged { id: WidgetId, text: String },
    Focus(WidgetId),
}

/// The result of a single `dispatch_*` call.
#[derive(Clone, Debug, Default)]
pub struct DispatchResult {
    /// A widget owned this event (the shell does not fall through to the canvas).
    pub consumed: bool,
    /// State visibly changed — the UI scene must be re-fed.
    pub dirty: bool,
    pub actions: Vec<Action>,
    /// A TextInput gained focus → the shell IME mounts (NEXT slice).
    pub edit: Option<EditRequest>,
}

/// A pointer phase the shell forwards verbatim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// The kind an owner id resolves to, for actuation routing. Variants that drive
/// actuation off geometry/value carry the borrowed widget; the rest are unit
/// (their state is read through the value caches, not the kind payload).
enum Kind<'a> {
    Button,
    Swatch,
    Toggle(&'a Toggle),
    // Slider carries no payload: its value rides the cache + the RESOLVED screen box
    // (`resolved_box`), never the un-laid-out widget's declared x/w.
    Slider,
    Segment(&'a Segment),
    TextInput(&'a TextInput),
}

#[derive(Default)]
struct UiState {
    hovered: Option<WidgetId>,
    pressed: Option<WidgetId>,
    focused: Option<WidgetId>,
    // Per-id value caches overlay the tree's declared values once interacted.
    // BTreeMap (not HashMap): no rng/seed, deterministic, pointer-width-agnostic.
    slider_values: BTreeMap<WidgetId, f64>,
    toggle_values: BTreeMap<WidgetId, bool>,
    segment_values: BTreeMap<WidgetId, usize>,
    text_values: BTreeMap<WidgetId, String>,
    /// Pointer-down screen pt (for slider drag math in-core).
    press_origin: Option<(f64, f64)>,
}

/// The renderer-owned retained UI runtime.
pub struct UiRuntime {
    tree: Widget,
    theme_dark: bool,
    viewport: (f64, f64),
    state: UiState,
}

impl UiRuntime {
    pub fn new(tree: Widget, viewport: (f64, f64), theme_dark: bool) -> Self {
        UiRuntime { tree, theme_dark, viewport, state: UiState::default() }
    }

    /// Replace the view tree (P4 re-derives UIs) while preserving interaction
    /// caches — a re-derived slider keeps its dragged value.
    pub fn set_tree(&mut self, tree: Widget) {
        self.tree = tree;
    }

    /// Returns whether a re-feed is needed (the viewport changed).
    pub fn set_viewport(&mut self, viewport: (f64, f64)) -> bool {
        if self.viewport == viewport {
            return false;
        }
        self.viewport = viewport;
        true
    }

    /// Returns whether a re-feed is needed (text colors re-resolve on theme flip).
    pub fn set_theme(&mut self, theme_dark: bool) -> bool {
        if self.theme_dark == theme_dark {
            return false;
        }
        self.theme_dark = theme_dark;
        true
    }

    pub fn focused_widget(&self) -> Option<&WidgetId> {
        self.state.focused.as_ref()
    }

    /// True when a TextInput owns focus (the window arbiter ORs this into `typing`).
    pub fn has_text_focus(&self) -> bool {
        match &self.state.focused {
            Some(id) => matches!(self.find_kind(id), Some(Kind::TextInput(_))),
            None => false,
        }
    }

    pub fn dispatch_pointer(&mut self, phase: PointerPhase, pt: (f64, f64)) -> DispatchResult {
        match phase {
            PointerPhase::Down => self.pointer_down(pt),
            PointerPhase::Move => self.pointer_move(pt),
            PointerPhase::Up => self.pointer_up(pt),
            PointerPhase::Cancel => self.pointer_cancel(),
        }
    }

    pub fn dispatch_key(&mut self, key: &KeyInput) -> DispatchResult {
        let Some(focused) = self.state.focused.clone() else {
            return DispatchResult::default();
        };
        if !matches!(self.find_kind(&focused), Some(Kind::TextInput(_))) {
            return DispatchResult::default();
        }
        // A shortcut chord (Cmd/Ctrl-modified) is NOT text the field owns — fall through
        // unconsumed so undo/copy/paste/select-all reach the catalog/browser. (Alt is
        // excluded: on macOS Option+key composes a printable, which the field must keep;
        // no catalog shortcut binds Alt without Mod.) A bare control key
        // (Backspace/Enter/Escape) or a printable insert is the field's; everything else
        // (e.g. arrows) falls through below.
        if key.ctrl || key.meta {
            return DispatchResult::default();
        }
        let mut text = self.current_text(&focused);
        let mut result = DispatchResult { consumed: true, dirty: true, ..Default::default() };
        match key.key.as_str() {
            "Backspace" => {
                text.pop();
                self.state.text_values.insert(focused.clone(), text.clone());
                result.actions.push(Action::TextChanged { id: focused, text });
            }
            "Enter" | "Escape" => {
                // Commit: blur and emit the current text.
                self.state.focused = None;
                result.actions.push(Action::TextChanged { id: focused, text });
            }
            _ => match &key.text {
                Some(s) => {
                    text.push_str(s);
                    self.state.text_values.insert(focused.clone(), text.clone());
                    result.actions.push(Action::TextChanged { id: focused, text });
                }
                // A non-printing key the field does not act on (e.g. an arrow) falls
                // through unconsumed so it can reach the catalog — not swallowed.
                None => return DispatchResult::default(),
            },
        }
        result
    }

    /// Commit ONE finished string into the focused field, blur it, and emit the
    /// final `TextChanged`. This is the IME seam: the shell's OS editing surface
    /// owns composition (the bare per-key `dispatch_key` relay must NOT also run
    /// while that surface is mounted, or the field would take each jamo twice) and
    /// hands back exactly one committed value. Setting the value wholesale — not
    /// diffing a suffix — stays correct when composition deletes/replaces in place
    /// (CJK), where the committed string is not a prefix-extension of the seed.
    /// A no-op (returns `consumed:false`) when no TextInput is focused.
    pub fn commit_text(&mut self, value: String) -> DispatchResult {
        let Some(focused) = self.state.focused.clone() else {
            return DispatchResult::default();
        };
        if !matches!(self.find_kind(&focused), Some(Kind::TextInput(_))) {
            return DispatchResult::default();
        }
        self.state.text_values.insert(focused.clone(), value.clone());
        self.state.focused = None;
        DispatchResult {
            consumed: true,
            dirty: true,
            actions: vec![Action::TextChanged { id: focused, text: value }],
            ..Default::default()
        }
    }

    /// Project the tree through the value caches + VisualState token swaps and
    /// render. Called ONLY on a `DispatchResult.dirty` (a discrete state change),
    /// so cloning the small UI tree is off the hot path.
    pub fn render(&self) -> RenderObjectScene {
        let projected = self.project(&self.tree);
        crate::render::render(&projected, self.viewport, self.theme_dark)
    }

    // ---- pointer phases ----

    fn pointer_down(&mut self, pt: (f64, f64)) -> DispatchResult {
        let Some(id) = hit(&self.tree, pt) else {
            // Canvas fall-through; a previously focused field blurs.
            if self.state.focused.take().is_some() {
                return DispatchResult { consumed: false, dirty: true, ..Default::default() };
            }
            return DispatchResult::default();
        };

        let mut result = DispatchResult { consumed: true, dirty: true, ..Default::default() };
        self.state.pressed = Some(id.clone());
        self.state.press_origin = Some(pt);

        // Read everything needed off the borrowed widget BEFORE mutating state.
        enum Down {
            TextInput { rect: (f64, f64, f64, f64), size_px: f64 },
            Slider(f64),
            Other,
        }
        let down = match self.find_kind(&id) {
            Some(Kind::TextInput(ti)) => {
                Down::TextInput { rect: (ti.x, ti.y, ti.w, ti.h), size_px: ti.size_px }
            }
            Some(Kind::Slider) => {
                let (ox, _, w, _) = resolved_box(&self.tree, &id).unwrap_or((0.0, 0.0, 0.0, 0.0));
                Down::Slider(slider_value_at(ox, w, pt.0))
            }
            _ => Down::Other,
        };

        match down {
            Down::TextInput { rect, size_px } => {
                let value = self.current_text(&id);
                self.state.focused = Some(id.clone());
                result.actions.push(Action::Focus(id.clone()));
                result.edit = Some(EditRequest { id, rect, value, size_px });
            }
            Down::Slider(value) => {
                self.state.slider_values.insert(id.clone(), value);
                result.actions.push(Action::SliderChanged { id, value });
            }
            Down::Other => {
                // Pressing a non-text widget drops any prior text focus.
                self.state.focused = None;
            }
        }
        result
    }

    fn pointer_move(&mut self, pt: (f64, f64)) -> DispatchResult {
        // A captured slider streams its value regardless of hover.
        if let Some(pressed) = self.state.pressed.clone() {
            let value = match self.find_kind(&pressed) {
                Some(Kind::Slider) => {
                    let (ox, _, w, _) =
                        resolved_box(&self.tree, &pressed).unwrap_or((0.0, 0.0, 0.0, 0.0));
                    Some(slider_value_at(ox, w, pt.0))
                }
                _ => None,
            };
            if let Some(value) = value {
                let changed = self.state.slider_values.get(&pressed) != Some(&value);
                self.state.slider_values.insert(pressed.clone(), value);
                let mut result = DispatchResult { consumed: true, dirty: changed, ..Default::default() };
                if changed {
                    result.actions.push(Action::SliderChanged { id: pressed, value });
                }
                return result;
            }
        }

        let hovered = hit(&self.tree, pt);
        let changed = hovered != self.state.hovered;
        self.state.hovered = hovered.clone();
        DispatchResult { consumed: hovered.is_some(), dirty: changed, ..Default::default() }
    }

    fn pointer_up(&mut self, pt: (f64, f64)) -> DispatchResult {
        let Some(pressed) = self.state.pressed.take() else {
            return DispatchResult::default();
        };
        self.state.press_origin = None;
        let mut result = DispatchResult { consumed: true, dirty: true, ..Default::default() };

        // Standard cancel: up must land on the same owner the press started on.
        if hit(&self.tree, pt).as_ref() != Some(&pressed) {
            return result;
        }

        // Decide the actuation off the borrowed widget BEFORE mutating state.
        enum Up {
            Press,
            Toggle,
            Segment(usize),
            None,
        }
        let actuation = match self.find_kind(&pressed) {
            Some(Kind::Button | Kind::Swatch) => Up::Press,
            Some(Kind::Toggle(_)) => Up::Toggle,
            Some(Kind::Segment(s)) => {
                let (ox, _, w, _) =
                    resolved_box(&self.tree, &pressed).unwrap_or((0.0, 0.0, 0.0, 0.0));
                Up::Segment(segment_cell_at(s, ox, w, pt.0))
            }
            // Slider already streamed; TextInput actuated on down.
            _ => Up::None,
        };

        match actuation {
            Up::Press => result.actions.push(Action::Pressed(pressed)),
            Up::Toggle => {
                let on = !self.current_toggle(&pressed);
                self.state.toggle_values.insert(pressed.clone(), on);
                result.actions.push(Action::ToggleChanged { id: pressed, on });
            }
            Up::Segment(index) => {
                self.state.segment_values.insert(pressed.clone(), index);
                result.actions.push(Action::SegmentChanged { id: pressed, index });
            }
            Up::None => {}
        }
        result
    }

    fn pointer_cancel(&mut self) -> DispatchResult {
        let was_pressed = self.state.pressed.take().is_some();
        self.state.press_origin = None;
        DispatchResult { consumed: was_pressed, dirty: was_pressed, ..Default::default() }
    }

    // ---- cache reads ----

    fn current_text(&self, id: &WidgetId) -> String {
        if let Some(v) = self.state.text_values.get(id) {
            return v.clone();
        }
        match self.find_kind(id) {
            Some(Kind::TextInput(ti)) => ti.value.clone(),
            _ => String::new(),
        }
    }

    fn current_toggle(&self, id: &WidgetId) -> bool {
        if let Some(v) = self.state.toggle_values.get(id) {
            return *v;
        }
        match self.find_kind(id) {
            Some(Kind::Toggle(t)) => t.on,
            _ => false,
        }
    }

    /// Resolve an owner id to its widget kind for actuation routing.
    fn find_kind(&self, id: &WidgetId) -> Option<Kind<'_>> {
        find_kind_in(&self.tree, id)
    }

    // ---- projection (caches + VisualState swaps → a fresh view tree) ----

    fn project(&self, widget: &Widget) -> Widget {
        match widget {
            Widget::Container(c) => {
                let mut c = c.clone();
                c.children = c.children.iter().map(|child| self.project(child)).collect();
                Widget::Container(c)
            }
            Widget::Button(b) => {
                let mut b = b.clone();
                if self.state.pressed.as_ref() == Some(&b.id) {
                    b.style.fill = Some(crate::widget::Paint::Token("surface-muted".to_string()));
                } else if self.state.hovered.as_ref() == Some(&b.id) {
                    b.style.fill = Some(crate::widget::Paint::Token("hover".to_string()));
                }
                Widget::Button(b)
            }
            // A hoverable Rect is an interactive icon-button body: when pointed at (and
            // not pressed) it takes the `hover` background, so a borderless button still
            // gives feedback. The `hoverable` flag keeps ui-core prefix-free — the
            // caller marks which bodies are interactive.
            Widget::Rect(r) if r.hoverable => {
                let mut r = r.clone();
                if self.state.pressed.as_ref() == Some(&r.id) {
                    r.style.fill = Some(crate::widget::Paint::Token("surface-muted".to_string()));
                } else if self.state.hovered.as_ref() == Some(&r.id) {
                    r.style.fill = Some(crate::widget::Paint::Token("hover".to_string()));
                }
                Widget::Rect(r)
            }
            Widget::Toggle(t) => {
                let mut t = t.clone();
                t.on = self.current_toggle(&t.id);
                Widget::Toggle(t)
            }
            Widget::Slider(s) => {
                let mut s = s.clone();
                if let Some(v) = self.state.slider_values.get(&s.id) {
                    s.value = *v;
                }
                Widget::Slider(s)
            }
            Widget::Segment(s) => {
                let mut s = s.clone();
                if let Some(i) = self.state.segment_values.get(&s.id) {
                    s.selected = *i;
                }
                Widget::Segment(s)
            }
            Widget::TextInput(t) => {
                let mut t = t.clone();
                t.focused = self.state.focused.as_ref() == Some(&t.id);
                if let Some(v) = self.state.text_values.get(&t.id) {
                    t.value = v.clone();
                }
                Widget::TextInput(t)
            }
            // Non-stateful views pass through unchanged.
            other => other.clone(),
        }
    }
}

fn find_kind_in<'a>(widget: &'a Widget, id: &WidgetId) -> Option<Kind<'a>> {
    match widget {
        Widget::Container(c) => {
            for child in &c.children {
                if let Some(k) = find_kind_in(child, id) {
                    return Some(k);
                }
            }
            None
        }
        Widget::Button(b) if &b.id == id => Some(Kind::Button),
        // A hoverable Rect is an icon-only command body (toolbar/chrome icon buttons,
        // e.g. `cmd:toggle-theme`); it actuates as a press exactly like a Button, so a
        // click fires its `Action::Pressed`. A decorative (non-hoverable) Rect does not.
        Widget::Rect(r) if r.hoverable && &r.id == id => Some(Kind::Button),
        Widget::Swatch(s) if &s.id == id => Some(Kind::Swatch),
        Widget::Toggle(t) if &t.id == id => Some(Kind::Toggle(t)),
        Widget::Slider(s) if &s.id == id => Some(Kind::Slider),
        Widget::Segment(s) if &s.id == id => Some(Kind::Segment(s)),
        Widget::TextInput(t) if &t.id == id => Some(Kind::TextInput(t)),
        _ => None,
    }
}

/// Slider value in [0,1] from a screen x within the owner box. `origin_x`/`width`
/// are the RESOLVED screen box (from `resolved_box`), NOT the declared `s.x`/`s.w` —
/// a slider nested in an offset/flex container renders away from its declared x.
fn slider_value_at(origin_x: f64, width: f64, screen_x: f64) -> f64 {
    if width <= 0.0 {
        return 0.0;
    }
    ((screen_x - origin_x) / width).clamp(0.0, 1.0)
}

/// Segment cell index from a screen x within the owner box. `origin_x`/`width` are
/// the RESOLVED screen box (from `resolved_box`), NOT the declared `s.x`/`s.w`.
fn segment_cell_at(s: &Segment, origin_x: f64, width: f64, screen_x: f64) -> usize {
    let count = s.labels.len();
    if count == 0 || width <= 0.0 {
        return 0;
    }
    let cell_w = width / count as f64;
    let raw = ((screen_x - origin_x) / cell_w).floor();
    let idx = raw.clamp(0.0, (count - 1) as f64);
    // idx is an exact integer in [0, count-1]; map to usize without a width cast.
    let mut n = 0usize;
    while (n as f64) < idx && n + 1 < count {
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::{
        Axis, Button, Container, CrossAlign, Edges, Paint, RectStyle, Slider, TextPaint,
    };

    fn root(children: Vec<Widget>) -> Widget {
        Widget::Container(Container {
            id: "root".to_string(),
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 400.0,
            direction: Axis::None,
            spacing: 0.0,
            padding: Edges::all(0.0),
            align: CrossAlign::Start,
            children,
        })
    }

    fn button(id: &str) -> Widget {
        Widget::Button(Button {
            id: id.to_string(),
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 40.0,
            label: "Go".to_string(),
            style: RectStyle {
                fill: Some(Paint::Token("surface".to_string())),
                stroke: None,
                corner_radius: 8.0,
                opacity: 1.0,
            },
            label_size_px: 16.0,
            label_color: TextPaint::Token("text".to_string()),
        })
    }

    fn runtime(children: Vec<Widget>) -> UiRuntime {
        UiRuntime::new(root(children), (400.0, 400.0), false)
    }

    /// A bare (modifier-free) key the focused field owns.
    fn key_input(key: &str, text: Option<&str>) -> KeyInput {
        KeyInput {
            key: key.to_string(),
            text: text.map(str::to_string),
            ctrl: false,
            meta: false,
            alt: false,
        }
    }

    /// A hoverable `Rect` is an icon-only command body (toolbar `push_icon_button`,
    /// chrome `icon_chrome_button` — e.g. `cmd:toggle-theme`). A press over it MUST
    /// emit `Action::Pressed`, exactly like a `Button`, so the resolved
    /// `Command("toggle-theme")` fires and the theme flips. FAILS if `find_kind`
    /// lacks the hoverable-Rect arm (the live "toggle does nothing" defect: the hover
    /// background showed yet no action ever fired).
    #[test]
    fn down_then_up_on_a_hoverable_icon_body_actuates() {
        let body = Widget::Rect(crate::widget::Rect {
            id: "cmd:toggle-theme".to_string(),
            x: 10.0,
            y: 10.0,
            w: 36.0,
            h: 36.0,
            style: RectStyle { fill: None, stroke: None, corner_radius: 8.0, opacity: 1.0 },
            hoverable: true,
        });
        let mut rt = runtime(vec![body]);
        let down = rt.dispatch_pointer(PointerPhase::Down, (28.0, 28.0));
        assert!(down.consumed && down.dirty);
        assert!(down.actions.is_empty(), "no actuation on down");
        let up = rt.dispatch_pointer(PointerPhase::Up, (28.0, 28.0));
        assert_eq!(
            up.actions,
            vec![Action::Pressed("cmd:toggle-theme".to_string())],
            "a hoverable icon body actuates as a press on up"
        );

        // A non-hoverable (decorative) Rect must NOT actuate — it is not a control.
        let decor = Widget::Rect(crate::widget::Rect {
            id: "decor".to_string(),
            x: 10.0,
            y: 10.0,
            w: 36.0,
            h: 36.0,
            style: RectStyle::default(),
            hoverable: false,
        });
        let mut rt = runtime(vec![decor]);
        rt.dispatch_pointer(PointerPhase::Down, (28.0, 28.0));
        let up = rt.dispatch_pointer(PointerPhase::Up, (28.0, 28.0));
        assert!(
            !up.actions.iter().any(|a| matches!(a, Action::Pressed(_))),
            "a decorative (non-hoverable) Rect does not actuate"
        );
    }

    #[test]
    fn down_then_up_inside_button_actuates() {
        let mut rt = runtime(vec![button("go")]);
        let down = rt.dispatch_pointer(PointerPhase::Down, (50.0, 30.0));
        assert!(down.consumed && down.dirty);
        assert!(down.actions.is_empty(), "no actuation on down");
        let up = rt.dispatch_pointer(PointerPhase::Up, (50.0, 30.0));
        assert!(up.consumed && up.dirty);
        assert_eq!(up.actions, vec![Action::Pressed("go".to_string())]);

        // Down inside, up OUTSIDE -> cancel, no Pressed.
        let mut rt = runtime(vec![button("go")]);
        rt.dispatch_pointer(PointerPhase::Down, (50.0, 30.0));
        let up = rt.dispatch_pointer(PointerPhase::Up, (300.0, 300.0));
        assert!(up.consumed);
        assert!(!up.actions.iter().any(|a| matches!(a, Action::Pressed(_))), "no Pressed on cancel");
    }

    #[test]
    fn hover_dirty_only_on_change() {
        let mut rt = runtime(vec![button("go")]);
        let first = rt.dispatch_pointer(PointerPhase::Move, (50.0, 30.0));
        assert!(first.dirty, "first move onto widget is dirty");
        let second = rt.dispatch_pointer(PointerPhase::Move, (55.0, 30.0));
        assert!(!second.dirty, "same-widget move skips re-feed");
        let off = rt.dispatch_pointer(PointerPhase::Move, (300.0, 300.0));
        assert!(off.dirty, "move off the widget is dirty");
    }

    #[test]
    fn slider_drag_streams_value_in_core() {
        let slider = Widget::Slider(Slider {
            id: "s".to_string(),
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
            value: 0.0,
        });
        let mut rt = runtime(vec![slider]);
        let down = rt.dispatch_pointer(PointerPhase::Down, (25.0, 10.0));
        assert_eq!(down.actions, vec![Action::SliderChanged { id: "s".to_string(), value: 0.25 }]);
        let mv = rt.dispatch_pointer(PointerPhase::Move, (75.0, 10.0));
        assert_eq!(mv.actions, vec![Action::SliderChanged { id: "s".to_string(), value: 0.75 }]);
        // out-of-range clamps.
        let mv = rt.dispatch_pointer(PointerPhase::Move, (200.0, 10.0));
        assert_eq!(mv.actions, vec![Action::SliderChanged { id: "s".to_string(), value: 1.0 }]);
        // render reflects the cached value: filled width == 1.0*w == 100 ⇒ 800.
        let scene = rt.render();
        let fill = scene.objects.iter().find(|o| o.id == "s::fill").expect("fill");
        assert!(fill.geometry_d.contains("800"), "fill tracks cached value: {}", fill.geometry_d);
    }

    #[test]
    fn toggle_and_segment_actuate_on_up() {
        let toggle = Widget::Toggle(Toggle { id: "t".to_string(), x: 0.0, y: 0.0, w: 48.0, h: 24.0, on: false });
        let mut rt = runtime(vec![toggle]);
        rt.dispatch_pointer(PointerPhase::Down, (10.0, 10.0));
        let up = rt.dispatch_pointer(PointerPhase::Up, (10.0, 10.0));
        assert_eq!(up.actions, vec![Action::ToggleChanged { id: "t".to_string(), on: true }]);
        // render reflects the flipped state (track token = selection-ring).
        let scene = rt.render();
        match &scene.objects[0].fill.as_ref().unwrap().paint {
            shape_renderer_core::render_object::RPaint::Token { name } => assert_eq!(name, "selection-ring"),
            _ => panic!("token"),
        }

        let segment = Widget::Segment(Segment {
            id: "g".to_string(),
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 30.0,
            labels: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            selected: 0,
            label_size_px: 14.0,
            label_color: TextPaint::Token("text".to_string()),
        });
        let mut rt = runtime(vec![segment]);
        rt.dispatch_pointer(PointerPhase::Down, (250.0, 15.0));
        let up = rt.dispatch_pointer(PointerPhase::Up, (250.0, 15.0));
        // 250 / (300/3=100) = cell 2.
        assert_eq!(up.actions, vec![Action::SegmentChanged { id: "g".to_string(), index: 2 }]);
        let scene = rt.render();
        let sel = scene.objects.iter().find(|o| o.id == "g::sel").expect("sel");
        // cell 2 starts at 200; the selected pill is inset 2px within its cell, so 202.
        assert_eq!(sel.transform[0][2], 202.0, "selected cell 2 pill is inset 2px from x=200");
    }

    #[test]
    fn textinput_focus_emits_edit_request_and_key_edits_value() {
        let ti = Widget::TextInput(TextInput {
            id: "ti".to_string(),
            x: 5.0,
            y: 5.0,
            w: 200.0,
            h: 32.0,
            value: "ab".to_string(),
            focused: false,
            size_px: 14.0,
            color: TextPaint::Token("text".to_string()),
            placeholder: String::new(),
        });
        let mut rt = runtime(vec![ti]);
        let down = rt.dispatch_pointer(PointerPhase::Down, (50.0, 20.0));
        assert!(down.actions.contains(&Action::Focus("ti".to_string())));
        let edit = down.edit.expect("edit request");
        assert_eq!(edit.id, "ti");
        assert_eq!(edit.value, "ab");
        assert!(rt.has_text_focus());
        assert_eq!(rt.focused_widget(), Some(&"ti".to_string()));

        let typed = rt.dispatch_key(&key_input("c", Some("c")));
        assert!(typed.consumed && typed.dirty);
        assert_eq!(typed.actions, vec![Action::TextChanged { id: "ti".to_string(), text: "abc".to_string() }]);

        let back = rt.dispatch_key(&key_input("Backspace", None));
        assert_eq!(back.actions, vec![Action::TextChanged { id: "ti".to_string(), text: "ab".to_string() }]);

        let enter = rt.dispatch_key(&key_input("Enter", None));
        assert!(enter.consumed);
        assert!(!rt.has_text_focus(), "Enter commits and blurs");

        // render reflects the cached edited value.
        // (re-focus to re-render with the cache; value text should be "ab")
        let scene = rt.render();
        let value = scene.objects.iter().find(|o| o.id == "ti::value").expect("value");
        assert_eq!(value.text.as_ref().unwrap().runs[0].text, "ab");
    }

    #[test]
    fn commit_text_lands_one_composed_string_not_a_per_jamo_run() {
        // The IME seam: the OS surface composes ㅎ→하→한 IN PLACE (each step is NOT a
        // prefix-extension of the prior), then hands back ONE committed string. A
        // suffix-diff relay would push "ㅎ","하","한" and corrupt the field to "ㅎ하한";
        // commit_text must land exactly "한" and blur. FAILS if commit appends.
        let ti = Widget::TextInput(TextInput {
            id: "ti".to_string(),
            x: 5.0,
            y: 5.0,
            w: 200.0,
            h: 32.0,
            value: String::new(),
            focused: false,
            size_px: 14.0,
            color: TextPaint::Token("text".to_string()),
            placeholder: String::new(),
        });
        let mut rt = runtime(vec![ti]);
        rt.dispatch_pointer(PointerPhase::Down, (50.0, 20.0));
        assert!(rt.has_text_focus());

        let committed = rt.commit_text("한".to_string());
        assert!(committed.consumed && committed.dirty);
        assert_eq!(
            committed.actions,
            vec![Action::TextChanged { id: "ti".to_string(), text: "한".to_string() }]
        );
        assert!(!rt.has_text_focus(), "commit blurs the field");

        let scene = rt.render();
        let value = scene.objects.iter().find(|o| o.id == "ti::value").expect("value");
        assert_eq!(value.text.as_ref().unwrap().runs[0].text, "한");
    }

    #[test]
    fn commit_text_without_focus_is_a_noop() {
        let mut rt = runtime(vec![button("go")]);
        let r = rt.commit_text("x".to_string());
        assert!(!r.consumed && !r.dirty);
        assert!(r.actions.is_empty());
    }

    #[test]
    fn dispatch_pointer_off_widget_does_not_consume() {
        let ti = Widget::TextInput(TextInput {
            id: "ti".to_string(),
            x: 5.0,
            y: 5.0,
            w: 200.0,
            h: 32.0,
            value: String::new(),
            focused: false,
            size_px: 14.0,
            color: TextPaint::Token("text".to_string()),
            placeholder: String::new(),
        });
        let mut rt = runtime(vec![ti]);
        // empty space with nothing focused -> not consumed, not dirty.
        let down = rt.dispatch_pointer(PointerPhase::Down, (300.0, 300.0));
        assert!(!down.consumed && !down.dirty);

        // focus then click empty -> not consumed but dirty (focus cleared).
        rt.dispatch_pointer(PointerPhase::Down, (50.0, 20.0));
        assert!(rt.has_text_focus());
        let down = rt.dispatch_pointer(PointerPhase::Down, (300.0, 300.0));
        assert!(!down.consumed);
        assert!(down.dirty, "blurring a focused field is a visible change");
        assert!(!rt.has_text_focus());
    }

    #[test]
    fn set_theme_and_viewport_report_dirty_on_change() {
        let mut rt = runtime(vec![button("go")]);
        assert!(rt.set_theme(true));
        assert!(!rt.set_theme(true));
        assert!(rt.set_viewport((800.0, 600.0)));
        assert!(!rt.set_viewport((800.0, 600.0)));
    }

    #[test]
    fn set_tree_preserves_value_caches() {
        let slider = Widget::Slider(Slider { id: "s".to_string(), x: 0.0, y: 0.0, w: 100.0, h: 20.0, value: 0.0 });
        let mut rt = runtime(vec![slider]);
        rt.dispatch_pointer(PointerPhase::Down, (50.0, 10.0));
        // a re-derived identical tree keeps the dragged value in render().
        let slider2 = Widget::Slider(Slider { id: "s".to_string(), x: 0.0, y: 0.0, w: 100.0, h: 20.0, value: 0.0 });
        rt.set_tree(root(vec![slider2]));
        let scene = rt.render();
        let fill = scene.objects.iter().find(|o| o.id == "s::fill").expect("fill");
        // 0.5*100 = 50 ⇒ 400.
        assert!(fill.geometry_d.contains("400"), "cache preserved across set_tree: {}", fill.geometry_d);
    }

    /// A slider/segment nested in an OFFSET container maps pt.x→value against its
    /// RESOLVED screen origin, not its declared x. FAILS if the value math reads the
    /// declared `s.x` (a slider declared at x=0 inside a container at x=100, pressed
    /// 10px into a 100px track at screen x=110, would wrongly read value 1.0).
    #[test]
    fn slider_and_segment_value_use_resolved_offset_not_declared_x() {
        let offset = |children: Vec<Widget>| {
            Widget::Container(Container {
                id: "offset".to_string(),
                x: 100.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
                direction: Axis::None,
                spacing: 0.0,
                padding: Edges::all(0.0),
                align: CrossAlign::Start,
                children,
            })
        };

        // Slider declared at x=0 (renders at screen x=100). Press at screen x=110 is
        // 10px into the 100px track ⇒ value 0.10, NOT 1.0.
        let slider = Widget::Slider(Slider { id: "s".to_string(), x: 0.0, y: 0.0, w: 100.0, h: 20.0, value: 0.0 });
        let mut rt = runtime(vec![offset(vec![slider])]);
        let down = rt.dispatch_pointer(PointerPhase::Down, (110.0, 10.0));
        assert_eq!(
            down.actions,
            vec![Action::SliderChanged { id: "s".to_string(), value: 0.10 }],
            "value reads the resolved screen origin (100), not declared x (0)"
        );
        // A drag streams against the same resolved origin.
        let mv = rt.dispatch_pointer(PointerPhase::Move, (150.0, 10.0));
        assert_eq!(mv.actions, vec![Action::SliderChanged { id: "s".to_string(), value: 0.5 }]);

        // Segment declared at x=0 (renders at x=100), 3 cells over 300px. Press at
        // screen x=350 is 250px into the track ⇒ cell 2, NOT cell index off declared x.
        let segment = Widget::Segment(Segment {
            id: "g".to_string(),
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 30.0,
            labels: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            selected: 0,
            label_size_px: 14.0,
            label_color: TextPaint::Token("text".to_string()),
        });
        let mut rt = runtime(vec![offset(vec![segment])]);
        rt.dispatch_pointer(PointerPhase::Down, (350.0, 15.0));
        let up = rt.dispatch_pointer(PointerPhase::Up, (350.0, 15.0));
        assert_eq!(
            up.actions,
            vec![Action::SegmentChanged { id: "g".to_string(), index: 2 }],
            "cell reads the resolved screen origin (100), not declared x (0)"
        );
    }

    /// A focused TextInput must NOT swallow a shortcut chord (Cmd/Ctrl-modified): it
    /// returns consumed:false and emits no TextChanged, so undo/copy/paste/select-all
    /// fall through to the catalog/browser. FAILS if a chord is consumed (the dead
    /// clipboard/undo regression).
    #[test]
    fn focused_textinput_does_not_consume_a_modifier_chord() {
        let ti = Widget::TextInput(TextInput {
            id: "ti".to_string(),
            x: 5.0,
            y: 5.0,
            w: 200.0,
            h: 32.0,
            value: "ab".to_string(),
            focused: false,
            size_px: 14.0,
            color: TextPaint::Token("text".to_string()),
            placeholder: String::new(),
        });
        let mut rt = runtime(vec![ti]);
        rt.dispatch_pointer(PointerPhase::Down, (50.0, 20.0));
        assert!(rt.has_text_focus());

        for chord in [
            KeyInput { key: "z".to_string(), text: None, ctrl: false, meta: true, alt: false }, // Cmd+Z undo
            KeyInput { key: "c".to_string(), text: None, ctrl: true, meta: false, alt: false }, // Ctrl+C copy
            KeyInput { key: "v".to_string(), text: None, ctrl: false, meta: true, alt: false }, // Cmd+V paste
            KeyInput { key: "a".to_string(), text: None, ctrl: false, meta: true, alt: false }, // Cmd+A select-all
        ] {
            let r = rt.dispatch_key(&chord);
            assert!(!r.consumed, "a {chord:?} chord must fall through to the catalog");
            assert!(r.actions.is_empty(), "a chord emits no TextChanged: {chord:?}");
        }
        // The field still holds focus and its value is untouched by the chords.
        assert!(rt.has_text_focus());
        assert_eq!(rt.dispatch_key(&key_input("c", Some("c"))).actions, vec![Action::TextChanged { id: "ti".to_string(), text: "abc".to_string() }]);

        // Option/Alt composes a PRINTABLE on macOS (Alt+e ⇒ "é"); it is text the field
        // owns, NOT a chord — it inserts. (No catalog shortcut binds Alt without Mod.)
        let alt = KeyInput { key: "é".to_string(), text: Some("é".to_string()), ctrl: false, meta: false, alt: true };
        let r = rt.dispatch_key(&alt);
        assert!(r.consumed, "an Alt-composed printable is the field's text, not a chord");
        assert_eq!(r.actions, vec![Action::TextChanged { id: "ti".to_string(), text: "abcé".to_string() }]);
    }
}
