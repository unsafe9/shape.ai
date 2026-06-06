mod adapter_contract;
mod model;
mod render_cache;
mod stats;
mod text;

#[cfg(feature = "wgpu-probe")]
mod lod;
#[cfg(feature = "wgpu-probe")]
mod webgpu;

use serde::Serialize;
use wasm_bindgen::prelude::*;
#[cfg(not(feature = "wgpu-probe"))]
use web_sys::HtmlCanvasElement;

pub use model::{
    CameraState, RenderCard, RenderEdge, RenderGroup, SceneSelection, SceneSnapshot,
    SceneStyleToken, WorldRect,
};
pub use stats::{CoreHitResult, WebGpuFrameStats, WebGpuProbeReport};
#[cfg(feature = "wgpu-probe")]
pub use webgpu::ShapeWebGpuRenderer;

#[wasm_bindgen]
pub fn renderer_backend() -> String {
    "rust-wasm-scene-core".to_string()
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
