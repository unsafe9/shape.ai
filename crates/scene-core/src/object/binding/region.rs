//! Derived outline/region: geometry -> outline/region. Pure (no IO/time/rng);
//! coordinates are object-local quantized i32. The consumer caches by
//! (object id, geometry revision).

use serde::{Deserialize, Serialize};

use crate::object::model::{Geometry, LocalPoint};

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
