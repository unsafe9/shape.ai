//! Durable canvas index: the list of canvases that exist, persisted as a single
//! `canvas-index` [`Record`] so the set survives a restart independently of any
//! live actor.
//!
//! The index is metadata only: it never touches a canvas's scene Records (keyed
//! by `"{canvasId}:..."` and owned by the actor). Removing an entry here does not
//! delete those Records — the registry's `delete_canvas` prunes them separately.

use serde::{Deserialize, Serialize};
use shape_scene_core::{new_canvas, CanvasId, CanvasSummary};
use shape_storage_core::{Record, StorageAdapter};

/// Fixed Record id of the canvas index. No `"{canvasId}:"` prefix, so it never
/// collides with a canvas's per-object/journal Records.
pub const CANVAS_INDEX_RECORD_ID: &str = "canvas-index";
pub const KIND_CANVAS_INDEX: &str = "canvas-index";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct IndexPayload {
    canvases: Vec<CanvasSummary>,
}

/// A missing index Record returns an empty list (not an error).
pub fn load_index<S: StorageAdapter>(store: &S) -> Vec<CanvasSummary> {
    match store.load(CANVAS_INDEX_RECORD_ID) {
        Ok(record) => serde_json::from_slice::<IndexPayload>(&record.payload)
            .map(|p| p.canvases)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_index<S: StorageAdapter>(store: &mut S, canvases: &[CanvasSummary]) -> anyhow::Result<()> {
    let payload = IndexPayload {
        canvases: canvases.to_vec(),
    };
    store.save(Record {
        id: CANVAS_INDEX_RECORD_ID.to_string(),
        kind: KIND_CANVAS_INDEX.to_string(),
        version: canvases.len() as u64,
        payload: serde_json::to_vec(&payload).expect("canvas index serializes"),
    })?;
    Ok(())
}

/// `now` is the injected RFC3339 timestamp (scene-core stays ambient-time free).
/// Idempotent on a duplicate `id`: the existing summary is returned unchanged.
pub fn create_canvas<S: StorageAdapter>(
    store: &mut S,
    id: &str,
    title: &str,
    now: &str,
) -> anyhow::Result<CanvasSummary> {
    let mut canvases = load_index(store);
    if let Some(existing) = canvases.iter().find(|c| c.id == CanvasId::from(id)) {
        return Ok(existing.clone());
    }
    let canvas = new_canvas(id, title, now);
    let summary = CanvasSummary {
        id: canvas.id,
        title: canvas.title,
        updated_at: canvas.updated_at,
    };
    canvases.push(summary.clone());
    save_index(store, &canvases)?;
    Ok(summary)
}

/// Remove `id` from the index, returning whether an entry was removed. The caller
/// evicts the actor and prunes the canvas's scene Records.
pub fn delete_canvas<S: StorageAdapter>(store: &mut S, id: &str) -> anyhow::Result<bool> {
    let mut canvases = load_index(store);
    let before = canvases.len();
    canvases.retain(|c| c.id != CanvasId::from(id));
    if canvases.len() == before {
        return Ok(false);
    }
    save_index(store, &canvases)?;
    Ok(true)
}

/// In insertion order.
pub fn list_canvases<S: StorageAdapter>(store: &S) -> Vec<CanvasSummary> {
    load_index(store)
}
