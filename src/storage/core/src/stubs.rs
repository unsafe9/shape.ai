//! Adapter stubs for backends whose drivers are not available offline.
//!
//! sqlite / postgres / s3 / remote-server are part of the [`AdapterKind`]
//! vocabulary but require external crates or services that cannot be fetched
//! in this environment. Each is a real `StorageAdapter` *shape* so callers can
//! name and route to it, but the per-record I/O returns
//! [`StorageError::Unsupported`] until a backend is wired in.
//!
//! Crucially, the portability surface is *not* faked: `export`/`import` still
//! flow through `snapshot`/`restore`, so once a real backend lands the bundle
//! format works unchanged.

use crate::adapter::{AdapterKind, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};

macro_rules! unsupported_adapter {
    ($name:ident, $kind:expr, $label:literal) => {
        #[doc = concat!("Stub adapter for the `", $label, "` backend (driver unavailable offline).")]
        #[derive(Clone, Debug, Default)]
        pub struct $name;

        impl $name {
            /// Construct the stub.
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

unsupported_adapter!(SqliteAdapter, AdapterKind::Sqlite, "sqlite");
unsupported_adapter!(PostgresAdapter, AdapterKind::Postgres, "postgres");
unsupported_adapter!(S3Adapter, AdapterKind::S3, "s3");
unsupported_adapter!(RemoteServerAdapter, AdapterKind::RemoteServer, "remote-server");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_report_kind_and_unsupported() {
        let mut s = SqliteAdapter::new();
        assert_eq!(s.kind(), AdapterKind::Sqlite);
        assert!(matches!(
            s.save(Record::new("a", "k", b"x".to_vec())),
            Err(StorageError::Unsupported { kind: "sqlite", op: "save" })
        ));
        assert_eq!(PostgresAdapter::new().kind(), AdapterKind::Postgres);
        assert_eq!(S3Adapter::new().kind(), AdapterKind::S3);
        assert_eq!(RemoteServerAdapter::new().kind(), AdapterKind::RemoteServer);
    }
}
