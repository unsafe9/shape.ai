//! Native adapter contract for the shape canvas core.
//!
//! Defines the trait any native platform adapter (e.g., Metal/macOS, Metal/iOS)
//! must implement in order to drive the shared `SceneRenderer` engine.
//!
//! Design intent (T1.3):
//!   - The portable render/scene logic lives in `SceneRenderer` (no `web_sys`, no
//!     `HtmlCanvasElement`). Each platform adapter owns surface acquisition and
//!     platform I/O, then calls into the shared engine methods.
//!   - This file is **contract only**. No Metal implementation is produced here;
//!     a `metal-adapter` cargo feature (parallel to `wgpu-probe`) will gate the
//!     actual implementation when scaffolded in a later migration task (P6/T6.3).
//!
//! Feature gating: the types `CoreInputBatchResult`, `CoreOverlayRequest`, and
//! `WebGpuDebugSnapshot` currently live under `wgpu-probe` (they will migrate to
//! a `metal-adapter`-compatible position when P6/T6.3 extracts `SceneRenderer`).
//! Until that extraction happens this module is compiled under `wgpu-probe` so
//! that `cargo build` stays green without structural changes to the existing
//! feature graph.
//!
//! Boundary invariants (from T1.3 §8):
//!   - The adapter never owns or forks the semantic scene model (`SceneSnapshot`,
//!     `RenderScenePatch`, etc.).
//!   - Business/MCP/export logic stays in the shell/server — out of scope here.
//!   - Canonical persistence (SQLite/API) is not touched by the adapter.
//!   - Ephemeral state (viewport, hover, active tool) is adapter/shell-local.

#[cfg(feature = "wgpu-probe")]
use crate::model::{CameraState, SceneSnapshot, WorldRect};
#[cfg(feature = "wgpu-probe")]
use crate::stats::{
    CoreInputBatchResult, CoreOverlayRequest, WebGpuDebugSnapshot, WebGpuFrameStats,
    WebGpuProbeReport,
};

// ── Surface lifecycle ──────────────────────────────────────────────────────────

/// Logical size and backing scale supplied by the native host on each resize.
///
/// `width` and `height` are in logical points (CSS-px equivalents); the core
/// multiplies by `display_scale` to derive physical pixels (mirrors
/// `webgpu.rs:735` `config.width = round(w * scale)`).
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub struct NativeSurfaceSize {
    /// Logical width in platform points (not physical pixels).
    pub width: f64,
    /// Logical height in platform points (not physical pixels).
    pub height: f64,
    /// Backing scale factor (macOS `NSScreen.backingScaleFactor` /
    /// iOS `UIScreen.nativeScale`). Equivalent to web `window.devicePixelRatio`.
    pub display_scale: f64,
}

/// Capability probe result returned by a native adapter, mirroring
/// `WebGpuProbeReport` (`stats.rs:141`) with the same field shape so that
/// the diagnostics drawer (`RendererDiagnosticsDrawer`) can consume it
/// unchanged.
///
/// The `backend` string is set to a Metal marker (e.g. `"rust-wgpu-metal"`)
/// instead of `"rust-wgpu-visible"`.
#[cfg(feature = "wgpu-probe")]
pub type NativeProbeReport = WebGpuProbeReport;

// ── Native input bridge ────────────────────────────────────────────────────────

/// A native pointer/touch event translated by the adapter before being
/// forwarded to the core's `input_batch`.
///
/// The adapter maps:
///   - macOS `NSEvent` mouse / iOS `UITouch` → `NativePointerEvent`
///
/// All `screen` coordinates are in logical points; the core scales internally
/// (mirrors `webgpu.rs:518,690` `device_pixel_ratio` handling).
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub struct NativePointerEvent {
    /// Platform touch/stylus/mouse identifier (multi-touch safe).
    pub pointer_id: i32,
    /// Logical-point screen position x.
    pub screen_x: f64,
    /// Logical-point screen position y.
    pub screen_y: f64,
}

/// A native scroll/zoom gesture translated by the adapter.
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub struct NativeWheelEvent {
    /// Logical-point screen position of the scroll origin x.
    pub screen_x: f64,
    /// Logical-point screen position of the scroll origin y.
    pub screen_y: f64,
    /// Vertical scroll delta (positive = scroll down). Pinch-zoom on iOS maps
    /// to synthetic wheel deltas, matching the web `deltaY` convention.
    pub delta_y: f64,
}

// ── Adapter trait ──────────────────────────────────────────────────────────────

/// Contract that every native platform adapter must satisfy in order to drive
/// the shared canvas core.
///
/// The trait covers four responsibility areas:
///
/// 1. **Surface lifecycle** — create, resize, suspend/resume, destroy.
/// 2. **Frame rendering** — trigger a render pass and receive frame diagnostics.
/// 3. **Native text overlay** — receive `CoreOverlayRequest` from the core and
///    mount a platform text control (`NSTextView` / `UITextView`).
/// 4. **Diagnostics** — capability probe and debug snapshot passthrough.
///
/// The adapter does *not* own the scene model. It receives `SceneSnapshot` from
/// the shell and forwards it to the core; it returns `CoreInputBatchResult` from
/// the core back to the shell — no interpretation.
///
/// ## Metal surface lifecycle
///
/// Web today (`webgpu.rs`) | Metal adapter contract
/// --- | ---
/// `HtmlCanvasElement` arg | `CAMetalLayer`/`MTKView` via `raw-window-handle`
/// `wgpu::Backends::BROWSER_WEBGPU` | `wgpu::Backends::METAL`
/// `SurfaceTarget::Canvas(...)` | `SurfaceTargetUnsafe::from_metal_layer`
/// `canvas.set_width/height` + `surface.configure` on resize | `MTKView.drawableSize` set by host; adapter calls `resize` then `surface.configure`
/// `requestAnimationFrame` driver | `CADisplayLink` / `MTKViewDelegate.draw`
#[cfg(feature = "wgpu-probe")]
pub trait NativeAdapter {
    // ── Lifecycle ──────────────────────────────────────────────────────────

    /// Initialize the GPU surface from the platform window/layer handle.
    ///
    /// Web equivalent: `ShapeWebGpuRenderer::create` (`webgpu.rs:511`) which
    /// calls `request_adapter` + `request_device` + `instance.create_surface`.
    ///
    /// The native path uses `wgpu::Backends::METAL` and
    /// `wgpu::SurfaceTargetUnsafe::from_metal_layer` (or `raw-window-handle`
    /// `RawWindowHandle::AppKit`/`UiKit`) instead of
    /// `SurfaceTarget::Canvas(HtmlCanvasElement)`.
    ///
    /// Async: the native shell must bridge `request_adapter`/`request_device`
    /// to the AppKit/UIKit lifecycle (e.g. via `pollster` at init, or the
    /// app's own async executor). The async bridging is an adapter concern.
    fn create(size: NativeSurfaceSize) -> impl std::future::Future<Output = Result<Self, String>>
    where
        Self: Sized;

    /// Notify the adapter that the drawable area has changed.
    ///
    /// Called from the platform resize callback:
    ///   - macOS: `viewDidChangeBackingProperties` / layout pass
    ///   - iOS: `traitCollectionDidChange` / `viewDidLayoutSubviews`
    ///
    /// The adapter calls `resize(size.width, size.height, size.display_scale)`
    /// on the shared core, then reconfigures the wgpu surface (mirrors
    /// `webgpu.rs:737` `canvas.set_width/height` + `surface.configure`).
    fn resize(&mut self, size: NativeSurfaceSize);

    /// Suspend rendering (e.g., iOS app backgrounded).
    ///
    /// The adapter stops the display link / `CADisplayLink`, drops the current
    /// drawable, and keeps the core scene in memory. The surface may become
    /// invalid; the adapter must re-acquire it on `resume`.
    fn suspend(&mut self);

    /// Resume rendering after suspension.
    ///
    /// The adapter re-acquires the drawable (surface-lost recovery path) and
    /// restarts the display link. Mirrors the surface-lost / device-lost
    /// reconfigure path described in T1.3 §1.
    fn resume(&mut self);

    /// Tear down the surface and release GPU resources. Core scene data is not
    /// touched — the shell owns canonical state.
    fn destroy(self);

    // ── Scene ──────────────────────────────────────────────────────────────

    /// Replace the full scene in the core (mirrors `load_scene` / `loadScene`
    /// in the shared engine). The adapter simply forwards; no interpretation.
    fn load_scene(&mut self, snapshot: SceneSnapshot);

    // ── Frame ──────────────────────────────────────────────────────────────

    /// Execute one render pass and return per-frame diagnostics.
    ///
    /// The native run-loop (`CADisplayLink` / `MTKViewDelegate.draw`) calls
    /// this instead of the browser `requestAnimationFrame` driver.
    ///
    /// Internally: calls `render_frame` on the shared core, which runs
    /// `build_draw_list` → encoder → pass → `surface.get_current_texture` →
    /// `present` (identical wgpu calls against the Metal drawable,
    /// `webgpu.rs:1177,1227`).
    fn render_frame(&mut self) -> WebGpuFrameStats;

    // ── Input ──────────────────────────────────────────────────────────────

    /// Translate and forward a batch of native pointer-down events to the core.
    ///
    /// The adapter converts `NSEvent`/`UITouch` into the core's
    /// `CanvasInputEvent::PointerDown` vocabulary (logical-point `screen`
    /// coordinates) and calls `input_batch` on the shared engine.
    fn pointer_down(&mut self, events: Vec<NativePointerEvent>) -> CoreInputBatchResult;

    /// Translate and forward a batch of native pointer-move events.
    fn pointer_move(&mut self, events: Vec<NativePointerEvent>) -> CoreInputBatchResult;

    /// Translate and forward a batch of native pointer-up events.
    ///
    /// `edge_id` is `Some` when the pointer-up should complete a new edge
    /// creation gesture (matches `CanvasInputEvent::PointerUp.edge_id`).
    fn pointer_up(
        &mut self,
        events: Vec<NativePointerEvent>,
        edge_id: Option<String>,
    ) -> CoreInputBatchResult;

    /// Translate and forward a native scroll/zoom gesture.
    fn wheel(&mut self, event: NativeWheelEvent) -> CoreInputBatchResult;

    /// Translate and forward a native double-tap / double-click.
    fn double_click(&mut self, screen_x: f64, screen_y: f64) -> CoreInputBatchResult;

    /// Forward a camera set command to the core.
    ///
    /// Triggered by native toolbar actions or keyboard shortcuts handled in
    /// the shell — not in the renderer (matching the web shell convention).
    fn set_camera(&mut self, camera: CameraState) -> CoreInputBatchResult;

    /// Focus the scene to a world-space bounding rect (e.g., "fit to group").
    ///
    /// `zoom` is an optional override; when `None` the core computes the
    /// optimal zoom to fill the viewport.
    fn focus_bounds(&mut self, bounds: WorldRect, zoom: Option<f64>) -> CoreInputBatchResult;

    // ── Text overlay ───────────────────────────────────────────────────────

    /// Ask the core whether a text overlay is active for the given node field.
    ///
    /// Returns `Some(CoreOverlayRequest)` when an `NSTextView`/`UITextView`
    /// should be mounted at `screen_rect`, styled from `CoreOverlayStyle`.
    ///
    /// The adapter maps `CoreOverlayStyle` CSS-ish fields (e.g. `box_shadow`,
    /// hex `text_color`) onto AppKit/UIKit attributes. The native text system
    /// owns IME (incl. Korean 2-set), selection, clipboard, focus/blur, and
    /// commit/cancel — mirroring the "do not reimplement text input in Rust"
    /// locked decision (T1.3 §3).
    ///
    /// On commit, the adapter emits `EditCardText { id, field, value }` back
    /// through the core's apply-patch path (same as the web shell,
    /// `model.rs:374`).
    fn overlay_request(&self, id: &str, field: &str) -> Option<CoreOverlayRequest>;

    // ── Diagnostics ────────────────────────────────────────────────────────

    /// Run the Metal capability probe and return a report with the same field
    /// shape as `WebGpuProbeReport` (`stats.rs:141`).
    ///
    /// `backend` should be set to `"rust-wgpu-metal"` (or similar). Metal-only
    /// telemetry (GPU family, ProMotion refresh rate, thermal state) is
    /// adapter-layer data and does *not* appear in the shared `WebGpuFrameStats`
    /// fields (T1.3 §6).
    fn probe_metal() -> impl std::future::Future<Output = NativeProbeReport>
    where
        Self: Sized;

    /// Return a debug snapshot of the current core state (mirrors
    /// `debug_snapshot` / `ShapeWebGpuRenderer::debug_snapshot`,
    /// `webgpu.rs:1324`). Used by the diagnostics drawer.
    fn debug_snapshot(&self) -> WebGpuDebugSnapshot;
}
