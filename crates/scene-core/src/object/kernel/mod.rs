//! Tier dependency is one-way: kernel imports only itself and root substrate
//! modules (lww, fractional) — never authoring, binding, or catalog.

pub mod apply;
pub mod model;
pub mod op;
pub mod selection;
pub mod undo;
pub mod validate;
