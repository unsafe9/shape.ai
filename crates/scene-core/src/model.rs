//! Shared scalar value types: `Bounds` (wire/server region windows) and the
//! template-recipe scalars. camelCase serde is preserved for wire parity.

use serde::{Deserialize, Serialize};

pub type ObjectMeta = serde_json::Map<String, serde_json::Value>;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub type WorldPoint = Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportType {
    Madr,
    Yadr,
    ImagePrompt,
    AiPlanMd,
    DesignDocMd,
    ConfluenceHtml,
    Mermaid,
    ArchitectureImage,
}
