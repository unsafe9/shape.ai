//! Figma-style server-authoritative per-property LWW + standalone group
//! integrity validators (PC3 / Sync Technical Design, MG0.2b).
//!
//! A document is modeled as `Map<ObjectId, Map<Property, LwwEntry>>`: every
//! object carries an independent winner per property. The authority token is the
//! server's monotonic arrival sequence (`seq`) — the canvas actor assigns a
//! strictly increasing `seq` to each op as it serializes them (PC3: 타이브레이크
//! = 서버 monotonic seq(arrival)), and the entry with the higher `seq` wins.
//! `revision` (the op's `baseRevision`) rides along as informational metadata; it
//! does NOT participate in the ordering.
//!
//! Platform-pure: no clock / rng / IO. `seq` is injected by the caller, exactly
//! like `now: &str` elsewhere in scene-core.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One property's winning value plus the arrival sequence that won it.
///
/// `seq` is the server monotonic arrival sequence (authority token / tiebreak).
/// `value` is the opaque property payload, preserved verbatim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LwwEntry {
    pub value: serde_json::Value,
    pub seq: i64,
}

impl LwwEntry {
    pub fn new(value: serde_json::Value, seq: i64) -> Self {
        LwwEntry { value, seq }
    }
}

/// The authority token carried by an incoming change.
///
/// `seq` is authoritative (server arrival order). `revision` is the op's
/// `baseRevision` — informational only, never used to decide the winner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LwwToken {
    pub revision: i64,
    pub seq: i64,
}

impl LwwToken {
    pub fn new(revision: i64, seq: i64) -> Self {
        LwwToken { revision, seq }
    }

    /// `true` if `self` outranks `other` — strictly higher arrival seq wins.
    /// Equal seq does NOT win (idempotent re-apply must not flip).
    pub fn wins_over(&self, other: &LwwToken) -> bool {
        self.seq > other.seq
    }
}

/// Decide whether an incoming entry beats the currently-held one.
///
/// `true` when there is no current entry (first writer) or the incoming entry
/// arrived strictly later (`incoming.seq > current.seq`). Equal `seq` returns
/// `false`, so replaying the same op is a no-op (idempotent).
pub fn lww_merge_property(current: Option<&LwwEntry>, incoming: &LwwEntry) -> bool {
    match current {
        None => true,
        Some(cur) => incoming.seq > cur.seq,
    }
}

/// Per-property LWW document store: `ObjectId -> Property -> LwwEntry`.
///
/// `BTreeMap` keeps a deterministic key order so serialization and test
/// assertions are stable across runs (the actual canvas position order is
/// carried by the scene model / fractional index, not by this store).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PropertyStore {
    pub objects: BTreeMap<String, BTreeMap<String, LwwEntry>>,
}

impl PropertyStore {
    pub fn new() -> Self {
        PropertyStore {
            objects: BTreeMap::new(),
        }
    }

    /// Apply one property write under the LWW rule.
    ///
    /// Keeps the winner per property: writes `value` at `(object_id, property)`
    /// iff `seq` strictly exceeds the seq already stored there (or none is).
    /// Returns `true` when the incoming write won and the store changed.
    pub fn apply(
        &mut self,
        object_id: &str,
        property: &str,
        value: serde_json::Value,
        seq: i64,
    ) -> bool {
        let incoming = LwwEntry::new(value, seq);
        let props = self.objects.entry(object_id.to_string()).or_default();
        if lww_merge_property(props.get(property), &incoming) {
            props.insert(property.to_string(), incoming);
            true
        } else {
            false
        }
    }

    /// Read the current winning entry for a property, if any.
    pub fn get(&self, object_id: &str, property: &str) -> Option<&LwwEntry> {
        self.objects.get(object_id).and_then(|p| p.get(property))
    }

    /// Read the current winning value for a property, if any.
    pub fn get_value(&self, object_id: &str, property: &str) -> Option<&serde_json::Value> {
        self.get(object_id, property).map(|e| &e.value)
    }

    /// All properties currently held for an object, if it exists.
    pub fn object(&self, object_id: &str) -> Option<&BTreeMap<String, LwwEntry>> {
        self.objects.get(object_id)
    }
}

// ---------------------------------------------------------------------------
// Group integrity validators (standalone, reusable).
//
// These port the inline checks scattered through apply.rs (group-objects /
// create-group / create-card) into whole-scene validators a sync server can run
// on a candidate scene before journaling, and extend them with a full ancestor
// cycle check.
// ---------------------------------------------------------------------------

use crate::model::Scene;

/// No frame may be its own ancestor through `parentGroupId`.
///
/// Walks each group's parent chain; if the chain revisits the start group (or
/// any already-seen group, i.e. a cycle it participates in), the start group is
/// reported. One message per group that sits on a cycle.
pub fn validate_no_group_cycle(scene: &Scene) -> Vec<String> {
    let mut errors = Vec::new();
    let parent_of: BTreeMap<&str, Option<&str>> = scene
        .groups
        .iter()
        .map(|g| (g.id.as_str(), g.parent_group_id.as_deref()))
        .collect();

    for group in &scene.groups {
        let start = group.id.as_str();
        let mut seen: Vec<&str> = vec![start];
        let mut cursor = parent_of.get(start).copied().flatten();
        loop {
            match cursor {
                None => break,
                Some(parent) => {
                    if seen.contains(&parent) {
                        // `start` reaches a node it already passed through, so it
                        // lies on a cycle.
                        errors.push(format!("Group cycle detected at: {start}"));
                        break;
                    }
                    seen.push(parent);
                    cursor = parent_of.get(parent).copied().flatten();
                }
            }
        }
    }
    errors
}

/// Every group reference must resolve to an existing group:
/// `node.groupId`, `edge.groupId`, and `group.parentGroupId`.
pub fn validate_group_targets(scene: &Scene) -> Vec<String> {
    let mut errors = Vec::new();
    let group_ids: std::collections::HashSet<&str> =
        scene.groups.iter().map(|g| g.id.as_str()).collect();

    for group in &scene.groups {
        if let Some(parent) = &group.parent_group_id {
            if !group_ids.contains(parent.as_str()) {
                errors.push(format!(
                    "Group {} references unknown parentGroupId: {parent}",
                    group.id
                ));
            }
        }
    }
    for node in &scene.nodes {
        if !group_ids.contains(node.group_id.as_str()) {
            errors.push(format!(
                "Node {} references unknown groupId: {}",
                node.id, node.group_id
            ));
        }
    }
    for edge in &scene.edges {
        if !group_ids.contains(edge.group_id.as_str()) {
            errors.push(format!(
                "Edge {} references unknown groupId: {}",
                edge.id, edge.group_id
            ));
        }
    }
    errors
}

/// Every group's bounds must have strictly positive width and height.
pub fn validate_bounds_positive(scene: &Scene) -> Vec<String> {
    let mut errors = Vec::new();
    for group in &scene.groups {
        if group.bounds.width <= 0.0 || group.bounds.height <= 0.0 {
            errors.push(format!("Group {} bounds must be positive", group.id));
        }
    }
    errors
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bounds, Scene, SceneGroup, SceneSelection};
    use serde_json::json;

    fn group(id: &str, parent: Option<&str>, w: f64, h: f64) -> SceneGroup {
        SceneGroup {
            id: id.to_string(),
            parent_group_id: parent.map(|p| p.to_string()),
            title: id.to_string(),
            summary: String::new(),
            bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: w,
                height: h,
            },
            tag_ids: vec![],
            z_index: 0.0,
            collapsed: false,
            created_at: "t0".to_string(),
            updated_at: "t0".to_string(),
            meta: None,
        }
    }

    fn scene_with_groups(groups: Vec<SceneGroup>) -> Scene {
        Scene {
            version: 1,
            scene_version: 0,
            groups,
            nodes: vec![],
            edges: vec![],
            tags: vec![],
            comments: vec![],
            artifacts: vec![],
            proposals: None,
            selection: SceneSelection::Canvas,
            updated_at: "t0".to_string(),
        }
    }

    // ---- LWW winner selection ------------------------------------------------

    #[test]
    fn merge_into_empty_always_wins() {
        let incoming = LwwEntry::new(json!("a"), 1);
        assert!(lww_merge_property(None, &incoming));
    }

    #[test]
    fn higher_seq_wins() {
        let current = LwwEntry::new(json!("a"), 5);
        let incoming = LwwEntry::new(json!("b"), 6);
        assert!(lww_merge_property(Some(&current), &incoming));

        let mut store = PropertyStore::new();
        assert!(store.apply("obj1", "title", json!("a"), 5));
        assert!(store.apply("obj1", "title", json!("b"), 6));
        assert_eq!(store.get_value("obj1", "title"), Some(&json!("b")));
        assert_eq!(store.get("obj1", "title").unwrap().seq, 6);
    }

    #[test]
    fn lower_seq_loses() {
        let current = LwwEntry::new(json!("winner"), 10);
        let incoming = LwwEntry::new(json!("stale"), 3);
        assert!(!lww_merge_property(Some(&current), &incoming));

        let mut store = PropertyStore::new();
        assert!(store.apply("obj1", "x", json!("winner"), 10));
        assert!(!store.apply("obj1", "x", json!("stale"), 3));
        assert_eq!(store.get_value("obj1", "x"), Some(&json!("winner")));
    }

    #[test]
    fn distinct_properties_are_independent() {
        let mut store = PropertyStore::new();
        store.apply("obj1", "title", json!("hi"), 1);
        store.apply("obj1", "x", json!(42), 2);
        assert_eq!(store.get_value("obj1", "title"), Some(&json!("hi")));
        assert_eq!(store.get_value("obj1", "x"), Some(&json!(42)));
        // A later write to one property leaves the sibling untouched.
        assert!(store.apply("obj1", "title", json!("bye"), 3));
        assert_eq!(store.get_value("obj1", "x"), Some(&json!(42)));
        assert_eq!(store.object("obj1").unwrap().len(), 2);
    }

    #[test]
    fn token_ordering_matches_seq() {
        let a = LwwToken::new(7, 2);
        let b = LwwToken::new(99, 1);
        // Higher seq wins even though b has a much higher (informational) revision.
        assert!(a.wins_over(&b));
        assert!(!b.wins_over(&a));
    }

    // ---- idempotent re-apply (equal seq does not flip) -----------------------

    #[test]
    fn equal_seq_does_not_flip() {
        let current = LwwEntry::new(json!("original"), 4);
        let incoming = LwwEntry::new(json!("replayed"), 4);
        assert!(!lww_merge_property(Some(&current), &incoming));
        assert!(!LwwToken::new(0, 4).wins_over(&LwwToken::new(0, 4)));

        let mut store = PropertyStore::new();
        assert!(store.apply("obj1", "title", json!("original"), 4));
        // Re-applying the same op (same seq) must be a no-op and keep the value.
        assert!(!store.apply("obj1", "title", json!("replayed"), 4));
        assert_eq!(store.get_value("obj1", "title"), Some(&json!("original")));
    }

    // ---- cycle detection -----------------------------------------------------

    #[test]
    fn acyclic_parent_chain_is_clean() {
        // root <- child <- grandchild, plus a sibling at root.
        let scene = scene_with_groups(vec![
            group("root", None, 100.0, 100.0),
            group("child", Some("root"), 50.0, 50.0),
            group("grandchild", Some("child"), 20.0, 20.0),
            group("sibling", Some("root"), 30.0, 30.0),
        ]);
        assert!(validate_no_group_cycle(&scene).is_empty());
    }

    #[test]
    fn two_frame_cycle_is_detected() {
        // a -> b -> a
        let scene = scene_with_groups(vec![
            group("a", Some("b"), 100.0, 100.0),
            group("b", Some("a"), 100.0, 100.0),
        ]);
        let errors = validate_no_group_cycle(&scene);
        // Both frames sit on the cycle, so both are reported.
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.contains("a")));
        assert!(errors.iter().any(|e| e.contains("b")));
    }

    #[test]
    fn self_parent_cycle_is_detected() {
        let scene = scene_with_groups(vec![group("solo", Some("solo"), 10.0, 10.0)]);
        let errors = validate_no_group_cycle(&scene);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("solo"));
    }

    // ---- target validation ---------------------------------------------------

    #[test]
    fn unknown_group_targets_are_reported() {
        let mut scene = scene_with_groups(vec![group("g1", None, 100.0, 100.0)]);
        // node references missing group; edge references missing group;
        // group references missing parent.
        scene.nodes.push(crate::model::SceneNode {
            id: "n1".to_string(),
            node_type: crate::model::NodeType::Task,
            title: "n".to_string(),
            summary: String::new(),
            detail: String::new(),
            status: crate::model::NodeStatus::Draft,
            confidence: 0.5,
            evidence_refs: vec![],
            child_decision_ids: vec![],
            group_id: "ghost".to_string(),
            position: crate::model::Point { x: 0.0, y: 0.0 },
            size: crate::model::Size {
                width: 10.0,
                height: 10.0,
            },
            z_index: 0.0,
            tag_ids: vec![],
            updated_at: None,
            meta: None,
        });
        scene.groups.push(group("g2", Some("missing-parent"), 10.0, 10.0));

        let errors = validate_group_targets(&scene);
        assert!(errors.iter().any(|e| e.contains("unknown groupId: ghost")));
        assert!(errors
            .iter()
            .any(|e| e.contains("unknown parentGroupId: missing-parent")));
    }

    #[test]
    fn valid_targets_pass() {
        let scene = scene_with_groups(vec![
            group("root", None, 100.0, 100.0),
            group("child", Some("root"), 50.0, 50.0),
        ]);
        assert!(validate_group_targets(&scene).is_empty());
    }

    // ---- bounds validation ---------------------------------------------------

    #[test]
    fn non_positive_bounds_are_reported() {
        let scene = scene_with_groups(vec![
            group("ok", None, 10.0, 10.0),
            group("zero-w", None, 0.0, 10.0),
            group("neg-h", None, 10.0, -5.0),
        ]);
        let errors = validate_bounds_positive(&scene);
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.contains("zero-w")));
        assert!(errors.iter().any(|e| e.contains("neg-h")));
    }

    #[test]
    fn positive_bounds_pass() {
        let scene = scene_with_groups(vec![group("ok", None, 1.0, 1.0)]);
        assert!(validate_bounds_positive(&scene).is_empty());
    }

    // ---- serde shape ---------------------------------------------------------

    #[test]
    fn lww_entry_serializes_camel_case() {
        let entry = LwwEntry::new(json!({"nested": true}), 7);
        let s = serde_json::to_string(&entry).unwrap();
        assert!(s.contains("\"value\""));
        assert!(s.contains("\"seq\":7"));
        let back: LwwEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn lww_token_round_trips() {
        let token = LwwToken::new(12, 34);
        let s = serde_json::to_string(&token).unwrap();
        assert!(s.contains("\"revision\":12"));
        assert!(s.contains("\"seq\":34"));
        let back: LwwToken = serde_json::from_str(&s).unwrap();
        assert_eq!(back, token);
    }
}
