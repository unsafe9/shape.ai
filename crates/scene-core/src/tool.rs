//! Active-tool model (CC0.1) — the small enum the cockpit shell uses to track
//! which canvas tool is currently armed.
//!
//! This is **ephemeral shell state**: scene-core owns only the type (and its
//! default) so every platform shell shares one vocabulary, the same way the
//! command catalog is shared. There is no persistence, no `Scene` field, and no
//! op variant for the active tool — picking a tool is a UI concern; the result
//! of *using* one is an ordinary `RenderScenePatch` (see [`crate::tool`] callers
//! and `insert_primitive_ops`).

use serde::{Deserialize, Serialize};

/// The canvas tool currently armed in the cockpit.
///
/// `Select` (the default) drags/moves objects; `Hand` pans the viewport; the
/// `Insert*` variants arm an insert gesture for the matching primitive. The
/// serde tags are kebab-case to match the rest of the wire vocabulary and the
/// command catalog ids (`select-move`, `hand-pan`, `insert-rectangle`, …).
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

/// The default armed tool (`Select`). A free helper so a shell can request the
/// default without naming the variant.
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
