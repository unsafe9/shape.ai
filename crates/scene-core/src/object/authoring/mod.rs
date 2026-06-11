//! Authoring tier — input lowered into op synthesis.
//!
//! Pen-up stroke recognition, multi-stroke endpoint merge, open-class deform,
//! primitive/drag builders, the drawing brush, and template lowering. Each
//! turns user input into kernel ops.

pub mod deform;
pub mod drawing;
pub mod merge;
pub mod primitives;
pub mod recognize;
pub mod templates;
