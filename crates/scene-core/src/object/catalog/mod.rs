//! Catalog tier — self-documenting command/gesture/theme registries.
//!
//! The object command catalog (click/shortcut), the hold-key gesture catalog,
//! and the semantic theme token table. The crate-root `tool` module belongs to
//! this tier conceptually but stays at the crate root so its public re-export
//! path is unchanged.

pub mod commands;
pub mod gestures;
pub mod theme;
