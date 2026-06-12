use serde::Serialize;

#[cfg(feature = "wgpu-probe")]
use crate::model::{CameraState, RenderScenePatch, SceneSelection, WorldRect};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGpuFrameStats {
    pub total_groups: usize,
    pub total_cards: usize,
    pub total_edges: usize,
    pub visible_group_count: usize,
    pub visible_card_count: usize,
    pub visible_edge_count: usize,
    pub full_tier_count: usize,
    pub compact_tier_count: usize,
    pub shape_only_tier_count: usize,
    pub density_tier_count: usize,
    pub minimap_tier_count: usize,
    pub vertex_count: usize,
    pub drawn_vertex_count: usize,
    pub draw_range_count: usize,
    pub text_glyph_count: usize,
    pub fallback_text_glyph_count: usize,
    pub cjk_text_glyph_count: usize,
    pub font_fallback_run_count: usize,
    pub missing_text_glyph_count: usize,
    pub text_atlas_overflow_glyph_count: usize,
    pub text_missing_raster_glyph_count: usize,
    pub text_atlas_glyph_count: usize,
    pub text_raster_cache_hits: usize,
    pub text_raster_cache_misses: usize,
    pub text_layout_cache_hits: usize,
    pub text_layout_cache_misses: usize,
    pub style_token_count: usize,
    pub patch_update_count: usize,
    pub dirty_range_write_count: usize,
    pub full_buffer_rebuild_count: usize,
    pub vertex_truncation_count: usize,
    pub truncated_vertex_count: usize,
    pub edge_capacity_grow_count: usize,
    pub edge_compaction_count: usize,
    pub edge_slot_count: usize,
    pub edge_slot_free_count: usize,
    pub card_capacity_grow_count: usize,
    pub card_compaction_count: usize,
    pub card_slot_count: usize,
    pub card_slot_free_count: usize,
    pub group_capacity_grow_count: usize,
    pub group_compaction_count: usize,
    pub group_slot_count: usize,
    pub group_slot_free_count: usize,
    // Object draw-path diagnostics, populated when an object scene is loaded.
    pub object_count: usize,
    pub object_fill_index_count: usize,
    pub object_stroke_vertex_count: usize,
    pub object_draw_count: usize,
    pub object_patch_count: usize,
    pub object_rebuild_count: usize,
    pub backend: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreHitResult {
    pub id: String,
    pub kind: String,
    pub group_id: Option<String>,
    pub field: Option<String>,
    pub port: Option<String>,
    pub world_x: f64,
    pub world_y: f64,
    pub screen_x: f64,
    pub screen_y: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreOverlayTarget {
    pub kind: String,
    pub id: String,
    pub field: String,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreOverlayStyle {
    pub font_family: String,
    pub font_size: f64,
    pub font_weight: u16,
    pub line_height: f64,
    pub letter_spacing: f64,
    pub padding_x: f64,
    pub padding_y: f64,
    pub text_color: String,
    pub background_color: String,
    pub border_color: String,
    pub border_width: f64,
    pub border_radius: f64,
    pub focus_ring_color: String,
    pub focus_ring_width: f64,
    pub box_shadow: String,
    pub caret_color: String,
    pub accent_color: String,
    pub selection_background_color: String,
    pub max_lines: u8,
    pub overflow_x: String,
    pub overflow_y: String,
    pub state: String,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreOverlayRequest {
    pub target: CoreOverlayTarget,
    pub value: String,
    pub world_rect: WorldRect,
    pub screen_rect: WorldRect,
    pub style: CoreOverlayStyle,
}

/// Result of a completed drag marquee: `rect` is the final world-space marquee
/// rectangle. Emitted only on the pointer-up that ends a marquee drag.
#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMarqueeResult {
    pub rect: WorldRect,
    pub ids: Vec<String>,
}

/// Cumulative object transform delta. `matrix` is a row-major world-space delta to
/// pre-multiply onto the existing transform (`new = matrix * obj.transform`,
/// homogeneous `(x,y,1)`); cumulative from the fixed pointer-down anchor, so the
/// last delta of a gesture is the whole transform. `kind` is `"translate"` |
/// `"resize"` | `"rotate"`. The renderer never mutates the object transform itself.
#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectTransformDelta {
    pub id: String,
    pub matrix: [[f64; 3]; 3],
    pub kind: &'static str,
}

/// Live endpoint-drag sample for an open-class selection. `node_index` is the
/// dragged endpoint in geometry pair space (0 or the last coordinate pair, the
/// space scene-core `endpoint_release_ops` addresses); `(x, y)` is the cumulative
/// pointer world position. The renderer never mutates the geometry itself.
#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectEndpointDelta {
    pub id: String,
    pub node_index: i32,
    pub x: f64,
    pub y: f64,
}

/// Result of a double-click that hit an object. `has_children` discriminates the
/// shell branch: true => container (drill in), false => leaf (inline text edit).
#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDoubleClick {
    pub id: String,
    pub has_children: bool,
}

/// Result of the nearest-outline-point query (anchor snapping for shape drag-create).
/// On a hit, `snapped = true`, `(x, y)` is the nearest world point and `target_id`
/// the object id; on a miss, `snapped = false`, `x = y = 0`, `target_id = None`.
#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreNearestOutlinePoint {
    pub snapped: bool,
    pub x: f64,
    pub y: f64,
    pub target_id: Option<String>,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreInputBatchResult {
    pub camera: CameraState,
    pub hit: Option<CoreHitResult>,
    pub selection: SceneSelection,
    pub patches: Vec<RenderScenePatch>,
    pub overlay: Option<CoreOverlayRequest>,
    pub marquee: Option<CoreMarqueeResult>,
    // Object-path input results, non-null only when an object scene is loaded and
    // the corresponding event occurred.
    pub object_selection: Option<String>,
    pub object_transform_delta: Option<ObjectTransformDelta>,
    pub object_endpoint_delta: Option<ObjectEndpointDelta>,
    pub object_marquee_ids: Option<Vec<String>>,
    pub object_double_click: Option<ObjectDoubleClick>,
    // Stable affordance string ("empty" | "body" | "resize-*" | "rotate") the shell
    // maps to a cursor; "empty" outside object mode.
    pub hover_affordance: String,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGpuDebugSnapshot {
    pub camera: CameraState,
    pub selection: SceneSelection,
    pub selection_world_rect: Option<WorldRect>,
    pub selection_screen_rect: Option<WorldRect>,
    pub last_hit: Option<CoreHitResult>,
    pub total_groups: usize,
    pub total_cards: usize,
    pub total_edges: usize,
    pub patch_update_count: usize,
    pub dirty_range_write_count: usize,
    pub full_buffer_rebuild_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGpuProbeReport {
    pub supported: bool,
    pub adapter_found: bool,
    pub device_created: bool,
    pub surface_configured: bool,
    pub render_pass_submitted: bool,
    pub presented: bool,
    pub backend: String,
    pub enabled_backends: String,
    pub format: Option<String>,
    pub present_mode: Option<String>,
    pub width: u32,
    pub height: u32,
    pub detail: String,
}
