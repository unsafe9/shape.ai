#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

//! WebGPU renderer module, split by responsibility:
//! - [`scene_build`] — portable pure-CPU layer (geometry/style/hit-test/marquee/
//!   overlay/camera math + the fallback WGSL shader), host-testable, no wgpu device.
//! - [`device`] — web-only `web_sys` + wgpu device/surface/config lifecycle.
//! - [`frame`] — the single-present render path (object pass + legacy fallback).
//! - [`input`] — input state machine, hit-test routing, rollback, debug snapshot.
//! - [`scene_feed`] — object/legacy scene load + feed + slot retention.
//!
//! This file owns [`ShapeWebGpuRenderer`] and the state/data types whose private
//! fields the sibling-module impls touch (field privacy is module-scoped, so
//! keeping them here lets every submodule reach the fields without widening).

use std::{collections::HashMap, f32::consts::PI};

use shape_renderer_core::lod::{apparent_px, lod_tier, LodTier};
use shape_renderer_core::model::{
    ActiveTool, CameraState, CanvasInputEvent, CubicRoute, RenderCard, RenderEdge, RenderGroup,
    RenderScenePatch, SceneSelection, SceneShadowLayerToken, SceneSnapshot, SceneStyleToken,
    WorldPoint, WorldRect,
};
use shape_renderer_core::hit_test_object::{hit_test_object, HoverAffordance};
use crate::object_pipeline::{ObjectPipeline, ObjectRenderer};
use shape_renderer_core::outline::{derive_region, parse_path_string};
use shape_renderer_core::render_object::RenderObjectScene;
use crate::serde_wasm;
use shape_renderer_core::stats::{
    CoreHitResult, CoreInputBatchResult, CoreMarqueeResult, CoreOverlayRequest, CoreOverlayStyle,
    CoreOverlayTarget, ObjectDoubleClick, ObjectEndpointDelta, ObjectTransformDelta,
    WebGpuDebugSnapshot, WebGpuFrameStats, WebGpuProbeReport,
};
use shape_renderer_core::text::{
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

/// Per-object derived region for live hit-test / marquee. `outline` is the region
/// boundary polygon in OBJECT-LOCAL px; `transform` maps object-local px to world.
/// The outline is local so a moving transform never forces a region rebuild — the
/// query point is inverse-transformed into local space at hit time.
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Debug)]
pub(crate) struct ObjectRegion {
    id: String,
    transform: [[f64; 3]; 3],
    outline: Vec<(f32, f32)>,
    /// Whether the source contour was closed (rect/ellipse fill) vs open (line/
    /// freehand). Nearest-point includes the implicit closing edge only for closed
    /// shapes; hit-test/marquee ignore it.
    closed: bool,
    /// The OPEN-CLASS endpoint pair, derived once per feed. `Some` switches the
    /// selection surface to two endpoint handles (no bbox 8-handle/rotate); `None`
    /// keeps the closed-class surface.
    open_endpoints: Option<OpenEndpoints>,
}

/// An open-class region's endpoints in OBJECT-LOCAL px, plus the END node's
/// geometry PAIR index (node 0 is always pair 0) — the same pair space anchors and
/// scene-core `endpoint_release_ops` address.
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenEndpoints {
    start: (f64, f64),
    end: (f64, f64),
    last_index: i32,
}

/// Per-batch accumulator for the object-path input results, folded into the
/// [`CoreInputBatchResult`] at the end of the batch.
#[cfg(feature = "wgpu-probe")]
#[derive(Default)]
pub(crate) struct ObjectInputOut {
    selection: Option<String>,
    transform_delta: Option<ObjectTransformDelta>,
    // A live endpoint-drag sample (open-class): one endpoint moves (chord deform),
    // not the whole transform.
    endpoint_delta: Option<ObjectEndpointDelta>,
    marquee_ids: Option<Vec<String>>,
    // Hover affordance for the shell's cursor, set on a no-button move; `None`
    // outside object mode / no hover move in the batch.
    hover_affordance: Option<HoverAffordance>,
    // A double-click that hit an object, branched by `has_children`.
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
    /// Per-object LOD tier resolved this frame, carrying hysteresis state forward to
    /// the next frame's `lod_tier` resolution.
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

    /// Resolve and record a visible object's LOD tier — a derived diagnostic that
    /// never alters slot identity, vertex ranges, or culling.
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
    // Dragging a selected object. `start` is the FIXED pointer-down WORLD point so
    // each move emits a cumulative delta the shell turns into one undoable op; the
    // renderer never mutates the object transform.
    Object {
        pointer_id: i32,
        object_id: String,
        start: WorldPoint,
    },
    // Resizing by a grabbed handle. `corner` is the grabbed affordance (anchor = its
    // OPPOSITE); `world_bbox` is the WORLD AABB captured AT pointer-down
    // (`(min_x, min_y, max_x, max_y)`) so the gesture is anchored, not chasing the
    // live preview transform.
    Resize {
        pointer_id: i32,
        object_id: String,
        corner: HoverAffordance,
        start: WorldPoint,
        world_bbox: (f64, f64, f64, f64),
    },
    // Rotating about the bbox `center` (WORLD px, captured at pointer-down).
    Rotate {
        pointer_id: i32,
        object_id: String,
        start: WorldPoint,
        center: WorldPoint,
    },
    // Dragging an OPEN-CLASS endpoint handle. `node_index` is the endpoint's geometry
    // PAIR index (0 | last). Each move emits a cumulative `ObjectEndpointDelta`; the
    // shell previews the chord deform and commits once on release. The renderer never
    // mutates the geometry.
    Endpoint {
        pointer_id: i32,
        object_id: String,
        node_index: i32,
    },
}

#[cfg(feature = "wgpu-probe")]
struct RendererRollbackState {
    scene: Option<SceneSnapshot>,
    camera: CameraState,
    input_drag: Option<InputDragState>,
    active_tool: ActiveTool,
    coarse_rotate: bool,
    multi_select: Vec<String>,
    last_hit: Option<CoreHitResult>,
    text_layout_cache: TextLayoutCache,
    counters: MutationCounters,
    // Object selection lives on `object_scene.selection`; capture the whole scene so
    // a failed input batch restores it (regions are local-space, so need no rollback).
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
    // Selection-handle overlay (8 resize handles + rotate zone), written per-frame,
    // drawn in a LoadOp::Load pass on top.
    handle_vertex_buffer: wgpu::Buffer,
    // Per-object multi-select outline highlight, written per-frame, drawn in a
    // LoadOp::Load pass on top of the object pass.
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
    // Coarse-rotate modifier (e.g. Shift held). Renderer-held mode bit; the rotate
    // drag arm reads it live so snapping engages/disengages mid-gesture.
    coarse_rotate: bool,
    // Transient multi-select highlight set, renderer-held (like `active_tool`) so it
    // survives a `load_scene` rebuild, then mirrored into the scene the draw path
    // reads. Never serialized.
    multi_select: Vec<String>,
    last_hit: Option<CoreHitResult>,
    last_lod_tiers: HashMap<String, LodTier>,
    // Object draw path: the pipeline is built lazily on the first
    // `load_object_scene`; the renderer holds the CPU-built + uploaded geometry. A
    // parallel pass sharing the legacy device/queue/surface/format.
    object_pipeline: Option<ObjectPipeline>,
    object_renderer: Option<ObjectRenderer>,
    // The parsed object scene + per-object regions for draw + hit-test.
    // `object_scene.is_some()` is the live-object branch switch; when None the
    // legacy 2D path stays authoritative.
    object_scene: Option<RenderObjectScene>,
    object_regions: Vec<ObjectRegion>,
    // FramePlan feed accounting: `object_patch_count` accumulates targeted re-feed
    // patches, `object_rebuild_count` counts feeds that fell back to a full
    // `ObjectRenderer::new`. Mirror the legacy `dirty_range_write_count` /
    // `full_buffer_rebuild_count`.
    object_patch_count: usize,
    object_rebuild_count: usize,
    // Ids whose GPU-baked GEOMETRY deviates from canonical because a live chord
    // deform patched it. GPU-only transient state (like instance-matrix previews);
    // the next preview frame / clear re-expands canonical geometry back in, never
    // rolled back.
    preview_deformed: std::collections::HashSet<String>,
    // The live endpoint-drag sample `(id, pair index, world point)`, held so the
    // endpoint-handle overlay rides the pointer while the geometry patch lands.
    endpoint_preview: Option<(String, i32, WorldPoint)>,
    // The move-together propagation graph (parent->child SameDelta + target->follower
    // Reproject), built ONCE per `load_object_scene` so a per-drag preview is
    // O(closure). Derived from `object_scene`, so rebuilt (not rolled back) on restore.
    object_bindings: shape_scene_core::object::move_together::BindingGraph,
    // The persisted light/dark theme bit. The live theme on the per-scene
    // `ObjectRenderer` is destroyed and rebuilt on every re-feed, so holding the bit
    // here lets `set_object_theme` remember it and rebuilds land in the right theme —
    // sticky across reloads, zero rebake.
    object_theme: shape_renderer_core::object_theme::Theme,
    // Offscreen targets + pipelines for the separable-Gaussian drop-shadow blur.
    // Surface-sized, recreated in `resize`. Isolated underlay — a fault drops the
    // shadow, never the fill/stroke/text on top.
    shadow_blur: crate::shadow_blur::ShadowBlur,
}
