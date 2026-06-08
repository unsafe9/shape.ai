//! Hit testing against an object's derived region (OB3.R5, D8).
//!
//! D8: a pointer is hit-tested by *inverse-transforming* it into object-local
//! space and running a point-in-region test there, rather than by transforming
//! the region into world space. Inverting the object's 3×3 projective matrix
//! once and pushing the query point through it means rotation, scale, shear, and
//! perspective are all handled by the same arithmetic — there is no special case
//! per transform kind, and the region (D6) stays in the object-local coordinates
//! it was derived/cached in.
//!
//! The transform is a full 3×3 projective matrix `[[f64;3];3]` operating on
//! homogeneous `(x, y, 1)`; the third output row drives the perspective divide.
//! Region outlines are object-local polygons in pixels (f32). Object-local
//! quantized i32 path coordinates convert to f64 px by `/ 8.0` (D2: 8 units/px)
//! before any matrix math.
//!
//! This module is pure CPU and host-neutral: no device, no time, no I/O. It is
//! additive — the legacy `RenderGroup/RenderCard/RenderEdge` draw path and
//! `ViewUniform` are untouched; this is wired into the GPU draw path at the OB-4
//! cutover.

/// Object-local coordinate quantization: path-string integer units per CSS
/// pixel (D2). A quantized `i32` coordinate becomes `f64` px by dividing by this.
pub const UNITS_PER_PX: f64 = 8.0;

/// Selection-handle side length in SCREEN pixels (W2-02/W2-04). Handles are a
/// fixed on-screen size regardless of zoom — the layout helper consumes an
/// already-camera-transformed (screen-space) selection bbox, so this constant is
/// the literal square size the shell draws and the pointer hit-tests against.
pub const HANDLE_SIZE_PX: f64 = 8.0;

/// Distance in SCREEN pixels from the selection's top edge up to the center of
/// the rotate zone (W2-02/W2-04). The rotate zone is a handle-sized square
/// centered above the top edge, used to detect the "rotate" affordance.
pub const ROTATE_ZONE_OFFSET_PX: f64 = 20.0;

/// A hover affordance: what the pointer is currently over, used by the shell to
/// pick a cursor (W2-02). Stable string forms (via [`HoverAffordance::as_str`])
/// are the wire contract the shell reads off the input-batch result; do not
/// rename them without updating the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverAffordance {
    /// Blank canvas — nothing under the pointer.
    Empty,
    /// Over an object's interior/fill.
    Body,
    ResizeNw,
    ResizeN,
    ResizeNe,
    ResizeE,
    ResizeSe,
    ResizeS,
    ResizeSw,
    ResizeW,
    /// Over the rotate zone above the selection's top edge.
    Rotate,
}

impl HoverAffordance {
    /// The stable wire string the shell maps to a cursor. Keep in sync with the
    /// shell's affordance->cursor table (W2-03).
    pub fn as_str(self) -> &'static str {
        match self {
            HoverAffordance::Empty => "empty",
            HoverAffordance::Body => "body",
            HoverAffordance::ResizeNw => "resize-nw",
            HoverAffordance::ResizeN => "resize-n",
            HoverAffordance::ResizeNe => "resize-ne",
            HoverAffordance::ResizeE => "resize-e",
            HoverAffordance::ResizeSe => "resize-se",
            HoverAffordance::ResizeS => "resize-s",
            HoverAffordance::ResizeSw => "resize-sw",
            HoverAffordance::ResizeW => "resize-w",
            HoverAffordance::Rotate => "rotate",
        }
    }
}

/// An axis-aligned rectangle in SCREEN pixels (top-left origin), used for the
/// selection-handle layout. Kept host-neutral (plain `f64`) so the pure hit-test
/// module stays free of the renderer's camera/world types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ScreenRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// Screen-space geometry of a selection's 8 resize handles + rotate zone (W2-02).
/// This is the SINGLE SOURCE OF TRUTH for handle placement: W2-02 builds it to
/// classify the hover affordance over the selected object, and W2-04 reuses the
/// same layout to RENDER the handles and to run the pointer-down hit-test, so the
/// thing the user sees and the thing they can grab are guaranteed identical.
///
/// Each handle is a [`HANDLE_SIZE_PX`]-square centered on a corner / edge-midpoint
/// of `bbox`; the rotate zone is the same-sized square centered
/// [`ROTATE_ZONE_OFFSET_PX`] above the top edge's midpoint.
#[derive(Clone, Copy, Debug)]
pub struct SelectionHandles {
    pub nw: ScreenRect,
    pub n: ScreenRect,
    pub ne: ScreenRect,
    pub e: ScreenRect,
    pub se: ScreenRect,
    pub s: ScreenRect,
    pub sw: ScreenRect,
    pub w: ScreenRect,
    pub rotate: ScreenRect,
}

impl SelectionHandles {
    /// Lay out the 8 resize handles + rotate zone around a screen-space selection
    /// `bbox` (already camera-transformed; pass `world_rect_to_screen_rect` output).
    pub fn from_screen_bbox(bbox: &ScreenRect) -> Self {
        let half = HANDLE_SIZE_PX / 2.0;
        // A handle square centered on (cx, cy).
        let at = |cx: f64, cy: f64| ScreenRect {
            x: cx - half,
            y: cy - half,
            width: HANDLE_SIZE_PX,
            height: HANDLE_SIZE_PX,
        };
        let left = bbox.x;
        let right = bbox.x + bbox.width;
        let top = bbox.y;
        let bottom = bbox.y + bbox.height;
        let cx = bbox.x + bbox.width / 2.0;
        let cy = bbox.y + bbox.height / 2.0;
        SelectionHandles {
            nw: at(left, top),
            n: at(cx, top),
            ne: at(right, top),
            e: at(right, cy),
            se: at(right, bottom),
            s: at(cx, bottom),
            sw: at(left, bottom),
            w: at(left, cy),
            rotate: at(cx, top - ROTATE_ZONE_OFFSET_PX),
        }
    }

    /// Classify a screen point against the handles. Returns the resize/rotate
    /// affordance the point falls in, or `None` when it is over no handle. The
    /// rotate zone is tested first so it wins over a corner only where they do not
    /// overlap (the offset keeps them apart for any non-degenerate selection).
    pub fn affordance_at(&self, x: f64, y: f64) -> Option<HoverAffordance> {
        if self.rotate.contains(x, y) {
            return Some(HoverAffordance::Rotate);
        }
        let table = [
            (&self.nw, HoverAffordance::ResizeNw),
            (&self.ne, HoverAffordance::ResizeNe),
            (&self.se, HoverAffordance::ResizeSe),
            (&self.sw, HoverAffordance::ResizeSw),
            (&self.n, HoverAffordance::ResizeN),
            (&self.e, HoverAffordance::ResizeE),
            (&self.s, HoverAffordance::ResizeS),
            (&self.w, HoverAffordance::ResizeW),
        ];
        table
            .iter()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, affordance)| *affordance)
    }
}

/// Convert a quantized object-local `i32` coordinate to `f64` pixels.
///
/// `f64` exactly represents every `i32`, so this is lossless; the explicit
/// helper keeps the `8 units/px` convention in one place instead of scattering
/// `as f64 / 8.0` across callers.
pub fn quantized_to_px(units: i32) -> f64 {
    f64::from(units) / UNITS_PER_PX
}

/// Invert a 3×3 matrix via the adjugate / determinant. Returns `None` when the
/// matrix is singular (determinant ≈ 0), which is exactly the case where no
/// well-defined object-local preimage of a world point exists.
///
/// `m[row][col]`: row-major, so `m[i]` is the `i`-th row. The result satisfies
/// `m * inv(m) ≈ identity` for any non-singular `m`.
pub fn invert_3x3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    // Cofactors of the first row give the determinant by expansion.
    let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
    let c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
    let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];

    let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
    if !det.is_finite() || det.abs() < f64::EPSILON {
        return None;
    }
    let inv_det = 1.0 / det;

    // Remaining cofactors. The inverse is the transpose of the cofactor matrix
    // (the adjugate) scaled by 1/det — note the [col][row] placement below.
    let c10 = m[0][2] * m[2][1] - m[0][1] * m[2][2];
    let c11 = m[0][0] * m[2][2] - m[0][2] * m[2][0];
    let c12 = m[0][1] * m[2][0] - m[0][0] * m[2][1];

    let c20 = m[0][1] * m[1][2] - m[0][2] * m[1][1];
    let c21 = m[0][2] * m[1][0] - m[0][0] * m[1][2];
    let c22 = m[0][0] * m[1][1] - m[0][1] * m[1][0];

    let inv = [
        [c00 * inv_det, c10 * inv_det, c20 * inv_det],
        [c01 * inv_det, c11 * inv_det, c21 * inv_det],
        [c02 * inv_det, c12 * inv_det, c22 * inv_det],
    ];
    if inv.iter().flatten().all(|v| v.is_finite()) {
        Some(inv)
    } else {
        None
    }
}

/// Apply a 3×3 projective matrix to a 2D point, dividing through by the
/// homogeneous `w` component. Returns the projected `(x, y)`; when `w` is zero
/// (point at infinity) the components are `±inf`/`NaN`, which callers detect via
/// the `None` paths in [`world_to_local`].
pub fn apply_3x3(m: &[[f64; 3]; 3], x: f64, y: f64) -> (f64, f64) {
    let ox = m[0][0] * x + m[0][1] * y + m[0][2];
    let oy = m[1][0] * x + m[1][1] * y + m[1][2];
    let ow = m[2][0] * x + m[2][1] * y + m[2][2];
    (ox / ow, oy / ow)
}

/// Map a world (or screen-pre-camera) point into the object's local space by
/// inverting its transform and projecting the point through the inverse
/// (D8). Returns `None` when the transform is singular or the result is not
/// finite (e.g. the point maps to the transform's vanishing line).
pub fn world_to_local(transform: &[[f64; 3]; 3], wx: f64, wy: f64) -> Option<(f64, f64)> {
    let inv = invert_3x3(transform)?;
    let (lx, ly) = apply_3x3(&inv, wx, wy);
    if lx.is_finite() && ly.is_finite() {
        Some((lx, ly))
    } else {
        None
    }
}

/// Even-odd ray-cast point-in-polygon test. `poly` is a closed ring given as an
/// ordered vertex list (the closing edge from the last vertex back to the first
/// is implicit). Returns `true` when `(x, y)` is inside.
///
/// The crossing test compares the query `y` against each edge's endpoints using
/// a strict-vs-inclusive pair (`(yi > y) != (yj > y)`) so a vertex shared by two
/// edges is counted exactly once, avoiding the classic double-count / sign bug
/// at horizontal extents. The x-intersection is computed in `f64` to keep the
/// comparison robust for near-horizontal edges.
pub fn point_in_polygon(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let px = f64::from(x);
    let py = f64::from(y);
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (f64::from(poly[i].0), f64::from(poly[i].1));
        let (xj, yj) = (f64::from(poly[j].0), f64::from(poly[j].1));
        // Does a ray going in +x from (px, py) cross edge (j -> i)?
        let crosses = (yi > py) != (yj > py)
            && px < (xj - xi) * (py - yi) / (yj - yi) + xi;
        if crosses {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Hit-test a world point against one object: invert its transform to reach
/// object-local space (D8), then run an even-odd point-in-region test against
/// the object's derived region outline (D6). Returns `false` when the transform
/// is non-invertible (the point has no local preimage) — a degenerate object
/// cannot be hit.
pub fn hit_test_object(
    transform: &[[f64; 3]; 3],
    region_outline: &[(f32, f32)],
    world_x: f64,
    world_y: f64,
) -> bool {
    let Some((lx, ly)) = world_to_local(transform, world_x, world_y) else {
        return false;
    };
    point_in_polygon(region_outline, lx as f32, ly as f32)
}

/// One command in a parsed object-local geometry path (D2 SVG-subset):
/// `M`/`L`/`C`/`Z`. Coordinates are object-local pixels (already de-quantized
/// from the `i32` 8-units/px encoding). `C` carries absolute control points,
/// matching the encoding (handles are stored relative to nodes but a `C`
/// command is written with absolute control points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSeg {
    MoveTo { x: f64, y: f64 },
    LineTo { x: f64, y: f64 },
    CurveTo {
        c1x: f64,
        c1y: f64,
        c2x: f64,
        c2y: f64,
        x: f64,
        y: f64,
    },
    Close,
}

/// Parse a small SVG-subset path string (`M`/`L`/`C`/`Z`, absolute coords,
/// multi-subpath) into a flat command list. This is a deliberately local parser
/// so the renderer crate stays standalone (no scene-core dependency); it mirrors
/// the geometry encoding: absolute integer coordinates, `C` with absolute
/// control points, multiple subpaths in one string.
///
/// The input coordinates are the quantized `i32` units of the encoding; each is
/// de-quantized to pixels via [`quantized_to_px`] as it is parsed. Returns
/// `None` on any malformed command (unknown verb, short coordinate run, or a
/// non-integer token).
pub fn parse_path(input: &str) -> Option<Vec<PathSeg>> {
    let mut tokens = input.split_whitespace().peekable();
    let mut out = Vec::new();

    let next_coord = |tokens: &mut std::iter::Peekable<std::str::SplitWhitespace>| -> Option<f64> {
        let raw: i32 = tokens.next()?.parse().ok()?;
        Some(quantized_to_px(raw))
    };

    while let Some(verb) = tokens.next() {
        match verb {
            "M" => {
                let x = next_coord(&mut tokens)?;
                let y = next_coord(&mut tokens)?;
                out.push(PathSeg::MoveTo { x, y });
            }
            "L" => {
                let x = next_coord(&mut tokens)?;
                let y = next_coord(&mut tokens)?;
                out.push(PathSeg::LineTo { x, y });
            }
            "C" => {
                let c1x = next_coord(&mut tokens)?;
                let c1y = next_coord(&mut tokens)?;
                let c2x = next_coord(&mut tokens)?;
                let c2y = next_coord(&mut tokens)?;
                let x = next_coord(&mut tokens)?;
                let y = next_coord(&mut tokens)?;
                out.push(PathSeg::CurveTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                });
            }
            "Z" | "z" => out.push(PathSeg::Close),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    /// Unit square `[0,1]×[0,1]` as a closed ring, in object-local px.
    fn unit_rect() -> Vec<(f32, f32)> {
        vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
    }

    fn approx_eq_mat(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3], eps: f64) -> bool {
        a.iter()
            .zip(b.iter())
            .all(|(ra, rb)| ra.iter().zip(rb.iter()).all(|(x, y)| (x - y).abs() < eps))
    }

    #[test]
    fn point_in_polygon_basic_inside_outside() {
        let rect = unit_rect();
        assert!(point_in_polygon(&rect, 0.5, 0.5));
        assert!(!point_in_polygon(&rect, 1.5, 0.5));
        assert!(!point_in_polygon(&rect, -0.5, 0.5));
        assert!(!point_in_polygon(&rect, 0.5, 2.0));
    }

    #[test]
    fn point_in_polygon_concave_notch_is_excluded() {
        // An arrow-like concave polygon; the notch region must read as outside.
        let poly = vec![
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (2.0, 2.0), // inward notch apex
            (0.0, 4.0),
        ];
        assert!(point_in_polygon(&poly, 2.0, 0.5)); // body
        assert!(!point_in_polygon(&poly, 2.0, 3.5)); // inside the notch
    }

    #[test]
    fn point_in_polygon_degenerate_ring_is_miss() {
        assert!(!point_in_polygon(&[], 0.0, 0.0));
        assert!(!point_in_polygon(&[(0.0, 0.0), (1.0, 1.0)], 0.5, 0.5));
    }

    #[test]
    fn identity_transform_rect_hit_and_miss() {
        let rect = unit_rect();
        assert!(hit_test_object(&IDENTITY, &rect, 0.25, 0.75));
        assert!(!hit_test_object(&IDENTITY, &rect, 2.0, 0.5));
    }

    #[test]
    fn translated_scaled_transform_maps_world_into_local_rect() {
        // Object placed at world origin (10, 20) and scaled ×4. A point inside
        // the on-screen rect must map back into the local unit rect and hit; a
        // point just outside must miss.
        let transform = [[4.0, 0.0, 10.0], [0.0, 4.0, 20.0], [0.0, 0.0, 1.0]];

        // World (12, 22) -> local (0.5, 0.5): inside.
        assert!(hit_test_object(&transform, &unit_rect(), 12.0, 22.0));
        // World (15, 22) -> local (1.25, 0.5): outside the unit rect.
        assert!(!hit_test_object(&transform, &unit_rect(), 15.0, 22.0));
        // The corner just inside the far edge still hits.
        assert!(hit_test_object(&transform, &unit_rect(), 13.9, 23.9));
    }

    #[test]
    fn rotated_transform_hits_through_inverse() {
        // 90° rotation about the origin: world (x, y) <- local (-y, x), so the
        // local unit rect occupies world x in [-1, 0], y in [0, 1].
        let transform = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        assert!(hit_test_object(&transform, &unit_rect(), -0.5, 0.5));
        assert!(!hit_test_object(&transform, &unit_rect(), 0.5, 0.5));
    }

    #[test]
    fn invert_3x3_round_trips_to_identity() {
        // A transform with translation, scale, shear, and a perspective row.
        let m = [
            [2.0, 0.5, 7.0],
            [-1.0, 3.0, -4.0],
            [0.001, 0.002, 1.0],
        ];
        let inv = invert_3x3(&m).expect("non-singular");

        // m * inv ≈ identity.
        let mut prod = [[0.0; 3]; 3];
        for (i, prow) in prod.iter_mut().enumerate() {
            for (j, cell) in prow.iter_mut().enumerate() {
                *cell = (0..3).map(|k| m[i][k] * inv[k][j]).sum();
            }
        }
        assert!(
            approx_eq_mat(&prod, &IDENTITY, 1e-9),
            "m * inv(m) should be identity, got {prod:?}"
        );
    }

    #[test]
    fn invert_3x3_singular_returns_none() {
        // Second row is twice the first -> determinant zero.
        let singular = [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]];
        assert!(invert_3x3(&singular).is_none());
    }

    #[test]
    fn world_to_local_rejects_singular_transform() {
        let singular = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        assert!(world_to_local(&singular, 1.0, 1.0).is_none());
    }

    #[test]
    fn apply_3x3_perspective_divide() {
        // A pure perspective row halves w at x=1, doubling the projected x.
        let m = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0]];
        let (px, py) = apply_3x3(&m, 1.0, 4.0);
        // w = 1*1 + 0 + 1 = 2; x = 1/2, y = 4/2.
        assert!((px - 0.5).abs() < 1e-12);
        assert!((py - 2.0).abs() < 1e-12);
    }

    #[test]
    fn parse_path_dequantizes_and_handles_subpaths() {
        // 8 units/px: "8 8" -> (1.0, 1.0). Two subpaths, second closed.
        let segs = parse_path("M 0 0 L 8 0 L 8 8 Z M 16 16 C 24 16 24 24 16 24 Z")
            .expect("parses");
        assert_eq!(
            segs[0],
            PathSeg::MoveTo { x: 0.0, y: 0.0 }
        );
        assert_eq!(segs[1], PathSeg::LineTo { x: 1.0, y: 0.0 });
        assert_eq!(segs[3], PathSeg::Close);
        assert_eq!(
            segs[5],
            PathSeg::CurveTo {
                c1x: 3.0,
                c1y: 2.0,
                c2x: 3.0,
                c2y: 3.0,
                x: 2.0,
                y: 3.0,
            }
        );
    }

    #[test]
    fn parse_path_rejects_malformed() {
        assert!(parse_path("M 0").is_none()); // short coordinate run
        assert!(parse_path("Q 0 0").is_none()); // unsupported verb
        assert!(parse_path("M 0 x").is_none()); // non-integer token
    }

    #[test]
    fn hover_affordance_strings_are_stable() {
        // The shell reads these exact strings; pin them.
        assert_eq!(HoverAffordance::Empty.as_str(), "empty");
        assert_eq!(HoverAffordance::Body.as_str(), "body");
        assert_eq!(HoverAffordance::ResizeNw.as_str(), "resize-nw");
        assert_eq!(HoverAffordance::ResizeN.as_str(), "resize-n");
        assert_eq!(HoverAffordance::ResizeNe.as_str(), "resize-ne");
        assert_eq!(HoverAffordance::ResizeE.as_str(), "resize-e");
        assert_eq!(HoverAffordance::ResizeSe.as_str(), "resize-se");
        assert_eq!(HoverAffordance::ResizeS.as_str(), "resize-s");
        assert_eq!(HoverAffordance::ResizeSw.as_str(), "resize-sw");
        assert_eq!(HoverAffordance::ResizeW.as_str(), "resize-w");
        assert_eq!(HoverAffordance::Rotate.as_str(), "rotate");
    }

    #[test]
    fn selection_handles_classify_corners_edges_and_rotate() {
        // A 100×80 selection at (10, 20) in screen px.
        let bbox = ScreenRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 80.0,
        };
        let handles = SelectionHandles::from_screen_bbox(&bbox);

        // Corners (handles are centered on the bbox corners).
        assert_eq!(
            handles.affordance_at(10.0, 20.0),
            Some(HoverAffordance::ResizeNw)
        );
        assert_eq!(
            handles.affordance_at(110.0, 100.0),
            Some(HoverAffordance::ResizeSe)
        );
        // Edge midpoints.
        assert_eq!(
            handles.affordance_at(60.0, 20.0),
            Some(HoverAffordance::ResizeN)
        );
        assert_eq!(
            handles.affordance_at(110.0, 60.0),
            Some(HoverAffordance::ResizeE)
        );
        // Rotate zone: centered ROTATE_ZONE_OFFSET_PX above the top-edge midpoint.
        assert_eq!(
            handles.affordance_at(60.0, 20.0 - ROTATE_ZONE_OFFSET_PX),
            Some(HoverAffordance::Rotate)
        );
        // Interior and far-away points are over no handle.
        assert_eq!(handles.affordance_at(60.0, 60.0), None);
        assert_eq!(handles.affordance_at(500.0, 500.0), None);
    }

    #[test]
    fn selection_handle_size_is_fixed_screen_px() {
        let bbox = ScreenRect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
        };
        let handles = SelectionHandles::from_screen_bbox(&bbox);
        // Each handle is a HANDLE_SIZE_PX square, centered on its anchor.
        assert_eq!(handles.nw.width, HANDLE_SIZE_PX);
        assert_eq!(handles.nw.height, HANDLE_SIZE_PX);
        assert_eq!(handles.nw.x, -HANDLE_SIZE_PX / 2.0);
        assert_eq!(handles.nw.y, -HANDLE_SIZE_PX / 2.0);
    }
}
