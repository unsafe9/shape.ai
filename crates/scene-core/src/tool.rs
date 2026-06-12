//! Active-tool enum — ephemeral shell state, no persistence and no op variant;
//! scene-core owns only the type so every platform shell shares one vocabulary.

use serde::{Deserialize, Serialize};

/// Serde tags are kebab-case to match the wire vocabulary and command catalog
/// ids (`select-move`, `hand-pan`, `insert-rectangle`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActiveTool {
    Select,
    Hand,
    InsertRectangle,
    InsertEllipse,
    InsertConnector,
    InsertSticky,
    InsertFrame,
}

impl Default for ActiveTool {
    fn default() -> Self {
        ActiveTool::Select
    }
}

pub fn default_tool() -> ActiveTool {
    ActiveTool::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_select() {
        assert_eq!(ActiveTool::default(), ActiveTool::Select);
        assert_eq!(default_tool(), ActiveTool::Select);
    }

    #[test]
    fn serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&ActiveTool::Select).unwrap(),
            "\"select\""
        );
        assert_eq!(
            serde_json::to_string(&ActiveTool::InsertRectangle).unwrap(),
            "\"insert-rectangle\""
        );
        assert_eq!(
            serde_json::to_string(&ActiveTool::Hand).unwrap(),
            "\"hand\""
        );
    }

    #[test]
    fn round_trips_every_variant() {
        for tool in [
            ActiveTool::Select,
            ActiveTool::Hand,
            ActiveTool::InsertRectangle,
            ActiveTool::InsertEllipse,
            ActiveTool::InsertConnector,
            ActiveTool::InsertSticky,
            ActiveTool::InsertFrame,
        ] {
            let json = serde_json::to_string(&tool).unwrap();
            let back: ActiveTool = serde_json::from_str(&json).unwrap();
            assert_eq!(tool, back);
        }
    }
}
