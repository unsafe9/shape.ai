//! Contract-only crate: the [`NativeAdapter`] trait and its input/surface event
//! types for a future Metal adapter. No `wgpu`, no `web_sys` — engine types come
//! from [`shape_renderer_core`].

#[allow(dead_code)]
mod adapter_contract;

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use adapter_contract::{
    NativeAdapter, NativePointerEvent, NativeProbeReport, NativeSurfaceSize, NativeWheelEvent,
};
