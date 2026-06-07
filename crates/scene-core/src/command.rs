//! Command catalog — the canonical list of cockpit actions and their default
//! keyboard shortcuts.
//!
//! This data drives two surfaces: the cockpit command palette/toolbar and the
//! settings panel's read-only shortcut reference. Keeping it in scene-core lets
//! every platform shell render the same catalog without re-declaring it, and
//! `command_catalog_json()` is the wire seam the shell consumes.
//!
//! Shortcuts use the platform-agnostic `Mod` token for the primary modifier
//! (Cmd on macOS, Ctrl elsewhere); the shell resolves it per platform.

use serde::Serialize;

/// The functional grouping a command belongs to. Serialized kebab-case to match
/// the rest of the wire model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandCategory {
    Tool,
    Shape,
    View,
    Edit,
    Selection,
    Template,
    Canvas,
}

/// A single cockpit command entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub id: String,
    pub label: String,
    pub category: CommandCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_shortcut: Option<String>,
    pub description: String,
}

impl Command {
    fn new(
        id: &str,
        label: &str,
        category: CommandCategory,
        default_shortcut: Option<&str>,
        description: &str,
    ) -> Self {
        Command {
            id: id.to_string(),
            label: label.to_string(),
            category,
            default_shortcut: default_shortcut.map(|s| s.to_string()),
            description: description.to_string(),
        }
    }
}

/// The full cockpit command catalog, in display order grouped by category.
pub fn command_catalog() -> Vec<Command> {
    use CommandCategory::*;
    vec![
        // Tool
        Command::new(
            "select-move",
            "Select / Move",
            Tool,
            Some("V"),
            "Activate the select-and-move tool for picking and dragging objects.",
        ),
        Command::new(
            "hand-pan",
            "Hand / Pan",
            Tool,
            Some("H"),
            "Activate the hand tool to pan the canvas viewport.",
        ),
        // Shape
        Command::new(
            "insert-rectangle",
            "Rectangle",
            Shape,
            Some("R"),
            "Insert a rectangle shape.",
        ),
        Command::new(
            "insert-ellipse",
            "Ellipse",
            Shape,
            Some("O"),
            "Insert an ellipse shape.",
        ),
        Command::new(
            "insert-connector",
            "Connector",
            Shape,
            Some("C"),
            "Insert a connector between two objects.",
        ),
        Command::new(
            "insert-sticky",
            "Sticky Note",
            Shape,
            Some("S"),
            "Insert a sticky note.",
        ),
        Command::new(
            "insert-frame",
            "Frame",
            Shape,
            Some("F"),
            "Insert a frame to group objects.",
        ),
        // View
        Command::new(
            "zoom-in",
            "Zoom In",
            View,
            Some("Mod+="),
            "Zoom the canvas in.",
        ),
        Command::new(
            "zoom-out",
            "Zoom Out",
            View,
            Some("Mod+-"),
            "Zoom the canvas out.",
        ),
        Command::new(
            "zoom-fit",
            "Zoom to Fit",
            View,
            Some("Shift+1"),
            "Zoom and pan so the whole scene fits in the viewport.",
        ),
        Command::new(
            "zoom-reset",
            "Reset Zoom",
            View,
            Some("Mod+0"),
            "Reset the zoom level to 100%.",
        ),
        Command::new(
            "toggle-fullscreen",
            "Toggle Fullscreen",
            View,
            Some("F11"),
            "Toggle fullscreen canvas mode.",
        ),
        // Edit
        Command::new(
            "delete",
            "Delete",
            Edit,
            Some("Backspace"),
            "Delete the current selection.",
        ),
        Command::new(
            "duplicate",
            "Duplicate",
            Edit,
            Some("Mod+D"),
            "Duplicate the current selection.",
        ),
        Command::new(
            "copy",
            "Copy",
            Edit,
            Some("Mod+C"),
            "Copy the current selection to the clipboard.",
        ),
        Command::new(
            "paste",
            "Paste",
            Edit,
            Some("Mod+V"),
            "Paste from the clipboard.",
        ),
        Command::new(
            "group",
            "Group",
            Edit,
            Some("Mod+G"),
            "Group the selected objects into a frame.",
        ),
        Command::new(
            "ungroup",
            "Ungroup",
            Edit,
            Some("Mod+Shift+G"),
            "Ungroup the selected frame.",
        ),
        Command::new(
            "align-left",
            "Align Left",
            Edit,
            None,
            "Align the selected objects to their left edges.",
        ),
        Command::new(
            "align-center",
            "Align Center",
            Edit,
            None,
            "Align the selected objects to their horizontal centers.",
        ),
        Command::new(
            "align-right",
            "Align Right",
            Edit,
            None,
            "Align the selected objects to their right edges.",
        ),
        Command::new(
            "distribute-horizontal",
            "Distribute Horizontally",
            Edit,
            None,
            "Distribute the selected objects evenly along the horizontal axis.",
        ),
        Command::new(
            "bring-to-front",
            "Bring to Front",
            Edit,
            Some("]"),
            "Bring the current selection to the front of the z-order.",
        ),
        Command::new(
            "send-to-back",
            "Send to Back",
            Edit,
            Some("["),
            "Send the current selection to the back of the z-order.",
        ),
        // Selection
        Command::new(
            "select-all",
            "Select All",
            Selection,
            Some("Mod+A"),
            "Select all objects in the scene.",
        ),
        Command::new(
            "clear-selection",
            "Clear Selection",
            Selection,
            Some("Escape"),
            "Clear the current selection.",
        ),
        // Template
        Command::new(
            "open-template-library",
            "Template Library",
            Template,
            Some("T"),
            "Open the template library.",
        ),
        // Canvas
        Command::new(
            "new-canvas",
            "New Canvas",
            Canvas,
            Some("Mod+N"),
            "Create a new canvas.",
        ),
        Command::new(
            "open-settings",
            "Settings",
            Canvas,
            Some("Mod+,"),
            "Open the settings panel.",
        ),
    ]
}

/// The command catalog serialized to JSON — the seam the shell consumes
/// (`command_catalog() -> JSON`).
pub fn command_catalog_json() -> String {
    // The catalog is statically constructed, so serialization cannot fail; the
    // `expect` documents that invariant rather than masking a real error.
    serde_json::to_string(&command_catalog()).expect("command catalog serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_is_non_empty() {
        assert!(!command_catalog().is_empty());
    }

    #[test]
    fn ids_are_unique() {
        let catalog = command_catalog();
        let mut seen = HashSet::new();
        for cmd in &catalog {
            assert!(seen.insert(cmd.id.as_str()), "duplicate id: {}", cmd.id);
        }
        assert_eq!(seen.len(), catalog.len());
    }

    #[test]
    fn every_command_has_a_category() {
        // CommandCategory is a closed enum, so presence is type-guaranteed; this
        // asserts the catalog actually exercises every category we ship.
        let categories: HashSet<CommandCategory> =
            command_catalog().iter().map(|c| c.category).collect();
        for expected in [
            CommandCategory::Tool,
            CommandCategory::Shape,
            CommandCategory::View,
            CommandCategory::Edit,
            CommandCategory::Selection,
            CommandCategory::Template,
            CommandCategory::Canvas,
        ] {
            assert!(
                categories.contains(&expected),
                "no command in category {:?}",
                expected
            );
        }
    }

    #[test]
    fn json_serializes() {
        let json = command_catalog_json();
        assert!(json.starts_with('['));
        // Round-trips back to a JSON array of the same length.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().expect("array");
        assert_eq!(arr.len(), command_catalog().len());
    }

    #[test]
    fn category_serializes_kebab_case() {
        let json = serde_json::to_string(&CommandCategory::Selection).unwrap();
        assert_eq!(json, "\"selection\"");
    }

    #[test]
    fn command_serializes_camel_case_fields() {
        let cmd = Command::new(
            "zoom-in",
            "Zoom In",
            CommandCategory::View,
            Some("Mod+="),
            "Zoom in.",
        );
        let value: serde_json::Value = serde_json::to_value(&cmd).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("defaultShortcut"));
        assert_eq!(obj["defaultShortcut"], "Mod+=");
        assert_eq!(obj["category"], "view");
    }

    #[test]
    fn missing_shortcut_is_omitted() {
        let cmd = command_catalog()
            .into_iter()
            .find(|c| c.id == "align-left")
            .unwrap();
        assert!(cmd.default_shortcut.is_none());
        let value: serde_json::Value = serde_json::to_value(&cmd).unwrap();
        assert!(!value.as_object().unwrap().contains_key("defaultShortcut"));
    }

    #[test]
    fn mod_token_used_for_primary_modifier() {
        let copy = command_catalog()
            .into_iter()
            .find(|c| c.id == "copy")
            .unwrap();
        assert_eq!(copy.default_shortcut.as_deref(), Some("Mod+C"));
    }
}
