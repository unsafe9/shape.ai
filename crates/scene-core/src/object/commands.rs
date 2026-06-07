//! OB3.S9 — the object command catalog: the canonical list of user-facing
//! actions over objects and their default keyboard shortcuts.
//!
//! This data drives two surfaces: the U3 context menu and the U4 shortcut layer.
//! Keeping it in scene-core lets every platform shell render the same catalog
//! without re-declaring it, and `object_command_catalog_json()` is the wire seam
//! the shell consumes.
//!
//! Where a command maps 1:1 onto an [`ObjectOp`], `op_kind` carries the same
//! kebab discriminant string [`ObjectOp::kind`] returns, so the shell can route
//! a command straight to an op. Composite or shell-only actions (copy/paste —
//! clipboard I/O is shell-side per P1 — plus duplicate, select-all, undo, redo)
//! leave `op_kind` `None`.
//!
//! Shortcuts use the platform-agnostic `Mod` token for the primary modifier
//! (Cmd on macOS, Ctrl elsewhere); the shell resolves it per platform.

use serde::Serialize;

/// The functional grouping an object command belongs to. Serialized kebab-case
/// to match the rest of the wire model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectCommandCategory {
    Clipboard,
    Edit,
    Selection,
    Arrange,
    Order,
    History,
    Path,
    Style,
    Annotate,
}

/// A single object command entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectCommand {
    pub id: String,
    pub label: String,
    pub category: ObjectCommandCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_shortcut: Option<String>,
    pub description: String,
    /// The [`ObjectOp`](super::op::ObjectOp) kind this command lowers to, when it
    /// maps 1:1. `None` for composite or shell-only actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_kind: Option<String>,
}

impl ObjectCommand {
    fn new(
        id: &str,
        label: &str,
        category: ObjectCommandCategory,
        default_shortcut: Option<&str>,
        description: &str,
        op_kind: Option<&str>,
    ) -> Self {
        ObjectCommand {
            id: id.to_string(),
            label: label.to_string(),
            category,
            default_shortcut: default_shortcut.map(str::to_string),
            description: description.to_string(),
            op_kind: op_kind.map(str::to_string),
        }
    }
}

/// The full object command catalog, in display order grouped by category.
pub fn object_command_catalog() -> Vec<ObjectCommand> {
    use ObjectCommandCategory::*;
    vec![
        // Clipboard — clipboard I/O is shell-side (P1), so these carry no op kind.
        ObjectCommand::new(
            "copy",
            "Copy",
            Clipboard,
            Some("Mod+C"),
            "Copy the current selection to the clipboard.",
            None,
        ),
        ObjectCommand::new(
            "paste",
            "Paste",
            Clipboard,
            Some("Mod+V"),
            "Paste objects from the clipboard.",
            None,
        ),
        // Edit
        ObjectCommand::new(
            "duplicate",
            "Duplicate",
            Edit,
            Some("Mod+D"),
            "Duplicate the current selection in place.",
            None,
        ),
        ObjectCommand::new(
            "delete",
            "Delete",
            Edit,
            Some("Backspace"),
            "Delete the current selection.",
            Some("delete"),
        ),
        ObjectCommand::new(
            "group",
            "Group",
            Edit,
            Some("Mod+G"),
            "Group the selected objects under a new parent.",
            Some("reparent"),
        ),
        ObjectCommand::new(
            "ungroup",
            "Ungroup",
            Edit,
            Some("Mod+Shift+G"),
            "Ungroup the selected group back to its parent.",
            Some("reparent"),
        ),
        // Selection
        ObjectCommand::new(
            "select-all",
            "Select All",
            Selection,
            Some("Mod+A"),
            "Select all objects in the scene.",
            None,
        ),
        // Arrange — nudge the selection by transform.
        ObjectCommand::new(
            "nudge-up",
            "Nudge Up",
            Arrange,
            Some("Up"),
            "Move the selection up by one step.",
            Some("set-transform"),
        ),
        ObjectCommand::new(
            "nudge-down",
            "Nudge Down",
            Arrange,
            Some("Down"),
            "Move the selection down by one step.",
            Some("set-transform"),
        ),
        ObjectCommand::new(
            "nudge-left",
            "Nudge Left",
            Arrange,
            Some("Left"),
            "Move the selection left by one step.",
            Some("set-transform"),
        ),
        ObjectCommand::new(
            "nudge-right",
            "Nudge Right",
            Arrange,
            Some("Right"),
            "Move the selection right by one step.",
            Some("set-transform"),
        ),
        // Order — z-order changes via the fractional order key.
        ObjectCommand::new(
            "bring-forward",
            "Bring Forward",
            Order,
            Some("Mod+]"),
            "Move the selection one step toward the front of the z-order.",
            Some("reorder"),
        ),
        ObjectCommand::new(
            "send-backward",
            "Send Backward",
            Order,
            Some("Mod+["),
            "Move the selection one step toward the back of the z-order.",
            Some("reorder"),
        ),
        ObjectCommand::new(
            "bring-to-front",
            "Bring to Front",
            Order,
            Some("]"),
            "Bring the selection to the front of the z-order.",
            Some("reorder"),
        ),
        ObjectCommand::new(
            "send-to-back",
            "Send to Back",
            Order,
            Some("["),
            "Send the selection to the back of the z-order.",
            Some("reorder"),
        ),
        // History — composite undo/redo over the op log; no single op kind.
        ObjectCommand::new(
            "undo",
            "Undo",
            History,
            Some("Mod+Z"),
            "Undo the last edit.",
            None,
        ),
        ObjectCommand::new(
            "redo",
            "Redo",
            History,
            Some("Mod+Shift+Z"),
            "Redo the last undone edit.",
            None,
        ),
        // Path
        ObjectCommand::new(
            "split",
            "Split",
            Path,
            None,
            "Split a multi-subpath object into one object per contour.",
            Some("split"),
        ),
        ObjectCommand::new(
            "merge",
            "Merge",
            Path,
            None,
            "Merge the selected sibling objects into one multi-subpath object.",
            Some("merge"),
        ),
        // Style
        ObjectCommand::new(
            "set-fill",
            "Set Fill",
            Style,
            None,
            "Set the fill of the selection.",
            Some("set-style"),
        ),
        ObjectCommand::new(
            "set-stroke",
            "Set Stroke",
            Style,
            None,
            "Set the stroke of the selection.",
            Some("set-style"),
        ),
        ObjectCommand::new(
            "edit-text",
            "Edit Text",
            Style,
            Some("Enter"),
            "Edit the text of the selection.",
            Some("set-text"),
        ),
        // Annotate
        ObjectCommand::new(
            "add-comment",
            "Add Comment",
            Annotate,
            Some("Mod+Shift+M"),
            "Add a comment to the selection.",
            Some("add-comment"),
        ),
        ObjectCommand::new(
            "toggle-clip",
            "Toggle Clip",
            Annotate,
            None,
            "Toggle whether the selection clips its children.",
            Some("set-clip"),
        ),
        ObjectCommand::new(
            "set-tags",
            "Set Tags",
            Annotate,
            None,
            "Set the tags on the selection.",
            Some("set-tags"),
        ),
    ]
}

/// The object command catalog serialized to JSON — the seam the shell consumes
/// (`object_command_catalog() -> JSON`).
pub fn object_command_catalog_json() -> String {
    // The catalog is statically constructed, so serialization cannot fail; the
    // `expect` documents that invariant rather than masking a real error.
    serde_json::to_string(&object_command_catalog()).expect("object command catalog serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::op::ObjectOp;
    use std::collections::HashSet;

    #[test]
    fn catalog_is_non_empty() {
        assert!(!object_command_catalog().is_empty());
    }

    #[test]
    fn ids_are_unique() {
        let catalog = object_command_catalog();
        let mut seen = HashSet::new();
        for cmd in &catalog {
            assert!(seen.insert(cmd.id.as_str()), "duplicate id: {}", cmd.id);
        }
        assert_eq!(seen.len(), catalog.len());
    }

    fn find(id: &str) -> ObjectCommand {
        object_command_catalog()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no command {id}"))
    }

    #[test]
    fn undo_has_mod_z() {
        assert_eq!(find("undo").default_shortcut.as_deref(), Some("Mod+Z"));
    }

    #[test]
    fn redo_has_mod_shift_z() {
        assert_eq!(find("redo").default_shortcut.as_deref(), Some("Mod+Shift+Z"));
    }

    #[test]
    fn requested_commands_all_present() {
        let catalog = object_command_catalog();
        let ids: HashSet<&str> = catalog.iter().map(|c| c.id.as_str()).collect();
        for expected in [
            "copy",
            "paste",
            "duplicate",
            "delete",
            "select-all",
            "group",
            "ungroup",
            "nudge-up",
            "nudge-down",
            "nudge-left",
            "nudge-right",
            "bring-forward",
            "send-backward",
            "bring-to-front",
            "send-to-back",
            "undo",
            "redo",
            "split",
            "merge",
            "set-fill",
            "set-stroke",
            "edit-text",
            "add-comment",
            "toggle-clip",
            "set-tags",
        ] {
            assert!(ids.contains(expected), "missing command: {expected}");
        }
    }

    #[test]
    fn op_kinds_are_real_object_op_discriminants() {
        // Every `op_kind` the catalog claims must equal some `ObjectOp::kind()`.
        // Build the authoritative set straight from the op enum so this stays in
        // lockstep with op.rs without re-listing the kebab strings here.
        let known: HashSet<&'static str> = [
            ObjectOp::InsertObject {
                object: crate::object::model::Object::new(
                    crate::object::model::ObjectId::from("probe"),
                    "a".to_string(),
                    crate::object::model::Geometry::from_subpaths(
                        vec![],
                        crate::object::model::FillRule::NonZero,
                    ),
                ),
            }
            .kind(),
            ObjectOp::EditGeometry {
                id: "x".into(),
                geometry: crate::object::model::Geometry::from_subpaths(
                    vec![],
                    crate::object::model::FillRule::NonZero,
                ),
            }
            .kind(),
            ObjectOp::SetTransform {
                id: "x".into(),
                transform: crate::object::model::Transform3x3::IDENTITY,
            }
            .kind(),
            ObjectOp::SetStyle {
                id: "x".into(),
                fill: None,
                stroke: None,
            }
            .kind(),
            ObjectOp::SetText {
                id: "x".into(),
                text: None,
            }
            .kind(),
            ObjectOp::SetAnchor {
                id: "x".into(),
                anchors: vec![],
            }
            .kind(),
            ObjectOp::SetLayout {
                id: "x".into(),
                layout: None,
            }
            .kind(),
            ObjectOp::SetClip {
                id: "x".into(),
                clip: None,
            }
            .kind(),
            ObjectOp::AddComment {
                id: "x".into(),
                comment: crate::object::model::Comment {
                    id: "c".into(),
                    author: String::new(),
                    body: String::new(),
                    at: None,
                    resolved: false,
                },
            }
            .kind(),
            ObjectOp::SetComments {
                id: "x".into(),
                comments: vec![],
            }
            .kind(),
            ObjectOp::SetTags {
                id: "x".into(),
                tags: vec![],
            }
            .kind(),
            ObjectOp::Reparent {
                id: "x".into(),
                parent: None,
                order: "a".into(),
            }
            .kind(),
            ObjectOp::Reorder {
                id: "x".into(),
                order: "a".into(),
            }
            .kind(),
            ObjectOp::Delete { id: "x".into() }.kind(),
            ObjectOp::Split {
                id: "x".into(),
                new_ids: vec![],
                contours: vec![],
            }
            .kind(),
            ObjectOp::Merge {
                ids: vec![],
                into: None,
            }
            .kind(),
            ObjectOp::Batch { ops: vec![] }.kind(),
        ]
        .into_iter()
        .collect();
        for cmd in object_command_catalog() {
            if let Some(kind) = &cmd.op_kind {
                assert!(
                    known.contains(kind.as_str()),
                    "command {} maps to unknown op kind {}",
                    cmd.id,
                    kind
                );
            }
        }
    }

    #[test]
    fn shell_only_commands_have_no_op_kind() {
        for id in ["copy", "paste", "duplicate", "select-all", "undo", "redo"] {
            assert!(
                find(id).op_kind.is_none(),
                "{id} should not map to an op kind"
            );
        }
    }

    #[test]
    fn category_serializes_kebab_case() {
        let json = serde_json::to_string(&ObjectCommandCategory::Order).unwrap();
        assert_eq!(json, "\"order\"");
    }

    #[test]
    fn command_serializes_camel_case_fields() {
        let value: serde_json::Value = serde_json::to_value(find("delete")).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("defaultShortcut"));
        assert!(obj.contains_key("opKind"));
        assert_eq!(obj["opKind"], "delete");
    }

    #[test]
    fn missing_shortcut_is_omitted() {
        let value: serde_json::Value = serde_json::to_value(find("set-tags")).unwrap();
        assert!(!value.as_object().unwrap().contains_key("defaultShortcut"));
    }

    #[test]
    fn json_round_trips() {
        let json = object_command_catalog_json();
        assert!(json.starts_with('['));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().expect("array");
        assert_eq!(arr.len(), object_command_catalog().len());
    }
}
