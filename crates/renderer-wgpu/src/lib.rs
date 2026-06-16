//! WebGPU/web half of the shape.ai renderer. The pure CPU half lives in
//! [`shape_renderer_core`]. The package name stays `shape_canvas_core` so the
//! wasm-pack JS module name and the client loader path are unchanged.

mod object_pipeline;
mod shaders;
mod shadow_blur;
mod world_target;

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

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use object_pipeline::{ObjectPipeline, ObjectRenderer};

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use shadow_blur::ShadowBlur;

#[cfg(feature = "wgpu-probe")]
#[allow(unused_imports)]
pub use world_target::WorldTarget;

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

/// Parse a [`shape_renderer_core::RenderObjectScene`] from JSON and build the
/// CPU-side draw geometry (fill megabuffer + stroke ribbons) the GPU
/// `ObjectPipeline` uploads, returning a summary.
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectGeometrySummary {
    objects: usize,
    fill_vertices: usize,
    fill_triangles: usize,
    stroke_vertices: usize,
    draws: usize,
}

/// Project a canonical `ObjectScene` (JSON) into the renderer-core
/// `RenderObjectScene` (JSON) through the core's `from_object_scene` — the same
/// identity-default / selection-flatten / stroke-de-quant the shell hand-built in
/// TS. `camera_json`/`selection_json` are the live (transient) camera + selection
/// the shell drives; `scene_id` is the renderer's stable scene tag. Returns the
/// projected JSON string the visible-renderer `loadObjectScene` consumes.
#[wasm_bindgen(js_name = projectObjectScene)]
pub fn project_object_scene(
    scene_json: &str,
    camera_json: &str,
    selection_json: &str,
    scene_id: &str,
) -> Result<String, JsValue> {
    let mut scene: shape_scene_core::object::model::ObjectScene = serde_json::from_str(scene_json)
        .map_err(|e| JsValue::from_str(&format!("invalid object scene: {e}")))?;
    // The layout reflow's StubOutlineDeriver needs parsed subpaths; ensure_parsed is
    // idempotent (no-op when already populated), mirroring object_inspector_view.
    scene
        .ensure_parsed()
        .map_err(|e| JsValue::from_str(&format!("invalid object geometry: {e}")))?;
    let camera: shape_renderer_core::model::CameraState = serde_json::from_str(camera_json)
        .map_err(|e| JsValue::from_str(&format!("invalid camera: {e}")))?;
    let selection: shape_scene_core::object::model::ObjectSelection =
        serde_json::from_str(selection_json)
            .map_err(|e| JsValue::from_str(&format!("invalid selection: {e}")))?;
    let projected = shape_renderer_core::RenderObjectScene::from_object_scene(
        &scene, camera, &selection, scene_id,
    );
    serde_json::to_string(&projected)
        .map_err(|e| JsValue::from_str(&format!("projection serialization failed: {e}")))
}

/// The P1 proof widget tree: a single labeled pill, top-left screen-space.
/// Token paints so the canvas dark/light bit recolors it with zero rebake.
fn p1_proof_tree() -> shape_ui_core::Widget {
    shape_ui_core::Widget::Button(shape_ui_core::Button {
        id: "ui-proof-button".to_string(),
        x: 24.0,
        y: 24.0,
        w: 140.0,
        h: 40.0,
        label: "Rust UI".to_string(),
        style: shape_ui_core::RectStyle {
            // Filled accent pill: selection-ring is blue in both themes, so the white
            // label stays readable. RText color is a fixed hex (not a theme token), so
            // theme-following label color is a P2 toolkit item, not available here.
            fill: Some(shape_ui_core::Paint::Token("selection-ring".to_string())),
            stroke: None,
            corner_radius: 12.0,
            opacity: 1.0,
        },
        label_size_px: 16.0,
        label_color: shape_ui_core::TextPaint::Hex("#ffffff".to_string()),
    })
}

/// Build the P1 UI [`shape_renderer_core::RenderObjectScene`] (screen-space px,
/// identity camera) as JSON for the shell's `loadUiScene`. The widget tree lives in
/// `ui-core`; this only renders it and serializes — the only ui-core call site.
#[wasm_bindgen(js_name = buildP1UiScene)]
pub fn build_p1_ui_scene(viewport_w: f64, viewport_h: f64) -> Result<String, JsValue> {
    let scene = shape_ui_core::render(&p1_proof_tree(), (viewport_w, viewport_h), false);
    serde_json::to_string(&scene)
        .map_err(|e| JsValue::from_str(&format!("ui scene serialization failed: {e}")))
}

/// The P2 demo widget set: an absolute-positioned panel exercising every new widget
/// (Swatch / Toggle / Slider / Segment / TextInput) plus a theme-aware Text header
/// and the P1 button, so the live proof drives the toolkit + stateful dispatch. The
/// runtime owns this tree (seeded via `initUiRuntime`); built in `ui-core` because the
/// widget tree must stay there until P4 built-in UIs replace it.
///
/// The panel is `Axis::None` (absolute): each `Axis::None` container offsets children
/// by its origin, so a child laid out at `(cx, cy)` declares its OWN absolute screen
/// coord. The slider/segment pt.x→value mapping resolves the laid-out screen box
/// (`resolved_box`), so a flex/offset nesting maps correctly too.
///
/// Only the wasm seed (`init_ui_runtime`) and the host test drive this, so it is
/// compiled exactly there (a plain host build has no caller).
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn demo_ui_tree() -> shape_ui_core::Widget {
    use shape_ui_core::{
        Axis, Button, Container, CrossAlign, Edges, Paint, RectStyle, Segment, Slider, Swatch, Text,
        TextInput, TextPaint, Toggle, Widget,
    };
    Widget::Container(Container {
        id: "demo-panel".to_string(),
        // Origin 0: children carry absolute screen coords directly (no flex offset).
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![
            // Theme-aware header: a `text` token flips light↔dark on a theme change.
            Widget::Text(Text {
                id: "demo-header".to_string(),
                x: 24.0,
                y: 24.0,
                w: 248.0,
                h: 24.0,
                label: "Rust UI Toolkit".to_string(),
                size_px: 18.0,
                color: TextPaint::Token("text".to_string()),
                align_center: false,
            }),
            Widget::Button(Button {
                id: "demo-button".to_string(),
                x: 24.0,
                y: 60.0,
                w: 140.0,
                h: 40.0,
                label: "Action".to_string(),
                style: RectStyle {
                    fill: Some(Paint::Token("selection-ring".to_string())),
                    stroke: None,
                    corner_radius: 12.0,
                    opacity: 1.0,
                },
                label_size_px: 16.0,
                label_color: TextPaint::Hex("#ffffff".to_string()),
            }),
            Widget::Swatch(Swatch {
                id: "demo-swatch".to_string(),
                x: 24.0,
                y: 116.0,
                w: 48.0,
                h: 32.0,
                fill: Paint::Solid("#ff375f".to_string()),
                selected: false,
            }),
            Widget::Toggle(Toggle {
                id: "demo-toggle".to_string(),
                x: 92.0,
                y: 118.0,
                w: 52.0,
                h: 28.0,
                on: false,
            }),
            Widget::Slider(Slider {
                id: "demo-slider".to_string(),
                x: 24.0,
                y: 164.0,
                w: 248.0,
                h: 24.0,
                value: 0.4,
            }),
            Widget::Segment(Segment {
                id: "demo-segment".to_string(),
                x: 24.0,
                y: 204.0,
                w: 248.0,
                h: 32.0,
                labels: vec!["One".to_string(), "Two".to_string(), "Three".to_string()],
                selected: 0,
                label_size_px: 14.0,
                label_color: TextPaint::Token("text".to_string()),
            }),
            Widget::TextInput(TextInput {
                id: "demo-input".to_string(),
                x: 24.0,
                y: 252.0,
                w: 248.0,
                h: 32.0,
                value: String::new(),
                focused: false,
                size_px: 14.0,
                color: TextPaint::Token("text".to_string()),
                placeholder: "Type here".to_string(),
            }),
        ],
    })
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

#[cfg(test)]
mod tests {
    use super::demo_ui_tree;
    use shape_ui_core::{Action, PointerPhase, UiRuntime};

    /// The demo widget tree the runtime seed (`init_ui_runtime`) feeds must be a valid,
    /// hit-testable, interactive scene through the REAL ui-core render/hit/dispatch — so
    /// the live proof actually exercises every P2 widget. Drives the same `UiRuntime` the
    /// wasm surface drives (no second impl); a regression in the demo layout fails here.
    #[test]
    fn demo_tree_renders_every_widget_and_a_slider_drag_streams_a_value() {
        let viewport = (1024.0, 768.0);
        // The seed renders the tree screen-space; every widget's owner box must emit objects.
        let scene = shape_ui_core::render(&demo_ui_tree(), viewport, false);
        for owner in [
            "demo-header",
            "demo-button",
            "demo-swatch",
            "demo-toggle",
            "demo-slider",
            "demo-segment",
            "demo-input",
        ] {
            assert!(
                scene.objects.iter().any(|o| o.id == owner || o.id.starts_with(&format!("{owner}::"))),
                "demo widget {owner} must emit at least one render object"
            );
        }

        // The slider sits at x=24, w=248: a press at its midpoint streams value ~0.5 IN-CORE
        // (declared coords == hit box, since the panel is Axis::None at origin 0).
        let mut runtime = UiRuntime::new(demo_ui_tree(), viewport, false);
        let mid_x = 24.0 + 248.0 / 2.0;
        let down = runtime.dispatch_pointer(PointerPhase::Down, (mid_x, 164.0 + 12.0));
        assert!(down.consumed, "a press on the slider is consumed");
        let value = down.actions.iter().find_map(|a| match a {
            Action::SliderChanged { id, value } if id == "demo-slider" => Some(*value),
            _ => None,
        });
        assert_eq!(value, Some(0.5), "the slider streams its value from pt.x within the owner box");

        // A press on the text input focuses it (the arbiter's `uiHasFocus` then suppresses the catalog).
        let mut runtime = UiRuntime::new(demo_ui_tree(), viewport, false);
        runtime.dispatch_pointer(PointerPhase::Down, (24.0 + 10.0, 252.0 + 16.0));
        assert!(runtime.has_text_focus(), "a press on the demo TextInput grabs text focus");
    }
}
