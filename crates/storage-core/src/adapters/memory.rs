//! In-memory adapter backing tests and the default in-process store, over a
//! resident [`StoreSnapshot`]. Its export/import stay **streaming** so they
//! don't duplicate the whole serialized bundle on top of the store.

use crate::adapter::{AdapterKind, RecordCursor, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};
use crate::spatial::{bbox_overlaps, RegionKey, SpatialStore};
use std::collections::BTreeMap;

/// In-process store holding records in a sorted map. `regions` is the optional
/// spatial side index (see [`SpatialStore`]), empty unless
/// [`save_indexed`](SpatialStore::save_indexed) is used.
#[derive(Clone, Debug, Default)]
pub struct MemoryAdapter {
    snapshot: StoreSnapshot,
    regions: BTreeMap<String, RegionKey>,
}

impl MemoryAdapter {
    pub fn new() -> Self {
        MemoryAdapter::default()
    }

    pub fn from_snapshot(snapshot: StoreSnapshot) -> Self {
        MemoryAdapter {
            snapshot,
            regions: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.snapshot.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshot.is_empty()
    }
}

impl StorageAdapter for MemoryAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Memory
    }

    fn save(&mut self, record: Record) -> Result<()> {
        self.snapshot.insert(record);
        Ok(())
    }

    fn load(&self, id: &str) -> Result<Record> {
        self.snapshot
            .get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound { id: id.to_string() })
    }

    fn delete(&mut self, id: &str) -> Result<bool> {
        self.regions.remove(id);
        Ok(self.snapshot.remove(id).is_some())
    }

    fn list(&self) -> Result<Vec<String>> {
        Ok(self.snapshot.ids().cloned().collect())
    }

    fn records(&self) -> Result<RecordCursor<'_>> {
        Ok(Box::new(self.snapshot.records().cloned().map(Ok)))
    }

    fn snapshot(&self) -> Result<StoreSnapshot> {
        Ok(self.snapshot.clone())
    }

    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()> {
        self.snapshot = snapshot;
        // Drop the region index so no stale rows survive a replace.
        self.regions.clear();
        Ok(())
    }
}

impl SpatialStore for MemoryAdapter {
    fn save_indexed(&mut self, record: Record, key: Option<RegionKey>) -> Result<()> {
        match key {
            Some(key) => {
                self.regions.insert(record.id.clone(), key);
            }
            None => {
                self.regions.remove(&record.id);
            }
        }
        self.snapshot.insert(record);
        Ok(())
    }

    fn query_region(
        &self,
        canvas_id: &str,
        bbox: Option<(f64, f64, f64, f64)>,
    ) -> Result<RecordCursor<'_>> {
        let canvas_id = canvas_id.to_string();
        // `regions` is a BTreeMap, so iteration is id-sorted; matched records are
        // cloned lazily.
        let cursor = self
            .regions
            .iter()
            .filter(move |(_, key)| key.canvas_id == canvas_id)
            .filter(move |(_, key)| match bbox {
                None => true,
                Some((qminx, qminy, qmaxx, qmaxy)) => bbox_overlaps(
                    key.min_x, key.min_y, key.max_x, key.max_y, qminx, qminy, qmaxx, qmaxy,
                ),
            })
            .filter_map(|(id, _)| self.snapshot.get(id).cloned())
            .map(Ok);
        Ok(Box::new(cursor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_cycle() {
        let mut store = MemoryAdapter::new();
        assert!(store.is_empty());

        store.save(Record::new("a", "card", b"one".to_vec())).unwrap();
        store.save(Record::new("b", "edge", b"two".to_vec())).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("a").unwrap().payload, b"one");
        assert_eq!(store.list().unwrap(), vec!["a", "b"]);

        store
            .save(Record::new("a", "card", b"one-v2".to_vec()))
            .unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("a").unwrap().payload, b"one-v2");

        assert!(store.delete("a").unwrap());
        assert!(!store.delete("a").unwrap());
        assert!(matches!(
            store.load("a"),
            Err(StorageError::NotFound { .. })
        ));
        assert_eq!(store.list().unwrap(), vec!["b"]);
    }

    #[test]
    fn records_cursor_is_id_sorted_and_complete() {
        let mut store = MemoryAdapter::new();
        for id in ["c", "a", "b"] {
            store.save(Record::new(id, "k", id.as_bytes().to_vec())).unwrap();
        }
        let ids: Vec<String> = store
            .records()
            .unwrap()
            .map(|r| r.unwrap().id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }
}
