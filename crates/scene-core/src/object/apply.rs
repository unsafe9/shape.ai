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

use crate::fractional::generate_key_between;

use super::model::{
    Anchor, Comment, Fill, Geometry, Layout, Object, ObjectId, ObjectScene, Stroke, SubPath,
};
use super::op::{FieldEdit, ObjectOp};
use super::validate::{validate_geometry, ValidationError};

/// Map a [`ValidationError`] from `validate.rs` onto the apply path's
/// [`ApplyError`]. Geometry defects collapse to `BadGeometry`; structural ones
/// keep their dedicated variant.
fn apply_error_from_validation(e: ValidationError) -> ApplyError {
    match e {
        ValidationError::EmptyGeometry
        | ValidationError::DegenerateSubpath { .. }
        | ValidationError::AnchorNodeOutOfRange { .. }
        | ValidationError::CommentNodeOutOfRange { .. } => ApplyError::BadGeometry(e.to_string()),
        ValidationError::ParentCycle { id } => ApplyError::Cycle(id),
        ValidationError::MissingAnchorTarget { target, .. } => {
            ApplyError::MissingAnchorTarget(target)
        }
    }
}

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
            validate_geometry(&object.geometry).map_err(apply_error_from_validation)?;
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
            validate_geometry(&geometry).map_err(apply_error_from_validation)?;
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

        ObjectOp::Split { id, new_ids, contours } => apply_split(scene, id, new_ids, contours),

        ObjectOp::Merge { ids, into } => apply_merge(scene, ids, into),

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

// ---------------------------------------------------------------------------
// OB3.S3 — Split / Merge.
//
// Both rebuild geometry by re-bucketing subpaths across objects, then re-home
// the peer anchors that addressed the touched objects (anchor `node_index` is a
// flat index across all subpaths in declaration order, so any reshuffle of
// subpaths must re-map it). Each returns a **faithful** inverse built from a
// captured pre-op snapshot: rather than a clever structural reverse, the inverse
// is a `Batch` that deletes whatever the op produced and re-inserts the exact
// originals (objects + their peer anchors), so undo restores byte-identical
// state. This trades a few extra ops for an inverse that is obviously correct
// and test-provable (Split↔Merge round-trip).
// ---------------------------------------------------------------------------

/// Per-subpath node-index offsets for a geometry: `offsets[i]` is the flat index
/// of subpath `i`'s first node; the final entry is the total node count. So
/// subpath `i` owns flat indices `offsets[i]..offsets[i + 1]`.
fn subpath_offsets(subpaths: &[SubPath]) -> Vec<i32> {
    let mut offsets = Vec::with_capacity(subpaths.len() + 1);
    let mut acc: i32 = 0;
    offsets.push(0);
    for sp in subpaths {
        acc = acc.saturating_add(i32::try_from(sp.nodes.len()).unwrap_or(i32::MAX));
        offsets.push(acc);
    }
    offsets
}

/// Snapshot of every peer anchor (across the whole scene) that points at
/// `target_id`, captured *before* a structural edit so the inverse can restore
/// the originals verbatim. Keyed by the peer object id, holding its full anchor
/// array (not just the matching anchors) so a single `SetAnchor` restores it.
fn capture_peer_anchors(scene: &ObjectScene, target_ids: &[&str]) -> Vec<(ObjectId, Vec<Anchor>)> {
    let mut out = Vec::new();
    for peer in &scene.objects {
        if peer
            .anchors
            .iter()
            .any(|a| target_ids.contains(&a.target.as_str()))
        {
            out.push((peer.id.clone(), peer.anchors.clone()));
        }
    }
    out
}

/// Implement `Split`: peel the listed contours off `id` into one new object each.
fn apply_split(
    scene: &mut ObjectScene,
    id: ObjectId,
    new_ids: Vec<ObjectId>,
    contours: Vec<i32>,
) -> Result<ObjectOp, ApplyError> {
    let idx = index_of(scene, &id)?;

    // Resolve which contour indices to peel (empty => all), validated + sorted.
    let n_sub = scene.objects[idx].geometry.subpaths.len();
    let peel: Vec<usize> = if contours.is_empty() {
        (0..n_sub).collect()
    } else {
        let mut seen = Vec::with_capacity(contours.len());
        for c in &contours {
            let ci = usize::try_from(*c)
                .ok()
                .filter(|ci| *ci < n_sub)
                .ok_or_else(|| ApplyError::BadGeometry(format!("split: contour index {c} out of range")))?;
            if seen.contains(&ci) {
                return Err(ApplyError::BadGeometry(format!("split: duplicate contour index {c}")));
            }
            seen.push(ci);
        }
        seen
    };

    if peel.is_empty() {
        return Err(ApplyError::BadGeometry("split: no contours to peel".to_string()));
    }
    if new_ids.len() != peel.len() {
        return Err(ApplyError::BadGeometry(format!(
            "split: {} new ids for {} contours",
            new_ids.len(),
            peel.len()
        )));
    }
    for nid in &new_ids {
        if *nid == id || scene.objects.iter().any(|o| o.id == *nid) {
            return Err(ApplyError::DuplicateId(nid.clone()));
        }
    }

    // Capture the faithful-inverse inputs before any mutation.
    let original = scene.objects[idx].clone();
    let peer_before = capture_peer_anchors(scene, &[id.as_str()]);

    let offsets = subpath_offsets(&original.geometry.subpaths);
    // Flat index of each contour's first node + which new object (if any) it maps
    // to. `dest[c]` = Some(new index k) if contour c is peeled, else None.
    let mut dest: Vec<Option<usize>> = vec![None; n_sub];
    for (k, c) in peel.iter().enumerate() {
        dest[*c] = Some(k);
    }

    // The remaining contours stay on the source, in original order. Build the
    // source's new flat-offset map so kept-contour anchors can be re-indexed.
    let kept: Vec<usize> = (0..n_sub).filter(|c| dest[*c].is_none()).collect();
    let mut kept_new_base: Vec<i32> = vec![0; n_sub];
    {
        let mut base: i32 = 0;
        for c in &kept {
            kept_new_base[*c] = base;
            base = base.saturating_add(
                i32::try_from(original.geometry.subpaths[*c].nodes.len()).unwrap_or(i32::MAX),
            );
        }
    }

    // Allocate fractional order keys for the new objects, strictly after the
    // source's order (append-style; deterministic). `a` ratchets forward so the
    // keys stay distinct and ordered even with no successor bound.
    let mut order_keys = Vec::with_capacity(peel.len());
    let mut prev = original.order.clone();
    for _ in 0..peel.len() {
        let key = generate_key_between(Some(&prev), None)
            .map_err(|e| ApplyError::BadGeometry(format!("split: order key: {e}")))?;
        prev = key.clone();
        order_keys.push(key);
    }

    // Materialize the new objects (one per peeled contour, original peel order).
    let mut produced: Vec<ObjectId> = Vec::with_capacity(peel.len());
    let mut new_objects: Vec<Object> = Vec::with_capacity(peel.len());
    for (k, c) in peel.iter().enumerate() {
        let mut child = original.clone();
        child.id = new_ids[k].clone();
        child.order = order_keys[k].clone();
        child.geometry = Geometry::from_subpaths(
            vec![original.geometry.subpaths[*c].clone()],
            original.geometry.fill_rule,
        );
        // The contour-peeled child carries no anchors of its own — anchors are an
        // edge property re-homed from peers below, never inherited from source.
        child.anchors = Vec::new();
        child.comments = Vec::new();
        produced.push(child.id.clone());
        new_objects.push(child);
    }

    // Re-home / re-index peer anchors that addressed the source.
    for peer in &mut scene.objects {
        if peer.id == id {
            continue;
        }
        for anchor in &mut peer.anchors {
            if anchor.target != id {
                continue;
            }
            let flat = anchor.node_index;
            // Find the source contour that owns this flat node index.
            let owner = (0..n_sub).find(|c| flat >= offsets[*c] && flat < offsets[*c + 1]);
            let Some(owner) = owner else { continue };
            let local = flat - offsets[owner];
            match dest[owner] {
                Some(k) => {
                    // Owner contour peeled => anchor follows it to the new object,
                    // re-indexed relative to that object's single subpath.
                    anchor.target = produced[k].clone();
                    anchor.node_index = local;
                }
                None => {
                    // Owner contour stays on the source => re-index against the
                    // source's compacted subpath order.
                    anchor.node_index = kept_new_base[owner] + local;
                }
            }
        }
    }

    // Update geometry of the source: keep remaining contours, or delete it.
    let source_survives = !kept.is_empty();
    if source_survives {
        let kept_subpaths: Vec<SubPath> = kept
            .iter()
            .map(|c| original.geometry.subpaths[*c].clone())
            .collect();
        scene.objects[idx].geometry =
            Geometry::from_subpaths(kept_subpaths, original.geometry.fill_rule);
    } else {
        scene.objects.remove(idx);
    }

    // Insert the new objects (append; canonical paint order is `order`).
    scene.objects.extend(new_objects);

    // Build the faithful inverse: delete everything produced (+ source if it
    // survived), re-insert the exact pre-split source, and restore peer anchors.
    let mut inverse_ops: Vec<ObjectOp> = Vec::new();
    for pid in &produced {
        inverse_ops.push(ObjectOp::Delete { id: pid.clone() });
    }
    if source_survives {
        inverse_ops.push(ObjectOp::Delete { id: id.clone() });
    }
    inverse_ops.push(ObjectOp::InsertObject { object: original });
    for (peer_id, anchors) in peer_before {
        inverse_ops.push(ObjectOp::SetAnchor { id: peer_id, anchors });
    }
    Ok(ObjectOp::Batch { ops: inverse_ops })
}

/// Implement `Merge`: concatenate the listed siblings' subpaths into the
/// surviving object (`into`, else the first id); delete the others.
fn apply_merge(
    scene: &mut ObjectScene,
    ids: Vec<ObjectId>,
    into: Option<ObjectId>,
) -> Result<ObjectOp, ApplyError> {
    if ids.is_empty() {
        return Err(ApplyError::BadGeometry("merge: no ids".to_string()));
    }
    let survivor_id = into.clone().unwrap_or_else(|| ids[0].clone());
    if !ids.contains(&survivor_id) {
        return Err(ApplyError::BadGeometry(format!(
            "merge: into `{survivor_id}` not among merged ids"
        )));
    }
    // Validate every id exists + dedupe-check before mutating.
    {
        let mut seen: Vec<&str> = Vec::with_capacity(ids.len());
        for mid in &ids {
            index_of(scene, mid)?;
            if seen.contains(&mid.as_str()) {
                return Err(ApplyError::DuplicateId(mid.clone()));
            }
            seen.push(mid.as_str());
        }
    }

    // The merge order is: survivor's own subpaths first, then the others in the
    // `ids` order given (excluding the survivor). This fixes the flat node-index
    // base each merged object lands at, which the anchor re-home below relies on.
    let mut merge_order: Vec<ObjectId> = vec![survivor_id.clone()];
    for mid in &ids {
        if *mid != survivor_id {
            merge_order.push(mid.clone());
        }
    }

    // Capture faithful-inverse inputs: every merged object verbatim (in `ids`
    // order, so Split can rebuild them) + every peer anchor that addressed any of
    // them.
    let originals: Vec<Object> = ids
        .iter()
        .map(|mid| scene.get(mid).expect("checked above").clone())
        .collect();
    let merged_id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    let peer_before = capture_peer_anchors(scene, &merged_id_refs);

    // Compute each merged object's flat-index base in the combined geometry +
    // assemble the concatenated subpaths.
    let mut base_of: std::collections::HashMap<ObjectId, i32> = std::collections::HashMap::new();
    let mut combined: Vec<SubPath> = Vec::new();
    let mut acc: i32 = 0;
    let survivor_fill_rule = scene
        .get(&survivor_id)
        .expect("survivor exists")
        .geometry
        .fill_rule;
    for mid in &merge_order {
        base_of.insert(mid.clone(), acc);
        let obj = scene.get(mid).expect("merged object exists");
        for sp in &obj.geometry.subpaths {
            acc = acc.saturating_add(i32::try_from(sp.nodes.len()).unwrap_or(i32::MAX));
            combined.push(sp.clone());
        }
    }

    // Remove the non-survivor merged objects.
    scene
        .objects
        .retain(|o| o.id == survivor_id || !ids.contains(&o.id));

    // Write the combined geometry onto the survivor (keeping its style/transform).
    let survivor_idx = index_of(scene, &survivor_id)?;
    scene.objects[survivor_idx].geometry = Geometry::from_subpaths(combined, survivor_fill_rule);

    // Re-home peer anchors: any anchor that addressed a merged object now points
    // at the survivor, with its flat node_index shifted by that object's base.
    for peer in &mut scene.objects {
        for anchor in &mut peer.anchors {
            if let Some(base) = base_of.get(&anchor.target) {
                anchor.node_index = anchor.node_index.saturating_add(*base);
                anchor.target = survivor_id.clone();
            }
        }
    }

    // Faithful inverse: delete the merged result, re-insert every original
    // (exact id/order/geometry/anchors), restore peer anchors.
    let mut inverse_ops: Vec<ObjectOp> = vec![ObjectOp::Delete { id: survivor_id.clone() }];
    for object in originals {
        inverse_ops.push(ObjectOp::InsertObject { object });
    }
    for (peer_id, anchors) in peer_before {
        inverse_ops.push(ObjectOp::SetAnchor { id: peer_id, anchors });
    }
    Ok(ObjectOp::Batch { ops: inverse_ops })
}

// ---------------------------------------------------------------------------
// OB3.S1 — optional per-property server-authoritative LWW layer.
//
// `apply_object_op` stays the seq-less direct-apply (single actor + tests).
// `apply_object_op_lww` wraps it with a [`PropertyStore`] gate: each op names
// the properties it touches; for every property whose stored winning `seq` is
// already `>=` the incoming `seq`, the write is *stale* and the whole op is
// skipped (Figma server-authoritative LWW — server arrival seq is the
// authority). A surviving op applies through the exact same path + records the
// new seq for each touched property.
// ---------------------------------------------------------------------------

/// The properties an op writes, as `(object_id, property_name)` pairs. These key
/// the [`PropertyStore`]; the names mirror the camelCase field/op vocabulary so
/// the store stays human-legible. Structural ops that span objects
/// (`Split`/`Merge`/`Batch`/`InsertObject`/`Delete`) return an empty slice and
/// are treated as always-apply (they are not single-property LWW writes — the
/// server orders them by seq globally, not per property).
fn touched_properties(op: &ObjectOp) -> Vec<(ObjectId, &'static str)> {
    match op {
        ObjectOp::EditGeometry { id, .. } => vec![(id.clone(), "geometry")],
        ObjectOp::SetTransform { id, .. } => vec![(id.clone(), "transform")],
        ObjectOp::SetStyle { id, fill, stroke } => {
            let mut v = Vec::new();
            if fill.is_some() {
                v.push((id.clone(), "fill"));
            }
            if stroke.is_some() {
                v.push((id.clone(), "stroke"));
            }
            v
        }
        ObjectOp::SetText { id, .. } => vec![(id.clone(), "text")],
        ObjectOp::SetAnchor { id, .. } => vec![(id.clone(), "anchors")],
        ObjectOp::SetLayout { id, .. } => vec![(id.clone(), "layout")],
        ObjectOp::SetClip { id, .. } => vec![(id.clone(), "clip")],
        ObjectOp::AddComment { id, .. } | ObjectOp::SetComments { id, .. } => {
            vec![(id.clone(), "comments")]
        }
        ObjectOp::SetTags { id, .. } => vec![(id.clone(), "tags")],
        ObjectOp::Reparent { id, .. } => vec![(id.clone(), "parent"), (id.clone(), "order")],
        ObjectOp::Reorder { id, .. } => vec![(id.clone(), "order")],
        ObjectOp::InsertObject { .. }
        | ObjectOp::Delete { .. }
        | ObjectOp::Split { .. }
        | ObjectOp::Merge { .. }
        | ObjectOp::Batch { .. } => Vec::new(),
    }
}

/// Apply `op` under per-property server-authoritative LWW (OB3.S1).
///
/// `seq` is the server's monotonic arrival sequence for this op (the authority
/// token, identical role to `lww.rs`). For every property the op touches: if the
/// [`PropertyStore`] already holds a winner whose `seq` is `>=` the incoming
/// `seq`, the write is **stale** and the op is skipped entirely (the scene is
/// left untouched and the returned inverse is a no-op `Batch`). Otherwise the op
/// applies through [`apply_object_op`] and each touched property records the new
/// `seq`. Structural ops (insert/delete/split/merge/batch) carry no
/// single-property key and always apply.
///
/// The seq-less [`apply_object_op`] remains the single-actor / test path.
pub fn apply_object_op_lww(
    scene: &mut ObjectScene,
    store: &mut crate::lww::PropertyStore,
    op: ObjectOp,
    seq: u64,
) -> Result<ObjectOp, ApplyError> {
    let seq_i64 = i64::try_from(seq).unwrap_or(i64::MAX);
    let props = touched_properties(&op);

    // Stale check: if ANY touched property already has a winner at >= this seq,
    // the op lost the race for that property. Per-property LWW with a
    // multi-property op (only `Reparent`/`SetStyle` touch >1) treats the op
    // atomically — it applies as a unit or not at all — so a single stale
    // property vetoes the whole op. Equal seq is stale (idempotent re-apply).
    for (object_id, property) in &props {
        if let Some(entry) = store.get(object_id, *property) {
            if seq_i64 <= entry.seq {
                return Ok(ObjectOp::Batch { ops: Vec::new() });
            }
        }
    }

    // The op won every property it touches; apply it, then record the new seqs.
    let inverse = apply_object_op(scene, op)?;
    for (object_id, property) in &props {
        // Value payload is informational here (the scene holds the authoritative
        // value); record a null marker carrying the winning seq so future writes
        // compare against it.
        store.apply(object_id, *property, serde_json::Value::Null, seq_i64);
    }
    Ok(inverse)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lww::PropertyStore;
    use crate::object::model::{FillRule, LocalPoint, PathNode, Transform3x3};

    /// A closed rect contour (4 corners) in quantized units.
    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> SubPath {
        SubPath {
            closed: true,
            nodes: vec![
                PathNode::corner(x0, y0),
                PathNode::corner(x1, y0),
                PathNode::corner(x1, y1),
                PathNode::corner(x0, y1),
            ],
        }
    }

    /// An object with the given multi-subpath geometry.
    fn multi_obj(id: &str, order: &str, subpaths: Vec<SubPath>) -> Object {
        Object::new(id, order, Geometry::from_subpaths(subpaths, FillRule::EvenOdd))
    }

    fn scene_of(objects: Vec<Object>) -> ObjectScene {
        ObjectScene { objects, ..Default::default() }
    }

    // ---- Split -------------------------------------------------------------

    #[test]
    fn split_peels_all_contours_into_new_objects() {
        // A two-contour object split into both contours -> 2 new objects, source
        // deleted (no contours remain).
        let src = multi_obj("src", "a5", vec![rect(0, 0, 80, 40), rect(100, 0, 180, 40)]);
        let mut scene = scene_of(vec![src]);

        let inverse = apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["c0".into(), "c1".into()], contours: vec![] },
        )
        .expect("split");

        assert!(scene.get("src").is_none(), "source deleted when no contour remains");
        assert!(scene.get("c0").is_some());
        assert!(scene.get("c1").is_some());
        // Each new object carries exactly one contour.
        assert_eq!(scene.get("c0").unwrap().geometry.subpaths.len(), 1);
        assert_eq!(scene.get("c1").unwrap().geometry.subpaths.len(), 1);
        // New keys sort strictly after the source order.
        assert!(scene.get("c0").unwrap().order.as_str() > "a5");
        assert!(scene.get("c1").unwrap().order.as_str() > scene.get("c0").unwrap().order.as_str());

        // The inverse is a Batch.
        assert!(matches!(inverse, ObjectOp::Batch { .. }));
    }

    #[test]
    fn split_keeps_remaining_contours_on_source() {
        // Peel only contour 1 of 3 -> source keeps contours 0 and 2.
        let src = multi_obj(
            "src",
            "a5",
            vec![rect(0, 0, 10, 10), rect(20, 0, 30, 10), rect(40, 0, 50, 10)],
        );
        let mut scene = scene_of(vec![src]);

        apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["c".into()], contours: vec![1] },
        )
        .expect("split");

        assert_eq!(scene.get("src").unwrap().geometry.subpaths.len(), 2, "two contours remain");
        assert_eq!(scene.get("c").unwrap().geometry.subpaths.len(), 1);
        // The kept subpaths are the original 0 and 2 (first node x = 0 then 40).
        let kept = &scene.get("src").unwrap().geometry.subpaths;
        assert_eq!(kept[0].nodes[0].x, 0);
        assert_eq!(kept[1].nodes[0].x, 40);
    }

    #[test]
    fn split_then_inverse_restores_original() {
        // Faithful inverse: split, then apply the returned inverse -> byte-equal
        // pre-split scene (geometry, order, ids).
        let src = multi_obj("src", "a5", vec![rect(0, 0, 80, 40), rect(100, 0, 180, 40)]);
        let mut scene = scene_of(vec![src]);
        let before = scene.clone();

        let inverse = apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["c0".into(), "c1".into()], contours: vec![] },
        )
        .expect("split");
        apply_object_op(&mut scene, inverse).expect("apply inverse");

        // The single source object is back with its two-contour geometry; the
        // produced children are gone.
        assert!(scene.get("c0").is_none());
        assert!(scene.get("c1").is_none());
        let restored = scene.get("src").expect("source restored");
        let original = before.get("src").unwrap();
        assert_eq!(restored.geometry, original.geometry);
        assert_eq!(restored.order, original.order);
        assert_eq!(scene.objects.len(), before.objects.len());
    }

    #[test]
    fn split_rehomes_and_reindexes_peer_anchors() {
        // src has two 4-node contours: flat indices 0..4 (contour 0), 4..8
        // (contour 1). An edge anchors node-index 0 (contour 0) and node-index 5
        // (contour 1, local 1). After splitting both contours, anchor 0 follows
        // c0 (stays index 0); anchor 5 follows c1 (re-indexed to local 1).
        let src = multi_obj("src", "a5", vec![rect(0, 0, 80, 40), rect(100, 0, 180, 40)]);
        let mut edge = multi_obj("edge", "a6", vec![rect(0, 0, 10, 10)]);
        edge.anchors = vec![
            Anchor { node_index: 0, target: "src".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 5, target: "src".into(), at: LocalPoint { x: 100, y: 0 } },
        ];
        let mut scene = scene_of(vec![src, edge]);

        apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["c0".into(), "c1".into()], contours: vec![] },
        )
        .expect("split");

        let anchors = &scene.get("edge").unwrap().anchors;
        assert_eq!(anchors[0].target, "c0");
        assert_eq!(anchors[0].node_index, 0);
        assert_eq!(anchors[1].target, "c1");
        assert_eq!(anchors[1].node_index, 1, "flat 5 -> contour 1 local 1");
    }

    #[test]
    fn split_with_peer_anchors_round_trips() {
        let src = multi_obj("src", "a5", vec![rect(0, 0, 80, 40), rect(100, 0, 180, 40)]);
        let mut edge = multi_obj("edge", "a6", vec![rect(0, 0, 10, 10)]);
        edge.anchors = vec![
            Anchor { node_index: 5, target: "src".into(), at: LocalPoint { x: 100, y: 0 } },
        ];
        let mut scene = scene_of(vec![src, edge]);
        let before = scene.clone();

        let inverse = apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["c0".into(), "c1".into()], contours: vec![] },
        )
        .expect("split");
        apply_object_op(&mut scene, inverse).expect("apply inverse");

        let restored = scene.get("edge").unwrap();
        assert_eq!(restored.anchors, before.get("edge").unwrap().anchors);
        assert_eq!(scene.get("src").unwrap().geometry, before.get("src").unwrap().geometry);
    }

    #[test]
    fn split_rejects_id_count_mismatch() {
        let src = multi_obj("src", "a5", vec![rect(0, 0, 80, 40), rect(100, 0, 180, 40)]);
        let mut scene = scene_of(vec![src]);
        let err = apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["only-one".into()], contours: vec![] },
        )
        .unwrap_err();
        assert!(matches!(err, ApplyError::BadGeometry(_)));
        // Scene untouched on rejection.
        assert_eq!(scene.objects.len(), 1);
        assert!(scene.get("only-one").is_none());
    }

    #[test]
    fn split_rejects_duplicate_new_id() {
        let src = multi_obj("src", "a5", vec![rect(0, 0, 80, 40), rect(100, 0, 180, 40)]);
        let mut scene = scene_of(vec![src]);
        let err = apply_object_op(
            &mut scene,
            ObjectOp::Split { id: "src".into(), new_ids: vec!["src".into(), "c1".into()], contours: vec![] },
        )
        .unwrap_err();
        assert!(matches!(err, ApplyError::DuplicateId(_)));
    }

    // ---- Merge -------------------------------------------------------------

    #[test]
    fn merge_concatenates_subpaths_into_survivor() {
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let b = multi_obj("b", "a1", vec![rect(20, 0, 30, 10)]);
        let c = multi_obj("c", "a2", vec![rect(40, 0, 50, 10)]);
        let mut scene = scene_of(vec![a, b, c]);

        apply_object_op(
            &mut scene,
            ObjectOp::Merge { ids: vec!["a".into(), "b".into(), "c".into()], into: Some("a".into()) },
        )
        .expect("merge");

        assert!(scene.get("b").is_none());
        assert!(scene.get("c").is_none());
        let merged = scene.get("a").unwrap();
        assert_eq!(merged.geometry.subpaths.len(), 3, "all three contours concatenated");
        assert_eq!(merged.order, "a0", "survivor keeps its own order");
    }

    #[test]
    fn merge_defaults_survivor_to_first_id() {
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let b = multi_obj("b", "a1", vec![rect(20, 0, 30, 10)]);
        let mut scene = scene_of(vec![a, b]);

        apply_object_op(&mut scene, ObjectOp::Merge { ids: vec!["a".into(), "b".into()], into: None })
            .expect("merge");

        assert!(scene.get("a").is_some());
        assert!(scene.get("b").is_none());
        assert_eq!(scene.get("a").unwrap().geometry.subpaths.len(), 2);
    }

    #[test]
    fn merge_then_inverse_restores_originals() {
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let b = multi_obj("b", "a1", vec![rect(20, 0, 30, 10)]);
        let mut scene = scene_of(vec![a, b]);
        let before = scene.clone();

        let inverse = apply_object_op(
            &mut scene,
            ObjectOp::Merge { ids: vec!["a".into(), "b".into()], into: Some("a".into()) },
        )
        .expect("merge");
        apply_object_op(&mut scene, inverse).expect("apply inverse");

        // Both originals are back with their exact geometry + order.
        assert_eq!(scene.get("a").unwrap().geometry, before.get("a").unwrap().geometry);
        assert_eq!(scene.get("b").unwrap().geometry, before.get("b").unwrap().geometry);
        assert_eq!(scene.get("a").unwrap().order, "a0");
        assert_eq!(scene.get("b").unwrap().order, "a1");
        assert_eq!(scene.objects.len(), before.objects.len());
    }

    #[test]
    fn merge_rehomes_peer_anchors_to_survivor() {
        // edge anchors b's node 0 (b is single 4-node rect). After merge into a
        // (a has 4 nodes), b's nodes start at flat base 4, so anchor -> a node 4.
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let b = multi_obj("b", "a1", vec![rect(20, 0, 30, 10)]);
        let mut edge = multi_obj("edge", "a2", vec![rect(0, 0, 5, 5)]);
        edge.anchors = vec![Anchor { node_index: 0, target: "b".into(), at: LocalPoint { x: 20, y: 0 } }];
        let mut scene = scene_of(vec![a, b, edge]);

        apply_object_op(
            &mut scene,
            ObjectOp::Merge { ids: vec!["a".into(), "b".into()], into: Some("a".into()) },
        )
        .expect("merge");

        let anchor = &scene.get("edge").unwrap().anchors[0];
        assert_eq!(anchor.target, "a");
        assert_eq!(anchor.node_index, 4, "b's node 0 lands at flat base 4 in the merged geometry");
    }

    #[test]
    fn merge_with_peer_anchors_round_trips() {
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let b = multi_obj("b", "a1", vec![rect(20, 0, 30, 10)]);
        let mut edge = multi_obj("edge", "a2", vec![rect(0, 0, 5, 5)]);
        edge.anchors = vec![Anchor { node_index: 0, target: "b".into(), at: LocalPoint { x: 20, y: 0 } }];
        let mut scene = scene_of(vec![a, b, edge]);
        let before = scene.clone();

        let inverse = apply_object_op(
            &mut scene,
            ObjectOp::Merge { ids: vec!["a".into(), "b".into()], into: Some("a".into()) },
        )
        .expect("merge");
        apply_object_op(&mut scene, inverse).expect("apply inverse");

        assert_eq!(scene.get("edge").unwrap().anchors, before.get("edge").unwrap().anchors);
    }

    #[test]
    fn merge_rejects_into_not_in_ids() {
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let b = multi_obj("b", "a1", vec![rect(20, 0, 30, 10)]);
        let mut scene = scene_of(vec![a, b]);
        let err = apply_object_op(
            &mut scene,
            ObjectOp::Merge { ids: vec!["a".into(), "b".into()], into: Some("z".into()) },
        )
        .unwrap_err();
        assert!(matches!(err, ApplyError::BadGeometry(_)));
        assert_eq!(scene.objects.len(), 2);
    }

    #[test]
    fn merge_rejects_missing_id() {
        let a = multi_obj("a", "a0", vec![rect(0, 0, 10, 10)]);
        let mut scene = scene_of(vec![a]);
        let err = apply_object_op(
            &mut scene,
            ObjectOp::Merge { ids: vec!["a".into(), "ghost".into()], into: Some("a".into()) },
        )
        .unwrap_err();
        assert!(matches!(err, ApplyError::NotFound(_)));
        assert_eq!(scene.objects.len(), 1);
    }

    // ---- Validation gates --------------------------------------------------

    #[test]
    fn insert_rejects_degenerate_geometry() {
        let bad = multi_obj(
            "x",
            "a0",
            vec![SubPath { closed: true, nodes: vec![PathNode::corner(0, 0), PathNode::corner(1, 0)] }],
        );
        let mut scene = ObjectScene::default();
        let err = apply_object_op(&mut scene, ObjectOp::InsertObject { object: bad }).unwrap_err();
        assert!(matches!(err, ApplyError::BadGeometry(_)));
        assert!(scene.objects.is_empty());
    }

    #[test]
    fn edit_geometry_rejects_empty() {
        let mut scene = scene_of(vec![multi_obj("x", "a0", vec![rect(0, 0, 10, 10)])]);
        let err = apply_object_op(
            &mut scene,
            ObjectOp::EditGeometry { id: "x".into(), geometry: Geometry::default() },
        )
        .unwrap_err();
        assert!(matches!(err, ApplyError::BadGeometry(_)));
        // Original geometry untouched.
        assert_eq!(scene.get("x").unwrap().geometry.subpaths.len(), 1);
    }

    // ---- LWW staleness -----------------------------------------------------

    #[test]
    fn lww_ignores_stale_lower_seq() {
        let mut scene = scene_of(vec![multi_obj("o", "a0", vec![rect(0, 0, 10, 10)])]);
        let mut store = PropertyStore::new();

        // seq 5 sets a transform; it wins (first writer).
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(10.0, 0.0) },
            5,
        )
        .expect("seq 5");
        assert_eq!(scene.get("o").unwrap().transform, Transform3x3::translate(10.0, 0.0));

        // seq 3 (older) loses: scene unchanged, inverse is a no-op batch.
        let inv = apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(99.0, 0.0) },
            3,
        )
        .expect("stale op accepted but skipped");
        assert_eq!(inv, ObjectOp::Batch { ops: Vec::new() }, "stale op yields a no-op inverse");
        assert_eq!(
            scene.get("o").unwrap().transform,
            Transform3x3::translate(10.0, 0.0),
            "stale write must not mutate"
        );
    }

    #[test]
    fn lww_equal_seq_is_stale() {
        let mut scene = scene_of(vec![multi_obj("o", "a0", vec![rect(0, 0, 10, 10)])]);
        let mut store = PropertyStore::new();
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(10.0, 0.0) },
            7,
        )
        .expect("seq 7");
        // Re-applying at the same seq is idempotent (does not flip).
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(50.0, 0.0) },
            7,
        )
        .expect("equal seq");
        assert_eq!(scene.get("o").unwrap().transform, Transform3x3::translate(10.0, 0.0));
    }

    #[test]
    fn lww_newer_seq_wins() {
        let mut scene = scene_of(vec![multi_obj("o", "a0", vec![rect(0, 0, 10, 10)])]);
        let mut store = PropertyStore::new();
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(10.0, 0.0) },
            5,
        )
        .expect("seq 5");
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(20.0, 0.0) },
            8,
        )
        .expect("seq 8");
        assert_eq!(scene.get("o").unwrap().transform, Transform3x3::translate(20.0, 0.0));
    }

    #[test]
    fn lww_distinct_properties_are_independent() {
        // A stale write to one property does not block a fresh write to another.
        let mut scene = scene_of(vec![multi_obj("o", "a0", vec![rect(0, 0, 10, 10)])]);
        let mut store = PropertyStore::new();
        // transform wins at seq 10.
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTransform { id: "o".into(), transform: Transform3x3::translate(10.0, 0.0) },
            10,
        )
        .expect("transform seq 10");
        // tags is a different property: a seq-3 write still wins (no prior tags seq).
        apply_object_op_lww(
            &mut scene,
            &mut store,
            ObjectOp::SetTags { id: "o".into(), tags: vec!["t1".into()] },
            3,
        )
        .expect("tags seq 3");
        assert_eq!(scene.get("o").unwrap().tags, vec!["t1".to_string()]);
        assert_eq!(scene.get("o").unwrap().transform, Transform3x3::translate(10.0, 0.0));
    }
}
