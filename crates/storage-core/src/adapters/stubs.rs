//! Adapter stubs for backends (postgres / s3 / remote-server) whose drivers are
//! unavailable offline. Each is a real `StorageAdapter` *shape* callers can name
//! and route to, but per-record I/O returns [`StorageError::Unsupported`] until
//! a backend is wired in (`export`/`import` then come for free via the
//! streaming core once `records`/`ingest`/`snapshot`/`restore` are implemented).

use crate::adapter::{AdapterKind, RecordCursor, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};

macro_rules! unsupported_adapter {
    ($name:ident, $kind:expr, $label:literal) => {
        #[doc = concat!("Stub adapter for the `", $label, "` backend (driver unavailable offline).")]
        #[derive(Clone, Debug, Default)]
        pub struct $name;

        impl $name {
            pub fn new() -> Self {
                $name
            }
        }

        impl StorageAdapter for $name {
            fn kind(&self) -> AdapterKind {
                $kind
            }

            fn save(&mut self, _record: Record) -> Result<()> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "save",
                })
            }

            fn load(&self, _id: &str) -> Result<Record> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "load",
                })
            }

            fn delete(&mut self, _id: &str) -> Result<bool> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "delete",
                })
            }

            fn list(&self) -> Result<Vec<String>> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "list",
                })
            }

            fn records(&self) -> Result<RecordCursor<'_>> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "records",
                })
            }

            fn snapshot(&self) -> Result<StoreSnapshot> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "snapshot",
                })
            }

            fn restore(&mut self, _snapshot: StoreSnapshot) -> Result<()> {
                Err(StorageError::Unsupported {
                    kind: $label,
                    op: "restore",
                })
            }
        }
    };
}

unsupported_adapter!(PostgresAdapter, AdapterKind::Postgres, "postgres");
unsupported_adapter!(S3Adapter, AdapterKind::S3, "s3");
unsupported_adapter!(RemoteServerAdapter, AdapterKind::RemoteServer, "remote-server");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_report_kind_and_unsupported() {
        let mut s = PostgresAdapter::new();
        assert_eq!(s.kind(), AdapterKind::Postgres);
        assert!(matches!(
            s.save(Record::new("a", "k", b"x".to_vec())),
            Err(StorageError::Unsupported { kind: "postgres", op: "save" })
        ));
        assert_eq!(S3Adapter::new().kind(), AdapterKind::S3);
        assert_eq!(RemoteServerAdapter::new().kind(), AdapterKind::RemoteServer);
    }
}
