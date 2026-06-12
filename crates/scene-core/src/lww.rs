//! Server-authoritative per-property LWW: `Map<ObjectId, Map<Property, LwwEntry>>`,
//! an independent winner per property. The authority token is the server's
//! monotonic arrival `seq`; the higher `seq` wins. `revision` (the op's
//! `baseRevision`) is informational and does NOT participate in ordering. Pure:
//! `seq` is caller-injected.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

/// `seq` is authoritative (server arrival order); `revision` is informational
/// only, never used to decide the winner.
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

    /// Strictly higher arrival seq wins; equal seq does NOT win (idempotent
    /// re-apply must not flip).
    pub fn wins_over(&self, other: &LwwToken) -> bool {
        self.seq > other.seq
    }
}

/// `true` for the first writer or a strictly-later arrival; equal `seq` returns
/// `false` so replaying the same op is a no-op.
pub fn lww_merge_property(current: Option<&LwwEntry>, incoming: &LwwEntry) -> bool {
    match current {
        None => true,
        Some(cur) => incoming.seq > cur.seq,
    }
}

/// `BTreeMap` keeps a deterministic key order so serialization is stable (canvas
/// position order is carried by the scene model / fractional index, not here).
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

    /// Writes `value` iff `seq` strictly exceeds the seq already stored there
    /// (or none is). Returns `true` when the incoming write won.
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

    pub fn get(&self, object_id: &str, property: &str) -> Option<&LwwEntry> {
        self.objects.get(object_id).and_then(|p| p.get(property))
    }

    pub fn get_value(&self, object_id: &str, property: &str) -> Option<&serde_json::Value> {
        self.get(object_id, property).map(|e| &e.value)
    }

    pub fn object(&self, object_id: &str) -> Option<&BTreeMap<String, LwwEntry>> {
        self.objects.get(object_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert!(store.apply("obj1", "title", json!("bye"), 3));
        assert_eq!(store.get_value("obj1", "x"), Some(&json!(42)));
        assert_eq!(store.object("obj1").unwrap().len(), 2);
    }

    #[test]
    fn token_ordering_matches_seq() {
        let a = LwwToken::new(7, 2);
        let b = LwwToken::new(99, 1);
        assert!(a.wins_over(&b));
        assert!(!b.wins_over(&a));
    }

    #[test]
    fn equal_seq_does_not_flip() {
        let current = LwwEntry::new(json!("original"), 4);
        let incoming = LwwEntry::new(json!("replayed"), 4);
        assert!(!lww_merge_property(Some(&current), &incoming));
        assert!(!LwwToken::new(0, 4).wins_over(&LwwToken::new(0, 4)));

        let mut store = PropertyStore::new();
        assert!(store.apply("obj1", "title", json!("original"), 4));
        assert!(!store.apply("obj1", "title", json!("replayed"), 4));
        assert_eq!(store.get_value("obj1", "title"), Some(&json!("original")));
    }

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
