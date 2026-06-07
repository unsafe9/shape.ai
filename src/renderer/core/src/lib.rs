mod adapter_contract;
mod frame_budget;
mod lod;
mod model;
mod render_cache;
mod stats;
mod text;

// OB-3 object render-model groundwork (additive). These modules are pure-CPU
// object-pipeline pieces wired into the GPU draw path at the OB-4 cutover; until
// then they are unused by the live 2D pipeline, so they carry module-local
// `#[allow(dead_code)]` to avoid tripping warnings before they are consumed.
#[allow(dead_code)]
mod curve_lod;
#[allow(dead_code)]
mod hit_test_object;
#[allow(dead_code)]
mod outline;
#[allow(dead_code)]
mod render_object;
#[allow(dead_code)]
mod shaders;
#[allow(dead_code)]
mod stroke_expand;
#[allow(dead_code)]
mod tessellate;
#[allow(dead_code)]
mod text_layout;

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

// OB-3 object render-model surface (additive; consumed at the OB-4 cutover).
#[allow(unused_imports)]
pub use curve_lod::{
    bucket_anchor_zoom, flatness_for_bucket, flatten_cubic, zoom_bucket, FlattenCache,
};
#[allow(unused_imports)]
pub use hit_test_object::{
    apply_3x3, hit_test_object, invert_3x3, point_in_polygon, world_to_local, PathSeg,
};
#[allow(unused_imports)]
pub use outline::{derive_region, parse_path_string, Region, RegionCache};
#[allow(unused_imports)]
pub use render_object::{
    default_fill, default_stroke, parse_path_d, resolve_visual, FocusRing, RFill, RGradientStop,
    RHandle, RNode, RPaint, RStroke, RStrokeCap, RStrokeJoin, RSubPath, RText, RTextAlign,
    RTextRun, RTextValign, RenderObject, RenderObjectScene, ResolvedStyle, VisualState,
};
#[allow(unused_imports)]
pub use shaders::{CLIP_WGSL, MSDF_TEXT_WGSL, OBJECT_FILL_WGSL, OBJECT_STROKE_WGSL};
#[allow(unused_imports)]
pub use stroke_expand::{
    dash_segments, expand_stroke, Cap, Join, Mesh as StrokeMesh,
};
#[allow(unused_imports)]
pub use tessellate::{
    parse_path, quantized_to_px, tessellate_fill, DrawRange, FillRuleKind, MegaBuffer, Mesh,
    ParsedSubpath, PathCommand, TessCache,
};
#[allow(unused_imports)]
pub use text_layout::{
    layout_runs, GlyphPlacement, MsdfAtlasPlan, MsdfGlyphEntry, MsdfGlyphKey, TextAlign,
    TextRunInput, TextVAlign,
};

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
