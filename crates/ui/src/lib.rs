//! shape_ui: the UI extension layer. It composes `shape_ui_core` widget
//! primitives into the built-in product UIs (toolbar, inspector) and binds every
//! actuation to the `shape_scene_core` catalogs — the ONE binding source. It is a
//! pure VIEW BUILDER: given app state (`UiModel`) it returns one
//! `shape_ui_core::Widget` tree; given a fired `shape_ui_core::Action` it returns
//! a typed [`Intent`] the shell authors as a scene-core op or a shell-only effect.
//! It owns ZERO widget primitives and ZERO op-apply.
//!
//! WidgetId namespacing IS the binding key: ids are `cmd:<command-id>` (toolbar
//! buttons), `insp:<control-id>` (inspector controls), `swatch:<hex>`, `tool:*`,
//! etc., so [`resolve`] parses the id prefix — there is no second catalog copy.
//!
//! Pure: no time/rng/threads/IO (matching ui-core + scene-core). A theme flip or a
//! selection change re-derives the small tree off `UiModel`; the runtime preserves
//! interaction caches across `set_tree`, so a focused field / dragged value lives.

mod chrome;
mod composites;
mod context_menu;
mod icons;
mod inspector;
mod intent;
mod presence;
mod settings;
mod status;
mod template;
mod toolbar;

#[cfg(test)]
mod tests;

use shape_scene_core::object::catalog::commands::ObjectCommand;
use shape_scene_core::object::catalog::gestures::ObjectGesture;
use shape_scene_core::object::catalog::inspector::InspectorView;
use shape_ui_core::Widget;

pub use context_menu::{ContextMenuItem, ContextMenuModel};
pub use intent::{resolve, Intent};
pub use presence::PeerCursor;
pub use template::TemplateEntry;

/// One canvas row the top-left switcher lists. Shell-fed (the canvas registry is a
/// shell/runtime concern), so it deserializes from the shell's JSON like the other
/// shell-fed lists.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasEntry {
    pub id: String,
    pub title: String,
}

/// The diagnostics panel readout, shown when `diagnostics_open`. Every field is a
/// pre-formatted display string the shell computes (the cores are time-free and own
/// no frame timing); this layer only renders the rows.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub state: String,
    pub detail: String,
    pub objects: String,
    pub frame_ms: String,
    pub camera: String,
}

/// The single input the shell feeds each UI-affecting state change. The shell
/// builds it from the render-only mirror the core hands it (selection, the
/// `inspector_view` over that mirror, the command catalog) plus shell-owned UI
/// state (active tool, color/stroke selection, theme/viewport) — it authors no
/// decision; this builder does.
///
/// Lifetimes borrow the catalog + view the shell already holds, so a per-change
/// rebuild allocates only the small widget tree, never a catalog copy.
pub struct UiModel<'a> {
    pub theme_dark: bool,
    /// Screen viewport in CSS px; the toolbar anchors bottom-center off it.
    pub viewport: (f64, f64),
    /// The armed tool command id (`select-move`/`hand-pan`/`draw`) — the core
    /// decides it; the toolbar marks the matching button active off this mirror,
    /// never a raw flag branch.
    pub active_tool: &'a str,
    /// The armed insert kind command id (`insert-rectangle`/…), or `None`.
    pub create_kind: Option<&'a str>,
    /// The pen color applied to new strokes / the selection, as `#rrggbb`. The
    /// toolbar's Color popup marks the matching palette chip selected off this
    /// mirror and the swatch shows it; the shell owns the value.
    pub selected_color: &'a str,
    /// The palette of pickable pen colors (`#rrggbb` each) the Color popup chips.
    pub pen_palette: &'a [String],
    /// The active pen/brush width in px. The Stroke popup marks the matching size
    /// button active off this mirror.
    pub pen_width: f64,
    /// The pickable brush widths (px) the Stroke popup buttons.
    pub pen_widths: &'a [f64],
    /// The template library rows the More→Templates popup lists; opened iff
    /// `template_open`.
    pub templates: &'a [TemplateEntry],
    /// Whether the template-library popup is open (the toolbar's Templates toggle
    /// is active and the popup renders).
    pub template_open: bool,
    /// The canvases the top-left switcher lists.
    pub canvases: &'a [CanvasEntry],
    /// The id of the active canvas (the switcher marks it selected).
    pub active_canvas_id: &'a str,
    /// Whether the realtime transport is online (the switcher's connection glyph).
    pub connection_online: bool,
    /// Whether a canvas create/delete/switch is in flight — the switcher's controls
    /// disable while busy.
    pub canvas_busy: bool,
    /// The diagnostics readout, rendered when `diagnostics_open`.
    pub diagnostics: Option<&'a Diagnostics>,
    /// Whether the diagnostics panel is open (the toolbar's Diagnostics toggle is
    /// active and the panel renders).
    pub diagnostics_open: bool,
    /// The command catalog (the toolbar + the active-state binding source).
    pub command_catalog: &'a [ObjectCommand],
    /// The gesture catalog (the settings modal's second, hold-key section).
    pub gesture_catalog: &'a [ObjectGesture],
    /// The dynamic inspector view over the current selection, or `None` when the
    /// selection is empty/canvas (the panel hides). Built by the core's
    /// `inspector_view`; the panel renders it read-only-structurally.
    pub inspector_view: Option<&'a InspectorView>,
    /// True on macOS, so the settings modal renders `Mod`/`Shift`/`Alt` as
    /// `⌘`/`⇧`/`⌥`. Fed in by the shell — the pure cores read no platform.
    pub is_mac: bool,
    /// Whether the settings modal (Cmd+,) is open. The shell owns the flag; the
    /// modal renders the command + gesture catalogs READ-ONLY when set.
    pub settings_open: bool,
    /// The open right-click context menu (anchor + catalog-driven items), or `None`.
    pub context_menu: Option<&'a ContextMenuModel>,
    /// Live peer cursors at already-projected SCREEN coords (projection stays
    /// shell-side; a null projection is filtered before feeding).
    pub peers: &'a [PeerCursor],
    /// True while a long-running op is in flight — the status strip shows a spinner.
    pub busy: bool,
    /// The status line, shown when non-`Ready` or `busy`. `None` is the steady state.
    pub status: Option<&'a str>,
    /// A transient toast message, or `None`. The auto-dismiss TIMER stays shell-side
    /// (the cores are time-free); the shell sets/clears this.
    pub toast: Option<&'a str>,
}

/// Compose every built-in UI into ONE absolute screen-space root container. The
/// shell hands this to `UiRuntime::set_tree` and the runtime renders/hit-tests/
/// dispatches it. Token paints throughout, so a theme flip is a recolor re-feed
/// with zero geometry rebake.
pub fn build_root(model: &UiModel) -> Widget {
    use shape_ui_core::{Axis, Container, CrossAlign, Edges, MainAlign};

    // Render order is paint order (later children sit on top). Chrome and presence
    // ride above the toolbar/inspector; the scrim-backed overlays (settings,
    // context menu) come LAST so their scrim covers everything beneath.
    // The watermark sits first so it paints behind every other UI element.
    let mut children = vec![chrome::watermark(model), toolbar::build(model)];
    if model.template_open && !model.templates.is_empty() {
        children.push(template::build(model));
    }
    if let Some(view) = model.inspector_view {
        if !intent::view_is_empty(view) {
            children.push(inspector::build(view, model.viewport));
        }
    }
    children.push(chrome::theme_toggle(model));
    children.push(chrome::canvas_switcher(model));
    if !model.peers.is_empty() {
        children.push(presence::build(model.peers));
    }
    if let Some(strip) = status::build(model) {
        children.push(strip);
    }
    if model.diagnostics_open {
        if let Some(panel) = status::diagnostics(model) {
            children.push(panel);
        }
    }
    if model.settings_open {
        children.push(settings::build(model));
    }
    if let Some(menu) = model.context_menu {
        children.push(context_menu::build(menu, model));
    }

    // Origin-0 absolute root: each child carries its own absolute screen coord, so
    // the toolbar (bottom-center) and the inspector (top-right) place independently.
    Widget::Container(Container {
        id: "ui-root".to_string(),
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}
