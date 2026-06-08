//! shape.ai shared scene core.
//!
//! Pure, platform-free logic shared by the client (wasm32) and server (native):
//! the object document model + op-apply, per-property LWW, fractional indexing,
//! the template-recipe library, the wire protocol serde, and the canvas concept.
//!
//! Invariants (see CLAUDE.md + canvas-cockpit task breakdown):
//! - No ambient time, randomness, threads, or IO. Every such seam is an injected
//!   parameter (`now: &str`, an explicit operation id, etc.).
//! - The object model (`object::*`) is the single canonical substrate; the
//!   legacy Group/Card/Edge scene + RenderScenePatch path was removed at OB4.4.

pub mod canvas;
pub mod fractional;
pub mod lww;
pub mod model;
pub mod object;
pub mod tool;
pub mod wire;

// MG0.3/MG0.4: the wasm-bindgen JS bridge. Gated so native builds/tests and a
// bare wasm32 check never pull wasm-bindgen; built on with `--features wasm`.
#[cfg(feature = "wasm")]
pub mod wasm_api;

pub use canvas::{new_canvas, Canvas, CanvasId, CanvasSummary};
pub use fractional::{cmp_keys, generate_key_between, generate_n_keys_between};
pub use lww::{lww_merge_property, LwwEntry, LwwToken, PropertyStore};
pub use model::{Bounds, ExportType, ObjectMeta, Point, WorldPoint};
pub use tool::{default_tool, ActiveTool};
pub use wire::{Channel, ClientMessage, OpId, Region, ServerMessage, WireOp};
