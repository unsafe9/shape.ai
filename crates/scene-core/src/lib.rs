//! Pure, platform-free logic shared by client (wasm32) and server (native).
//!
//! No ambient time, randomness, threads, or IO — every such seam is an injected
//! parameter (`now: &str`, an explicit operation id, etc.). The object model
//! (`object::*`) is the single canonical substrate.

pub mod canvas;
pub mod fractional;
pub mod lww;
pub mod model;
pub mod object;
pub mod tool;
pub mod wire;

// Gated so native builds/tests and a bare wasm32 check never pull wasm-bindgen.
#[cfg(feature = "wasm")]
pub mod wasm_api;

pub use canvas::{new_canvas, Canvas, CanvasId, CanvasSummary};
pub use fractional::{cmp_keys, generate_key_between, generate_n_keys_between};
pub use lww::{lww_merge_property, LwwEntry, LwwToken, PropertyStore};
pub use model::{Bounds, ExportType, ObjectMeta, Point, WorldPoint};
pub use tool::{default_tool, ActiveTool};
pub use wire::{Channel, ClientMessage, OpId, Region, ServerMessage, WireOp};
