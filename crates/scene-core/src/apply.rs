//! Canonical op-apply — a faithful port of `src/shared/renderPatch.ts`.
//!
//! `apply_render_patch_to_shape_scene` validates a patch, resolves an operation
//! envelope, dispatches by kind to build an intermediate `ScenePatch` diff, then
//! funnels everything through `commit_app_patch`. Output is verified byte-for-byte
//! against golden vectors generated from the TS source.

use std::collections::{HashMap, HashSet};

use crate::envelope::{synthesise_local_envelope, OperationEnvelope};
use crate::model::{
    NodeStatus, NodeType, Point, Scene, SceneComment, SceneEdge, SceneGroup, SceneNode,
    SceneSelection, ScenePatch, Size, Tag, TranslateGroup, WorldPoint, WorldRect,
};
use crate::op::{
    AlignMode, Axis, EditField, ExtendedRenderPatch, RenderCard, RenderGroup, RenderScenePatch,
    TargetKind,
};

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedRenderPatch {
    pub scene: Scene,
    pub app_patch: ScenePatch,
    pub errors: Vec<String>,
    pub envelope: OperationEnvelope,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedCommentUpdate {
    pub scene: Scene,
    pub comment: Option<SceneComment>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Insertion-ordered map (mirrors JS `Map` semantics: set keeps a key's position,
// new keys append, delete removes).
// ---------------------------------------------------------------------------

struct OrderedMap<V> {
    order: Vec<String>,
    map: HashMap<String, V>,
}

impl<V: Clone> OrderedMap<V> {
    fn new() -> Self {
        OrderedMap {
            order: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn set(&mut self, key: String, value: V) {
        if !self.map.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.map.insert(key, value);
    }

    fn delete(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key)
    }

    fn key_set(&self) -> HashSet<String> {
        self.map.keys().cloned().collect()
    }

    fn into_values(mut self) -> Vec<V> {
        self.order
            .iter()
            .filter_map(|k| self.map.remove(k))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

pub fn apply_render_patch_to_shape_scene(
    scene: &Scene,
    patch: &RenderScenePatch,
    now: &str,
    envelope: Option<OperationEnvelope>,
) -> AppliedRenderPatch {
    let errors = validate_render_patch_for_shape_scene(scene, patch);
    let resolved_envelope = envelope.unwrap_or_else(|| {
        synthesise_local_envelope(
            ExtendedRenderPatch::Render(patch.clone()),
            scene.scene_version,
            now,
        )
    });
    if !errors.is_empty() {
        return AppliedRenderPatch {
            scene: scene.clone(),
            app_patch: ScenePatch::default(),
            errors,
            envelope: resolved_envelope,
        };
    }

    match patch {
        RenderScenePatch::CreateGroup { group } => {
            let g = render_group_to_scene_group(group, now, None);
            let id = g.id.clone();
            commit_app_patch(
                scene,
                ScenePatch {
                    groups: Some(vec![g]),
                    selection: Some(SceneSelection::Group { id }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::DeleteGroup { id } => {
            let removed_group_ids = descendant_group_ids(scene, id);
            let removed_node_ids: Vec<String> = scene
                .nodes
                .iter()
                .filter(|n| removed_group_ids.contains(&n.group_id))
                .map(|n| n.id.clone())
                .collect();
            let removed_node_id_set: HashSet<String> = removed_node_ids.iter().cloned().collect();
            let removed_edge_ids: Vec<String> = scene
                .edges
                .iter()
                .filter(|e| {
                    removed_group_ids.contains(&e.group_id)
                        || removed_node_id_set.contains(&e.source)
                        || removed_node_id_set.contains(&e.target)
                })
                .map(|e| e.id.clone())
                .collect();
            commit_app_patch(
                scene,
                ScenePatch {
                    remove_group_ids: Some(removed_group_ids.into_iter().collect()),
                    remove_node_ids: Some(removed_node_ids),
                    remove_edge_ids: Some(removed_edge_ids),
                    selection: Some(SceneSelection::Canvas),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::MoveGroup { id, delta } => commit_app_patch(
            scene,
            ScenePatch {
                translate_groups: Some(vec![TranslateGroup {
                    group_id: id.clone(),
                    dx: delta.x,
                    dy: delta.y,
                }]),
                selection: Some(SceneSelection::Group { id: id.clone() }),
                ..Default::default()
            },
            now,
            resolved_envelope,
        ),
        RenderScenePatch::MoveCard { id, position } => {
            let mut node = find_node(scene, id).clone();
            node.position = *position;
            node.updated_at = Some(now.to_string());
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(vec![node]),
                    selection: Some(SceneSelection::Node { id: id.clone() }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::SetCardZIndex { id, z_index } => {
            let mut node = find_node(scene, id).clone();
            node.z_index = *z_index;
            node.updated_at = Some(now.to_string());
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(vec![node]),
                    selection: Some(SceneSelection::Node { id: id.clone() }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::EditCardText { id, field, value } => {
            let mut node = find_node(scene, id).clone();
            match field {
                EditField::Title => node.title = value.clone(),
                EditField::Summary => node.summary = value.clone(),
                EditField::Detail => node.detail = value.clone(),
            }
            node.updated_at = Some(now.to_string());
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(vec![node]),
                    selection: Some(SceneSelection::Node { id: id.clone() }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::CreateCard { card } => {
            let node = render_card_to_scene_node(card, now);
            let id = node.id.clone();
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(vec![node]),
                    selection: Some(SceneSelection::Node { id }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::DeleteCard { id } => {
            let incident_edge_ids: Vec<String> = scene
                .edges
                .iter()
                .filter(|e| &e.source == id || &e.target == id)
                .map(|e| e.id.clone())
                .collect();
            commit_app_patch(
                scene,
                ScenePatch {
                    remove_node_ids: Some(vec![id.clone()]),
                    remove_edge_ids: Some(incident_edge_ids),
                    selection: Some(SceneSelection::Canvas),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::CreateEdge {
            group_id,
            source,
            target,
            edge_id,
            label,
        } => {
            let edge = SceneEdge {
                id: edge_id.clone(),
                edge_type: crate::model::EdgeType::Supports,
                source: source.clone(),
                target: target.clone(),
                label: label.clone().unwrap_or_else(|| "relates".to_string()),
                rationale: String::new(),
                confidence: 0.5,
                group_id: group_id.clone(),
                tag_ids: vec![],
                updated_at: Some(now.to_string()),
                meta: None,
            };
            let id = edge.id.clone();
            commit_app_patch(
                scene,
                ScenePatch {
                    edges: Some(vec![edge]),
                    selection: Some(SceneSelection::Edge { id }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::DeleteEdge { id } => commit_app_patch(
            scene,
            ScenePatch {
                remove_edge_ids: Some(vec![id.clone()]),
                selection: Some(SceneSelection::Canvas),
                ..Default::default()
            },
            now,
            resolved_envelope,
        ),
        RenderScenePatch::ResizeCard { id, bounds } => {
            let mut node = find_node(scene, id).clone();
            node.position = Point {
                x: bounds.x,
                y: bounds.y,
            };
            node.size = Size {
                width: bounds.width,
                height: bounds.height,
            };
            node.updated_at = Some(now.to_string());
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(vec![node]),
                    selection: Some(SceneSelection::Node { id: id.clone() }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::ResizeGroup { id, bounds } => {
            let mut group = find_group(scene, id).clone();
            group.bounds = *bounds;
            group.updated_at = now.to_string();
            commit_app_patch(
                scene,
                ScenePatch {
                    groups: Some(vec![group]),
                    selection: Some(SceneSelection::Group { id: id.clone() }),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::AlignCards { ids, axis, mode } => {
            let targets: Vec<SceneNode> = scene
                .nodes
                .iter()
                .filter(|n| ids.contains(&n.id))
                .cloned()
                .collect();
            if targets.is_empty() {
                return AppliedRenderPatch {
                    scene: scene.clone(),
                    app_patch: ScenePatch::default(),
                    errors: vec!["No matching nodes for align-cards".to_string()],
                    envelope: resolved_envelope,
                };
            }
            let aligned = align_nodes(targets, *axis, *mode, now);
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(aligned),
                    selection: Some(SceneSelection::Canvas),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::DistributeCards { ids, axis } => {
            let targets: Vec<SceneNode> = scene
                .nodes
                .iter()
                .filter(|n| ids.contains(&n.id))
                .cloned()
                .collect();
            if targets.is_empty() {
                return AppliedRenderPatch {
                    scene: scene.clone(),
                    app_patch: ScenePatch::default(),
                    errors: vec!["No matching nodes for distribute-cards".to_string()],
                    envelope: resolved_envelope,
                };
            }
            let distributed = distribute_nodes(targets, *axis, now);
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(distributed),
                    selection: Some(SceneSelection::Canvas),
                    ..Default::default()
                },
                now,
                resolved_envelope,
            )
        }
        RenderScenePatch::DuplicateObjects { ids, delta } => {
            apply_duplicate_objects(scene, ids, *delta, now, resolved_envelope)
        }
        RenderScenePatch::Batch { ops } => apply_batch(scene, ops, now, resolved_envelope),
        RenderScenePatch::GroupObjects {
            ids,
            frame_id,
            parent_group_id,
            title,
            bounds,
        } => apply_group_objects(
            scene,
            ids,
            frame_id,
            parent_group_id.clone(),
            title.clone(),
            *bounds,
            now,
            resolved_envelope,
        ),
        RenderScenePatch::Ungroup { id } => apply_ungroup(scene, id, now, resolved_envelope),
        RenderScenePatch::SetObjectGroup { ids, frame_id } => {
            apply_set_object_group(scene, ids, frame_id, now, resolved_envelope)
        }
        RenderScenePatch::SetObjectTags {
            target_kind,
            id,
            tag_ids,
        } => apply_set_object_tags(scene, *target_kind, id, tag_ids, now, resolved_envelope),
        RenderScenePatch::CreateTag { tag } => {
            apply_create_tag(scene, tag, now, resolved_envelope)
        }
        RenderScenePatch::Select { selection } => commit_app_patch(
            scene,
            ScenePatch {
                selection: Some(selection.clone()),
                ..Default::default()
            },
            now,
            resolved_envelope,
        ),
    }
}

// ---------------------------------------------------------------------------
// Side functions
// ---------------------------------------------------------------------------

/// Apply a bulk [`ScenePatch`] (the legacy `saveScenePatch` shape) to a scene.
///
/// Additive wrapper over [`commit_app_patch`]: it expands the patch's group
/// removals to their descendant-group subtree (and the nodes/edges those groups
/// own) before committing, so a `removeGroupIds` cascades exactly like the Node
/// `saveScenePatch`. Node/edge removals are passed straight through;
/// `commit_app_patch` already prunes edges orphaned by a node removal and any
/// node/edge whose group was removed. `scene_version` is bumped once.
pub fn apply_scene_patch(scene: &Scene, patch: &ScenePatch, now: &str) -> Scene {
    // Cascade group removals to descendant groups + their owned nodes/edges,
    // mirroring the recursive `removeGroupInDb` in the Node storage layer.
    let mut removed_group_ids: HashSet<String> = HashSet::new();
    for gid in patch.remove_group_ids.clone().unwrap_or_default() {
        for descendant in descendant_group_ids(scene, &gid) {
            removed_group_ids.insert(descendant);
        }
    }
    let mut remove_node_ids: Vec<String> = patch.remove_node_ids.clone().unwrap_or_default();
    let mut remove_edge_ids: Vec<String> = patch.remove_edge_ids.clone().unwrap_or_default();
    for node in &scene.nodes {
        if removed_group_ids.contains(&node.group_id) {
            remove_node_ids.push(node.id.clone());
        }
    }
    let removed_node_id_set: HashSet<&String> = remove_node_ids.iter().collect();
    for edge in &scene.edges {
        if removed_group_ids.contains(&edge.group_id)
            || removed_node_id_set.contains(&edge.source)
            || removed_node_id_set.contains(&edge.target)
        {
            remove_edge_ids.push(edge.id.clone());
        }
    }

    let envelope = synthesise_local_envelope(
        ExtendedRenderPatch::Render(RenderScenePatch::Select {
            selection: scene.selection.clone(),
        }),
        scene.scene_version,
        now,
    );
    commit_app_patch(
        scene,
        ScenePatch {
            groups: patch.groups.clone(),
            nodes: patch.nodes.clone(),
            edges: patch.edges.clone(),
            translate_groups: patch.translate_groups.clone(),
            remove_group_ids: non_empty(removed_group_ids.into_iter().collect()),
            remove_node_ids: non_empty(remove_node_ids),
            remove_edge_ids: non_empty(remove_edge_ids),
            selection: patch.selection.clone(),
        },
        now,
        envelope,
    )
    .scene
}

pub fn update_shape_scene_group_tags(
    scene: &Scene,
    group_id: &str,
    tag_ids: &[String],
    now: &str,
) -> AppliedRenderPatch {
    let group_tags_envelope = synthesise_local_envelope(
        ExtendedRenderPatch::Render(RenderScenePatch::Select {
            selection: SceneSelection::Group {
                id: group_id.to_string(),
            },
        }),
        scene.scene_version,
        now,
    );
    let group = scene.groups.iter().find(|g| g.id == group_id);
    let group = match group {
        Some(g) => g.clone(),
        None => {
            return AppliedRenderPatch {
                scene: scene.clone(),
                app_patch: ScenePatch::default(),
                errors: vec![format!("Unknown group id: {group_id}")],
                envelope: group_tags_envelope,
            }
        }
    };
    let known_tag_ids: HashSet<&String> = scene.tags.iter().map(|t| &t.id).collect();
    if let Some(unknown) = tag_ids.iter().find(|t| !known_tag_ids.contains(t)) {
        return AppliedRenderPatch {
            scene: scene.clone(),
            app_patch: ScenePatch::default(),
            errors: vec![format!("Unknown tag id: {unknown}")],
            envelope: group_tags_envelope,
        };
    }
    let mut next_group = group;
    next_group.tag_ids = tag_ids.to_vec();
    next_group.updated_at = now.to_string();
    let id = next_group.id.clone();
    commit_app_patch(
        scene,
        ScenePatch {
            groups: Some(vec![next_group]),
            selection: Some(SceneSelection::Group { id }),
            ..Default::default()
        },
        now,
        group_tags_envelope,
    )
}

pub fn add_shape_scene_comment(
    scene: &Scene,
    target: &SceneSelection,
    body: &str,
    now: &str,
) -> AppliedCommentUpdate {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return AppliedCommentUpdate {
            scene: scene.clone(),
            comment: None,
            errors: vec!["Comment body is required".to_string()],
        };
    }
    let selection_errors = validate_scene_selection(scene, target);
    if !selection_errors.is_empty() {
        return AppliedCommentUpdate {
            scene: scene.clone(),
            comment: None,
            errors: selection_errors,
        };
    }
    // TS uses `renderer-comment-${Date.now().toString(36)}`; scene-core stays
    // deterministic by deriving the id from the injected `now`. Golden tests mask
    // this id field.
    let comment = SceneComment {
        id: format!("renderer-comment-{now}"),
        target: target.clone(),
        body: trimmed.to_string(),
        author: "human".to_string(),
        resolved: false,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    let mut comments = vec![comment.clone()];
    comments.extend(scene.comments.iter().cloned());
    let next = Scene {
        scene_version: scene.scene_version + 1,
        comments,
        updated_at: now.to_string(),
        ..scene.clone()
    };
    AppliedCommentUpdate {
        scene: next,
        comment: Some(comment),
        errors: vec![],
    }
}

/// Update a comment's `body` and/or `resolved` flag (the legacy `updateComment`
/// path). Returns the updated comment plus the new scene with `scene_version`
/// bumped. A missing comment id is reported in `errors`.
pub fn update_shape_scene_comment(
    scene: &Scene,
    comment_id: &str,
    body: Option<&str>,
    resolved: Option<bool>,
    now: &str,
) -> AppliedCommentUpdate {
    let Some(current) = scene.comments.iter().find(|c| c.id == comment_id).cloned() else {
        return AppliedCommentUpdate {
            scene: scene.clone(),
            comment: None,
            errors: vec![format!("Comment not found: {comment_id}")],
        };
    };
    let mut updated = current;
    if let Some(body) = body {
        updated.body = body.to_string();
    }
    if let Some(resolved) = resolved {
        updated.resolved = resolved;
    }
    updated.updated_at = now.to_string();
    let comments: Vec<SceneComment> = scene
        .comments
        .iter()
        .map(|c| if c.id == comment_id { updated.clone() } else { c.clone() })
        .collect();
    let next = Scene {
        scene_version: scene.scene_version + 1,
        comments,
        updated_at: now.to_string(),
        ..scene.clone()
    };
    AppliedCommentUpdate {
        scene: next,
        comment: Some(updated),
        errors: vec![],
    }
}

// ---------------------------------------------------------------------------
// Commit funnel
// ---------------------------------------------------------------------------

fn commit_app_patch(
    scene: &Scene,
    patch: ScenePatch,
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let remove_group_ids: HashSet<String> =
        patch.remove_group_ids.clone().unwrap_or_default().into_iter().collect();
    let remove_node_ids: HashSet<String> =
        patch.remove_node_ids.clone().unwrap_or_default().into_iter().collect();
    let remove_edge_ids: HashSet<String> =
        patch.remove_edge_ids.clone().unwrap_or_default().into_iter().collect();

    let mut groups: OrderedMap<SceneGroup> = OrderedMap::new();
    for g in &scene.groups {
        if !remove_group_ids.contains(&g.id) {
            groups.set(g.id.clone(), g.clone());
        }
    }
    if let Some(patch_groups) = &patch.groups {
        for g in patch_groups {
            groups.set(g.id.clone(), g.clone());
        }
    }

    let mut nodes: OrderedMap<SceneNode> = OrderedMap::new();
    for n in &scene.nodes {
        nodes.set(n.id.clone(), n.clone());
    }
    for id in &remove_node_ids {
        nodes.delete(id);
    }
    if let Some(patch_nodes) = &patch.nodes {
        for n in patch_nodes {
            nodes.set(n.id.clone(), n.clone());
        }
    }

    if let Some(translate_groups) = &patch.translate_groups {
        for movement in translate_groups {
            let group = match groups.get(&movement.group_id) {
                Some(g) => g.clone(),
                None => continue,
            };
            let mut moved = group;
            moved.bounds.x += movement.dx;
            moved.bounds.y += movement.dy;
            moved.updated_at = now.to_string();
            groups.set(movement.group_id.clone(), moved);

            let member_ids: Vec<String> = nodes
                .order
                .iter()
                .filter(|id| {
                    nodes
                        .map
                        .get(*id)
                        .map(|n| n.group_id == movement.group_id)
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            for nid in member_ids {
                if let Some(node) = nodes.get(&nid).cloned() {
                    let mut moved_node = node;
                    moved_node.position.x += movement.dx;
                    moved_node.position.y += movement.dy;
                    moved_node.updated_at = Some(now.to_string());
                    nodes.set(nid, moved_node);
                }
            }
        }
    }

    let mut edges: OrderedMap<SceneEdge> = OrderedMap::new();
    for e in &scene.edges {
        edges.set(e.id.clone(), e.clone());
    }
    for id in &remove_edge_ids {
        edges.delete(id);
    }
    if let Some(patch_edges) = &patch.edges {
        for e in patch_edges {
            edges.set(e.id.clone(), e.clone());
        }
    }

    let group_ids = groups.key_set();
    let groups_vec = groups.into_values();
    let nodes_vec: Vec<SceneNode> = nodes
        .into_values()
        .into_iter()
        .filter(|n| group_ids.contains(&n.group_id))
        .collect();
    let live_node_ids: HashSet<String> = nodes_vec.iter().map(|n| n.id.clone()).collect();
    let edges_vec: Vec<SceneEdge> = edges
        .into_values()
        .into_iter()
        .filter(|e| {
            group_ids.contains(&e.group_id)
                && live_node_ids.contains(&e.source)
                && live_node_ids.contains(&e.target)
        })
        .collect();

    let selection = patch
        .selection
        .clone()
        .unwrap_or_else(|| scene.selection.clone());

    let next = Scene {
        scene_version: scene.scene_version + 1,
        groups: groups_vec,
        nodes: nodes_vec,
        edges: edges_vec,
        selection,
        updated_at: now.to_string(),
        ..scene.clone()
    };
    AppliedRenderPatch {
        scene: next,
        app_patch: patch,
        errors: vec![],
        envelope,
    }
}

// ---------------------------------------------------------------------------
// Render-projection -> scene mappers
// ---------------------------------------------------------------------------

fn render_group_to_scene_group(
    group: &RenderGroup,
    now: &str,
    parent_group_id: Option<String>,
) -> SceneGroup {
    SceneGroup {
        id: group.id.clone(),
        parent_group_id,
        title: if group.title.is_empty() {
            "Untitled group".to_string()
        } else {
            group.title.clone()
        },
        summary: group.summary.clone(),
        bounds: group.bounds,
        tag_ids: group.tag_ids.clone(),
        z_index: group.z_index,
        collapsed: false,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        meta: None,
    }
}

fn render_card_to_scene_node(card: &RenderCard, now: &str) -> SceneNode {
    SceneNode {
        id: card.id.clone(),
        node_type: scene_node_type(&card.node_type),
        title: if card.title.is_empty() {
            "Untitled node".to_string()
        } else {
            card.title.clone()
        },
        summary: card.summary.clone(),
        detail: card.detail.clone(),
        status: scene_node_status(&card.status),
        confidence: 0.5,
        evidence_refs: vec![],
        child_decision_ids: vec![],
        group_id: card.group_id.clone(),
        position: Point {
            x: card.bounds.x,
            y: card.bounds.y,
        },
        size: Size {
            width: card.bounds.width,
            height: card.bounds.height,
        },
        z_index: card.z_index,
        tag_ids: vec![],
        updated_at: Some(now.to_string()),
        meta: None,
    }
}

fn scene_node_type(t: &str) -> NodeType {
    match t {
        "proposition" => NodeType::Proposition,
        "decision_point" => NodeType::DecisionPoint,
        "option" => NodeType::Option,
        "evidence" => NodeType::Evidence,
        "tradeoff" => NodeType::Tradeoff,
        "blocker" => NodeType::Blocker,
        "subdecision" => NodeType::Subdecision,
        "task" => NodeType::Task,
        "artifact" => NodeType::Artifact,
        _ => NodeType::Task,
    }
}

fn scene_node_status(s: &str) -> NodeStatus {
    match s {
        "draft" => NodeStatus::Draft,
        "viable" => NodeStatus::Viable,
        "conditional" => NodeStatus::Conditional,
        "infeasible" => NodeStatus::Infeasible,
        "unknown" => NodeStatus::Unknown,
        "selected" => NodeStatus::Selected,
        "deferred" => NodeStatus::Deferred,
        "complete" => NodeStatus::Complete,
        _ => NodeStatus::Draft,
    }
}

// ---------------------------------------------------------------------------
// align / distribute
// ---------------------------------------------------------------------------

fn align_nodes(nodes: Vec<SceneNode>, axis: Axis, mode: AlignMode, now: &str) -> Vec<SceneNode> {
    match axis {
        Axis::X => {
            let min_x = nodes.iter().map(|n| n.position.x).fold(f64::INFINITY, f64::min);
            let max_right = nodes
                .iter()
                .map(|n| n.position.x + n.size.width)
                .fold(f64::NEG_INFINITY, f64::max);
            nodes
                .into_iter()
                .map(|mut n| {
                    let new_x = match mode {
                        AlignMode::Start => min_x,
                        AlignMode::End => max_right - n.size.width,
                        AlignMode::Center => (min_x + max_right) / 2.0 - n.size.width / 2.0,
                    };
                    n.position = Point {
                        x: new_x,
                        y: n.position.y,
                    };
                    n.updated_at = Some(now.to_string());
                    n
                })
                .collect()
        }
        Axis::Y => {
            let min_y = nodes.iter().map(|n| n.position.y).fold(f64::INFINITY, f64::min);
            let max_bottom = nodes
                .iter()
                .map(|n| n.position.y + n.size.height)
                .fold(f64::NEG_INFINITY, f64::max);
            nodes
                .into_iter()
                .map(|mut n| {
                    let new_y = match mode {
                        AlignMode::Start => min_y,
                        AlignMode::End => max_bottom - n.size.height,
                        AlignMode::Center => (min_y + max_bottom) / 2.0 - n.size.height / 2.0,
                    };
                    n.position = Point {
                        x: n.position.x,
                        y: new_y,
                    };
                    n.updated_at = Some(now.to_string());
                    n
                })
                .collect()
        }
    }
}

fn distribute_nodes(mut nodes: Vec<SceneNode>, axis: Axis, now: &str) -> Vec<SceneNode> {
    match axis {
        Axis::X => {
            nodes.sort_by(|a, b| {
                a.position
                    .x
                    .partial_cmp(&b.position.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let min_x = nodes[0].position.x;
            let max_right = nodes
                .iter()
                .map(|n| n.position.x + n.size.width)
                .fold(f64::NEG_INFINITY, f64::max);
            let total_width: f64 = nodes.iter().map(|n| n.size.width).sum();
            let gap = (max_right - min_x - total_width) / (nodes.len() as f64 - 1.0);
            let mut cursor = min_x;
            let mut out = Vec::with_capacity(nodes.len());
            for mut n in nodes {
                let width = n.size.width;
                n.position = Point {
                    x: cursor,
                    y: n.position.y,
                };
                n.updated_at = Some(now.to_string());
                out.push(n);
                cursor += width + gap;
            }
            out
        }
        Axis::Y => {
            nodes.sort_by(|a, b| {
                a.position
                    .y
                    .partial_cmp(&b.position.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let min_y = nodes[0].position.y;
            let max_bottom = nodes
                .iter()
                .map(|n| n.position.y + n.size.height)
                .fold(f64::NEG_INFINITY, f64::max);
            let total_height: f64 = nodes.iter().map(|n| n.size.height).sum();
            let gap = (max_bottom - min_y - total_height) / (nodes.len() as f64 - 1.0);
            let mut cursor = min_y;
            let mut out = Vec::with_capacity(nodes.len());
            for mut n in nodes {
                let height = n.size.height;
                n.position = Point {
                    x: n.position.x,
                    y: cursor,
                };
                n.updated_at = Some(now.to_string());
                out.push(n);
                cursor += height + gap;
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------
// duplicate / batch
// ---------------------------------------------------------------------------

fn apply_duplicate_objects(
    scene: &Scene,
    ids: &[String],
    delta: WorldPoint,
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let id_set: HashSet<&String> = ids.iter().collect();
    let mut old_to_new: HashMap<String, String> = HashMap::new();
    for id in ids {
        old_to_new.insert(id.clone(), format!("{id}-dup-{now}"));
    }

    let new_nodes: Vec<SceneNode> = scene
        .nodes
        .iter()
        .filter(|n| id_set.contains(&n.id))
        .map(|n| {
            let mut nn = n.clone();
            nn.id = old_to_new.get(&n.id).cloned().unwrap();
            nn.position = Point {
                x: n.position.x + delta.x,
                y: n.position.y + delta.y,
            };
            nn.updated_at = Some(now.to_string());
            nn
        })
        .collect();

    let new_edges: Vec<SceneEdge> = scene
        .edges
        .iter()
        .filter(|e| id_set.contains(&e.source) && id_set.contains(&e.target))
        .map(|e| {
            let mut ne = e.clone();
            ne.id = format!("{}-dup-{}", e.id, now);
            ne.source = old_to_new.get(&e.source).cloned().unwrap();
            ne.target = old_to_new.get(&e.target).cloned().unwrap();
            ne.updated_at = Some(now.to_string());
            ne
        })
        .collect();

    let selection = if let Some(first) = new_nodes.first() {
        SceneSelection::Node {
            id: first.id.clone(),
        }
    } else {
        SceneSelection::Canvas
    };
    commit_app_patch(
        scene,
        ScenePatch {
            nodes: Some(new_nodes),
            edges: Some(new_edges),
            selection: Some(selection),
            ..Default::default()
        },
        now,
        envelope,
    )
}

fn apply_batch(
    scene: &Scene,
    ops: &[RenderScenePatch],
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let mut current = scene.clone();
    let mut last_errors: Vec<String> = vec![];
    for op in ops {
        let result =
            apply_render_patch_to_shape_scene(&current, op, now, Some(envelope.clone()));
        if !result.errors.is_empty() {
            last_errors = result.errors;
            break;
        }
        current = result.scene;
    }
    if !last_errors.is_empty() {
        return AppliedRenderPatch {
            scene: scene.clone(),
            app_patch: ScenePatch::default(),
            errors: last_errors,
            envelope,
        };
    }

    // Reconstruct a merged appPatch by value-diff (this field is not part of the
    // golden-compared scene output).
    let orig_nodes: HashMap<&String, &SceneNode> =
        scene.nodes.iter().map(|n| (&n.id, n)).collect();
    let current_node_ids: HashSet<&String> = current.nodes.iter().map(|n| &n.id).collect();
    let removed_node_ids: Vec<String> = scene
        .nodes
        .iter()
        .filter(|n| !current_node_ids.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();
    let changed_nodes: Vec<SceneNode> = current
        .nodes
        .iter()
        .filter(|n| orig_nodes.get(&n.id).map(|o| *o != *n).unwrap_or(true))
        .cloned()
        .collect();

    let orig_edges: HashMap<&String, &SceneEdge> =
        scene.edges.iter().map(|e| (&e.id, e)).collect();
    let current_edge_ids: HashSet<&String> = current.edges.iter().map(|e| &e.id).collect();
    let removed_edge_ids: Vec<String> = scene
        .edges
        .iter()
        .filter(|e| !current_edge_ids.contains(&e.id))
        .map(|e| e.id.clone())
        .collect();
    let changed_edges: Vec<SceneEdge> = current
        .edges
        .iter()
        .filter(|e| orig_edges.get(&e.id).map(|o| *o != *e).unwrap_or(true))
        .cloned()
        .collect();

    let orig_groups: HashMap<&String, &SceneGroup> =
        scene.groups.iter().map(|g| (&g.id, g)).collect();
    let current_group_ids: HashSet<&String> = current.groups.iter().map(|g| &g.id).collect();
    let removed_group_ids: Vec<String> = scene
        .groups
        .iter()
        .filter(|g| !current_group_ids.contains(&g.id))
        .map(|g| g.id.clone())
        .collect();
    let changed_groups: Vec<SceneGroup> = current
        .groups
        .iter()
        .filter(|g| orig_groups.get(&g.id).map(|o| *o != *g).unwrap_or(true))
        .cloned()
        .collect();

    let app_patch = ScenePatch {
        nodes: non_empty(changed_nodes),
        edges: non_empty(changed_edges),
        groups: non_empty(changed_groups),
        remove_node_ids: non_empty(removed_node_ids),
        remove_edge_ids: non_empty(removed_edge_ids),
        remove_group_ids: non_empty(removed_group_ids),
        translate_groups: None,
        selection: Some(current.selection.clone()),
    };
    AppliedRenderPatch {
        scene: current,
        app_patch,
        errors: vec![],
        envelope,
    }
}

fn non_empty<T>(v: Vec<T>) -> Option<Vec<T>> {
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

// ---------------------------------------------------------------------------
// grouping / labeling
// ---------------------------------------------------------------------------

fn descendant_group_ids(scene: &Scene, root_group_id: &str) -> HashSet<String> {
    let mut ids: HashSet<String> = HashSet::new();
    ids.insert(root_group_id.to_string());
    let mut changed = true;
    while changed {
        changed = false;
        for group in &scene.groups {
            if let Some(parent) = &group.parent_group_id {
                if ids.contains(parent) && !ids.contains(&group.id) {
                    ids.insert(group.id.clone());
                    changed = true;
                }
            }
        }
    }
    ids
}

fn compute_group_bounds(members: &[&SceneNode], padding: f64) -> WorldRect {
    if members.is_empty() {
        return WorldRect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        };
    }
    let min_x = members
        .iter()
        .map(|n| n.position.x)
        .fold(f64::INFINITY, f64::min);
    let min_y = members
        .iter()
        .map(|n| n.position.y)
        .fold(f64::INFINITY, f64::min);
    let max_x = members
        .iter()
        .map(|n| n.position.x + n.size.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = members
        .iter()
        .map(|n| n.position.y + n.size.height)
        .fold(f64::NEG_INFINITY, f64::max);
    WorldRect {
        x: min_x - padding,
        y: min_y - padding,
        width: max_x - min_x + padding * 2.0,
        height: max_y - min_y + padding * 2.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_group_objects(
    scene: &Scene,
    ids: &[String],
    frame_id: &str,
    parent_group_id: Option<String>,
    title: Option<String>,
    bounds: Option<WorldRect>,
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let id_set: HashSet<&String> = ids.iter().collect();
    let member_nodes: Vec<&SceneNode> =
        scene.nodes.iter().filter(|n| id_set.contains(&n.id)).collect();
    let computed_bounds = bounds.unwrap_or_else(|| compute_group_bounds(&member_nodes, 40.0));
    let new_group = SceneGroup {
        id: frame_id.to_string(),
        parent_group_id,
        title: title.unwrap_or_else(|| "Group".to_string()),
        summary: String::new(),
        bounds: computed_bounds,
        tag_ids: vec![],
        z_index: 0.0,
        collapsed: false,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        meta: None,
    };
    let updated_nodes: Vec<SceneNode> = member_nodes
        .iter()
        .map(|n| {
            let mut nn = (*n).clone();
            nn.group_id = frame_id.to_string();
            nn.updated_at = Some(now.to_string());
            nn
        })
        .collect();
    let updated_groups: Vec<SceneGroup> = scene
        .groups
        .iter()
        .filter(|g| id_set.contains(&g.id))
        .map(|g| {
            let mut gg = g.clone();
            gg.parent_group_id = Some(frame_id.to_string());
            gg.updated_at = now.to_string();
            gg
        })
        .collect();
    let mut groups = vec![new_group];
    groups.extend(updated_groups);
    commit_app_patch(
        scene,
        ScenePatch {
            groups: Some(groups),
            nodes: Some(updated_nodes),
            selection: Some(SceneSelection::Group {
                id: frame_id.to_string(),
            }),
            ..Default::default()
        },
        now,
        envelope,
    )
}

fn apply_ungroup(
    scene: &Scene,
    id: &str,
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let frame = scene.groups.iter().find(|g| g.id == id).unwrap();
    let new_parent = frame.parent_group_id.clone();
    let fallback_group_id: Option<String> = new_parent.clone().or_else(|| {
        scene
            .groups
            .iter()
            .find(|g| g.id != id)
            .map(|g| g.id.clone())
    });

    let updated_nodes: Vec<SceneNode> = scene
        .nodes
        .iter()
        .filter(|n| n.group_id == id)
        .map(|n| {
            let mut nn = n.clone();
            nn.group_id = fallback_group_id.clone().unwrap_or_else(|| n.group_id.clone());
            nn.updated_at = Some(now.to_string());
            nn
        })
        .collect();

    let updated_groups: Vec<SceneGroup> = scene
        .groups
        .iter()
        .filter(|g| g.parent_group_id.as_deref() == Some(id))
        .map(|g| {
            let mut gg = g.clone();
            gg.parent_group_id = new_parent.clone();
            gg.updated_at = now.to_string();
            gg
        })
        .collect();

    let updated_edges: Vec<SceneEdge> = scene
        .edges
        .iter()
        .filter(|e| e.group_id == id)
        .map(|e| {
            let mut ee = e.clone();
            ee.group_id = fallback_group_id.clone().unwrap_or_else(|| e.group_id.clone());
            ee.updated_at = Some(now.to_string());
            ee
        })
        .collect();

    commit_app_patch(
        scene,
        ScenePatch {
            groups: Some(updated_groups),
            nodes: Some(updated_nodes),
            edges: Some(updated_edges),
            remove_group_ids: Some(vec![id.to_string()]),
            selection: Some(SceneSelection::Canvas),
            ..Default::default()
        },
        now,
        envelope,
    )
}

fn apply_set_object_group(
    scene: &Scene,
    ids: &[String],
    frame_id: &str,
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let id_set: HashSet<&String> = ids.iter().collect();
    let updated_nodes: Vec<SceneNode> = scene
        .nodes
        .iter()
        .filter(|n| id_set.contains(&n.id))
        .map(|n| {
            let mut nn = n.clone();
            nn.group_id = frame_id.to_string();
            nn.updated_at = Some(now.to_string());
            nn
        })
        .collect();
    let updated_edges: Vec<SceneEdge> = scene
        .edges
        .iter()
        .filter(|e| id_set.contains(&e.id))
        .map(|e| {
            let mut ee = e.clone();
            ee.group_id = frame_id.to_string();
            ee.updated_at = Some(now.to_string());
            ee
        })
        .collect();
    let selection = if let Some(first) = updated_nodes.first() {
        SceneSelection::Node {
            id: first.id.clone(),
        }
    } else if let Some(first) = updated_edges.first() {
        SceneSelection::Edge {
            id: first.id.clone(),
        }
    } else {
        SceneSelection::Group {
            id: frame_id.to_string(),
        }
    };
    commit_app_patch(
        scene,
        ScenePatch {
            nodes: Some(updated_nodes),
            edges: Some(updated_edges),
            selection: Some(selection),
            ..Default::default()
        },
        now,
        envelope,
    )
}

fn apply_set_object_tags(
    scene: &Scene,
    target_kind: TargetKind,
    id: &str,
    tag_ids: &[String],
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    match target_kind {
        TargetKind::Frame => {
            let mut group = find_group(scene, id).clone();
            group.tag_ids = tag_ids.to_vec();
            group.updated_at = now.to_string();
            commit_app_patch(
                scene,
                ScenePatch {
                    groups: Some(vec![group]),
                    selection: Some(SceneSelection::Group { id: id.to_string() }),
                    ..Default::default()
                },
                now,
                envelope,
            )
        }
        TargetKind::Card => {
            let mut node = find_node(scene, id).clone();
            node.tag_ids = tag_ids.to_vec();
            node.updated_at = Some(now.to_string());
            commit_app_patch(
                scene,
                ScenePatch {
                    nodes: Some(vec![node]),
                    selection: Some(SceneSelection::Node { id: id.to_string() }),
                    ..Default::default()
                },
                now,
                envelope,
            )
        }
        TargetKind::Edge => {
            let mut edge = find_edge(scene, id).clone();
            edge.tag_ids = tag_ids.to_vec();
            edge.updated_at = Some(now.to_string());
            commit_app_patch(
                scene,
                ScenePatch {
                    edges: Some(vec![edge]),
                    selection: Some(SceneSelection::Edge { id: id.to_string() }),
                    ..Default::default()
                },
                now,
                envelope,
            )
        }
    }
}

fn apply_create_tag(
    scene: &Scene,
    tag: &Tag,
    now: &str,
    envelope: OperationEnvelope,
) -> AppliedRenderPatch {
    let mut tags = scene.tags.clone();
    let mut next_tag = tag.clone();
    next_tag.updated_at = now.to_string();
    tags.push(next_tag);
    let next = Scene {
        scene_version: scene.scene_version + 1,
        tags,
        updated_at: now.to_string(),
        ..scene.clone()
    };
    AppliedRenderPatch {
        scene: next,
        app_patch: ScenePatch::default(),
        errors: vec![],
        envelope,
    }
}

// ---------------------------------------------------------------------------
// validation
// ---------------------------------------------------------------------------

fn validate_render_patch_for_shape_scene(scene: &Scene, patch: &RenderScenePatch) -> Vec<String> {
    let mut errors: Vec<String> = vec![];
    let has_node = |id: &str| scene.nodes.iter().any(|n| n.id == id);
    let has_group = |id: &str| scene.groups.iter().any(|g| g.id == id);
    let has_edge = |id: &str| scene.edges.iter().any(|e| e.id == id);

    match patch {
        RenderScenePatch::MoveCard { id, .. }
        | RenderScenePatch::SetCardZIndex { id, .. }
        | RenderScenePatch::EditCardText { id, .. } => {
            if !has_node(id) {
                errors.push(format!("Unknown node id: {id}"));
            }
        }
        RenderScenePatch::CreateGroup { group } => {
            if has_group(&group.id) {
                errors.push(format!("Duplicate group id: {}", group.id));
            }
            if group.bounds.width <= 0.0 || group.bounds.height <= 0.0 {
                errors.push("Group bounds must be positive".to_string());
            }
        }
        RenderScenePatch::DeleteGroup { id } => {
            if !has_group(id) {
                errors.push(format!("Unknown group id: {id}"));
            }
        }
        RenderScenePatch::MoveGroup { id, .. } => {
            if !has_group(id) {
                errors.push(format!("Unknown group id: {id}"));
            }
        }
        RenderScenePatch::CreateCard { card } => {
            if has_node(&card.id) {
                errors.push(format!("Duplicate node id: {}", card.id));
            }
            if !has_group(&card.group_id) {
                errors.push(format!("Unknown group id: {}", card.group_id));
            }
            if card.bounds.width <= 0.0 || card.bounds.height <= 0.0 {
                errors.push("Card bounds must be positive".to_string());
            }
        }
        RenderScenePatch::DeleteCard { id } => {
            if !has_node(id) {
                errors.push(format!("Unknown node id: {id}"));
            }
        }
        RenderScenePatch::CreateEdge {
            source,
            target,
            group_id,
            edge_id,
            ..
        } => {
            if source == target {
                errors.push("Edge source and target must differ".to_string());
            }
            if !has_node(source) {
                errors.push(format!("Unknown source node id: {source}"));
            }
            if !has_node(target) {
                errors.push(format!("Unknown target node id: {target}"));
            }
            if !has_group(group_id) {
                errors.push(format!("Unknown group id: {group_id}"));
            }
            if has_edge(edge_id) {
                errors.push(format!("Duplicate edge id: {edge_id}"));
            }
        }
        RenderScenePatch::DeleteEdge { id } => {
            if !has_edge(id) {
                errors.push(format!("Unknown edge id: {id}"));
            }
        }
        RenderScenePatch::Select { selection } => {
            errors.extend(validate_scene_selection(scene, selection));
        }
        RenderScenePatch::ResizeCard { id, bounds } => {
            if !has_node(id) {
                errors.push(format!("Unknown node id: {id}"));
            }
            if bounds.width <= 0.0 || bounds.height <= 0.0 {
                errors.push("resize-card bounds must be positive".to_string());
            }
        }
        RenderScenePatch::ResizeGroup { id, bounds } => {
            if !has_group(id) {
                errors.push(format!("Unknown group id: {id}"));
            }
            if bounds.width <= 0.0 || bounds.height <= 0.0 {
                errors.push("resize-group bounds must be positive".to_string());
            }
        }
        RenderScenePatch::AlignCards { ids, .. } => {
            if ids.len() < 2 {
                errors.push("align-cards requires at least 2 ids".to_string());
            }
            for id in ids {
                if !has_node(id) {
                    errors.push(format!("Unknown node id: {id}"));
                }
            }
        }
        RenderScenePatch::DistributeCards { ids, .. } => {
            if ids.len() < 3 {
                errors.push("distribute-cards requires at least 3 ids".to_string());
            }
            for id in ids {
                if !has_node(id) {
                    errors.push(format!("Unknown node id: {id}"));
                }
            }
        }
        RenderScenePatch::DuplicateObjects { ids, .. } => {
            if ids.is_empty() {
                errors.push("duplicate-objects requires at least 1 id".to_string());
            }
            for id in ids {
                if !has_node(id) && !has_group(id) {
                    errors.push(format!("Unknown id: {id}"));
                }
            }
        }
        RenderScenePatch::Batch { ops } => {
            if ops.is_empty() {
                errors.push("batch requires at least 1 op".to_string());
            }
        }
        RenderScenePatch::GroupObjects {
            ids,
            frame_id,
            parent_group_id,
            ..
        } => {
            if ids.is_empty() {
                errors.push("group-objects requires at least 1 id".to_string());
            }
            if frame_id.is_empty() {
                errors.push("group-objects requires a frameId".to_string());
            }
            if has_group(frame_id) {
                errors.push(format!("Duplicate group id: {frame_id}"));
            }
            for id in ids {
                if !has_node(id) && !has_group(id) {
                    errors.push(format!("Unknown id: {id}"));
                }
            }
            if let Some(parent) = parent_group_id {
                if !has_group(parent) {
                    errors.push(format!("Unknown parentGroupId: {parent}"));
                }
            }
        }
        RenderScenePatch::Ungroup { id } => {
            if !has_group(id) {
                errors.push(format!("Unknown group id: {id}"));
            }
        }
        RenderScenePatch::SetObjectGroup { ids, frame_id } => {
            if ids.is_empty() {
                errors.push("set-object-group requires at least 1 id".to_string());
            }
            if !has_group(frame_id) {
                errors.push(format!("Unknown group id: {frame_id}"));
            }
            for id in ids {
                if !has_node(id) && !has_edge(id) {
                    errors.push(format!("Unknown id: {id}"));
                }
            }
        }
        RenderScenePatch::SetObjectTags {
            target_kind,
            id,
            tag_ids,
        } => {
            match target_kind {
                TargetKind::Frame => {
                    if !has_group(id) {
                        errors.push(format!("Unknown group id: {id}"));
                    }
                }
                TargetKind::Card => {
                    if !has_node(id) {
                        errors.push(format!("Unknown node id: {id}"));
                    }
                }
                TargetKind::Edge => {
                    if !has_edge(id) {
                        errors.push(format!("Unknown edge id: {id}"));
                    }
                }
            }
            let known_tag_ids: HashSet<&String> = scene.tags.iter().map(|t| &t.id).collect();
            for tag_id in tag_ids {
                if !known_tag_ids.contains(tag_id) {
                    errors.push(format!("Unknown tag id: {tag_id}"));
                }
            }
        }
        RenderScenePatch::CreateTag { tag } => {
            if scene.tags.iter().any(|t| t.id == tag.id) {
                errors.push(format!("Duplicate tag id: {}", tag.id));
            }
        }
    }
    errors
}

fn validate_scene_selection(scene: &Scene, selection: &SceneSelection) -> Vec<String> {
    match selection {
        SceneSelection::Canvas => vec![],
        SceneSelection::Group { id } => {
            if scene.groups.iter().any(|g| &g.id == id) {
                vec![]
            } else {
                vec![format!("Unknown group selection id: {id}")]
            }
        }
        SceneSelection::Node { id } => {
            if scene.nodes.iter().any(|n| &n.id == id) {
                vec![]
            } else {
                vec![format!("Unknown node selection id: {id}")]
            }
        }
        SceneSelection::Edge { id } => {
            if scene.edges.iter().any(|e| &e.id == id) {
                vec![]
            } else {
                vec![format!("Unknown edge selection id: {id}")]
            }
        }
        SceneSelection::Multi { ids } => {
            match ids.iter().find(|id| !scene.nodes.iter().any(|n| &n.id == *id)) {
                Some(unknown) => vec![format!("Unknown node selection id: {unknown}")],
                None => vec![],
            }
        }
    }
}

// ---------------------------------------------------------------------------
// lookup helpers (post-validation, existence guaranteed)
// ---------------------------------------------------------------------------

fn find_node<'a>(scene: &'a Scene, id: &str) -> &'a SceneNode {
    scene.nodes.iter().find(|n| n.id == id).unwrap()
}

fn find_group<'a>(scene: &'a Scene, id: &str) -> &'a SceneGroup {
    scene.groups.iter().find(|g| g.id == id).unwrap()
}

fn find_edge<'a>(scene: &'a Scene, id: &str) -> &'a SceneEdge {
    scene.edges.iter().find(|e| e.id == id).unwrap()
}
