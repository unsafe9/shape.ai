//! The hold-key gesture catalog: the canonical list of press-and-hold input
//! gestures over the canvas (sibling to the click/shortcut [`commands`]).
//!
//! [`commands`](super::commands) covers discrete click/shortcut actions, but a
//! second family of inputs are *held*: Space to pan, the middle mouse button to
//! pan, Shift/Mod to add to the selection, Alt to suppress snapping or erase
//! partially, and Shift to coarsen rotation. These were undocumented shell
//! constants; surfacing them here gives every platform shell one source for the
//! gesture ids it keys behavior off, mirroring `object_command_catalog_json()`.
//!
//! These are data-only and pure: no time, randomness, threads, or I/O. The shell
//! still owns the actual pointer/keyboard wiring; this module only freezes the
//! ids, the held input each gesture binds, and any numeric parameter.

use serde::Serialize;

/// The functional grouping a gesture belongs to. Serialized kebab-case to match
/// the rest of the wire model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectGestureCategory {
    /// Viewport panning while a key/button is held.
    Pan,
    /// Adding to the current selection while a modifier is held.
    Select,
    /// Suppressing snap-to-geometry while a modifier is held.
    Snap,
    /// Partial (eraser-style) erase while a modifier is held.
    Erase,
    /// Coarsening a transform (rotation step) while a modifier is held.
    Transform,
    /// Detaching anchors (move whole, ignoring attachments) while a modifier is held.
    Anchor,
}

/// Which physical input must be HELD to engage a gesture. Serialized kebab-case;
/// `Key`/`Button`/`Modifier` are the three input families the shell distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HoldInput {
    /// A keyboard key held down (e.g. `Space`), carried in `key`.
    Key,
    /// A mouse button held down (e.g. `middle`), carried in `button`.
    Button,
    /// A keyboard modifier held down (`Shift`/`Alt`/`Mod`), carried in `modifier`.
    Modifier,
}

/// The hold-key trigger descriptor: which input family is held, the concrete
/// token within it, and an optional numeric parameter (e.g. the coarse-rotate
/// step in degrees). Exactly one of `key`/`button`/`modifier` is populated, the
/// one matching `input`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldTrigger {
    pub input: HoldInput,
    /// The held key token (e.g. `Space`), set iff `input` is `Key`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The held mouse button (e.g. `middle`), set iff `input` is `Button`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    /// The held modifier (`Shift`/`Alt`/`Mod`), set iff `input` is `Modifier`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<String>,
    /// An optional numeric parameter, e.g. the coarse-rotate step in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degrees: Option<f64>,
}

impl HoldTrigger {
    fn key(key: &str) -> Self {
        HoldTrigger { input: HoldInput::Key, key: Some(key.to_string()), button: None, modifier: None, degrees: None }
    }

    fn button(button: &str) -> Self {
        HoldTrigger { input: HoldInput::Button, key: None, button: Some(button.to_string()), modifier: None, degrees: None }
    }

    fn modifier(modifier: &str) -> Self {
        HoldTrigger { input: HoldInput::Modifier, key: None, button: None, modifier: Some(modifier.to_string()), degrees: None }
    }

    fn with_degrees(mut self, degrees: f64) -> Self {
        self.degrees = Some(degrees);
        self
    }
}

/// A single hold-key gesture entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectGesture {
    pub id: String,
    pub label: String,
    pub category: ObjectGestureCategory,
    pub trigger: HoldTrigger,
    pub description: String,
}

impl ObjectGesture {
    fn new(
        id: &str,
        label: &str,
        category: ObjectGestureCategory,
        trigger: HoldTrigger,
        description: &str,
    ) -> Self {
        ObjectGesture {
            id: id.to_string(),
            label: label.to_string(),
            category,
            trigger,
            description: description.to_string(),
        }
    }
}

/// The full hold-key gesture catalog, in display order grouped by category. The
/// ids here are FROZEN — downstream shell code keys behavior off them.
pub fn object_gesture_catalog() -> Vec<ObjectGesture> {
    use ObjectGestureCategory::*;
    vec![
        // Pan — hold to temporarily drag the viewport.
        ObjectGesture::new(
            "pan-space",
            "Pan (Space)",
            Pan,
            HoldTrigger::key("Space"),
            "Hold Space to pan the viewport by dragging.",
        ),
        ObjectGesture::new(
            "pan-middle",
            "Pan (Middle Button)",
            Pan,
            HoldTrigger::button("middle"),
            "Hold the middle mouse button to pan the viewport by dragging.",
        ),
        // Select — hold to add to the current selection instead of replacing it.
        ObjectGesture::new(
            "additive-select-shift",
            "Add to Selection (Shift)",
            Select,
            HoldTrigger::modifier("Shift"),
            "Hold Shift while selecting to add to the current selection.",
        ),
        ObjectGesture::new(
            "additive-select-mod",
            "Add to Selection (Cmd/Ctrl)",
            Select,
            HoldTrigger::modifier("Mod"),
            "Hold the primary modifier (Cmd/Ctrl) while selecting to add to the current selection.",
        ),
        // Snap — hold to suppress snap-to-geometry for free placement.
        ObjectGesture::new(
            "no-snap-alt",
            "Disable Snap (Alt)",
            Snap,
            HoldTrigger::modifier("Alt"),
            "Hold Alt to suppress snapping while moving or inserting.",
        ),
        // Erase — hold for eraser-style partial erase.
        ObjectGesture::new(
            "partial-erase-alt",
            "Partial Erase (Alt)",
            Erase,
            HoldTrigger::modifier("Alt"),
            "Hold Alt with the erase tool to erase partial geometry instead of whole objects.",
        ),
        // Transform — hold to coarsen the rotation step (D5: 15 degrees per tick).
        ObjectGesture::new(
            "coarse-rotate-shift",
            "Coarse Rotate (Shift)",
            Transform,
            HoldTrigger::modifier("Shift").with_degrees(15.0),
            "Hold Shift while rotating to snap to 15-degree steps (90 degrees = 6 ticks).",
        ),
        // Anchor — hold to detach an anchored object and move it wholesale (DU4).
        ObjectGesture::new(
            "detach-alt",
            "Detach Anchors (Alt)",
            Anchor,
            HoldTrigger::modifier("Alt"),
            "Hold Alt while dragging an anchored object to move it whole, ignoring its anchors and detaching them.",
        ),
    ]
}

/// The hold-key gesture catalog serialized to JSON — the seam the shell consumes
/// (`object_gesture_catalog() -> JSON`).
pub fn object_gesture_catalog_json() -> String {
    // The catalog is statically constructed, so serialization cannot fail; the
    // `expect` documents that invariant rather than masking a real error.
    serde_json::to_string(&object_gesture_catalog()).expect("object gesture catalog serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn find(id: &str) -> ObjectGesture {
        object_gesture_catalog()
            .into_iter()
            .find(|g| g.id == id)
            .unwrap_or_else(|| panic!("no gesture {id}"))
    }

    #[test]
    fn catalog_is_non_empty() {
        assert!(!object_gesture_catalog().is_empty());
    }

    #[test]
    fn all_frozen_ids_present_exactly_once() {
        let catalog = object_gesture_catalog();
        let expected = [
            "pan-space",
            "pan-middle",
            "additive-select-shift",
            "additive-select-mod",
            "no-snap-alt",
            "partial-erase-alt",
            "coarse-rotate-shift",
            "detach-alt",
        ];
        // Exactly the 8 ids, each once.
        let ids: Vec<&str> = catalog.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids.len(), expected.len(), "catalog must hold exactly 8 gestures");
        let mut seen = HashSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "duplicate id: {id}");
        }
        for id in expected {
            assert!(seen.contains(id), "missing gesture: {id}");
        }
    }

    #[test]
    fn coarse_rotate_carries_fifteen_degrees() {
        let g = find("coarse-rotate-shift");
        assert_eq!(g.trigger.input, HoldInput::Modifier);
        assert_eq!(g.trigger.modifier.as_deref(), Some("Shift"));
        assert_eq!(g.trigger.degrees, Some(15.0));
    }

    #[test]
    fn non_rotate_gestures_carry_no_degrees() {
        for g in object_gesture_catalog() {
            if g.id != "coarse-rotate-shift" {
                assert_eq!(g.trigger.degrees, None, "{} should carry no degrees", g.id);
            }
        }
    }

    #[test]
    fn each_trigger_populates_exactly_its_input_field() {
        for g in object_gesture_catalog() {
            let t = &g.trigger;
            match t.input {
                HoldInput::Key => {
                    assert!(t.key.is_some() && t.button.is_none() && t.modifier.is_none(), "{} key trigger", g.id);
                }
                HoldInput::Button => {
                    assert!(t.button.is_some() && t.key.is_none() && t.modifier.is_none(), "{} button trigger", g.id);
                }
                HoldInput::Modifier => {
                    assert!(t.modifier.is_some() && t.key.is_none() && t.button.is_none(), "{} modifier trigger", g.id);
                }
            }
        }
    }

    #[test]
    fn category_serializes_kebab_case() {
        assert_eq!(serde_json::to_string(&ObjectGestureCategory::Transform).unwrap(), "\"transform\"");
        assert_eq!(serde_json::to_string(&ObjectGestureCategory::Pan).unwrap(), "\"pan\"");
        assert_eq!(serde_json::to_string(&ObjectGestureCategory::Anchor).unwrap(), "\"anchor\"");
    }

    #[test]
    fn input_serializes_kebab_case() {
        assert_eq!(serde_json::to_string(&HoldInput::Modifier).unwrap(), "\"modifier\"");
        assert_eq!(serde_json::to_string(&HoldInput::Button).unwrap(), "\"button\"");
    }

    #[test]
    fn json_round_trips() {
        let json = object_gesture_catalog_json();
        assert!(json.starts_with('['));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().expect("array");
        assert_eq!(arr.len(), object_gesture_catalog().len());
    }

    /// Contract snapshot: pin the exact serialized wire shape of the catalog so a
    /// drift in ids, trigger fields, or the degrees param fails the test.
    #[test]
    fn catalog_matches_wire_snapshot() {
        let expected = serde_json::json!([
            {
                "id": "pan-space",
                "label": "Pan (Space)",
                "category": "pan",
                "trigger": { "input": "key", "key": "Space" },
                "description": "Hold Space to pan the viewport by dragging."
            },
            {
                "id": "pan-middle",
                "label": "Pan (Middle Button)",
                "category": "pan",
                "trigger": { "input": "button", "button": "middle" },
                "description": "Hold the middle mouse button to pan the viewport by dragging."
            },
            {
                "id": "additive-select-shift",
                "label": "Add to Selection (Shift)",
                "category": "select",
                "trigger": { "input": "modifier", "modifier": "Shift" },
                "description": "Hold Shift while selecting to add to the current selection."
            },
            {
                "id": "additive-select-mod",
                "label": "Add to Selection (Cmd/Ctrl)",
                "category": "select",
                "trigger": { "input": "modifier", "modifier": "Mod" },
                "description": "Hold the primary modifier (Cmd/Ctrl) while selecting to add to the current selection."
            },
            {
                "id": "no-snap-alt",
                "label": "Disable Snap (Alt)",
                "category": "snap",
                "trigger": { "input": "modifier", "modifier": "Alt" },
                "description": "Hold Alt to suppress snapping while moving or inserting."
            },
            {
                "id": "partial-erase-alt",
                "label": "Partial Erase (Alt)",
                "category": "erase",
                "trigger": { "input": "modifier", "modifier": "Alt" },
                "description": "Hold Alt with the erase tool to erase partial geometry instead of whole objects."
            },
            {
                "id": "coarse-rotate-shift",
                "label": "Coarse Rotate (Shift)",
                "category": "transform",
                "trigger": { "input": "modifier", "modifier": "Shift", "degrees": 15.0 },
                "description": "Hold Shift while rotating to snap to 15-degree steps (90 degrees = 6 ticks)."
            },
            {
                "id": "detach-alt",
                "label": "Detach Anchors (Alt)",
                "category": "anchor",
                "trigger": { "input": "modifier", "modifier": "Alt" },
                "description": "Hold Alt while dragging an anchored object to move it whole, ignoring its anchors and detaching them."
            }
        ]);
        let actual: serde_json::Value =
            serde_json::from_str(&object_gesture_catalog_json()).unwrap();
        assert_eq!(actual, expected);
    }
}
