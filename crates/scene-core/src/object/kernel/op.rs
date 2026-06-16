//! The object operation union + wire feature channels. Every edit is one
//! [`ObjectOp`], internally tagged on `kind` (kebab-case) so it deserializes
//! straight from a `WireOp.propDelta` JSON value. The inverse is itself a normal
//! op authored through the same pipeline (state rollback is never used).

use serde::{Deserialize, Serialize};

use crate::object::model::{
    Anchor, Comment, Fill, Geometry, Layout, Object, ObjectId, Sizing, Stroke, Text, Transform3x3,
};

/// Three-state edit for an optional field: absent leaves the current value, `Set`
/// replaces it, `Clear` removes it. Lets one `set-style` op touch fill without
/// implying anything about stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum FieldEdit<T> {
    Set { value: T },
    Clear,
}

impl<T> FieldEdit<T> {
    /// `Some(v)` -> `Set{v}`, `None` -> `Clear`. Captures the inverse of a style
    /// edit from the prior value.
    pub fn from_option(value: Option<T>) -> Self {
        match value {
            Some(value) => FieldEdit::Set { value },
            None => FieldEdit::Clear,
        }
    }

    pub fn resolve(self, _current: Option<T>) -> Option<T> {
        match self {
            FieldEdit::Set { value } => Some(value),
            FieldEdit::Clear => None,
        }
    }
}

/// `kind` is the kebab op name; the rest are the per-property delta — what a
/// `WireOp.propDelta` decodes into server-side, so the wire stays decoupled from
/// the op shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ObjectOp {
    #[serde(rename_all = "camelCase")]
    InsertObject { object: Object },

    /// Triggers region/tessellation rebake for that object only.
    #[serde(rename_all = "camelCase")]
    EditGeometry { id: ObjectId, geometry: Geometry },

    /// 0-rebake.
    #[serde(rename_all = "camelCase")]
    SetTransform {
        id: ObjectId,
        // See `Object.transform`: emit the transparent matrix AS its bare array.
        #[cfg_attr(feature = "ts-gen", ts(as = "[[f64; 3]; 3]"))]
        transform: Transform3x3,
    },

    /// Each present `FieldEdit` is its own LWW property.
    #[serde(rename_all = "camelCase")]
    SetStyle {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<FieldEdit<Fill>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<FieldEdit<Stroke>>,
    },

    #[serde(rename_all = "camelCase")]
    SetText {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<Text>,
    },

    #[serde(rename_all = "camelCase")]
    SetAnchor { id: ObjectId, anchors: Vec<Anchor> },

    #[serde(rename_all = "camelCase")]
    SetLayout {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<Layout>,
    },

    #[serde(rename_all = "camelCase")]
    SetClip {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip: Option<bool>,
    },

    /// Per-object sizing (Hug/Fill/Fixed per axis). LWW on the `sizing` property.
    #[serde(rename_all = "camelCase")]
    SetSizing {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sizing: Option<Sizing>,
    },

    /// Panel header metadata. Each field is optional — an absent field leaves the
    /// stored value unchanged. `name` is a `FieldEdit` so it can be cleared; the
    /// bools are a plain `Option<bool>` (absent = unchanged, present = the value).
    #[serde(rename_all = "camelCase")]
    SetMeta {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<FieldEdit<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locked: Option<bool>,
    },

    /// Extract a geometry's dominant orientation into `transform` and rewrite the
    /// geometry axis-aligned. A geometry-rebake op (NOT zero-rebake), driven by an
    /// explicit user action; its inverse restores the prior geometry + transform.
    #[serde(rename_all = "camelCase")]
    Canonicalize { id: ObjectId },

    /// Append sugar; inverse is `set-comments` with the prior array.
    #[serde(rename_all = "camelCase")]
    AddComment { id: ObjectId, comment: Comment },

    #[serde(rename_all = "camelCase")]
    SetComments { id: ObjectId, comments: Vec<Comment> },

    #[serde(rename_all = "camelCase")]
    SetTags { id: ObjectId, tags: Vec<String> },

    #[serde(rename_all = "camelCase")]
    Reparent {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<ObjectId>,
        order: String,
    },

    #[serde(rename_all = "camelCase")]
    Reorder { id: ObjectId, order: String },

    /// Cascades children + peer anchors. Inverse re-inserts the captured snapshot.
    #[serde(rename_all = "camelCase")]
    Delete { id: ObjectId },

    /// Split a multi-subpath object into one object per listed contour (or all).
    #[serde(rename_all = "camelCase")]
    Split {
        id: ObjectId,
        new_ids: Vec<ObjectId>,
        /// Contour indices to peel off; empty => every contour.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        contours: Vec<i32>,
    },

    /// The `into` object (or the first id) keeps its style/transform.
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
            ObjectOp::SetSizing { .. } => "set-sizing",
            ObjectOp::SetMeta { .. } => "set-meta",
            ObjectOp::Canonicalize { .. } => "canonicalize",
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
            | ObjectOp::SetSizing { id, .. }
            | ObjectOp::SetMeta { id, .. }
            | ObjectOp::Canonicalize { id }
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

// Request/response RPC: the server lowers mutating requests to ObjectOps through
// the same apply pipeline; read requests reply directly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
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
    /// Styled objects the server lowers to insert-object ops.
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
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "feature", rename_all = "camelCase")]
pub enum FeatureResponse {
    CanvasSwitched {
        canvas_id: String,
        // u64 is a JSON number, not ts-rs's default `bigint`.
        #[cfg_attr(feature = "ts-gen", ts(type = "number"))]
        seq: u64,
        #[cfg_attr(feature = "ts-gen", ts(type = "number"))]
        revision: u64,
    },
    CommentUpserted { object_id: ObjectId, comment_id: String },
    TemplateApplied { object_ids: Vec<String> },
    ExportReady { request_id: String, artifact_ref: String, content_type: String },
    FeatureError {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        message: String,
    },
}
