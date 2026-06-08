#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

//! WebGPU renderer module.
//!
//! Split by responsibility (W2-13/S8, behavior-preserving):
//! - [`scene_build`] — portable pure-CPU layer: vertex/geometry build, style
//!   resolve, hit-test math, marquee, object-region derive/hit, overlay geometry,
//!   camera math, plus the consts/WGSL shader and the unit tests. Compiles and is
//!   exercised on the host (no wgpu device).
//! - [`device`] — web-only surface: `web_sys` + wgpu device/surface/config
//!   lifecycle (`probe_web_gpu`, `create`, `resize`, the glyph atlas upload).
//! - [`frame`] — the single-present render path (object pass + legacy fallback).
//! - [`input`] — input state machine, hit-test routing, rollback, debug snapshot.
//! - [`scene_feed`] — object/legacy scene load + feed + slot retention.
//!
//! This file owns the [`ShapeWebGpuRenderer`] struct and the small state/data
//! types whose private fields the sibling-module impls touch (field privacy is
//! scoped to the struct's defining module, so keeping them here lets every child
//! submodule reach those fields without widening). `scene_build`'s shared items are
//! re-exported crate-internally below so the web submodules and tests resolve them.

use std::{collections::HashMap, f32::consts::PI};

use crate::lod::{apparent_px, lod_tier, LodTier};
use crate::model::{
    ActiveTool, CameraState, CanvasInputEvent, CubicRoute, RenderCard, RenderEdge, RenderGroup,
    RenderScenePatch, SceneSelection, SceneShadowLayerToken, SceneSnapshot, SceneStyleToken,
    WorldPoint, WorldRect,
};
use crate::hit_test_object::{hit_test_object, HoverAffordance};
use crate::object_pipeline::{ObjectPipeline, ObjectRenderer};
use crate::outline::{derive_region, parse_path_string};
use crate::render_object::RenderObjectScene;
use crate::serde_wasm;
use crate::stats::{
    CoreHitResult, CoreInputBatchResult, CoreMarqueeResult, CoreOverlayRequest, CoreOverlayStyle,
    CoreOverlayTarget, ObjectDoubleClick, ObjectTransformDelta, WebGpuDebugSnapshot,
    WebGpuFrameStats, WebGpuProbeReport,
};
use crate::text::{
    CachedTextLine, TextBuildStats, TextEngine, TextLayoutCache, TEXT_ATLAS_HEIGHT,
    TEXT_ATLAS_SOLID_UV, TEXT_ATLAS_WIDTH,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

mod device;
mod frame;
mod input;
mod scene_build;
mod scene_feed;

pub(crate) use scene_build::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectSceneLoadResult {
    objects: usize,
    fill_indices: usize,
    stroke_vertices: usize,
}

/// Per-object derived region retained on the renderer for live hit-testing /
/// marquee against the loaded object scene (FC-04). `outline` is the region
/// boundary polygon in OBJECT-LOCAL pixels (D6); `transform` maps object-local px
/// to world px (D7). The outline is local so a moving transform never forces a
/// region rebuild — the query point is inverse-transformed into local space at
/// hit time (D8, see [`crate::hit_test_object`]).
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub(crate) struct ObjectRegion {
    id: String,
    transform: [[f64; 3]; 3],
    outline: Vec<(f32, f32)>,
    /// W2-06: whether the source contour was closed (rect/ellipse fill) vs open
    /// (line/freehand stroke). The nearest-point query (anchor snapping) includes
    /// the implicit closing edge only for closed shapes; hit-test/marquee ignore it.
    closed: bool,
}

/// FC-07: per-batch accumulator for the object-path input results, threaded through
/// [`ShapeWebGpuRenderer::apply_input_event`] and folded into the
/// [`CoreInputBatchResult`] at the end of the batch.
#[cfg(feature = "wgpu-probe")]
#[derive(Default)]
pub(crate) struct ObjectInputOut {
    selection: Option<String>,
    transform_delta: Option<ObjectTransformDelta>,
    marquee_ids: Option<Vec<String>>,
    // W2-02: hover affordance for the shell's cursor, set on a no-button move.
    // `None` outside object mode / when no hover move occurred in the batch.
    hover_affordance: Option<HoverAffordance>,
    // RA2b: a double-click that hit an object, branched by `has_children` (D6).
    double_click: Option<ObjectDoubleClick>,
}

#[cfg(feature = "wgpu-probe")]
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

#[cfg(feature = "wgpu-probe")]
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    camera: [f32; 4],
    viewport: [f32; 4],
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct VertexSlot {
    offset: usize,
    capacity: usize,
    text_stats: TextBuildStats,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Default)]
struct VertexRanges {
    groups: HashMap<String, VertexSlot>,
    group_free_offsets: Vec<usize>,
    edges: HashMap<String, VertexSlot>,
    edge_free_offsets: Vec<usize>,
    cards: HashMap<String, VertexSlot>,
    card_free_offsets: Vec<usize>,
}

#[cfg(feature = "wgpu-probe")]
struct DrawRange {
    start: u32,
    end: u32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Default)]
struct FrameDrawList {
    ranges: Vec<DrawRange>,
    visible_group_count: usize,
    visible_card_count: usize,
    visible_edge_count: usize,
    drawn_vertex_count: usize,
    full_tier_count: usize,
    compact_tier_count: usize,
    shape_only_tier_count: usize,
    density_tier_count: usize,
    minimap_tier_count: usize,
    /// Per-object LOD tier resolved this frame, keyed by object id. Carries the
    /// hysteresis state forward to the next frame's `lod_tier` resolution.
    lod_tiers: HashMap<String, LodTier>,
}

#[cfg(feature = "wgpu-probe")]
impl FrameDrawList {
    fn push_slot(&mut self, slot: VertexSlot) {
        let start = slot.offset as u32;
        let end = (slot.offset + slot.capacity) as u32;
        if let Some(last) = self.ranges.last_mut() {
            if last.end == start {
                last.end = end;
                self.drawn_vertex_count += slot.capacity;
                return;
            }
        }
        self.ranges.push(DrawRange { start, end });
        self.drawn_vertex_count += slot.capacity;
    }

    /// Resolve and record a visible object's LOD tier. The tier is a derived
    /// diagnostic value: it never alters slot identity, vertex ranges, or
    /// culling — only which token groups the draw build consumes (T3.1 §2/§4).
    fn record_tier(
        &mut self,
        id: &str,
        bounds: &WorldRect,
        camera: &CameraState,
        previous: &HashMap<String, LodTier>,
    ) {
        let tier = lod_tier(apparent_px(bounds, camera), previous.get(id).copied());
        match tier {
            LodTier::Full => self.full_tier_count += 1,
            LodTier::Compact => self.compact_tier_count += 1,
            LodTier::ShapeOnly => self.shape_only_tier_count += 1,
            LodTier::Density => self.density_tier_count += 1,
            LodTier::Minimap => self.minimap_tier_count += 1,
        }
        self.lod_tiers.insert(id.to_string(), tier);
    }
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Default)]
pub(crate) struct VertexFitStats {
    truncation_count: usize,
    truncated_vertex_count: usize,
}

#[cfg(feature = "wgpu-probe")]
impl VertexFitStats {
    fn add(&mut self, other: VertexFitStats) {
        self.truncation_count += other.truncation_count;
        self.truncated_vertex_count += other.truncated_vertex_count;
    }
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct MutationCounters {
    patch_update_count: usize,
    dirty_range_write_count: usize,
    full_buffer_rebuild_count: usize,
    vertex_truncation_count: usize,
    truncated_vertex_count: usize,
    edge_capacity_grow_count: usize,
    edge_compaction_count: usize,
    card_capacity_grow_count: usize,
    card_compaction_count: usize,
    group_capacity_grow_count: usize,
    group_compaction_count: usize,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone)]
pub(crate) enum InputDragState {
    Pan {
        pointer_id: i32,
        start: WorldPoint,
        camera: CameraState,
    },
    Group {
        pointer_id: i32,
        group_id: String,
        start: WorldPoint,
    },
    Card {
        pointer_id: i32,
        card_id: String,
        start: WorldPoint,
        start_bounds: WorldRect,
    },
    Edge {
        pointer_id: i32,
        source_id: String,
    },
    Marquee {
        pointer_id: i32,
        start: WorldPoint,
        current: WorldPoint,
    },
    // FC-07: dragging a selected object. `start` is the pointer-down WORLD point and
    // stays FIXED for the gesture so each move emits a cumulative delta the shell
    // turns into one undoable op; the renderer never mutates the object transform.
    Object {
        pointer_id: i32,
        object_id: String,
        start: WorldPoint,
    },
    // W2-04: resizing the selected object by a grabbed handle. `corner` is the
    // grabbed resize affordance (anchor = its OPPOSITE). `start` is the fixed
    // pointer-down WORLD point; `world_bbox` is the selection's WORLD AABB captured
    // AT pointer-down (`(min_x, min_y, max_x, max_y)`) so the gesture is anchored
    // and does not chase the live preview transform.
    Resize {
        pointer_id: i32,
        object_id: String,
        corner: HoverAffordance,
        start: WorldPoint,
        world_bbox: (f64, f64, f64, f64),
    },
    // W2-04: rotating the selected object about its bbox `center` (WORLD px,
    // captured at pointer-down). `start` is the fixed pointer-down WORLD point.
    Rotate {
        pointer_id: i32,
        object_id: String,
        start: WorldPoint,
        center: WorldPoint,
    },
}

#[cfg(feature = "wgpu-probe")]
struct RendererRollbackState {
    scene: Option<SceneSnapshot>,
    camera: CameraState,
    input_drag: Option<InputDragState>,
    active_tool: ActiveTool,
    multi_select: Vec<String>,
    last_hit: Option<CoreHitResult>,
    text_layout_cache: TextLayoutCache,
    counters: MutationCounters,
    // FC-07: object selection lives on `object_scene.selection`; capture the whole
    // scene so a failed input batch restores it (regions are local-space and follow
    // the transform, so they need no rollback).
    object_scene: Option<RenderObjectScene>,
}

#[cfg(feature = "wgpu-probe")]
#[wasm_bindgen]
pub struct ShapeWebGpuRenderer {
    canvas: HtmlCanvasElement,
    scene: Option<SceneSnapshot>,
    camera: CameraState,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    _text_texture: wgpu::Texture,
    _text_view: wgpu::TextureView,
    _text_sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    overlay_vertex_buffer: wgpu::Buffer,
    // W2-04: dedicated buffer for the selection-handle overlay (8 resize handles +
    // rotate zone), written per-frame and drawn in a LoadOp::Load pass on top.
    handle_vertex_buffer: wgpu::Buffer,
    // W3-G7/#1: dedicated buffer for the per-object multi-select outline highlight,
    // written per-frame and drawn in a LoadOp::Load pass on top of the object pass.
    multi_select_overlay_vertex_buffer: wgpu::Buffer,
    vertex_ranges: VertexRanges,
    text_engine: TextEngine,
    text_layout_cache: TextLayoutCache,
    vertex_count: usize,
    text_glyph_count: usize,
    fallback_text_glyph_count: usize,
    cjk_text_glyph_count: usize,
    font_fallback_run_count: usize,
    missing_text_glyph_count: usize,
    text_atlas_overflow_glyph_count: usize,
    text_missing_raster_glyph_count: usize,
    patch_update_count: usize,
    dirty_range_write_count: usize,
    full_buffer_rebuild_count: usize,
    vertex_truncation_count: usize,
    truncated_vertex_count: usize,
    edge_capacity_grow_count: usize,
    edge_compaction_count: usize,
    card_capacity_grow_count: usize,
    card_compaction_count: usize,
    group_capacity_grow_count: usize,
    group_compaction_count: usize,
    input_drag: Option<InputDragState>,
    active_tool: ActiveTool,
    // Transient multi-select highlight set. Held on the renderer (like
    // `active_tool`) so it survives a `load_scene` rebuild, then mirrored into the
    // scene snapshot the draw path reads. Never part of the serialized snapshot.
    multi_select: Vec<String>,
    last_hit: Option<CoreHitResult>,
    last_lod_tiers: HashMap<String, LodTier>,
    // OB-4 object draw path (additive). The pipeline is built lazily on the first
    // `load_object_scene`; the renderer holds the CPU-built + uploaded object
    // geometry for the current object scene. The legacy `load_scene`/`render_frame`
    // 2D path above is untouched — this is a parallel object pass that shares the
    // same device/queue/surface/format.
    object_pipeline: Option<ObjectPipeline>,
    object_renderer: Option<ObjectRenderer>,
    // FC-04: the parsed object scene + its per-object derived regions, retained so
    // the live frame loop draws objects (render_frame) and pointer input hit-tests
    // against them. `object_scene.is_some()` is the live-object branch switch; when
    // None the legacy 2D path stays authoritative.
    object_scene: Option<RenderObjectScene>,
    object_regions: Vec<ObjectRegion>,
    // W3-G6/#3: the persisted light/dark theme bit. The live theme is owned by the
    // per-scene `ObjectRenderer` (`self.theme`), which is destroyed and rebuilt on
    // every `load_object_scene` re-feed (pan/move/create), so the dark bit would be
    // lost on each re-feed. Holding it here (like `multi_select`/`active_tool`) lets
    // `set_object_theme` remember the last-set theme and `load_object_scene` rebuild
    // every renderer directly in that theme — sticky across reloads, zero rebake.
    object_theme: crate::object_theme::Theme,
    // W3-G8/A: offscreen targets + pipelines for the real separable-Gaussian drop-
    // shadow blur. Surface-sized (config.width x config.height); recreated in
    // `resize` after the config updates. Isolated underlay — a fault here can at
    // worst drop the shadow, never the fill/stroke/text on top.
    shadow_blur: crate::shadow_blur::ShadowBlur,
}
