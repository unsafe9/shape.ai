//! Template subsystem — recipe data types + builtin template library.
//!
//! A `TemplateContract` is a *recipe over existing primitives*: it declares the
//! frames / shapes / edges / tags a template creates. The builtin contracts are
//! served by the server's template-library API (`/api/templates`); the live
//! drawing path lowers object-template recipes in `object::templates`, so the
//! legacy render-patch lowering that used to live here is gone (OB4.4).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::model::{ExportType, ObjectMeta, WorldPoint};

// ---------------------------------------------------------------------------
// §1 Template metadata
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateCategory {
    Planning,
    Knowledge,
    Engineering,
    Presentation,
    General,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMetadata {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: TemplateCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Value written to created objects' `meta.templateKind`.
    pub template_kind: String,
}

// ---------------------------------------------------------------------------
// §2 Primitive recipe
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeFrame {
    pub local_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_local_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_local_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ObjectMeta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeShape {
    pub local_id: String,
    pub frame_local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Chosen by the template, not derived from a demoted nodeType.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_local_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ObjectMeta>,
    pub position: WorldPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<RecipeSize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecipeSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeEdge {
    pub local_id: String,
    pub frame_local_id: String,
    pub source_local_id: String,
    pub target_local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ObjectMeta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateRecipe {
    pub frames: Vec<RecipeFrame>,
    pub shapes: Vec<RecipeShape>,
    pub edges: Vec<RecipeEdge>,
}

// ---------------------------------------------------------------------------
// §3 Default layout
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeLayout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<WorldPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_shape_size: Option<RecipeSize>,
}

// ---------------------------------------------------------------------------
// §4 Allowed exports
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateExports {
    pub allowed: Vec<ExportType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ExportType>,
}

// ---------------------------------------------------------------------------
// §5 Suggested tags
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedTag {
    pub local_id: String,
    pub name: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateTags {
    pub suggested: Vec<SuggestedTag>,
}

// ---------------------------------------------------------------------------
// §6 Optional AI prompt hints
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePromptHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_hints: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_operations: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// §7 Full contract
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateContract {
    pub metadata: TemplateMetadata,
    pub recipe: TemplateRecipe,
    pub layout: RecipeLayout,
    pub exports: TemplateExports,
    pub tags: TemplateTags,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hints: Option<TemplatePromptHints>,
}

// ===========================================================================
// Builtin construction helpers (terse literal builders)
// ===========================================================================

fn meta_map(pairs: &[(&str, Value)]) -> ObjectMeta {
    let mut m: ObjectMeta = Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

fn frame(local_id: &str, title: &str) -> RecipeFrame {
    RecipeFrame {
        local_id: local_id.to_string(),
        title: title.to_string(),
        summary: None,
        parent_local_id: None,
        tag_local_ids: None,
        meta: None,
    }
}

fn size(w: f64, h: f64) -> RecipeSize {
    RecipeSize {
        width: w,
        height: h,
    }
}

fn point(x: f64, y: f64) -> WorldPoint {
    WorldPoint { x, y }
}

fn tag(local_id: &str, name: &str, color: &str, description: Option<&str>) -> SuggestedTag {
    SuggestedTag {
        local_id: local_id.to_string(),
        name: name.to_string(),
        color: color.to_string(),
        description: description.map(|s| s.to_string()),
    }
}

fn edge(
    local_id: &str,
    frame_local_id: &str,
    source: &str,
    target: &str,
    label: &str,
    style_key: Option<&str>,
    meta: Option<ObjectMeta>,
) -> RecipeEdge {
    RecipeEdge {
        local_id: local_id.to_string(),
        frame_local_id: frame_local_id.to_string(),
        source_local_id: source.to_string(),
        target_local_id: target.to_string(),
        label: Some(label.to_string()),
        style_key: style_key.map(|s| s.to_string()),
        meta,
    }
}

// ===========================================================================
// Builtin: todo-board (todoBoard.ts)
// ===========================================================================

pub fn todo_board_template() -> TemplateContract {
    let frames = vec![
        RecipeFrame {
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("board")),
            ])),
            ..frame("f-board", "Task Board")
        },
        RecipeFrame {
            parent_local_id: Some("f-board".to_string()),
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("column")),
                ("column", json!("todo")),
            ])),
            ..frame("f-todo", "To Do")
        },
        RecipeFrame {
            parent_local_id: Some("f-board".to_string()),
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("column")),
                ("column", json!("doing")),
            ])),
            ..frame("f-doing", "In Progress")
        },
        RecipeFrame {
            parent_local_id: Some("f-board".to_string()),
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("column")),
                ("column", json!("done")),
            ])),
            ..frame("f-done", "Done")
        },
    ];
    let shapes = vec![
        RecipeShape {
            local_id: "t-1".to_string(),
            frame_local_id: "f-todo".to_string(),
            title: Some("Define requirements".to_string()),
            summary: Some("Capture the acceptance criteria for the feature.".to_string()),
            detail: Some(String::new()),
            style_key: Some("task".to_string()),
            tag_local_ids: Some(vec![
                "tg-status-todo".to_string(),
                "tg-priority-normal".to_string(),
            ]),
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("task")),
                ("status", json!("todo")),
                ("owner", json!("")),
                ("priority", json!("normal")),
            ])),
            position: point(0.0, 0.0),
            size: Some(size(390.0, 160.0)),
        },
        RecipeShape {
            local_id: "t-2".to_string(),
            frame_local_id: "f-doing".to_string(),
            title: Some("Wire up auth".to_string()),
            summary: Some("Add OAuth callback route and session handling.".to_string()),
            detail: Some(String::new()),
            style_key: Some("task".to_string()),
            tag_local_ids: Some(vec![
                "tg-status-doing".to_string(),
                "tg-priority-high".to_string(),
            ]),
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("task")),
                ("status", json!("doing")),
                ("owner", json!("")),
                ("priority", json!("high")),
            ])),
            position: point(470.0, 0.0),
            size: Some(size(390.0, 160.0)),
        },
        RecipeShape {
            local_id: "t-3".to_string(),
            frame_local_id: "f-done".to_string(),
            title: Some("Scaffold project".to_string()),
            summary: Some("Initialize repo, CI, and base configuration.".to_string()),
            detail: Some(String::new()),
            style_key: Some("task".to_string()),
            tag_local_ids: Some(vec!["tg-status-done".to_string()]),
            meta: Some(meta_map(&[
                ("templateKind", json!("todo")),
                ("semanticType", json!("task")),
                ("status", json!("done")),
                ("owner", json!("")),
                ("priority", json!("normal")),
            ])),
            position: point(940.0, 0.0),
            size: Some(size(390.0, 160.0)),
        },
    ];
    let edges = vec![edge(
        "e-1",
        "f-board",
        "t-2",
        "t-1",
        "blocks",
        Some("depends_on"),
        Some(meta_map(&[("semanticType", json!("depends_on"))])),
    )];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "todo-board".to_string(),
            title: "Todo / Task Board".to_string(),
            description:
                "Columns of task cards with status, owner, priority, and dependency edges."
                    .to_string(),
            category: TemplateCategory::Planning,
            icon: Some("board".to_string()),
            template_kind: "todo".to_string(),
        },
        recipe: TemplateRecipe {
            frames,
            shapes,
            edges,
        },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(390.0, 160.0)),
        },
        exports: TemplateExports {
            allowed: vec![ExportType::AiPlanMd, ExportType::Mermaid],
            default: Some(ExportType::AiPlanMd),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tg-status-todo", "To Do", "#94a3b8", None),
                tag("tg-status-doing", "In Progress", "#3b82f6", None),
                tag("tg-status-done", "Done", "#22c55e", None),
                tag("tg-status-blocked", "Blocked", "#ef4444", None),
                tag("tg-priority-low", "Low", "#a3e635", None),
                tag("tg-priority-normal", "Normal", "#facc15", None),
                tag("tg-priority-high", "High", "#f97316", None),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some(
                "This is a todo/task board. Cards are tasks; frames are Kanban columns. \
                 Move a card between columns with set-object-group. \
                 Add a dependency with create-edge (styleKey:depends_on). \
                 Change status/owner/priority by updating meta on the card."
                    .to_string(),
            ),
            field_hints: Some(meta_map(&[
                ("task", json!("meta.status ∈ {todo,doing,done,blocked}; meta.priority ∈ {low,normal,high}; meta.owner is a free-form handle or name.")),
                ("column", json!("meta.column ∈ {todo,doing,done} identifies the column's semantic role. Add/rename/remove columns with create-group/edit-text/delete-group.")),
            ])),
            suggested_operations: Some(vec![
                "create-card".to_string(),
                "edit-card-text".to_string(),
                "move-card".to_string(),
                "set-object-group".to_string(),
                "create-edge".to_string(),
                "delete-edge".to_string(),
                "set-object-tags".to_string(),
                "add-comment".to_string(),
                "delete-card".to_string(),
            ]),
        }),
    }
}

// ===========================================================================
// Builtin: wiki-note (wikiNote.ts)
// ===========================================================================

pub fn wiki_note_template() -> TemplateContract {
    let frames = vec![
        RecipeFrame {
            summary: Some("A linked cluster of notes with sources and references.".to_string()),
            meta: Some(meta_map(&[("semanticType", json!("wiki-cluster"))])),
            ..frame("f-cluster", "Untitled Wiki")
        },
        RecipeFrame {
            parent_local_id: Some("f-cluster".to_string()),
            meta: Some(meta_map(&[("semanticType", json!("wiki-section"))])),
            ..frame("f-section-a", "Section")
        },
    ];
    let note = |local_id: &str, frame_local_id: &str, title: &str, summary: &str,
                pos: WorldPoint, sz: RecipeSize, tag_lid: &str|
     -> RecipeShape {
        RecipeShape {
            local_id: local_id.to_string(),
            frame_local_id: frame_local_id.to_string(),
            title: Some(title.to_string()),
            summary: Some(summary.to_string()),
            detail: Some(String::new()),
            style_key: Some("proposition".to_string()),
            tag_local_ids: Some(vec![tag_lid.to_string()]),
            meta: Some(meta_map(&[("semanticType", json!("note"))])),
            position: pos,
            size: Some(sz),
        }
    };
    let shapes = vec![
        note("n-overview", "f-cluster", "Overview", "Introduce the topic here.",
             point(40.0, 40.0), size(390.0, 390.0), "t-topic"),
        note("n-note-1", "f-section-a", "Note 1", "",
             point(40.0, 40.0), size(390.0, 390.0), "t-draft"),
        note("n-note-2", "f-section-a", "Note 2", "",
             point(470.0, 40.0), size(390.0, 390.0), "t-draft"),
        RecipeShape {
            local_id: "n-source-1".to_string(),
            frame_local_id: "f-cluster".to_string(),
            title: Some("Source Title".to_string()),
            summary: Some("Citation summary".to_string()),
            detail: Some("Quoted excerpt or abstract.".to_string()),
            style_key: Some("evidence".to_string()),
            tag_local_ids: Some(vec!["t-source".to_string()]),
            meta: Some(meta_map(&[
                ("semanticType", json!("source")),
                ("citationKind", json!("url")),
                ("evidenceRefsHint", json!(["https://example.com"])),
            ])),
            position: point(470.0, 40.0),
            size: Some(size(390.0, 220.0)),
        },
    ];
    let edges = vec![
        edge("e-ref-1", "f-cluster", "n-note-1", "n-note-2", "see also",
             Some("supports"), Some(meta_map(&[("semanticType", json!("reference"))]))),
        edge("e-cite-1", "f-cluster", "n-note-1", "n-source-1", "cited by",
             Some("supports"), Some(meta_map(&[("semanticType", json!("citation"))]))),
    ];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "wiki-note".to_string(),
            title: "Wiki Note Cluster".to_string(),
            description: "Linked prose notes with sources and references.".to_string(),
            category: TemplateCategory::Knowledge,
            icon: None,
            template_kind: "wiki-note".to_string(),
        },
        recipe: TemplateRecipe {
            frames,
            shapes,
            edges,
        },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(390.0, 390.0)),
        },
        exports: TemplateExports {
            allowed: vec![
                ExportType::DesignDocMd,
                ExportType::ConfluenceHtml,
                ExportType::Madr,
                ExportType::Mermaid,
            ],
            default: Some(ExportType::DesignDocMd),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("t-topic", "topic", "#5B8DEF", None),
                tag("t-source", "source", "#8E8E93", None),
                tag("t-draft", "draft", "#E0A458", None),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some(
                "This is a wiki note cluster; cards are notes, evidence cards carry sources in evidenceRefs, edges are references."
                    .to_string(),
            ),
            field_hints: Some(meta_map(&[
                ("note", json!("Heading in title, lede in summary, body in detail")),
                ("source", json!("Put the citation URL/path in evidenceRefs; quote in detail")),
            ])),
            suggested_operations: Some(vec![
                "create".to_string(),
                "connect".to_string(),
                "tag".to_string(),
                "comment".to_string(),
            ]),
        }),
    }
}

// ===========================================================================
// Builtin: idea-board (wikiNote.ts)
// ===========================================================================

pub fn idea_board_template() -> TemplateContract {
    let frames = vec![
        RecipeFrame {
            meta: Some(meta_map(&[("semanticType", json!("idea-board"))])),
            ..frame("f-board", "Idea Board")
        },
        RecipeFrame {
            parent_local_id: Some("f-board".to_string()),
            meta: Some(meta_map(&[("semanticType", json!("idea-theme"))])),
            ..frame("f-theme-1", "Theme")
        },
    ];
    let idea = |local_id: &str, frame_local_id: &str, title: &str, pos: WorldPoint, tag_lid: &str|
     -> RecipeShape {
        RecipeShape {
            local_id: local_id.to_string(),
            frame_local_id: frame_local_id.to_string(),
            title: Some(title.to_string()),
            summary: Some("Describe the idea.".to_string()),
            detail: Some(String::new()),
            style_key: Some("option".to_string()),
            tag_local_ids: Some(vec![tag_lid.to_string()]),
            meta: Some(meta_map(&[("semanticType", json!("idea"))])),
            position: pos,
            size: Some(size(390.0, 390.0)),
        }
    };
    let shapes = vec![
        idea("n-idea-1", "f-board", "Idea 1", point(40.0, 40.0), "t-spark"),
        idea("n-idea-2", "f-theme-1", "Idea 2", point(40.0, 40.0), "t-theme"),
        idea("n-idea-3", "f-theme-1", "Idea 3", point(470.0, 40.0), "t-theme"),
        RecipeShape {
            local_id: "n-source-1".to_string(),
            frame_local_id: "f-board".to_string(),
            title: Some("Source Title".to_string()),
            summary: Some("Citation summary".to_string()),
            detail: Some("Quoted excerpt.".to_string()),
            style_key: Some("evidence".to_string()),
            tag_local_ids: Some(vec!["t-source".to_string()]),
            meta: Some(meta_map(&[
                ("semanticType", json!("source")),
                ("citationKind", json!("url")),
                ("evidenceRefsHint", json!(["https://example.com"])),
            ])),
            position: point(470.0, 40.0),
            size: Some(size(390.0, 220.0)),
        },
    ];
    let edges = vec![RecipeEdge {
        local_id: "e-rel-1".to_string(),
        frame_local_id: "f-board".to_string(),
        source_local_id: "n-idea-1".to_string(),
        target_local_id: "n-idea-2".to_string(),
        label: Some("relates to".to_string()),
        style_key: None,
        meta: Some(meta_map(&[("semanticType", json!("relates-to"))])),
    }];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "idea-board".to_string(),
            title: "Idea Board".to_string(),
            description: "Freeform ideation cards grouped into themes.".to_string(),
            category: TemplateCategory::Knowledge,
            icon: None,
            template_kind: "idea-board".to_string(),
        },
        recipe: TemplateRecipe {
            frames,
            shapes,
            edges,
        },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(390.0, 390.0)),
        },
        exports: TemplateExports {
            allowed: vec![
                ExportType::DesignDocMd,
                ExportType::Mermaid,
                ExportType::ImagePrompt,
            ],
            default: Some(ExportType::DesignDocMd),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("t-theme", "theme", "#5B8DEF", None),
                tag("t-spark", "spark", "#E0A458", None),
                tag("t-parked", "parked", "#8E8E93", None),
                tag("t-source", "source", "#636366", None),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some(
                "This is an idea board; cards are ideas grouped into themes, evidence cards carry sources, edges are associative links."
                    .to_string(),
            ),
            field_hints: Some(meta_map(&[
                ("idea", json!("Idea title in title, note in summary, elaboration in detail")),
                ("source", json!("Put the citation URL/path in evidenceRefs; quote in detail")),
            ])),
            suggested_operations: Some(vec![
                "create".to_string(),
                "connect".to_string(),
                "tag".to_string(),
                "comment".to_string(),
            ]),
        }),
    }
}

// ===========================================================================
// Builtin: adr (adrArchitecture.ts)
// ===========================================================================

/// Helper for the ADR / architecture shapes: title+summary used as detail too,
/// fixed 270x178 size, status carried in meta.
fn adr_shape(
    local_id: &str,
    frame_local_id: &str,
    title: &str,
    body: &str,
    style_key: &str,
    pos: WorldPoint,
    semantic_type: &str,
    status: Option<&str>,
) -> RecipeShape {
    let mut pairs: Vec<(&str, Value)> = vec![("semanticType", json!(semantic_type))];
    if let Some(s) = status {
        pairs.push(("status", json!(s)));
    }
    RecipeShape {
        local_id: local_id.to_string(),
        frame_local_id: frame_local_id.to_string(),
        title: Some(title.to_string()),
        summary: Some(body.to_string()),
        detail: Some(body.to_string()),
        style_key: Some(style_key.to_string()),
        tag_local_ids: None,
        meta: Some(meta_map(&pairs)),
        position: pos,
        size: Some(size(270.0, 178.0)),
    }
}

fn adr_edge(
    local_id: &str,
    source: &str,
    target: &str,
    label: &str,
    semantic_type: &str,
) -> RecipeEdge {
    edge(
        local_id,
        "root",
        source,
        target,
        label,
        Some(semantic_type),
        Some(meta_map(&[("semanticType", json!(semantic_type))])),
    )
}

pub fn adr_template() -> TemplateContract {
    let frames = vec![RecipeFrame {
        summary: Some("Architecture Decision Record".to_string()),
        meta: Some(meta_map(&[("templateKind", json!("adr"))])),
        ..frame("root", "ADR")
    }];
    let shapes = vec![
        adr_shape("proposition", "root", "Proposition",
            "Problem statement and target outcome.", "proposition",
            point(0.0, 300.0), "proposition", Some("selected")),
        adr_shape("decision-points", "root", "Decision points",
            "The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.",
            "decision_point", point(360.0, 300.0), "decision_point", Some("draft")),
        adr_shape("option-graph", "root", "Typed decision graph",
            "Use a typed graph as the source of truth for discussion and exports.",
            "option", point(720.0, 120.0), "option", Some("viable")),
        adr_shape("option-freeform", "root", "Freeform mindmap",
            "Flexible, but weak at enforcing architectural decision quality.",
            "option", point(720.0, 480.0), "option", Some("conditional")),
        adr_shape("evidence", "root", "Evidence ledger",
            "Every recommendation should trace to a concrete assumption, source, or probe.",
            "evidence", point(1080.0, 40.0), "evidence", Some("draft")),
        adr_shape("tradeoff", "root", "Readable vs complete",
            "Show compact nodes by default and move deep rationale into the inspector.",
            "tradeoff", point(1080.0, 300.0), "tradeoff", Some("draft")),
        adr_shape("blocker", "root", "Unbounded local permissions",
            "Shell execution and code editing are out of MVP scope unless a future approval boundary is added.",
            "blocker", point(1080.0, 560.0), "blocker", Some("infeasible")),
        adr_shape("subdecision", "root", "Export scope",
            "Exports must work for the whole graph and selected subgraphs.",
            "subdecision", point(1440.0, 120.0), "subdecision", Some("draft")),
        adr_shape("task", "root", "First vertical slice",
            "Create a group, inspect a node, leave comments, and export Markdown.",
            "task", point(1440.0, 380.0), "task", Some("draft")),
        adr_shape("artifact", "root", "Derived artifacts",
            "MADR, YADR, Mermaid, and image-generation prompts.",
            "artifact", point(1440.0, 640.0), "artifact", Some("draft")),
    ];
    let edges = vec![
        adr_edge("e1", "proposition", "decision-points", "decide by", "decomposes_to"),
        adr_edge("e2", "decision-points", "option-graph", "recommended", "chooses_between"),
        adr_edge("e3", "decision-points", "option-freeform", "alternative", "chooses_between"),
        adr_edge("e4", "evidence", "option-graph", "supports", "supports"),
        adr_edge("e5", "option-graph", "tradeoff", "accepts", "trades_off_with"),
        adr_edge("e6", "blocker", "option-freeform", "weakens", "blocks"),
        adr_edge("e7", "option-graph", "subdecision", "needs", "decomposes_to"),
        adr_edge("e8", "task", "option-graph", "builds on", "depends_on"),
        adr_edge("e9", "subdecision", "artifact", "exports", "produces"),
    ];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "adr".to_string(),
            title: "ADR / Design Decision".to_string(),
            description: "Architecture Decision Record — full ADR scaffold with proposition, options, evidence, tradeoffs, blockers, and derived artifacts.".to_string(),
            category: TemplateCategory::Engineering,
            icon: None,
            template_kind: "adr".to_string(),
        },
        recipe: TemplateRecipe { frames, shapes, edges },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(270.0, 178.0)),
        },
        exports: TemplateExports {
            allowed: vec![
                ExportType::Madr,
                ExportType::Yadr,
                ExportType::DesignDocMd,
                ExportType::ConfluenceHtml,
                ExportType::Mermaid,
            ],
            default: Some(ExportType::Madr),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tag-viable", "viable", "#22c55e", Some("Option is viable")),
                tag("tag-conditional", "conditional", "#f59e0b", Some("Option is conditional")),
                tag("tag-infeasible", "infeasible", "#ef4444", Some("Option is infeasible")),
                tag("tag-selected", "selected", "#6366f1", Some("Option is selected")),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some("An ADR (Architecture Decision Record): a proposition, decision points, competing options, evidence, tradeoffs, blockers, subdecisions, tasks, and derived artifacts. The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.".to_string()),
            field_hints: Some(meta_map(&[
                ("proposition", json!("State the problem and target outcome clearly.")),
                ("decision_point", json!("Name the key decision drivers that guide option selection.")),
                ("option", json!("Describe each option with enough detail to evaluate feasibility and tradeoffs.")),
                ("evidence", json!("Cite concrete assumptions, sources, or probes that support an option.")),
                ("tradeoff", json!("State what is accepted or sacrificed by choosing an option.")),
                ("blocker", json!("Identify hard constraints that rule out an option.")),
                ("subdecision", json!("Decompose the main decision into scoped sub-decisions.")),
                ("task", json!("List the first implementation steps once an option is selected.")),
                ("artifact", json!("Name the derived export artifacts (MADR, YADR, Mermaid, image prompt).")),
            ])),
            suggested_operations: None,
        }),
    }
}

// ===========================================================================
// Builtin: decision-map (adrArchitecture.ts)
// ===========================================================================

pub fn decision_map_template() -> TemplateContract {
    let frames = vec![RecipeFrame {
        summary: Some("Decision spine: decision point, options, sub-decisions.".to_string()),
        meta: Some(meta_map(&[("templateKind", json!("decision-map"))])),
        ..frame("root", "Decision Map")
    }];
    let shapes = vec![
        adr_shape("decision", "root", "Decision point", "What needs to be decided?",
            "decision_point", point(0.0, 200.0), "decision_point", Some("draft")),
        adr_shape("option-a", "root", "Option A", "First competing option.",
            "option", point(360.0, 80.0), "option", Some("viable")),
        adr_shape("option-b", "root", "Option B", "Second competing option.",
            "option", point(360.0, 320.0), "option", Some("conditional")),
        adr_shape("subdecision", "root", "Sub-decision",
            "A decision that depends on the chosen option.",
            "subdecision", point(720.0, 80.0), "subdecision", Some("draft")),
    ];
    let edges = vec![
        adr_edge("e1", "decision", "option-a", "considers", "chooses_between"),
        adr_edge("e2", "decision", "option-b", "considers", "chooses_between"),
        adr_edge("e3", "option-a", "subdecision", "leads to", "decomposes_to"),
    ];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "decision-map".to_string(),
            title: "Decision Map".to_string(),
            description: "Lightweight decision map — one decision point, competing options, optional sub-decisions.".to_string(),
            category: TemplateCategory::Engineering,
            icon: None,
            template_kind: "decision-map".to_string(),
        },
        recipe: TemplateRecipe { frames, shapes, edges },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(270.0, 178.0)),
        },
        exports: TemplateExports {
            allowed: vec![ExportType::Mermaid, ExportType::Madr, ExportType::DesignDocMd],
            default: Some(ExportType::Mermaid),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tag-viable", "viable", "#22c55e", Some("Option is viable")),
                tag("tag-conditional", "conditional", "#f59e0b", Some("Option is conditional")),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some("A decision map: one decision point, competing options, optional sub-decisions. Edges show choice and decomposition.".to_string()),
            field_hints: None,
            suggested_operations: None,
        }),
    }
}

// ===========================================================================
// Builtin: server-architecture (adrArchitecture.ts)
// ===========================================================================

pub fn server_architecture_template() -> TemplateContract {
    let frames = vec![
        RecipeFrame {
            summary: Some("Component diagram with deployment zones.".to_string()),
            meta: Some(meta_map(&[("templateKind", json!("server-architecture"))])),
            ..frame("root", "Server Architecture")
        },
        RecipeFrame {
            parent_local_id: Some("root".to_string()),
            summary: Some("Edge / ingress tier — load balancers, API gateways, CDN.".to_string()),
            meta: Some(meta_map(&[("semanticType", json!("zone"))])),
            ..frame("zone-edge", "Edge")
        },
        RecipeFrame {
            parent_local_id: Some("root".to_string()),
            summary: Some("Service tier — application servers, workers.".to_string()),
            meta: Some(meta_map(&[("semanticType", json!("zone"))])),
            ..frame("zone-service", "Service")
        },
        RecipeFrame {
            parent_local_id: Some("root".to_string()),
            summary: Some("Data tier — databases, caches, message queues.".to_string()),
            meta: Some(meta_map(&[("semanticType", json!("zone"))])),
            ..frame("zone-data", "Data")
        },
    ];
    let comp = |local_id: &str, frame_local_id: &str, title: &str, body: &str, pos: WorldPoint|
     -> RecipeShape {
        adr_shape(local_id, frame_local_id, title, body, "artifact", pos, "component", None)
    };
    let shapes = vec![
        comp("component-gateway", "zone-edge", "API Gateway",
             "Entry point for all inbound requests.", point(0.0, 0.0)),
        comp("component-server", "zone-service", "App Server",
             "Core application logic and request handling.", point(400.0, 0.0)),
        comp("component-worker", "zone-service", "Worker",
             "Background job processing.", point(400.0, 260.0)),
        comp("component-db", "zone-data", "Database",
             "Primary persistent store.", point(800.0, 0.0)),
        comp("component-cache", "zone-data", "Cache",
             "In-memory cache for hot reads.", point(800.0, 260.0)),
    ];
    let edges = vec![
        adr_edge("e1", "component-gateway", "component-server", "HTTP/REST", "depends_on"),
        adr_edge("e2", "component-server", "component-db", "SQL", "depends_on"),
        adr_edge("e3", "component-server", "component-cache", "Redis", "depends_on"),
        adr_edge("e4", "component-server", "component-worker", "enqueue", "produces"),
        adr_edge("e5", "component-worker", "component-db", "writes", "produces"),
    ];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "server-architecture".to_string(),
            title: "Server Architecture".to_string(),
            description: "Server architecture diagram — components as boxes, deployment zones as frames, dependencies as labeled edges.".to_string(),
            category: TemplateCategory::Engineering,
            icon: None,
            template_kind: "server-architecture".to_string(),
        },
        recipe: TemplateRecipe { frames, shapes, edges },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(270.0, 178.0)),
        },
        exports: TemplateExports {
            allowed: vec![
                ExportType::Mermaid,
                ExportType::ArchitectureImage,
                ExportType::ImagePrompt,
            ],
            default: Some(ExportType::Mermaid),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tag-edge", "Edge", "#6366f1", Some("Edge / ingress tier")),
                tag("tag-service", "Service", "#22c55e", Some("Service tier")),
                tag("tag-data", "Data", "#f59e0b", Some("Data tier")),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some("A server architecture diagram: boxes are components, frames are deployment zones/tiers, edges are dependencies labeled with the protocol or call.".to_string()),
            field_hints: None,
            suggested_operations: None,
        }),
    }
}

// ===========================================================================
// Builtin: dependency-diagram (adrArchitecture.ts)
// ===========================================================================

pub fn dependency_diagram_template() -> TemplateContract {
    let frames = vec![RecipeFrame {
        summary: Some("Module/service dependency graph.".to_string()),
        meta: Some(meta_map(&[("templateKind", json!("dependency-diagram"))])),
        ..frame("root", "Dependency Diagram")
    }];
    let unit = |local_id: &str, title: &str, body: &str, style_key: &str, pos: WorldPoint|
     -> RecipeShape {
        adr_shape(local_id, "root", title, body, style_key, pos, "unit", None)
    };
    let shapes = vec![
        unit("unit-a", "Unit A", "First unit (module, service, or package).", "artifact", point(0.0, 0.0)),
        unit("unit-b", "Unit B", "Second unit.", "artifact", point(400.0, 0.0)),
        unit("unit-c", "Unit C", "Third unit.", "artifact", point(800.0, 0.0)),
        unit("unit-d", "Unit D", "Fourth unit — may block C.", "task", point(800.0, 260.0)),
    ];
    let edges = vec![
        adr_edge("e1", "unit-a", "unit-b", "depends on", "depends_on"),
        adr_edge("e2", "unit-b", "unit-c", "depends on", "depends_on"),
        adr_edge("e3", "unit-d", "unit-c", "blocks", "blocks"),
    ];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "dependency-diagram".to_string(),
            title: "Dependency Diagram".to_string(),
            description: "Dependency diagram — units (modules, services, packages) with depends-on and blocking edges.".to_string(),
            category: TemplateCategory::Engineering,
            icon: None,
            template_kind: "dependency-diagram".to_string(),
        },
        recipe: TemplateRecipe { frames, shapes, edges },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(270.0, 178.0)),
        },
        exports: TemplateExports {
            allowed: vec![ExportType::Mermaid, ExportType::ArchitectureImage],
            default: Some(ExportType::Mermaid),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tag-blocking", "blocking", "#ef4444", Some("Blocking dependency")),
                tag("tag-external", "external", "#6366f1", Some("External dependency")),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some("A dependency diagram: boxes are units (modules/services/packages), edges are 'depends on'; mark blocking dependencies with a blocks edge.".to_string()),
            field_hints: None,
            suggested_operations: None,
        }),
    }
}

// ===========================================================================
// Builtin: investigation-map (adrArchitecture.ts)
// ===========================================================================

pub fn investigation_map_template() -> TemplateContract {
    let frames = vec![RecipeFrame {
        summary: Some("Question → evidence → hypotheses → findings → next steps.".to_string()),
        meta: Some(meta_map(&[("templateKind", json!("investigation-map"))])),
        ..frame("root", "Investigation Map")
    }];
    let shapes = vec![
        adr_shape("question", "root", "Investigation question", "What are we trying to find out?",
            "proposition", point(0.0, 260.0), "proposition", Some("draft")),
        adr_shape("evidence-1", "root", "Evidence 1", "A concrete observation, log entry, or data point.",
            "evidence", point(360.0, 40.0), "evidence", Some("draft")),
        adr_shape("evidence-2", "root", "Evidence 2", "A second observation or data point.",
            "evidence", point(360.0, 300.0), "evidence", Some("draft")),
        adr_shape("hypothesis-a", "root", "Hypothesis A", "First competing explanation.",
            "option", point(720.0, 40.0), "option", Some("viable")),
        adr_shape("hypothesis-b", "root", "Hypothesis B", "Second competing explanation.",
            "option", point(720.0, 300.0), "option", Some("conditional")),
        adr_shape("finding", "root", "Finding", "Concluded finding from evidence and hypotheses.",
            "decision_point", point(1080.0, 160.0), "decision_point", Some("draft")),
        adr_shape("next-step", "root", "Next step", "Action item driven by the finding.",
            "task", point(1440.0, 160.0), "task", Some("draft")),
    ];
    let edges = vec![
        adr_edge("e1", "question", "hypothesis-a", "considers", "chooses_between"),
        adr_edge("e2", "question", "hypothesis-b", "considers", "chooses_between"),
        adr_edge("e3", "evidence-1", "hypothesis-a", "supports", "supports"),
        adr_edge("e4", "evidence-2", "hypothesis-b", "supports", "supports"),
        adr_edge("e5", "hypothesis-a", "finding", "informs", "depends_on"),
        adr_edge("e6", "finding", "next-step", "drives", "depends_on"),
    ];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "investigation-map".to_string(),
            title: "Investigation Map".to_string(),
            description: "Investigation map — a question, evidence cards, competing hypotheses, findings, and next steps.".to_string(),
            category: TemplateCategory::Engineering,
            icon: None,
            template_kind: "investigation-map".to_string(),
        },
        recipe: TemplateRecipe { frames, shapes, edges },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(270.0, 178.0)),
        },
        exports: TemplateExports {
            allowed: vec![ExportType::DesignDocMd, ExportType::Mermaid, ExportType::AiPlanMd],
            default: Some(ExportType::DesignDocMd),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tag-confirmed", "confirmed", "#22c55e", Some("Hypothesis or finding confirmed")),
                tag("tag-refuted", "refuted", "#ef4444", Some("Hypothesis refuted")),
                tag("tag-open", "open", "#6366f1", Some("Still open / unresolved")),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some("An investigation map: a question, evidence cards, competing hypotheses, findings, and next steps. Edges show which evidence supports which hypothesis and which findings drive next steps.".to_string()),
            field_hints: None,
            suggested_operations: None,
        }),
    }
}

// ===========================================================================
// Builtin: presentation (presentation.ts)
// ===========================================================================

const SLIDE_WIDTH: f64 = 1280.0;
const SLIDE_HEIGHT: f64 = 720.0;
const SLIDE_GAP: f64 = 80.0;
const TITLE_X: f64 = 60.0;
const TITLE_Y: f64 = 40.0;
const TITLE_W: f64 = 1160.0;
const TITLE_H: f64 = 100.0;
const BODY_X: f64 = 60.0;
const BODY_Y: f64 = 180.0;
const BODY_W: f64 = 780.0;
const BODY_H: f64 = 460.0;
const IMAGE_X: f64 = 880.0;
const IMAGE_Y: f64 = 180.0;
const IMAGE_W: f64 = 360.0;
const IMAGE_H: f64 = 460.0;
const NOTE_X: f64 = 0.0;
const NOTE_Y: f64 = SLIDE_HEIGHT + 20.0;
const NOTE_W: f64 = SLIDE_WIDTH;
const NOTE_H: f64 = 120.0;

pub fn presentation_template() -> TemplateContract {
    let frames = vec![
        RecipeFrame {
            summary: Some("Slide deck".to_string()),
            meta: Some(meta_map(&[
                ("templateKind", json!("presentation")),
                ("semanticType", json!("deck")),
            ])),
            ..frame("deck", "Presentation")
        },
        RecipeFrame {
            parent_local_id: Some("deck".to_string()),
            meta: Some(meta_map(&[
                ("templateKind", json!("presentation")),
                ("semanticType", json!("slide")),
                ("slideIndex", json!(0)),
            ])),
            ..frame("slide-1", "Title slide")
        },
        RecipeFrame {
            parent_local_id: Some("deck".to_string()),
            meta: Some(meta_map(&[
                ("templateKind", json!("presentation")),
                ("semanticType", json!("slide")),
                ("slideIndex", json!(1)),
            ])),
            ..frame("slide-2", "Slide 2")
        },
    ];

    let pres_shape = |local_id: &str, frame_local_id: &str, title: &str, summary: &str,
                      style_key: &str, pos: WorldPoint, sz: RecipeSize, meta: ObjectMeta|
     -> RecipeShape {
        RecipeShape {
            local_id: local_id.to_string(),
            frame_local_id: frame_local_id.to_string(),
            title: Some(title.to_string()),
            summary: Some(summary.to_string()),
            detail: None,
            style_key: Some(style_key.to_string()),
            tag_local_ids: None,
            meta: Some(meta),
            position: pos,
            size: Some(sz),
        }
    };
    let pres_meta = |semantic_type: &str| -> ObjectMeta {
        meta_map(&[
            ("templateKind", json!("presentation")),
            ("semanticType", json!(semantic_type)),
        ])
    };
    let image_meta = || -> ObjectMeta {
        meta_map(&[
            ("templateKind", json!("presentation")),
            ("semanticType", json!("slide-image")),
            ("artifactId", Value::Null),
        ])
    };

    let shapes = vec![
        // Slide 1
        pres_shape("s1-title", "slide-1", "Presentation title", "Subtitle or tagline",
            "slide-title", point(TITLE_X, TITLE_Y), size(TITLE_W, TITLE_H), pres_meta("slide-title")),
        pres_shape("s1-body", "slide-1", "Body", "Opening statement\nKey theme\nAgenda overview",
            "slide-body", point(BODY_X, BODY_Y), size(BODY_W, BODY_H), pres_meta("slide-body")),
        pres_shape("s1-image", "slide-1", "Image placeholder", "Replace with artifact image",
            "slide-image", point(IMAGE_X, IMAGE_Y), size(IMAGE_W, IMAGE_H), image_meta()),
        pres_shape("s1-notes", "slide-1", "Speaker notes", "What to say for slide 1…",
            "speaker-note", point(NOTE_X, NOTE_Y), size(NOTE_W, NOTE_H), pres_meta("speaker-note")),
        // Slide 2
        pres_shape("s2-title", "slide-2", "Slide 2 title", "",
            "slide-title", point(SLIDE_WIDTH + SLIDE_GAP + TITLE_X, TITLE_Y), size(TITLE_W, TITLE_H), pres_meta("slide-title")),
        pres_shape("s2-body", "slide-2", "Body", "First bullet\nSecond bullet\nThird bullet",
            "slide-body", point(SLIDE_WIDTH + SLIDE_GAP + BODY_X, BODY_Y), size(BODY_W, BODY_H), pres_meta("slide-body")),
        pres_shape("s2-image", "slide-2", "Image placeholder", "Replace with artifact image",
            "slide-image", point(SLIDE_WIDTH + SLIDE_GAP + IMAGE_X, IMAGE_Y), size(IMAGE_W, IMAGE_H), image_meta()),
        pres_shape("s2-notes", "slide-2", "Speaker notes", "What to say for slide 2…",
            "speaker-note", point(SLIDE_WIDTH + SLIDE_GAP + NOTE_X, NOTE_Y), size(NOTE_W, NOTE_H), pres_meta("speaker-note")),
    ];

    let edges = vec![edge(
        "flow-1-2",
        "deck",
        "s1-title",
        "s2-title",
        "then",
        Some("slide-flow"),
        Some(meta_map(&[
            ("templateKind", json!("presentation")),
            ("semanticType", json!("slide-flow")),
        ])),
    )];

    TemplateContract {
        metadata: TemplateMetadata {
            id: "presentation".to_string(),
            title: "Presentation".to_string(),
            description: "Slide deck template: nested frames per slide with title/body text, image/artifact previews, optional flow connectors, and speaker-note cards.".to_string(),
            category: TemplateCategory::Presentation,
            icon: None,
            template_kind: "presentation".to_string(),
        },
        recipe: TemplateRecipe { frames, shapes, edges },
        layout: RecipeLayout {
            origin: Some(point(0.0, 0.0)),
            default_shape_size: Some(size(TITLE_W, TITLE_H)),
        },
        exports: TemplateExports {
            allowed: vec![ExportType::DesignDocMd, ExportType::ImagePrompt],
            default: Some(ExportType::DesignDocMd),
        },
        tags: TemplateTags {
            suggested: vec![
                tag("tag-draft-slide", "draft-slide", "#f5a623", Some("Slide still in draft")),
                tag("tag-needs-image", "needs-image", "#7ed321", Some("Slide needs an image")),
                tag("tag-final", "final", "#4a9eff", Some("Slide is final")),
            ],
        },
        prompt_hints: Some(TemplatePromptHints {
            system_hint: Some("This is a slide deck. Each child frame is a slide ordered by meta.slideIndex; title/body are text cards (styleKey slide-title/slide-body), speaker-note cards hold narration, image cards reference artifacts via meta.artifactId.".to_string()),
            field_hints: Some(meta_map(&[
                ("slide-title", json!("One short headline.")),
                ("slide-body", json!("2–5 short bullets.")),
                ("speaker-note", json!("Narration, not shown on the slide.")),
                ("slide-image", json!("Caption for the image; set meta.artifactId to a SceneArtifact id.")),
            ])),
            suggested_operations: Some(vec![
                "create".to_string(),
                "edit text".to_string(),
                "connect".to_string(),
                "tag".to_string(),
            ]),
        }),
    }
}

// Catalog & registry
// ---------------------------------------------------------------------------

/// The 6 user-visible templates, in display order.
pub fn catalog() -> Vec<TemplateContract> {
    vec![
        todo_board_template(),
        adr_template(),
        server_architecture_template(),
        wiki_note_template(),
        idea_board_template(),
        presentation_template(),
    ]
}

/// All 9 templates (catalog + decision-map, dependency-diagram, investigation-map).
pub fn registry() -> Vec<TemplateContract> {
    vec![
        todo_board_template(),
        adr_template(),
        server_architecture_template(),
        wiki_note_template(),
        idea_board_template(),
        presentation_template(),
        decision_map_template(),
        dependency_diagram_template(),
        investigation_map_template(),
    ]
}
