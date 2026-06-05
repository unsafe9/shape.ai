use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraState {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct WorldPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[cfg(feature = "wgpu-probe")]
pub(crate) struct CubicRoute {
    pub(crate) start: WorldPoint,
    pub(crate) cp1: WorldPoint,
    pub(crate) cp2: WorldPoint,
    pub(crate) end: WorldPoint,
}

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneStyleToken {
    pub id: String,
    pub fill: String,
    pub stroke: String,
    pub text: String,
    pub muted_text: String,
    pub accent: String,
}

#[derive(Debug, Deserialize, Serialize)]
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
}

#[cfg(feature = "wgpu-probe")]
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum RenderScenePatch {
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

fn default_style_key() -> String {
    "default".to_string()
}
