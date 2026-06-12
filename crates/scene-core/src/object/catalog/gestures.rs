//! The hold-key gesture catalog: the canonical list of press-and-hold input
//! gestures (sibling to the click/shortcut [`commands`]), one source for the
//! gesture ids every shell keys behavior off.
//!
//! Data-only and pure (no time/rng/threads/IO). The shell owns the actual
//! pointer/keyboard wiring; this module freezes the ids, the held input each
//! gesture binds, and any numeric parameter.

use serde::Serialize;

/// Serialized kebab-case to match the wire model.
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
    /// Forcing free-form pen recognition while a modifier is held.
    Draw,
}

/// Which physical input must be HELD. The three input families the shell
/// distinguishes; serialized kebab-case.
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

/// Exactly one of `key`/`button`/`modifier` is populated — the one matching
/// `input`.
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

/// In display order grouped by category. The ids are FROZEN — downstream shell
/// code keys behavior off them.
pub fn object_gesture_catalog() -> Vec<ObjectGesture> {
    use ObjectGestureCategory::*;
    vec![
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
        ObjectGesture::new(
            "no-snap-alt",
            "Disable Snap (Alt)",
            Snap,
            HoldTrigger::modifier("Alt"),
            "Hold Alt to suppress snapping while moving or inserting.",
        ),
        ObjectGesture::new(
            "partial-erase-alt",
            "Partial Erase (Alt)",
            Erase,
            HoldTrigger::modifier("Alt"),
            "Hold Alt with the erase tool to erase partial geometry instead of whole objects.",
        ),
        ObjectGesture::new(
            "coarse-rotate-shift",
            "Fine Rotate (Shift)",
            Transform,
            HoldTrigger::modifier("Shift").with_degrees(15.0),
            "Rotation snaps to 15-degree steps by default; hold Shift to rotate freely (fine).",
        ),
        ObjectGesture::new(
            "detach-alt",
            "Detach Anchors (Alt)",
            Anchor,
            HoldTrigger::modifier("Alt"),
            "Hold Alt while dragging an anchored object to move it whole, ignoring its anchors and detaching them.",
        ),
        ObjectGesture::new(
            "free-recognize-shift",
            "Free-form Recognition (Shift)",
            Draw,
            HoldTrigger::modifier("Shift"),
            "Hold Shift while drawing with the pen to recognize the stroke as a free-form shape instead of snapping to a basic shape (release returns to Basic).",
        ),
    ]
}

/// The catalog serialized to JSON — the seam the shell consumes.
pub fn object_gesture_catalog_json() -> String {
    // Statically constructed, so serialization cannot fail.
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
            "free-recognize-shift",
        ];
        let ids: Vec<&str> = catalog.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids.len(), expected.len(), "catalog must hold exactly 9 gestures");
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
        assert_eq!(serde_json::to_string(&ObjectGestureCategory::Draw).unwrap(), "\"draw\"");
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

    /// Pins the exact serialized wire shape so a drift in ids, trigger fields, or
    /// the degrees param fails.
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
                "label": "Fine Rotate (Shift)",
                "category": "transform",
                "trigger": { "input": "modifier", "modifier": "Shift", "degrees": 15.0 },
                "description": "Rotation snaps to 15-degree steps by default; hold Shift to rotate freely (fine)."
            },
            {
                "id": "detach-alt",
                "label": "Detach Anchors (Alt)",
                "category": "anchor",
                "trigger": { "input": "modifier", "modifier": "Alt" },
                "description": "Hold Alt while dragging an anchored object to move it whole, ignoring its anchors and detaching them."
            },
            {
                "id": "free-recognize-shift",
                "label": "Free-form Recognition (Shift)",
                "category": "draw",
                "trigger": { "input": "modifier", "modifier": "Shift" },
                "description": "Hold Shift while drawing with the pen to recognize the stroke as a free-form shape instead of snapping to a basic shape (release returns to Basic)."
            }
        ]);
        let actual: serde_json::Value =
            serde_json::from_str(&object_gesture_catalog_json()).unwrap();
        assert_eq!(actual, expected);
    }
}
