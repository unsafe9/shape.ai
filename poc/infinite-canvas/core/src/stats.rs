use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreFrameStats {
    pub total_groups: usize,
    pub total_cards: usize,
    pub total_edges: usize,
    pub hit_testable_cards: usize,
    pub backend: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGpuFrameStats {
    pub total_groups: usize,
    pub total_cards: usize,
    pub total_edges: usize,
    pub vertex_count: usize,
    pub text_glyph_count: usize,
    pub fallback_text_glyph_count: usize,
    pub cjk_text_glyph_count: usize,
    pub style_token_count: usize,
    pub patch_update_count: usize,
    pub dirty_range_write_count: usize,
    pub full_buffer_rebuild_count: usize,
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
    pub backend: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreHitResult {
    pub id: String,
    pub kind: String,
    pub group_id: Option<String>,
    pub field: Option<String>,
    pub port: Option<String>,
    pub world_x: f64,
    pub world_y: f64,
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
