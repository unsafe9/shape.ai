//! Object op-apply + inverse-op capture (foundation slice of OB3.S1/S8/D21).
//!
//! This is the single source of truth for applying an [`ObjectOp`] to an
//! [`ObjectScene`]. Each apply returns the **inverse op** — a normal op that,
//! authored through this same path, undoes the edit (D21: reverse-op, not state
//! rollback, so undo composes with concurrent edits). Per-property seq LWW
//! (PropertyStore in `lww.rs`) is layered on in OB3.S1; this slice mutates the
//! scene directly and is enough to prove the vertical slice (OB2.1).
//!
//! Pure: no IO/time/rng. `Batch` is atomic — it applies to a clone and commits
//! only if every child op succeeds.

use super::model::{Anchor, Comment, Fill, Layout, ObjectId, ObjectScene, Stroke};
use super::op::{FieldEdit, ObjectOp};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    NotFound(ObjectId),
    DuplicateId(ObjectId),
    BadGeometry(String),
    Cycle(ObjectId),
    MissingAnchorTarget(ObjectId),
    Unsupported(&'static str),
}

impl core::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ApplyError::NotFound(id) => write!(f, "object not found: {id}"),
            ApplyError::DuplicateId(id) => write!(f, "duplicate object id: {id}"),
            ApplyError::BadGeometry(e) => write!(f, "bad geometry: {e}"),
            ApplyError::Cycle(id) => write!(f, "reparent would create a cycle: {id}"),
            ApplyError::MissingAnchorTarget(id) => write!(f, "anchor target missing: {id}"),
            ApplyError::Unsupported(what) => write!(f, "unsupported op: {what}"),
        }
    }
}

/// Apply `op` to `scene`, returning the inverse op for undo (D21). On error the
/// scene is left untouched (top-level ops mutate in place only after their
/// checks pass; `Batch` commits atomically via a clone).
pub fn apply_object_op(scene: &mut ObjectScene, op: ObjectOp) -> Result<ObjectOp, ApplyError> {
    let inverse = apply_inner(scene, op)?;
    scene.scene_version += 1;
    Ok(inverse)
}

fn index_of(scene: &ObjectScene, id: &str) -> Result<usize, ApplyError> {
    scene
        .objects
        .iter()
        .position(|o| o.id == id)
        .ok_or_else(|| ApplyError::NotFound(id.to_string()))
}

fn apply_inner(scene: &mut ObjectScene, op: ObjectOp) -> Result<ObjectOp, ApplyError> {
    match op {
        ObjectOp::InsertObject { mut object } => {
            if scene.objects.iter().any(|o| o.id == object.id) {
                return Err(ApplyError::DuplicateId(object.id));
            }
            object
                .ensure_parsed()
                .map_err(ApplyError::BadGeometry)?;
            let id = object.id.clone();
            scene.objects.push(object);
            Ok(ObjectOp::Delete { id })
        }

        ObjectOp::Delete { id } => {
            let idx = index_of(scene, &id)?;
            let removed = scene.objects.remove(idx);
            // Capture + prune peer anchors that referenced the deleted object so
            // the inverse can restore them alongside re-inserting the object.
            let mut peer_restores: Vec<ObjectOp> = Vec::new();
            for peer in &mut scene.objects {
                if peer.anchors.iter().any(|a| a.target == id) {
                    let before = peer.anchors.clone();
                    peer.anchors.retain(|a| a.target != id);
                    peer_restores.push(ObjectOp::SetAnchor { id: peer.id.clone(), anchors: before });
                }
            }
            let mut inverse = vec![ObjectOp::InsertObject { object: removed }];
            inverse.extend(peer_restores);
            if inverse.len() == 1 {
                Ok(inverse.pop().expect("len checked"))
            } else {
                Ok(ObjectOp::Batch { ops: inverse })
            }
        }

        ObjectOp::EditGeometry { id, mut geometry } => {
            geometry.ensure_parsed().map_err(ApplyError::BadGeometry)?;
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].geometry.clone();
            scene.objects[idx].geometry = geometry;
            Ok(ObjectOp::EditGeometry { id, geometry: old })
        }

        ObjectOp::SetTransform { id, transform } => {
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].transform;
            scene.objects[idx].transform = transform;
            Ok(ObjectOp::SetTransform { id, transform: old })
        }

        ObjectOp::SetStyle { id, fill, stroke } => {
            let idx = index_of(scene, &id)?;
            let inv_fill = fill.map(|edit| {
                let old = scene.objects[idx].fill.clone();
                scene.objects[idx].fill = edit.resolve(old.clone());
                field_edit_from_old::<Fill>(old)
            });
            let inv_stroke = stroke.map(|edit| {
                let old = scene.objects[idx].stroke.clone();
                scene.objects[idx].stroke = edit.resolve(old.clone());
                field_edit_from_old::<Stroke>(old)
            });
            Ok(ObjectOp::SetStyle { id, fill: inv_fill, stroke: inv_stroke })
        }

        ObjectOp::SetText { id, text } => {
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].text.clone();
            scene.objects[idx].text = text;
            Ok(ObjectOp::SetText { id, text: old })
        }

        ObjectOp::SetAnchor { id, anchors } => {
            for a in &anchors {
                if !scene.objects.iter().any(|o| o.id == a.target) {
                    return Err(ApplyError::MissingAnchorTarget(a.target.clone()));
                }
            }
            let idx = index_of(scene, &id)?;
            let old: Vec<Anchor> = scene.objects[idx].anchors.clone();
            scene.objects[idx].anchors = anchors;
            Ok(ObjectOp::SetAnchor { id, anchors: old })
        }

        ObjectOp::SetLayout { id, layout } => {
            let idx = index_of(scene, &id)?;
            let old: Option<Layout> = scene.objects[idx].layout.clone();
            scene.objects[idx].layout = layout;
            Ok(ObjectOp::SetLayout { id, layout: old })
        }

        ObjectOp::SetClip { id, clip } => {
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].clip;
            scene.objects[idx].clip = clip;
            Ok(ObjectOp::SetClip { id, clip: old })
        }

        ObjectOp::AddComment { id, comment } => {
            let idx = index_of(scene, &id)?;
            let old: Vec<Comment> = scene.objects[idx].comments.clone();
            scene.objects[idx].comments.push(comment);
            Ok(ObjectOp::SetComments { id, comments: old })
        }

        ObjectOp::SetComments { id, comments } => {
            let idx = index_of(scene, &id)?;
            let old: Vec<Comment> = scene.objects[idx].comments.clone();
            scene.objects[idx].comments = comments;
            Ok(ObjectOp::SetComments { id, comments: old })
        }

        ObjectOp::SetTags { id, tags } => {
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].tags.clone();
            scene.objects[idx].tags = tags;
            Ok(ObjectOp::SetTags { id, tags: old })
        }

        ObjectOp::Reorder { id, order } => {
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].order.clone();
            scene.objects[idx].order = order;
            Ok(ObjectOp::Reorder { id, order: old })
        }

        ObjectOp::Reparent { id, parent, order } => {
            if let Some(p) = &parent {
                if would_cycle(scene, &id, p) {
                    return Err(ApplyError::Cycle(id));
                }
            }
            let idx = index_of(scene, &id)?;
            let old_parent = scene.objects[idx].parent.clone();
            let old_order = scene.objects[idx].order.clone();
            scene.objects[idx].parent = parent;
            scene.objects[idx].order = order;
            Ok(ObjectOp::Reparent { id, parent: old_parent, order: old_order })
        }

        ObjectOp::Split { .. } => Err(ApplyError::Unsupported("split (OB3.S3)")),
        ObjectOp::Merge { .. } => Err(ApplyError::Unsupported("merge (OB3.S3)")),

        ObjectOp::Batch { ops } => {
            // Atomic: apply to a clone; commit only if every child succeeds.
            let mut working = scene.clone();
            let mut inverses: Vec<ObjectOp> = Vec::with_capacity(ops.len());
            for child in ops {
                let inv = apply_inner(&mut working, child)?;
                inverses.push(inv);
            }
            inverses.reverse();
            *scene = working;
            Ok(ObjectOp::Batch { ops: inverses })
        }
    }
}

/// Build the inverse `FieldEdit` from a captured old optional value: `Some` ->
/// `Set{old}`, `None` -> `Clear`.
fn field_edit_from_old<T>(old: Option<T>) -> FieldEdit<T> {
    match old {
        Some(value) => FieldEdit::Set { value },
        None => FieldEdit::Clear,
    }
}

/// Would re-homing `id` under `new_parent` create a cycle (new_parent is `id`
/// or a descendant of `id`)? Walks the parent chain of `new_parent`.
fn would_cycle(scene: &ObjectScene, id: &str, new_parent: &str) -> bool {
    if id == new_parent {
        return true;
    }
    let mut cursor = Some(new_parent.to_string());
    let mut guard = 0usize;
    while let Some(cur) = cursor {
        if cur == id {
            return true;
        }
        guard += 1;
        if guard > scene.objects.len() + 1 {
            return true; // malformed chain — treat as cycle, never loop forever
        }
        cursor = scene
            .objects
            .iter()
            .find(|o| o.id == cur)
            .and_then(|o| o.parent.clone());
    }
    false
}

/// Convenience used by the vertical slice + undo engine: apply each op in order,
/// collecting inverses (reversed) so the whole sequence can be undone as a unit.
pub fn apply_sequence(
    scene: &mut ObjectScene,
    ops: impl IntoIterator<Item = ObjectOp>,
) -> Result<Vec<ObjectOp>, ApplyError> {
    let mut inverses = Vec::new();
    for op in ops {
        inverses.push(apply_object_op(scene, op)?);
    }
    inverses.reverse();
    Ok(inverses)
}
