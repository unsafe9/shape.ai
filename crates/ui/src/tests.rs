//! Falsifiable tests for the built-in UIs. Each drives the REAL ui-core
//! render/hit/dispatch + the REAL scene-core catalogs/`inspector_view` — never a
//! second impl. A test fails when the BEHAVIOR is wrong (a dropped binding, a
//! drifted op_kind, a rebaked theme flip), not merely when something doesn't
//! compile.

use serde_json::Value;
use shape_renderer_core::render_object::{RPaint, RenderObject};
use shape_scene_core::object::affine::compose_affine;
use shape_scene_core::object::catalog::commands::object_command_catalog;
use shape_scene_core::object::catalog::gestures::object_gesture_catalog;
use shape_scene_core::object::catalog::inspector::{inspector_view, InspectorView};
use shape_scene_core::object::kernel::model::{
    Align, AxisSizing, CrossAlign, FillRule, Geometry, Lanes, Layout, LayoutAxis, MainAlign,
    Object, ObjectScene, ObjectSelection, PathNode, Sizing, SubPath, Text, TextAlign, TextRun,
    TextVAlign,
};
use shape_scene_core::object::region::StubOutlineDeriver;
use shape_ui_core::{Action, PointerPhase, UiRuntime, Widget};

use crate::{build_root, resolve, ContextMenuItem, ContextMenuModel, Intent, PeerCursor, UiModel};

// ---- fixtures (real core scenes/views) ----

fn rect(id: &str) -> Object {
    Object::new(
        id,
        "a0",
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(80, 0),
                    PathNode::corner(80, 40),
                    PathNode::corner(0, 40),
                ],
            }],
            FillRule::EvenOdd,
        ),
    )
}

fn scene_with(objects: Vec<Object>) -> ObjectScene {
    ObjectScene {
        objects,
        ..Default::default()
    }
}

/// A scene with a flow-container parent + a flow child, so views over each exercise
/// the segment/lanes/align composites and the sizing controls.
fn flow_scene() -> ObjectScene {
    let mut parent = rect("p");
    parent.layout = Some(Layout {
        axis: LayoutAxis::Horizontal,
        lanes: Lanes::Count { value: 2 },
        spacing: 16,
        align: Align {
            main: MainAlign::Center,
            cross: CrossAlign::Start,
        },
    });
    let mut child = rect("c");
    child.parent = Some("p".into());
    child.sizing = Some(Sizing {
        w: AxisSizing::Fixed { value: 120 },
        h: AxisSizing::Fill,
    });
    scene_with(vec![parent, child])
}

fn view_of(scene: &ObjectScene, id: &str) -> InspectorView {
    inspector_view(
        scene,
        &ObjectSelection::Object { id: id.into() },
        &StubOutlineDeriver,
    )
}

/// The empty gesture catalog reference the base fixture uses (tests that exercise the
/// settings modal pass the real `object_gesture_catalog()` explicitly).
const NO_GESTURES: &[shape_scene_core::object::catalog::gestures::ObjectGesture] = &[];
const NO_PEERS: &[crate::PeerCursor] = &[];
const NO_TEMPLATES: &[crate::TemplateEntry] = &[];

fn model<'a>(
    catalog: &'a [shape_scene_core::object::catalog::commands::ObjectCommand],
    view: &'a InspectorView,
) -> UiModel<'a> {
    UiModel {
        theme_dark: false,
        viewport: (1280.0, 800.0),
        active_tool: "select-move",
        create_kind: None,
        selected_color: "#000000",
        pen_palette: &[],
        pen_width: 2.0,
        pen_widths: &[],
        templates: NO_TEMPLATES,
        template_open: false,
        canvases: &[],
        active_canvas_id: "",
        connection_online: true,
        canvas_busy: false,
        diagnostics: None,
        diagnostics_open: false,
        command_catalog: catalog,
        gesture_catalog: NO_GESTURES,
        inspector_view: Some(view),
        is_mac: false,
        settings_open: false,
        context_menu: None,
        peers: NO_PEERS,
        busy: false,
        status: None,
        toast: None,
    }
}

/// Every interactive owner id in a widget tree (the ids `hit`/`resolve` see).
fn owner_ids(tree: &Widget, out: &mut Vec<String>) {
    match tree {
        Widget::Container(c) => {
            for child in &c.children {
                owner_ids(child, out);
            }
        }
        Widget::Rect(r) => out.push(r.id.clone()),
        Widget::Button(b) => out.push(b.id.clone()),
        Widget::Swatch(s) => out.push(s.id.clone()),
        Widget::Toggle(t) => out.push(t.id.clone()),
        Widget::Slider(s) => out.push(s.id.clone()),
        Widget::Segment(s) => out.push(s.id.clone()),
        Widget::TextInput(t) => out.push(t.id.clone()),
        Widget::Icon(_) | Widget::Text(_) => {}
    }
}

/// EVERY widget id in the tree (Containers + Text included), for asserting on
/// non-interactive structure the renderer may or may not emit a RenderObject for.
fn all_ids(tree: &Widget, out: &mut Vec<String>) {
    match tree {
        Widget::Container(c) => {
            out.push(c.id.clone());
            for child in &c.children {
                all_ids(child, out);
            }
        }
        Widget::Rect(r) => out.push(r.id.clone()),
        Widget::Icon(i) => out.push(i.id.clone()),
        Widget::Button(b) => out.push(b.id.clone()),
        Widget::Swatch(s) => out.push(s.id.clone()),
        Widget::Toggle(t) => out.push(t.id.clone()),
        Widget::Slider(s) => out.push(s.id.clone()),
        Widget::Segment(s) => out.push(s.id.clone()),
        Widget::TextInput(t) => out.push(t.id.clone()),
        Widget::Text(t) => out.push(t.id.clone()),
    }
}

// ---- (1) inspector binding integrity ----

/// `build_root`'s inspector emits an interactive widget for EVERY control id the
/// core's `inspector_view` produced (each maps to an `insp:<control-id>` owner) —
/// no control is silently dropped. FAILS if a widget arm is missing for a control.
#[test]
fn inspector_emits_a_bound_widget_for_every_control_in_the_view() {
    let scene = flow_scene();
    let view = view_of(&scene, "p"); // flow container: segment/lanes/align present
    let catalog = object_command_catalog();
    let tree = build_root(&model(&catalog, &view));

    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);

    for section in &view.sections {
        for control in &section.controls {
            // The role badge is a read-only display (no op_kind, no interactive id);
            // every other control must surface an `insp:<id>`-prefixed owner.
            if control.op_kind.is_none() {
                continue;
            }
            let prefix = format!("insp:{}", control.id);
            assert!(
                ids.iter().any(|id| id == &prefix
                    || id.starts_with(&format!("{prefix}:"))
                    || id.starts_with(&format!("{prefix}::"))),
                "control {} has no bound insp: widget (ids: {:?})",
                control.id,
                ids
            );
        }
    }
}

// ---- (2) toolbar ↔ command catalog round-trip ----

/// Every toolbar button id round-trips: a `Pressed("cmd:<id>")` resolves to
/// `Command("<id>")`, AND every id the toolbar surfaces is a real command-catalog
/// id (the single-source rule — no orphan button). FAILS if a button isn't in the
/// catalog or `resolve` mis-maps it.
#[test]
fn toolbar_buttons_are_catalog_commands_and_resolve_round_trips() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    let tree = build_root(&m);

    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);
    let cmd_ids: Vec<&str> = ids
        .iter()
        .filter_map(|id| id.strip_prefix("cmd:"))
        .collect();
    assert!(!cmd_ids.is_empty(), "toolbar must surface commands");

    let catalog_ids: std::collections::HashSet<&str> =
        catalog.iter().map(|c| c.id.as_str()).collect();
    for id in &cmd_ids {
        assert!(
            catalog_ids.contains(id),
            "toolbar button {id} is not a command-catalog id (orphan)"
        );
        let intent = resolve(&Action::Pressed(format!("cmd:{id}")), &m);
        assert_eq!(
            intent,
            Some(Intent::Command(id.to_string())),
            "cmd:{id} must resolve to Command({id})"
        );
    }

    // The named requests the toolbar promises (undo/redo/insert/zoom) are present.
    for expected in [
        "undo",
        "redo",
        "insert-rectangle",
        "zoom-in",
        "zoom-fit",
        "select-move",
    ] {
        assert!(cmd_ids.contains(&expected), "toolbar missing {expected}");
    }
}

/// An undefined `cmd:` id (not in the catalog) still resolves structurally to a
/// `Command` — `resolve` is a pure prefix parser; the SHELL's handler map gates an
/// unknown id. (Pins the prefix contract, not a catalog membership check here.)
#[test]
fn resolve_parses_the_cmd_prefix_purely() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    assert_eq!(
        resolve(&Action::Pressed("cmd:zoom-out".to_string()), &m),
        Some(Intent::Command("zoom-out".to_string()))
    );
    assert_eq!(
        resolve(&Action::Focus("cmd:undo".to_string()), &m),
        None,
        "Focus authors nothing"
    );
}

// ---- (3) zero-rebake theme flip ----

/// A theme_dark flip changes ONLY text colors; every `geometry_d` and every token
/// fill stays identical (no re-tessellation, no fill rebake). FAILS if the dark
/// tree's geometry diverges from the light tree's — the perf contract.
#[test]
fn theme_flip_recolors_text_only_no_geometry_or_fill_rebake() {
    let scene = flow_scene();
    let view = view_of(&scene, "p");
    let catalog = object_command_catalog();

    let light_model = UiModel {
        theme_dark: false,
        ..model(&catalog, &view)
    };
    let dark_model = UiModel {
        theme_dark: true,
        ..model(&catalog, &view)
    };
    let light = shape_ui_core::render(&build_root(&light_model), (1280.0, 800.0), false);
    let dark = shape_ui_core::render(&build_root(&dark_model), (1280.0, 800.0), true);

    assert_eq!(light.objects.len(), dark.objects.len(), "same object count");
    let mut text_changed = false;
    for (l, d) in light.objects.iter().zip(&dark.objects) {
        assert_eq!(l.id, d.id, "object order/ids identical across theme");
        assert_eq!(
            l.geometry_d, d.geometry_d,
            "{}: geometry must not rebake on theme flip",
            l.id
        );
        // Token fills are emitted as RPaint::Token (resolved by the renderer), so they
        // are byte-identical across the flip.
        assert_eq!(
            fill_repr(l),
            fill_repr(d),
            "{}: token fill must not change",
            l.id
        );
        if let (Some(lt), Some(dt)) = (&l.text, &d.text) {
            if lt.runs.first().map(|r| &r.color) != dt.runs.first().map(|r| &r.color) {
                text_changed = true;
            }
        }
    }
    assert!(
        text_changed,
        "a theme flip must re-resolve at least one text color"
    );
}

fn fill_repr(o: &RenderObject) -> String {
    o.fill
        .as_ref()
        .map(|f| format!("{:?}", f.paint))
        .unwrap_or_default()
}

// ---- (4) inspector segment → InspectorEdit carries catalog op_kind/field/scale ----

/// A segment cell change resolves to an `InspectorEdit` carrying the control's
/// catalog op_kind/field/unit_scale, with the wire value mapped from the chosen
/// option (the `inspectorSegmentSelected`/`segmentValue` logic, now in Rust). FAILS
/// if the op_kind/field/scale isn't carried straight from the catalog.
#[test]
fn inspector_segment_change_carries_catalog_metadata() {
    let scene = flow_scene();
    let view = view_of(&scene, "p"); // flow container has the `axis` segment
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    // `axis` is a Segment {Horizontal, Vertical}; picking cell 1 (Vertical) authors
    // a set-layout edit on field `axis` with value "vertical".
    let intent = resolve(
        &Action::SegmentChanged {
            id: "insp:axis".to_string(),
            index: 1,
        },
        &m,
    );
    match intent {
        Some(Intent::InspectorEdit {
            control_id,
            op_kind,
            field,
            unit_scale,
            value,
        }) => {
            assert_eq!(control_id, "axis");
            assert_eq!(op_kind, "set-layout", "op_kind carried from the catalog");
            assert_eq!(field.as_deref(), Some("axis"));
            assert_eq!(unit_scale, 1.0);
            assert_eq!(value, Value::String("vertical".to_string()));
        }
        other => panic!("expected an axis InspectorEdit, got {other:?}"),
    }
}

/// A Fixed sizing segment carries the Q-scale (so a later px edit re-quantizes), and
/// the wire value is the AxisSizing tag — `sizing-w` cell "Fixed" emits
/// `{kind:fixed,value:0}` with the catalog's non-1 unit_scale.
#[test]
fn inspector_sizing_segment_emits_axis_sizing_with_quantized_scale() {
    let scene = flow_scene();
    let view = view_of(&scene, "c"); // flow child has sizing-w / sizing-h
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    // sizing-w options are [Hug, Fill, Fixed]; cell 2 == Fixed.
    let intent = resolve(
        &Action::SegmentChanged {
            id: "insp:sizing-w".to_string(),
            index: 2,
        },
        &m,
    );
    match intent {
        Some(Intent::InspectorEdit {
            control_id,
            op_kind,
            unit_scale,
            value,
            ..
        }) => {
            assert_eq!(control_id, "sizing-w");
            assert_eq!(op_kind, "set-sizing");
            assert!(
                unit_scale > 1.0,
                "sizing carries the quantized Q-scale, got {unit_scale}"
            );
            assert_eq!(value["kind"], Value::String("fixed".to_string()));
            assert_eq!(value["value"], Value::from(0));
        }
        other => panic!("expected a sizing-w InspectorEdit, got {other:?}"),
    }
}

/// A `visible` toggle (Header) authors a `set-meta` edit on field `hidden` carrying
/// the toggle's on-state — exactly the panel's onEdit. FAILS if the toggle isn't
/// routed to the control's op_kind/field.
#[test]
fn inspector_toggle_carries_catalog_metadata() {
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    let intent = resolve(
        &Action::ToggleChanged {
            id: "insp:visible".to_string(),
            on: false,
        },
        &m,
    );
    match intent {
        Some(Intent::InspectorEdit {
            control_id,
            op_kind,
            field,
            value,
            ..
        }) => {
            assert_eq!(control_id, "visible");
            assert_eq!(op_kind, "set-meta");
            assert_eq!(field.as_deref(), Some("hidden"));
            assert_eq!(value, Value::Bool(false));
        }
        other => panic!("expected a visible InspectorEdit, got {other:?}"),
    }
}

/// An align grid cell encodes its `{main,cross}` in the id, and `resolve` decodes it
/// to a `set-layout` edit on field `align`. FAILS if the encoded value isn't
/// recovered (the only way a `Pressed` carries the cell's value).
#[test]
fn align_grid_cell_decodes_main_cross_into_an_inspector_edit() {
    let scene = flow_scene();
    let view = view_of(&scene, "p");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    let intent = resolve(
        &Action::Pressed("insp:align:main=center,cross=end".to_string()),
        &m,
    );
    match intent {
        Some(Intent::InspectorEdit {
            control_id,
            op_kind,
            field,
            value,
            ..
        }) => {
            assert_eq!(control_id, "align");
            assert_eq!(op_kind, "set-layout");
            assert_eq!(field.as_deref(), Some("align"));
            assert_eq!(value["main"], Value::String("center".to_string()));
            assert_eq!(value["cross"], Value::String("end".to_string()));
        }
        other => panic!("expected an align InspectorEdit, got {other:?}"),
    }
}

/// The canonicalize button is an Action (no value): a press resolves to
/// `InspectorAction`, not an `InspectorEdit`.
#[test]
fn canonicalize_button_resolves_to_an_inspector_action() {
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);
    assert_eq!(
        resolve(&Action::Pressed("insp:canonicalize".to_string()), &m),
        Some(Intent::InspectorAction {
            control_id: "canonicalize".to_string()
        })
    );
}

// ---- (5) re-feed preserves a focused/edited field through the real runtime ----

/// A re-fed `UiModel` (same selection, a toggled theme) keeps a focused inspector
/// TextInput's typed value through the REAL `UiRuntime::set_tree` cache — a theme
/// flip or selection refresh must not wipe an in-progress edit. FAILS if set_tree
/// drops the cached value (the P4 re-derive hazard).
#[test]
fn refed_model_preserves_a_focused_inspector_field_edit() {
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let catalog = object_command_catalog();
    let viewport = (1280.0, 800.0);
    let light = build_root(&model(&catalog, &view));

    let mut rt = UiRuntime::new(light, viewport, false);

    // Find the Name field's resolved screen box and click it to focus, then type.
    let scene_objs = rt.render();
    let name_box = scene_objs
        .objects
        .iter()
        .find(|o| o.id == "insp:name")
        .expect("name field");
    let (nx, ny) = (
        name_box.transform[0][2] + 4.0,
        name_box.transform[1][2] + 4.0,
    );
    let down = rt.dispatch_pointer(PointerPhase::Down, (nx, ny));
    assert!(
        rt.has_text_focus(),
        "clicking the Name field grabs focus (edit: {:?})",
        down.edit
    );
    rt.dispatch_key(&shape_ui_core::KeyInput {
        key: "X".to_string(),
        text: Some("X".to_string()),
        ctrl: false,
        meta: false,
        alt: false,
    });

    // Re-feed an identical tree in the DARK theme (the P4 per-change re-derive).
    let dark = build_root(&UiModel {
        theme_dark: true,
        ..model(&catalog, &view)
    });
    rt.set_tree(dark);
    rt.set_theme(true);

    let after = rt.render();
    let name_value = after
        .objects
        .iter()
        .find(|o| o.id == "insp:name::value")
        .expect("name value");
    assert_eq!(
        name_value.text.as_ref().unwrap().runs[0].text,
        "X",
        "the typed edit survives a re-fed/re-themed tree"
    );
}

// ---- toolbar active-state binding ----

/// The toolbar marks the armed tool/insert active off the model mirror (a
/// VisualState fill), never a raw flag. In the material-tray language an active
/// button body fills `accent-soft` (NOT the old bright-blue) and its glyph tints
/// `selection-ring`; an inactive button has NO body fill (the frosted tray shows
/// through) and a `text`-tinted glyph. FAILS if the active mark reads something
/// other than the `active_tool`/`create_kind` mirror.
#[test]
fn toolbar_marks_the_active_tool_off_the_model_mirror() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = UiModel {
        active_tool: "draw",
        create_kind: Some("insert-ellipse"),
        ..model(&catalog, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // A borderless body carries an EXPLICIT fully-transparent fill (the
    // decorative-empty encoding); read that as "no active background" (`None`) so the
    // active-mark assertion stays about the `accent-soft` tint, not the transparency.
    let body_fill = |id: &str| -> Option<String> {
        let o = scene
            .objects
            .iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("no {id}"));
        o.fill.as_ref().and_then(|f| {
            if f.opacity <= f64::EPSILON {
                return None;
            }
            match &f.paint {
                shape_renderer_core::render_object::RPaint::Token { name } => Some(name.clone()),
                other => panic!("expected token fill, got {other:?}"),
            }
        })
    };
    let glyph_tint = |id: &str| -> String {
        let o = scene
            .objects
            .iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("no {id}"));
        match &o.stroke.as_ref().expect("glyph stroke").paint {
            shape_renderer_core::render_object::RPaint::Token { name } => name.clone(),
            other => panic!("expected token stroke, got {other:?}"),
        }
    };
    assert_eq!(
        body_fill("cmd:draw").as_deref(),
        Some("accent-soft"),
        "armed tool body is accent-soft"
    );
    assert_eq!(
        body_fill("cmd:select-move"),
        None,
        "unarmed tool body has no fill"
    );
    assert_eq!(
        glyph_tint("cmd:draw::icon"),
        "selection-ring",
        "armed tool glyph is accent-tinted"
    );
    assert_eq!(
        glyph_tint("cmd:select-move::icon"),
        "text",
        "unarmed tool glyph is text-tinted"
    );
    assert_eq!(
        body_fill("cmd:insert-ellipse").as_deref(),
        Some("accent-soft"),
        "armed insert is accent-soft"
    );
    assert_eq!(
        body_fill("cmd:insert-rectangle"),
        None,
        "unarmed insert body has no fill"
    );
}

/// Collect every `Widget::Icon` id in the tree (the icon-only-toolbar guard reads
/// these; `owner_ids`/`all_ids` either skip Icons or fold them in with Rects).
fn icon_ids(tree: &Widget, out: &mut Vec<String>) {
    match tree {
        Widget::Container(c) => {
            for child in &c.children {
                icon_ids(child, out);
            }
        }
        Widget::Icon(i) => out.push(i.id.clone()),
        _ => {}
    }
}

/// The redesigned tray draws each command as an ICON, not a text label: every
/// surfaced `cmd:<id>` body has a sibling `cmd:<id>::icon` Icon widget, AND the
/// rendered scene carries NO non-empty text run for any toolbar command body
/// (`cmd:<id>::label`). FAILS the moment a command regresses to a wrapping text
/// label or loses its glyph — the icon-tray contract.
#[test]
fn toolbar_draws_an_icon_per_command_not_a_text_label() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    let tree = build_root(&m);

    // Every command-body id present in the tree has a matching `::icon` Icon.
    let mut owners = Vec::new();
    owner_ids(&tree, &mut owners);
    let cmd_bodies: Vec<&str> = owners
        .iter()
        .filter_map(|id| {
            let c = id.strip_prefix("cmd:")?;
            // The chrome theme toggle is a real labeled Button, not a tray glyph;
            // scope this guard to the tray's command bodies.
            (c != "toggle-theme").then_some(c)
        })
        .collect();
    assert!(
        !cmd_bodies.is_empty(),
        "the tray must surface command bodies"
    );

    let mut icons = Vec::new();
    icon_ids(&tree, &mut icons);
    for c in &cmd_bodies {
        let icon = format!("cmd:{c}::icon");
        assert!(
            icons.contains(&icon),
            "command {c} must draw a glyph (`{icon}`), not a label"
        );
    }

    // No rendered text run belongs to a tray command body's label.
    let scene = shape_ui_core::render(&tree, (1280.0, 800.0), false);
    for o in &scene.objects {
        if let Some(c) =
            o.id.strip_prefix("cmd:")
                .and_then(|s| s.strip_suffix("::label"))
        {
            if c == "toggle-theme" {
                continue;
            }
            let has_text = o
                .text
                .as_ref()
                .is_some_and(|t| t.runs.iter().any(|r| !r.text.is_empty()));
            assert!(
                !has_text,
                "tray command {c} must not render a text label run"
            );
        }
    }
}

// ---- swatch + text field edits ----

/// A palette swatch press resolves to a pen-color selection (`swatch:<hex>` →
/// SelectColor).
#[test]
fn swatch_press_resolves_to_select_color() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    assert_eq!(
        resolve(&Action::Pressed("swatch:#ff375f".to_string()), &m),
        Some(Intent::SelectColor("#ff375f".to_string()))
    );
}

/// A Paint control's hex text commit authors a solid paint value; a Number control's
/// commit parses to a number and drops a non-numeric string (validation in
/// `resolve`, so ui-core's TextInput stays one type).
#[test]
fn paint_and_number_text_commits_shape_their_wire_values() {
    let mut obj = rect("t");
    obj.text = Some(Text {
        runs: vec![TextRun {
            text: "hi".into(),
            color: Some("#112233".into()),
            size: Some(24),
            bold: false,
            italic: false,
            font: Some("Sans".into()),
        }],
        align: TextAlign::Start,
        valign: TextVAlign::Top,
    });
    let scene = scene_with(vec![obj]);
    let view = view_of(&scene, "t");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    // text-color is a Paint control: a hex commit authors {kind:solid,color}.
    let paint = resolve(
        &Action::TextChanged {
            id: "insp:text-color".to_string(),
            text: "#abcdef".to_string(),
        },
        &m,
    );
    match paint {
        Some(Intent::InspectorEdit { value, op_kind, .. }) => {
            assert_eq!(op_kind, "set-text");
            assert_eq!(value["kind"], Value::String("solid".to_string()));
            assert_eq!(value["color"], Value::String("#abcdef".to_string()));
        }
        other => panic!("expected a text-color paint edit, got {other:?}"),
    }

    // font-size is a Number control: "30" parses to 30.0; "abc" drops (no edit).
    match resolve(
        &Action::TextChanged {
            id: "insp:font-size".to_string(),
            text: "30".to_string(),
        },
        &m,
    ) {
        Some(Intent::InspectorEdit { value, .. }) => assert_eq!(value.as_f64(), Some(30.0)),
        other => panic!("expected a font-size number edit, got {other:?}"),
    }
    assert_eq!(
        resolve(
            &Action::TextChanged {
                id: "insp:font-size".to_string(),
                text: "abc".to_string()
            },
            &m
        ),
        None,
        "a non-numeric font-size commit is dropped"
    );
}

/// A rotated free object's inspector still surfaces the placement controls, and a
/// rotation number commit carries `set-transform`/`rotation`. (Drives the real
/// decomposition-backed view.)
#[test]
fn rotation_number_commit_routes_to_set_transform() {
    let mut obj = rect("r");
    obj.transform = compose_affine((0.0, 0.0), 30.0_f64.to_radians(), (1.0, 1.0), 0.0);
    let scene = scene_with(vec![obj]);
    let view = view_of(&scene, "r");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    match resolve(
        &Action::TextChanged {
            id: "insp:rotation".to_string(),
            text: "45".to_string(),
        },
        &m,
    ) {
        Some(Intent::InspectorEdit {
            op_kind,
            field,
            value,
            ..
        }) => {
            assert_eq!(op_kind, "set-transform");
            assert_eq!(field.as_deref(), Some("rotation"));
            assert_eq!(value.as_f64(), Some(45.0));
        }
        other => panic!("expected a rotation set-transform edit, got {other:?}"),
    }
}

/// `build_root` over an empty (canvas) selection still renders the toolbar but no
/// inspector panel — the panel hides when the view is empty.
#[test]
fn empty_selection_renders_toolbar_without_inspector() {
    let catalog = object_command_catalog();
    let scene = scene_with(vec![rect("r")]);
    let empty_view = inspector_view(&scene, &ObjectSelection::Canvas, &StubOutlineDeriver);
    let m = model(&catalog, &empty_view);
    let tree = build_root(&m);
    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);
    assert!(
        ids.iter().any(|id| id.starts_with("cmd:")),
        "toolbar present"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("insp:")),
        "no inspector for an empty selection"
    );
}

// ============================================================================
// Settings modal, context menu, presence, status/chrome — the remaining built-in
// UIs. Each test drives the REAL catalogs + the REAL ui-core render; a behavior
// regression (a dropped catalog row, a broken binding, a rebaked theme flip) fails.
// ============================================================================

/// Every rendered RenderObject id in the laid-out scene (the ids the renderer emits,
/// including non-interactive Text/Rect the `owner_ids` walker skips).
fn rendered_ids(tree: &Widget, theme_dark: bool) -> Vec<String> {
    shape_ui_core::render(tree, (1280.0, 800.0), theme_dark)
        .objects
        .into_iter()
        .map(|o| o.id)
        .collect()
}

// ---- (S1) settings modal single-source pin ----

/// The settings modal emits EXACTLY one labelled row per command-catalog entry AND
/// one per gesture-catalog entry — the single-source rule. Registering a command or
/// gesture in scene-core makes it appear here with zero edits in `settings.rs`; this
/// FAILS the moment a catalog entry is dropped from (or an extra row leaks into) the
/// modal.
#[test]
fn settings_modal_emits_one_row_per_command_and_gesture_catalog_entry() {
    let commands = object_command_catalog();
    let gestures = object_gesture_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let m = UiModel {
        settings_open: true,
        gesture_catalog: &gestures,
        ..model(&commands, &view)
    };
    let mut ids = Vec::new();
    all_ids(&build_root(&m), &mut ids);

    // The row container ids the modal emits are `settings::cmd::<id>::row` /
    // `settings::gesture::<id>::row`. Each catalog entry must have exactly one.
    for cmd in &commands {
        let row = format!("settings::cmd::{}::row", cmd.id);
        let count = ids.iter().filter(|id| **id == row).count();
        assert_eq!(
            count, 1,
            "command {} must have exactly one settings row",
            cmd.id
        );
    }
    for gesture in &gestures {
        let row = format!("settings::gesture::{}::row", gesture.id);
        let count = ids.iter().filter(|id| **id == row).count();
        assert_eq!(
            count, 1,
            "gesture {} must have exactly one settings row",
            gesture.id
        );
    }

    // No settings command row exists for an id that ISN'T in the catalog (no leak).
    let cmd_rows: Vec<&str> = ids
        .iter()
        .filter_map(|id| {
            id.strip_prefix("settings::cmd::")
                .and_then(|s| s.strip_suffix("::row"))
        })
        .collect();
    assert_eq!(
        cmd_rows.len(),
        commands.len(),
        "no extra/duplicate command rows"
    );
}

/// The modal is READ-ONLY: a press on ANY settings widget id (rows, labels, kbd
/// pills) resolves to nothing actuable except the scrim's `Dismiss`. Driven through
/// the real `resolve`, so a settings id that accidentally became a `cmd:`/`insp:`
/// actuator FAILS here — the behavioral guarantee, not a substring check.
#[test]
fn settings_modal_rows_are_read_only() {
    let commands = object_command_catalog();
    let gestures = object_gesture_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let m = UiModel {
        settings_open: true,
        gesture_catalog: &gestures,
        ..model(&commands, &view)
    };
    let mut ids = Vec::new();
    all_ids(&build_root(&m), &mut ids);
    for id in ids.iter().filter(|id| id.starts_with("settings::")) {
        let intent = resolve(&Action::Pressed(id.clone()), &m);
        // The scrim authors Dismiss; every other settings widget authors nothing.
        if id == "settings::scrim" {
            assert_eq!(intent, Some(Intent::Dismiss));
        } else {
            assert_eq!(
                intent, None,
                "settings widget {id} must not resolve to an op"
            );
        }
    }
}

/// A press on the settings scrim resolves to `Dismiss` — the only way the modal
/// authors anything.
#[test]
fn settings_scrim_press_resolves_to_dismiss() {
    let commands = object_command_catalog();
    let gestures = object_gesture_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let m = UiModel {
        settings_open: true,
        gesture_catalog: &gestures,
        ..model(&commands, &view)
    };
    assert_eq!(
        resolve(&Action::Pressed("settings::scrim".to_string()), &m),
        Some(Intent::Dismiss)
    );
}

/// The shortcut formatter resolves `Mod`/`Shift` to the platform symbols off the fed
/// `is_mac` bit (the cores read no platform). The modal's `Mod+Z` row text differs
/// mac vs non-mac. FAILS if the formatting hard-codes one platform or ignores the bit.
#[test]
fn settings_shortcut_formatting_honors_the_fed_is_mac_bit() {
    use crate::settings::format_shortcut;
    assert_eq!(format_shortcut("Mod+Z", true), "⌘Z");
    assert_eq!(format_shortcut("Mod+Z", false), "Ctrl+Z");
    assert_eq!(format_shortcut("Mod+Shift+G", true), "⌘⇧G");
    assert_eq!(format_shortcut("Backspace", false), "Backspace");
}

// ---- (C1) context menu items round-trip to catalog Commands ----

fn ctx_item(command_id: &str, label: &str) -> ContextMenuItem {
    ContextMenuItem {
        command_id: Some(command_id.to_string()),
        label: label.to_string(),
        danger: false,
        disabled: false,
    }
}

/// Every enabled context-menu item is bound to a catalog command (`cmd:<id>`) and a
/// press round-trips to `Command(<id>)` — the same single binding source as the
/// toolbar (no second op vocabulary). FAILS if a menu row stopped resolving to its
/// command.
#[test]
fn context_menu_items_resolve_to_catalog_commands() {
    let commands = object_command_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let menu = ContextMenuModel {
        x: 100.0,
        y: 120.0,
        title: Some("object:r".to_string()),
        items: vec![
            ctx_item("duplicate", "Duplicate"),
            ContextMenuItem {
                command_id: None,
                label: String::new(),
                danger: false,
                disabled: false,
            },
            ctx_item("delete", "Delete"),
        ],
    };
    let m = UiModel {
        context_menu: Some(&menu),
        ..model(&commands, &view)
    };
    let tree = build_root(&m);
    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);

    for command_id in ["duplicate", "delete"] {
        let id = format!("cmd:{command_id}");
        assert!(
            ids.contains(&id),
            "menu item {command_id} must bind cmd:{command_id}"
        );
        assert_eq!(
            resolve(&Action::Pressed(id), &m),
            Some(Intent::Command(command_id.to_string())),
            "menu cmd:{command_id} must resolve to Command({command_id})"
        );
    }
}

/// A DISABLED context-menu item carries an inert id (no `cmd:` prefix), so a press
/// resolves to nothing — the shell can't fire a gated action. FAILS if a disabled
/// item became actuable.
#[test]
fn disabled_context_menu_item_is_inert() {
    let commands = object_command_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let menu = ContextMenuModel {
        x: 10.0,
        y: 10.0,
        title: None,
        items: vec![ContextMenuItem {
            command_id: Some("ungroup".to_string()),
            label: "Ungroup".to_string(),
            danger: false,
            disabled: true,
        }],
    };
    let m = UiModel {
        context_menu: Some(&menu),
        ..model(&commands, &view)
    };
    let tree = build_root(&m);
    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);
    // The disabled item carries the inert id, NOT a `cmd:` actuator.
    assert!(
        ids.contains(&"context-menu::disabled::ungroup".to_string()),
        "a disabled item carries the inert id"
    );
    // Pressing the inert id resolves to nothing — the gated action can't fire.
    assert_eq!(
        resolve(
            &Action::Pressed("context-menu::disabled::ungroup".to_string()),
            &m
        ),
        None,
        "a disabled menu item is inert"
    );
}

/// A press on the context-menu scrim resolves to `Dismiss`.
#[test]
fn context_menu_scrim_press_resolves_to_dismiss() {
    let commands = object_command_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let menu = ContextMenuModel {
        x: 0.0,
        y: 0.0,
        title: None,
        items: vec![ctx_item("duplicate", "Duplicate")],
    };
    let m = UiModel {
        context_menu: Some(&menu),
        ..model(&commands, &view)
    };
    assert_eq!(
        resolve(&Action::Pressed("context-menu::scrim".to_string()), &m),
        Some(Intent::Dismiss)
    );
}

// ---- (P1) presence cursors use the peer's literal color ----

/// A peer cursor glyph + label pill paint in the peer's LITERAL color (Solid/Hex),
/// never a theme token — a theme flip must not recolor a peer. FAILS if the glyph or
/// pill resolved to a token. Also pins the cursor lays out at the fed SCREEN coord.
#[test]
fn presence_cursor_paints_the_peer_literal_color_at_the_screen_point() {
    let commands = object_command_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let peers = vec![PeerCursor {
        user_id: "u1".to_string(),
        screen: [300.0, 220.0],
        color: "#ff5733".to_string(),
        label: "Ada".to_string(),
    }];
    let m = UiModel {
        peers: &peers,
        ..model(&commands, &view)
    };
    let scene_objs = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // The label pill is a Solid fill of the peer color, not a token.
    let pill = scene_objs
        .objects
        .iter()
        .find(|o| o.id == "presence::u1::pill-bg")
        .expect("peer pill present");
    match &pill.fill.as_ref().expect("pill has fill").paint {
        RPaint::Solid { color } => assert_eq!(color, "#ff5733", "pill uses the peer literal color"),
        other => panic!("peer pill must be a literal Solid, got {other:?}"),
    }
    // The glyph text run is the peer color hex (not a token-resolved theme color).
    let glyph = scene_objs
        .objects
        .iter()
        .find(|o| o.id == "presence::u1::glyph")
        .expect("peer glyph present");
    assert_eq!(
        glyph.text.as_ref().unwrap().runs[0].color,
        "#ff5733",
        "glyph uses the peer literal color"
    );
    // The glyph sits at the cursor container origin, so its transform translate is the
    // fed screen point (projection stays shell-side; this layer only places).
    assert_eq!(glyph.transform[0][2], 300.0, "cursor x is the fed screen x");
    assert_eq!(glyph.transform[1][2], 220.0, "cursor y is the fed screen y");
}

// ---- (T1) theme-toggle is catalog-bound ----

/// The theme-toggle chrome binds to the `toggle-theme` command-catalog entry
/// (`cmd:toggle-theme`) and a press resolves to `Command("toggle-theme")` — so it
/// self-documents in the settings modal (no raw shell theme flag). FAILS if the
/// toggle stopped being a catalog command (the single-source gap the design closes).
#[test]
fn theme_toggle_is_a_catalog_command() {
    let commands = object_command_catalog();
    assert!(
        commands.iter().any(|c| c.id == "toggle-theme"),
        "toggle-theme must exist in the command catalog (single source)"
    );
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");
    let m = model(&commands, &view);
    let tree = build_root(&m);
    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);
    assert!(
        ids.contains(&"cmd:toggle-theme".to_string()),
        "theme toggle binds cmd:toggle-theme"
    );
    assert_eq!(
        resolve(&Action::Pressed("cmd:toggle-theme".to_string()), &m),
        Some(Intent::Command("toggle-theme".to_string()))
    );
}

// ---- (Z1) zero-rebake theme flip across ALL the new UIs active ----

/// With EVERY built-in UI active (settings open, a context menu, a status strip +
/// toast, a peer cursor, the theme toggle), a theme flip changes ONLY text colors —
/// every `geometry_d` and every token fill stays byte-identical (no re-tessellation,
/// no rebake). Peer-literal Solid colors stay fixed across the flip too. FAILS if any
/// new UI rebakes geometry or changes a token fill on a theme flip — the perf bar.
#[test]
fn theme_flip_across_all_new_uis_is_text_only_zero_rebake() {
    let commands = object_command_catalog();
    let gestures = object_gesture_catalog();
    let scene = flow_scene();
    let view = view_of(&scene, "p");
    let peers = vec![PeerCursor {
        user_id: "u1".to_string(),
        screen: [200.0, 200.0],
        color: "#22aa55".to_string(),
        label: "Bo".to_string(),
    }];
    let menu = ContextMenuModel {
        x: 400.0,
        y: 300.0,
        title: Some("object:p".to_string()),
        items: vec![
            ctx_item("duplicate", "Duplicate"),
            ctx_item("delete", "Delete"),
        ],
    };
    let base = UiModel {
        gesture_catalog: &gestures,
        settings_open: true,
        context_menu: Some(&menu),
        peers: &peers,
        busy: true,
        status: Some("Saving…"),
        toast: Some("Copied"),
        ..model(&commands, &view)
    };
    let light = shape_ui_core::render(
        &build_root(&UiModel {
            theme_dark: false,
            ..base
        }),
        (1280.0, 800.0),
        false,
    );
    let dark = shape_ui_core::render(
        &build_root(&UiModel {
            theme_dark: true,
            ..base
        }),
        (1280.0, 800.0),
        true,
    );

    // The theme toggle GLYPH legitimately changes (Sun↔Moon), so its label text run
    // text differs; exclude only that one object from the geometry-identity scan.
    assert_eq!(
        light.objects.len(),
        dark.objects.len(),
        "same object count across theme"
    );
    let mut text_changed = false;
    for (l, d) in light.objects.iter().zip(&dark.objects) {
        assert_eq!(l.id, d.id, "object order/ids identical across theme");
        if l.id == "cmd:toggle-theme::label" {
            continue; // the toggle glyph flips Sun/Moon by design.
        }
        assert_eq!(
            l.geometry_d, d.geometry_d,
            "{}: geometry must not rebake on theme flip",
            l.id
        );
        assert_eq!(
            fill_repr(l),
            fill_repr(d),
            "{}: token/literal fill must not change",
            l.id
        );
        if let (Some(lt), Some(dt)) = (&l.text, &d.text) {
            if lt.runs.first().map(|r| &r.color) != dt.runs.first().map(|r| &r.color) {
                text_changed = true;
            }
        }
    }
    assert!(
        text_changed,
        "a theme flip must re-resolve at least one token text color"
    );
}

// ---- (St1) status strip presence is gated on busy / non-Ready ----

/// The status strip shows iff busy or the status is non-`Ready`; the toast shows iff a
/// message is set. A steady (Ready, not busy, no toast) model emits NO status chrome.
/// FAILS if the strip/toast leaked into the steady state or hid when it should show.
#[test]
fn status_chrome_is_gated_on_transient_state() {
    let commands = object_command_catalog();
    let scene = scene_with(vec![rect("r")]);
    let view = view_of(&scene, "r");

    let steady = UiModel {
        busy: false,
        status: Some("Ready"),
        toast: None,
        ..model(&commands, &view)
    };
    let steady_ids = rendered_ids(&build_root(&steady), false);
    assert!(
        !steady_ids.iter().any(|id| id.starts_with("status::")),
        "no status chrome when steady"
    );

    let busy = UiModel {
        busy: true,
        status: Some("Saving…"),
        toast: Some("Saved"),
        ..model(&commands, &view)
    };
    let busy_ids = rendered_ids(&build_root(&busy), false);
    assert!(
        busy_ids.iter().any(|id| id == "status::strip-bg"),
        "busy shows the status strip"
    );
    assert!(
        busy_ids.iter().any(|id| id == "status::spinner"),
        "busy shows the spinner"
    );
    assert!(
        busy_ids.iter().any(|id| id == "status::toast-bg"),
        "a toast message shows the toast"
    );
}

// ---- P3 review-fix: toolbar parity (erase, color, stroke, templates, switcher, diagnostics) ----

/// A model carrying a palette + selected color + widths so the new toolbar controls
/// render. Built off the base `model` fixture, overriding only the new fields.
fn parity_model<'a>(
    catalog: &'a [shape_scene_core::object::catalog::commands::ObjectCommand],
    view: &'a InspectorView,
    palette: &'a [String],
    widths: &'a [f64],
    canvases: &'a [crate::CanvasEntry],
    templates: &'a [crate::TemplateEntry],
) -> UiModel<'a> {
    UiModel {
        selected_color: "#ff0000",
        pen_palette: palette,
        pen_width: 4.0,
        pen_widths: widths,
        canvases,
        active_canvas_id: "c2",
        templates,
        ..model(catalog, view)
    }
}

/// The eraser is a first-class toolbar tool: `cmd:erase` is surfaced AND armed-active
/// off the `active_tool` mirror. FAILS if the erase command was dropped from the
/// catalog/toolbar (P4 would then lose the only way to arm the eraser).
#[test]
fn toolbar_surfaces_the_erase_tool_and_marks_it_active() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = UiModel {
        active_tool: "erase",
        ..model(&catalog, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    let erase = scene
        .objects
        .iter()
        .find(|o| o.id == "cmd:erase")
        .expect("erase toolbar button");
    match &erase
        .fill
        .as_ref()
        .expect("armed eraser has an accent-soft body fill")
        .paint
    {
        RPaint::Token { name } => assert_eq!(name, "accent-soft", "armed eraser reads active"),
        other => panic!("expected token fill, got {other:?}"),
    }
    // It is a real catalog command (single-source), so a press resolves to a Command.
    assert_eq!(
        resolve(&Action::Pressed("cmd:erase".to_string()), &m),
        Some(Intent::Command("erase".to_string()))
    );
}

/// The toolbar emits a `swatch:<hex>` chip per palette color — a REAL producer for the
/// SelectColor path (closing the dead-capability gap) — and the selected color's chip
/// reads `selected`. FAILS if the palette stops producing swatch chips (then the
/// SelectColor binding would again have no built-in producer).
#[test]
fn toolbar_emits_a_color_swatch_per_palette_entry() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let palette = vec!["#ff0000".to_string(), "#00ff00".to_string()];
    let m = parity_model(&catalog, &view, &palette, &[], &[], &[]);

    let mut ids = Vec::new();
    owner_ids(&build_root(&m), &mut ids);
    assert!(
        ids.iter().any(|id| id == "swatch:#ff0000"),
        "a chip for the first palette color"
    );
    assert!(
        ids.iter().any(|id| id == "swatch:#00ff00"),
        "a chip for the second palette color"
    );

    // The selected color's chip is marked selected; the others are not.
    let tree = build_root(&m);
    assert!(
        swatch_selected(&tree, "swatch:#ff0000"),
        "the selected color's chip is ringed"
    );
    assert!(
        !swatch_selected(&tree, "swatch:#00ff00"),
        "an unselected color's chip is not"
    );

    // The chip press resolves to a pen-color selection.
    assert_eq!(
        resolve(&Action::Pressed("swatch:#00ff00".to_string()), &m),
        Some(Intent::SelectColor("#00ff00".to_string()))
    );
}

/// Find a Swatch by id and return its `selected` flag.
fn swatch_selected(tree: &Widget, id: &str) -> bool {
    fn walk(w: &Widget, id: &str) -> Option<bool> {
        match w {
            Widget::Swatch(s) if s.id == id => Some(s.selected),
            Widget::Container(c) => c.children.iter().find_map(|c| walk(c, id)),
            _ => None,
        }
    }
    walk(tree, id).unwrap_or_else(|| panic!("no swatch {id}"))
}

/// The toolbar emits a `pen-width:<px>` chip per pickable width, the active one marked,
/// and a press resolves to `SelectPenWidth`. FAILS if pen width can't be set from the
/// toolbar (the stroke control gap).
#[test]
fn toolbar_emits_a_pen_width_chip_per_width_and_resolves() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let widths = vec![2.0, 4.0, 8.0];
    let m = parity_model(&catalog, &view, &[], &widths, &[], &[]);

    let mut ids = Vec::new();
    owner_ids(&build_root(&m), &mut ids);
    assert!(
        ids.iter().any(|id| id == "pen-width:2"),
        "a chip for width 2"
    );
    assert!(
        ids.iter().any(|id| id == "pen-width:8"),
        "a chip for width 8"
    );

    // The active width (4.0 in parity_model) reads the accent-soft body fill.
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);
    let active = scene
        .objects
        .iter()
        .find(|o| o.id == "pen-width:4")
        .expect("active width chip");
    match &active.fill.as_ref().unwrap().paint {
        RPaint::Token { name } => assert_eq!(name, "accent-soft", "active width reads active"),
        other => panic!("expected token fill, got {other:?}"),
    }

    assert_eq!(
        resolve(&Action::Pressed("pen-width:8".to_string()), &m),
        Some(Intent::SelectPenWidth(8.0))
    );
}

/// An open template popup emits a `template:<id>` row per template; a press resolves to
/// `ApplyTemplate`. A closed popup emits nothing. FAILS if template insertion is
/// unreachable once Toolbar.svelte/TemplatePopup.svelte are deleted in P4.
#[test]
fn template_popup_emits_rows_and_resolves_apply() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let templates = vec![
        crate::TemplateEntry {
            id: "kanban".to_string(),
            title: "Kanban".to_string(),
            description: "A board".to_string(),
        },
        crate::TemplateEntry {
            id: "wire".to_string(),
            title: "Wireframe".to_string(),
            description: String::new(),
        },
    ];

    let closed = UiModel {
        template_open: false,
        templates: &templates,
        ..model(&catalog, &view)
    };
    let mut closed_ids = Vec::new();
    owner_ids(&build_root(&closed), &mut closed_ids);
    assert!(
        !closed_ids.iter().any(|id| id.starts_with("template:")),
        "no rows when closed"
    );

    let open = UiModel {
        template_open: true,
        templates: &templates,
        ..model(&catalog, &view)
    };
    let mut open_ids = Vec::new();
    owner_ids(&build_root(&open), &mut open_ids);
    assert!(
        open_ids.iter().any(|id| id == "template:kanban"),
        "a row per template"
    );
    assert!(open_ids.iter().any(|id| id == "template:wire"));

    assert_eq!(
        resolve(&Action::Pressed("template:kanban".to_string()), &open),
        Some(Intent::ApplyTemplate("kanban".to_string()))
    );
    // The toolbar Templates toggle marks active off `template_open`.
    let scene = shape_ui_core::render(&build_root(&open), (1280.0, 800.0), false);
    let toggle = scene
        .objects
        .iter()
        .find(|o| o.id == "cmd:open-template-library")
        .expect("templates toggle");
    match &toggle
        .fill
        .as_ref()
        .expect("open templates toggle has an accent-soft body fill")
        .paint
    {
        RPaint::Token { name } => {
            assert_eq!(name, "accent-soft", "open templates toggle reads active")
        }
        other => panic!("expected token fill, got {other:?}"),
    }
}

/// The canvas switcher emits a `canvas:<id>` tab per canvas (the active one marked), a
/// `canvas-new` button, and a `canvas-delete:<active>` button (only with >1 canvas);
/// each resolves to its intent. FAILS if switching/creating/deleting a canvas becomes
/// unreachable once Toolbar.svelte is deleted.
#[test]
fn canvas_switcher_emits_controls_and_resolves_intents() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let canvases = vec![
        crate::CanvasEntry {
            id: "c1".to_string(),
            title: "First".to_string(),
        },
        crate::CanvasEntry {
            id: "c2".to_string(),
            title: "Second".to_string(),
        },
    ];
    let m = parity_model(&catalog, &view, &[], &[], &canvases, &[]);

    let tree = build_root(&m);
    let mut ids = Vec::new();
    owner_ids(&tree, &mut ids);
    assert!(ids.iter().any(|id| id == "canvas:c1"), "a tab per canvas");
    assert!(ids.iter().any(|id| id == "canvas:c2"));
    assert!(
        ids.iter().any(|id| id == "canvas-new"),
        "a new-canvas button"
    );
    assert!(
        ids.iter().any(|id| id == "canvas-delete:c2"),
        "a delete button for the active canvas"
    );

    assert_eq!(
        resolve(&Action::Pressed("canvas:c1".to_string()), &m),
        Some(Intent::SelectCanvas("c1".to_string()))
    );
    assert_eq!(
        resolve(&Action::Pressed("canvas-new".to_string()), &m),
        Some(Intent::NewCanvas)
    );
    assert_eq!(
        resolve(&Action::Pressed("canvas-delete:c2".to_string()), &m),
        Some(Intent::DeleteCanvas("c2".to_string()))
    );

    // With only one canvas, the delete control is omitted (the ≤1 disabled rule).
    let one = vec![crate::CanvasEntry {
        id: "c1".to_string(),
        title: "Only".to_string(),
    }];
    let m1 = parity_model(&catalog, &view, &[], &[], &one, &[]);
    let mut ids1 = Vec::new();
    owner_ids(&build_root(&m1), &mut ids1);
    assert!(
        !ids1.iter().any(|id| id.starts_with("canvas-delete:")),
        "no delete with a single canvas"
    );
}

/// The diagnostics panel renders its five rows only when open; closed emits nothing.
/// FAILS if the diagnostics readout is unreachable once the App.svelte fragment is
/// deleted in P4.
#[test]
fn diagnostics_panel_renders_rows_only_when_open() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let diag = crate::Diagnostics {
        state: "running".to_string(),
        detail: "ok".to_string(),
        objects: "42".to_string(),
        frame_ms: "1.2".to_string(),
        camera: "0,0 @1.0".to_string(),
    };

    let closed = UiModel {
        diagnostics_open: false,
        diagnostics: Some(&diag),
        ..model(&catalog, &view)
    };
    let closed_ids = rendered_ids(&build_root(&closed), false);
    assert!(
        !closed_ids.iter().any(|id| id.starts_with("diagnostics::")),
        "no panel when closed"
    );

    let open = UiModel {
        diagnostics_open: true,
        diagnostics: Some(&diag),
        ..model(&catalog, &view)
    };
    let open_ids = rendered_ids(&build_root(&open), false);
    assert!(
        open_ids.iter().any(|id| id == "diagnostics::bg"),
        "the panel renders when open"
    );
    assert!(
        open_ids.iter().any(|id| id == "diagnostics::objects"),
        "an objects row renders"
    );
    assert!(
        open_ids.iter().any(|id| id == "diagnostics::frame-ms"),
        "a frame-ms row renders"
    );
}

// ============================================================================
// Phase 4 — Surfaces A: the macOS-material language on inspector / settings /
// diagnostics / template. Each test fails when a surface regresses OUT of the
// material language (a non-`material` panel fill, a `text` label that should be
// `text-secondary`, a non-`accent-soft` selected align cell), not merely when it
// compiles.
// ============================================================================

/// The token name of a rendered object's FILL (panics if the object is missing or its
/// fill isn't a Token).
fn fill_token(scene: &shape_renderer_core::render_object::RenderObjectScene, id: &str) -> String {
    let o = scene
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("no {id}"));
    match &o
        .fill
        .as_ref()
        .unwrap_or_else(|| panic!("{id} has no fill"))
        .paint
    {
        RPaint::Token { name } => name.clone(),
        other => panic!("{id} fill must be a token, got {other:?}"),
    }
}

/// The token name of a rendered object's STROKE (panics if missing / not a Token).
fn stroke_token(scene: &shape_renderer_core::render_object::RenderObjectScene, id: &str) -> String {
    let o = scene
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("no {id}"));
    match &o
        .stroke
        .as_ref()
        .unwrap_or_else(|| panic!("{id} has no stroke"))
        .paint
    {
        RPaint::Token { name } => name.clone(),
        other => panic!("{id} stroke must be a token, got {other:?}"),
    }
}

/// The first text run's color of a rendered Text object (panics if missing / no text).
fn text_color(scene: &shape_renderer_core::render_object::RenderObjectScene, id: &str) -> String {
    let o = scene
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("no {id}"));
    o.text
        .as_ref()
        .unwrap_or_else(|| panic!("{id} has no text"))
        .runs[0]
        .color
        .clone()
}

/// EVERY elevated panel paints in the macOS-material language: a `material` body fill +
/// a `hairline` 1px border + a SINGLE translucent soft-shadow underlay (a theme-
/// independent `#000000` Solid rect). FAILS the moment a panel regresses to the old
/// `surface`/`default-stroke` look, drops its shadow, or stacks MORE than one shadow
/// layer (the concentric-ring defect) — the surface-spec contract.
#[test]
fn every_surface_panel_uses_the_material_body_hairline_border_and_soft_shadow() {
    let commands = object_command_catalog();
    let gestures = object_gesture_catalog();
    let scene_obj = flow_scene();
    let view = view_of(&scene_obj, "p");
    let diag = crate::Diagnostics {
        state: "running".to_string(),
        detail: "ok".to_string(),
        objects: "1".to_string(),
        frame_ms: "1.0".to_string(),
        camera: "0,0".to_string(),
    };
    let templates = vec![crate::TemplateEntry {
        id: "kanban".to_string(),
        title: "Kanban".to_string(),
        description: "A board".to_string(),
    }];
    let m = UiModel {
        gesture_catalog: &gestures,
        settings_open: true,
        diagnostics_open: true,
        diagnostics: Some(&diag),
        template_open: true,
        templates: &templates,
        ..model(&commands, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    for prefix in ["inspector", "settings", "diagnostics", "template"] {
        let bg = format!("{prefix}::bg");
        assert_eq!(
            fill_token(&scene, &bg),
            "material",
            "{prefix} panel body must be `material`"
        );
        assert_eq!(
            stroke_token(&scene, &bg),
            "hairline",
            "{prefix} panel border must be `hairline`"
        );
        // EXACTLY ONE soft-shadow underlay rect (theme-independent literal Solid black,
        // translucent, fill-only). A multi-layer stack of differently-offset rects read
        // as concentric hard rings with gaps — the QA#1 defect — so the panel must carry
        // a single `{prefix}::shadow` layer and NO `{prefix}::shadow0` stack.
        let shadows: Vec<&RenderObject> = scene
            .objects
            .iter()
            .filter(|o| o.id.starts_with(&format!("{prefix}::shadow")))
            .collect();
        assert_eq!(
            shadows.len(),
            1,
            "{prefix} must have exactly ONE shadow layer (no concentric-ring stack), got {}",
            shadows.len()
        );
        let shadow = shadows[0];
        assert_eq!(
            shadow.id,
            format!("{prefix}::shadow"),
            "the single layer is `{prefix}::shadow`"
        );
        let f = shadow.fill.as_ref().expect("shadow fill");
        match &f.paint {
            RPaint::Solid { color } => {
                assert_eq!(color, "#000000", "{} is literal black", shadow.id)
            }
            other => panic!("{} must be a literal Solid, got {other:?}", shadow.id),
        }
        assert!(
            f.opacity > 0.0 && f.opacity < 1.0,
            "{} must be translucent (a solid box ringing the panel is a regression), got {}",
            shadow.id,
            f.opacity
        );
        // Fill-only: a stroked shadow re-introduces a hard ring edge.
        assert!(
            shadow.stroke.is_none(),
            "{} must be fill-only (no stroke ring)",
            shadow.id
        );
    }
}

/// The inspector reads in the secondary-text language: section TITLES stay primary
/// `text`, while control LABELS and trailing UNITS read muted `text-secondary`. FAILS
/// if a control label regresses to the primary token (the muted-caption contract) or a
/// section title goes muted.
#[test]
fn inspector_section_titles_are_primary_text_and_control_labels_are_secondary() {
    let scene_obj = flow_scene();
    let view = view_of(&scene_obj, "c"); // flow child: a sizing-w Number control w/ a unit
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // The Placement section title is primary `text` (light `#1d1d1f`).
    assert_eq!(
        text_color(&scene, "inspector::section::placement"),
        "#1d1d1f",
        "section titles stay primary `text`"
    );
    // A control label is the muted `text-secondary` (light `#8a8a8e`), distinct from
    // the primary token — so a regression to `text` flips this byte.
    assert_eq!(
        text_color(&scene, "inspector::label::sizing-w"),
        "#8a8a8e",
        "control labels read muted `text-secondary`"
    );
    assert_ne!(
        text_color(&scene, "inspector::label::sizing-w"),
        text_color(&scene, "inspector::section::placement"),
        "a muted label must not equal the primary section title"
    );
    // The Number control's trailing unit is muted too.
    assert_eq!(
        text_color(&scene, "sizing-w::unit"),
        "#8a8a8e",
        "trailing units read muted `text-secondary`"
    );
}

/// The align grid marks its SELECTED cell in the active-control language: the chosen
/// `{main,cross}` cell fills `accent-soft`, an idle cell stays `surface-muted`. FAILS
/// if the selected cell loses its accent tint (the active-state token contract).
#[test]
fn align_grid_selected_cell_tints_accent_soft() {
    let scene_obj = flow_scene(); // parent layout: align main=center, cross=start
    let view = view_of(&scene_obj, "p");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // The current align (center/start) cell reads `accent-soft`.
    assert_eq!(
        fill_token(&scene, "insp:align:main=center,cross=start"),
        "accent-soft",
        "the selected align cell tints accent-soft"
    );
    // An idle cell stays the muted surface.
    assert_eq!(
        fill_token(&scene, "insp:align:main=start,cross=end"),
        "surface-muted",
        "an idle align cell stays surface-muted"
    );
}

// ============================================================================
// Phase 5 — Surfaces B: the macOS-material language on the context menu, the status
// strip + toast, and the chrome (theme toggle / canvas switcher / watermark). Each
// test fails when a surface regresses OUT of the material language (a non-`material`
// panel/pill, a `surface-muted` separator that should be `hairline`, a chrome glyph
// that regressed to a text label, a dropped hit id), not merely when it compiles.
// ============================================================================

/// The context menu paints in the macOS-material language: a `material` body fill + a
/// `hairline` 1px border + a 3-layer soft-shadow underlay, and its separators are
/// `hairline` (not the old `surface`/`default-stroke` panel or `surface-muted`
/// divider). FAILS the moment the menu regresses out of the material idiom.
#[test]
fn context_menu_panel_uses_the_material_language() {
    let commands = object_command_catalog();
    let scene_obj = scene_with(vec![rect("r")]);
    let view = view_of(&scene_obj, "r");
    let menu = ContextMenuModel {
        x: 100.0,
        y: 120.0,
        title: Some("object:r".to_string()),
        items: vec![
            ctx_item("duplicate", "Duplicate"),
            ContextMenuItem {
                command_id: None,
                label: String::new(),
                danger: false,
                disabled: false,
            },
            ctx_item("delete", "Delete"),
        ],
    };
    let m = UiModel {
        context_menu: Some(&menu),
        ..model(&commands, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    assert_eq!(
        fill_token(&scene, "context-menu::bg"),
        "material",
        "menu body must be `material`"
    );
    assert_eq!(
        stroke_token(&scene, "context-menu::bg"),
        "hairline",
        "menu border must be `hairline`"
    );
    // EXACTLY ONE soft-shadow underlay rect (theme-independent literal Solid black,
    // translucent, fill-only) — a multi-layer stack reads as concentric rings (QA#1).
    let shadows: Vec<&RenderObject> = scene
        .objects
        .iter()
        .filter(|o| o.id.starts_with("context-menu::shadow"))
        .collect();
    assert_eq!(
        shadows.len(),
        1,
        "the menu has exactly ONE shadow layer, got {}",
        shadows.len()
    );
    let f = shadows[0].fill.as_ref().expect("shadow fill");
    match &f.paint {
        RPaint::Solid { color } => assert_eq!(color, "#000000", "the menu shadow is literal black"),
        other => panic!("the menu shadow must be a literal Solid, got {other:?}"),
    }
    assert!(
        f.opacity > 0.0 && f.opacity < 1.0,
        "the menu shadow is translucent, got {}",
        f.opacity
    );
    assert!(
        shadows[0].stroke.is_none(),
        "the menu shadow is fill-only (no ring)"
    );
    // The divider between Duplicate and Delete is a `hairline` (index 1, the separator).
    assert_eq!(
        fill_token(&scene, "context-menu::sep::1"),
        "hairline",
        "menu separators read `hairline`"
    );
}

/// The dismiss scrim must not wash out the canvas: its RESOLVED fill RGBA — the value
/// fed to the GPU after `paint_color` lowers the scrim's `Solid` paint — has alpha 0.0
/// in BOTH themes, so the world + panels stay visible behind the open menu. Drives the
/// real `ui-core::render` → `build_scene_geometry_themed` seam the renderer runs, then
/// reads the scrim's index-aligned `FillInstance.fill`. FAILS the moment the scrim
/// resolves to an opaque/near-white slab (e.g. the old 8-digit `#00000000`, which the
/// renderer's RGB-only `parse_hex_rgb` rejected to opaque white).
#[test]
fn context_menu_scrim_resolves_fully_transparent_in_both_themes() {
    use shape_renderer_core::object_pipeline::build_scene_geometry_themed;
    use shape_renderer_core::Theme;

    let commands = object_command_catalog();
    let scene_obj = scene_with(vec![rect("r")]);
    let view = view_of(&scene_obj, "r");
    let menu = ContextMenuModel {
        x: 400.0,
        y: 300.0,
        title: Some("object:r".to_string()),
        items: vec![
            ctx_item("duplicate", "Duplicate"),
            ctx_item("delete", "Delete"),
        ],
    };

    for (theme_dark, theme) in [(false, Theme::light()), (true, Theme::dark())] {
        let m = UiModel {
            theme_dark,
            context_menu: Some(&menu),
            ..model(&commands, &view)
        };
        let render = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), theme_dark);
        let geometry = build_scene_geometry_themed(&render, theme);

        let idx = geometry
            .draws
            .iter()
            .position(|d| d.id == "context-menu::scrim")
            .expect("the open menu emits its dismiss scrim");
        // `draws` and `fill_instances` are index-aligned; `.fill` is the GPU-bound RGBA.
        let rgba = geometry.fill_instances[idx].fill;
        assert_eq!(
            rgba[3], 0.0,
            "context-menu scrim must resolve fully transparent (alpha 0), got {rgba:?} (dark={theme_dark})"
        );
    }
}

/// The status strip is a frosted `material` pill with a `hairline` border, and the
/// toast is a `surface-muted` pill with a `hairline` border — the macOS capsule look.
/// FAILS if either regresses to the old `surface`/`default-stroke` pill.
#[test]
fn status_strip_and_toast_read_in_the_material_language() {
    let commands = object_command_catalog();
    let scene_obj = scene_with(vec![rect("r")]);
    let view = view_of(&scene_obj, "r");
    let m = UiModel {
        busy: true,
        status: Some("Saving…"),
        toast: Some("Copied"),
        ..model(&commands, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    assert_eq!(
        fill_token(&scene, "status::strip-bg"),
        "material",
        "the status strip is `material`"
    );
    assert_eq!(
        stroke_token(&scene, "status::strip-bg"),
        "hairline",
        "the strip border is `hairline`"
    );
    assert_eq!(
        fill_token(&scene, "status::toast-bg"),
        "surface-muted",
        "the toast is `surface-muted`"
    );
    assert_eq!(
        stroke_token(&scene, "status::toast-bg"),
        "hairline",
        "the toast border is `hairline`"
    );
}

/// The chrome theme toggle is now an ICON, not a text-glyph label: it keeps the
/// `cmd:toggle-theme` hit body (a material button), draws a `cmd:toggle-theme::icon`
/// registry glyph, renders NO `cmd:toggle-theme::label` text run, and still resolves to
/// the catalog `Command("toggle-theme")` (so it self-documents in settings). FAILS if
/// the toggle regressed to a label, lost its glyph, or stopped resolving.
#[test]
fn theme_toggle_is_an_icon_chrome_button_bound_to_the_catalog_command() {
    let commands = object_command_catalog();
    let scene_obj = scene_with(vec![rect("r")]);
    let view = view_of(&scene_obj, "r");
    let m = model(&commands, &view);
    let tree = build_root(&m);

    // The hit body keeps its catalog-command id.
    let mut owners = Vec::new();
    owner_ids(&tree, &mut owners);
    assert!(
        owners.contains(&"cmd:toggle-theme".to_string()),
        "the toggle keeps its `cmd:toggle-theme` body"
    );
    assert_eq!(
        resolve(&Action::Pressed("cmd:toggle-theme".to_string()), &m),
        Some(Intent::Command("toggle-theme".to_string())),
        "the toggle still resolves to the catalog command"
    );
    // It draws a glyph, not a text label.
    let mut icons = Vec::new();
    icon_ids(&tree, &mut icons);
    assert!(
        icons.contains(&"cmd:toggle-theme::icon".to_string()),
        "the toggle draws a registry glyph"
    );
    let scene = shape_ui_core::render(&tree, (1280.0, 800.0), false);
    assert!(
        !scene
            .objects
            .iter()
            .any(|o| o.id == "cmd:toggle-theme::label"),
        "the toggle renders no text-glyph label"
    );
    // The glyph is stroked with the `text` token (a recolorable paint, not a literal).
    assert_eq!(
        stroke_token(&scene, "cmd:toggle-theme::icon"),
        "text",
        "the toggle glyph is a `text`-token stroke"
    );

    // The glyph child must NOT steal the body hit: a press landing OVER the centered
    // glyph still resolves to the `cmd:toggle-theme` body, never the `::icon` (which
    // contributes no pick box) or a canvas fall-through. This is the exact path the
    // toggle's flip rides — pinning it falsifiably guards against an icon refactor that
    // drops the body hit (the QA #4 "toggle does nothing" symptom).
    let center = (16.0 + 36.0 / 2.0, 16.0 + 36.0 / 2.0);
    assert_eq!(
        shape_ui_core::hit(&tree, center),
        Some("cmd:toggle-theme".to_string()),
        "a press over the toggle glyph hits the `cmd:toggle-theme` body, not the non-hittable glyph"
    );
}

/// The LIVE theme-toggle click path end to end: a real `UiRuntime` over the built
/// chrome, a `dispatch_pointer` Down THEN Up at the toggle's center, must EMIT
/// `Action::Pressed("cmd:toggle-theme")`, which `resolve` maps to the catalog
/// `Command("toggle-theme")` the shell runs to flip the theme. This drives the exact
/// actuation `hit`+`resolve` alone never exercised — the toggle is an icon-only
/// hoverable Rect body, and before the `find_kind` hoverable-Rect arm a click showed
/// the hover background yet emitted NO action, so the theme never flipped (the live
/// "toggle does nothing" defect). FAILS if a press on the toggle body emits no
/// Pressed action or it stops resolving to the toggle-theme command.
#[test]
fn clicking_the_theme_toggle_actuates_and_resolves_to_the_toggle_theme_command() {
    let commands = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&commands, &view);
    let tree = build_root(&m);

    let center = (16.0 + 36.0 / 2.0, 16.0 + 36.0 / 2.0);
    let mut rt = UiRuntime::new(tree, (1280.0, 800.0), false);
    let down = rt.dispatch_pointer(PointerPhase::Down, center);
    assert!(down.consumed, "the toggle body owns the press");
    assert!(down.actions.is_empty(), "no actuation on down");
    let up = rt.dispatch_pointer(PointerPhase::Up, center);
    assert_eq!(
        up.actions,
        vec![Action::Pressed("cmd:toggle-theme".to_string())],
        "a click on the toggle body actuates a Pressed action (the dead-click defect)"
    );
    // That fired action is what the shell resolves + runs to flip the theme.
    assert_eq!(
        resolve(&up.actions[0], &m),
        Some(Intent::Command("toggle-theme".to_string())),
        "the fired press resolves to the catalog toggle-theme command"
    );
}

/// The canvas switcher's New/Delete controls are ICON chrome buttons: each keeps its
/// hit id (`canvas-new` / `canvas-delete:<active>`) AND draws a registry glyph
/// (`<id>::icon`) instead of a `+`/`🗑` text label; the press still resolves to its
/// intent. FAILS if a control regressed to a text label or dropped its hit id.
#[test]
fn canvas_switcher_controls_are_icon_buttons_keeping_their_ids() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let canvases = vec![
        crate::CanvasEntry {
            id: "c1".to_string(),
            title: "First".to_string(),
        },
        crate::CanvasEntry {
            id: "c2".to_string(),
            title: "Second".to_string(),
        },
    ];
    let m = parity_model(&catalog, &view, &[], &[], &canvases, &[]);
    let tree = build_root(&m);

    // Both controls keep their hit ids and resolve to their intents.
    let mut owners = Vec::new();
    owner_ids(&tree, &mut owners);
    assert!(
        owners.contains(&"canvas-new".to_string()),
        "New keeps its `canvas-new` body"
    );
    assert!(
        owners.contains(&"canvas-delete:c2".to_string()),
        "Delete keeps its `canvas-delete:<active>` body"
    );
    assert_eq!(
        resolve(&Action::Pressed("canvas-new".to_string()), &m),
        Some(Intent::NewCanvas)
    );
    assert_eq!(
        resolve(&Action::Pressed("canvas-delete:c2".to_string()), &m),
        Some(Intent::DeleteCanvas("c2".to_string()))
    );

    // Each draws a registry glyph keyed off its `<id>::icon`, not a text label.
    let mut icons = Vec::new();
    icon_ids(&tree, &mut icons);
    assert!(
        icons.contains(&"canvas-new::icon".to_string()),
        "New draws the `canvas-new` glyph"
    );
    assert!(
        icons.contains(&"canvas-delete:c2::icon".to_string()),
        "Delete draws the `canvas-delete` glyph"
    );
    let scene = shape_ui_core::render(&tree, (1280.0, 800.0), false);
    for label in ["canvas-new::label", "canvas-delete:c2::label"] {
        assert!(
            !scene.objects.iter().any(|o| o.id == label),
            "{label} must not render (icon, not label)"
        );
    }
}

/// The idle brand watermark reads as a muted caption (`text-secondary`), not the
/// primary `text`. FAILS if the watermark regressed to the primary token.
#[test]
fn watermark_reads_as_a_muted_caption() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let scene = shape_ui_core::render(&build_root(&model(&catalog, &view)), (1280.0, 800.0), false);
    // light `text-secondary` is `#8a8a8e`.
    assert_eq!(
        text_color(&scene, "watermark"),
        "#8a8a8e",
        "the watermark is a muted `text-secondary` caption"
    );
}

/// The borderless watermark must not top-clip: its text box has to be taller than the
/// 13px run so the valign-middle line box (plus the `p` descender) clears the box. The
/// box height rides the text object's `rect_path` (`L {qw} {qh}` corner) and the run
/// size both quantize at 8u/px (the fixed wire scale). FAILS if the box is shrunk back
/// under the run height (the original top-clipping) or the watermark gains a bordered box.
#[test]
fn watermark_box_clears_the_caption_run_height() {
    const QUANT_PER_PX: f64 = 8.0;
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let scene = shape_ui_core::render(&build_root(&model(&catalog, &view)), (1280.0, 800.0), false);
    let mark = scene
        .objects
        .iter()
        .find(|o| o.id == "watermark")
        .expect("watermark");
    // Borderless: a caption is fill-less + stroke-less (no bordered box around it).
    assert!(
        mark.fill.is_none() && mark.stroke.is_none(),
        "the watermark is a borderless caption, not a bordered box"
    );
    // The box's bottom-right `L {qw} {qh}` corner carries the quantized box height.
    let qh: f64 = mark
        .geometry_d
        .split(" L ")
        .nth(2)
        .and_then(|seg| seg.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .expect("watermark box height token");
    let box_h = qh / QUANT_PER_PX;
    let run_px = mark.text.as_ref().expect("caption text").runs[0].size / QUANT_PER_PX;
    assert!(run_px > 0.0, "the caption has a non-zero run size");
    assert!(
        box_h >= run_px + 6.0,
        "the watermark box ({box_h}px) must clear the {run_px}px run + descender headroom (top-clip guard)"
    );
}

// ---- inspector display + layout (defects E, F) ----

/// The first text run shown in a rendered control's text-box (`<id>::value`) — the
/// string the user actually reads in the inspector field.
fn shown_value(scene: &shape_renderer_core::render_object::RenderObjectScene, id: &str) -> String {
    let o = scene
        .objects
        .iter()
        .find(|o| o.id == format!("{id}::value"))
        .unwrap_or_else(|| panic!("no {id}::value"));
    o.text
        .as_ref()
        .unwrap_or_else(|| panic!("{id}::value has no text"))
        .runs[0]
        .text
        .clone()
}

/// (E) A non-integral Width must DISPLAY rounded to <=2 decimals, never the raw
/// full-precision f64. Drives the LIVE path: a real scene whose `scale.x` makes the
/// inspector read width = 310.44776119402985 → `build_root` → render → the rendered
/// `insp:width::value` text run reads "310.45", not "310.44776119402985" (the field
/// overflow). FAILS against the old `format!("{n}")` branch, which emits the raw f64.
#[test]
fn inspector_rounds_a_non_integral_dimension_for_display() {
    let catalog = object_command_catalog();
    // rect("r") is 10px wide locally; scale.x = 31.044776119402986 → width 310.44776…
    let mut obj = rect("r");
    obj.transform = compose_affine((0.0, 0.0), 0.0, (31.044_776_119_402_986, 1.0), 0.0);
    let scene_obj = scene_with(vec![obj]);
    let view = view_of(&scene_obj, "r");
    let scene = shape_ui_core::render(&build_root(&model(&catalog, &view)), (1280.0, 800.0), false);

    let shown = shown_value(&scene, "insp:width");
    assert_eq!(
        shown, "310.45",
        "a non-integral width displays rounded to <=2 decimals"
    );
    assert!(
        !shown.contains("310.447"),
        "the raw full-precision f64 ({shown}) must never reach the field"
    );
    // A whole-number dimension still reads clean (no trailing `.0`/`.00`).
    let mut whole = rect("r");
    whole.transform = compose_affine((0.0, 0.0), 0.0, (3.0, 1.0), 0.0);
    let scene = shape_ui_core::render(
        &build_root(&model(&catalog, &view_of(&scene_with(vec![whole]), "r"))),
        (1280.0, 800.0),
        false,
    );
    assert_eq!(
        shown_value(&scene, "insp:width"),
        "30",
        "a whole width has no trailing dot/zeros"
    );
}

/// (F) The inspector panel never overlaps the bottom toolbar: for a FULL inspector
/// (every section incl. Actions) on a short viewport, the rendered panel body's
/// bottom edge stays at or above the toolbar tray's top edge. Drives the live path:
/// a real flow-container view (the tallest inspector) → `build_root` → render → read
/// the `inspector::bg` body bounds and compare against the tray top. FAILS against
/// the old unbounded `panel_h = cursor_y + …`, which let the panel grow past the tray.
#[test]
fn inspector_panel_bottom_clears_the_bottom_toolbar() {
    let catalog = object_command_catalog();
    // The flow-child view is the tallest inspector (segments + the Fixed companion +
    // Actions). A deliberately short viewport forces the overflow the cap must handle.
    let scene_obj = flow_scene();
    let view = view_of(&scene_obj, "c");
    let viewport = (1280.0, 360.0);
    let m = UiModel {
        viewport,
        ..model(&catalog, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), viewport, false);

    // The panel body (`inspector::bg`) carries the panel rect. Its transform y +
    // quantized box height = the painted bottom edge in screen px.
    let bg = scene
        .objects
        .iter()
        .find(|o| o.id == "inspector::bg")
        .expect("inspector body");
    let top = bg.transform[1][2];
    let qh: f64 = bg
        .geometry_d
        .split(" L ")
        .nth(2)
        .and_then(|seg| seg.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .expect("panel body height token");
    let panel_bottom = top + qh / 8.0;

    // The centered tray sits at `vh - tray_h - BOTTOM_MARGIN`; tray_h = PADDING*2 + BTN
    // = 6*2 + 32 = 44, BOTTOM_MARGIN = 24 (toolbar.rs).
    let tray_top = viewport.1 - 44.0 - 24.0;
    assert!(
        panel_bottom <= tray_top,
        "inspector bottom ({panel_bottom}px) must clear the toolbar tray top ({tray_top}px)"
    );

    // The full inspector is still SHOWN: the last (Actions) section title still
    // renders (the cap bounds the body, it does not drop the section).
    let actions = scene
        .objects
        .iter()
        .find(|o| o.id == "inspector::section::action")
        .expect("the Actions section title still renders");
    assert!(actions.transform[1][2] >= top, "Actions sits below the panel top");
}

/// (F') The FIRST section header is never lifted above the panel's content-top inset:
/// even when the inspector overflows the capped band, its top row sits AT/below
/// `panel_top + PANEL_MARGIN`, below the body's rounded corner. Drives the live path:
/// the tallest inspector on a short viewport (the overflow case) → render → read the
/// first `inspector::section::*` title's screen y against the body top. FAILS against
/// the old `y: PANEL_MARGIN - scroll_y`, which lifted the first row under the corner.
#[test]
fn inspector_first_section_clears_the_panel_top_inset() {
    const PANEL_MARGIN: f64 = 16.0; // SPACE_LG, the inspector's content-top inset.
    let catalog = object_command_catalog();
    let scene_obj = flow_scene();
    let view = view_of(&scene_obj, "c");
    let viewport = (1280.0, 360.0); // short enough to force the overflow.
    let m = UiModel {
        viewport,
        ..model(&catalog, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), viewport, false);

    let top = scene
        .objects
        .iter()
        .find(|o| o.id == "inspector::bg")
        .expect("inspector body")
        .transform[1][2];

    // The first section in the flow-child view is the object Header.
    let first = scene
        .objects
        .iter()
        .find(|o| o.id == "inspector::section::header")
        .expect("the first section title renders");
    let first_top = first.transform[1][2];
    assert!(
        first_top >= top + PANEL_MARGIN,
        "first section top ({first_top}px) must sit at/below the content-top inset ({}px)",
        top + PANEL_MARGIN
    );
}

/// (D) The watermark is a borderless Text mark with NO backing rect: the rendered
/// scene has no `watermark`-prefixed sibling rect (`::bg`/`::box`/`::shadow`), the
/// `watermark` object is fill-less + stroke-less, and its text box clears the run
/// height. Drives the live render path. FAILS if a backing/bordering rect is ever
/// emitted under the mark or the box shrinks under the run.
#[test]
fn watermark_is_a_borderless_text_with_no_backing_rect() {
    const QUANT_PER_PX: f64 = 8.0;
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let scene = shape_ui_core::render(&build_root(&model(&catalog, &view)), (1280.0, 800.0), false);

    // No sibling backing/bordering rect under the mark — only the bare `watermark` text.
    let siblings: Vec<&str> = scene
        .objects
        .iter()
        .filter(|o| o.id.starts_with("watermark") && o.id != "watermark")
        .map(|o| o.id.as_str())
        .collect();
    assert!(
        siblings.is_empty(),
        "the watermark has no backing rect sibling, got {siblings:?}"
    );

    let mark = scene
        .objects
        .iter()
        .find(|o| o.id == "watermark")
        .expect("watermark");
    assert!(
        mark.fill.is_none() && mark.stroke.is_none(),
        "the watermark is a borderless caption, not a bordered box"
    );
    let qh: f64 = mark
        .geometry_d
        .split(" L ")
        .nth(2)
        .and_then(|seg| seg.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .expect("watermark box height token");
    let box_h = qh / QUANT_PER_PX;
    let run_px = mark.text.as_ref().expect("caption text").runs[0].size / QUANT_PER_PX;
    assert!(
        box_h >= run_px,
        "the watermark box ({box_h}px) must be at least the run height ({run_px}px) (top-clip guard)"
    );
}

// ---- wire contract: the two token tables are a byte-identical mirror ----

/// A `Paint::Token { name }` must resolve to the SAME RGBA in the server half
/// (scene-core) and the client half (renderer-core). This cross-references BOTH
/// tables directly — not two independently-pinned literal lists — so a token that
/// drifts in only one table FAILS here even if that table's own pin-test is edited
/// to match the drift. Mirrors ui-core's `text_paint_token_matches_object_theme`.
#[test]
fn token_tables_resolve_byte_identically_across_scene_and_renderer_core() {
    use shape_renderer_core::resolve_token as render_resolve;
    use shape_scene_core::object::catalog::theme::Token;
    use shape_scene_core::object::catalog::theme::{resolve_token as scene_resolve, ALL_TOKENS};

    // Every token in the canonical scene-core table must resolve identically in the
    // renderer-core mirror, both modes — covers the material-language tokens and all
    // pre-existing ones, so neither half can silently diverge.
    for token in ALL_TOKENS {
        let name = Token::name(token);
        for dark in [false, true] {
            assert_eq!(
                scene_resolve(name, dark),
                render_resolve(name, dark),
                "token `{name}` (dark={dark}) must resolve byte-identically across both tables"
            );
        }
    }
}

// ---- borderless + hover (QA #1/#2/#6) ----

/// The resting (un-hovered) `RenderObject` for `id` in a freshly built root scene.
fn resting_object(m: &UiModel, id: &str) -> shape_renderer_core::render_object::RenderObject {
    let scene = shape_ui_core::render(&build_root(m), (1280.0, 800.0), false);
    scene
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("no {id}"))
        .clone()
}

/// Whether a body PAINTS no fill: either it carries no fill at all, or an EXPLICIT
/// fully-transparent fill (the borderless-body encoding — a Some(opacity==0) the
/// object pipeline reads as decorative-empty). The live box was an OPAQUE resolved
/// fill, so a transparent declared fill is "no box" here.
fn paints_no_fill(o: &RenderObject) -> bool {
    o.fill
        .as_ref()
        .map(|f| f.opacity <= f64::EPSILON)
        .unwrap_or(true)
}

/// Whether the body `id` RESOLVES to a painted box the way the LIVE renderer does:
/// `shape_ui_core::render` → `build_scene_geometry` (which runs `resolve_visual` +
/// its structural defaults). Returns `(draws_fill, draws_stroke, draws_shadow)` for
/// the object's own draw. A resting borderless body MUST draw none of the three; the
/// prior tests read `o.fill` off the PRE-resolve scene, so they stayed green while
/// the resolver painted the white-fill/dark-stroke box live.
fn body_resolves_to_paint(m: &UiModel, id: &str) -> (bool, bool, bool) {
    use shape_renderer_core::object_pipeline::build_scene_geometry_themed;
    use shape_renderer_core::object_theme::Theme;

    let scene = shape_ui_core::render(&build_root(m), (1280.0, 800.0), false);
    let geo = build_scene_geometry_themed(&scene, Theme::light());
    let draw = geo
        .draws
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("no draw for {id}"));
    (
        !draw.fill_range.is_empty(),
        !draw.stroke_range.is_empty(),
        !draw.shadow_range.is_empty(),
    )
}

/// EVERY interactive body — toolbar icon button, chrome icon button (theme toggle +
/// canvas New), and a context-menu row — is FULLY TRANSPARENT at rest: it PAINTS no
/// fill (no fill, or an explicit transparent one) AND carries no stroke, so it never
/// draws the boxes-in-boxes wireframe; a fill appears only on hover/active. Inspector
/// control labels stay plain `Text` with no backing rect. The pin is BOTH the
/// declared style here AND — in `toolbar_icon_body_draws_no_resting_box` — the
/// RESOLVED draw geometry, because the live box came from the resolver, not the
/// declared style. FAILS the moment any resting paint returns to a body.
#[test]
fn interactive_bodies_are_fully_transparent_at_rest() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    // A canvas pair so the switcher draws (the New control is always present).
    let canvases = vec![
        crate::CanvasEntry {
            id: "c1".to_string(),
            title: "First".to_string(),
        },
        crate::CanvasEntry {
            id: "c2".to_string(),
            title: "Second".to_string(),
        },
    ];
    let menu = ContextMenuModel {
        x: 100.0,
        y: 100.0,
        title: None,
        items: vec![ctx_item("duplicate", "Duplicate")],
    };
    let m = UiModel {
        context_menu: Some(&menu),
        canvases: &canvases,
        active_canvas_id: "c1",
        ..model(&catalog, &view)
    };

    // Every interactive body that should read as borderless+fill-less at rest. The
    // assertion is BOTH-None per body (the box the prior stroke-only pin missed).
    for (id, what) in [
        ("cmd:hand-pan", "an idle toolbar icon button"),
        ("cmd:toggle-theme", "the theme-toggle chrome button"),
        ("canvas-new", "the canvas New chrome button"),
        ("cmd:duplicate", "a context-menu row"),
    ] {
        let body = resting_object(&m, id);
        assert!(
            paints_no_fill(&body),
            "{what} ({id}) has a resting FILL (the boxes-in-boxes box)"
        );
        assert!(body.stroke.is_none(), "{what} ({id}) has a resting BORDER");
    }

    // Inspector control LABELS are plain Text with no backing rect: a Text object
    // always emits fill==None / stroke==None, and there is no `inspector::row::*`
    // body rect carrying a paint. FAILS if a label ever grows a backing box.
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);
    let label = scene
        .objects
        .iter()
        .find(|o| o.id == "inspector::label::name")
        .expect("the always-present Name control label");
    assert!(label.text.is_some(), "an inspector label is a Text");
    assert!(
        label.fill.is_none() && label.stroke.is_none(),
        "an inspector label has no backing rect paint"
    );
    assert!(
        !scene
            .objects
            .iter()
            .any(|o| o.id.starts_with("inspector::row::")
                && (!paints_no_fill(o) || o.stroke.is_some())),
        "an inspector control row emits no painted body rect (labels are bare Text)"
    );
}

/// Pointing at a borderless toolbar icon button swaps its body fill to the `hover`
/// token — the feedback a fill-less/border-less button relies on. Drives the REAL
/// `UiRuntime::dispatch_pointer(Move)` over the built root at the button's resolved
/// screen box, then renders. FAILS if the projection never reads `state.hovered`
/// (the dead-`hover`-token gap), so a hovered button stays invisible.
#[test]
fn hovering_a_button_swaps_its_body_fill_to_the_hover_token() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    let viewport = (1280.0, 800.0);
    let mut rt = UiRuntime::new(build_root(&m), viewport, false);

    // Resolve the idle screen box of the hand-pan button (NOT the active select-move
    // tool, which carries an `accent-soft` fill), then hover its center.
    let resting = rt.render();
    let body = resting
        .objects
        .iter()
        .find(|o| o.id == "cmd:hand-pan")
        .expect("hand-pan body");
    assert!(
        paints_no_fill(body),
        "the button is fill-less at rest (precondition)"
    );
    let bx = body.transform[0][2] + 16.0;
    let by = body.transform[1][2] + 16.0;

    let moved = rt.dispatch_pointer(PointerPhase::Move, (bx, by));
    assert!(
        moved.dirty,
        "moving onto a fresh button is a visible (hover) change"
    );

    let hovered = rt.render();
    let body = hovered
        .objects
        .iter()
        .find(|o| o.id == "cmd:hand-pan")
        .expect("hand-pan body after hover");
    match &body
        .fill
        .as_ref()
        .expect("a hovered button takes a fill")
        .paint
    {
        RPaint::Token { name } => {
            assert_eq!(name, "hover", "hovered body fills with the `hover` token")
        }
        other => panic!("hovered body fill must be the `hover` token, got {other:?}"),
    }

    // Moving off the button drops the hover fill again (no sticky highlight).
    rt.dispatch_pointer(PointerPhase::Move, (5.0, 5.0));
    let off = rt.render();
    let body = off
        .objects
        .iter()
        .find(|o| o.id == "cmd:hand-pan")
        .expect("body off-hover");
    assert!(
        paints_no_fill(body),
        "moving off the button clears the hover fill"
    );

    // The ACTIVE half of the contract: the armed tool (`select-move`, the model's
    // default `active_tool`) carries the `accent-soft` body fill at rest — through the
    // SAME real render()/projection path, not the resting builder alone. FAILS if the
    // active tint regresses (e.g. back to the old opaque bright-blue fill or to None).
    let active = off
        .objects
        .iter()
        .find(|o| o.id == "cmd:select-move")
        .expect("active tool body");
    match &active
        .fill
        .as_ref()
        .expect("the armed tool keeps its accent-soft fill")
        .paint
    {
        RPaint::Token { name } => {
            assert_eq!(name, "accent-soft", "the armed tool body tints accent-soft")
        }
        other => panic!("active body fill must be the `accent-soft` token, got {other:?}"),
    }
}

/// A context-menu row gets the same `hover` feedback (defect G): pointing at an
/// enabled, fill-less row swaps its body fill to the `hover` token through the REAL
/// `UiRuntime::dispatch_pointer(Move)` + render(). Uses `copy` (a catalog command the
/// toolbar does NOT surface) so the hovered id is unique to the menu — no toolbar
/// `cmd:` body shares it. FAILS if the row stays invisible on hover (the row is a
/// borderless flat row, so hover is the only feedback it has).
#[test]
fn hovering_a_context_menu_row_swaps_its_body_fill_to_the_hover_token() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let menu = ContextMenuModel {
        x: 300.0,
        y: 200.0,
        title: None,
        items: vec![ctx_item("copy", "Copy")],
    };
    let m = UiModel {
        context_menu: Some(&menu),
        ..model(&catalog, &view)
    };
    let viewport = (1280.0, 800.0);
    let mut rt = UiRuntime::new(build_root(&m), viewport, false);

    let resting = rt.render();
    let row = resting
        .objects
        .iter()
        .find(|o| o.id == "cmd:copy")
        .expect("copy row");
    assert!(
        paints_no_fill(row),
        "the menu row is fill-less at rest (precondition)"
    );
    let cx = row.transform[0][2] + 20.0;
    let cy = row.transform[1][2] + 15.0;

    let moved = rt.dispatch_pointer(PointerPhase::Move, (cx, cy));
    assert!(
        moved.dirty,
        "moving onto a fresh menu row is a visible (hover) change"
    );

    let hovered = rt.render();
    let row = hovered
        .objects
        .iter()
        .find(|o| o.id == "cmd:copy")
        .expect("copy row after hover");
    match &row
        .fill
        .as_ref()
        .expect("a hovered menu row takes a fill")
        .paint
    {
        RPaint::Token { name } => assert_eq!(
            name, "hover",
            "hovered menu row fills with the `hover` token"
        ),
        other => panic!("hovered menu row fill must be the `hover` token, got {other:?}"),
    }
}

/// Inspector text inputs read as macOS inset fields: a `surface-muted` body with a
/// low-alpha `hairline` border at rest (NOT the hard `default-stroke` wireframe box).
/// FAILS if an idle input regresses to an opaque per-element border.
#[test]
fn inspector_inputs_are_surface_muted_with_a_hairline_border() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // The Name field (`insp:name`) is the always-present inspector TextInput.
    assert_eq!(
        fill_token(&scene, "insp:name"),
        "surface-muted",
        "an idle inspector input fills `surface-muted`"
    );
    assert_eq!(
        stroke_token(&scene, "insp:name"),
        "hairline",
        "an idle inspector input borders with the low-alpha `hairline`, not `default-stroke`"
    );
}

/// The dark-mode contrast contract: in DARK every load-bearing chrome paint must
/// resolve to an actually-legible RGBA, not a white-in-dark or dark-on-dark. This
/// resolves each token through the real renderer-core table (NOT its name), so a
/// future palette regression that keeps the token name but makes it white-in-dark
/// (the original QA #3 defect) fails HERE:
///   - the inspector input body must be a DARK fill (a light `surface`-white box on
///     a dark panel was the white-on-white defect),
///   - a toolbar icon glyph stroke must be a near-WHITE high-contrast `text` (a near
///     -black `text` on the dark tray was the faint-icons defect),
///   - the active canvas-switcher tab must carry no opaque light fill (it tints with
///     the translucent `accent-soft`, the frosted tray showing through).
#[test]
fn dark_mode_chrome_paints_resolve_to_legible_contrast() {
    use shape_renderer_core::resolve_token;

    // Perceived luminance (Rec. 601) of a token's resolved RGBA in the given theme.
    let luma = |token: &str, dark: bool| -> f64 {
        let [r, g, b, _] = resolve_token(token, dark).unwrap_or_else(|| panic!("token {token}"));
        0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
    };

    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let canvases = vec![
        crate::CanvasEntry {
            id: "c1".to_string(),
            title: "First".to_string(),
        },
        crate::CanvasEntry {
            id: "c2".to_string(),
            title: "Second".to_string(),
        },
    ];
    let m = parity_model(&catalog, &view, &[], &[], &canvases, &[]);
    // Render in DARK — the failing theme in the QA report.
    let dark = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), true);

    // The inspector input fill resolves DARK in dark (well below mid-grey 128), so a
    // dark `text` value reads on it. A regression to a light/white surface fails here.
    let input_fill = fill_token(&dark, "insp:name");
    assert!(
        luma(&input_fill, true) < 96.0,
        "inspector input `{input_fill}` must be a DARK fill in dark mode (luma {} >= 96)",
        luma(&input_fill, true)
    );

    // An INACTIVE toolbar glyph (undo is never armed) strokes with the `text` token,
    // which resolves NEAR-WHITE in dark so it reads on the dark tray. A regression to a
    // faint/near-black idle-icon stroke (the QA #3c defect) fails here. (An ACTIVE tool
    // glyph tints `selection-ring` instead, so this pins the always-idle one.)
    let icon_stroke = stroke_token(&dark, "cmd:undo::icon");
    assert_eq!(
        icon_stroke, "text",
        "an idle toolbar glyph strokes with the recolorable `text` token"
    );
    assert!(
        luma(&icon_stroke, true) > 200.0,
        "toolbar icon stroke `{icon_stroke}` must be HIGH-contrast in dark (luma {} <= 200)",
        luma(&icon_stroke, true)
    );

    // The active canvas tab fills the OPAQUE, theme-aware `surface-muted` token (marked
    // by a `selection-ring` stroke), so it flips to a legible DARK pill in dark mode.
    // The old translucent `accent-soft` wash read as a pale near-white pill over the
    // dark tray (the QA dark-contrast defect); pin the bound token NAME and assert its
    // resolved RGBA is actually dark, so a regression to a white-in-dark fill fails here.
    let tab_fill = fill_token(&dark, "canvas:c2");
    assert_eq!(
        tab_fill, "surface-muted",
        "the active tab fills the opaque theme-aware `surface-muted`, not a translucent wash"
    );
    assert!(
        luma(&tab_fill, true) < 96.0,
        "the active tab fill `{tab_fill}` must resolve DARK in dark mode (luma {} >= 96), not a white-in-dark pill",
        luma(&tab_fill, true)
    );
    assert_eq!(
        stroke_token(&dark, "canvas:c2"),
        "selection-ring",
        "the active tab is marked by a `selection-ring` stroke"
    );

    // The inspector "Straighten" action button fills the OPAQUE `surface-muted` token,
    // so it flips to a legible DARK pill in dark (the old fill-less button showed the
    // light panel `material` through and read near-white — the QA dark-contrast defect).
    let action_fill = fill_token(&dark, "insp:canonicalize");
    assert_eq!(
        action_fill, "surface-muted",
        "the action button fills the opaque theme-aware `surface-muted`"
    );
    assert!(
        luma(&action_fill, true) < 96.0,
        "the action button fill `{action_fill}` must resolve DARK in dark mode (luma {} >= 96)",
        luma(&action_fill, true)
    );

    // The repaired control itself: the theme-toggle glyph strokes the `text` token, so
    // once dark is reachable (defect B fixed) the sun/moon mark reads near-white on the
    // dark chrome. Pins the bound token NAME and its resolved legibility together.
    let toggle_stroke = stroke_token(&dark, "cmd:toggle-theme::icon");
    assert_eq!(
        toggle_stroke, "text",
        "the theme-toggle glyph strokes the recolorable `text` token"
    );
    assert!(
        luma(&toggle_stroke, true) > 200.0,
        "the theme-toggle glyph `{toggle_stroke}` must read HIGH-contrast in dark (luma {})",
        luma(&toggle_stroke, true)
    );

    // An inspector SECTION TITLE uses the `text` token (the primary-text contract), so
    // it inverts to near-white in dark — no dark-on-dark heading. Pins the bound token
    // name + its dark legibility on the first section title the view emits.
    let title = dark
        .objects
        .iter()
        .find(|o| o.id.starts_with("inspector::section::"))
        .expect("the inspector emits at least one section title");
    let title_color = title.text.as_ref().expect("section title has text").runs[0]
        .color
        .clone();
    assert_eq!(
        title_color.to_lowercase(),
        format!(
            "#{:02x}{:02x}{:02x}",
            resolve_token("text", true).unwrap()[0],
            resolve_token("text", true).unwrap()[1],
            resolve_token("text", true).unwrap()[2]
        ),
        "a section title is painted the dark `text` hex (primary text, not dark-on-dark)"
    );
    let [tr, tg, tb, _] = resolve_token("text", true).unwrap();
    assert!(
        0.299 * f64::from(tr) + 0.587 * f64::from(tg) + 0.114 * f64::from(tb) > 200.0,
        "the section-title `text` color reads HIGH-contrast in dark"
    );
}

// ---- toolbar borderless + polish (QA C/D) ----

/// (C) A toolbar icon button BODY is truly borderless at rest — NO resting fill AND
/// NO resting stroke — so the tray shows through (no boxes-in-boxes outline), while an
/// ARMED tool body still carries its `accent-soft` background. Drives the real
/// `build_root`→`render` seam over an idle (`hand-pan`) and an armed (`select-move`,
/// the default `active_tool`) tray button. FAILS the moment a resting fill or stroke
/// returns to an idle toolbar body (the always-on box the QA saw), or the armed body
/// loses its active bg.
#[test]
fn toolbar_icon_body_is_borderless_at_rest_and_active_keeps_its_bg() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view); // default active_tool == "select-move"
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // An idle tray button (`hand-pan` is never the default tool) paints NOTHING at
    // rest: no fill box, no stroke outline — the frosted tray is the only surface.
    let idle = scene
        .objects
        .iter()
        .find(|o| o.id == "cmd:hand-pan")
        .expect("hand-pan body");
    assert!(
        paints_no_fill(idle),
        "an idle toolbar icon body has a resting FILL (the always-on box)"
    );
    assert!(
        idle.stroke.is_none(),
        "an idle toolbar icon body has a resting STROKE (the always-on outline)"
    );

    // The armed tool keeps its active background (`accent-soft`), so the active half of
    // the contract — a bg only on active/hover — still holds.
    let armed = scene
        .objects
        .iter()
        .find(|o| o.id == "cmd:select-move")
        .expect("armed tool body");
    match &armed
        .fill
        .as_ref()
        .expect("the armed tool body keeps an active bg")
        .paint
    {
        RPaint::Token { name } => {
            assert_eq!(name, "accent-soft", "the armed tool body tints accent-soft")
        }
        other => panic!("active toolbar body fill must be the accent-soft token, got {other:?}"),
    }
    assert!(
        armed.stroke.is_none(),
        "even the armed toolbar body draws no resting outline"
    );
}

/// (A) THE live-representative pin for "kill the toolbar icon box": it drives the real
/// `build_root`→`render`→`build_scene_geometry` path (the SAME pipeline `ObjectRenderer::
/// new` feeds live, running `resolve_visual` + its structural defaults) and asserts that
/// an idle (non-active, non-hovered) toolbar icon body RESOLVES to NO drawn geometry —
/// no fill mesh, no stroke ribbon, no shadow. The prior borderless tests read `o.fill`
/// off the PRE-resolve scene, so they stayed green while `resolve_visual` injected the
/// white `default_fill` + `#283644` `default_stroke` — the box the QA saw twice. This
/// asserts at the RESOLVED layer, so it FAILS on that box and only passes when the body
/// is truly decorative-empty. The tray bg still draws a fill (the assertion is not
/// vacuous), and an ARMED body draws its `accent-soft` fill (the active half still
/// paints).
#[test]
fn toolbar_icon_body_draws_no_resting_box() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view); // default active_tool == "select-move"

    // An idle tray button resolves to nothing: the white default_fill, the dark
    // default_stroke ribbon, and the shadow that fill would cast must ALL be absent.
    let (idle_fill, idle_stroke, idle_shadow) = body_resolves_to_paint(&m, "cmd:hand-pan");
    assert!(
        !idle_fill,
        "an idle toolbar icon body resolves to a FILL box (the default white box live)"
    );
    assert!(
        !idle_stroke,
        "an idle toolbar icon body resolves to a STROKE ribbon (the default dark border live)"
    );
    assert!(
        !idle_shadow,
        "an idle toolbar icon body resolves to a drop SHADOW (the box's elevation)"
    );

    // Guard against a vacuous pass: the frosted tray DOES resolve an opaque fill, so the
    // scene reaches the resolver and the idle assertion is meaningful.
    let (tray_fill, _, _) = body_resolves_to_paint(&m, "toolbar::tray");
    assert!(
        tray_fill,
        "the tray bg must still resolve a fill (else the idle pin is vacuous)"
    );

    // The ARMED tool body DOES resolve its `accent-soft` fill — a bg on active is kept.
    let (armed_fill, _, _) = body_resolves_to_paint(&m, "cmd:select-move");
    assert!(
        armed_fill,
        "the armed tool body must resolve its active accent-soft fill"
    );
}

/// The number of DISTINCT text-baseline rows the object `id`'s glyph quads occupy in
/// the REAL build (`render`→`build_scene_geometry_themed`). A single-line label's glyph
/// tops all share one y; a char-wrapped label ("Cente/r") spans two rows a full line
/// height apart — so this returns 2 exactly when the live wrap defect is present.
fn label_line_rows(m: &UiModel, id: &str) -> usize {
    use shape_renderer_core::object_pipeline::build_scene_geometry_themed;
    use shape_renderer_core::object_theme::Theme;

    let scene = shape_ui_core::render(&build_root(m), (1280.0, 800.0), false);
    let geo = build_scene_geometry_themed(&scene, Theme::light());
    let draw = geo
        .draws
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("no draw for {id}"));
    let start = usize::try_from(draw.text_range.start).unwrap();
    let end = usize::try_from(draw.text_range.end).unwrap();
    // Each glyph emits 6 vertices (tl, tr, br, tl, br, bl); the TOP-LEFT (the glyph
    // top y0) is vertex 0 of every group. Read only those tops so a single-line label's
    // glyphs collapse to one row (a glyph's own top↔bottom span would otherwise read as
    // two rows). A char-wrapped label's two lines differ by a full line height.
    let mut tops: Vec<f32> = geo.text_vertices[start..end]
        .chunks_exact(6)
        .map(|glyph| glyph[0].position[1])
        .collect();
    tops.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut rows = 0usize;
    let mut last = f32::NEG_INFINITY;
    for y in tops {
        if y - last > 6.0 {
            rows += 1;
            last = y;
        }
    }
    rows
}

/// (B) A segmented control's cells are wide enough that NO label wraps, and its track
/// spans the full panel content width (the inactive cells' shared background). This
/// drives the real `render`→`build_scene_geometry_themed` path and pins the EMITTED
/// glyph geometry: the worst-case "Horizontal" axis label (10 glyphs) must lay out on
/// ONE line. At the old narrow 132px right-column width a 2-cell segment gave 66px
/// cells and "Horizontal" char-wrapped to two rows; the full-width stacked segment
/// gives 116px cells, so it fits. FAILS the moment a segment label wraps again.
#[test]
fn segment_labels_fit_their_cells_without_wrapping() {
    let scene = flow_scene(); // "p" is a flow container → the Axis segment is present
    let view = view_of(&scene, "p");
    let catalog = object_command_catalog();
    let m = model(&catalog, &view);

    // The "Horizontal" label (cell 0 of the `insp:axis` segment) lays out on ONE line.
    let rows = label_line_rows(&m, "insp:axis::seg0::label");
    assert_eq!(
        rows, 1,
        "the 'Horizontal' segment label wrapped to {rows} lines — the cell is too narrow"
    );

    let rendered = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);
    // The cell pitch (seg1 origin − seg0 origin) is the per-cell width; with the
    // full-width stacked segment it is ~116px (232/2), not the old 66px. A wide cell
    // is what keeps the 10-glyph label on one line.
    let seg0_x = rendered
        .objects
        .iter()
        .find(|o| o.id == "insp:axis::seg0::label")
        .unwrap()
        .transform[0][2];
    let seg1_x = rendered
        .objects
        .iter()
        .find(|o| o.id == "insp:axis::seg1::label")
        .unwrap()
        .transform[0][2];
    assert!(
        seg1_x - seg0_x > 100.0,
        "the segment cell pitch is only {}px — too narrow for the label",
        seg1_x - seg0_x
    );

    // The track resolves a painted fill (the inactive cells' shared background): an
    // inactive cell reads against this `surface-muted` trough.
    let (track_fill, _, _) = body_resolves_to_paint(&m, "insp:axis");
    assert!(
        track_fill,
        "the segment track must resolve a fill (the inactive cells' background)"
    );
    assert_eq!(
        fill_token(&rendered, "insp:axis"),
        "surface-muted",
        "the track fills `surface-muted`"
    );
}

/// (D) A toolbar/chrome icon glyph strokes at the CRISP weight (1.8px), heavier than a
/// hairline, so a `text`-tinted line-art glyph reads sharp on the frosted tray instead
/// of a faint thread — the QA "faint medium-gray, not crisp" reading in dark. The
/// literal 1.8 pins the real pixel weight (asserting the private `STROKE_W` const would
/// stay green if it were thinned). Checked in BOTH themes (the stroke width is
/// theme-independent, but the icon must read in dark, the failing theme). FAILS if the
/// glyph weight regresses to the old thin 1.6.
#[test]
fn toolbar_icon_glyph_strokes_at_the_crisp_weight() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = model(&catalog, &view);
    for theme_dark in [false, true] {
        let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), theme_dark);
        let glyph = scene
            .objects
            .iter()
            .find(|o| o.id == "cmd:undo::icon")
            .expect("an idle toolbar glyph");
        let stroke = glyph.stroke.as_ref().expect("the glyph is stroked");
        assert_eq!(
            stroke.width, 1.8,
            "toolbar icon glyphs stroke at the crisp 1.8px weight (dark={theme_dark}), not the faint 1.6"
        );
    }
}

/// (D) The inspector numeric display uses a PERIOD fraction separator, never a comma:
/// a non-integral dimension renders "310.45" (a `.`), and the shown string contains no
/// `,`. Drives the live `build_root`→render path over a real scaled scene (the same
/// `scale.x` the rounding test uses). FAILS if the field ever shows the "226,46"-style
/// comma the QA reported.
#[test]
fn inspector_numeric_display_uses_a_period_not_a_comma() {
    let catalog = object_command_catalog();
    let mut obj = rect("r");
    // scale.x = 31.044776… → width 310.44776… → displays "310.45".
    obj.transform = compose_affine((0.0, 0.0), 0.0, (31.044_776_119_402_986, 1.0), 0.0);
    let scene_obj = scene_with(vec![obj]);
    let view = view_of(&scene_obj, "r");
    let scene = shape_ui_core::render(&build_root(&model(&catalog, &view)), (1280.0, 800.0), false);

    let shown = shown_value(&scene, "insp:width");
    assert!(
        shown.contains('.'),
        "a fractional dimension shows a PERIOD separator, got {shown:?}"
    );
    assert!(
        !shown.contains(','),
        "the inspector numeric display must never use a COMMA, got {shown:?}"
    );
    assert_eq!(shown, "310.45", "the fractional width reads with a period");
}

// ---- flex layout-system conversion (centralized scale + real flex containers) ----

/// The screen-space top-left a render object is translated to (its `transform`
/// column), the same origin `hit`/`layout_children` place it at.
fn render_origin(
    scene: &shape_renderer_core::render_object::RenderObjectScene,
    id: &str,
) -> (f64, f64) {
    let o = scene
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("no render object {id}"));
    (o.transform[0][2], o.transform[1][2])
}

/// FALSIFIABLE: after the Axis::None→flex conversion the settings modal's command
/// rows are laid out by the engine with a UNIFORM gap — consecutive rows in a
/// section differ in y by exactly `row_h + SPACE_XS`, the section's flex spacing.
/// FAILS if a row reverts to the old hand-rolled `y += ROW_H` cursor (whose drift
/// this conversion removes) or the spacing token changes out from under the layout.
#[test]
fn settings_section_rows_have_a_uniform_engine_gap() {
    use shape_ui_core::SPACE_XS;
    let catalog = object_command_catalog();
    let gestures = object_gesture_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = UiModel {
        settings_open: true,
        gesture_catalog: &gestures,
        ..model(&catalog, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // Three consecutive command rows in the first category (Tool group has ≥3).
    let mut tool_rows: Vec<f64> = catalog
        .iter()
        .filter(|c| {
            c.category == shape_scene_core::object::catalog::commands::ObjectCommandCategory::Tool
        })
        .map(|c| render_origin(&scene, &format!("settings::cmd::{}::label", c.id)).1)
        .collect();
    assert!(tool_rows.len() >= 2, "need ≥2 Tool rows to measure a gap");
    tool_rows.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // The kbd-pill body is `ROW_H - SPACE_XS` tall, so the row height the flex packs is
    // the label height ROW_H(24); the gap between row labels is that height + SPACE_XS.
    let row_h = 24.0;
    let gap = tool_rows[1] - tool_rows[0];
    assert!(
        (gap - (row_h + SPACE_XS)).abs() < 1e-6,
        "command rows must be a uniform engine gap apart, got {gap} (want {})",
        row_h + SPACE_XS
    );
    // EVERY consecutive pair shares that one gap (no off-by-cursor row).
    for pair in tool_rows.windows(2) {
        assert!(
            (pair[1] - pair[0] - gap).abs() < 1e-6,
            "non-uniform gap: {pair:?}"
        );
    }
}

/// FALSIFIABLE: a settings command row is a HORIZONTAL SpaceBetween flex — the label
/// pins to the leading edge and the kbd pill to the trailing edge, both cross-centered
/// on the row. FAILS if the label|value row stops being cross-centered (the y-origins
/// diverge) or the pill stops pinning right (it would sit mid-row under packed Start).
#[test]
fn settings_command_row_is_space_between_and_cross_centered() {
    let catalog = object_command_catalog();
    let view = view_of(&scene_with(vec![rect("r")]), "r");
    let m = UiModel {
        settings_open: true,
        ..model(&catalog, &view)
    };
    let scene = shape_ui_core::render(&build_root(&m), (1280.0, 800.0), false);

    // `undo` has a default shortcut, so its row has both a label and a kbd glyph.
    let (label_x, label_y) = render_origin(&scene, "settings::cmd::undo::label");
    let (kbd_x, _kbd_y) = render_origin(&scene, "settings::cmd::undo::kbd");

    // The kbd pill is pinned to the trailing edge: its x is well to the RIGHT of the
    // label's x (SpaceBetween split the slack, not a packed `spacing`).
    assert!(
        kbd_x > label_x + 100.0,
        "kbd pill must pin right of the label (x {kbd_x} vs {label_x})"
    );

    // Cross-centered: the kbd pill body (ROW_H - SPACE_XS tall) is vertically centered
    // inside the ROW_H row, so its top sits BELOW the (full-height) label's top.
    let (_, kbd_bg_y) = render_origin(&scene, "settings::cmd::undo::kbd-bg");
    assert!(
        kbd_bg_y > label_y,
        "the shorter kbd pill must be cross-centered, not top-aligned"
    );
}

/// FALSIFIABLE (the sync invariant): after the inspector flex conversion, `hit` and
/// the RENDERED box still agree for a nested control. We render the converted tree,
/// read where `insp:name` actually paints (a control nested inside a flex row inside a
/// flex section inside a flex content stack), and hit that exact point — both paths
/// run through `layout_children`, so a drift between drawn and hit boxes fails HERE.
#[test]
fn inspector_flex_hit_agrees_with_rendered_box() {
    let scene_obj = flow_scene();
    let view = view_of(&scene_obj, "c");
    let catalog = object_command_catalog();
    let tree = build_root(&model(&catalog, &view));
    let scene = shape_ui_core::render(&tree, (1280.0, 800.0), false);

    // The Name field's body is the `insp:name` text-input owner (a deeply-nested flex
    // child — the strongest sync probe in the converted tree).
    let (sx, sy) = render_origin(&scene, "insp:name");
    // Hit just inside the rendered top-left corner: drawn == hit means this lands on
    // the owner, never a neighbor or a ::part.
    assert_eq!(
        shape_ui_core::hit(&tree, (sx + 2.0, sy + 2.0)),
        Some("insp:name".to_string()),
        "hit must resolve to the same owner the renderer drew at this point"
    );
}

/// FALSIFIABLE: the relayout is GEOMETRY ONLY — every load-bearing id/prefix the
/// intent resolver keys on survives the flex conversion. FAILS if a converted surface
/// dropped or renamed an `insp:`/`cmd:`/`settings::` id (which would silently break
/// the press round-trip the resolver depends on).
#[test]
fn flex_conversion_preserves_load_bearing_ids() {
    let scene_obj = flow_scene();
    let view = view_of(&scene_obj, "c");
    let catalog = object_command_catalog();
    let gestures = object_gesture_catalog();
    let m = UiModel {
        settings_open: true,
        gesture_catalog: &gestures,
        ..model(&catalog, &view)
    };
    let tree = build_root(&m);
    let mut ids = Vec::new();
    all_ids(&tree, &mut ids);

    // The inspector name control still carries its `insp:` owner id.
    assert!(
        ids.iter().any(|id| id == "insp:name"),
        "inspector lost insp:name"
    );
    // The toolbar command still carries its `cmd:` id.
    assert!(
        ids.iter().any(|id| id == "cmd:undo"),
        "toolbar lost cmd:undo"
    );
    // The settings kbd glyph id is intact (the row converted to flex kept its parts).
    assert!(
        ids.iter().any(|id| id == "settings::cmd::undo::kbd"),
        "settings row lost its kbd id"
    );
}
