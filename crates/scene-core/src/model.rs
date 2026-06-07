//! Shared scalar primitives retained after the OB4.4 legacy-model removal.
//!
//! The legacy Group/Card/Edge scene model is gone — the object model
//! (`crate::object`) is the canonical document. What survives here are the
//! small value types still referenced by kept paths: `Bounds` (wire/server
//! region windows) and the template-recipe scalars (`Point`/`WorldPoint`,
//! `ObjectMeta`, `ExportType`). camelCase serde is preserved for wire parity.

use serde::{Deserialize, Serialize};

/// Free-form metadata bag (`z.record(string, unknown)`), preserved verbatim.
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

/// `WorldPoint` (renderScene.ts) is structurally identical to `Point`; template
/// recipe payloads use this alias.
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
