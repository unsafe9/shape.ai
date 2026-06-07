//! Durable canvas index (MG9.1): the list of canvases that exist.
//!
//! `canvasId` is the actor/lease/routing key, but the set of canvases must
//! survive a restart on its own — an actor only exists once a canvas is opened,
//! so the registry cannot enumerate canvases from live actors. This module
//! persists a single `canvas-index` [`Record`] whose payload is the ordered list
//! of [`CanvasSummary`] entries, read/written through the shared storage adapter.
//!
//! The index is metadata only: it never touches a canvas's scene Records (those
//! are keyed by `"{canvasId}:..."` and owned by the actor). Deleting a canvas
//! from the index does not by itself delete its scene Records; the registry's
//! `delete_canvas` evicts the actor and prunes the scene separately.

use serde::{Deserialize, Serialize};
use shape_scene_core::{new_canvas, CanvasId, CanvasSummary};
use shape_storage_core::{Record, StorageAdapter};

/// The fixed Record id of the canvas index. Has no `"{canvasId}:"` prefix, so it
/// never collides with a canvas's per-object/journal Records.
pub const CANVAS_INDEX_RECORD_ID: &str = "canvas-index";
/// The Record `kind` for the canvas index.
pub const KIND_CANVAS_INDEX: &str = "canvas-index";

/// The persisted index payload: the ordered list of canvas summaries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct IndexPayload {
    canvases: Vec<CanvasSummary>,
}

/// Load the canvas index from `store`. A missing index Record means no canvases
/// have been created yet, so an empty list is returned (not an error).
pub fn load_index<S: StorageAdapter>(store: &S) -> Vec<CanvasSummary> {
    match store.load(CANVAS_INDEX_RECORD_ID) {
        Ok(record) => serde_json::from_slice::<IndexPayload>(&record.payload)
            .map(|p| p.canvases)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Overwrite the canvas index Record with `canvases`.
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

/// Create a canvas titled `title`, append it to the index, and return its
/// summary. `id` is the caller-chosen `canvasId` (the actor/lease/routing key);
/// `now` is the injected RFC3339 timestamp (scene-core stays ambient-time free).
///
/// Idempotent on a duplicate id: if the index already lists `id`, the existing
/// summary is returned unchanged rather than creating a second entry.
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

/// Remove `id` from the index. Returns whether an entry was removed. The caller
/// is responsible for evicting the actor and pruning the canvas's scene Records.
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

/// List every canvas in the index, in insertion order.
pub fn list_canvases<S: StorageAdapter>(store: &S) -> Vec<CanvasSummary> {
    load_index(store)
}
