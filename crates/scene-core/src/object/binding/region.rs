//! Derived outline/region: geometry -> outline/region. Pure (no IO/time/rng);
//! coordinates are object-local quantized i32. The consumer caches by
//! (object id, geometry revision).

use serde::{Deserialize, Serialize};

use crate::object::anchor_follow::{affine_of, apply_affine, invert_affine, local_nodes};
use crate::object::model::{Geometry, LocalPoint, Object, GEOMETRY_QUANTUM_PER_PX};

/// Quantized units per logical pixel (Q=8); the world<->local map quantizes with
/// the SAME factor as the renderer / anchor reproject.
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// `closed` records filled-interior vs open-stroke; fill/stroke render order
/// keys off it (fill-below-stroke; open => no fill).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub outline: Vec<LocalPoint>,
    pub bounds: LocalBounds,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionError {
    Empty,
    Degenerate,
}

/// The contract every consumer (fill/text/hit-test/anchor) shares. `flatness`
/// is the curve-flattening tolerance in quantized units, varied per zoom bucket.
pub trait OutlineDeriver {
    fn derive_region(&self, geometry: &Geometry, flatness: i32) -> Result<Region, RegionError>;

    fn contains(&self, region: &Region, p: LocalPoint) -> bool {
        point_in_polygon(&region.outline, p)
    }

    /// Re-project an anchor's local point onto a (possibly edited) target region
    /// without drift. Default: nearest outline vertex.
    fn reproject(&self, region: &Region, at: LocalPoint) -> LocalPoint {
        nearest_on_outline(&region.outline, at).unwrap_or(at)
    }
}

/// Reference stub: flattens to subpath node positions (no curve subdivision, no
/// alpha-shape) and computes the AABB.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubOutlineDeriver;

impl OutlineDeriver for StubOutlineDeriver {
    fn derive_region(&self, geometry: &Geometry, _flatness: i32) -> Result<Region, RegionError> {
        let mut outline: Vec<LocalPoint> = Vec::new();
        let mut any_closed = false;
        for sp in &geometry.subpaths {
            any_closed |= sp.closed;
            for n in &sp.nodes {
                outline.push(LocalPoint { x: n.x, y: n.y });
            }
        }
        if outline.is_empty() {
            return Err(RegionError::Empty);
        }
        if outline.len() < 2 {
            return Err(RegionError::Degenerate);
        }
        let (mut min_x, mut min_y) = (outline[0].x, outline[0].y);
        let (mut max_x, mut max_y) = (outline[0].x, outline[0].y);
        for p in &outline[1..] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        Ok(Region {
            outline,
            bounds: LocalBounds { min_x, min_y, max_x, max_y },
            closed: any_closed,
        })
    }
}

/// Even-odd ray-cast point-in-polygon on quantized integer coordinates. i64 for
/// the cross-multiply so an i32*i32 product never overflows (no lossy `as`).
pub fn point_in_polygon(poly: &[LocalPoint], p: LocalPoint) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let (px, py) = (i64::from(p.x), i64::from(p.y));
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (i64::from(poly[i].x), i64::from(poly[i].y));
        let (xj, yj) = (i64::from(poly[j].x), i64::from(poly[j].y));
        if (yi > py) != (yj > py) {
            // Cross-multiplied to stay integer; the `dy = yj - yi` sign flips
            // the comparison (dividing by a negative).
            let dy = yj - yi;
            let lhs = (px - xi) * dy;
            let rhs = (xj - xi) * (py - yi);
            let crosses = if dy > 0 { lhs < rhs } else { lhs > rhs };
            if crosses {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

fn nearest_on_outline(poly: &[LocalPoint], at: LocalPoint) -> Option<LocalPoint> {
    poly.iter().copied().min_by_key(|q| {
        let dx = i64::from(q.x - at.x);
        let dy = i64::from(q.y - at.y);
        dx * dx + dy * dy
    })
}

/// A world-aligned bounding box in logical px (the AABB of the transformed
/// geometry corners).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldAabb {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// Map a WORLD point (logical px) into `object`'s local quantized geometry space:
/// inverse-affine into object-local logical px, then quantize by [`UNITS_PER_PX`]
/// with `round()` — byte-equivalent to the shell's old `worldToObjectLocalQuantized`
/// and to `node_index_nearest_world`'s mapping. `None` when the object's transform
/// is singular (degenerate scale), so no inverse-affine math lives in the shell.
pub fn world_to_local_quantized(object: &Object, wx: f64, wy: f64) -> Option<(i32, i32)> {
    let a = affine_of(&object.transform);
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = invert_affine(&a);
    let (lx, ly) = apply_affine(&inv, wx, wy);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quantize to integer object-local units: .round() then narrow, the canonical de/quantize semantic"
    )]
    let qx = (lx * UNITS_PER_PX).round() as i32;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quantize to integer object-local units: .round() then narrow, the canonical de/quantize semantic"
    )]
    let qy = (ly * UNITS_PER_PX).round() as i32;
    Some((qx, qy))
}

#[cfg(test)]
mod tests {
    use super::{object_world_aabb, world_to_local_quantized};
    use crate::object::model::{
        FillRule, Geometry, Object, PathNode, SubPath, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
    };

    fn rect_object(transform: Transform3x3) -> Object {
        // A 10x6 logical-px rect, local-quantized.
        let q = GEOMETRY_QUANTUM_PER_PX;
        let geometry = Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(10 * q, 0),
                    PathNode::corner(10 * q, 6 * q),
                    PathNode::corner(0, 6 * q),
                ],
            }],
            FillRule::EvenOdd,
        );
        let mut obj = Object::new("r", "a0", geometry);
        obj.transform = transform;
        obj
    }

    // A rotate(90°) + scale(2) transform; det != 0 so it inverts cleanly.
    fn rot90_scale2() -> Transform3x3 {
        // local (x, y) -> world (-2y + 100, 2x + 50): 90° CCW, scale 2, offset.
        Transform3x3 {
            m: [[0.0, -2.0, 100.0], [2.0, 0.0, 50.0], [0.0, 0.0, 1.0]],
        }
    }

    #[test]
    fn world_point_inside_rotated_scaled_object_maps_to_right_local_node() {
        let obj = rect_object(rot90_scale2());
        // Target the node at local px (10, 6) — quantized (80, 48). Its world point is
        // (-2*6 + 100, 2*10 + 50) = (88, 70). A world touch ON it must recover (80, 48),
        // NOT the origin node (0,0) or any other corner.
        let (lx, ly) = world_to_local_quantized(&obj, 88.0, 70.0).expect("invertible");
        assert_eq!((lx, ly), (10 * 8, 6 * 8));
        // The origin corner's world point (100, 50) must map back to (0, 0), proving the
        // inverse is faithful (a wrong inverse would land elsewhere).
        assert_eq!(world_to_local_quantized(&obj, 100.0, 50.0).unwrap(), (0, 0));
    }

    #[test]
    fn world_to_local_quantized_none_on_singular_transform() {
        // Collapse the x-axis: det = 0, not invertible.
        let singular = Transform3x3 {
            m: [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let obj = rect_object(singular);
        assert_eq!(world_to_local_quantized(&obj, 5.0, 5.0), None);
    }

    #[test]
    fn world_aabb_matches_union_of_transformed_corners() {
        let t = rot90_scale2();
        let obj = rect_object(t);
        let aabb = object_world_aabb(&obj).expect("has nodes");
        // Local-px corners (0,0),(10,0),(10,6),(0,6) through world(x,y)=(-2y+100, 2x+50):
        //   (100,50), (100,70), (88,70), (88,50)  => x in [88,100], y in [50,70].
        assert_eq!(aabb.min_x, 88.0);
        assert_eq!(aabb.max_x, 100.0);
        assert_eq!(aabb.min_y, 50.0);
        assert_eq!(aabb.max_y, 70.0);
    }

    #[test]
    fn world_aabb_none_on_empty_geometry() {
        let obj = Object::new("e", "a0", Geometry::default());
        assert_eq!(object_world_aabb(&obj), None);
    }
}

/// The world-space AABB (logical px) of `object`: each geometry node de-quantized
/// to object-local px, carried through the transform's affine, then min/max'd. This
/// is the union of the transformed corners for an axis-aligned object, and the true
/// transformed-node AABB for a rotated/scaled one. `None` when the geometry has no
/// nodes.
pub fn object_world_aabb(object: &Object) -> Option<WorldAabb> {
    let nodes = local_nodes(&object.geometry.path_string);
    if nodes.is_empty() {
        return None;
    }
    let a = affine_of(&object.transform);
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (nx, ny) in nodes {
        let (wx, wy) = apply_affine(&a, nx / UNITS_PER_PX, ny / UNITS_PER_PX);
        min_x = min_x.min(wx);
        min_y = min_y.min(wy);
        max_x = max_x.max(wx);
        max_y = max_y.max(wy);
    }
    if min_x.is_finite() {
        Some(WorldAabb { min_x, min_y, max_x, max_y })
    } else {
        None
    }
}
