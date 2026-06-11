//! OB1.3 — derived outline/region (D6).
//!
//! geometry -> outline/region. Open contour => concave hull / alpha-shape;
//! closed => interior. The single derived region feeds fill area, text layout
//! bounds, hit-test point-in, selection vis, and anchor border (D6). Computed
//! once per geometry edit and cached by (object id, geometry revision) at the
//! consumer. Pure: no IO/time/rng. Coordinates are object-local quantized i32.

use serde::{Deserialize, Serialize};

use crate::object::model::{Geometry, LocalPoint};

/// An axis-aligned bound in object-local quantized units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// The derived "shape" of an object (D6). `outline` is the boundary polygon
/// (flattened contour for closed geometry; concave hull for open). `closed`
/// records whether the source classified as filled-interior vs open-stroke —
/// fill/stroke render order keys off this (D6 fill-below-stroke; open => no
/// fill).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub outline: Vec<LocalPoint>,
    pub bounds: LocalBounds,
    pub closed: bool,
}

/// Errors a region derivation can reject cleanly (degenerate geometry per D2:
/// empty path, single point). Never panics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionError {
    Empty,
    Degenerate,
}

/// The contract every consumer (fill/text/hit-test/anchor) shares. A backend
/// (the renderer's lyon-backed impl, OB3.R4) implements this; scene-core ships
/// the reference stub below. `flatness` is the curve-flattening tolerance in
/// quantized units, varied per zoom bucket by the LOD layer (OB3.R6).
pub trait OutlineDeriver {
    fn derive_region(&self, geometry: &Geometry, flatness: i32) -> Result<Region, RegionError>;

    /// Point-in-region test for hit-testing (D8). Default: even-odd ray cast on
    /// the derived outline.
    fn contains(&self, region: &Region, p: LocalPoint) -> bool {
        point_in_polygon(&region.outline, p)
    }

    /// Re-project an anchor's local point onto a (possibly edited) target region
    /// without drift (D5/OB3.S4). Default: nearest outline vertex.
    fn reproject(&self, region: &Region, at: LocalPoint) -> LocalPoint {
        nearest_on_outline(&region.outline, at).unwrap_or(at)
    }
}

/// Reference stub deriver: flattens to subpath node positions (no curve
/// subdivision, no alpha-shape) and computes the AABB. Correctness placeholder
/// for the OB1.3 contract + OB2.1 rect slice; OB3.R4 replaces with lyon/hull.
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

/// Even-odd ray-cast point-in-polygon on quantized integer coordinates. Uses
/// i64 for the cross-multiply so an i32*i32 product never overflows (no lossy
/// `as`; satisfies workspace `cast_possible_truncation = deny`).
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
            // Ray cast: px < intersection_x of edge (i,j) with the horizontal
            // line y=py. Cross-multiplied to stay integer; the `dy = yj - yi`
            // sign flips the comparison (dividing by a negative).
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

/// Nearest outline vertex to `at` (stub for reproject; OB3.R4 does edge-nearest).
fn nearest_on_outline(poly: &[LocalPoint], at: LocalPoint) -> Option<LocalPoint> {
    poly.iter().copied().min_by_key(|q| {
        let dx = i64::from(q.x - at.x);
        let dy = i64::from(q.y - at.y);
        dx * dx + dy * dy
    })
}
