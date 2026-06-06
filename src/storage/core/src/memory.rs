//! Fully-working in-memory adapter.
//!
//! Backs tests and serves as the default in-process store. Implements all of
//! the per-record I/O directly against an in-memory [`StoreSnapshot`], and
//! inherits `export`/`import` from the trait's portable-format machinery.

use crate::adapter::{AdapterKind, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};

/// In-process store holding records in a sorted map.
#[derive(Clone, Debug, Default)]
pub struct MemoryAdapter {
    snapshot: StoreSnapshot,
}

impl MemoryAdapter {
    /// A new empty in-memory store.
    pub fn new() -> Self {
        MemoryAdapter::default()
    }

    /// Build directly from an existing snapshot.
    pub fn from_snapshot(snapshot: StoreSnapshot) -> Self {
        MemoryAdapter { snapshot }
    }

    /// Number of records held.
    pub fn len(&self) -> usize {
        self.snapshot.len()
    }

    /// Whether the store is empty.
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
        Ok(self.snapshot.remove(id).is_some())
    }

    fn list(&self) -> Result<Vec<String>> {
        Ok(self.snapshot.ids().cloned().collect())
    }

    fn snapshot(&self) -> Result<StoreSnapshot> {
        Ok(self.snapshot.clone())
    }

    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()> {
        self.snapshot = snapshot;
        Ok(())
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

        // Overwrite by id.
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
}
