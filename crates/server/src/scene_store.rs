//! Per-object scene <-> Record mapping (MG5.1).
//!
//! The actor used to checkpoint a whole canvas as one `Record`; MG-5 makes the
//! per-object `Record` the canonical persisted unit so the store can index
//! objects spatially (via [`SpatialStore`](shape_storage_core::SpatialStore)) and
//! later window large canvases by region. This module is the *pure* mapping seam:
//!
//! * [`scene_to_records`] explodes a [`Scene`] into one `Record` per object
//!   (`group`/`node`/`edge`/`tag`/`comment`/`artifact`) plus one small
//!   `canvas-meta` `Record` carrying the scene-level fields that are not tied to a
//!   single object (`version`, `sceneVersion`, `selection`, `updatedAt`,
//!   `proposals`). Placement-bearing objects (groups, nodes, and edges whose
//!   endpoints are both present) also get a [`RegionKey`] so region queries work.
//! * [`records_to_scene`] reconstructs a [`Scene`] from those records.
//!
//! ## Canonical ordering
//!
//! A [`Scene`]'s collections are `Vec`s, so `Scene` equality is order-sensitive,
//! but the storage layer addresses objects by id and yields them id-sorted. The
//! mapping therefore defines a **canonical order**: within each kind, objects are
//! sorted by id. [`scene_to_records`] is order-independent (it keys by id), and
//! [`records_to_scene`] always emits id-sorted-within-kind collections. To compare
//! a live scene against a reconstructed one, normalize the live scene first with
//! [`canonicalize_scene`]; round-tripping a canonical scene is the identity.

use serde::{Deserialize, Serialize};
use shape_scene_core::{
    node_bounds, CanvasId, Scene, SceneArtifact, SceneComment, SceneEdge, SceneGroup, SceneNode,
    SceneProposal, SceneSelection, Tag,
};
use shape_storage_core::{Record, RegionKey};

/// Record `kind` for each per-object Record. The scene-level fields that do not
/// belong to any one object live in the `canvas-meta` Record.
pub const KIND_GROUP: &str = "group";
pub const KIND_NODE: &str = "node";
pub const KIND_EDGE: &str = "edge";
pub const KIND_TAG: &str = "tag";
pub const KIND_COMMENT: &str = "comment";
pub const KIND_ARTIFACT: &str = "artifact";
pub const KIND_CANVAS_META: &str = "canvas-meta";

/// The scene-level fields that are not attached to any single object. Persisted
/// as the single `canvas-meta` Record's payload so a reconstructed scene restores
/// `version`, `sceneVersion`, `selection`, `updatedAt`, and `proposals` exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasMeta {
    pub version: i64,
    pub scene_version: i64,
    pub selection: SceneSelection,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposals: Option<Vec<SceneProposal>>,
}

/// The Record id for a per-object Record: `"{canvasId}:{kind}:{objId}"`.
pub fn object_record_id(canvas_id: &CanvasId, kind: &str, object_id: &str) -> String {
    format!("{canvas_id}:{kind}:{object_id}")
}

/// The Record id for the canvas-meta Record: `"{canvasId}:canvas-meta"`.
pub fn canvas_meta_record_id(canvas_id: &CanvasId) -> String {
    format!("{canvas_id}:canvas-meta")
}

/// The id-prefix that scopes every Record belonging to `canvas_id`: `"{canvasId}:"`.
///
/// Storage `list()`/`records()` are global id-sorted, so a prefix scan enumerates
/// exactly one canvas's per-object + canvas-meta records (the journal records
/// share the `"{canvasId}:journal:"` sub-prefix and are filtered out by kind).
pub fn canvas_record_prefix(canvas_id: &CanvasId) -> String {
    format!("{canvas_id}:")
}

/// The [`RegionKey`] for a group: its bounds, as an axis-aligned bbox.
fn group_region(canvas_id: &CanvasId, group: &SceneGroup) -> RegionKey {
    RegionKey {
        canvas_id: canvas_id.to_string(),
        min_x: group.bounds.x,
        min_y: group.bounds.y,
        max_x: group.bounds.x + group.bounds.width,
        max_y: group.bounds.y + group.bounds.height,
    }
}

/// The [`RegionKey`] for a node: its position + size, as an axis-aligned bbox.
fn node_region(canvas_id: &CanvasId, node: &SceneNode) -> RegionKey {
    let b = node_bounds(node);
    RegionKey {
        canvas_id: canvas_id.to_string(),
        min_x: b.x,
        min_y: b.y,
        max_x: b.x + b.width,
        max_y: b.y + b.height,
    }
}

/// The [`RegionKey`] for an edge: the bbox hull of its two endpoint nodes, if
/// both are present in the scene. Returns `None` when an endpoint is missing
/// (the edge stays un-indexed but is still persisted).
fn edge_region(canvas_id: &CanvasId, edge: &SceneEdge, nodes: &[SceneNode]) -> Option<RegionKey> {
    let src = nodes.iter().find(|n| n.id == edge.source)?;
    let dst = nodes.iter().find(|n| n.id == edge.target)?;
    let sb = node_bounds(src);
    let db = node_bounds(dst);
    Some(RegionKey {
        canvas_id: canvas_id.to_string(),
        min_x: sb.x.min(db.x),
        min_y: sb.y.min(db.y),
        max_x: (sb.x + sb.width).max(db.x + db.width),
        max_y: (sb.y + sb.height).max(db.y + db.height),
    })
}

/// Explode a [`Scene`] into one [`Record`] per object plus a `canvas-meta`
/// Record, each paired with its optional [`RegionKey`].
///
/// Record ids are namespaced `"{canvasId}:{kind}:{objId}"` so they stay globally
/// unique. `version` is stamped on every Record by the caller: the actor passes
/// the server seq at write time, which doubles as the recovery floor read back
/// from the `canvas-meta` Record (see [`records_to_scene`] and the actor's
/// checkpoint path). Region keys are produced for groups, nodes, and edges with
/// both endpoints present; tags, comments, artifacts, and canvas-meta carry
/// `None`.
pub fn scene_to_records(
    canvas_id: &CanvasId,
    scene: &Scene,
    version: u64,
) -> Vec<(Record, Option<RegionKey>)> {
    let mut out: Vec<(Record, Option<RegionKey>)> = Vec::new();

    for group in &scene.groups {
        out.push((
            Record {
                id: object_record_id(canvas_id, KIND_GROUP, &group.id),
                kind: KIND_GROUP.to_string(),
                version,
                payload: serde_json::to_vec(group).expect("group serializes"),
            },
            Some(group_region(canvas_id, group)),
        ));
    }

    for node in &scene.nodes {
        out.push((
            Record {
                id: object_record_id(canvas_id, KIND_NODE, &node.id),
                kind: KIND_NODE.to_string(),
                version,
                payload: serde_json::to_vec(node).expect("node serializes"),
            },
            Some(node_region(canvas_id, node)),
        ));
    }

    for edge in &scene.edges {
        out.push((
            Record {
                id: object_record_id(canvas_id, KIND_EDGE, &edge.id),
                kind: KIND_EDGE.to_string(),
                version,
                payload: serde_json::to_vec(edge).expect("edge serializes"),
            },
            edge_region(canvas_id, edge, &scene.nodes),
        ));
    }

    for tag in &scene.tags {
        out.push((
            Record {
                id: object_record_id(canvas_id, KIND_TAG, &tag.id),
                kind: KIND_TAG.to_string(),
                version,
                payload: serde_json::to_vec(tag).expect("tag serializes"),
            },
            None,
        ));
    }

    for comment in &scene.comments {
        out.push((
            Record {
                id: object_record_id(canvas_id, KIND_COMMENT, &comment.id),
                kind: KIND_COMMENT.to_string(),
                version,
                payload: serde_json::to_vec(comment).expect("comment serializes"),
            },
            None,
        ));
    }

    for artifact in &scene.artifacts {
        out.push((
            Record {
                id: object_record_id(canvas_id, KIND_ARTIFACT, &artifact.id),
                kind: KIND_ARTIFACT.to_string(),
                version,
                payload: serde_json::to_vec(artifact).expect("artifact serializes"),
            },
            None,
        ));
    }

    let meta = CanvasMeta {
        version: scene.version,
        scene_version: scene.scene_version,
        selection: scene.selection.clone(),
        updated_at: scene.updated_at.clone(),
        proposals: scene.proposals.clone(),
    };
    out.push((
        Record {
            id: canvas_meta_record_id(canvas_id),
            kind: KIND_CANVAS_META.to_string(),
            version,
            payload: serde_json::to_vec(&meta).expect("canvas-meta serializes"),
        },
        None,
    ));

    out
}

/// Reconstruct a [`Scene`] from per-object Records + the canvas-meta Record.
///
/// `records` is any iterator over this canvas's Records (e.g. a prefix scan of
/// the store). Records whose `kind` is not one of the object/meta kinds (e.g.
/// `journal`) are ignored, so a raw prefix scan can be passed straight through.
/// Each kind's collection is emitted in **canonical (id-sorted) order**. When no
/// `canvas-meta` Record is present (a brand-new canvas), scene-level defaults are
/// used (`version: 1`, `sceneVersion: 0`, selection `Canvas`, empty `updatedAt`).
pub fn records_to_scene<I: IntoIterator<Item = Record>>(_canvas_id: &CanvasId, records: I) -> Scene {
    let mut groups: Vec<SceneGroup> = Vec::new();
    let mut nodes: Vec<SceneNode> = Vec::new();
    let mut edges: Vec<SceneEdge> = Vec::new();
    let mut tags: Vec<Tag> = Vec::new();
    let mut comments: Vec<SceneComment> = Vec::new();
    let mut artifacts: Vec<SceneArtifact> = Vec::new();
    let mut meta: Option<CanvasMeta> = None;

    for record in records {
        match record.kind.as_str() {
            KIND_GROUP => groups.push(decode(&record)),
            KIND_NODE => nodes.push(decode(&record)),
            KIND_EDGE => edges.push(decode(&record)),
            KIND_TAG => tags.push(decode(&record)),
            KIND_COMMENT => comments.push(decode(&record)),
            KIND_ARTIFACT => artifacts.push(decode(&record)),
            KIND_CANVAS_META => meta = Some(decode(&record)),
            _ => {}
        }
    }

    groups.sort_by(|a, b| a.id.cmp(&b.id));
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    tags.sort_by(|a, b| a.id.cmp(&b.id));
    comments.sort_by(|a, b| a.id.cmp(&b.id));
    artifacts.sort_by(|a, b| a.id.cmp(&b.id));

    let meta = meta.unwrap_or(CanvasMeta {
        version: 1,
        scene_version: 0,
        selection: SceneSelection::Canvas,
        updated_at: String::new(),
        proposals: None,
    });

    Scene {
        version: meta.version,
        scene_version: meta.scene_version,
        groups,
        nodes,
        edges,
        tags,
        comments,
        artifacts,
        proposals: meta.proposals,
        selection: meta.selection,
        updated_at: meta.updated_at,
    }
}

/// Normalize a [`Scene`] into the canonical order [`records_to_scene`] produces
/// (each kind id-sorted). Use this to compare a live scene against a
/// reconstructed one — round-tripping a canonical scene is the identity.
pub fn canonicalize_scene(scene: &Scene) -> Scene {
    let mut out = scene.clone();
    out.groups.sort_by(|a, b| a.id.cmp(&b.id));
    out.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    out.edges.sort_by(|a, b| a.id.cmp(&b.id));
    out.tags.sort_by(|a, b| a.id.cmp(&b.id));
    out.comments.sort_by(|a, b| a.id.cmp(&b.id));
    out.artifacts.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Deserialize a Record's payload into `T`, panicking on malformed data (a
/// persisted object that fails to decode is a corruption the actor cannot
/// continue past — same contract as the journal/checkpoint decode).
fn decode<T: for<'de> Deserialize<'de>>(record: &Record) -> T {
    serde_json::from_slice(&record.payload)
        .unwrap_or_else(|e| panic!("record {} ({}) deserializes: {e}", record.id, record.kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::{
        Bounds, EdgeType, ExportType, NodeStatus, NodeType, Point, ProposalStatus, Size,
    };

    fn group(id: &str, x: f64, y: f64) -> SceneGroup {
        SceneGroup {
            id: id.to_string(),
            parent_group_id: None,
            title: format!("group {id}"),
            summary: String::new(),
            bounds: Bounds { x, y, width: 100.0, height: 80.0 },
            tag_ids: vec![],
            z_index: 0.0,
            collapsed: false,
            created_at: "t0".to_string(),
            updated_at: "t0".to_string(),
            meta: None,
        }
    }

    fn node(id: &str, group_id: &str, x: f64, y: f64) -> SceneNode {
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
            size: Size { width: 50.0, height: 40.0 },
            z_index: 0.0,
            tag_ids: vec![],
            updated_at: None,
            meta: None,
        }
    }

    fn edge(id: &str, group_id: &str, source: &str, target: &str) -> SceneEdge {
        SceneEdge {
            id: id.to_string(),
            edge_type: EdgeType::DependsOn,
            source: source.to_string(),
            target: target.to_string(),
            label: String::new(),
            rationale: String::new(),
            confidence: 0.5,
            group_id: group_id.to_string(),
            tag_ids: vec![],
            updated_at: None,
            meta: None,
        }
    }

    fn tag(id: &str) -> Tag {
        Tag {
            id: id.to_string(),
            name: format!("tag {id}"),
            color: "#fff".to_string(),
            description: String::new(),
            created_at: "t0".to_string(),
            updated_at: "t0".to_string(),
        }
    }

    fn comment(id: &str, target: SceneSelection) -> SceneComment {
        SceneComment {
            id: id.to_string(),
            target,
            body: format!("comment {id}"),
            author: "human".to_string(),
            resolved: false,
            created_at: "t0".to_string(),
            updated_at: "t0".to_string(),
        }
    }

    fn artifact(id: &str, target: SceneSelection) -> SceneArtifact {
        SceneArtifact {
            id: id.to_string(),
            export_type: ExportType::Madr,
            title: format!("artifact {id}"),
            target,
            path: format!("/exports/{id}.md"),
            content_type: "text/markdown".to_string(),
            created_at: "t0".to_string(),
            scene_version: 3,
        }
    }

    /// A non-trivial scene: two groups, three nodes, an edge, tags, a comment, an
    /// artifact, a non-default selection + proposals, deliberately built in a
    /// non-id-sorted order so the canonical round-trip is exercised.
    fn sample_scene() -> Scene {
        Scene {
            version: 1,
            scene_version: 7,
            groups: vec![group("g2", 200.0, 0.0), group("g1", 0.0, 0.0)],
            nodes: vec![
                node("n3", "g2", 210.0, 10.0),
                node("n1", "g1", 10.0, 10.0),
                node("n2", "g1", 70.0, 10.0),
            ],
            edges: vec![edge("e1", "g1", "n1", "n2")],
            tags: vec![tag("t2"), tag("t1")],
            comments: vec![comment("c1", SceneSelection::Node { id: "n1".to_string() })],
            artifacts: vec![artifact("a1", SceneSelection::Group { id: "g1".to_string() })],
            proposals: Some(vec![SceneProposal {
                id: "p1".to_string(),
                actor_id: "agent-1".to_string(),
                status: ProposalStatus::Pending,
                operation: serde_json::json!({"kind": "noop"}),
                created_at: "t0".to_string(),
            }]),
            selection: SceneSelection::Group { id: "g1".to_string() },
            updated_at: "t-updated".to_string(),
        }
    }

    #[test]
    fn round_trip_reproduces_canonical_scene() {
        let canvas = CanvasId::from("c-round");
        let scene = sample_scene();

        let records: Vec<Record> =
            scene_to_records(&canvas, &scene, scene.scene_version as u64).into_iter().map(|(r, _)| r).collect();
        let rebuilt = records_to_scene(&canvas, records);

        assert_eq!(rebuilt, canonicalize_scene(&scene));
    }

    #[test]
    fn canonical_scene_round_trips_as_identity() {
        let canvas = CanvasId::from("c-id");
        let scene = canonicalize_scene(&sample_scene());

        let records: Vec<Record> =
            scene_to_records(&canvas, &scene, scene.scene_version as u64).into_iter().map(|(r, _)| r).collect();
        let rebuilt = records_to_scene(&canvas, records);

        assert_eq!(rebuilt, scene, "round-tripping a canonical scene is the identity");
    }

    #[test]
    fn one_record_per_object_plus_canvas_meta() {
        let canvas = CanvasId::from("c-count");
        let scene = sample_scene();
        let records = scene_to_records(&canvas, &scene, scene.scene_version as u64);

        // 2 groups + 3 nodes + 1 edge + 2 tags + 1 comment + 1 artifact + 1 meta.
        assert_eq!(records.len(), 11);
        let metas = records
            .iter()
            .filter(|(r, _)| r.kind == KIND_CANVAS_META)
            .count();
        assert_eq!(metas, 1, "exactly one canvas-meta record");

        // Ids are namespaced and the version is the scene revision.
        for (r, _) in &records {
            assert!(r.id.starts_with(&canvas_record_prefix(&canvas)));
            assert_eq!(r.version, scene.scene_version as u64);
        }
    }

    #[test]
    fn region_keys_for_placement_objects_only() {
        let canvas = CanvasId::from("c-region");
        let scene = sample_scene();
        let records = scene_to_records(&canvas, &scene, scene.scene_version as u64);

        let region_of = |kind: &str, id: &str| -> Option<RegionKey> {
            let want = object_record_id(&canvas, kind, id);
            records.iter().find(|(r, _)| r.id == want).unwrap().1.clone()
        };

        // Group bbox = its bounds.
        assert_eq!(
            region_of(KIND_GROUP, "g1"),
            Some(RegionKey {
                canvas_id: "c-region".to_string(),
                min_x: 0.0,
                min_y: 0.0,
                max_x: 100.0,
                max_y: 80.0,
            })
        );
        // Node bbox = position + size.
        assert_eq!(
            region_of(KIND_NODE, "n1"),
            Some(RegionKey {
                canvas_id: "c-region".to_string(),
                min_x: 10.0,
                min_y: 10.0,
                max_x: 60.0,
                max_y: 50.0,
            })
        );
        // Edge bbox = hull of n1 (10,10,50,40) and n2 (70,10,50,40).
        assert_eq!(
            region_of(KIND_EDGE, "e1"),
            Some(RegionKey {
                canvas_id: "c-region".to_string(),
                min_x: 10.0,
                min_y: 10.0,
                max_x: 120.0,
                max_y: 50.0,
            })
        );
        // Non-placement kinds carry no region.
        assert_eq!(region_of(KIND_TAG, "t1"), None);
        assert_eq!(region_of(KIND_COMMENT, "c1"), None);
        assert_eq!(region_of(KIND_ARTIFACT, "a1"), None);
    }

    #[test]
    fn edge_without_both_endpoints_has_no_region() {
        let canvas = CanvasId::from("c-dangling");
        let mut scene = sample_scene();
        // An edge whose target is absent from the scene's nodes.
        scene.edges.push(edge("e-dangling", "g1", "n1", "missing"));
        let records = scene_to_records(&canvas, &scene, scene.scene_version as u64);
        let want = object_record_id(&canvas, KIND_EDGE, "e-dangling");
        let region = records.iter().find(|(r, _)| r.id == want).unwrap().1.clone();
        assert_eq!(region, None);
    }

    #[test]
    fn empty_scene_round_trips_via_canvas_meta() {
        let canvas = CanvasId::from("c-empty");
        let scene = Scene {
            version: 1,
            scene_version: 0,
            groups: vec![],
            nodes: vec![],
            edges: vec![],
            tags: vec![],
            comments: vec![],
            artifacts: vec![],
            proposals: None,
            selection: SceneSelection::Canvas,
            updated_at: "t0".to_string(),
        };
        let records: Vec<Record> =
            scene_to_records(&canvas, &scene, scene.scene_version as u64).into_iter().map(|(r, _)| r).collect();
        // Only the canvas-meta record exists for an empty scene.
        assert_eq!(records.len(), 1);
        let rebuilt = records_to_scene(&canvas, records);
        assert_eq!(rebuilt, scene);
    }

    #[test]
    fn unknown_kinds_are_ignored_in_reconstruction() {
        let canvas = CanvasId::from("c-mixed");
        let scene = sample_scene();
        let mut records: Vec<Record> =
            scene_to_records(&canvas, &scene, scene.scene_version as u64).into_iter().map(|(r, _)| r).collect();
        // A journal-like record sharing the canvas prefix must be ignored.
        records.push(Record {
            id: format!("{canvas}:journal:99"),
            kind: "journal".to_string(),
            version: 99,
            payload: b"{}".to_vec(),
        });
        let rebuilt = records_to_scene(&canvas, records);
        assert_eq!(rebuilt, canonicalize_scene(&scene));
    }
}
