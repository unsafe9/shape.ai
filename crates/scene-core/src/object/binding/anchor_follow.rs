//! Tier-1/#14 — commit-time anchor move-together (the canonical SetTransform
//! reproject). The single source of truth for "what-you-drag-is-what-you-commit":
//! when a target object is moved by a `set-transform` op, every object anchored
//! onto that target reprojects its bound geometry node through the target's NEW
//! transform, so the anchored endpoint tracks the target without drift.
//!
//! Ported from the shell `anchorCreate.ts` (`anchorFollowOps` /
//! `reprojectAnchoredGeometry` / `synthesizeCreateAnchors`) so the canvas logic
//! lives in the Rust core; the shell now only batches the returned ops.
//!
//! The TRANSFORM reproject here is the canonical mirror of the renderer-core LIVE
//! preview (`transform_bindings::reproject_node_local_px` + `rewrite_geometry_node`):
//! both de-quantize the target-local anchor point by [`UNITS_PER_PX`], carry it to
//! world through the target's transform, map back into the follower's OWN local
//! pixel space (follower inverse), then quantize with the SAME `round()`. A
//! cross-core equivalence test (`reproject_matches_cross_core_vector`) pins one
//! concrete numeric vector in BOTH cores so either copy drifting fails its own test.
//!
//! Pure (no time/rng/IO/GPU), pointer-width-agnostic. Region-based anchor
//! resolution ([`crate::object::anchors`]) is a SEPARATE axis (geometry-edit follow); this
//! module is the transform-based move-together only and must not be folded into it.

use crate::object::deform::{deform_open_path, is_open_class_d};
use crate::object::model::{
    Anchor, Geometry, LocalPoint, Object, ObjectScene, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::op::ObjectOp;
use crate::object::region::OutlineDeriver;

/// Quantized units per logical pixel. Matches renderer-core
/// `transform_bindings::UNITS_PER_PX` and the shell `GEOMETRY_QUANTUM_PER_PX`
/// (Q=8); the reproject quantization MUST stay byte-equivalent across all three.
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

/// A 2x3 affine (the top two rows of a row-major 3x3 with `g=h=0,i=1`), used for
/// the tiny invert/apply the reproject needs. A scene-core-local copy (no shared
/// crate); equivalent to the shell `applyTransform` / `invertAffine`. Shared
/// (`pub(crate)`) with the open-class endpoint routing in [`crate::object::deform`].
pub(crate) type Affine = [[f64; 3]; 2];

/// The affine rows of a transform (absent => identity), dropping the projective
/// bottom row (anchors are affine-only, matching the shell `Transform3x3` shape).
pub(crate) fn affine_of(t: &Transform3x3) -> Affine {
    [
        [t.m[0][0], t.m[0][1], t.m[0][2]],
        [t.m[1][0], t.m[1][1], t.m[1][2]],
    ]
}

/// Apply a row-major affine to a point. Mirrors the shell `applyTransform`.
pub(crate) fn apply_affine(a: &Affine, x: f64, y: f64) -> (f64, f64) {
    (
        a[0][0] * x + a[0][1] * y + a[0][2],
        a[1][0] * x + a[1][1] * y + a[1][2],
    )
}

/// Invert a row-major affine (`g=h=0,i=1`). Returns identity when singular,
/// matching the shell `invertAffine` (a singular follower then no-ops the node).
pub(crate) fn invert_affine(a: &Affine) -> Affine {
    let (a00, a01, a02) = (a[0][0], a[0][1], a[0][2]);
    let (a10, a11, a12) = (a[1][0], a[1][1], a[1][2]);
    let det = a00 * a11 - a01 * a10;
    if det == 0.0 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    }
    let ia = a11 / det;
    let ib = -a01 / det;
    let id = -a10 / det;
    let ie = a00 / det;
    [
        [ia, ib, -(ia * a02 + ib * a12)],
        [id, ie, -(id * a02 + ie * a12)],
    ]
}

/// The signed-decimal number spans of a path-string, matching the shell regex
/// `-?\d+(?:\.\d+)?` and renderer-core `NumberSpans`. Each `(start, end)` is a
/// half-open byte range; non-number characters between spans are skipped.
fn number_spans(d: &str) -> Vec<(usize, usize)> {
    let bytes = d.as_bytes();
    let n = bytes.len();
    let mut spans = Vec::new();
    let mut pos = 0usize;
    while pos < n {
        let b = bytes[pos];
        if b != b'-' && !b.is_ascii_digit() {
            pos += 1;
            continue;
        }
        let start = pos;
        if bytes[pos] == b'-' {
            pos += 1;
        }
        while pos < n && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if pos < n && bytes[pos] == b'.' {
            pos += 1;
            while pos < n && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
        }
        spans.push((start, pos));
    }
    spans
}

/// The object-local node points parsed from a path-string's M/L/C coords, in pair
/// order (M/L/C all contribute pairs). Mirrors the shell `localNodes`. Anchor
/// `node_index` addresses THIS pair space (so 0 and `len()-1` are the open-path
/// endpoints — control points are never first or last in the codec's output).
/// `pub` because the renderer-core consumes it via the rlib as the pair-space
/// single source (endpoint handles + live chord deform, anchor-semantics v3 §2b).
pub fn local_nodes(d: &str) -> Vec<(f64, f64)> {
    let spans = number_spans(d);
    let mut out = Vec::with_capacity(spans.len() / 2);
    let mut i = 0;
    while i + 1 < spans.len() {
        let x: f64 = d[spans[i].0..spans[i].1].parse().unwrap_or(0.0);
        let y: f64 = d[spans[i + 1].0..spans[i + 1].1].parse().unwrap_or(0.0);
        out.push((x, y));
        i += 2;
    }
    out
}

/// The geometry node index of `object` closest to the world point `(wx, wy)`,
/// after mapping the world point into the object's local quantized space (so the
/// match is transform-invariant). Returns `None` when the geometry has no nodes.
/// Mirrors the shell `nodeIndexNearestWorld`.
fn node_index_nearest_world(object: &Object, wx: f64, wy: f64) -> Option<i32> {
    let nodes = local_nodes(&object.geometry.path_string);
    if nodes.is_empty() {
        return None;
    }
    let inv = invert_affine(&affine_of(&object.transform));
    let (lx, ly) = apply_affine(&inv, wx, wy);
    let lx = lx * UNITS_PER_PX;
    let ly = ly * UNITS_PER_PX;
    let mut best = 0i32;
    let mut best_d = f64::INFINITY;
    for (i, (nx, ny)) in nodes.iter().enumerate() {
        let dx = nx - lx;
        let dy = ny - ly;
        let dist = dx * dx + dy * dy;
        if dist < best_d {
            best_d = dist;
            best = i32::try_from(i).unwrap_or(i32::MAX);
        }
    }
    Some(best)
}

/// Rewrite the `node_index`-th coordinate PAIR of a path-string `d` to the
/// quantized object-local ints `(x, y)`, preserving every command token and every
/// other coordinate. Returns `None` when the pair is unaddressable OR the rewrite
/// is a no-op (the new coords already match). Mirrors the shell `setPathNode`
/// (returning `None` on no-op so the caller skips a pointless edit) and is
/// byte-equivalent to renderer-core `rewrite_geometry_node`.
fn set_path_node(d: &str, node_index: i32, x: i64, y: i64) -> Option<String> {
    let spans = number_spans(d);
    let mut out = String::with_capacity(d.len());
    let mut last = 0usize;
    let mut pair: i32 = 0;
    let mut numbers_seen = 0usize;
    let mut hit = false;
    let mut changed = false;
    for (start, end) in spans {
        out.push_str(&d[last..start]);
        let is_x = numbers_seen % 2 == 0;
        let at_target = pair == node_index;
        if !is_x {
            pair += 1;
        }
        numbers_seen += 1;
        if at_target {
            hit = true;
            let replacement = if is_x { x } else { y };
            let original = &d[start..end];
            let replacement_str = replacement.to_string();
            if original != replacement_str {
                changed = true;
            }
            out.push_str(&replacement_str);
        } else {
            out.push_str(&d[start..end]);
        }
        last = end;
    }
    out.push_str(&d[last..]);
    if hit && changed {
        Some(out)
    } else {
        None
    }
}

/// The canonical TRANSFORM reproject: the follower-local QUANTIZED node position
/// of `anchor`'s bound node when its `target` carries `target_transform` (the NEW
/// transform, post-move).
///
/// Formula (shared with renderer-core `reproject_node_local_px`, then quantized
/// like `rewrite_geometry_node`):
///   1. de-quantize the target-local anchor point: `(at / UNITS_PER_PX)`
///   2. world point: `target_transform * (at / UNITS_PER_PX)`
///   3. follower-local px: `follower_transform^-1 * world`
///   4. quantize: `(px * UNITS_PER_PX).round() as i64`
///
/// A pure target translation therefore shifts the bound node by EXACTLY the same
/// world delta (identity bases), the falsifiable move-together property.
fn reproject_node_local_quantized(
    follower_transform: &Transform3x3,
    target_transform: &Transform3x3,
    at: LocalPoint,
) -> (i64, i64) {
    let lx = f64::from(at.x) / UNITS_PER_PX;
    let ly = f64::from(at.y) / UNITS_PER_PX;
    let (wx, wy) = apply_affine(&affine_of(target_transform), lx, ly);
    let inv = invert_affine(&affine_of(follower_transform));
    let (fx, fy) = apply_affine(&inv, wx, wy);
    let qx = (fx * UNITS_PER_PX).round() as i64;
    let qy = (fy * UNITS_PER_PX).round() as i64;
    (qx, qy)
}

/// Reproject `follower`'s node bound by `anchor` through `target`'s NEW transform,
/// returning the updated geometry — or `None` when the rewrite is a no-op (the
/// node already sits where the target puts it) or the node is unaddressable.
/// Mirrors the shell `reprojectAnchoredGeometry`.
///
/// W3-G13: the commit path ([`anchor_follow_ops`]) now folds EVERY moved-target
/// anchor into one cumulative rewrite via the same two helpers; this single-anchor
/// wrapper survives only as the cross-core pin's entry point
/// (`reproject_matches_cross_core_vector`), hence test-only.
#[cfg(test)]
fn reproject_anchored_geometry(
    follower: &Object,
    anchor: &Anchor,
    target_transform: &Transform3x3,
) -> Option<Geometry> {
    let (qx, qy) = reproject_node_local_quantized(&follower.transform, target_transform, anchor.at);
    let d = set_path_node(&follower.geometry.path_string, anchor.node_index, qx, qy)?;
    Some(Geometry {
        path_string: d,
        fill_rule: follower.geometry.fill_rule,
        subpaths: Vec::new(),
    })
}

/// The shared LIVE reproject: rewrite the `node_index`-th node of `geometry_d` to
/// where `at` lands when `target_transform` is moved by `delta`, returning the new
/// path-string — or `None` on a no-op / unaddressable node / singular follower.
///
/// The renderer holds the target's BASE transform and the drag `delta` separately
/// (the same `(target_base, delta)` shape its instance-matrix preview writes), so
/// this composes `delta * target_transform` to form the target's NEW transform
/// before the reproject, then drives the SAME two helpers the commit path's
/// [`anchor_follow_ops`] uses ([`reproject_node_local_quantized`] +
/// [`set_path_node`]). Sharing those helpers is what makes the renderer preview and
/// the committed move byte-equivalent (pinned by `reproject_matches_cross_core_vector`).
///
/// `at` is the target-local QUANTIZED anchor point. Pure (no time/rng/IO/GPU),
/// pointer-width-agnostic.
pub fn reproject_geometry_node(
    follower_transform: &Transform3x3,
    target_transform: &Transform3x3,
    delta: &Transform3x3,
    at: LocalPoint,
    node_index: i32,
    geometry_d: &str,
) -> Option<String> {
    let target_new = delta.mul(target_transform);
    let (qx, qy) = reproject_node_local_quantized(follower_transform, &target_new, at);
    set_path_node(geometry_d, node_index, qx, qy)
}

/// The `edit-geometry` ops a committed move produces so anchored objects follow
/// their targets. `transform_ops` are the move's `set-transform` ops (each a moved
/// id + its NEW transform); every follower anchor whose target moved reprojects
/// through that target's NEW transform.
///
/// Anchor-semantics v3 §3: an open-class follower whose anchors all bind
/// ENDPOINTS deforms as ONE chord ([`deform_open_path`]) — anchored endpoints at
/// their reprojected positions, the un-anchored endpoint pinned at its original
/// position — so interior nodes follow the chord instead of staying behind as a
/// spike. Everything else (closed-class, legacy multi-subpath ink, an
/// interior-node anchor) keeps the node-splice path (rule 5 — no regression).
///
/// W3-G13: ONE cumulative `edit-geometry` per follower — each follower starts from
/// its base path-string and folds in EVERY moved-target anchor (two anchors onto
/// one moved target, or anchors onto two different moved targets, land in the SAME
/// op), so a later rewrite can never stomp an earlier one when the Batch applies
/// sequentially. Ops come out in scene-object (follower) order.
///
/// A follower that is itself in the moved set is skipped (it rides its own
/// transform), as is one whose geometry the SAME batch already rewrote (an
/// open-class moved member commits as a chord-deform `edit-geometry` instead of a
/// `set-transform`, §2c — reprojecting it again from its base would stomp that
/// deform). An object anchored to nothing moved authors nothing — so an
/// unanchored (Alt-created) move stays a no-op. The whole batch is returned in ONE
/// call (the shell batches it). Mirrors the shell `anchorFollowOps`.
pub fn anchor_follow_ops(scene: &ObjectScene, transform_ops: &[ObjectOp]) -> Vec<ObjectOp> {
    // The moved set: each id paired with its NEW transform, last write winning (a
    // Batch applies sequentially). A moved id absent from the scene anchors nothing
    // (the shell's `find` miss is a continue).
    let mut moved: Vec<(&str, &Transform3x3)> = Vec::new();
    for op in transform_ops {
        if let ObjectOp::SetTransform { id, transform } = op {
            if scene.get(id).is_none() {
                continue;
            }
            if let Some(entry) = moved.iter_mut().find(|(mid, _)| *mid == id.as_str()) {
                entry.1 = transform;
            } else {
                moved.push((id.as_str(), transform));
            }
        }
    }
    // Ids whose geometry the batch already rewrote: they ride that rewrite.
    let edited: Vec<&str> = transform_ops
        .iter()
        .filter_map(|op| match op {
            ObjectOp::EditGeometry { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut ops = Vec::new();
    for follower in &scene.objects {
        if moved.iter().any(|(id, _)| *id == follower.id)
            || edited.contains(&follower.id.as_str())
            || follower.anchors.is_empty()
        {
            continue;
        }
        // Each anchor's bound node reprojects through its moved target's NEW
        // transform; an anchor whose target did not move contributes nothing.
        let new_node = |anchor: &Anchor| -> Option<(i64, i64)> {
            let (_, target_new) = moved.iter().find(|(id, _)| *id == anchor.target)?;
            Some(reproject_node_local_quantized(&follower.transform, target_new, anchor.at))
        };
        if let Some(geometry) = follower_edit_geometry(follower, new_node) {
            ops.push(ObjectOp::EditGeometry { id: follower.id.clone(), geometry });
        }
    }
    ops
}

/// The single per-follower edit-authoring path shared by transform-follow
/// ([`anchor_follow_ops`]) and geometry-edit-follow ([`geometry_follow_ops`]):
/// fold `new_node` (the reprojected follower-local QUANTIZED position of each
/// moved-target anchor's bound node) into ONE `Geometry`, or `None` when nothing
/// changed (no anchor's target moved, or the result equals the base).
///
/// Anchor-semantics v3 §3: an open-class follower whose anchors all bind
/// ENDPOINTS deforms as ONE chord ([`deform_open_path`]) — anchored endpoints at
/// their reprojected positions, the un-anchored endpoint pinned at its original
/// position. Everything else (closed-class, legacy multi-subpath ink, an
/// interior-node anchor) keeps the node-splice fold (rule 5 — no regression).
fn follower_edit_geometry(
    follower: &Object,
    new_node: impl Fn(&Anchor) -> Option<(i64, i64)>,
) -> Option<Geometry> {
    let base = &follower.geometry.path_string;
    let pairs = local_nodes(base);
    let last_pair = i32::try_from(pairs.len().saturating_sub(1)).unwrap_or(i32::MAX);
    let endpoint_deform = pairs.len() >= 2
        && follower.anchors.iter().all(|a| a.node_index == 0 || a.node_index == last_pair)
        && is_open_class_d(base);
    let d = if endpoint_deform {
        // §3 chord follow: reprojected endpoints in, the whole silhouette out.
        let mut new_start = pairs[0];
        let mut new_end = pairs[pairs.len() - 1];
        let mut any_moved = false;
        for anchor in &follower.anchors {
            let Some((qx, qy)) = new_node(anchor) else {
                continue;
            };
            let p = (qx as f64, qy as f64);
            if anchor.node_index == 0 {
                new_start = p;
            } else {
                new_end = p;
            }
            any_moved = true;
        }
        if !any_moved {
            return None;
        }
        deform_open_path(base, new_start, new_end)?
    } else {
        // Legacy node-splice fold (rule 5). `Some` once any anchor's rewrite
        // landed; later anchors fold into it.
        let mut rewritten: Option<String> = None;
        for anchor in &follower.anchors {
            let Some((qx, qy)) = new_node(anchor) else {
                continue;
            };
            let current = rewritten.as_deref().unwrap_or(base);
            if let Some(d) = set_path_node(current, anchor.node_index, qx, qy) {
                rewritten = Some(d);
            }
        }
        rewritten?
    };
    // Author only when the cumulative result differs from the base — the
    // no-op-authors-nothing contract.
    if d == *base {
        return None;
    }
    Some(Geometry {
        path_string: d,
        fill_rule: follower.geometry.fill_rule,
        subpaths: Vec::new(),
    })
}

/// Curve-flattening tolerance for the geometry-edit reprojection of an anchor's
/// `at` onto a target's NEW outline. The finest bucket (matching [`crate::object::anchors`])
/// so endpoints land on the true outline regardless of zoom LOD.
const FOLLOW_FLATNESS: i32 = 1;

/// How a target moved within a committed batch: a `set-transform` (the outline is
/// unchanged in target-local space — the existing transform reproject) or an
/// `edit-geometry` (the outline RESHAPED — the anchor `at` must re-project onto
/// the new outline before the transform reproject).
enum MovedTarget {
    /// `set-transform`: the target's NEW transform (geometry unchanged).
    Transform(Transform3x3),
    /// `edit-geometry`: the target's NEW geometry (transform unchanged).
    Reshape(Geometry),
}

/// The reprojected follower-local QUANTIZED bound-node position for `anchor`
/// onto `target` (read from the scene for its transform), given how the target
/// moved in this batch. For a reshape, `anchor.at` is first re-projected onto the
/// NEW outline (target-local) via `deriver` so the endpoint stays glued to the
/// edited outline; for a transform move the local `at` is carried straight
/// through. Returns `None` when a reshaped target's region cannot be derived.
fn reproject_through_moved(
    deriver: &impl OutlineDeriver,
    follower_transform: &Transform3x3,
    target: &Object,
    moved: &MovedTarget,
    anchor: &Anchor,
) -> Option<(i64, i64)> {
    match moved {
        MovedTarget::Transform(target_new) => {
            Some(reproject_node_local_quantized(follower_transform, target_new, anchor.at))
        }
        MovedTarget::Reshape(geometry) => {
            // The deriver reads `subpaths`; a reshape geometry straight off the
            // commit may carry only its path-string, so hydrate before deriving.
            let mut hydrated = geometry.clone();
            hydrated.ensure_parsed().ok()?;
            let region = deriver.derive_region(&hydrated, FOLLOW_FLATNESS).ok()?;
            let at = deriver.reproject(&region, anchor.at);
            Some(reproject_node_local_quantized(follower_transform, &target.transform, at))
        }
    }
}

/// The follow `edit-geometry` ops a committed batch produces so anchored objects
/// track their targets — the geometry-edit-aware, CHAINING superset of
/// [`anchor_follow_ops`]. `ops` is the committed batch: `set-transform` ops move a
/// target (transform reproject, §3) and `edit-geometry` ops RESHAPE a target (the
/// anchor `at` re-projects onto the new outline before the same chord-deform, #2).
///
/// #3 chain propagation: a follower whose geometry this pass rewrites is itself a
/// reshaped target on the next wave, so a follower chained to THAT follower
/// (line A → line B → shape T) updates too. A `visited` set keyed by follower id
/// bounds the recursion — each follower is reprojected at most once, so a cycle in
/// the binding graph terminates instead of looping.
///
/// A follower already carrying its OWN edit in the incoming batch is skipped (its
/// geometry was placed by the same commit; reprojecting from its base would stomp
/// that edit). Followers come out in scene-object order, first wave before the
/// chained waves. Pure (no time/rng/IO/GPU), pointer-width-agnostic.
pub fn geometry_follow_ops(
    deriver: &impl OutlineDeriver,
    scene: &ObjectScene,
    ops: &[ObjectOp],
) -> Vec<ObjectOp> {
    // Seed the moved set from the committed batch (last write wins, a Batch
    // applies sequentially); a moved id absent from the scene anchors nothing.
    let mut moved: Vec<(String, MovedTarget)> = Vec::new();
    for op in ops {
        let (id, target) = match op {
            ObjectOp::SetTransform { id, transform } if scene.get(id).is_some() => {
                (id, MovedTarget::Transform(*transform))
            }
            ObjectOp::EditGeometry { id, geometry } if scene.get(id).is_some() => {
                (id, MovedTarget::Reshape(geometry.clone()))
            }
            _ => continue,
        };
        if let Some(entry) = moved.iter_mut().find(|(mid, _)| mid == id) {
            entry.1 = target;
        } else {
            moved.push((id.clone(), target));
        }
    }

    // `visited` = ids whose geometry is already pinned for this commit: every id
    // moved/reshaped by the incoming batch (an open-class moved member that
    // committed as a chord-deform is in here as a `Reshape`), plus every follower
    // this pass has already authored. Bounds the chain recursion (cycle-safe).
    let mut visited: Vec<String> = moved.iter().map(|(id, _)| id.clone()).collect();
    let mut out = Vec::new();
    // The frontier is the ids whose move/reshape we still have to propagate from.
    let mut frontier: Vec<String> = moved.iter().map(|(id, _)| id.clone()).collect();
    while !frontier.is_empty() {
        let mut next_frontier: Vec<String> = Vec::new();
        for follower in &scene.objects {
            if visited.contains(&follower.id) || follower.anchors.is_empty() {
                continue;
            }
            // Only reproject when at least one anchor targets a FRONTIER id (a
            // newly moved/reshaped target this wave), so a follower waits until
            // its target's own edit is settled before chaining off it.
            if !follower.anchors.iter().any(|a| frontier.contains(&a.target)) {
                continue;
            }
            let new_node = |anchor: &Anchor| -> Option<(i64, i64)> {
                let (_, mt) = moved.iter().find(|(id, _)| id == &anchor.target)?;
                let target = scene.get(&anchor.target)?;
                reproject_through_moved(deriver, &follower.transform, target, mt, anchor)
            };
            let Some(geometry) = follower_edit_geometry(follower, new_node) else {
                continue;
            };
            // This follower is now a reshaped target for the next wave (#3).
            visited.push(follower.id.clone());
            next_frontier.push(follower.id.clone());
            moved.push((follower.id.clone(), MovedTarget::Reshape(geometry.clone())));
            out.push(ObjectOp::EditGeometry { id: follower.id.clone(), geometry });
        }
        frontier = next_frontier;
    }
    out
}

/// Synthesize the persistent anchor(s) for a snapped drag-create, or `None` when
/// no anchor should be authored (the target is the created object, or the created
/// geometry has no node). The anchor binds `created`'s node nearest the snapped
/// world endpoint `(endpoint_x, endpoint_y)` to `target`; `at` is the snapped
/// world point mapped into the target's LOCAL quantized space, so the endpoint
/// reprojects through the target's transform on a later move. Mirrors the shell
/// `synthesizeCreateAnchors`.
pub fn synthesize_create_anchors(
    created: &Object,
    target: &Object,
    endpoint_x: f64,
    endpoint_y: f64,
) -> Option<Vec<Anchor>> {
    if target.id == created.id {
        return None;
    }
    let node_index = node_index_nearest_world(created, endpoint_x, endpoint_y)?;
    let inv = invert_affine(&affine_of(&target.transform));
    let (lx, ly) = apply_affine(&inv, endpoint_x, endpoint_y);
    let at = LocalPoint {
        x: (lx * UNITS_PER_PX).round() as i32,
        y: (ly * UNITS_PER_PX).round() as i32,
    };
    Some(vec![Anchor {
        node_index,
        target: target.id.clone(),
        at,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, ObjectSelection, SubPath};

    /// The quantum (Q=8) used by the fixtures, for de/quantizing in assertions.
    const Q: i32 = GEOMETRY_QUANTUM_PER_PX;

    fn polyline(d: &str) -> Geometry {
        let mut g = Geometry { path_string: d.to_string(), fill_rule: FillRule::EvenOdd, subpaths: Vec::new() };
        g.ensure_parsed().unwrap();
        g
    }

    /// A two-node open line at object-local (0,0)-(lx,ly) quantized units, with the
    /// given transform translate.
    fn line(id: &str, lx: i32, ly: i32, tx: f64, ty: f64) -> Object {
        let mut o = Object::new(id, "a0", polyline(&format!("M 0 0 L {lx} {ly}")));
        o.transform = Transform3x3::translate(tx, ty);
        o
    }

    fn translate(tx: f64, ty: f64) -> Transform3x3 {
        Transform3x3::translate(tx, ty)
    }

    fn move_op(id: &str, transform: Transform3x3) -> ObjectOp {
        ObjectOp::SetTransform { id: id.to_string(), transform }
    }

    fn scene_of(objects: Vec<Object>) -> ObjectScene {
        ObjectScene {
            scene_version: 1,
            objects,
            tags: Vec::new(),
            selection: ObjectSelection::Canvas,
            updated_at: String::new(),
        }
    }

    /// The world position of object `obj`'s geometry node `i` under its transform
    /// (de-quantize the local node, apply the affine). Used to assert move-together.
    fn world_node(obj: &Object, i: usize) -> (f64, f64) {
        let nodes = local_nodes(&obj.geometry.path_string);
        let (nx, ny) = nodes[i];
        apply_affine(&affine_of(&obj.transform), nx / f64::from(Q), ny / f64::from(Q))
    }

    /// A line whose node 1 is anchored to `target` at the snapped world point.
    fn anchored_line(target: &Object, endpoint_x: f64, endpoint_y: f64) -> Object {
        // Start at world (40,30); snapped endpoint at (endpoint_x, endpoint_y).
        let start_lx = 40 * Q;
        let start_ly = 30 * Q;
        let end_lx = (endpoint_x as i32) * Q;
        let end_ly = (endpoint_y as i32) * Q;
        let mut o = Object::new(
            "edge-1",
            "a1",
            polyline(&format!("M {start_lx} {start_ly} L {end_lx} {end_ly}")),
        );
        o.transform = Transform3x3::IDENTITY;
        o.anchors = synthesize_create_anchors(&o, target, endpoint_x, endpoint_y).unwrap();
        o
    }

    // (a) a pure-translation SetTransform of the target shifts the follower bound
    //     node by the SAME world delta.
    #[test]
    fn pure_translation_moves_follower_node_by_the_same_world_delta() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        // At rest the bound node sits at the snap world point (200,30).
        let (rx, ry) = world_node(&follower, 1);
        assert!((rx - 200.0).abs() < 1e-9 && (ry - 30.0).abs() < 1e-9, "rest ({rx},{ry})");

        let scene = scene_of(vec![target.clone(), follower.clone()]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        assert_eq!(ops.len(), 1, "one follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        let mut followed = follower.clone();
        followed.geometry = geometry.clone();
        let (mx, my) = world_node(&followed, 1);
        // Target moved +50 x / +20 y from its base (200,0) => endpoint (250,50).
        assert!((mx - 250.0).abs() < 1e-9 && (my - 50.0).abs() < 1e-9, "moved ({mx},{my})");
        // The free node 0 is untouched.
        assert_eq!(world_node(&followed, 0), world_node(&follower, 0));
    }

    // FALSIFY (a): if the reproject is removed (op emitted but geometry unchanged),
    // the moved node would NOT reach the target's new world point — covered above by
    // the strict (250,50) assertion failing.

    // (b) an unanchored object authors nothing.
    #[test]
    fn unanchored_object_authors_nothing() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        // Same drag, but no anchor synthesized (Alt-create bypassed the snap).
        let alt = line("edge-1", 160 * Q, 0, 0.0, 0.0);
        assert!(alt.anchors.is_empty());
        let scene = scene_of(vec![target, alt]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        assert!(ops.is_empty(), "an unanchored move-target authors no follow ops");
    }

    // (c) an object anchored only to something-not-moved authors nothing.
    #[test]
    fn anchored_only_to_unmoved_authors_nothing() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target, follower]);
        // Move a DIFFERENT object the follower is not anchored to.
        let ops = anchor_follow_ops(&scene, &[move_op("rect-z", translate(10.0, 10.0))]);
        assert!(ops.is_empty(), "moving an unrelated object reprojects nothing");
    }

    // (d) a follower that is itself in the moved set is skipped.
    #[test]
    fn follower_in_the_moved_set_is_skipped() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target, follower]);
        // Move BOTH the target and the follower in one batch: the follower rides its
        // own transform, so it must NOT also get a reproject edit-geometry.
        let ops = anchor_follow_ops(
            &scene,
            &[move_op("rect-a", translate(250.0, 20.0)), move_op("edge-1", translate(5.0, 5.0))],
        );
        assert!(
            ops.iter().all(|op| !matches!(op, ObjectOp::EditGeometry { id, .. } if id == "edge-1")),
            "a follower moved in the same batch is not separately reprojected"
        );
        assert!(ops.is_empty(), "the only anchored follower is itself moved => no ops");
    }

    // W3-G13 (RED before the cumulative rewrite): TWO anchors onto the SAME moved
    // target land in ONE edit-geometry with BOTH nodes rewritten — the old
    // first-anchor `find` dropped the second bound node.
    #[test]
    fn two_anchors_to_one_moved_target_author_one_cumulative_edit() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        // Both endpoints anchored to rect-a: node 0 at world (200,0), node 1 at
        // world (200,30) (target-local (0,0) and (0,30) px).
        let mut follower = Object::new("edge-1", "a1", polyline("M 1600 0 L 1600 240"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "rect-a".into(), at: LocalPoint { x: 0, y: 30 * Q } },
        ];
        let scene = scene_of(vec![target, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        assert_eq!(ops.len(), 1, "one cumulative follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Target new transform translate(250,20): node 0 -> world (250,20) ->
        // quantized (2000,160); node 1 -> world (250,50) -> quantized (2000,400).
        assert_eq!(geometry.path_string, "M 2000 160 L 2000 400");
    }

    // W3-G13 (RED before the cumulative rewrite): a follower anchored to two
    // DIFFERENT targets, both moved in one batch, authors ONE edit-geometry with
    // both nodes rewritten — the old per-target ops each started from the BASE
    // geometry, so the second op stomped the first node rewrite on apply.
    #[test]
    fn anchors_to_two_moved_targets_author_one_cumulative_edit() {
        let a = line("rect-a", 0, 0, 100.0, 0.0);
        let b = line("rect-b", 0, 0, 300.0, 0.0);
        // Node 0 anchored to rect-a at world (100,0); node 1 to rect-b at (300,0).
        let mut follower = Object::new("edge-1", "a1", polyline("M 800 0 L 2400 0"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 0, y: 0 } },
        ];
        let scene = scene_of(vec![a, b, follower]);
        let ops = anchor_follow_ops(
            &scene,
            &[move_op("rect-a", translate(110.0, 5.0)), move_op("rect-b", translate(310.0, 7.0))],
        );
        assert_eq!(ops.len(), 1, "one cumulative op, not one per moved target");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Node 0 -> world (110,5) -> quantized (880,40); node 1 -> world (310,7)
        // -> quantized (2480,56). BOTH survive in the one op.
        assert_eq!(geometry.path_string, "M 880 40 L 2480 56");
    }

    // W3-G13: with two different targets but only ONE moved, only its bound node
    // rewrites; the other node stays byte-identical to the base.
    #[test]
    fn only_the_moved_target_node_is_rewritten() {
        let a = line("rect-a", 0, 0, 100.0, 0.0);
        let b = line("rect-b", 0, 0, 300.0, 0.0);
        let mut follower = Object::new("edge-1", "a1", polyline("M 800 0 L 2400 0"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 0, y: 0 } },
        ];
        let scene = scene_of(vec![a, b, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(110.0, 5.0))]);
        assert_eq!(ops.len(), 1, "one follow op for the one moved target");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Node 0 follows rect-a; node 1 (anchored to unmoved rect-b) is untouched.
        assert_eq!(geometry.path_string, "M 880 40 L 2400 0");
    }

    // v3 §3 (RED before the chord deform): a moved target drags a multi-node
    // follower's anchored ENDPOINT — the interior node must follow the chord, not
    // stay behind as a spike (the old splice rewrote only the bound endpoint).
    #[test]
    fn target_move_carries_interior_nodes_along_the_chord() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        // Three-node line (0,0)->(100,0)->(200,0)px; the END (pair 2) is anchored
        // to rect-a's origin at world (200,0).
        let mut follower = Object::new("edge-1", "a1", polyline("M 0 0 L 800 0 L 1600 0"));
        follower.anchors =
            vec![Anchor { node_index: 2, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let scene = scene_of(vec![target, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(360.0, 0.0))]);
        assert_eq!(ops.len(), 1, "one cumulative follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Chord (0,0)->(1600,0) becomes (0,0)->(2880,0) (σ=1.8): the interior node
        // rides the chord to 1440. The old splice left it at 800 — the spike.
        assert_eq!(geometry.path_string, "M 0 0 L 1440 0 L 2880 0");
    }

    // v3 §3 rule 5: an anchor bound to an INTERIOR node is the node-splice era —
    // only the bound node is rewritten, the endpoints stay (no chord deform).
    #[test]
    fn interior_node_anchor_keeps_the_legacy_splice() {
        let target = line("rect-a", 0, 0, 100.0, 0.0);
        // The MIDDLE node (pair 1) sits at world (100,0) — the target's origin.
        let mut follower = Object::new("edge-1", "a1", polyline("M 0 0 L 800 0 L 1600 0"));
        follower.anchors =
            vec![Anchor { node_index: 1, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let scene = scene_of(vec![target, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(110.0, 5.0))]);
        assert_eq!(ops.len(), 1, "one splice follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Node 1 reprojects to (110,5)px => (880,40); both endpoints untouched.
        assert_eq!(geometry.path_string, "M 0 0 L 880 40 L 1600 0");
    }

    // v3 §2c: an id whose geometry the SAME batch already rewrote (an open-class
    // moved member commits as a chord-deform edit-geometry, not a set-transform)
    // is skipped as a follower — its endpoints were already placed; a reproject
    // from its base geometry would stomp the deform.
    #[test]
    fn follower_with_a_batch_edit_geometry_is_skipped() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let deformed = follower.geometry.clone();
        let scene = scene_of(vec![target, follower]);
        let ops = anchor_follow_ops(
            &scene,
            &[
                move_op("rect-a", translate(250.0, 20.0)),
                ObjectOp::EditGeometry { id: "edge-1".to_string(), geometry: deformed },
            ],
        );
        assert!(ops.is_empty(), "a batch-deformed follower is not reprojected again");
    }

    // The §3 deform path must land EXACTLY on the cross-core pinned vector: for a
    // two-node line the chord deform and the old node splice are the same map, so
    // the pin (`reproject_matches_cross_core_vector`) stays green across the
    // endpoint-routing rewrite. RED if the deform drifts by even one quantum.
    #[test]
    fn endpoint_deform_reproduces_the_cross_core_pin_vector() {
        let target = line("t", 0, 0, 10.0, 20.0);
        let mut follower = Object::new("f", "a0", polyline("M 0 0 L 64 0"));
        follower.transform = translate(100.0, 0.0);
        follower.anchors =
            vec![Anchor { node_index: 1, target: "t".into(), at: LocalPoint { x: 16, y: 8 } }];
        let scene = scene_of(vec![target, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("t", translate(15.0, 27.0))]);
        assert_eq!(ops.len(), 1, "one follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "f");
        assert_eq!(geometry.path_string, "M 0 0 L -664 224");
    }

    // (e) synthesize round-trip: a snapped create yields an Anchor whose `at` maps
    //     back (through the target transform) to the snap world point; no-target
    //     (self) yields None.
    #[test]
    fn synthesize_round_trips_through_target_transform() {
        let target = line("rect-a", 0, 0, 200.0, 0.0); // outline origin at world (200,0)
        let line = line("edge-1", 160 * Q, 0, 0.0, 0.0);
        let anchors = synthesize_create_anchors(&line, &target, 200.0, 30.0).expect("anchor");
        assert_eq!(anchors.len(), 1);
        let a = &anchors[0];
        assert_eq!(a.target, "rect-a");
        assert_eq!(a.node_index, 1, "node 1 is the dragged endpoint");
        // `at` is world (200,30) in the target's local quantized space: local (0,30)
        // => quantized (0, 30*Q).
        assert_eq!(a.at, LocalPoint { x: 0, y: 30 * Q });
        // Round-trip: de-quantize `at`, apply the target transform => snap world pt.
        let (wx, wy) = apply_affine(
            &affine_of(&target.transform),
            f64::from(a.at.x) / f64::from(Q),
            f64::from(a.at.y) / f64::from(Q),
        );
        assert!((wx - 200.0).abs() < 1e-9 && (wy - 30.0).abs() < 1e-9, "round-trip ({wx},{wy})");
    }

    #[test]
    fn synthesize_returns_none_for_self_target() {
        let line = line("edge-1", 100 * Q, 0, 0.0, 0.0);
        assert!(synthesize_create_anchors(&line, &line, 100.0, 0.0).is_none());
    }

    // CROSS-CORE EQUIVALENCE GUARD (the drift killer). ONE concrete numeric vector,
    // hand-computed, asserted HERE and in renderer-core
    // `reproject_matches_cross_core_vector` against the SAME expected, so either
    // copy drifting fails its own test.
    //
    // Shared formula (documented in both cores):
    //   follower_base = translate(100, 0); target_new = translate(15, 27)
    //     (= delta translate(5,7) composed onto target_base translate(10,20));
    //   anchor.at = (16, 8) quantized => (2, 1) px de-quantized (Q=8);
    //   d = "M 0 0 L 64 0"; node_index = 1.
    //   world = target_new * (2, 1) = (17, 28)
    //   follower-local px = follower_base^-1 * world = (17-100, 28) = (-83, 28)
    //   quantized = round((-83, 28) * 8) = (-664, 224)
    //   => set_path_node("M 0 0 L 64 0", 1, -664, 224) = "M 0 0 L -664 224"
    #[test]
    fn reproject_matches_cross_core_vector() {
        let follower = {
            let mut o = Object::new("f", "a0", polyline("M 0 0 L 64 0"));
            o.transform = translate(100.0, 0.0);
            o
        };
        let anchor = Anchor { node_index: 1, target: "t".into(), at: LocalPoint { x: 16, y: 8 } };
        let target_new = translate(15.0, 27.0);

        let (qx, qy) = reproject_node_local_quantized(&follower.transform, &target_new, anchor.at);
        assert_eq!((qx, qy), (-664, 224), "hand-computed quantized follower-local node");

        let geometry = reproject_anchored_geometry(&follower, &anchor, &target_new).expect("rewrite");
        assert_eq!(geometry.path_string, "M 0 0 L -664 224");

        // The shared LIVE wrapper composes `delta * target_base` internally and must
        // reach the SAME pinned vector — so the renderer preview (which calls it) and
        // the commit path stay byte-equivalent.
        let target_base = translate(10.0, 20.0);
        let delta = translate(5.0, 7.0);
        let rewritten = reproject_geometry_node(
            &follower.transform,
            &target_base,
            &delta,
            anchor.at,
            anchor.node_index,
            "M 0 0 L 64 0",
        )
        .expect("addressable, changed");
        assert_eq!(rewritten, "M 0 0 L -664 224");
    }

    /// WIRE-CASING PIN (cross-core). The shell forwards each object's `anchors` to
    /// the renderer VERBATIM (`canvasHost.ts`), so the renderer's `RAnchor` serde
    /// (camelCase `nodeIndex`) only round-trips if scene-core EMITS that casing. A
    /// `rename_all` regression to snake_case would silently drop `anchors` at the
    /// renderer parse and kill live follow with no host-test failure anywhere. This
    /// pins the emitted wire shape; its renderer twin
    /// (`anchored_follower_survives_wire_serde_round_trip`) pins the parse of it.
    #[test]
    fn anchor_serializes_with_camelcase_wire_keys() {
        let mut follower = Object::new("f", "a0", polyline("M 0 0 L 64 0"));
        follower.anchors =
            vec![Anchor { node_index: 1, target: "t".into(), at: LocalPoint { x: 16, y: 8 } }];
        let json = serde_json::to_string(&follower).expect("object serializes to wire JSON");
        assert!(json.contains("\"nodeIndex\":1"), "anchor must wire as camelCase nodeIndex: {json}");
        assert!(json.contains("\"target\":\"t\""), "anchor target must wire: {json}");
        assert!(json.contains("\"at\":{\"x\":16,\"y\":8}"), "anchor `at` must wire {{x,y}}: {json}");
        assert!(!json.contains("node_index"), "snake_case node_index would be dropped: {json}");
    }

    /// A `set-path-node` no-op (the new coords already match) authors nothing, so
    /// the commit path skips a pointless edit-geometry — matching the shell.
    #[test]
    fn reproject_no_op_authors_nothing() {
        // Target at its base so the follower node stays where it already is.
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target.clone(), follower]);
        // Move the target to its CURRENT transform (a no-move): the reproject is a
        // no-op, so no follow op is authored.
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", target.transform)]);
        assert!(ops.is_empty(), "a no-op reproject authors nothing");
    }

    /// `SubPath` import keeps the parse path exercised by `polyline` honest.
    #[test]
    fn polyline_parses_into_subpaths() {
        let g = polyline("M 0 0 L 80 0");
        assert_eq!(g.subpaths.len(), 1);
        assert_eq!(g.subpaths[0], SubPath { closed: false, nodes: g.subpaths[0].nodes.clone() });
    }

    // --- #2 geometry-edit follow + #3 chain propagation -------------------

    use crate::object::model::PathNode;
    use crate::object::region::StubOutlineDeriver;

    /// A hydrated closed rect with the given quantized corners (the stub deriver
    /// reprojects an anchor onto the nearest of these four outline vertices).
    fn rect(id: &str, x0: i32, y0: i32, x1: i32, y1: i32) -> Object {
        Object::new(
            id,
            "a0",
            Geometry::from_subpaths(
                vec![SubPath {
                    closed: true,
                    nodes: vec![
                        PathNode::corner(x0, y0),
                        PathNode::corner(x1, y0),
                        PathNode::corner(x1, y1),
                        PathNode::corner(x0, y1),
                    ],
                }],
                FillRule::EvenOdd,
            ),
        )
    }

    /// The committed `edit-geometry` op reshaping `id` to the given closed rect.
    fn reshape_rect_op(id: &str, x0: i32, y0: i32, x1: i32, y1: i32) -> ObjectOp {
        let geometry = rect("ignored", x0, y0, x1, y1).geometry;
        ObjectOp::EditGeometry { id: id.to_string(), geometry }
    }

    // #2 GOLDEN: a committed `edit-geometry` that RESHAPES a target reprojects the
    // anchor's `at` onto the target's NEW outline and chord-deforms the follower —
    // the SAME endpoint-deform path the transform-follow uses. FALSIFY: neutralize
    // the reprojection wiring (drop EditGeometry targets, or reproject through the
    // OLD outline) and the op vanishes / the endpoint stays at (800,0) — RED.
    #[test]
    fn target_geometry_edit_reprojects_open_follower() {
        // shape-t: identity-transform rect, corners (0,0)-(800,400). edge: an open
        // line whose node 1 anchors to shape-t's top-right corner at local (800,0).
        let target = rect("shape-t", 0, 0, 800, 400);
        let mut follower = Object::new("edge", "a1", polyline("M 320 160 L 800 0"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors =
            vec![Anchor { node_index: 1, target: "shape-t".into(), at: LocalPoint { x: 800, y: 0 } }];
        let scene = scene_of(vec![target, follower]);

        // Reshape: drag the top-right corner (800,0) down to (800,240). The stub
        // reprojects the anchor's old `at`=(800,0) onto the nearest NEW vertex,
        // which is the moved corner (800,240) (dist 240² < every other corner).
        let reshape = reshape_rect_op("shape-t", 0, 240, 800, 400);
        let ops = geometry_follow_ops(&StubOutlineDeriver, &scene, &[reshape.clone()]);
        assert_eq!(ops.len(), 1, "one follow op for the reshaped target: {ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry, got {ops:?}");
        };
        assert_eq!(id, "edge");
        // node 1 reprojects to local (800,240); the free node 0 pins at (320,160).
        assert_eq!(geometry.path_string, "M 320 160 L 800 240");

        // Through REAL op-apply (the oracle): commit the reshape + the follow op as
        // one batch and assert the follower's bound endpoint world position lands on
        // the target's NEW outline vertex (100,30)px — the reprojected attachment.
        let mut applied = scene.clone();
        let mut batch = vec![reshape];
        batch.extend(ops);
        crate::object::apply::apply_object_op(&mut applied, ObjectOp::Batch { ops: batch })
            .expect("the follow batch applies through op-apply");
        let edge = applied.get("edge").unwrap();
        let (wx, wy) = world_node(edge, 1);
        assert!(
            (wx - 100.0).abs() < 1e-9 && (wy - 30.0).abs() < 1e-9,
            "the committed endpoint sits on the reshaped outline ({wx},{wy})"
        );
    }

    // #2: a follower already carrying its OWN edit in the batch is NOT reprojected
    // again (its geometry was placed by the same commit).
    #[test]
    fn follower_with_its_own_edit_in_the_batch_is_skipped_on_reshape() {
        let target = rect("shape-t", 0, 0, 800, 400);
        let mut follower = Object::new("edge", "a1", polyline("M 320 160 L 800 0"));
        follower.anchors =
            vec![Anchor { node_index: 1, target: "shape-t".into(), at: LocalPoint { x: 800, y: 0 } }];
        let own_edit = follower.geometry.clone();
        let scene = scene_of(vec![target, follower]);
        let ops = geometry_follow_ops(
            &StubOutlineDeriver,
            &scene,
            &[
                reshape_rect_op("shape-t", 0, 240, 800, 400),
                ObjectOp::EditGeometry { id: "edge".to_string(), geometry: own_edit },
            ],
        );
        assert!(
            ops.iter().all(|op| !matches!(op, ObjectOp::EditGeometry { id, .. } if id == "edge")),
            "a follower with its own batch edit is not reprojected again: {ops:?}"
        );
    }

    // #3 GOLDEN: line A anchored to line B anchored to shape T. Moving T must
    // propagate TWO hops — B follows T, then A follows B's new geometry. FALSIFY:
    // remove the chain recursion and only B updates (A is dropped) — RED.
    #[test]
    fn line_to_line_chain_propagates_two_hops() {
        // shape-t: identity rect (0,0)-(800,400). line-b: node 1 anchored to shape-t
        // at the top-right corner (800,0). line-a: node 1 anchored to line-b's END
        // node (its node 1) — so A follows B follows T.
        let target = rect("shape-t", 0, 0, 800, 400);
        let mut b = Object::new("line-b", "a1", polyline("M 0 0 L 800 0"));
        b.transform = Transform3x3::IDENTITY;
        b.anchors =
            vec![Anchor { node_index: 1, target: "shape-t".into(), at: LocalPoint { x: 800, y: 0 } }];
        let mut a = Object::new("line-a", "a2", polyline("M 0 400 L 800 0"));
        a.transform = Transform3x3::IDENTITY;
        // A's node 1 anchors onto line-b's node-1 endpoint at B-local (800,0).
        a.anchors =
            vec![Anchor { node_index: 1, target: "line-b".into(), at: LocalPoint { x: 800, y: 0 } }];
        let scene = scene_of(vec![target, b, a]);

        // Move shape-t by (+50,+20)px: its top-right corner world (100,0) -> (150,20).
        let ops = geometry_follow_ops(
            &StubOutlineDeriver,
            &scene,
            &[move_op("shape-t", translate(50.0, 20.0))],
        );

        let edited: std::collections::HashMap<&str, &str> = ops
            .iter()
            .filter_map(|op| match op {
                ObjectOp::EditGeometry { id, geometry } => {
                    Some((id.as_str(), geometry.path_string.as_str()))
                }
                _ => None,
            })
            .collect();
        // B follows T's moved corner: node 1 -> world (150,20) -> B-local (1200,160).
        assert_eq!(edited.get("line-b"), Some(&"M 0 0 L 1200 160"), "B follows T: {ops:?}");
        // A follows B's new endpoint: the chain reaches the second hop.
        assert_eq!(edited.get("line-a"), Some(&"M 0 400 L 1200 160"), "A follows B: {ops:?}");
    }

    // #3: a binding CYCLE (A anchored to B, B anchored to A) must terminate via the
    // visited set — no infinite recursion when a reshaped follower feeds back.
    #[test]
    fn anchor_cycle_terminates() {
        let target = rect("shape-t", 0, 0, 800, 400);
        let mut b = Object::new("line-b", "a1", polyline("M 0 0 L 800 0"));
        b.anchors = vec![
            Anchor { node_index: 1, target: "shape-t".into(), at: LocalPoint { x: 800, y: 0 } },
            Anchor { node_index: 0, target: "line-a".into(), at: LocalPoint { x: 0, y: 0 } },
        ];
        let mut a = Object::new("line-a", "a2", polyline("M 0 400 L 800 0"));
        a.anchors =
            vec![Anchor { node_index: 1, target: "line-b".into(), at: LocalPoint { x: 800, y: 0 } }];
        let scene = scene_of(vec![target, b, a]);
        // Must return (not hang); each follower is reprojected at most once.
        let ops = geometry_follow_ops(
            &StubOutlineDeriver,
            &scene,
            &[move_op("shape-t", translate(50.0, 20.0))],
        );
        let b_edits = ops
            .iter()
            .filter(|op| matches!(op, ObjectOp::EditGeometry { id, .. } if id == "line-b"))
            .count();
        let a_edits = ops
            .iter()
            .filter(|op| matches!(op, ObjectOp::EditGeometry { id, .. } if id == "line-a"))
            .count();
        assert_eq!(b_edits, 1, "B reprojected exactly once despite the cycle: {ops:?}");
        assert_eq!(a_edits, 1, "A reprojected exactly once despite the cycle: {ops:?}");
    }

    // #3: a transform move with a single anchored follower and NO chain stays a
    // one-op follow — the recursion adds nothing when there is no second hop (the
    // geometry-edit-aware path is a superset of the transform-only follow).
    #[test]
    fn geometry_follow_matches_transform_follow_without_a_chain() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target, follower]);
        let transform_only = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        let geometry_aware = geometry_follow_ops(
            &StubOutlineDeriver,
            &scene,
            &[move_op("rect-a", translate(250.0, 20.0))],
        );
        assert_eq!(transform_only, geometry_aware, "no chain => identical to the transform follow");
    }
}
