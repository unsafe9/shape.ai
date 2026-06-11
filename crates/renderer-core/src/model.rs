use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraState {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WorldPoint {
    pub x: f64,
    pub y: f64,
}

#[cfg(feature = "wgpu-probe")]
pub struct CubicRoute {
    pub start: WorldPoint,
    pub cp1: WorldPoint,
    pub cp2: WorldPoint,
    pub end: WorldPoint,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderGroup {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub bounds: WorldRect,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    pub z_index: f64,
    #[serde(default = "default_style_key")]
    pub style_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderCard {
    pub id: String,
    pub group_id: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "type")]
    pub node_type: String,
    pub bounds: WorldRect,
    pub z_index: f64,
    #[serde(default = "default_style_key")]
    pub style_key: String,
    #[serde(default)]
    pub accessibility_label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderEdge {
    pub id: String,
    pub group_id: String,
    pub source: String,
    pub target: String,
    pub label: String,
    #[serde(default, rename = "type")]
    pub edge_type: String,
    #[serde(default)]
    pub z_index: f64,
    #[serde(default = "default_style_key")]
    pub style_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneStyleToken {
    pub id: String,
    pub fill: String,
    pub stroke: String,
    pub text: String,
    pub muted_text: String,
    pub accent: String,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub surface2: Option<String>,
    #[serde(default)]
    pub surface3: Option<String>,
    #[serde(default)]
    pub pastel: Option<String>,
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub line_strong: Option<String>,
    #[serde(default)]
    pub focus: Option<String>,
    #[serde(default)]
    pub radius: Option<SceneRadiusToken>,
    #[serde(default)]
    pub stroke_widths: Option<SceneStrokeWidthToken>,
    #[serde(default)]
    pub typography: Option<SceneTypographyToken>,
    #[serde(default)]
    pub spacing: Option<SceneSpacingToken>,
    #[serde(default)]
    pub shadow: Vec<SceneShadowLayerToken>,
    #[serde(default)]
    pub selected_shadow: Vec<SceneShadowLayerToken>,
    #[serde(default)]
    pub glow: Vec<SceneShadowLayerToken>,
    #[serde(default)]
    pub gradient: Option<SceneGradientToken>,
    #[serde(default)]
    pub states: Option<SceneStateTokens>,
    #[serde(default)]
    pub badge: Option<SceneBadgeToken>,
    #[serde(default)]
    pub edge: Option<SceneEdgeStyleToken>,
    #[serde(default)]
    pub port: Option<ScenePortStyleToken>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneShadowLayerToken {
    pub offset_x: f64,
    pub offset_y: f64,
    pub blur: f64,
    #[serde(default)]
    pub spread: f64,
    pub color: String,
    pub alpha: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneRadiusToken {
    #[serde(default)]
    pub group: Option<f64>,
    #[serde(default)]
    pub group_selected: Option<f64>,
    #[serde(default)]
    pub card: Option<f64>,
    #[serde(default)]
    pub card_selected: Option<f64>,
    #[serde(default)]
    pub badge: Option<f64>,
    #[serde(default)]
    pub edge_label: Option<f64>,
    #[serde(default)]
    pub port: Option<f64>,
    #[serde(default)]
    pub focus_ring: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneStrokeWidthToken {
    #[serde(default)]
    pub group: Option<f64>,
    #[serde(default)]
    pub group_selected: Option<f64>,
    #[serde(default)]
    pub card: Option<f64>,
    #[serde(default)]
    pub card_selected: Option<f64>,
    #[serde(default)]
    pub inner: Option<f64>,
    #[serde(default)]
    pub focus_ring: Option<f64>,
    #[serde(default)]
    pub edge: Option<f64>,
    #[serde(default)]
    pub edge_compact: Option<f64>,
    #[serde(default)]
    pub edge_selected: Option<f64>,
    #[serde(default)]
    pub separator: Option<f64>,
    #[serde(default)]
    pub port: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTypographyToken {
    #[serde(default)]
    pub group_title_size: Option<f64>,
    #[serde(default)]
    pub group_summary_size: Option<f64>,
    #[serde(default)]
    pub card_title_size: Option<f64>,
    #[serde(default)]
    pub card_selected_title_size: Option<f64>,
    #[serde(default)]
    pub card_summary_size: Option<f64>,
    #[serde(default)]
    pub badge_size: Option<f64>,
    #[serde(default)]
    pub edge_label_size: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSpacingToken {
    #[serde(default)]
    pub group_padding_x: Option<f64>,
    #[serde(default)]
    pub group_padding_y: Option<f64>,
    #[serde(default)]
    pub card_padding: Option<f64>,
    #[serde(default)]
    pub card_gap: Option<f64>,
    #[serde(default)]
    pub badge_padding_x: Option<f64>,
    #[serde(default)]
    pub badge_height: Option<f64>,
    #[serde(default)]
    pub label_padding_x: Option<f64>,
    #[serde(default)]
    pub edge_label_height: Option<f64>,
    #[serde(default)]
    pub port_radius: Option<f64>,
    #[serde(default)]
    pub separator_inset: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneGradientToken {
    #[serde(default)]
    pub surface_top_alpha: Option<f64>,
    #[serde(default)]
    pub pastel_bottom_alpha: Option<f64>,
    #[serde(default)]
    pub accent_start_alpha: Option<f64>,
    #[serde(default)]
    pub accent_end_alpha: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneStateTokens {
    #[serde(default)]
    pub default: Option<SceneStateVariantToken>,
    #[serde(default)]
    pub selected: Option<SceneStateVariantToken>,
    #[serde(default)]
    pub compact: Option<SceneStateVariantToken>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneStateVariantToken {
    #[serde(default)]
    pub fill_alpha: Option<f64>,
    #[serde(default)]
    pub stroke_alpha: Option<f64>,
    #[serde(default)]
    pub focus_alpha: Option<f64>,
    #[serde(default)]
    pub shadow_alpha: Option<f64>,
    #[serde(default)]
    pub glow_alpha: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneBadgeToken {
    #[serde(default)]
    pub fill_alpha: Option<f64>,
    #[serde(default)]
    pub stroke_alpha: Option<f64>,
    #[serde(default)]
    pub text_alpha: Option<f64>,
    #[serde(default)]
    pub min_width: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneEdgeStyleToken {
    #[serde(default)]
    pub stroke_alpha: Option<f64>,
    #[serde(default)]
    pub selected_stroke_alpha: Option<f64>,
    #[serde(default)]
    pub compact_stroke_alpha: Option<f64>,
    #[serde(default)]
    pub label_fill_alpha: Option<f64>,
    #[serde(default)]
    pub label_stroke_alpha: Option<f64>,
    #[serde(default)]
    pub label_text_alpha: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenePortStyleToken {
    #[serde(default)]
    pub fill_alpha: Option<f64>,
    #[serde(default)]
    pub stroke_alpha: Option<f64>,
    #[serde(default)]
    pub selected_fill_alpha: Option<f64>,
    #[serde(default)]
    pub selected_stroke_alpha: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSnapshot {
    pub scene_id: String,
    pub camera: CameraState,
    pub groups: Vec<RenderGroup>,
    pub cards: Vec<RenderCard>,
    pub edges: Vec<RenderEdge>,
    #[serde(default)]
    pub styles: Vec<SceneStyleToken>,
    #[serde(default)]
    pub selection: SceneSelection,
    // Transient shell-owned multi-select set, pushed via `set-multi-select`. Never
    // serialized: the persisted single-anchor `selection` invariant stays intact,
    // while the draw path highlights every id in this set in addition to it.
    #[serde(skip)]
    pub multi_select: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SceneSelection {
    #[default]
    Canvas,
    Group {
        id: String,
    },
    Node {
        id: String,
    },
    Edge {
        id: String,
    },
    // Transient multi-select set produced by a drag marquee. The shell merges
    // these ids into its own `multiSelectIds` set; the single-anchor persisted
    // selection invariant lives in the shell, not here. Matches scene-core/TS
    // `{ kind: "multi", ids: string[] }`.
    Multi {
        ids: Vec<String>,
    },
}

#[cfg(feature = "wgpu-probe")]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RenderScenePatch {
    CreateGroup {
        group: RenderGroup,
    },
    DeleteGroup {
        id: String,
    },
    MoveGroup {
        id: String,
        delta: WorldPoint,
    },
    MoveCard {
        id: String,
        position: WorldPoint,
    },
    SetCardZIndex {
        id: String,
        #[serde(rename = "zIndex")]
        z_index: f64,
    },
    EditCardText {
        id: String,
        field: String,
        value: String,
    },
    CreateCard {
        card: RenderCard,
    },
    DeleteCard {
        id: String,
    },
    CreateEdge {
        #[serde(rename = "groupId")]
        group_id: String,
        source: String,
        target: String,
        #[serde(rename = "edgeId")]
        edge_id: String,
        label: Option<String>,
    },
    DeleteEdge {
        id: String,
    },
    Select {
        selection: SceneSelection,
    },
}

// Active pointer tool. Select is the default: pointer-down on an object starts a
// drag, pointer-down on empty space starts a marquee. Hand always pans. Insert
// tools are handled shell-side via insert-primitive ops; the core only needs to
// distinguish Select vs Hand for pointer routing.
#[cfg(feature = "wgpu-probe")]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActiveTool {
    #[default]
    Select,
    Hand,
}

#[cfg(feature = "wgpu-probe")]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CanvasInputEvent {
    PointerDown {
        #[serde(rename = "pointerId")]
        pointer_id: i32,
        screen: WorldPoint,
    },
    PointerMove {
        #[serde(rename = "pointerId")]
        pointer_id: i32,
        screen: WorldPoint,
    },
    PointerUp {
        #[serde(rename = "pointerId")]
        pointer_id: i32,
        screen: WorldPoint,
        #[serde(rename = "edgeId")]
        edge_id: Option<String>,
    },
    PointerCancel {
        #[serde(rename = "pointerId")]
        pointer_id: i32,
    },
    Wheel {
        screen: WorldPoint,
        #[serde(rename = "deltaY")]
        delta_y: f64,
    },
    DoubleClick {
        screen: WorldPoint,
    },
    FitScene,
    FocusBounds {
        bounds: WorldRect,
        screen: Option<WorldPoint>,
        zoom: Option<f64>,
        padding: Option<WorldPoint>,
        #[serde(rename = "minZoom")]
        min_zoom: Option<f64>,
        #[serde(rename = "maxZoom")]
        max_zoom: Option<f64>,
    },
    SetCamera {
        camera: CameraState,
    },
    SetTool {
        tool: ActiveTool,
    },
    // Replace the transient multi-select set highlighted on the canvas. The shell
    // pushes its `multiSelectIds`; an empty list clears the set. The persisted
    // single-anchor selection is unaffected.
    SetMultiSelect {
        ids: Vec<String>,
    },
    // Right-click pick: returns the hit for `screen` in CoreInputBatchResult
    // without mutating selection or starting a drag, so the shell can show a
    // context menu for the picked object (CC4.1).
    ContextPick {
        screen: WorldPoint,
    },
}

fn default_style_key() -> String {
    "default".to_string()
}
