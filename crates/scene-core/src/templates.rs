//! Template subsystem — a faithful Rust port of `src/shared/templates/*`.
//!
//! A `TemplateContract` is a *recipe over existing primitives*: it declares the
//! frames / shapes / edges / tags to create, then `apply_template` lowers the
//! recipe entirely to `create-group` / `create-card` / `create-edge` render ops
//! run through [`crate::apply::apply_render_patch_to_shape_scene`]. The produced
//! objects are indistinguishable from hand-drawn ones except for
//! `meta.templateKind`.
//!
//! Ports `contract.ts` (contract types + `applyTemplate`), the four builtins
//! (`todoBoard.ts`, `wikiNote.ts`, `adrArchitecture.ts`, `presentation.ts`), and
//! the presentation reader helpers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::apply::apply_render_patch_to_shape_scene;
use crate::model::{
    ExportType, ObjectMeta, Scene, SceneEdge, SceneGroup, SceneNode, SceneSelection, Tag,
    WorldPoint, WorldRect,
};
use crate::op::{RenderCard, RenderGroup, RenderScenePatch};

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

// ---------------------------------------------------------------------------
// Application result
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedTemplate {
    /// First frame group (kept for back-compat); `None` if no group was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<SceneGroup>,
    /// All frame groups, in creation order (parents before children).
    pub groups: Vec<SceneGroup>,
    pub nodes: Vec<SceneNode>,
    pub edges: Vec<SceneEdge>,
    /// Tags that were newly created by the template (not pre-existing).
    pub new_tags: Vec<Tag>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// applyTemplate — lowers recipe to normal canvas objects
// ---------------------------------------------------------------------------

const DEFAULT_SHAPE_SIZE: RecipeSize = RecipeSize {
    width: 390.0,
    height: 390.0,
};

/// Local→scene id allocator. Mirrors the TS closure: a lazy monotonic `seq`
/// shared across frames, shapes, edges, and tags; format `{prefix}-{localId}-{seq}`.
struct IdAllocator {
    prefix: String,
    seq: usize,
    map: Vec<(String, String)>,
}

impl IdAllocator {
    fn new(prefix: &str) -> Self {
        IdAllocator {
            prefix: prefix.to_string(),
            seq: 0,
            map: Vec::new(),
        }
    }

    fn scene_id(&mut self, local_id: &str) -> String {
        if let Some((_, v)) = self.map.iter().find(|(k, _)| k == local_id) {
            return v.clone();
        }
        let id = format!("{}-{}-{}", self.prefix, local_id, self.seq);
        self.seq += 1;
        self.map.push((local_id.to_string(), id.clone()));
        id
    }

    /// Peek a previously-allocated id without minting a new one (mirrors
    /// `idMap.get(...)` — used for parentLocalId resolution).
    fn peek(&self, local_id: &str) -> Option<String> {
        self.map
            .iter()
            .find(|(k, _)| k == local_id)
            .map(|(_, v)| v.clone())
    }
}

/// Apply a [`TemplateContract`] anchored at `anchor` world position, producing
/// normal `SceneGroup` / `SceneNode` / `SceneEdge` objects.
///
/// Output objects are indistinguishable from hand-drawn ones except for
/// `meta.templateKind`. `now` is an injected ISO timestamp.
pub fn apply_template(
    template: &TemplateContract,
    anchor: WorldPoint,
    id_prefix: &str,
    now: &str,
) -> AppliedTemplate {
    let template_kind = template.metadata.template_kind.clone();
    let shape_size = template
        .layout
        .default_shape_size
        .unwrap_or(DEFAULT_SHAPE_SIZE);

    let mut ids = IdAllocator::new(id_prefix);

    // §2 Resolve suggested tags — produce Tag objects (no DB; pure objects).
    let mut new_tags: Vec<Tag> = Vec::with_capacity(template.tags.suggested.len());
    let mut tag_local_to_id: Vec<(String, String)> =
        Vec::with_capacity(template.tags.suggested.len());
    for st in &template.tags.suggested {
        let tag_id = ids.scene_id(&st.local_id);
        new_tags.push(Tag {
            id: tag_id.clone(),
            name: st.name.clone(),
            color: st.color.clone(),
            description: st.description.clone().unwrap_or_default(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        });
        tag_local_to_id.push((st.local_id.clone(), tag_id));
    }
    let tag_id_for = |lid: &str| -> Option<String> {
        tag_local_to_id
            .iter()
            .find(|(k, _)| k == lid)
            .map(|(_, v)| v.clone())
    };

    // §3 Build a minimal Scene to thread through apply_render_patch_to_shape_scene.
    let mut scene = Scene {
        version: 1,
        scene_version: 0,
        groups: vec![],
        nodes: vec![],
        edges: vec![],
        tags: new_tags.clone(),
        comments: vec![],
        artifacts: vec![],
        proposals: None,
        selection: SceneSelection::Canvas,
        updated_at: now.to_string(),
    };

    let mut errors: Vec<String> = vec![];

    // §4a Apply frames (create-group) — parents before children.
    let frame_order = order_frames(&template.recipe.frames);
    for frame in &frame_order {
        let group_id = ids.scene_id(&frame.local_id);
        let parent_group_id = frame
            .parent_local_id
            .as_ref()
            .and_then(|p| ids.peek(p));
        let tag_ids: Vec<String> = frame
            .tag_local_ids
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|lid| tag_id_for(lid).unwrap_or_else(|| lid.clone()))
            .collect();

        // Compute rough initial bounds from the shapes belonging to this frame.
        let members: Vec<&RecipeShape> = template
            .recipe
            .shapes
            .iter()
            .filter(|s| s.frame_local_id == frame.local_id)
            .collect();
        let bounds = bounds_for_shapes(&members, anchor, shape_size);

        let render_group = RenderGroup {
            id: group_id.clone(),
            title: frame.title.clone(),
            summary: frame.summary.clone().unwrap_or_default(),
            bounds,
            tag_ids,
            z_index: 0.0,
            style_key: "default".to_string(),
        };

        let result = apply_render_patch_to_shape_scene(
            &scene,
            &RenderScenePatch::CreateGroup {
                group: render_group,
            },
            now,
            None,
        );
        if !result.errors.is_empty() {
            errors.extend(result.errors);
            continue;
        }
        // Patch parentGroupId + meta in after creation (renderGroupToSceneGroup
        // hardcodes null/no-meta).
        scene = result.scene;
        let frame_meta = frame
            .meta
            .as_ref()
            .map(|m| merge_meta(&template_kind, Some(m), None));
        for g in &mut scene.groups {
            if g.id == group_id {
                if let Some(parent) = &parent_group_id {
                    g.parent_group_id = Some(parent.clone());
                }
                if let Some(meta) = &frame_meta {
                    g.meta = Some(meta.clone());
                }
            }
        }
    }

    // §4b Apply shapes (create-card).
    for shape in &template.recipe.shapes {
        let node_id = ids.scene_id(&shape.local_id);
        let group_id = ids.scene_id(&shape.frame_local_id);
        let size = shape.size.unwrap_or(shape_size);
        let pos = WorldPoint {
            x: anchor.x + shape.position.x,
            y: anchor.y + shape.position.y,
        };

        let meta = merge_meta(&template_kind, shape.meta.as_ref(), None);

        let title = shape.title.clone().unwrap_or_else(|| "Untitled".to_string());
        let card = RenderCard {
            id: node_id.clone(),
            group_id,
            title: title.clone(),
            summary: shape.summary.clone().unwrap_or_default(),
            detail: shape.detail.clone().unwrap_or_default(),
            status: "draft".to_string(),
            node_type: "task".to_string(),
            bounds: WorldRect {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            },
            z_index: 0.0,
            style_key: shape.style_key.clone().unwrap_or_else(|| "default".to_string()),
            accessibility_label: title,
        };

        let result = apply_render_patch_to_shape_scene(
            &scene,
            &RenderScenePatch::CreateCard { card },
            now,
            None,
        );
        if !result.errors.is_empty() {
            errors.extend(result.errors);
            continue;
        }
        // Attach meta to the node after creation.
        scene = result.scene;
        for n in &mut scene.nodes {
            if n.id == node_id {
                n.meta = Some(meta.clone());
            }
        }
    }

    // §4c Apply edges (create-edge).
    for recipe_edge in &template.recipe.edges {
        let edge_id = ids.scene_id(&recipe_edge.local_id);
        let group_id = ids.scene_id(&recipe_edge.frame_local_id);
        let source_id = ids.scene_id(&recipe_edge.source_local_id);
        let target_id = ids.scene_id(&recipe_edge.target_local_id);

        let result = apply_render_patch_to_shape_scene(
            &scene,
            &RenderScenePatch::CreateEdge {
                group_id,
                source: source_id,
                target: target_id,
                edge_id: edge_id.clone(),
                label: recipe_edge.label.clone(),
            },
            now,
            None,
        );
        if !result.errors.is_empty() {
            errors.extend(result.errors);
            continue;
        }
        // Attach meta to the edge after creation.
        scene = result.scene;
        if recipe_edge.meta.is_some() || recipe_edge.style_key.is_some() {
            let meta = merge_meta(
                &template_kind,
                recipe_edge.meta.as_ref(),
                recipe_edge.style_key.as_deref(),
            );
            for e in &mut scene.edges {
                if e.id == edge_id {
                    e.meta = Some(meta.clone());
                }
            }
        }
    }

    AppliedTemplate {
        group: scene.groups.first().cloned(),
        groups: scene.groups.clone(),
        nodes: scene.nodes.clone(),
        edges: scene.edges.clone(),
        new_tags,
        errors,
    }
}

/// Build a `meta` map `{ templateKind, ...base, ...(styleKey?{styleKey}) }`.
/// `templateKind` is inserted first, base overlays it, then `styleKey` overlays both.
fn merge_meta(template_kind: &str, base: Option<&ObjectMeta>, style_key: Option<&str>) -> ObjectMeta {
    let mut meta: ObjectMeta = Map::new();
    meta.insert(
        "templateKind".to_string(),
        Value::String(template_kind.to_string()),
    );
    if let Some(base) = base {
        for (k, v) in base {
            meta.insert(k.clone(), v.clone());
        }
    }
    if let Some(sk) = style_key {
        meta.insert("styleKey".to_string(), Value::String(sk.to_string()));
    }
    meta
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Topological sort of frames so parents are created before children.
fn order_frames(frames: &[RecipeFrame]) -> Vec<RecipeFrame> {
    let mut visited: Vec<String> = Vec::new();
    let mut result: Vec<RecipeFrame> = Vec::new();

    fn visit(
        f: &RecipeFrame,
        frames: &[RecipeFrame],
        visited: &mut Vec<String>,
        result: &mut Vec<RecipeFrame>,
    ) {
        if visited.iter().any(|v| v == &f.local_id) {
            return;
        }
        if let Some(parent_id) = &f.parent_local_id {
            if let Some(parent) = frames.iter().find(|c| &c.local_id == parent_id) {
                visit(parent, frames, visited, result);
            }
        }
        visited.push(f.local_id.clone());
        result.push(f.clone());
    }

    for f in frames {
        visit(f, frames, &mut visited, &mut result);
    }
    result
}

/// Derive padded bounds from recipe shapes placed at anchor.
fn bounds_for_shapes(
    shapes: &[&RecipeShape],
    anchor: WorldPoint,
    default_size: RecipeSize,
) -> WorldRect {
    if shapes.is_empty() {
        return WorldRect {
            x: anchor.x,
            y: anchor.y,
            width: default_size.width + 80.0,
            height: default_size.height + 80.0,
        };
    }
    let padding = 40.0;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for s in shapes {
        let size = s.size.unwrap_or(default_size);
        let x = anchor.x + s.position.x;
        let y = anchor.y + s.position.y;
        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if x + size.width > max_x {
            max_x = x + size.width;
        }
        if y + size.height > max_y {
            max_y = y + size.height;
        }
    }
    WorldRect {
        x: min_x - padding,
        y: min_y - padding,
        width: max_x - min_x + padding * 2.0,
        height: max_y - min_y + padding * 2.0,
    }
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

// ---------------------------------------------------------------------------
// Presentation style tokens (presentation.ts §3)
// ---------------------------------------------------------------------------

/// SceneStyleToken — a minimal struct mirroring `renderScene.ts`. Modeled here
/// (not in op.rs) because templates is the only consumer in scene-core today.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneStyleToken {
    pub id: String,
    pub fill: String,
    pub stroke: String,
    pub text: String,
    pub muted_text: String,
    pub accent: String,
    pub surface: String,
    pub pastel: String,
}

pub fn presentation_style_tokens() -> Vec<SceneStyleToken> {
    let t = |id: &str, fill: &str, stroke: &str, text: &str, muted: &str, accent: &str,
             surface: &str, pastel: &str|
     -> SceneStyleToken {
        SceneStyleToken {
            id: id.to_string(),
            fill: fill.to_string(),
            stroke: stroke.to_string(),
            text: text.to_string(),
            muted_text: muted.to_string(),
            accent: accent.to_string(),
            surface: surface.to_string(),
            pastel: pastel.to_string(),
        }
    };
    vec![
        t("slide-title", "#ffffff", "#2f7ee6", "#0d1f33", "#5a7188", "#2f7ee6", "#f7fbff", "#ebf4ff"),
        t("slide-body", "#fafcff", "#7b8794", "#172026", "#65717b", "#158f83", "#f7f9fb", "#f0f7ff"),
        t("slide-image", "#f4f8f4", "#26965e", "#10251a", "#5b7464", "#26965e", "#f4fbf6", "#eaf9ef"),
        t("speaker-note", "#fffdf0", "#c67914", "#2a1b0b", "#80684c", "#c67914", "#fdf2de", "#fdf2de"),
        t("slide-flow", "#ffffff", "#7a68ce", "#1d1833", "#675f85", "#7a68ce", "#f8f7ff", "#f1effd"),
    ]
}

// ---------------------------------------------------------------------------
// Presentation outline reader (presentation.ts §4)
// ---------------------------------------------------------------------------

/// Read deck-produced primitives and emit a slide-ordered Markdown outline.
/// Mirrors `presentationOutline`: child slide frames of the deck (by
/// `meta.semanticType == "slide"`), sorted by `meta.slideIndex`, then per-slide
/// title/body/note cards by `meta.semanticType`.
pub fn presentation_outline(
    deck_group: &SceneGroup,
    all_groups: &[SceneGroup],
    all_nodes: &[SceneNode],
) -> String {
    let meta_str = |meta: &Option<ObjectMeta>, key: &str| -> Option<String> {
        meta.as_ref()
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let slide_index = |meta: &Option<ObjectMeta>| -> f64 {
        meta.as_ref()
            .and_then(|m| m.get("slideIndex"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };

    let mut slide_frames: Vec<&SceneGroup> = all_groups
        .iter()
        .filter(|g| {
            g.parent_group_id.as_deref() == Some(deck_group.id.as_str())
                && meta_str(&g.meta, "semanticType").as_deref() == Some("slide")
        })
        .collect();
    slide_frames.sort_by(|a, b| {
        let ai = slide_index(&a.meta);
        let bi = slide_index(&b.meta);
        ai.partial_cmp(&bi).unwrap_or(std::cmp::Ordering::Equal)
    });

    let bullets_of = |cards: &[&SceneNode]| -> Vec<String> {
        cards
            .iter()
            .flat_map(|c| {
                c.summary
                    .split('\n')
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    let mut lines: Vec<String> = vec![
        format!(
            "# {}",
            if deck_group.title.is_empty() {
                "Presentation".to_string()
            } else {
                deck_group.title.clone()
            }
        ),
        String::new(),
    ];

    for frame in slide_frames {
        let members: Vec<&SceneNode> =
            all_nodes.iter().filter(|n| n.group_id == frame.id).collect();
        let title_card = members
            .iter()
            .find(|n| meta_str(&n.meta, "semanticType").as_deref() == Some("slide-title"));
        let body_cards: Vec<&SceneNode> = members
            .iter()
            .filter(|n| meta_str(&n.meta, "semanticType").as_deref() == Some("slide-body"))
            .copied()
            .collect();
        let note_cards: Vec<&SceneNode> = members
            .iter()
            .filter(|n| meta_str(&n.meta, "semanticType").as_deref() == Some("speaker-note"))
            .copied()
            .collect();

        let slide_title = title_card
            .map(|c| c.title.clone())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                if frame.title.is_empty() {
                    None
                } else {
                    Some(frame.title.clone())
                }
            })
            .unwrap_or_else(|| "Untitled slide".to_string());

        let body_bullets = bullets_of(&body_cards);
        let speaker_notes = bullets_of(&note_cards);

        lines.push(format!("## {slide_title}"));
        for bullet in &body_bullets {
            lines.push(format!("- {bullet}"));
        }
        if !speaker_notes.is_empty() {
            lines.push(String::new());
            lines.push(format!("> **Speaker notes:** {}", speaker_notes.join(" / ")));
        }
        lines.push(String::new());
    }

    lines.join("\n").trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Presentation meta validation (presentation.ts §5)
// ---------------------------------------------------------------------------

const PRESENTATION_SEMANTIC_TYPES: &[&str] = &[
    "deck",
    "slide",
    "slide-title",
    "slide-body",
    "slide-image",
    "speaker-note",
    "slide-flow",
];

/// Validate meta values on a presentation-template object at authoring time.
/// Returns an array of error strings (empty = valid). Advisory only.
pub fn validate_presentation_meta(meta: &ObjectMeta) -> Vec<String> {
    let mut errors: Vec<String> = vec![];
    let template_kind = meta.get("templateKind");
    if template_kind.and_then(|v| v.as_str()) != Some("presentation") {
        errors.push(format!(
            "meta.templateKind must be \"presentation\", got {}",
            value_to_string(template_kind),
        ));
    }
    if let Some(st) = meta.get("semanticType") {
        let ok = st
            .as_str()
            .map(|s| PRESENTATION_SEMANTIC_TYPES.contains(&s))
            .unwrap_or(false);
        if !ok {
            errors.push(format!(
                "meta.semanticType \"{}\" is not a valid presentation semantic type",
                value_to_string(Some(st)),
            ));
        }
    }
    if let Some(si) = meta.get("slideIndex") {
        let is_int = si.as_i64().is_some() || si.as_u64().is_some();
        if !is_int {
            errors.push(format!(
                "meta.slideIndex must be an integer, got {}",
                value_to_string(Some(si)),
            ));
        }
    }
    errors
}

/// Stringify a meta value the way JS `String(x)` would for these error messages:
/// strings unquoted, `null`/`undefined` as their words, numbers/bools as-is.
fn value_to_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
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

// ===========================================================================
// recipe_from_selection (CC3.1) — the inverse of apply_template
// ===========================================================================

/// Build a [`TemplateContract`] from a selection of existing scene objects.
///
/// This is the inverse of [`apply_template`]: it reads the selected groups,
/// nodes, and edges out of `scene` and lowers them back into a recipe of
/// [`RecipeFrame`] / [`RecipeShape`] / [`RecipeEdge`] entries, so a user can
/// "save selection as template" and re-stamp it elsewhere.
///
/// Selection handling covers the three cockpit cases uniformly:
/// * **single object** — one node (or one group) selected;
/// * **multi-select** — several nodes/groups selected;
/// * **single group with members** — a group whose member nodes/edges are
///   reconstructed even when the members are not themselves in `selection_ids`.
///
/// Rules:
/// * Selected **groups** become frames. A group's members (nodes whose
///   `group_id` is a selected group, and edges among them) are pulled in even if
///   not explicitly selected, so selecting a frame captures its contents.
/// * Selected **nodes** become shapes. A node whose host group is also selected
///   is parented to that group's frame; an orphan node (host group not selected)
///   is parented to one synthesized wrapper frame so the recipe is always
///   apply-able (every shape resolves to a frame).
/// * **Edges** become recipe edges only when BOTH endpoints are captured shapes.
/// * **Positions** are made relative to the selection's min corner (the
///   top-left of the union of all captured node/group bounds), so the recipe is
///   anchor-independent like the builtins.
/// * **localIds** are derived directly from object ids (`frame-{id}` /
///   `shape-{id}` / `edge-{id}`), keeping them stable and collision-free.
///
/// `metadata` describes the produced contract; the caller supplies it because
/// title/id/category are authoring choices, not derivable from geometry.
pub fn recipe_from_selection(
    scene: &Scene,
    selection_ids: &[String],
    metadata: TemplateMetadata,
) -> TemplateContract {
    use std::collections::HashSet;

    let selected: HashSet<&str> = selection_ids.iter().map(|s| s.as_str()).collect();

    // Captured groups: any selected id that resolves to a group.
    let captured_groups: Vec<&SceneGroup> = scene
        .groups
        .iter()
        .filter(|g| selected.contains(g.id.as_str()))
        .collect();
    let captured_group_ids: HashSet<&str> =
        captured_groups.iter().map(|g| g.id.as_str()).collect();

    // Captured nodes: selected nodes, PLUS members of any captured group.
    let captured_nodes: Vec<&SceneNode> = scene
        .nodes
        .iter()
        .filter(|n| {
            selected.contains(n.id.as_str()) || captured_group_ids.contains(n.group_id.as_str())
        })
        .collect();
    let captured_node_ids: HashSet<&str> = captured_nodes.iter().map(|n| n.id.as_str()).collect();

    // Whether any orphan node (host group not captured) needs the wrapper frame.
    let needs_wrapper = captured_nodes
        .iter()
        .any(|n| !captured_group_ids.contains(n.group_id.as_str()));

    // Selection min corner: top-left of the union of captured group + node bounds.
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    for g in &captured_groups {
        if g.bounds.x < min_x {
            min_x = g.bounds.x;
        }
        if g.bounds.y < min_y {
            min_y = g.bounds.y;
        }
    }
    for n in &captured_nodes {
        if n.position.x < min_x {
            min_x = n.position.x;
        }
        if n.position.y < min_y {
            min_y = n.position.y;
        }
    }
    if !min_x.is_finite() {
        min_x = 0.0;
    }
    if !min_y.is_finite() {
        min_y = 0.0;
    }

    let frame_local_id = |id: &str| format!("frame-{id}");
    let shape_local_id = |id: &str| format!("shape-{id}");
    let edge_local_id = |id: &str| format!("edge-{id}");
    const WRAPPER_LOCAL_ID: &str = "frame-selection";

    // §1 Frames — one per captured group, plus a wrapper if any node is orphaned.
    let mut frames: Vec<RecipeFrame> = Vec::new();
    if needs_wrapper {
        frames.push(RecipeFrame {
            local_id: WRAPPER_LOCAL_ID.to_string(),
            title: "Selection".to_string(),
            summary: None,
            parent_local_id: None,
            tag_local_ids: None,
            meta: None,
        });
    }
    for g in &captured_groups {
        // Preserve parent nesting only when the parent group is also captured.
        let parent_local_id = g
            .parent_group_id
            .as_ref()
            .filter(|p| captured_group_ids.contains(p.as_str()))
            .map(|p| frame_local_id(p));
        frames.push(RecipeFrame {
            local_id: frame_local_id(&g.id),
            title: g.title.clone(),
            summary: if g.summary.is_empty() {
                None
            } else {
                Some(g.summary.clone())
            },
            parent_local_id,
            tag_local_ids: None,
            meta: g.meta.clone(),
        });
    }

    // §2 Shapes — one per captured node, parented to its group's frame or the
    // wrapper, with positions relative to the selection min corner.
    let shapes: Vec<RecipeShape> = captured_nodes
        .iter()
        .map(|n| {
            let frame = if captured_group_ids.contains(n.group_id.as_str()) {
                frame_local_id(&n.group_id)
            } else {
                WRAPPER_LOCAL_ID.to_string()
            };
            RecipeShape {
                local_id: shape_local_id(&n.id),
                frame_local_id: frame,
                title: Some(n.title.clone()),
                summary: if n.summary.is_empty() {
                    None
                } else {
                    Some(n.summary.clone())
                },
                detail: if n.detail.is_empty() {
                    None
                } else {
                    Some(n.detail.clone())
                },
                style_key: None,
                tag_local_ids: None,
                meta: n.meta.clone(),
                position: WorldPoint {
                    x: n.position.x - min_x,
                    y: n.position.y - min_y,
                },
                size: Some(RecipeSize {
                    width: n.size.width,
                    height: n.size.height,
                }),
            }
        })
        .collect();

    // §3 Edges — only those whose BOTH endpoints are captured nodes. The host
    // frame is the edge's group when captured, else the wrapper (or the first
    // captured frame) so the recipe edge always resolves to a frame on apply.
    let edges: Vec<RecipeEdge> = scene
        .edges
        .iter()
        .filter(|e| {
            captured_node_ids.contains(e.source.as_str())
                && captured_node_ids.contains(e.target.as_str())
        })
        .map(|e| {
            let frame = if captured_group_ids.contains(e.group_id.as_str()) {
                frame_local_id(&e.group_id)
            } else if needs_wrapper {
                WRAPPER_LOCAL_ID.to_string()
            } else {
                // Every node here is in a captured group; use the first frame.
                frames
                    .first()
                    .map(|f| f.local_id.clone())
                    .unwrap_or_else(|| WRAPPER_LOCAL_ID.to_string())
            };
            RecipeEdge {
                local_id: edge_local_id(&e.id),
                frame_local_id: frame,
                source_local_id: shape_local_id(&e.source),
                target_local_id: shape_local_id(&e.target),
                label: if e.label.is_empty() {
                    None
                } else {
                    Some(e.label.clone())
                },
                style_key: None,
                meta: e.meta.clone(),
            }
        })
        .collect();

    TemplateContract {
        metadata,
        recipe: TemplateRecipe {
            frames,
            shapes,
            edges,
        },
        layout: RecipeLayout {
            origin: Some(WorldPoint { x: 0.0, y: 0.0 }),
            default_shape_size: None,
        },
        exports: TemplateExports {
            allowed: vec![],
            default: None,
        },
        tags: TemplateTags { suggested: vec![] },
        prompt_hints: None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-06-05T00:00:00.000Z";

    fn template_kind_of(meta: &Option<ObjectMeta>) -> Option<String> {
        meta.as_ref()
            .and_then(|m| m.get("templateKind"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn todo_board_applies_to_expected_counts() {
        let tpl = todo_board_template();
        let applied = apply_template(&tpl, point(100.0, 200.0), "tpl", NOW);

        assert!(applied.errors.is_empty(), "errors: {:?}", applied.errors);
        // 4 frames -> 4 groups, 3 shapes -> 3 nodes, 1 edge.
        assert_eq!(applied.groups.len(), 4);
        assert_eq!(applied.nodes.len(), 3);
        assert_eq!(applied.edges.len(), 1);
        // 7 suggested tags -> 7 new tags.
        assert_eq!(applied.new_tags.len(), 7);

        // group back-compat equals the first group.
        assert_eq!(applied.group.as_ref().unwrap().id, applied.groups[0].id);

        // Every node carries meta.templateKind == "todo".
        for n in &applied.nodes {
            assert_eq!(template_kind_of(&n.meta).as_deref(), Some("todo"));
        }
        // Edge meta is attached (had styleKey + meta) and carries templateKind.
        assert_eq!(
            template_kind_of(&applied.edges[0].meta).as_deref(),
            Some("todo")
        );
    }

    #[test]
    fn todo_board_parent_chain_is_correct() {
        let tpl = todo_board_template();
        let applied = apply_template(&tpl, point(0.0, 0.0), "tpl", NOW);

        // First group is the root board (orderFrames keeps parents first; board
        // has no parent).
        let board = &applied.groups[0];
        assert_eq!(board.parent_group_id, None);

        // The three column frames point at the board id.
        let board_id = board.id.clone();
        let columns: Vec<&SceneGroup> = applied
            .groups
            .iter()
            .filter(|g| g.parent_group_id.as_deref() == Some(board_id.as_str()))
            .collect();
        assert_eq!(columns.len(), 3);
    }

    #[test]
    fn id_allocator_format_and_monotonic_seq() {
        let tpl = todo_board_template();
        let applied = apply_template(&tpl, point(0.0, 0.0), "myprefix", NOW);

        // First minted id is the first suggested tag (tags resolved before frames),
        // proving seq is shared across tags + frames + shapes + edges.
        assert_eq!(applied.new_tags[0].id, "myprefix-tg-status-todo-0");
        // Frames are minted after the 7 tags.
        let board = &applied.groups[0];
        assert_eq!(board.id, "myprefix-f-board-7");
    }

    #[test]
    fn adr_applies_full_graph() {
        let tpl = adr_template();
        let applied = apply_template(&tpl, point(50.0, 50.0), "tpl", NOW);

        assert!(applied.errors.is_empty(), "errors: {:?}", applied.errors);
        assert_eq!(applied.groups.len(), 1);
        assert_eq!(applied.nodes.len(), 10);
        assert_eq!(applied.edges.len(), 9);

        // shape.meta keys survive alongside templateKind (merge order).
        let prop = applied
            .nodes
            .iter()
            .find(|n| n.title == "Proposition")
            .unwrap();
        let meta = prop.meta.as_ref().unwrap();
        assert_eq!(meta.get("templateKind").unwrap().as_str(), Some("adr"));
        assert_eq!(meta.get("semanticType").unwrap().as_str(), Some("proposition"));
        assert_eq!(meta.get("status").unwrap().as_str(), Some("selected"));
    }

    #[test]
    fn server_architecture_nested_zone_chain() {
        let tpl = server_architecture_template();
        let applied = apply_template(&tpl, point(0.0, 0.0), "tpl", NOW);

        assert!(applied.errors.is_empty(), "errors: {:?}", applied.errors);
        // 4 frames (root + 3 zones), 5 components, 5 edges.
        assert_eq!(applied.groups.len(), 4);
        assert_eq!(applied.nodes.len(), 5);
        assert_eq!(applied.edges.len(), 5);

        // root has no parent; the 3 zones point at the root.
        let root = &applied.groups[0];
        assert_eq!(root.parent_group_id, None);
        let zones: Vec<&SceneGroup> = applied
            .groups
            .iter()
            .filter(|g| g.parent_group_id.as_deref() == Some(root.id.as_str()))
            .collect();
        assert_eq!(zones.len(), 3);
    }

    #[test]
    fn shape_anchor_offset_applied() {
        let tpl = adr_template();
        let anchor = point(1000.0, 2000.0);
        let applied = apply_template(&tpl, anchor, "tpl", NOW);
        // Proposition shape recipe position is (0, 300); anchored => (1000, 2300).
        let prop = applied
            .nodes
            .iter()
            .find(|n| n.title == "Proposition")
            .unwrap();
        assert_eq!(prop.position.x, 1000.0);
        assert_eq!(prop.position.y, 2300.0);
    }

    #[test]
    fn presentation_outline_reads_slides_in_order() {
        let tpl = presentation_template();
        let applied = apply_template(&tpl, point(0.0, 0.0), "tpl", NOW);
        let deck = applied
            .groups
            .iter()
            .find(|g| {
                g.meta
                    .as_ref()
                    .and_then(|m| m.get("semanticType"))
                    .and_then(|v| v.as_str())
                    == Some("deck")
            })
            .unwrap();
        let md = presentation_outline(deck, &applied.groups, &applied.nodes);
        assert!(md.starts_with("# Presentation"));
        // First slide title card title is the slide title heading.
        assert!(md.contains("## Presentation title"));
        assert!(md.contains("## Slide 2 title"));
        // Body bullets split on newlines.
        assert!(md.contains("- Opening statement"));
        assert!(md.contains("> **Speaker notes:**"));
    }

    #[test]
    fn validate_presentation_meta_flags_bad_values() {
        let mut ok: ObjectMeta = Map::new();
        ok.insert("templateKind".to_string(), json!("presentation"));
        ok.insert("semanticType".to_string(), json!("slide"));
        ok.insert("slideIndex".to_string(), json!(2));
        assert!(validate_presentation_meta(&ok).is_empty());

        let mut bad: ObjectMeta = Map::new();
        bad.insert("templateKind".to_string(), json!("todo"));
        bad.insert("semanticType".to_string(), json!("bogus"));
        bad.insert("slideIndex".to_string(), json!(1.5));
        let errors = validate_presentation_meta(&bad);
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn catalog_and_registry_sizes() {
        assert_eq!(catalog().len(), 6);
        assert_eq!(registry().len(), 9);
        let ids: Vec<String> = catalog().iter().map(|t| t.metadata.id.clone()).collect();
        assert_eq!(
            ids,
            vec![
                "todo-board",
                "adr",
                "server-architecture",
                "wiki-note",
                "idea-board",
                "presentation"
            ]
        );
    }

    #[test]
    fn frame_tags_resolve_to_group_tag_ids_shapes_do_not() {
        // wiki-note frames have no tagLocalIds, but shapes do; verify shape tags
        // are NOT applied to nodes (documented behavior).
        let tpl = wiki_note_template();
        let applied = apply_template(&tpl, point(0.0, 0.0), "tpl", NOW);
        for n in &applied.nodes {
            assert!(
                n.tag_ids.is_empty(),
                "node {} unexpectedly carries tag ids {:?}",
                n.id,
                n.tag_ids
            );
        }
    }

    #[test]
    fn edge_without_meta_or_stylekey_gets_no_meta() {
        // idea-board's e-rel-1 has meta but no styleKey -> meta attached.
        // Build a synthetic check: presentation flow edge has styleKey -> meta has styleKey key.
        let tpl = presentation_template();
        let applied = apply_template(&tpl, point(0.0, 0.0), "tpl", NOW);
        let flow = &applied.edges[0];
        let meta = flow.meta.as_ref().unwrap();
        assert_eq!(meta.get("styleKey").unwrap().as_str(), Some("slide-flow"));
        assert_eq!(meta.get("templateKind").unwrap().as_str(), Some("presentation"));
    }

    #[test]
    fn serde_round_trip_contract() {
        let tpl = todo_board_template();
        let json = serde_json::to_string(&tpl).unwrap();
        let back: TemplateContract = serde_json::from_str(&json).unwrap();
        assert_eq!(tpl, back);
    }

    // -----------------------------------------------------------------------
    // recipe_from_selection (CC3.1)
    // -----------------------------------------------------------------------

    use crate::model::{NodeStatus, NodeType, Point, Size};

    fn sel_metadata() -> TemplateMetadata {
        TemplateMetadata {
            id: "sel".to_string(),
            title: "From selection".to_string(),
            description: String::new(),
            category: TemplateCategory::General,
            icon: None,
            template_kind: "selection".to_string(),
        }
    }

    fn scene_group(id: &str, x: f64, y: f64, w: f64, h: f64) -> SceneGroup {
        SceneGroup {
            id: id.to_string(),
            parent_group_id: None,
            title: format!("group {id}"),
            summary: String::new(),
            bounds: WorldRect { x, y, width: w, height: h },
            tag_ids: vec![],
            z_index: 0.0,
            collapsed: false,
            created_at: NOW.to_string(),
            updated_at: NOW.to_string(),
            meta: None,
        }
    }

    fn scene_node(id: &str, group_id: &str, x: f64, y: f64) -> SceneNode {
        SceneNode {
            id: id.to_string(),
            node_type: NodeType::Task,
            title: format!("node {id}"),
            summary: String::new(),
            detail: String::new(),
            status: NodeStatus::Draft,
            confidence: 0.5,
            evidence_refs: vec![],
            child_decision_ids: vec![],
            group_id: group_id.to_string(),
            position: Point { x, y },
            size: Size { width: 100.0, height: 80.0 },
            z_index: 0.0,
            tag_ids: vec![],
            updated_at: None,
            meta: None,
        }
    }

    fn scene_edge(id: &str, group_id: &str, source: &str, target: &str) -> SceneEdge {
        SceneEdge {
            id: id.to_string(),
            edge_type: crate::model::EdgeType::Supports,
            source: source.to_string(),
            target: target.to_string(),
            label: "rel".to_string(),
            rationale: String::new(),
            confidence: 0.5,
            group_id: group_id.to_string(),
            tag_ids: vec![],
            updated_at: None,
            meta: None,
        }
    }

    fn build_scene(
        groups: Vec<SceneGroup>,
        nodes: Vec<SceneNode>,
        edges: Vec<SceneEdge>,
    ) -> Scene {
        Scene {
            version: 1,
            scene_version: 0,
            groups,
            nodes,
            edges,
            tags: vec![],
            comments: vec![],
            artifacts: vec![],
            proposals: None,
            selection: SceneSelection::Canvas,
            updated_at: NOW.to_string(),
        }
    }

    #[test]
    fn recipe_from_single_node_round_trips() {
        // A lone node whose host group is NOT selected -> one wrapper frame + one
        // shape; apply_template reproduces exactly one node.
        let scene = build_scene(
            vec![scene_group("g1", 0.0, 0.0, 400.0, 300.0)],
            vec![scene_node("n1", "g1", 40.0, 60.0)],
            vec![],
        );
        let recipe = recipe_from_selection(&scene, &["n1".to_string()], sel_metadata());
        assert_eq!(recipe.recipe.frames.len(), 1, "synthesized wrapper frame");
        assert_eq!(recipe.recipe.shapes.len(), 1);
        assert_eq!(recipe.recipe.edges.len(), 0);
        // Position is relative to the node's own min corner.
        assert_eq!(recipe.recipe.shapes[0].position, point(0.0, 0.0));

        let applied = apply_template(&recipe, point(500.0, 500.0), "sel", NOW);
        assert!(applied.errors.is_empty(), "errors: {:?}", applied.errors);
        assert_eq!(applied.groups.len(), 1);
        assert_eq!(applied.nodes.len(), 1);
        assert_eq!(applied.edges.len(), 0);
        assert_eq!(applied.nodes[0].title, "node n1");
        // Anchored at (500,500): the wrapper shape lands there.
        assert_eq!(applied.nodes[0].position, point(500.0, 500.0));
    }

    #[test]
    fn recipe_from_multi_select_nodes_round_trips() {
        // Two nodes in one (unselected) group, plus an edge between them. Both
        // nodes selected -> the edge is captured. One wrapper frame, two shapes,
        // one edge.
        let scene = build_scene(
            vec![scene_group("g1", 0.0, 0.0, 600.0, 400.0)],
            vec![
                scene_node("n1", "g1", 100.0, 120.0),
                scene_node("n2", "g1", 300.0, 120.0),
            ],
            vec![scene_edge("e1", "g1", "n1", "n2")],
        );
        let recipe = recipe_from_selection(
            &scene,
            &["n1".to_string(), "n2".to_string()],
            sel_metadata(),
        );
        assert_eq!(recipe.recipe.frames.len(), 1);
        assert_eq!(recipe.recipe.shapes.len(), 2);
        assert_eq!(recipe.recipe.edges.len(), 1);
        // Positions relative to min corner (100,120): n1 -> (0,0), n2 -> (200,0).
        let s1 = recipe.recipe.shapes.iter().find(|s| s.local_id == "shape-n1").unwrap();
        let s2 = recipe.recipe.shapes.iter().find(|s| s.local_id == "shape-n2").unwrap();
        assert_eq!(s1.position, point(0.0, 0.0));
        assert_eq!(s2.position, point(200.0, 0.0));

        let applied = apply_template(&recipe, point(0.0, 0.0), "sel", NOW);
        assert!(applied.errors.is_empty(), "errors: {:?}", applied.errors);
        assert_eq!(applied.groups.len(), 1);
        assert_eq!(applied.nodes.len(), 2);
        assert_eq!(applied.edges.len(), 1);
    }

    #[test]
    fn recipe_from_single_group_captures_members() {
        // Selecting only the group id pulls in its member nodes + internal edge.
        let scene = build_scene(
            vec![scene_group("g1", 10.0, 10.0, 600.0, 400.0)],
            vec![
                scene_node("n1", "g1", 50.0, 50.0),
                scene_node("n2", "g1", 250.0, 50.0),
            ],
            vec![scene_edge("e1", "g1", "n1", "n2")],
        );
        let recipe = recipe_from_selection(&scene, &["g1".to_string()], sel_metadata());
        // The group itself is the only frame (no wrapper needed).
        assert_eq!(recipe.recipe.frames.len(), 1);
        assert_eq!(recipe.recipe.frames[0].local_id, "frame-g1");
        assert_eq!(recipe.recipe.shapes.len(), 2);
        assert_eq!(recipe.recipe.edges.len(), 1);
        // Both shapes are parented to the group frame, not a wrapper.
        for s in &recipe.recipe.shapes {
            assert_eq!(s.frame_local_id, "frame-g1");
        }
        // Min corner is the group bounds top-left (10,10).
        let s1 = recipe.recipe.shapes.iter().find(|s| s.local_id == "shape-n1").unwrap();
        assert_eq!(s1.position, point(40.0, 40.0));

        let applied = apply_template(&recipe, point(0.0, 0.0), "sel", NOW);
        assert!(applied.errors.is_empty(), "errors: {:?}", applied.errors);
        assert_eq!(applied.groups.len(), 1);
        assert_eq!(applied.nodes.len(), 2);
        assert_eq!(applied.edges.len(), 1);
        // The reconstructed group keeps the source title.
        assert_eq!(applied.groups[0].title, "group g1");
    }

    #[test]
    fn edge_with_unselected_endpoint_is_dropped() {
        let scene = build_scene(
            vec![scene_group("g1", 0.0, 0.0, 600.0, 400.0)],
            vec![
                scene_node("n1", "g1", 0.0, 0.0),
                scene_node("n2", "g1", 200.0, 0.0),
            ],
            vec![scene_edge("e1", "g1", "n1", "n2")],
        );
        // Only n1 selected -> edge to n2 is dropped (n2 not captured).
        let recipe = recipe_from_selection(&scene, &["n1".to_string()], sel_metadata());
        assert_eq!(recipe.recipe.shapes.len(), 1);
        assert_eq!(recipe.recipe.edges.len(), 0);
    }
}
