//! OB1.2 — the object operation union + wire feature channels + undo capture.
//!
//! Every edit is one [`ObjectOp`]; there is a single op-apply path (P1). Each op
//! is internally tagged on `kind` (kebab-case) so it deserializes straight from
//! a `WireOp.propDelta` JSON value. Each op can derive its inverse for undo
//! (D21) — the inverse is itself a normal op authored through the same pipeline
//! (state rollback is never used), so undo composes with concurrent edits.

use serde::{Deserialize, Serialize};

use super::model::{
    Anchor, Comment, Fill, Geometry, Layout, Object, ObjectId, Stroke, Text, Transform3x3,
};

/// Three-state edit for an optional style field: distinguish "leave the current
/// value untouched" (the field is `None`/absent on the op) from "set to X"
/// (`Set`) and "remove the field" (`Clear`). Lets one `set-style` op touch fill
/// without implying anything about stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum FieldEdit<T> {
    Set { value: T },
    Clear,
}

impl<T> FieldEdit<T> {
    /// Resolve this edit against a current optional value, returning the new one.
    pub fn resolve(self, _current: Option<T>) -> Option<T> {
        match self {
            FieldEdit::Set { value } => Some(value),
            FieldEdit::Clear => None,
        }
    }
}

/// The object op union (OB1.2). `kind` is the kebab op name; the rest of the
/// fields are the per-property delta. This is what a `WireOp.propDelta` decodes
/// into server-side, so the wire stays decoupled from the op shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ObjectOp {
    /// Insert a fully-formed object. Inverse: `delete { id }`.
    #[serde(rename_all = "camelCase")]
    InsertObject { object: Object },

    /// Replace one object's whole geometry. Triggers region/tessellation rebake
    /// for that object only (P4). Inverse: `edit-geometry` with the old value.
    #[serde(rename_all = "camelCase")]
    EditGeometry { id: ObjectId, geometry: Geometry },

    /// Replace the 3x3 transform. 0-rebake (D7). Inverse: old transform.
    #[serde(rename_all = "camelCase")]
    SetTransform { id: ObjectId, transform: Transform3x3 },

    /// Edit fill and/or stroke. Each present `FieldEdit` is its own LWW property.
    #[serde(rename_all = "camelCase")]
    SetStyle {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<FieldEdit<Fill>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<FieldEdit<Stroke>>,
    },

    /// Replace the text runs atomically. `None` clears. Inverse: old text.
    #[serde(rename_all = "camelCase")]
    SetText {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<Text>,
    },

    /// Replace the whole anchor array (one LWW property). Inverse: old anchors.
    #[serde(rename_all = "camelCase")]
    SetAnchor { id: ObjectId, anchors: Vec<Anchor> },

    /// Set/clear auto-layout inputs (D3/OB3.A1). Inverse: old layout.
    #[serde(rename_all = "camelCase")]
    SetLayout {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<Layout>,
    },

    /// Set/clear the clip flag (D18). Inverse: old clip.
    #[serde(rename_all = "camelCase")]
    SetClip {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip: Option<bool>,
    },

    /// Append a comment (D20). Inverse: `set-comments` with the prior array.
    #[serde(rename_all = "camelCase")]
    AddComment { id: ObjectId, comment: Comment },

    /// Replace the whole comments array (D20). The general/undo form behind the
    /// `add-comment` append sugar; symmetric with `set-anchor` / `set-tags`.
    #[serde(rename_all = "camelCase")]
    SetComments { id: ObjectId, comments: Vec<Comment> },

    /// Replace the tag id array (D20). Inverse: old tags.
    #[serde(rename_all = "camelCase")]
    SetTags { id: ObjectId, tags: Vec<String> },

    /// Re-home into another children-group / canvas root (D3). Inverse: old
    /// parent + order.
    #[serde(rename_all = "camelCase")]
    Reparent {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<ObjectId>,
        order: String,
    },

    /// Change the fractional z-order key only. Inverse: old order.
    #[serde(rename_all = "camelCase")]
    Reorder { id: ObjectId, order: String },

    /// Delete an object (cascades children + peer anchors per OB3.S2). Inverse:
    /// re-insert the captured object snapshot.
    #[serde(rename_all = "camelCase")]
    Delete { id: ObjectId },

    /// Split a multi-subpath object into one object per listed contour (or all).
    /// New ids are caller-supplied. Inverse: `merge` of the produced ids.
    #[serde(rename_all = "camelCase")]
    Split {
        id: ObjectId,
        /// New object ids, one per produced contour (caller-allocated).
        new_ids: Vec<ObjectId>,
        /// Contour indices to peel off; empty => every contour.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        contours: Vec<i32>,
    },

    /// Combine sibling objects into one multi-subpath object. The `into` object
    /// (or the first id) keeps its style/transform. Inverse: `split`.
    #[serde(rename_all = "camelCase")]
    Merge {
        ids: Vec<ObjectId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        into: Option<ObjectId>,
    },

    /// Atomic batch (drag = 1 undo unit). All-or-nothing on validation failure.
    #[serde(rename_all = "camelCase")]
    Batch { ops: Vec<ObjectOp> },
}

impl ObjectOp {
    /// The kebab discriminant string (mirrors `WireOp.kind`).
    pub fn kind(&self) -> &'static str {
        match self {
            ObjectOp::InsertObject { .. } => "insert-object",
            ObjectOp::EditGeometry { .. } => "edit-geometry",
            ObjectOp::SetTransform { .. } => "set-transform",
            ObjectOp::SetStyle { .. } => "set-style",
            ObjectOp::SetText { .. } => "set-text",
            ObjectOp::SetAnchor { .. } => "set-anchor",
            ObjectOp::SetLayout { .. } => "set-layout",
            ObjectOp::SetClip { .. } => "set-clip",
            ObjectOp::AddComment { .. } => "add-comment",
            ObjectOp::SetComments { .. } => "set-comments",
            ObjectOp::SetTags { .. } => "set-tags",
            ObjectOp::Reparent { .. } => "reparent",
            ObjectOp::Reorder { .. } => "reorder",
            ObjectOp::Delete { .. } => "delete",
            ObjectOp::Split { .. } => "split",
            ObjectOp::Merge { .. } => "merge",
            ObjectOp::Batch { .. } => "batch",
        }
    }

    /// The object ids this op targets (for envelope `targetIds` derivation).
    /// The primary id rides `WireOp.objectId`; plural ops carry the rest in the
    /// propDelta payload.
    pub fn target_ids(&self) -> Vec<ObjectId> {
        match self {
            ObjectOp::InsertObject { object } => {
                let mut ids = vec![object.id.clone()];
                if let Some(p) = &object.parent {
                    ids.push(p.clone());
                }
                ids
            }
            ObjectOp::EditGeometry { id, .. }
            | ObjectOp::SetTransform { id, .. }
            | ObjectOp::SetStyle { id, .. }
            | ObjectOp::SetText { id, .. }
            | ObjectOp::SetAnchor { id, .. }
            | ObjectOp::SetLayout { id, .. }
            | ObjectOp::SetClip { id, .. }
            | ObjectOp::AddComment { id, .. }
            | ObjectOp::SetComments { id, .. }
            | ObjectOp::SetTags { id, .. }
            | ObjectOp::Reorder { id, .. }
            | ObjectOp::Delete { id } => vec![id.clone()],
            ObjectOp::Reparent { id, parent, .. } => {
                let mut ids = vec![id.clone()];
                if let Some(p) = parent {
                    ids.push(p.clone());
                }
                ids
            }
            ObjectOp::Split { id, new_ids, .. } => {
                let mut ids = vec![id.clone()];
                ids.extend(new_ids.iter().cloned());
                ids
            }
            ObjectOp::Merge { ids, into } => {
                let mut out = ids.clone();
                if let Some(i) = into {
                    if !out.contains(i) {
                        out.push(i.clone());
                    }
                }
                out
            }
            ObjectOp::Batch { ops } => {
                let mut out = Vec::new();
                for op in ops {
                    out.extend(op.target_ids());
                }
                out
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wire feature channels (OB1.2) — request/response RPC that replaces the
// bespoke REST surface. Server lowers mutating requests to ObjectOps through
// the same apply pipeline (OB3.S7); read requests reply directly. Types are
// authored here; wire.rs integration + server handlers are OB3.S7/OB4.5.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "feature", rename_all = "camelCase")]
pub enum FeatureRequest {
    CanvasSwitch {
        canvas_id: String,
    },
    CommentUpsert {
        canvas_id: String,
        object_id: ObjectId,
        comment: Comment,
    },
    /// template = styled objects (OB3.S5); server lowers to insert-object ops.
    TemplateApply {
        canvas_id: String,
        recipe: Vec<Object>,
        anchor_x: f64,
        anchor_y: f64,
    },
    ExportRequest {
        canvas_id: String,
        scope_ids: Vec<String>,
        export_type: String,
        request_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "feature", rename_all = "camelCase")]
pub enum FeatureResponse {
    CanvasSwitched { canvas_id: String, seq: u64, revision: u64 },
    CommentUpserted { object_id: ObjectId, comment_id: String },
    TemplateApplied { object_ids: Vec<String> },
    ExportReady { request_id: String, artifact_ref: String, content_type: String },
    FeatureError {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        message: String,
    },
}
