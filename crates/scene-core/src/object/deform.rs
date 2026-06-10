//! Chord-similarity deform for open-class paths (anchor-semantics design v3
//! §1/§2a, `docs/object-redesign-anchor-semantics-design.md`).
//!
//! open-class ⇔ exactly one subpath and that subpath is `closed:false` — the
//! data-level dichotomy of §1, not a UI classifier. An open path's pose IS its
//! endpoint pair: when the endpoints move (s,e) → (s′,e′), the unique
//! similarity `S = T(s′)·R(Δθ)·σI·T(−s)` taking the old chord to the new one is
//! applied to EVERY coordinate pair — bezier control points included — so the
//! drawn silhouette rotates/stretches as one piece (the "rope/rubber-band"
//! answer; absorbs v2 DU3 handle-follow with no separate mechanism).
//!
//! [`deform_open_path`] is the single source both the commit path
//! (EditGeometry) and the renderer live preview (G14 reexpand+patch, scene-core
//! consumed as an rlib in-process — no per-frame FFI) call, so committed and
//! previewed bytes cannot drift.
//!
//! Degenerate guard (§2a): an old chord under 1px, or a scale ratio σ outside
//! `[1/SIGMA_MAX, SIGMA_MAX]`, falls back to a pure translation by (s′−s) — no
//! rotation/scale — protecting spiral-like inputs whose chord ≪ arc length.
//!
//! Pure (no time/rng/IO), pointer-width-agnostic. Coordinates are object-local
//! quantized units (Q=8), the same space `reproject_node_local_quantized` emits.

use super::anchor_follow::{affine_of, apply_affine, invert_affine, local_nodes};
use super::model::{
    path_string, Anchor, Geometry, HandlePoint, LocalPoint, ObjectScene, PathNode, SubPath,
    Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use super::op::ObjectOp;

/// Quantized units per logical pixel (Q=8).
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

/// Old-chord length (quantized units) below which σ blows up: 1px. Falls back
/// to translation.
const MIN_CHORD_UNITS: f64 = UNITS_PER_PX;

/// Uniform-scale clamp bound: σ outside `[1/SIGMA_MAX, SIGMA_MAX]` is treated
/// as degenerate and falls back to translation.
const SIGMA_MAX: f64 = 64.0;

/// True iff `geometry` is open-class (§1): exactly one subpath, not closed.
/// Reads the hydrated `subpaths`; an unhydrated geometry (fresh off the wire)
/// is classified from its path-string instead of silently reporting false.
pub fn is_open_class(geometry: &Geometry) -> bool {
    if geometry.subpaths.is_empty() {
        return is_open_class_d(&geometry.path_string);
    }
    geometry.subpaths.len() == 1 && !geometry.subpaths[0].closed
}

/// Path-string form of [`is_open_class`] (the renderer consumes this via the
/// rlib). Malformed input is not open-class.
pub fn is_open_class_d(d: &str) -> bool {
    match path_string::parse(d) {
        Ok(subpaths) => subpaths.len() == 1 && !subpaths[0].closed,
        Err(_) => false,
    }
}

/// Round a deformed coordinate back to a quantized i32 unit (clamp into i32
/// range; NaN maps to 0) — the same provably-safe narrowing as
/// `drawing::quantize_px`, minus the px→unit scale (inputs are already units).
fn round_unit(v: f64) -> i32 {
    if v.is_nan() {
        return 0;
    }
    let r = v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the rounded f64 is an exact integer in range"
    )]
    let q = r as i32;
    q
}

/// Apply the chord similarity to an open-class path-string: node 0 is the old
/// start, the last node the old end; the similarity taking that chord to
/// `new_start`→`new_end` rewrites every coordinate pair (bezier control points
/// included), round-quantized back to integers. Coordinates — inputs and the
/// path-string alike — are quantized units (Q=8).
///
/// Returns `None` when `d` is not open-class (multi-subpath / closed /
/// malformed); a degenerate chord or σ falls back to translation (module doc).
pub fn deform_open_path(d: &str, new_start: (f64, f64), new_end: (f64, f64)) -> Option<String> {
    let subpaths = path_string::parse(d).ok()?;
    if subpaths.len() != 1 || subpaths[0].closed {
        return None;
    }
    let sub = &subpaths[0];
    let first = sub.nodes.first()?;
    let last = sub.nodes.last()?;
    let old_start = (f64::from(first.x), f64::from(first.y));
    let old_end = (f64::from(last.x), f64::from(last.y));
    let (ux, uy) = (old_end.0 - old_start.0, old_end.1 - old_start.1);
    let (vx, vy) = (new_end.0 - new_start.0, new_end.1 - new_start.1);
    let chord_sq = ux * ux + uy * uy;

    // The similarity as a complex ratio z = (e′−s′)/(e−s): one (zr, zi) pair
    // encodes R(Δθ)·σ, with σ = |z|. `None` = degenerate ⇒ translation only.
    let mut z = None;
    if chord_sq.sqrt() >= MIN_CHORD_UNITS {
        let zr = (vx * ux + vy * uy) / chord_sq;
        let zi = (vy * ux - vx * uy) / chord_sq;
        let sigma = zr.hypot(zi);
        if (1.0 / SIGMA_MAX..=SIGMA_MAX).contains(&sigma) {
            z = Some((zr, zi));
        }
    }
    let map = |x: f64, y: f64| -> (f64, f64) {
        match z {
            Some((zr, zi)) => {
                let dx = x - old_start.0;
                let dy = y - old_start.1;
                (new_start.0 + zr * dx - zi * dy, new_start.1 + zi * dx + zr * dy)
            }
            None => (x + new_start.0 - old_start.0, y + new_start.1 - old_start.1),
        }
    };

    let nodes = sub
        .nodes
        .iter()
        .map(|n| {
            let (x, y) = map(f64::from(n.x), f64::from(n.y));
            let (qx, qy) = (round_unit(x), round_unit(y));
            // Handles are node-relative, but the similarity applies to their
            // ABSOLUTE control points, so every emitted C coordinate is exactly
            // round(S(absolute)) — the handle-follow contract.
            let follow = |h: Option<HandlePoint>| {
                h.map(|h| {
                    let (ax, ay) = map(f64::from(n.x + h.dx), f64::from(n.y + h.dy));
                    HandlePoint { dx: round_unit(ax) - qx, dy: round_unit(ay) - qy }
                })
            };
            PathNode {
                x: qx,
                y: qy,
                in_handle: follow(n.in_handle),
                out_handle: follow(n.out_handle),
                width: n.width,
            }
        })
        .collect();
    Some(path_string::serialize(&[SubPath { closed: false, nodes }]))
}

/// True iff `t` is a pure translation (identity linear part, affine bottom
/// row) — the §2b/§3 routing predicate: a moved open-class member whose
/// endpoints all follow keeps the 0-rebake SetTransform only under a pure
/// translate (rule 1 and the rule-3 reduction).
pub fn is_pure_translate(t: &Transform3x3) -> bool {
    let m = &t.m;
    m[0][0] == 1.0
        && m[0][1] == 0.0
        && m[1][0] == 0.0
        && m[1][1] == 1.0
        && m[2][0] == 0.0
        && m[2][1] == 0.0
        && m[2][2] == 1.0
}

/// The `(start_pinned, end_pinned)` flags of an open-class member of a moved
/// set (§3 table): an endpoint anchored to a target OUTSIDE the moved set is
/// pinned (glued to its attachment point); one anchored to a moved target
/// follows the batch delta exactly like a free endpoint (the delta cancels).
/// Returns `None` for the legacy cases that keep the old route (rule 5):
/// fewer than two coordinate pairs, or an anchor bound to an interior node
/// (the node-splice era).
pub fn open_endpoint_pins(
    d: &str,
    anchors: &[Anchor],
    mut target_moved: impl FnMut(&str) -> bool,
) -> Option<(bool, bool)> {
    let pair_count = local_nodes(d).len();
    if pair_count < 2 {
        return None;
    }
    let last = i32::try_from(pair_count - 1).ok()?;
    if anchors.iter().any(|a| a.node_index != 0 && a.node_index != last) {
        return None;
    }
    let mut pinned =
        |idx: i32| anchors.iter().any(|a| a.node_index == idx && !target_moved(&a.target));
    let start = pinned(0);
    let end = pinned(last);
    Some((start, end))
}

/// How one open-class member of a moved batch commits — the §3 decision
/// table's single source (the commit cascade and the renderer live preview
/// both follow it; see [`route_open_endpoints`]).
pub enum EndpointRoute {
    /// Rule 1 (and the rule-3 reduction): nothing pins and the delta is a pure
    /// translate — keep the existing SetTransform (geometry untouched,
    /// instance-matrix live, 0-rebake).
    Translate,
    /// Both endpoints pinned by unmoved anchor targets: the member cannot move
    /// — author nothing (Alt-drag detach is the way out, DU4).
    Pinned,
    /// Rewrite the geometry to this endpoint pair (object-local quantized
    /// units) via [`deform_open_path`].
    Deform { new_start: (f64, f64), new_end: (f64, f64) },
}

/// Route one open-class moved member (§2b body drag, §2c slave, §3 table):
/// a followed endpoint `p` maps through `inv(T)·delta·T·p`; a pinned endpoint
/// keeps its original local position. Returns `None` when `d` is not
/// open-class (the caller keeps its legacy whole-transform path). The pin
/// flags come from [`open_endpoint_pins`].
pub fn route_open_endpoints(
    d: &str,
    transform: &Transform3x3,
    delta: &Transform3x3,
    start_pinned: bool,
    end_pinned: bool,
) -> Option<EndpointRoute> {
    let subpaths = path_string::parse(d).ok()?;
    if subpaths.len() != 1 || subpaths[0].closed {
        return None;
    }
    if start_pinned && end_pinned {
        return Some(EndpointRoute::Pinned);
    }
    if !start_pinned && !end_pinned && is_pure_translate(delta) {
        return Some(EndpointRoute::Translate);
    }
    let sub = &subpaths[0];
    let first = sub.nodes.first()?;
    let last = sub.nodes.last()?;
    let inv = invert_affine(&affine_of(transform));
    let follow = |n: &PathNode| {
        let (wx, wy) =
            transform.apply_point(f64::from(n.x) / UNITS_PER_PX, f64::from(n.y) / UNITS_PER_PX);
        let (mx, my) = delta.apply_point(wx, wy);
        let (lx, ly) = apply_affine(&inv, mx, my);
        (lx * UNITS_PER_PX, ly * UNITS_PER_PX)
    };
    let new_start =
        if start_pinned { (f64::from(first.x), f64::from(first.y)) } else { follow(first) };
    let new_end = if end_pinned { (f64::from(last.x), f64::from(last.y)) } else { follow(last) };
    Some(EndpointRoute::Deform { new_start, new_end })
}

/// The commit ops of an endpoint-drag release (§2b): ONE chord-deform
/// `edit-geometry` moving the dragged endpoint (`node_index`, 0 or last) to
/// `new_point_px` (world px), plus the `set-anchor` rewrite of the owner's
/// WHOLE anchors vector — that endpoint rebound to `snap`
/// `(target_id, world_at_px)` when the release snapped (the same target-local
/// quantization as `synthesize_create_anchors`), or its anchor removed when
/// the release landed in empty space. Returns `[]` when `id` is unknown, not
/// open-class, or `node_index` is not an endpoint (the endpoint surface exists
/// only on open-class ends). Forward authoring only — op-apply is untouched.
pub fn endpoint_release_ops(
    scene: &ObjectScene,
    id: &str,
    node_index: i32,
    new_point_px: (f64, f64),
    snap: Option<(&str, (f64, f64))>,
) -> Vec<ObjectOp> {
    let Some(object) = scene.get(id) else {
        return Vec::new();
    };
    let d = &object.geometry.path_string;
    if !is_open_class_d(d) {
        return Vec::new();
    }
    let pairs = local_nodes(d);
    if pairs.len() < 2 {
        return Vec::new();
    }
    let Ok(last) = i32::try_from(pairs.len() - 1) else {
        return Vec::new();
    };
    if node_index != 0 && node_index != last {
        return Vec::new();
    }
    let inv = invert_affine(&affine_of(&object.transform));
    let (lx, ly) = apply_affine(&inv, new_point_px.0, new_point_px.1);
    let new_local = (lx * UNITS_PER_PX, ly * UNITS_PER_PX);
    let (new_start, new_end) = if node_index == 0 {
        (new_local, pairs[pairs.len() - 1])
    } else {
        (pairs[0], new_local)
    };

    let mut ops = Vec::new();
    if let Some(new_d) = deform_open_path(d, new_start, new_end) {
        if new_d != *d {
            ops.push(ObjectOp::EditGeometry {
                id: object.id.clone(),
                geometry: Geometry {
                    path_string: new_d,
                    fill_rule: object.geometry.fill_rule,
                    subpaths: Vec::new(),
                },
            });
        }
    }

    // The whole-vector SetAnchor rewrite: drop this endpoint's binding, then
    // rebind it when the release snapped onto a live (non-self) target.
    let mut anchors: Vec<Anchor> =
        object.anchors.iter().filter(|a| a.node_index != node_index).cloned().collect();
    if let Some((target_id, (ax, ay))) = snap {
        if target_id != id {
            if let Some(target) = scene.get(target_id) {
                let tinv = invert_affine(&affine_of(&target.transform));
                let (tx, ty) = apply_affine(&tinv, ax, ay);
                anchors.push(Anchor {
                    node_index,
                    target: target.id.clone(),
                    at: LocalPoint {
                        x: (tx * UNITS_PER_PX).round() as i32,
                        y: (ty * UNITS_PER_PX).round() as i32,
                    },
                });
            }
        }
    }
    if anchors != object.anchors {
        ops.push(ObjectOp::SetAnchor { id: object.id.clone(), anchors });
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::FillRule;

    /// A canonical 3-node open path with bezier handles: n0 (0,0) with
    /// out-handle (8,−8), n1 (32,0) with in-handle (−8,−8), n2 (64,0).
    /// Chord: (0,0) → (64,0). All coords quantized units (8 = 1px).
    const CURVE: &str = "M 0 0 C 8 -8 24 -8 32 0 L 64 0";

    fn hydrated(d: &str) -> Geometry {
        let mut g = Geometry {
            path_string: d.to_string(),
            fill_rule: FillRule::EvenOdd,
            subpaths: Vec::new(),
        };
        g.ensure_parsed().unwrap();
        g
    }

    // -- is_open_class_d: positive / negative --

    #[test]
    fn open_class_d_accepts_a_single_open_subpath() {
        assert!(is_open_class_d("M 0 0 L 64 0"));
        assert!(is_open_class_d(CURVE));
    }

    #[test]
    fn open_class_d_rejects_closed_multi_and_malformed() {
        assert!(!is_open_class_d("M 0 0 L 80 0 L 80 40 L 0 40 Z"), "closed");
        assert!(!is_open_class_d("M 0 0 L 8 0 M 16 0 L 24 0"), "multi-subpath");
        assert!(!is_open_class_d(""), "empty");
        assert!(!is_open_class_d("Q 1 2"), "malformed");
    }

    // -- is_open_class: hydrated + unhydrated --

    #[test]
    fn open_class_reads_hydrated_subpaths() {
        assert!(is_open_class(&hydrated("M 0 0 L 64 0")));
        assert!(!is_open_class(&hydrated("M 0 0 L 80 0 L 80 40 L 0 40 Z")));
    }

    #[test]
    fn open_class_classifies_unhydrated_geometry_from_the_path_string() {
        let g = Geometry {
            path_string: "M 0 0 L 64 0".to_string(),
            fill_rule: FillRule::EvenOdd,
            subpaths: Vec::new(),
        };
        assert!(is_open_class(&g));
    }

    // -- deform: identity (same endpoints -> byte-identical d) --

    #[test]
    fn identity_endpoints_return_the_same_d() {
        assert_eq!(deform_open_path(CURVE, (0.0, 0.0), (64.0, 0.0)), Some(CURVE.to_string()));
    }

    // -- deform: pure translation (both endpoints share one delta) --

    #[test]
    fn pure_translation_shifts_every_coordinate_pair() {
        // Both endpoints +(16,24): z = 1 (no rotation/scale), so every node AND
        // every absolute control point shifts by exactly that delta.
        assert_eq!(
            deform_open_path(CURVE, (16.0, 24.0), (80.0, 24.0)),
            Some("M 16 24 C 24 16 40 16 48 24 L 80 24".to_string())
        );
    }

    // -- deform: hand-computed rotation + scale, handles following --

    #[test]
    fn rotate_and_scale_carry_the_bezier_handles() {
        // Chord (64,0) → (0,128): z = 2i, i.e. rotate +90° and scale ×2, so
        // S(x,y) = (−2y, 2x). Every absolute pair, control points included:
        //   n0 (0,0)   → (0,0);   c1 (8,−8)  → (16,16)
        //   c2 (24,−8) → (16,48); n1 (32,0)  → (0,64)
        //   n2 (64,0)  → (0,128)
        let out = deform_open_path(CURVE, (0.0, 0.0), (0.0, 128.0)).expect("open-class");
        assert_eq!(out, "M 0 0 C 16 16 16 48 0 64 L 0 128");
    }

    // -- degenerate guards: translation fallback --

    #[test]
    fn sub_pixel_chord_falls_back_to_translation() {
        // Old chord is 4 units (< 8 = 1px): the requested rotation/scale is
        // ignored; the whole path translates by (new_start − old_start) and the
        // free end does NOT land on new_end.
        assert_eq!(
            deform_open_path("M 0 0 L 4 0", (8.0, 8.0), (104.0, 208.0)),
            Some("M 8 8 L 12 8".to_string())
        );
    }

    #[test]
    fn sigma_outside_the_clamp_falls_back_to_translation() {
        // σ = 65 > 64: fallback. new_start == old_start, so the d is unchanged.
        assert_eq!(
            deform_open_path("M 0 0 L 8 0", (0.0, 0.0), (520.0, 0.0)),
            Some("M 0 0 L 8 0".to_string())
        );
        // σ = 5/640 < 1/64: same fallback on the shrink side.
        assert_eq!(
            deform_open_path("M 0 0 L 640 0", (0.0, 0.0), (5.0, 0.0)),
            Some("M 0 0 L 640 0".to_string())
        );
        // σ = 64 exactly stays a similarity (the clamp is inclusive).
        assert_eq!(
            deform_open_path("M 0 0 L 8 0", (0.0, 0.0), (512.0, 0.0)),
            Some("M 0 0 L 512 0".to_string())
        );
    }

    // -- deform: non-open-class input --

    #[test]
    fn closed_or_multi_subpath_deforms_to_none() {
        assert!(deform_open_path("M 0 0 L 80 0 L 80 40 L 0 40 Z", (0.0, 0.0), (1.0, 1.0)).is_none());
        assert!(deform_open_path("M 0 0 L 8 0 M 16 0 L 24 0", (0.0, 0.0), (1.0, 1.0)).is_none());
        assert!(deform_open_path("", (0.0, 0.0), (1.0, 1.0)).is_none());
    }

    // -- endpoint pins (§3 table) --

    #[test]
    fn pins_follow_moved_targets_and_pin_unmoved_ones() {
        let anchors = vec![
            Anchor { node_index: 0, target: "moved".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "still".into(), at: LocalPoint { x: 0, y: 0 } },
        ];
        // Start follows its moved target; end pins to the unmoved one.
        assert_eq!(
            open_endpoint_pins("M 0 0 L 800 0", &anchors, |t| t == "moved"),
            Some((false, true))
        );
        // Free endpoints never pin.
        assert_eq!(open_endpoint_pins("M 0 0 L 800 0", &[], |_| false), Some((false, false)));
    }

    #[test]
    fn interior_node_anchor_routes_to_legacy() {
        // Node 1 of a three-pair path is interior — the node-splice era (rule 5).
        let anchors =
            vec![Anchor { node_index: 1, target: "a".into(), at: LocalPoint { x: 0, y: 0 } }];
        assert_eq!(open_endpoint_pins("M 0 0 L 800 0 L 1600 0", &anchors, |_| false), None);
    }

    // -- endpoint routing (§2b/§2c/§3 decision table) --

    #[test]
    fn route_keeps_set_transform_when_nothing_pins_under_a_translate() {
        let delta = Transform3x3::translate(40.0, 30.0);
        assert!(matches!(
            route_open_endpoints("M 0 0 L 800 0", &Transform3x3::IDENTITY, &delta, false, false),
            Some(EndpointRoute::Translate)
        ));
    }

    #[test]
    fn route_is_a_no_op_when_both_endpoints_pin() {
        let delta = Transform3x3::translate(40.0, 30.0);
        assert!(matches!(
            route_open_endpoints("M 0 0 L 800 0", &Transform3x3::IDENTITY, &delta, true, true),
            Some(EndpointRoute::Pinned)
        ));
    }

    #[test]
    fn route_deforms_only_the_free_end_when_one_endpoint_pins() {
        // Body translate (40,30)px with the start pinned: the free end takes
        // the whole delta, the pinned end holds its original local position.
        let delta = Transform3x3::translate(40.0, 30.0);
        let Some(EndpointRoute::Deform { new_start, new_end }) =
            route_open_endpoints("M 0 0 L 800 0", &Transform3x3::IDENTITY, &delta, true, false)
        else {
            panic!("expected deform route");
        };
        assert_eq!(new_start, (0.0, 0.0));
        assert_eq!(new_end, (1120.0, 240.0));
    }

    #[test]
    fn route_deforms_under_a_non_translate_delta() {
        // A 90° rotation about the origin maps the chord (0,0)->(100,0)px to
        // (0,0)->(0,100)px even with nothing pinned (rule 2).
        let rot90 = Transform3x3 { m: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] };
        let Some(EndpointRoute::Deform { new_start, new_end }) =
            route_open_endpoints("M 0 0 L 800 0", &Transform3x3::IDENTITY, &rot90, false, false)
        else {
            panic!("expected deform route");
        };
        assert_eq!(new_start, (0.0, 0.0));
        assert_eq!(new_end, (0.0, 800.0));
    }

    #[test]
    fn route_returns_none_for_closed_class() {
        let delta = Transform3x3::translate(1.0, 1.0);
        assert!(route_open_endpoints(
            "M 0 0 L 80 0 L 80 40 L 0 40 Z",
            &Transform3x3::IDENTITY,
            &delta,
            false,
            false
        )
        .is_none());
    }

    // -- endpoint_release_ops: rebind / unbind --

    use crate::object::model::{Object, ObjectSelection};

    /// rect-a (old anchor target) at (100,0), rect-b (snap target) at (300,0),
    /// `edge` an identity-transform line (0,0)->(100,0)px whose node 1 is
    /// anchored to rect-a, plus a 3-node `wire` for the interior-node guard.
    fn release_scene() -> ObjectScene {
        let mut rect_a = Object::new("rect-a", "a0", hydrated("M 0 0 L 80 0 L 80 40 L 0 40 Z"));
        rect_a.transform = Transform3x3::translate(100.0, 0.0);
        let mut rect_b = Object::new("rect-b", "a1", hydrated("M 0 0 L 80 0 L 80 40 L 0 40 Z"));
        rect_b.transform = Transform3x3::translate(300.0, 0.0);
        let mut edge = Object::new("edge", "a2", hydrated("M 0 0 L 800 0"));
        edge.anchors =
            vec![Anchor { node_index: 1, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let wire = Object::new("wire", "a3", hydrated("M 0 0 L 80 0 L 160 0"));
        ObjectScene {
            scene_version: 1,
            objects: vec![rect_a, rect_b, edge, wire],
            tags: Vec::new(),
            selection: ObjectSelection::Canvas,
            updated_at: String::new(),
        }
    }

    #[test]
    fn endpoint_release_rebinds_the_snapped_endpoint() {
        let scene = release_scene();
        // Release node 1 at world (300,40), snapped onto rect-b at that point.
        let ops =
            endpoint_release_ops(&scene, "edge", 1, (300.0, 40.0), Some(("rect-b", (300.0, 40.0))));
        assert_eq!(ops.len(), 2, "deform + anchor rewrite: {ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry first");
        };
        assert_eq!(id, "edge");
        assert_eq!(geometry.path_string, "M 0 0 L 2400 320");
        let ObjectOp::SetAnchor { id, anchors } = &ops[1] else {
            panic!("expected set-anchor second");
        };
        assert_eq!(id, "edge");
        // The rect-a binding is replaced by rect-b at target-local (0,40)px.
        assert_eq!(
            anchors,
            &vec![Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 0, y: 320 } }]
        );
    }

    #[test]
    fn endpoint_release_in_empty_space_unbinds() {
        let scene = release_scene();
        let ops = endpoint_release_ops(&scene, "edge", 1, (150.0, 10.0), None);
        assert_eq!(ops.len(), 2, "deform + anchor removal: {ops:?}");
        let ObjectOp::EditGeometry { geometry, .. } = &ops[0] else {
            panic!("expected edit-geometry first");
        };
        assert_eq!(geometry.path_string, "M 0 0 L 1200 80");
        let ObjectOp::SetAnchor { anchors, .. } = &ops[1] else {
            panic!("expected set-anchor second");
        };
        assert!(anchors.is_empty(), "the endpoint's anchor is removed");
    }

    #[test]
    fn endpoint_release_rejects_non_endpoint_and_closed_class() {
        let scene = release_scene();
        // Closed-class objects have no endpoint surface.
        assert!(endpoint_release_ops(&scene, "rect-a", 1, (0.0, 0.0), None).is_empty());
        // An interior node of an open path is the edit-mode wave, not this op.
        assert!(endpoint_release_ops(&scene, "wire", 1, (50.0, 0.0), None).is_empty());
        // An unknown id authors nothing.
        assert!(endpoint_release_ops(&scene, "ghost", 0, (0.0, 0.0), None).is_empty());
    }
}
