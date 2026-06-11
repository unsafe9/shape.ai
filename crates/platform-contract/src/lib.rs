//! Native platform adapter contract for the shape.ai canvas core.
//!
//! Contract-only crate: the [`NativeAdapter`] trait and its native input/surface
//! event types that a future Metal/macOS or Metal/iOS adapter implements to drive
//! the shared renderer core. No `wgpu`, no `web_sys` — the engine types it
//! references come from [`shape_renderer_core`].

#[allow(dead_code)]
mod adapter_contract;

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use adapter_contract::{
    NativeAdapter, NativePointerEvent, NativeProbeReport, NativeSurfaceSize, NativeWheelEvent,
};
