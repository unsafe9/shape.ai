//! Contract-only trait any native platform adapter (Metal/macOS, Metal/iOS) must
//! implement to drive the shared `SceneRenderer` engine. No Metal implementation
//! here. Compiled under `wgpu-probe` until `SceneRenderer` is extracted.
//!
//! Boundary invariants: the adapter never owns or forks the semantic scene model;
//! business/MCP/export logic and canonical persistence stay in the shell/server;
//! ephemeral state (viewport, hover, active tool) is adapter/shell-local.

#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::model::{CameraState, SceneSnapshot, WorldRect};
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::stats::{
    CoreInputBatchResult, CoreOverlayRequest, WebGpuDebugSnapshot, WebGpuFrameStats,
    WebGpuProbeReport,
};

/// Logical size and backing scale supplied by the native host on each resize.
/// `width`/`height` are logical points; the core multiplies by `display_scale`
/// to derive physical pixels.
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub struct NativeSurfaceSize {
    pub width: f64,
    pub height: f64,
    /// macOS `NSScreen.backingScaleFactor` / iOS `UIScreen.nativeScale`.
    pub display_scale: f64,
}

/// Same field shape as `WebGpuProbeReport`, with `backend` set to a Metal marker.
#[cfg(feature = "wgpu-probe")]
pub type NativeProbeReport = WebGpuProbeReport;

/// A native pointer/touch event translated by the adapter for `input_batch`.
/// `screen` coordinates are logical points; the core scales internally.
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub struct NativePointerEvent {
    pub pointer_id: i32,
    pub screen_x: f64,
    pub screen_y: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub struct NativeWheelEvent {
    pub screen_x: f64,
    pub screen_y: f64,
    /// Positive = scroll down. iOS pinch-zoom maps to synthetic wheel deltas.
    pub delta_y: f64,
}

/// Contract every native platform adapter satisfies to drive the shared core.
/// The adapter does not own the scene model: it forwards `SceneSnapshot` to the
/// core and returns `CoreInputBatchResult` back to the shell with no interpretation.
#[cfg(feature = "wgpu-probe")]
pub trait NativeAdapter {
    /// Initialize the GPU surface from the platform window/layer handle. The
    /// native shell bridges `request_adapter`/`request_device` to the
    /// AppKit/UIKit lifecycle.
    fn create(size: NativeSurfaceSize) -> impl std::future::Future<Output = Result<Self, String>>
    where
        Self: Sized;

    /// Drawable area changed: the adapter resizes the core, then reconfigures
    /// the wgpu surface.
    fn resize(&mut self, size: NativeSurfaceSize);

    /// Suspend rendering, dropping the drawable but keeping the core scene; the
    /// surface may become invalid and is re-acquired on `resume`.
    fn suspend(&mut self);

    fn resume(&mut self);

    /// Release GPU resources; core scene data is untouched (shell owns it).
    fn destroy(self);

    /// Replace the full scene in the core; the adapter forwards, no interpretation.
    fn load_scene(&mut self, snapshot: SceneSnapshot);

    fn render_frame(&mut self) -> WebGpuFrameStats;

    /// Convert `NSEvent`/`UITouch` (logical-point coords) and call `input_batch`.
    fn pointer_down(&mut self, events: Vec<NativePointerEvent>) -> CoreInputBatchResult;

    fn pointer_move(&mut self, events: Vec<NativePointerEvent>) -> CoreInputBatchResult;

    /// `edge_id` is `Some` when the pointer-up completes a new edge gesture.
    fn pointer_up(
        &mut self,
        events: Vec<NativePointerEvent>,
        edge_id: Option<String>,
    ) -> CoreInputBatchResult;

    fn wheel(&mut self, event: NativeWheelEvent) -> CoreInputBatchResult;

    fn double_click(&mut self, screen_x: f64, screen_y: f64) -> CoreInputBatchResult;

    fn set_camera(&mut self, camera: CameraState) -> CoreInputBatchResult;

    /// `zoom` is an optional override; `None` lets the core compute it.
    fn focus_bounds(&mut self, bounds: WorldRect, zoom: Option<f64>) -> CoreInputBatchResult;

    /// `Some` when an `NSTextView`/`UITextView` should be mounted. The native
    /// text system owns IME, selection, clipboard, and commit/cancel — text
    /// input is never reimplemented in Rust.
    fn overlay_request(&self, id: &str, field: &str) -> Option<CoreOverlayRequest>;

    /// Metal capability probe with the same field shape as `WebGpuProbeReport`.
    fn probe_metal() -> impl std::future::Future<Output = NativeProbeReport>
    where
        Self: Sized;

    fn debug_snapshot(&self) -> WebGpuDebugSnapshot;
}
