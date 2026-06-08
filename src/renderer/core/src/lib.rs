mod adapter_contract;
mod frame_budget;
mod lod;
mod model;
mod render_cache;
mod stats;
mod text;

// OB-3 object render-model groundwork (additive). These pure-CPU object-pipeline
// pieces are consumed by `object_pipeline` (the OB-4 GPU draw path) under the
// `wgpu-probe` feature; the ones still unused by any draw path keep a
// module-local `#[allow(dead_code)]` until they are wired in.
mod curve_lod;
#[allow(dead_code)]
mod hit_test_object;
#[allow(dead_code)]
mod outline;
mod object_pipeline;
mod object_theme;
mod render_object;
mod shaders;
mod stroke_expand;
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
pub use stats::CoreNearestOutlinePoint;
#[cfg(feature = "wgpu-probe")]
pub use webgpu::ShapeWebGpuRenderer;

// OB-4 object CPU geometry build (device-independent; builds for every target,
// incl. the web wasm and a no-wgpu build, so `build_object_scene_geometry` works
// without the GPU pipeline).
#[allow(unused_imports)]
pub use object_pipeline::{
    build_scene_geometry, preview_instance_columns, FillInstance, FillVertex, ObjectDraw,
    ObjectMatrixUniform, SceneGeometry, StrokeInstance, StrokeParamsUniform, StrokeVertex,
};

// OB-4 object GPU pipeline (needs `wgpu`; the client flips to it at the cutover).
#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use object_pipeline::{ObjectPipeline, ObjectRenderer};

// OB-3 object render-model surface (additive; consumed at the OB-4 cutover).
#[allow(unused_imports)]
pub use curve_lod::{
    bucket_anchor_zoom, flatness_for_bucket, flatten_cubic, zoom_bucket, FlattenCache,
};
#[allow(unused_imports)]
pub use hit_test_object::{
    apply_3x3, hit_test_object, identity_3x3, invert_3x3, mat3_mul, point_in_polygon,
    resize_delta_matrix, rotate_about_3x3, rotate_delta_matrix, scale_about_3x3, translate_3x3,
    world_to_local, HoverAffordance, PathSeg, ScreenRect, SelectionHandles, HANDLE_SIZE_PX,
    ROTATE_ZONE_OFFSET_PX,
};
#[allow(unused_imports)]
pub use object_theme::{resolve_token, resolve_token_f32, Theme, ThemeToken, ALL_TOKENS};
#[allow(unused_imports)]
pub use outline::{derive_region, parse_path_string, Region, RegionCache};
#[allow(unused_imports)]
pub use render_object::{
    default_fill, default_stroke, parse_path_d, resolve_visual, FocusRing, RFill, RGradientStop,
    RHandle, RNode, RPaint, RStroke, RStrokeCap, RStrokeJoin, RSubPath, RText, RTextAlign,
    RTextRun, RTextValign, RenderObject, RenderObjectScene, ResolvedStyle, VisualState,
    QUANT_PER_PX,
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
    layout_runs, GlyphCoverage, GlyphPlacement, MsdfAtlasPlan, MsdfGlyphEntry, MsdfGlyphKey,
    TextAlign, TextRunInput, TextVAlign,
};

#[wasm_bindgen]
pub fn renderer_backend() -> String {
    "rust-wasm-scene-core".to_string()
}

/// OB-4 object render entry (web build). Parses a [`RenderObjectScene`] (the
/// object-substrate render view) from JSON and builds the CPU-side draw geometry
/// — fill megabuffer + stroke ribbons — that the GPU `ObjectPipeline` uploads.
/// Returns a summary `{ objects, fillVertices, fillTriangles, strokeVertices,
/// draws }` so the client can confirm the object scene reaches the renderer in
/// the web wasm. This proves the object render model + geometry build compile and
/// run for the web target (not only `wgpu-probe` tests); the full GPU draw wiring
/// (device/surface, `ObjectRenderer::render`) is wired at the renderer cutover
/// alongside the client object render adapter.
#[wasm_bindgen(js_name = buildObjectSceneGeometry)]
pub fn build_object_scene_geometry(scene_json: &str) -> Result<JsValue, JsValue> {
    let scene: render_object::RenderObjectScene = serde_json::from_str(scene_json)
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
