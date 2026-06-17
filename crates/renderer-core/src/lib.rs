//! Pure CPU renderer core for the shape.ai infinite canvas. No `wgpu`, no
//! `web_sys` — the GPU pipelines and `#[wasm_bindgen]` surface live in the
//! `shape_canvas_core` (renderer-wgpu) crate, which depends on this one by path.

pub mod backdrop_blur;
pub mod cast;
pub mod frame_budget;
pub mod lod;
pub mod model;
pub mod render_cache;
pub mod stats;
pub mod text;

pub mod curve_lod;
#[allow(dead_code)]
pub mod hit_test_object;
pub mod object_pipeline;
pub mod object_theme;
pub mod plan;
#[allow(dead_code)]
pub mod outline;
pub mod render_object;
pub mod shadow_blur;
pub mod stroke_expand;
pub mod tessellate;
#[allow(dead_code)]
pub mod text_layout;

pub use model::{
    CameraState, RenderCard, RenderEdge, RenderGroup, SceneSelection, SceneSnapshot,
    SceneStyleToken, WorldRect,
};
pub use stats::{CoreHitResult, WebGpuFrameStats, WebGpuProbeReport};
#[cfg(feature = "wgpu-probe")]
pub use stats::CoreNearestOutlinePoint;

#[allow(unused_imports)]
pub use object_pipeline::{
    build_scene_geometry, build_scene_geometry_themed_with_text, preview_instance_columns,
    FillInstance, FillVertex, GlyphUvProvider, ObjectDraw, ObjectMatrixUniform, SceneGeometry,
    StrokeInstance, StrokeParamsUniform, StrokeVertex,
};

// FramePlan IR: platform-neutral, diffable draw-plan contract between this crate
// (what to draw) and renderer-wgpu (GPU submission).
#[allow(unused_imports)]
pub use plan::{
    build_frame_plan, build_frame_plan_with_text, diff_plans, geometry_revision, pass_order,
    DrawPass, FramePlan, PlanDiff, PlanEntry, PlanInstance, PlanPatch, ResourceHandle, StyleSlot,
};

#[allow(unused_imports)]
pub use shadow_blur::{gaussian_kernel, SHADOW_BLUR_MAX_RADIUS, SHADOW_BLUR_RADIUS_PX};

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
    RTextMode, RTextRun, RTextValign, RenderObject, RenderObjectScene, ResolvedStyle, VisualState,
    QUANT_PER_PX,
};
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
