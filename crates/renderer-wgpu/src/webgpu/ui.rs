//! The wasm-facing mirror of a ui-core `DispatchResult`. The serializable shape +
//! the PURE mapping from `shape_ui_core::DispatchResult` are split out here so the
//! mapping is host-testable WITHOUT a wgpu device (the `#[wasm_bindgen]` dispatch
//! exports in `input.rs` live behind `target_arch = "wasm32"`). The shell treats
//! `actions`/`edit` as OPAQUE forward payloads — it computes nothing from them.

use serde::{Deserialize, Serialize};

use shape_scene_core::object::catalog::commands::object_command_catalog;
use shape_scene_core::object::catalog::gestures::object_gesture_catalog;
use shape_scene_core::object::catalog::inspector::InspectorView;
use shape_ui::{CanvasEntry, ContextMenuModel, Diagnostics, Intent, PeerCursor, TemplateEntry, UiModel};
use shape_ui_core::{Action, DispatchResult, EditRequest, Widget};

/// The shell-fed UI model state, owned by the renderer so it survives across
/// dispatches and a theme/viewport refresh. Built from the JSON the shell pushes
/// each UI-affecting change (`setUiModel`); the renderer composes a borrowed
/// `shape_ui::UiModel` from it (referencing the in-core command catalog) to
/// `build_root` the widget tree and to `resolve` a fired action into an `Intent`.
/// The command catalog is constant data built in-core (`object_command_catalog`),
/// so it is NOT carried over the wire — only the shell-owned state + the dynamic
/// inspector view are.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiModelInput {
    pub theme_dark: bool,
    pub viewport: [f64; 2],
    pub active_tool: String,
    #[serde(default)]
    pub create_kind: Option<String>,
    /// The pen color (`#rrggbb`) applied to new strokes / the selection. Defaults to
    /// black when absent.
    #[serde(default = "default_pen_color")]
    pub selected_color: String,
    /// The pickable pen-color palette the toolbar's Color chips render.
    #[serde(default)]
    pub pen_palette: Vec<String>,
    /// The active brush width in px. Defaults to 1 when absent.
    #[serde(default = "default_pen_width")]
    pub pen_width: f64,
    /// The pickable brush widths the toolbar's Stroke chips render.
    #[serde(default)]
    pub pen_widths: Vec<f64>,
    /// The template-library rows the More→Templates popup lists.
    #[serde(default)]
    pub templates: Vec<TemplateEntry>,
    /// Whether the template-library popup is open. Defaults closed.
    #[serde(default)]
    pub template_open: bool,
    /// The canvases the top-left switcher lists.
    #[serde(default)]
    pub canvases: Vec<CanvasEntry>,
    /// The id of the active canvas (the switcher marks it selected).
    #[serde(default)]
    pub active_canvas_id: String,
    /// Whether the realtime transport is online. Defaults online.
    #[serde(default = "default_true")]
    pub connection_online: bool,
    /// Whether a canvas create/delete/switch is in flight (controls disable).
    #[serde(default)]
    pub canvas_busy: bool,
    /// The diagnostics readout, rendered when `diagnostics_open`.
    #[serde(default)]
    pub diagnostics: Option<Diagnostics>,
    /// Whether the diagnostics panel is open. Defaults closed.
    #[serde(default)]
    pub diagnostics_open: bool,
    /// The dynamic inspector view over the current selection, or `null`/absent for
    /// an empty/canvas selection (the panel hides). Produced by scene-core's
    /// `object_inspector_view` wasm export and handed straight back.
    #[serde(default)]
    pub inspector_view: Option<InspectorView>,
    /// True on macOS, so the settings modal formats `Mod` as `⌘`. The shell feeds the
    /// bit; the core reads no platform. Defaults false (absent ⇒ non-mac).
    #[serde(default)]
    pub is_mac: bool,
    /// Whether the settings modal is open. Defaults closed.
    #[serde(default)]
    pub settings_open: bool,
    /// The open right-click context menu (anchor + resolved items), or absent.
    #[serde(default)]
    pub context_menu: Option<ContextMenuModel>,
    /// Live peer cursors at projected screen coords (the shell projects + filters).
    #[serde(default)]
    pub peers: Vec<PeerCursor>,
    /// True while a long-running op is in flight (the status strip shows a spinner).
    #[serde(default)]
    pub busy: bool,
    /// The status line, or absent for the steady `Ready` state.
    #[serde(default)]
    pub status: Option<String>,
    /// A transient toast message, or absent. The auto-dismiss timer stays shell-side.
    #[serde(default)]
    pub toast: Option<String>,
}

fn default_pen_color() -> String {
    "#000000".to_string()
}

fn default_pen_width() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

impl UiModelInput {
    /// Compose the borrowed `shape_ui::UiModel`, referencing the constant command
    /// catalog. `catalog` is held by the caller for the model's lifetime.
    pub(crate) fn as_model<'a>(
        &'a self,
        catalog: &'a [shape_scene_core::object::catalog::commands::ObjectCommand],
        gestures: &'a [shape_scene_core::object::catalog::gestures::ObjectGesture],
    ) -> UiModel<'a> {
        UiModel {
            theme_dark: self.theme_dark,
            viewport: (self.viewport[0], self.viewport[1]),
            active_tool: &self.active_tool,
            create_kind: self.create_kind.as_deref(),
            selected_color: &self.selected_color,
            pen_palette: &self.pen_palette,
            pen_width: self.pen_width,
            pen_widths: &self.pen_widths,
            templates: &self.templates,
            template_open: self.template_open,
            canvases: &self.canvases,
            active_canvas_id: &self.active_canvas_id,
            connection_online: self.connection_online,
            canvas_busy: self.canvas_busy,
            diagnostics: self.diagnostics.as_ref(),
            diagnostics_open: self.diagnostics_open,
            command_catalog: catalog,
            gesture_catalog: gestures,
            inspector_view: self.inspector_view.as_ref(),
            is_mac: self.is_mac,
            settings_open: self.settings_open,
            context_menu: self.context_menu.as_ref(),
            peers: &self.peers,
            busy: self.busy,
            status: self.status.as_deref(),
            toast: self.toast.as_deref(),
        }
    }

    /// Build the composed root widget tree for this model state.
    pub(crate) fn build_tree(&self) -> Widget {
        let catalog = object_command_catalog();
        let gestures = object_gesture_catalog();
        shape_ui::build_root(&self.as_model(&catalog, &gestures))
    }

    /// Resolve a fired ui-core `Action` to a typed `Intent` for THIS model state.
    pub(crate) fn resolve(&self, action: &Action) -> Option<Intent> {
        let catalog = object_command_catalog();
        let gestures = object_gesture_catalog();
        shape_ui::resolve(action, &self.as_model(&catalog, &gestures))
    }
}

/// A discriminated-union mirror of `shape_ui::Intent` for the JS side. The shell
/// authors each: a `command` runs its catalog handler, an `inspectorEdit` lowers
/// through scene-core `inspector_edit_op`, etc. The shell never invents an op.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum UiIntentDto {
    Command {
        id: String,
    },
    SelectColor {
        hex: String,
    },
    SelectPenWidth {
        px: f64,
    },
    ApplyTemplate {
        id: String,
    },
    SelectCanvas {
        id: String,
    },
    NewCanvas,
    DeleteCanvas {
        id: String,
    },
    // The container `rename_all` renames the variant tag but does NOT cascade to a
    // struct-variant's fields in this serde version, so the multi-word fields carry
    // an explicit camelCase rename — these are the keys the shell switches on.
    InspectorEdit {
        #[serde(rename = "controlId")]
        control_id: String,
        #[serde(rename = "opKind")]
        op_kind: String,
        field: Option<String>,
        #[serde(rename = "unitScale")]
        unit_scale: f64,
        value: serde_json::Value,
    },
    InspectorAction {
        #[serde(rename = "controlId")]
        control_id: String,
    },
    /// A floating overlay (settings modal / context menu) asked to close. The shell
    /// clears its matching open flag; it authors no op.
    Dismiss,
}

impl From<&Intent> for UiIntentDto {
    fn from(intent: &Intent) -> Self {
        match intent {
            Intent::Command(id) => UiIntentDto::Command { id: id.clone() },
            Intent::SelectColor(hex) => UiIntentDto::SelectColor { hex: hex.clone() },
            Intent::SelectPenWidth(px) => UiIntentDto::SelectPenWidth { px: *px },
            Intent::ApplyTemplate(id) => UiIntentDto::ApplyTemplate { id: id.clone() },
            Intent::SelectCanvas(id) => UiIntentDto::SelectCanvas { id: id.clone() },
            Intent::NewCanvas => UiIntentDto::NewCanvas,
            Intent::DeleteCanvas(id) => UiIntentDto::DeleteCanvas { id: id.clone() },
            Intent::InspectorEdit { control_id, op_kind, field, unit_scale, value } => {
                UiIntentDto::InspectorEdit {
                    control_id: control_id.clone(),
                    op_kind: op_kind.clone(),
                    field: field.clone(),
                    unit_scale: *unit_scale,
                    value: value.clone(),
                }
            }
            Intent::InspectorAction { control_id } => {
                UiIntentDto::InspectorAction { control_id: control_id.clone() }
            }
            Intent::Dismiss => UiIntentDto::Dismiss,
        }
    }
}

/// A discriminated-union mirror of `shape_ui_core::Action` for the JS side. `type`
/// is the tag; the remaining fields carry the variant payload.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum UiActionDto {
    Pressed { id: String },
    ToggleChanged { id: String, on: bool },
    SliderChanged { id: String, value: f64 },
    SegmentChanged { id: String, index: usize },
    TextChanged { id: String, text: String },
    Focus { id: String },
}

impl From<&Action> for UiActionDto {
    fn from(action: &Action) -> Self {
        match action {
            Action::Pressed(id) => UiActionDto::Pressed { id: id.clone() },
            Action::ToggleChanged { id, on } => {
                UiActionDto::ToggleChanged { id: id.clone(), on: *on }
            }
            Action::SliderChanged { id, value } => {
                UiActionDto::SliderChanged { id: id.clone(), value: *value }
            }
            Action::SegmentChanged { id, index } => {
                UiActionDto::SegmentChanged { id: id.clone(), index: *index }
            }
            Action::TextChanged { id, text } => {
                UiActionDto::TextChanged { id: id.clone(), text: text.clone() }
            }
            Action::Focus(id) => UiActionDto::Focus { id: id.clone() },
        }
    }
}

/// A serializable mirror of `shape_ui_core::EditRequest`. `rect` is `(x, y, w, h)`
/// in screen px — where the shell's IME (NEXT slice) mounts an editing surface.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiEditRequestDto {
    pub id: String,
    pub rect: [f64; 4],
    pub value: String,
    pub size_px: f64,
}

impl From<&EditRequest> for UiEditRequestDto {
    fn from(edit: &EditRequest) -> Self {
        UiEditRequestDto {
            id: edit.id.clone(),
            rect: [edit.rect.0, edit.rect.1, edit.rect.2, edit.rect.3],
            value: edit.value.clone(),
            size_px: edit.size_px,
        }
    }
}

/// The wasm dispatch return. `scene_changed` is the runtime's `dirty` bit AFTER the
/// renderer acted on it (re-rendered + re-fed the UI scene), so the shell only knows
/// "the RAF must redraw"; it never decides what changed.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoreUiDispatchResult {
    pub consumed: bool,
    pub scene_changed: bool,
    pub actions: Vec<UiActionDto>,
    pub edit: Option<UiEditRequestDto>,
    /// The typed intents the actions resolved to through the built-in UI binding
    /// (`shape_ui::resolve`). Empty when no model is set (the demo path) or no action
    /// authored anything. The shell forwards each intent to its existing op-authoring
    /// path; it never re-derives an intent from `actions`.
    pub intents: Vec<UiIntentDto>,
}

impl CoreUiDispatchResult {
    /// Map a ui-core `DispatchResult` into the wire shape with NO intent resolution
    /// (the demo path / a renderer without a set model). `scene_changed` is the
    /// dispatch's `dirty` bit (the renderer re-feeds on it before returning).
    pub(crate) fn from_dispatch(result: &DispatchResult) -> Self {
        CoreUiDispatchResult {
            consumed: result.consumed,
            scene_changed: result.dirty,
            actions: result.actions.iter().map(UiActionDto::from).collect(),
            edit: result.edit.as_ref().map(UiEditRequestDto::from),
            intents: Vec::new(),
        }
    }

    /// Map a dispatch into the wire shape AND resolve each action to a typed intent
    /// through the set model (the built-in UI binding). This is the single place a
    /// widget actuation becomes a core op request the shell authors.
    pub(crate) fn from_dispatch_resolved(result: &DispatchResult, model: &UiModelInput) -> Self {
        let mut dto = Self::from_dispatch(result);
        dto.intents = result.actions.iter().filter_map(|a| model.resolve(a)).map(|i| UiIntentDto::from(&i)).collect();
        dto
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_action_variant_and_edit_to_the_wire_shape() {
        let result = DispatchResult {
            consumed: true,
            dirty: true,
            actions: vec![
                Action::Pressed("b".to_string()),
                Action::ToggleChanged { id: "t".to_string(), on: true },
                Action::SliderChanged { id: "s".to_string(), value: 0.5 },
                Action::SegmentChanged { id: "g".to_string(), index: 2 },
                Action::TextChanged { id: "i".to_string(), text: "hi".to_string() },
                Action::Focus("i".to_string()),
            ],
            edit: Some(EditRequest {
                id: "i".to_string(),
                rect: (5.0, 6.0, 200.0, 32.0),
                value: "hi".to_string(),
                size_px: 14.0,
            }),
        };
        let dto = CoreUiDispatchResult::from_dispatch(&result);
        assert!(dto.consumed && dto.scene_changed);
        assert_eq!(dto.actions.len(), 6);
        assert_eq!(dto.actions[0], UiActionDto::Pressed { id: "b".to_string() });
        assert_eq!(dto.actions[2], UiActionDto::SliderChanged { id: "s".to_string(), value: 0.5 });
        let edit = dto.edit.expect("edit forwarded");
        assert_eq!(edit.rect, [5.0, 6.0, 200.0, 32.0]);
        assert_eq!(edit.size_px, 14.0);
    }

    /// The JSON `type` tags are the discriminated-union keys the shell switches on
    /// (camelCase, no enum-variant wrapper) — a serde-shape regression fails here.
    #[test]
    fn action_serializes_as_a_tagged_camelcase_union() {
        let json = serde_json::to_string(&UiActionDto::SliderChanged {
            id: "s".to_string(),
            value: 0.25,
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"sliderChanged","id":"s","value":0.25}"#);
        let json = serde_json::to_string(&UiActionDto::Pressed { id: "b".to_string() }).unwrap();
        assert_eq!(json, r#"{"type":"pressed","id":"b"}"#);
    }

    /// An empty dispatch (off-widget canvas fall-through) forwards nothing and is
    /// not consumed — the shell falls through to the canvas path.
    #[test]
    fn empty_dispatch_is_a_clean_fall_through() {
        let dto = CoreUiDispatchResult::from_dispatch(&DispatchResult::default());
        assert!(!dto.consumed && !dto.scene_changed);
        assert!(dto.actions.is_empty() && dto.edit.is_none() && dto.intents.is_empty());
    }

    /// A model-resolved dispatch carries the typed intents alongside the raw actions:
    /// a `cmd:undo` press resolves to a Command intent the shell authors. FAILS if the
    /// resolution path drops the intent (the shell would then have nothing to author).
    #[test]
    fn resolved_dispatch_carries_typed_intents_from_the_model() {
        let model: UiModelInput = serde_json::from_str(
            r#"{"themeDark":false,"viewport":[1280,800],"activeTool":"select-move"}"#,
        )
        .expect("model input deserializes");
        let result = DispatchResult {
            consumed: true,
            dirty: true,
            actions: vec![Action::Pressed("cmd:undo".to_string())],
            edit: None,
        };
        let dto = CoreUiDispatchResult::from_dispatch_resolved(&result, &model);
        assert_eq!(dto.intents.len(), 1, "the undo press resolves to one intent");
        assert_eq!(dto.intents[0], UiIntentDto::Command { id: "undo".to_string() });
        // The raw action is still forwarded for the shell's focus/IME bookkeeping.
        assert_eq!(dto.actions[0], UiActionDto::Pressed { id: "cmd:undo".to_string() });
    }

    /// The extended model input deserializes the new built-in-UI fields (settings
    /// open, context menu, peers) and a scrim press resolves to a `Dismiss` intent
    /// through the set model. FAILS if a new wire field stopped flowing or the scrim
    /// binding broke. (Pins the renderer-wgpu→shape_ui seam for the new UIs.)
    #[test]
    fn extended_model_input_resolves_a_scrim_dismiss() {
        // A `##` delimiter so the `"#` inside the peer color literal does not close it.
        let model: UiModelInput = serde_json::from_str(
            r##"{
                "themeDark": true,
                "viewport": [1280, 800],
                "activeTool": "select-move",
                "isMac": true,
                "settingsOpen": true,
                "contextMenu": {
                    "x": 100, "y": 120, "title": "object:r",
                    "items": [
                        { "commandId": "duplicate", "label": "Duplicate" },
                        { "commandId": null, "label": "" },
                        { "commandId": "delete", "label": "Delete", "danger": true }
                    ]
                },
                "peers": [
                    { "userId": "u1", "screen": [300, 220], "color": "#ff5733", "label": "Ada" }
                ],
                "busy": true,
                "status": "Saving",
                "toast": "Copied"
            }"##,
        )
        .expect("extended model input deserializes");

        // The built tree composes the new UIs (no panic) and the settings scrim
        // dismisses through the resolver.
        let _tree = model.build_tree();
        let dismiss = model.resolve(&Action::Pressed("settings::scrim".to_string()));
        assert_eq!(dismiss, Some(Intent::Dismiss), "the settings scrim resolves to Dismiss");
        // A context-menu item still resolves to its catalog Command.
        let dup = model.resolve(&Action::Pressed("cmd:duplicate".to_string()));
        assert_eq!(dup, Some(Intent::Command("duplicate".to_string())));

        let dto = UiIntentDto::from(&dismiss.unwrap());
        assert_eq!(dto, UiIntentDto::Dismiss);
    }

    /// The toolbar-parity model fields flow over the wire and resolve through the set
    /// model: a fed palette produces a `swatch:` chip whose press resolves to a
    /// SelectColor DTO, a canvas tab to SelectCanvas, and the bare new button to
    /// NewCanvas. FAILS if a new wire field stopped flowing or a new intent DTO arm is
    /// missing (the renderer-wgpu→shape_ui seam for the P3 toolbar parity).
    #[test]
    fn extended_model_input_resolves_toolbar_parity_intents() {
        // A `##` delimiter so the `"#` inside the color literals does not close it.
        let model: UiModelInput = serde_json::from_str(
            r##"{
                "themeDark": false,
                "viewport": [1280, 800],
                "activeTool": "draw",
                "selectedColor": "#ff0000",
                "penPalette": ["#ff0000", "#00ff00"],
                "penWidth": 4,
                "penWidths": [2, 4, 8],
                "canvases": [
                    { "id": "c1", "title": "First" },
                    { "id": "c2", "title": "Second" }
                ],
                "activeCanvasId": "c2",
                "templates": [ { "id": "kanban", "title": "Kanban", "description": "A board" } ],
                "templateOpen": true
            }"##,
        )
        .expect("toolbar-parity model input deserializes");

        // The tree composes the new controls without panic.
        let _tree = model.build_tree();

        let color = model.resolve(&Action::Pressed("swatch:#00ff00".to_string()));
        assert_eq!(color, Some(Intent::SelectColor("#00ff00".to_string())));
        assert_eq!(
            UiIntentDto::from(&color.unwrap()),
            UiIntentDto::SelectColor { hex: "#00ff00".to_string() }
        );

        let width = model.resolve(&Action::Pressed("pen-width:8".to_string()));
        assert_eq!(UiIntentDto::from(&width.unwrap()), UiIntentDto::SelectPenWidth { px: 8.0 });

        let template = model.resolve(&Action::Pressed("template:kanban".to_string()));
        assert_eq!(UiIntentDto::from(&template.unwrap()), UiIntentDto::ApplyTemplate { id: "kanban".to_string() });

        let switch = model.resolve(&Action::Pressed("canvas:c1".to_string()));
        assert_eq!(UiIntentDto::from(&switch.unwrap()), UiIntentDto::SelectCanvas { id: "c1".to_string() });

        let new = model.resolve(&Action::Pressed("canvas-new".to_string()));
        assert_eq!(UiIntentDto::from(&new.unwrap()), UiIntentDto::NewCanvas);

        let delete = model.resolve(&Action::Pressed("canvas-delete:c2".to_string()));
        assert_eq!(UiIntentDto::from(&delete.unwrap()), UiIntentDto::DeleteCanvas { id: "c2".to_string() });
    }

    /// The intent DTO is a tagged camelCase union (the keys the shell switches on);
    /// an inspectorEdit carries the catalog op_kind/field/unit_scale verbatim.
    #[test]
    fn intent_dto_serializes_as_a_tagged_camelcase_union() {
        let json = serde_json::to_string(&UiIntentDto::InspectorEdit {
            control_id: "axis".to_string(),
            op_kind: "set-layout".to_string(),
            field: Some("axis".to_string()),
            unit_scale: 1.0,
            value: serde_json::Value::String("vertical".to_string()),
        })
        .unwrap();
        assert!(json.contains(r#""type":"inspectorEdit""#));
        assert!(json.contains(r#""opKind":"set-layout""#));
        assert!(json.contains(r#""unitScale":1.0"#));
    }
}
