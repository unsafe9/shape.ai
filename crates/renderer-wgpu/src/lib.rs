//! WebGPU/web half of the shape.ai renderer.
//!
//! Holds the `wgpu` GPU pipelines, the drop-shadow blur passes, the WGSL shaders,
//! the web surface lifecycle, and the `#[wasm_bindgen]` renderer surface. The pure
//! CPU half (tessellation, text layout, hit testing, object geometry build, render
//! model) lives in the [`shape_renderer_core`] crate, which this depends on by
//! path. The package name stays `shape_canvas_core` so the wasm-pack JS module name
//! and the client loader path are unchanged.

mod object_pipeline;
mod shaders;
mod shadow_blur;

#[cfg(feature = "wgpu-probe")]
mod webgpu;

use serde::Serialize;
use wasm_bindgen::prelude::*;
#[cfg(not(feature = "wgpu-probe"))]
use web_sys::HtmlCanvasElement;

use shape_renderer_core::build_scene_geometry;
#[allow(unused_imports)]
use shape_renderer_core::WebGpuProbeReport;

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use webgpu::ShapeWebGpuRenderer;

// OB-4 object GPU pipeline (needs `wgpu`; the client flips to it at the cutover).
#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use object_pipeline::{ObjectPipeline, ObjectRenderer};

// W3-G8/A real drop-shadow blur GPU offscreen-target + pipeline holder. Re-exported
// so the wasm32-only consumer (the live frame loop) isn't the lone reference.
#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use shadow_blur::ShadowBlur;

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use shaders::{
    CLIP_WGSL, MSDF_TEXT_WGSL, OBJECT_FILL_WGSL, OBJECT_STROKE_WGSL, SHADOW_BLUR_WGSL,
    SHADOW_COMPOSITE_WGSL,
};

#[wasm_bindgen]
pub fn renderer_backend() -> String {
    "rust-wasm-scene-core".to_string()
}

/// OB-4 object render entry (web build). Parses a [`shape_renderer_core::RenderObjectScene`]
/// (the object-substrate render view) from JSON and builds the CPU-side draw
/// geometry — fill megabuffer + stroke ribbons — that the GPU `ObjectPipeline`
/// uploads. Returns a summary `{ objects, fillVertices, fillTriangles,
/// strokeVertices, draws }` so the client can confirm the object scene reaches the
/// renderer in the web wasm. This proves the object render model + geometry build
/// compile and run for the web target (not only `wgpu-probe` tests); the full GPU
/// draw wiring (device/surface, `ObjectRenderer::render`) is wired at the renderer
/// cutover alongside the client object render adapter.
#[wasm_bindgen(js_name = buildObjectSceneGeometry)]
pub fn build_object_scene_geometry(scene_json: &str) -> Result<JsValue, JsValue> {
    let scene: shape_renderer_core::RenderObjectScene = serde_json::from_str(scene_json)
        .map_err(|e| JsValue::from_str(&format!("invalid object scene: {e}")))?;
    let geometry = build_scene_geometry(&scene);
    serde_wasm(ObjectGeometrySummary {
        objects: scene.objects.len(),
        fill_vertices: geometry.fill.vertices.len(),
        fill_triangles: geometry.fill.indices.len() / 3,
        stroke_vertices: geometry.stroke_vertices.len(),
        draws: geometry.draws.len(),
    })
}

/// The CPU geometry summary returned by [`build_object_scene_geometry`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectGeometrySummary {
    objects: usize,
    fill_vertices: usize,
    fill_triangles: usize,
    stroke_vertices: usize,
    draws: usize,
}

#[cfg(not(feature = "wgpu-probe"))]
#[wasm_bindgen(js_name = probeWebGpu)]
pub fn probe_web_gpu(
    _canvas: HtmlCanvasElement,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
) -> Result<JsValue, JsValue> {
    serde_wasm(WebGpuProbeReport {
        supported: false,
        adapter_found: false,
        device_created: false,
        surface_configured: false,
        render_pass_submitted: false,
        presented: false,
        backend: "wgpu-probe-disabled".to_string(),
        enabled_backends: "none".to_string(),
        format: None,
        present_mode: None,
        width: ((width.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1),
        height: ((height.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1),
        detail: "Compiled without the wgpu-probe feature.".to_string(),
    })
}

#[wasm_bindgen(start)]
pub fn install_panic_hook() {
    console_error_panic_hook();
}

fn serde_wasm<T: Serialize>(value: T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&value)
        .map_err(|error| JsValue::from_str(&format!("Serialization failed: {error}")))
}

fn console_error_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!(
            "shape_canvas_core panic: {info}"
        )));
    }));
}
