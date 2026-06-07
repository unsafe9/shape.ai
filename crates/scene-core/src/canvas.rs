//! Canvas / document concept (PC9).
//!
//! A **canvas** is the unit of document, sync, actor, lease, and routing in
//! shape.ai. It owns exactly one [`Scene`](crate::model::Scene) at runtime.
//!
//! Per the PC9 design decision, the in-memory `Scene` stays byte-identical to
//! the legacy TS `schema.ts` model and deliberately does **not** carry a
//! `canvasId` on each object. The canvas is a document/storage dimension, not a
//! per-object property, so it is modeled here at the wrapper level. Storage keys
//! its `Record`s by `canvasId`; an operation envelope's actor/lease/routing all
//! resolve against the owning canvas rather than against any field inside the
//! scene graph.
//!
//! Region queries (PC10) follow the same separation: they key on
//! `(canvasId, bbox)` at the storage layer, again never on `Scene` objects. This
//! keeps the scene model portable and golden-verifiable against the TS source
//! while letting the document/storage layer scale on the canvas dimension.

use serde::{Deserialize, Serialize};

/// Opaque identifier for a [`Canvas`].
///
/// Serializes transparently as a bare JSON string so wire/storage payloads see
/// `"my-canvas"`, not `{ "0": "my-canvas" }`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanvasId(pub String);

impl std::fmt::Display for CanvasId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for CanvasId {
    fn from(value: &str) -> Self {
        CanvasId(value.to_string())
    }
}

impl From<String> for CanvasId {
    fn from(value: String) -> Self {
        CanvasId(value)
    }
}

/// A document wrapper that owns exactly one [`Scene`](crate::model::Scene) at
/// runtime. The scene itself is stored separately and keyed by [`CanvasId`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Canvas {
    pub id: CanvasId,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight projection of a [`Canvas`] for list views.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasSummary {
    pub id: CanvasId,
    pub title: String,
    pub updated_at: String,
}

/// Create a new canvas. `now` is the injected clock value (RFC3339 timestamp
/// string) used for both `created_at` and `updated_at`, keeping this function
/// free of ambient time per scene-core's purity invariant.
pub fn new_canvas(id: &str, title: &str, now: &str) -> Canvas {
    Canvas {
        id: CanvasId::from(id),
        title: title.to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_id_serializes_as_bare_string() {
        let id = CanvasId::from("c-123");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"c-123\"");

        let back: CanvasId = serde_json::from_str("\"c-123\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn canvas_id_constructors_and_display() {
        let from_str = CanvasId::from("a");
        let from_string = CanvasId::from(String::from("a"));
        assert_eq!(from_str, from_string);
        assert_eq!(from_str.to_string(), "a");
        assert_eq!(format!("{}", CanvasId::from("board-1")), "board-1");
    }

    #[test]
    fn new_canvas_uses_now_for_both_timestamps() {
        let c = new_canvas("c-1", "Untitled", "2026-06-07T00:00:00Z");
        assert_eq!(c.id, CanvasId::from("c-1"));
        assert_eq!(c.title, "Untitled");
        assert_eq!(c.created_at, "2026-06-07T00:00:00Z");
        assert_eq!(c.updated_at, "2026-06-07T00:00:00Z");
    }

    #[test]
    fn canvas_round_trip_uses_camel_case() {
        let c = new_canvas("c-1", "My Canvas", "2026-06-07T12:00:00Z");
        let json = serde_json::to_string(&c).unwrap();

        // camelCase keys; canvasId nested as a bare string.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], serde_json::json!("c-1"));
        assert_eq!(value["title"], serde_json::json!("My Canvas"));
        assert_eq!(value["createdAt"], serde_json::json!("2026-06-07T12:00:00Z"));
        assert_eq!(value["updatedAt"], serde_json::json!("2026-06-07T12:00:00Z"));
        assert!(value.get("created_at").is_none());

        let back: Canvas = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn canvas_summary_round_trip() {
        let s = CanvasSummary {
            id: CanvasId::from("c-2"),
            title: "List Item".to_string(),
            updated_at: "2026-06-07T09:30:00Z".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], serde_json::json!("c-2"));
        assert_eq!(value["title"], serde_json::json!("List Item"));
        assert_eq!(value["updatedAt"], serde_json::json!("2026-06-07T09:30:00Z"));

        let back: CanvasSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
