//! Error type shared across the storage core.

use std::fmt;

/// Errors surfaced by adapters and the portable export/import format.
#[derive(Debug)]
pub enum StorageError {
    /// A record with the requested id was not found.
    NotFound { id: String },
    /// The on-disk format did not match expectations (bad magic, version,
    /// truncated frame, checksum mismatch, ...).
    Format(String),
    /// Underlying I/O failure (file open/read/write, directory create, ...).
    Io(String),
    /// (De)serialization of metadata/manifest failed.
    Serde(String),
    /// The adapter cannot satisfy the request because it is an unimplemented
    /// stub (sqlite/postgres/s3/remote when their backends are unavailable).
    Unsupported { kind: &'static str, op: &'static str },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound { id } => write!(f, "record not found: {id}"),
            StorageError::Format(msg) => write!(f, "portable format error: {msg}"),
            StorageError::Io(msg) => write!(f, "io error: {msg}"),
            StorageError::Serde(msg) => write!(f, "serde error: {msg}"),
            StorageError::Unsupported { kind, op } => {
                write!(f, "adapter '{kind}' does not support operation '{op}'")
            }
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        StorageError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serde(e.to_string())
    }
}

/// Convenience alias for fallible storage operations.
pub type Result<T> = std::result::Result<T, StorageError>;
