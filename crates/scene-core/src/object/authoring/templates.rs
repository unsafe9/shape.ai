//! Templates are recipes of inline-styled [`Object`]s: a pure function that, given
//! an anchor and id/order allocators, returns a `Vec<Object>` of fully-formed
//! objects. Every object carries its colors inline as [`Fill`]/[`Stroke`] and its
//! label as [`Text`]; the semantic palette is [`semantic_preset_style`].
//!
//! The UI reads [`object_template_catalog`]; picking one sends a
//! `FeatureRequest::TemplateApply { recipe, .. }` whose `recipe` is
//! [`build_template`]'s output, lowered to ops by [`template_to_ops`].
//!
//! Pure: no time/rng/IO. Ids and fractional `order` keys come from injected
//! allocators, so the same call yields byte-identical objects. Geometry is
//! object-local quantized i32 at [`GEOMETRY_QUANTUM_PER_PX`], authored from (0,0);
//! the world placement is the `transform`, so no coordinate `f64 -> i32` narrowing
//! happens (the only `f64` values live in `Transform3x3`).

use crate::object::model::{
    Anchor, Fill, FillRule, Geometry, LineCap, LineJoin, LocalPoint, Object, ObjectScene, Paint,
    PathNode, Stroke, SubPath, Text, TextAlign, TextRun, TextVAlign, Transform3x3,
    GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::op::ObjectOp;
use serde::{Deserialize, Serialize};

/// Gap (logical px) placed to the right of the right-most object's transform
/// origin, so a freshly-applied template never lands on top of existing content.
const TEMPLATE_ANCHOR_GAP_PX: f64 = 240.0;

/// Coarse grouping for the template picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateCategory {
    Planning,
    Knowledge,
    Engineering,
    Presentation,
    General,
}

/// Semantic token names so templates track the active light/dark theme instead of
/// baking a fixed hex (see `object::theme`).
const TOKEN_SURFACE: &str = "surface";
const TOKEN_SURFACE_MUTED: &str = "surface-muted";
const TOKEN_DEFAULT_STROKE: &str = "default-stroke";
const TOKEN_TEXT: &str = "text";

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
            id: "todo_board",
            label: "To-do Board",
            category: TemplateCategory::Planning,
            description:
                "Three grouped columns — To do, In progress, Done — each holding starter task cards.",
            build: build_todo_board,
        },
        ObjectTemplate {
            id: "decision_map",
            label: "Decision Map",
            category: TemplateCategory::Engineering,
            description:
                "A decision point branching to two options and their outcomes, wired with anchored connectors.",
            build: build_decision_map,
        },
        ObjectTemplate {
            id: "presentation",
            label: "Presentation",
            category: TemplateCategory::Presentation,
            description: "A deck frame grouping a title slide and three content slides.",
            build: build_presentation,
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

/// Where a new template should land in world space. Content-extent + spacing: the
/// anchor sits [`TEMPLATE_ANCHOR_GAP_PX`] to the right of the right-most object's
/// transform origin, top-aligned to the highest origin. An empty scene has no
/// content to clear, so it falls back to `(fallback_x, fallback_y)` (the shell's
/// viewport center). Reads only the transform origin (the translation column), not
/// the geometry AABB, matching the placement of a template's own recipe origin.
pub fn template_anchor(scene: &ObjectScene, fallback_x: f64, fallback_y: f64) -> (f64, f64) {
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    for object in &scene.objects {
        // The transform origin is the translation column of the affine matrix.
        let (ox, oy) = (object.transform.m[0][2], object.transform.m[1][2]);
        max_x = max_x.max(ox);
        min_y = min_y.min(oy);
    }
    if max_x.is_finite() && min_y.is_finite() {
        (max_x + TEMPLATE_ANCHOR_GAP_PX, min_y)
    } else {
        (fallback_x, fallback_y)
    }
}

/// One [`ObjectOp::InsertObject`] per object, in recipe order (shapes before the
/// connectors that anchor to them).
pub fn template_to_ops(objs: Vec<Object>) -> Vec<ObjectOp> {
    objs.into_iter()
        .map(|object| ObjectOp::InsertObject { object })
        .collect()
}

/// The 12 semantic preset keys — a source of inline colors that [`build_template`]
/// bakes into objects, never stored on them.
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

/// Inline style for a semantic preset: `(fill surface, stroke accent, text color)`.
/// Stroke width is 1px in quantized units. An unknown preset falls back to default.
pub fn semantic_preset_style(preset: &str) -> (Option<Fill>, Option<Stroke>, Option<String>) {
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

/// A solid fill painted with a theme token (resolves light/dark at draw).
fn token_fill(token: &str) -> Fill {
    Fill { paint: Paint::Token { name: token.to_string() }, opacity: 1.0 }
}

/// A 1px-wide stroke painted with a theme token.
fn token_stroke(token: &str) -> Stroke {
    Stroke {
        paint: Paint::Token { name: token.to_string() },
        width: px(1),
        opacity: 1.0,
        dash: Vec::new(),
        cap: LineCap::Butt,
        join: LineJoin::Round,
    }
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

/// An open 2-node connector body. The world endpoints come from the anchors, not
/// this geometry; the short stub keeps the object non-degenerate before resolution.
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

/// A top-left header label in the theme `text` token color, at `size_px`. Used
/// for frame/slide titles, which sit at the top of their box rather than dead
/// center. `bold` distinguishes a deck/slide heading from a body line.
fn heading(text: &str, size_px: i32, bold: bool) -> Text {
    Text {
        runs: vec![TextRun {
            text: text.to_string(),
            color: Some(TOKEN_TEXT.to_string()),
            size: Some(px(size_px)),
            bold,
            italic: false,
            font: None,
        }],
        align: TextAlign::Start,
        valign: TextVAlign::Top,
    }
}

/// A token-styled rect placed at `anchor + (ox, oy)` px, optionally parented and
/// titled (the title sits top-left). Returns the object so the caller can wire
/// children to its id.
#[allow(clippy::too_many_arguments)]
fn framed(
    fill_token: &str,
    stroke_token: &str,
    title: Option<(&str, i32, bool)>,
    parent: Option<&str>,
    w_px: i32,
    h_px: i32,
    anchor_x: f64,
    anchor_y: f64,
    ox: f64,
    oy: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Object {
    let mut obj = Object::new(id_alloc(), order_alloc(), rect_geometry(w_px, h_px));
    obj.transform = Transform3x3::translate(anchor_x + ox, anchor_y + oy);
    obj.fill = Some(token_fill(fill_token));
    obj.stroke = Some(token_stroke(stroke_token));
    obj.parent = parent.map(|p| p.to_string());
    if let Some((t, size, bold)) = title {
        obj.text = Some(heading(t, size, bold));
    }
    obj
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

/// Like [`card`] but parented. The transform stays world-absolute (children carry
/// absolute transforms); `parent` only wires containment so a parent drag cascades.
#[allow(clippy::too_many_arguments)]
fn card_in(
    preset: &str,
    text: &str,
    w_px: i32,
    h_px: i32,
    anchor_x: f64,
    anchor_y: f64,
    ox: f64,
    oy: f64,
    parent: &str,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Object {
    let mut obj = card(preset, text, w_px, h_px, anchor_x, anchor_y, ox, oy, id_alloc, order_alloc);
    obj.parent = Some(parent.to_string());
    obj
}

/// An open 2-node object whose endpoints anchor to `from` (node 0) and `to`
/// (node 1); each `at` is the target's center, re-projected when its geometry edits.
fn connector(
    from: &Object,
    to: &Object,
    from_center: LocalPoint,
    to_center: LocalPoint,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Object {
    let mut obj = Object::new(id_alloc(), order_alloc(), segment_geometry(40, 0));
    // Placement is identity; the endpoints are governed by the anchors.
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

/// Decision Map: a decision point branching to two options, each leading to its
/// outcome, wired with four anchored connectors that re-project when a card moves.
fn build_decision_map(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    const W: i32 = 200;
    const H: i32 = 110;
    // Three columns (x), options/outcomes split across two rows (y). The gaps
    // (320px column pitch, 240px row pitch) keep 200x110 cards clear of overlap.
    let decision = card(
        "decision_point", "Decision point", W, H, ax, ay, 0.0, 180.0, id_alloc, order_alloc,
    );
    let option_a = card("option", "Option A", W, H, ax, ay, 320.0, 60.0, id_alloc, order_alloc);
    let option_b =
        card("option", "Option B", W, H, ax, ay, 320.0, 300.0, id_alloc, order_alloc);
    let outcome_a = card(
        "subdecision", "Outcome A", W, H, ax, ay, 640.0, 60.0, id_alloc, order_alloc,
    );
    let outcome_b = card(
        "subdecision", "Outcome B", W, H, ax, ay, 640.0, 300.0, id_alloc, order_alloc,
    );

    let c = center_of(W, H);
    let e1 = connector(&decision, &option_a, c, c, id_alloc, order_alloc);
    let e2 = connector(&decision, &option_b, c, c, id_alloc, order_alloc);
    let e3 = connector(&option_a, &outcome_a, c, c, id_alloc, order_alloc);
    let e4 = connector(&option_b, &outcome_b, c, c, id_alloc, order_alloc);

    vec![decision, option_a, option_b, outcome_a, outcome_b, e1, e2, e3, e4]
}

/// To-do Board: three `surface-muted` column frames each parenting `task`-preset
/// cards, so dragging a column cascades its cards. Emitted parents-first so the
/// recipe stays well-formed when applied in order.
fn build_todo_board(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    const COL_W: i32 = 240;
    const COL_H: i32 = 460;
    const COL_PITCH: f64 = 280.0;
    const CARD_W: i32 = 200;
    const CARD_H: i32 = 70;
    // Card inset within a column, and the y of the first/second card (below the
    // column title) — 90px card pitch clears the 70px-tall cards.
    const CARD_OX: f64 = 20.0;
    const CARD_Y0: f64 = 60.0;
    const CARD_PITCH: f64 = 90.0;

    let columns = [("To do", 0.0), ("In progress", COL_PITCH), ("Done", COL_PITCH * 2.0)];
    let cards: [(&str, &str); 3] =
        [("To do", "First task"), ("In progress", "Working on it"), ("Done", "Shipped")];

    let mut parents: Vec<Object> = Vec::with_capacity(columns.len());
    let mut children: Vec<Object> = Vec::with_capacity(cards.len() * 2);
    for (title, col_x) in columns {
        let column = framed(
            TOKEN_SURFACE_MUTED,
            TOKEN_DEFAULT_STROKE,
            Some((title, 14, true)),
            None,
            COL_W,
            COL_H,
            ax,
            ay,
            col_x,
            0.0,
            id_alloc,
            order_alloc,
        );
        // Two starter cards per column, parented to the column frame.
        let card_text = cards.iter().find(|(c, _)| *c == title).map(|(_, t)| *t).unwrap_or("Task");
        for row in 0..2 {
            let label = if row == 0 { card_text } else { "New task" };
            children.push(card_in(
                "task",
                label,
                CARD_W,
                CARD_H,
                ax,
                ay,
                col_x + CARD_OX,
                CARD_Y0 + CARD_PITCH * f64::from(row),
                &column.id,
                id_alloc,
                order_alloc,
            ));
        }
        parents.push(column);
    }
    parents.into_iter().chain(children).collect()
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

/// Presentation: a `surface-muted` deck frame parenting four `surface` slide
/// frames in a 2x2 grid, so dragging the deck cascades all slides. Parents-first.
fn build_presentation(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    const PAD: f64 = 24.0;
    const SLIDE_W: i32 = 280;
    const SLIDE_H: i32 = 160;
    const COL_PITCH: f64 = 304.0; // SLIDE_W + 24px gutter
    const ROW_PITCH: f64 = 184.0; // SLIDE_H + 24px gutter
    // Deck frame wraps the 2x2 grid plus padding on all sides.
    const DECK_W: i32 = 2 * SLIDE_W + 24 + 2 * 24; // two slides + inner gutter + outer pad
    const DECK_H: i32 = 2 * SLIDE_H + 24 + 2 * 24;

    let deck = framed(
        TOKEN_SURFACE_MUTED,
        TOKEN_DEFAULT_STROKE,
        Some(("Deck", 16, true)),
        None,
        DECK_W,
        DECK_H,
        ax,
        ay,
        0.0,
        0.0,
        id_alloc,
        order_alloc,
    );

    let slides: [(&str, i32, i32); 4] = [
        ("Title slide", 0, 0),
        ("Agenda", 1, 0),
        ("Content", 0, 1),
        ("Summary", 1, 1),
    ];
    let mut objs: Vec<Object> = Vec::with_capacity(1 + slides.len());
    let mut children: Vec<Object> = Vec::with_capacity(slides.len());
    for (title, col, row) in slides {
        let ox = PAD + COL_PITCH * f64::from(col);
        let oy = PAD + ROW_PITCH * f64::from(row);
        children.push(framed(
            TOKEN_SURFACE,
            TOKEN_DEFAULT_STROKE,
            Some((title, 13, false)),
            Some(&deck.id),
            SLIDE_W,
            SLIDE_H,
            ax,
            ay,
            ox,
            oy,
            id_alloc,
            order_alloc,
        ));
    }
    objs.push(deck);
    objs.extend(children);
    objs
}

/// One titled card with the given preset — the shared body for stub templates.
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

/// Stub: ADR (architecture decision record).
fn build_adr_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("decision", "ADR", ax, ay, id_alloc, order_alloc)
}

/// Stub: investigation map.
fn build_investigation_map_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("evidence", "Investigation Map", ax, ay, id_alloc, order_alloc)
}

/// Stub: dependency diagram.
fn build_dependency_diagram_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("artifact", "Dependency Diagram", ax, ay, id_alloc, order_alloc)
}

/// Stub: server architecture.
fn build_server_architecture_stub(
    ax: f64,
    ay: f64,
    id_alloc: &mut dyn FnMut() -> String,
    order_alloc: &mut dyn FnMut() -> String,
) -> Vec<Object> {
    titled_rect("artifact", "Server Architecture", ax, ay, id_alloc, order_alloc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sequential `id-{n}` / `o{n}` allocators (deterministic, no rng). A macro
    /// because `impl FnMut` cannot be returned nested inside a tuple.
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
        // the first card sits at anchor + (0,180) per the recipe.
        let first = &objs[0];
        assert_eq!(first.transform.m[0][2], 100.0);
        assert_eq!(first.transform.m[1][2], 50.0 + 180.0);
    }

    /// World-px bounds of a closed `framed`/`card` rect. `None` for open geometry.
    fn world_box(o: &Object) -> Option<(f64, f64, f64, f64)> {
        let sp = o.geometry.subpaths.first()?;
        if !sp.closed {
            return None;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for n in &sp.nodes {
            minx = minx.min(n.x);
            miny = miny.min(n.y);
            maxx = maxx.max(n.x);
            maxy = maxy.max(n.y);
        }
        let q = f64::from(GEOMETRY_QUANTUM_PER_PX);
        let tx = o.transform.m[0][2];
        let ty = o.transform.m[1][2];
        Some((
            tx + f64::from(minx) / q,
            ty + f64::from(miny) / q,
            tx + f64::from(maxx) / q,
            ty + f64::from(maxy) / q,
        ))
    }

    /// Do two world boxes overlap (strict interior intersection)?
    fn boxes_overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    }

    #[test]
    fn todo_board_is_grouped_with_parents_and_children() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("todo_board", 0.0, 0.0, &mut id_alloc, &mut order_alloc);
        // Three column frames (no parent) + their child cards (parent set).
        let parents: Vec<&Object> = objs.iter().filter(|o| o.parent.is_none()).collect();
        let children: Vec<&Object> = objs.iter().filter(|o| o.parent.is_some()).collect();
        assert_eq!(parents.len(), 3, "three grouped columns");
        assert!(children.len() >= 3, "columns hold task cards");
        // Every child points at a real parent in the same recipe (grouping wired).
        let parent_ids: Vec<&str> = parents.iter().map(|o| o.id.as_str()).collect();
        for c in &children {
            let p = c.parent.as_deref().unwrap();
            assert!(parent_ids.contains(&p), "child {} parent {p} not in recipe", c.id);
        }
        // Parents emitted before children, so the recipe applies in order.
        let first_child = objs.iter().position(|o| o.parent.is_some()).unwrap();
        assert!(
            objs[..first_child].iter().all(|o| o.parent.is_none()),
            "parents must precede children",
        );
        // Columns carry a theme-token fill, not a baked hex.
        assert!(parents.iter().all(|o| matches!(
            o.fill.as_ref().map(|f| &f.paint),
            Some(Paint::Token { .. })
        )));
    }

    #[test]
    fn presentation_groups_slides_under_a_deck() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("presentation", 0.0, 0.0, &mut id_alloc, &mut order_alloc);
        let parents: Vec<&Object> = objs.iter().filter(|o| o.parent.is_none()).collect();
        let children: Vec<&Object> = objs.iter().filter(|o| o.parent.is_some()).collect();
        assert_eq!(parents.len(), 1, "one deck frame");
        assert_eq!(children.len(), 4, "four slides under the deck");
        let deck = parents[0];
        assert!(children.iter().all(|c| c.parent.as_deref() == Some(deck.id.as_str())));
        // No two slides overlap (tuned 2x2 grid).
        let slide_boxes: Vec<(f64, f64, f64, f64)> =
            children.iter().filter_map(|o| world_box(o)).collect();
        assert_eq!(slide_boxes.len(), 4);
        for i in 0..slide_boxes.len() {
            for j in (i + 1)..slide_boxes.len() {
                assert!(
                    !boxes_overlap(slide_boxes[i], slide_boxes[j]),
                    "slides {i} and {j} overlap",
                );
            }
        }
    }

    #[test]
    fn decision_map_connectors_resolve_to_distinct_non_overlapping_cards() {
        allocators!(id_alloc, order_alloc);
        let objs = build_template("decision_map", 0.0, 0.0, &mut id_alloc, &mut order_alloc);
        let cards: Vec<&Object> = objs.iter().filter(|o| o.anchors.is_empty()).collect();
        let connectors: Vec<&Object> = objs.iter().filter(|o| o.anchors.len() == 2).collect();
        assert_eq!(cards.len(), 5, "decision point + 2 options + 2 outcomes");
        assert!(connectors.len() >= 4, "four anchored connectors");
        let card_ids: Vec<&str> = cards.iter().map(|o| o.id.as_str()).collect();
        // Every connector anchors to two *distinct* real cards in the recipe.
        for e in &connectors {
            let (a, b) = (e.anchors[0].target.as_str(), e.anchors[1].target.as_str());
            assert_ne!(a, b, "connector endpoints must differ");
            assert!(card_ids.contains(&a) && card_ids.contains(&b), "anchors resolve to cards");
        }
        // No two cards overlap (tuned columns/rows).
        let boxes: Vec<(f64, f64, f64, f64)> = cards.iter().filter_map(|o| world_box(o)).collect();
        assert_eq!(boxes.len(), 5);
        for i in 0..boxes.len() {
            for j in (i + 1)..boxes.len() {
                assert!(!boxes_overlap(boxes[i], boxes[j]), "cards {i} and {j} overlap");
            }
        }
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

    /// A scene of one rect placed at the given world origin (the only field
    /// `template_anchor` reads is the transform's translation column).
    fn scene_with_origins(origins: &[(f64, f64)]) -> ObjectScene {
        let mut scene = ObjectScene::default();
        for (i, &(x, y)) in origins.iter().enumerate() {
            let mut obj = Object::new(format!("o{i}"), format!("a{i}"), rect_geometry(40, 30));
            obj.transform = Transform3x3::translate(x, y);
            scene.objects.push(obj);
        }
        scene
    }

    #[test]
    fn template_anchor_lands_right_and_top_aligned() {
        // Three objects; right-most origin x = 500, highest origin y = 10.
        let scene = scene_with_origins(&[(100.0, 200.0), (500.0, 80.0), (300.0, 10.0)]);
        let (ax, ay) = template_anchor(&scene, -1.0, -1.0);
        // +240 right of the right-most origin, top-aligned to the highest origin.
        assert_eq!(ax, 500.0 + 240.0);
        assert_eq!(ay, 10.0);
        // The fallback is ignored when the scene has content.
        assert_ne!((ax, ay), (-1.0, -1.0));
    }

    #[test]
    fn template_anchor_empty_scene_uses_fallback() {
        let scene = ObjectScene::default();
        assert!(scene.objects.is_empty());
        let (ax, ay) = template_anchor(&scene, 17.0, 23.0);
        assert_eq!((ax, ay), (17.0, 23.0));
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
