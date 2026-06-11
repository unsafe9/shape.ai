//! Kernel tier — the document substrate and the single op-apply path.
//!
//! Holds the object model, the op union, op-apply + inverse capture, the
//! structural validators, and the per-actor undo engine. The crate-root
//! `lww` + `fractional` modules complete the substrate conceptually but stay
//! at the crate root (their public paths are unchanged). The kernel imports
//! only itself and those root substrate modules — never the authoring,
//! binding, or catalog tiers.

pub mod apply;
pub mod model;
pub mod op;
pub mod undo;
pub mod validate;
