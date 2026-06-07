//! OB3.S5 — templates are recipes of inline-styled [`Object`]s.
//!
//! A template is a *recipe*: a pure function that, given an anchor point and a
//! pair of id/order allocators, returns a `Vec<Object>` of fully-formed canvas
//! objects (rects, text labels, connectors). There is no legacy
//! group/card/edge recipe and no `styleKey`/palette indirection (OB3.R10):
//! every produced object carries its colors **inline** as [`Fill`]/[`Stroke`]
//! and its label as [`Text`]. The semantic palette (the 12 presets that used to
//! live behind `defaultStyles`/`shapeStyleToken` in `renderScene.ts`) is ported
//! here as [`semantic_preset_style`], read directly into inline object styles.
//!
//! How the pieces fit (OB1.2/OB3.S7):
//! * The UI reads [`object_template_catalog`] for the picker (id/label/category).
//! * Picking one sends a `FeatureRequest::TemplateApply { recipe, anchor_x,
//!   anchor_y, .. }` whose `recipe` is the output of [`build_template`].
//! * The server/client lowers that recipe to ops via [`template_to_ops`], which
//!   wraps each object in an `ObjectOp::InsertObject` and applies them through
//!   the one op-apply path (P1).
//!
//! Purity (CLAUDE.md): no time/rng/IO. Object ids and fractional `order` keys
//! are produced by injected allocators (`id_alloc`/`order_alloc`), so the same
//! call with the same allocators yields byte-identical objects.
//!
//! Geometry convention: each object's geometry is **object-local** quantized i32
//! at [`GEOMETRY_QUANTUM_PER_PX`] (8 units/px), authored from (0,0). The object's
//! world placement is its `transform` (`anchor + local px offset`), so no
//! `f64 -> i32` narrowing of coordinates ever happens — local box dimensions are
//! integer pixel constants quantized exactly, and the only `f64` values live in
//! the (`f64`) `Transform3x3`.

use super::model::{
    Anchor, Fill, FillRule, Geometry, LineCap, LineJoin, LocalPoint, Object, Paint, PathNode,
    Stroke, SubPath, Text, TextAlign, TextRun, TextVAlign, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use super::op::ObjectOp;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Catalog metadata (the UI picker reads this).
// ---------------------------------------------------------------------------

/// Coarse grouping for the template picker (mirrors the legacy
/// `TemplateCategory`, minus the legacy `Presentation` recipe shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateCategory {
    Planning,
    Knowledge,
    Engineering,
    Presentation,
    General,
}

/// One catalog entry: enough for the picker to list and trigger a template,
/// without building any objects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectTemplateMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub category: TemplateCategory,
    pub description: &'static str,
}

/// A registered template: metadata plus the builder that produces its objects.
/// `build` has the same signature as [`build_template`] minus the id (it is
/// already bound to this entry).
pub struct ObjectTemplate {
    pub id: &'static str,
    pub label: &'static str,
    pub category: TemplateCategory,
    pub description: &'static str,
    /// The recipe builder. `anchor_*` is the world placement of the recipe's
    /// origin; `id_alloc`/`order_alloc` supply unique object ids and z-order
    /// keys so the function stays pure (no rng).
    pub build: fn(
        anchor_x: f64,
        anchor_y: f64,
        id_alloc: &mut dyn FnMut() -> String,
        order_alloc: &mut dyn FnMut() -> String,
    ) -> Vec<Object>,
}

/// The full template registry. Order is the picker display order.
fn registry() -> Vec<ObjectTemplate> {
    vec![
        ObjectTemplate {
            id: "decision_map",
            label: "Decision Map",
            category: TemplateCategory::Engineering,
            description:
                "One decision point, competing options, and a sub-decision, wired with connectors.",
            build: build_decision_map,
        },
        ObjectTemplate {
            id: "todo_board",
            label: "To-do Board",
            category: TemplateCategory::Planning,
            description: "Three columns — To do, In progress, Done — with starter task cards.",
            build: build_todo_board,
        },
        ObjectTemplate {
            id: "idea_board",
            label: "Idea Board",
            category: TemplateCategory::Knowledge,
            description: "A scatter of idea cards around a central prompt.",
            build: build_idea_board,
        },
        ObjectTemplate {
            id: "wiki_note",
            label: "Wiki Note",
            category: TemplateCategory::Knowledge,
            description: "A title card over a body note — a lightweight document page.",
            build: build_wiki_note,
        },
        ObjectTemplate {
            id: "adr",
            label: "ADR",
            category: TemplateCategory::Engineering,
            description: "Architecture decision record — context, decision, consequences.",
            build: build_adr_stub,
        },
        ObjectTemplate {
            id: "investigation_map",
            label: "Investigation Map",
            category: TemplateCategory::Engineering,
            description: "Trace evidence and findings from a starting question.",
            build: build_investigation_map_stub,
        },
        ObjectTemplate {
            id: "presentation",
            label: "Presentation",
            category: TemplateCategory::Presentation,
            description: "A slide outline as a sequence of frames.",
            build: build_presentation_stub,
        },
        ObjectTemplate {
            id: "dependency_diagram",
            label: "Dependency Diagram",
            category: TemplateCategory::Engineering,
            description: "Components and the dependencies between them.",
            build: build_dependency_diagram_stub,
        },
        ObjectTemplate {
            id: "server_architecture",
            label: "Server Architecture",
            category: TemplateCategory::Engineering,
            description: "Components as boxes, zones as frames, calls as labeled edges.",
            build: build_server_architecture_stub,
        },
    ]
}

/// Metadata list for the UI picker. Never empty.
pub fn object_template_catalog() -> Vec<ObjectTemplateMeta> {
    registry()
        .into_iter()
        .map(|t| ObjectTemplateMeta {
            id: t.id,
            label: t.label,
            category: t.category,
            description: t.description,
        })
        .collect()
}

/// Build a template's objects by id. Unknown ids produce an empty `Vec`
/// (the caller treats empty as "no such template"). `anchor_*` is where the
/// recipe origin lands in world space; the allocators inject ids/orders.
pub fn build_template(
    template_id: &str,
    anchor_x: f64,
    anchor_y: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    match registry().into_iter().find(|t| t.id == template_id) {
        Some(t) => (t.build)(anchor_x, anchor_y, id_alloc, order_alloc),
        None => Vec::new(),
    }
}

/// Lower a built recipe into ops: one [`ObjectOp::InsertObject`] per object, in
/// recipe order (shapes before the connectors that anchor to them). The
/// server/client applies these through the single op-apply path when handling a
/// `FeatureRequest::TemplateApply` (OB1.2/OB3.S7).
pub fn template_to_ops(objs: Vec<Object>) -> Vec<ObjectOp> {
    objs.into_iter()
        .map(|object| ObjectOp::InsertObject { object })
        .collect()
}

// ---------------------------------------------------------------------------
// Semantic presets (OB3.R10) — the 12 inline styles ported from renderScene.ts.
// ---------------------------------------------------------------------------

/// The 12 semantic preset keys, ported from `renderScene.ts` `defaultStyles`.
/// These are not stored on objects (palette/styleKey is gone, OB3.R10); they are
/// only a source of *inline* colors that [`build_template`] bakes into objects.
pub fn semantic_presets() -> Vec<&'static str> {
    vec![
        "default",
        "decision",
        "risk",
        "proposition",
        "decision_point",
        "option",
        "evidence",
        "tradeoff",
        "blocker",
        "subdecision",
        "task",
        "artifact",
    ]
}

/// Inline style for a semantic preset: `(fill, stroke, text-color)`, ported
/// 1:1 from `renderScene.ts` `shapeStyleToken(id, fill, stroke, text, ..)`.
/// `fill` is the surface color, `stroke` the border accent, and the third
/// element the text color string (used to color a [`TextRun`]). Stroke width is
/// 1px expressed in quantized units. An unknown preset falls back to `default`.
pub fn semantic_preset_style(preset: &str) -> (Option<Fill>, Option<Stroke>, Option<String>) {
    // (fill, stroke, text) hex triples copied from renderScene.ts defaultStyles.
    let (fill_hex, stroke_hex, text_hex) = match preset {
        "decision" | "decision_point" => ("#f7fbff", "#2f7ee6", "#102033"),
        "risk" => ("#fff8f1", "#c67914", "#2a1b0b"),
        "proposition" => ("#f4fbf9", "#19917f", "#10231f"),
        "option" => ("#f4fbf6", "#26965e", "#10251a"),
        "evidence" => ("#f4fbff", "#228bb8", "#102432"),
        "tradeoff" => ("#fff8f1", "#c17518", "#2a1b0b"),
        "blocker" => ("#fff7f8", "#d14c58", "#2c1014"),
        "subdecision" => ("#f8f7ff", "#7a68ce", "#1d1833"),
        "task" => ("#f7faff", "#5371b3", "#111c33"),
        "artifact" => ("#f7fafb", "#617a85", "#142027"),
        // "default" and anything unrecognized.
        _ => ("#ffffff", "#7b8794", "#172026"),
    };
    let fill = Fill { paint: Paint::Solid { color: fill_hex.to_string() }, opacity: 1.0 };
    let stroke = Stroke {
        paint: Paint::Solid { color: stroke_hex.to_string() },
        width: px(1),
        opacity: 1.0,
        dash: Vec::new(),
        cap: LineCap::Butt,
        join: LineJoin::Round,
    };
    (Some(fill), Some(stroke), Some(text_hex.to_string()))
}

// ---------------------------------------------------------------------------
// Builder helpers (pure, no rng/IO).
// ---------------------------------------------------------------------------

/// Logical px -> object-local quantized units (8/px). Used only on integer
/// pixel constants authored in this module, so the multiply is exact.
const fn px(p: i32) -> i32 {
    p * GEOMETRY_QUANTUM_PER_PX
}

/// A closed rectangle from (0,0) to (w_px, h_px) in object-local quantized units.
fn rect_geometry(w_px: i32, h_px: i32) -> Geometry {
    let w = px(w_px);
    let h = px(h_px);
    Geometry::from_subpaths(
        vec![SubPath {
            closed: true,
            nodes: vec![
                PathNode::corner(0, 0),
                PathNode::corner(w, 0),
                PathNode::corner(w, h),
                PathNode::corner(0, h),
            ],
        }],
        FillRule::EvenOdd,
    )
}

/// An open 2-node segment in object-local quantized units (the connector body,
/// D5). The world endpoints are resolved from the anchors, not this geometry; a
/// short stub keeps the object non-degenerate before resolution.
fn segment_geometry(dx_px: i32, dy_px: i32) -> Geometry {
    Geometry::from_subpaths(
        vec![SubPath {
            closed: false,
            nodes: vec![PathNode::corner(0, 0), PathNode::corner(px(dx_px), px(dy_px))],
        }],
        FillRule::NonZero,
    )
}

/// A single centered text run with an inline color, at the preset size.
fn label(text: &str, color: Option<String>) -> Text {
    Text {
        runs: vec![TextRun {
            text: text.to_string(),
            color,
            size: Some(px(13)),
            bold: false,
            italic: false,
            font: None,
        }],
        align: TextAlign::Center,
        valign: TextVAlign::Middle,
    }
}

/// Build one styled card: a `w x h` rect placed at `anchor + (ox, oy)` px, with
/// the preset's inline fill/stroke and a centered label in the preset text color.
fn card(
    preset: &str,
    text: &str,
    w_px: i32,
    h_px: i32,
    anchor_x: f64,
    anchor_y: f64,
    ox: f64,
    oy: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Object {
    let (fill, stroke, text_color) = semantic_preset_style(preset);
    let mut obj = Object::new(id_alloc(), order_alloc(), rect_geometry(w_px, h_px));
    obj.transform = Transform3x3::translate(anchor_x + ox, anchor_y + oy);
    obj.fill = fill;
    obj.stroke = stroke;
    obj.text = Some(label(text, text_color));
    obj
}

/// Build a connector: an open 2-node object whose endpoints anchor to `from`
/// and `to` (D5). Node 0 attaches to `from`, node 1 to `to`; the `at` local
/// point is the target's center, re-projected when the target's geometry edits.
fn connector(
    from: &Object,
    to: &Object,
    from_center: LocalPoint,
    to_center: LocalPoint,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Object {
    let mut obj = Object::new(id_alloc(), order_alloc(), segment_geometry(40, 0));
    // The connector body lives in world space; placement is identity and the
    // endpoints are governed by the anchors (resolved against target outlines).
    obj.stroke = Some(Stroke {
        paint: Paint::Solid { color: "#7b8794".to_string() },
        width: px(2),
        opacity: 1.0,
        dash: Vec::new(),
        cap: LineCap::Round,
        join: LineJoin::Round,
    });
    obj.anchors = vec![
        Anchor { node_index: 0, target: from.id.clone(), at: from_center },
        Anchor { node_index: 1, target: to.id.clone(), at: to_center },
    ];
    obj
}

/// Center of a `w x h` px card in its own object-local quantized coords.
fn center_of(w_px: i32, h_px: i32) -> LocalPoint {
    LocalPoint { x: px(w_px) / 2, y: px(h_px) / 2 }
}

// ---------------------------------------------------------------------------
// Real templates (fully fleshed).
// ---------------------------------------------------------------------------

/// Decision Map: a decision point, two competing options, and a sub-decision,
/// wired with three connectors. Each card uses its matching semantic preset.
fn build_decision_map(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    const W: i32 = 200;
    const H: i32 = 110;
    let decision = card(
        "decision_point", "Decision point", W, H, ax, ay, 0.0, 160.0, id_alloc, order_alloc,
    );
    let option_a = card("option", "Option A", W, H, ax, ay, 320.0, 40.0, id_alloc, order_alloc);
    let option_b =
        card("option", "Option B", W, H, ax, ay, 320.0, 280.0, id_alloc, order_alloc);
    let subdecision = card(
        "subdecision", "Sub-decision", W, H, ax, ay, 640.0, 40.0, id_alloc, order_alloc,
    );

    let c = center_of(W, H);
    let e1 = connector(&decision, &option_a, c, c, id_alloc, order_alloc);
    let e2 = connector(&decision, &option_b, c, c, id_alloc, order_alloc);
    let e3 = connector(&option_a, &subdecision, c, c, id_alloc, order_alloc);

    vec![decision, option_a, option_b, subdecision, e1, e2, e3]
}

/// To-do Board: three column frames (To do / In progress / Done) plus a starter
/// task card in the first column. Columns use the neutral `default` preset;
/// the task card uses the `task` preset.
fn build_todo_board(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    const COL_W: i32 = 240;
    const COL_H: i32 = 460;
    let todo = card("default", "To do", COL_W, COL_H, ax, ay, 0.0, 0.0, id_alloc, order_alloc);
    let doing = card(
        "default", "In progress", COL_W, COL_H, ax, ay, 280.0, 0.0, id_alloc, order_alloc,
    );
    let done = card("default", "Done", COL_W, COL_H, ax, ay, 560.0, 0.0, id_alloc, order_alloc);
    let first_task =
        card("task", "First task", 200, 70, ax, ay, 20.0, 60.0, id_alloc, order_alloc);
    vec![todo, doing, done, first_task]
}

/// Idea Board: a central prompt card surrounded by four idea cards. The prompt
/// uses `proposition`; the ideas use `evidence`.
fn build_idea_board(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    const W: i32 = 180;
    const H: i32 = 100;
    let prompt =
        card("proposition", "Central idea", W, H, ax, ay, 280.0, 200.0, id_alloc, order_alloc);
    let i1 = card("evidence", "Idea 1", W, H, ax, ay, 0.0, 0.0, id_alloc, order_alloc);
    let i2 = card("evidence", "Idea 2", W, H, ax, ay, 560.0, 0.0, id_alloc, order_alloc);
    let i3 = card("evidence", "Idea 3", W, H, ax, ay, 0.0, 400.0, id_alloc, order_alloc);
    let i4 = card("evidence", "Idea 4", W, H, ax, ay, 560.0, 400.0, id_alloc, order_alloc);
    vec![prompt, i1, i2, i3, i4]
}

/// Wiki Note: a title card stacked over a larger body card — a lightweight
/// document page. Title uses `decision` (the blue accent); body uses `default`.
fn build_wiki_note(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    let title = card("decision", "Title", 420, 70, ax, ay, 0.0, 0.0, id_alloc, order_alloc);
    let body =
        card("default", "Write your note here...", 420, 320, ax, ay, 0.0, 90.0, id_alloc, order_alloc);
    vec![title, body]
}

// ---------------------------------------------------------------------------
// Stub templates — a single titled card each (documented placeholders).
// ---------------------------------------------------------------------------

/// One titled card with the given preset — the shared body for stub templates
/// that are not yet fully fleshed.
fn titled_rect(
    preset: &str,
    title: &str,
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    vec![card(preset, title, 260, 120, ax, ay, 0.0, 0.0, id_alloc, order_alloc)]
}

/// Stub: ADR (architecture decision record). Fleshing out is OB-future.
fn build_adr_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("decision", "ADR", ax, ay, id_alloc, order_alloc)
}

/// Stub: investigation map. Fleshing out is OB-future.
fn build_investigation_map_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("evidence", "Investigation Map", ax, ay, id_alloc, order_alloc)
}

/// Stub: presentation outline. Fleshing out is OB-future.
fn build_presentation_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("default", "Presentation", ax, ay, id_alloc, order_alloc)
}

/// Stub: dependency diagram. Fleshing out is OB-future.
fn build_dependency_diagram_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("artifact", "Dependency Diagram", ax, ay, id_alloc, order_alloc)
}

/// Stub: server architecture. Fleshing out is OB-future.
fn build_server_architecture_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("artifact", "Server Architecture", ax, ay, id_alloc, order_alloc)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Bind two sequential `id-{n}` / `o{n}` allocators into the caller's scope —
    /// deterministic, no rng. A macro (not a function) because `impl FnMut`
    /// cannot be returned nested inside a tuple.
    macro_rules! allocators {
        ($id:ident, $order:ident) => {
            let mut id_n = 0;
            let mut $id = move || {
                id_n += 1;
                format!("id-{id_n}")
            };
            let mut ord_n = 0;
            let mut $order = move || {
                ord_n += 1;
                format!("o{ord_n}")
            };
        };
    }

    #[test]
    fn catalog_is_non_empty_and_unique() {
        let cat = object_template_catalog();
        assert!(!cat.is_empty());
        // ids are unique.
        let mut ids: Vec<&str> = cat.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len());
        // the four real templates are present.
        for want in ["decision_map", "todo_board", "idea_board", "wiki_note"] {
            assert!(cat.iter().any(|m| m.id == want), "missing {want}");
        }
    }

    #[test]
    fn decision_map_has_styled_cards_and_anchored_connector() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("decision_map", 100.0, 50.0, &mut id_alloc, &mut order_alloc);
        // more than one object.
        assert!(objs.len() > 1);
        // at least one object carries inline fill + stroke + text.
        assert!(objs
            .iter()
            .any(|o| o.fill.is_some() && o.stroke.is_some() && o.text.is_some()));
        // at least one connector: an open geometry with two target anchors.
        let connectors: Vec<&Object> = objs.iter().filter(|o| o.anchors.len() == 2).collect();
        assert!(!connectors.is_empty(), "expected at least one connector");
        let c = connectors[0];
        assert_eq!(c.anchors[0].node_index, 0);
        assert_eq!(c.anchors[1].node_index, 1);
        // the connector's targets are ids of cards in the recipe.
        let card_ids: Vec<&str> = objs
            .iter()
            .filter(|o| o.anchors.is_empty())
            .map(|o| o.id.as_str())
            .collect();
        assert!(card_ids.contains(&c.anchors[0].target.as_str()));
        assert!(card_ids.contains(&c.anchors[1].target.as_str()));
    }

    #[test]
    fn cards_are_offset_from_the_anchor() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("decision_map", 100.0, 50.0, &mut id_alloc, &mut order_alloc);
        // the first card sits at anchor + (0,160) per the recipe.
        let first = &objs[0];
        assert_eq!(first.transform.m[0][2], 100.0);
        assert_eq!(first.transform.m[1][2], 50.0 + 160.0);
    }

    #[test]
    fn semantic_preset_risk_is_reddish() {
        let (fill, stroke, text) = semantic_preset_style("risk");
        let Some(Fill { paint: Paint::Solid { color }, .. }) = fill else {
            panic!("expected a solid fill");
        };
        assert_eq!(color, "#fff8f1");
        assert!(stroke.is_some());
        assert!(text.is_some());
    }

    #[test]
    fn semantic_presets_all_resolve() {
        let presets = semantic_presets();
        assert_eq!(presets.len(), 12);
        for p in presets {
            let (fill, stroke, text) = semantic_preset_style(p);
            assert!(fill.is_some() && stroke.is_some() && text.is_some(), "{p}");
        }
        // unknown preset falls back to the default surface (white).
        let (fill, _, _) = semantic_preset_style("nope");
        let Some(Fill { paint: Paint::Solid { color }, .. }) = fill else {
            panic!("default fill");
        };
        assert_eq!(color, "#ffffff");
    }

    #[test]
    fn template_to_ops_wraps_each_object() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("idea_board", 0.0, 0.0, &mut id_alloc, &mut order_alloc);
        let n = objs.len();
        assert!(n > 0);
        let ids: Vec<String> = objs.iter().map(|o| o.id.clone()).collect();
        let ops = template_to_ops(objs);
        assert_eq!(ops.len(), n);
        for (op, want_id) in ops.iter().zip(ids.iter()) {
            match op {
                ObjectOp::InsertObject { object } => assert_eq!(&object.id, want_id),
                other => panic!("expected insert-object, got {}", other.kind()),
            }
        }
    }

    #[test]
    fn ids_and_orders_come_from_allocators_deterministically() {
        let build = || {
            allocators!(id_alloc, order_alloc);
            build_template("decision_map", 7.0, 7.0, &mut id_alloc, &mut order_alloc)
        };
        let a = build();
        let b = build();
        // Same allocators -> byte-identical objects (no rng/time).
        assert_eq!(a, b);
        // ids/orders are exactly the allocator sequence in object order.
        for (i, o) in a.iter().enumerate() {
            assert_eq!(o.id, format!("id-{}", i + 1));
            assert_eq!(o.order, format!("o{}", i + 1));
        }
    }

    #[test]
    fn unknown_template_is_empty() {
        allocators!(id_alloc, order_alloc);
        assert!(build_template("nope", 0.0, 0.0, &mut id_alloc, &mut order_alloc).is_empty());
    }

    #[test]
    fn stub_template_is_one_titled_card() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("adr", 0.0, 0.0, &mut id_alloc, &mut order_alloc);
        assert_eq!(objs.len(), 1);
        assert!(objs[0].text.is_some());
        assert!(objs[0].fill.is_some());
    }
}
