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
    /// Anchor-semantics v3 §2b: over an OPEN-CLASS selection's START endpoint
    /// handle (geometry pair 0). Open-class selections surface only the two
    /// endpoint handles — no resize/rotate affordances.
    EndpointStart,
    /// v3 §2b: over an OPEN-CLASS selection's END endpoint handle (the last
    /// geometry coordinate pair).
    EndpointEnd,
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
            HoverAffordance::EndpointStart => "endpoint-start",
            HoverAffordance::EndpointEnd => "endpoint-end",
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
    pub fn contains(&self, x: f64, y: f64) -> bool {
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

/// Multiply two row-major 3×3 matrices: `a * b`. Allocation-free fixed array.
/// Used to compose a gesture's DELTA matrix to the LEFT of the object's existing
/// world transform (W2-04): `new = delta * obj.transform`.
pub fn mat3_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, orow) in out.iter_mut().enumerate() {
        for (j, cell) in orow.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// Row-major translation matrix by `(dx, dy)`.
pub fn translate_3x3(dx: f64, dy: f64) -> [[f64; 3]; 3] {
    [[1.0, 0.0, dx], [0.0, 1.0, dy], [0.0, 0.0, 1.0]]
}

/// Row-major scale by `(sx, sy)` about the anchor `(ax, ay)`:
/// `T(ax,ay) * S(sx,sy) * T(-ax,-ay)`, so the anchor point is fixed.
pub fn scale_about_3x3(sx: f64, sy: f64, ax: f64, ay: f64) -> [[f64; 3]; 3] {
    [
        [sx, 0.0, ax - sx * ax],
        [0.0, sy, ay - sy * ay],
        [0.0, 0.0, 1.0],
    ]
}

/// Row-major rotation by `theta` (radians, CCW in the +y-down screen frame) about
/// the center `(cx, cy)`: `T(c) * R(theta) * T(-c)`.
pub fn rotate_about_3x3(theta: f64, cx: f64, cy: f64) -> [[f64; 3]; 3] {
    let (s, c) = theta.sin_cos();
    [
        [c, -s, cx - c * cx + s * cy],
        [s, c, cy - s * cx - c * cy],
        [0.0, 0.0, 1.0],
    ]
}

/// W2-04 resize delta: a scale about the OPPOSITE anchor of the dragged handle
/// (drag NE -> anchor SW). `world_bbox` is `(min_x, min_y, max_x, max_y)` in WORLD
/// px; `corner` is the grabbed handle; `world_now`/`world_start` are the live and
/// pointer-down WORLD points. Edge handles gate to one axis (N/S keep `sx=1`, E/W
/// keep `sy=1`). A degenerate start extent yields scale 1 on that axis (no NaN).
///
/// Returns the identity matrix for a non-resize `corner` so callers can route the
/// drag kind through one function.
pub fn resize_delta_matrix(
    world_bbox: (f64, f64, f64, f64),
    corner: HoverAffordance,
    world_now: (f64, f64),
    world_start: (f64, f64),
) -> [[f64; 3]; 3] {
    let (min_x, min_y, max_x, max_y) = world_bbox;
    // Anchor = the OPPOSITE corner/edge; scale_x/scale_y gate which axes move.
    let (ax, ay, scale_x, scale_y) = match corner {
        HoverAffordance::ResizeNw => (max_x, max_y, true, true),
        HoverAffordance::ResizeNe => (min_x, max_y, true, true),
        HoverAffordance::ResizeSe => (min_x, min_y, true, true),
        HoverAffordance::ResizeSw => (max_x, min_y, true, true),
        HoverAffordance::ResizeN => (min_x, max_y, false, true),
        HoverAffordance::ResizeS => (min_x, min_y, false, true),
        HoverAffordance::ResizeE => (min_x, min_y, true, false),
        HoverAffordance::ResizeW => (max_x, min_y, true, false),
        _ => return identity_3x3(),
    };
    let sx = if scale_x {
        axis_scale(world_start.0, world_now.0, ax)
    } else {
        1.0
    };
    let sy = if scale_y {
        axis_scale(world_start.1, world_now.1, ay)
    } else {
        1.0
    };
    scale_about_3x3(sx, sy, ax, ay)
}

/// Per-axis scale factor from the anchor: how much the pointer's distance to the
/// anchor changed between the start and now. A near-zero start extent (pointer
/// grabbed at the anchor) returns 1 to avoid a divide-by-zero blow-up.
fn axis_scale(start: f64, now: f64, anchor: f64) -> f64 {
    let start_extent = start - anchor;
    if start_extent.abs() < f64::EPSILON {
        return 1.0;
    }
    (now - anchor) / start_extent
}

/// W2-04 rotate delta: rotate about the bbox `center` by the angle swept from the
/// pointer-down point to the live point (both WORLD px). `theta = atan2(now-c) -
/// atan2(start-c)`.
pub fn rotate_delta_matrix(
    center: (f64, f64),
    world_now: (f64, f64),
    world_start: (f64, f64),
) -> [[f64; 3]; 3] {
    let a_now = (world_now.1 - center.1).atan2(world_now.0 - center.0);
    let a_start = (world_start.1 - center.1).atan2(world_start.0 - center.0);
    rotate_about_3x3(a_now - a_start, center.0, center.1)
}

/// Round `theta` (radians) to the nearest multiple of `snap_deg` (degrees). The
/// coarse-rotate gesture (C2 `coarse-rotate-shift`, 15°) passes `snap_deg = 15`.
pub fn snap_angle(theta: f64, snap_deg: f64) -> f64 {
    let step = snap_deg.to_radians();
    (theta / step).round() * step
}

/// D5 coarse-rotate variant of [`rotate_delta_matrix`]: when `snap_deg` is
/// `Some`, the swept delta theta is snapped to the nearest `snap_deg` increment
/// before building the rotation; when `None`, behaves identically to
/// [`rotate_delta_matrix`].
pub fn rotate_delta_matrix_snapped(
    center: (f64, f64),
    world_now: (f64, f64),
    world_start: (f64, f64),
    snap_deg: Option<f64>,
) -> [[f64; 3]; 3] {
    let a_now = (world_now.1 - center.1).atan2(world_now.0 - center.0);
    let a_start = (world_start.1 - center.1).atan2(world_start.0 - center.0);
    let theta = a_now - a_start;
    let theta = match snap_deg {
        Some(deg) => snap_angle(theta, deg),
        None => theta,
    };
    rotate_about_3x3(theta, center.0, center.1)
}

/// The row-major 3×3 identity.
pub fn identity_3x3() -> [[f64; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
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

/// Object-local axis-aligned bounding box of an outline as `(min_x, min_y,
/// max_x, max_y)`. `None` when the outline is empty or has no finite vertex.
/// Used for the stroke/text/open/zero-size body grab fallback (RA3).
pub fn outline_local_bbox(outline: &[(f32, f32)]) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &(x, y) in outline {
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if min_x.is_finite() && max_x >= min_x && max_y >= min_y {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

/// RA3 body hit with a bbox fallback for objects with no closed fill polygon.
///
/// A FILLED object (`closed == true`, a real ≥3-vertex polygon) keeps the
/// even-odd fill hit, so empty canvas and a concave notch still MISS the body
/// and fall through to the marquee. A stroke / text / open / zero-size object
/// has no closed fill region; its even-odd test always misses (an open hull or
/// a degenerate ring), making it ungrabbable. For those, fall back to the
/// object-LOCAL bounding box of the outline (a zero-size bbox is grown by
/// `bbox_pad_px` on each side so a collapsed object is still a finite target).
///
/// `bbox_pad_px` is an OBJECT-LOCAL pad; callers pass a screen-pixel grab radius
/// already de-scaled to local units (or `0.0` to use the raw bbox).
pub fn hit_test_object_or_bbox(
    transform: &[[f64; 3]; 3],
    region_outline: &[(f32, f32)],
    closed: bool,
    bbox_pad_px: f32,
    world_x: f64,
    world_y: f64,
) -> bool {
    let Some((lx, ly)) = world_to_local(transform, world_x, world_y) else {
        return false;
    };
    let (lx, ly) = (lx as f32, ly as f32);
    if closed && point_in_polygon(region_outline, lx, ly) {
        return true;
    }
    if closed {
        return false;
    }
    match outline_local_bbox(region_outline) {
        Some((min_x, min_y, max_x, max_y)) => {
            lx >= min_x - bbox_pad_px
                && lx <= max_x + bbox_pad_px
                && ly >= min_y - bbox_pad_px
                && ly <= max_y + bbox_pad_px
        }
        None => false,
    }
}

/// Do the two segments `p1->p2` and `p3->p4` intersect (including touching at an
/// endpoint)? Uses the orientation / cross-product test; handles the collinear
/// overlap case via bounding-box containment of the touching point. Pure `f32`.
fn segments_intersect(
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    p4: (f32, f32),
) -> bool {
    let orient = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| -> f32 {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let on_segment = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| -> bool {
        // c is collinear with a-b; is it within the segment's bbox?
        c.0 >= a.0.min(b.0) && c.0 <= a.0.max(b.0) && c.1 >= a.1.min(b.1) && c.1 <= a.1.max(b.1)
    };
    let d1 = orient(p3, p4, p1);
    let d2 = orient(p3, p4, p2);
    let d3 = orient(p1, p2, p3);
    let d4 = orient(p1, p2, p4);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(p3, p4, p1))
        || (d2 == 0.0 && on_segment(p3, p4, p2))
        || (d3 == 0.0 && on_segment(p1, p2, p3))
        || (d4 == 0.0 && on_segment(p1, p2, p4))
}

/// Does the local-space segment `(ax, ay)->(bx, by)` cross / touch the polygon
/// `poly` (a closed ring; implicit closing edge included)? True when either
/// endpoint is inside the ring (even-odd) or the segment intersects any edge.
/// `poly` shorter than a triangle is treated as edge-only (no interior).
pub fn segment_hits_polygon(
    poly: &[(f32, f32)],
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
) -> bool {
    if point_in_polygon(poly, ax, ay) || point_in_polygon(poly, bx, by) {
        return true;
    }
    let n = poly.len();
    if n < 2 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        if segments_intersect((ax, ay), (bx, by), poly[j], poly[i]) {
            return true;
        }
        j = i;
    }
    false
}

/// Does the local-space segment `(ax, ay)->(bx, by)` cross / touch the
/// axis-aligned bbox `(min_x, min_y, max_x, max_y)`? True when either endpoint is
/// inside the bbox or the segment crosses any of its four edges. Used for the RA3
/// stroke/text/open/zero-size swept-erase fallback.
pub fn segment_hits_bbox(
    bbox: (f32, f32, f32, f32),
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
) -> bool {
    let (min_x, min_y, max_x, max_y) = bbox;
    let inside = |x: f32, y: f32| x >= min_x && x <= max_x && y >= min_y && y <= max_y;
    if inside(ax, ay) || inside(bx, by) {
        return true;
    }
    let corners = [
        (min_x, min_y),
        (max_x, min_y),
        (max_x, max_y),
        (min_x, max_y),
    ];
    let mut j = 3;
    for i in 0..4 {
        if segments_intersect((ax, ay), (bx, by), corners[j], corners[i]) {
            return true;
        }
        j = i;
    }
    false
}

/// RA3 swept hit-test for ONE object: does the WORLD-space segment
/// `(world_ax, world_ay)->(world_bx, world_by)` cross / touch the object? Both
/// segment endpoints are inverse-transformed into object-LOCAL space (D8) and the
/// crossing is tested there against the local outline (filled) or its local bbox
/// (stroke / text / open / zero-size, padded by `bbox_pad_px`). Returns `false`
/// when the transform is non-invertible (no local preimage of either endpoint).
///
/// This is the per-object kernel a region-level swept-erase loop calls; it
/// catches every object the segment passes through between two pointer samples,
/// not just whichever is top-most at the endpoints.
pub fn swept_segment_hits_object(
    transform: &[[f64; 3]; 3],
    region_outline: &[(f32, f32)],
    closed: bool,
    bbox_pad_px: f32,
    world_ax: f64,
    world_ay: f64,
    world_bx: f64,
    world_by: f64,
) -> bool {
    let Some((lax, lay)) = world_to_local(transform, world_ax, world_ay) else {
        return false;
    };
    let Some((lbx, lby)) = world_to_local(transform, world_bx, world_by) else {
        return false;
    };
    let (lax, lay, lbx, lby) = (lax as f32, lay as f32, lbx as f32, lby as f32);
    if closed {
        return segment_hits_polygon(region_outline, lax, lay, lbx, lby);
    }
    match outline_local_bbox(region_outline) {
        Some((min_x, min_y, max_x, max_y)) => segment_hits_bbox(
            (
                min_x - bbox_pad_px,
                min_y - bbox_pad_px,
                max_x + bbox_pad_px,
                max_y + bbox_pad_px,
            ),
            lax,
            lay,
            lbx,
            lby,
        ),
        None => false,
    }
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
        assert_eq!(HoverAffordance::EndpointStart.as_str(), "endpoint-start");
        assert_eq!(HoverAffordance::EndpointEnd.as_str(), "endpoint-end");
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
    fn translate_matrix_is_pure_translation() {
        let m = translate_3x3(8.0, 3.0);
        assert!(approx_eq_mat(
            &m,
            &[[1.0, 0.0, 8.0], [0.0, 1.0, 3.0], [0.0, 0.0, 1.0]],
            1e-12
        ));
    }

    #[test]
    fn scale_about_keeps_anchor_fixed() {
        // Scale 2x about (10, 20): the anchor maps to itself; (11,21) -> (12,22).
        let m = scale_about_3x3(2.0, 2.0, 10.0, 20.0);
        let (ax, ay) = apply_3x3(&m, 10.0, 20.0);
        assert!((ax - 10.0).abs() < 1e-12 && (ay - 20.0).abs() < 1e-12);
        let (px, py) = apply_3x3(&m, 11.0, 21.0);
        assert!((px - 12.0).abs() < 1e-12 && (py - 22.0).abs() < 1e-12);
    }

    #[test]
    fn resize_ne_drag_scales_about_sw_anchor() {
        // 100x100 bbox at world origin: (min,min,max,max) = (0,0,100,100).
        // Grab NE (top-right) at start (100, 0); drag to (200, -100) so the box's
        // top-right doubles its distance from the SW anchor (0, 100): width 100->200,
        // height 100->200 => scale 2x about (0, 100).
        let bbox = (0.0, 0.0, 100.0, 100.0);
        let m = resize_delta_matrix(
            bbox,
            HoverAffordance::ResizeNe,
            (200.0, -100.0),
            (100.0, 0.0),
        );
        let expected = scale_about_3x3(2.0, 2.0, 0.0, 100.0);
        assert!(
            approx_eq_mat(&m, &expected, 1e-9),
            "NE drag should scale 2x about the SW anchor, got {m:?}"
        );
        // The SW anchor is fixed; the dragged NE corner lands on the pointer.
        let (ax, ay) = apply_3x3(&m, 0.0, 100.0);
        assert!((ax - 0.0).abs() < 1e-9 && (ay - 100.0).abs() < 1e-9);
        let (nx, ny) = apply_3x3(&m, 100.0, 0.0);
        assert!((nx - 200.0).abs() < 1e-9 && (ny + 100.0).abs() < 1e-9);
    }

    #[test]
    fn resize_edge_handle_gates_to_one_axis() {
        // East edge handle scales only x (sy = 1). Anchor is the west edge (min_x).
        let bbox = (0.0, 0.0, 100.0, 100.0);
        let m = resize_delta_matrix(bbox, HoverAffordance::ResizeE, (200.0, 999.0), (100.0, 50.0));
        let expected = scale_about_3x3(2.0, 1.0, 0.0, 0.0);
        assert!(approx_eq_mat(&m, &expected, 1e-9), "E drag scales x only");
    }

    #[test]
    fn resize_degenerate_start_extent_is_identity_scale() {
        // Grabbing exactly at the anchor (zero start extent) must not divide by zero;
        // the axis stays at scale 1.
        let bbox = (0.0, 0.0, 100.0, 100.0);
        // SE anchor is NW (0,0); start the pointer AT the anchor on x.
        let m = resize_delta_matrix(bbox, HoverAffordance::ResizeSe, (50.0, 50.0), (0.0, 50.0));
        // x axis: start_extent 0 -> sx = 1; y axis: (50-0)/(50-0) = 1.
        assert!(approx_eq_mat(&m, &identity_3x3(), 1e-9));
    }

    #[test]
    fn rotate_delta_is_rotation_about_center_by_swept_angle() {
        // Center (0,0); pointer-down at (10, 0) (angle 0), now at (0, 10) (angle +pi/2
        // in the +y-down frame). theta = pi/2.
        let m = rotate_delta_matrix((0.0, 0.0), (0.0, 10.0), (10.0, 0.0));
        let expected = rotate_about_3x3(std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        assert!(
            approx_eq_mat(&m, &expected, 1e-9),
            "90deg pointer sweep should rotate by pi/2, got {m:?}"
        );
    }

    #[test]
    fn coarse_rotate_snaps_swept_delta_to_15deg_increments() {
        // Center (0,0); start at angle 0 (1,0). `now` at angle `d` degrees gives a
        // swept delta of exactly `d`. Expected matrix = rotation by the snapped (or
        // raw) angle about the center.
        let center = (0.0, 0.0);
        let start = (1.0, 0.0);
        let point_at = |deg: f64| (deg.to_radians().cos(), deg.to_radians().sin());
        let expect = |deg: f64| rotate_about_3x3(deg.to_radians(), center.0, center.1);

        // snap ON: 47->45, 7->0, 83->90.
        for (raw, snapped) in [(47.0, 45.0), (7.0, 0.0), (83.0, 90.0)] {
            let m =
                rotate_delta_matrix_snapped(center, point_at(raw), start, Some(15.0));
            assert!(
                approx_eq_mat(&m, &expect(snapped), 1e-9),
                "{raw}deg swept w/ 15deg snap should rotate by {snapped}deg, got {m:?}"
            );
        }

        // snap OFF: 47 stays 47, identical to the unsnapped fn.
        let m = rotate_delta_matrix_snapped(center, point_at(47.0), start, None);
        assert!(approx_eq_mat(&m, &expect(47.0), 1e-9), "no snap -> raw 47deg");
        let unsnapped = rotate_delta_matrix(center, point_at(47.0), start);
        assert!(
            approx_eq_mat(&m, &unsnapped, 1e-12),
            "None snap must equal rotate_delta_matrix exactly"
        );
    }

    #[test]
    fn mat3_mul_premultiplies_delta_onto_transform() {
        // new = delta * obj: a translate delta pre-multiplied onto a scale transform
        // applies the scale first, then the translation.
        let obj = [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]];
        let delta = translate_3x3(5.0, 7.0);
        let m = mat3_mul(&delta, &obj);
        let expected = [[2.0, 0.0, 5.0], [0.0, 2.0, 7.0], [0.0, 0.0, 1.0]];
        assert!(approx_eq_mat(&m, &expected, 1e-12));
    }

    #[test]
    fn bbox_fallback_grabs_zero_fill_open_stroke() {
        // RA3 (1): an OPEN 2-vertex stroke from local (0,0)->(10,0). Its outline is
        // not a closed fill, so the even-odd polygon test always misses — without the
        // bbox fallback this object is ungrabbable. The fallback hits its local bbox
        // (padded), so a point near the stroke grabs it; a far point still misses.
        let stroke = vec![(0.0_f32, 0.0_f32), (10.0, 0.0)];
        // Bare polygon hit misses (open, < closed fill): proves the gap the fallback fills.
        assert!(!hit_test_object(&IDENTITY, &stroke, 5.0, 0.0));
        // closed=false => bbox fallback. On the stroke (pad lets a slightly-off point hit).
        assert!(hit_test_object_or_bbox(&IDENTITY, &stroke, false, 1.0, 5.0, 0.5));
        // Far outside the padded bbox: misses (empty space still misses).
        assert!(!hit_test_object_or_bbox(&IDENTITY, &stroke, false, 1.0, 50.0, 50.0));
    }

    #[test]
    fn bbox_fallback_does_not_make_filled_body_grab_empty_space() {
        // RA3 (1): a CLOSED fill keeps the exact even-odd hit — the bbox fallback must
        // NOT fire for filled objects, or a click in a concave notch / on empty canvas
        // inside the bbox would wrongly grab. Concave arrow: notch reads as a miss.
        let arrow = vec![
            (0.0_f32, 0.0_f32),
            (4.0, 0.0),
            (4.0, 4.0),
            (2.0, 2.0),
            (0.0, 4.0),
        ];
        assert!(hit_test_object_or_bbox(&IDENTITY, &arrow, true, 4.0, 2.0, 0.5)); // body
        assert!(!hit_test_object_or_bbox(&IDENTITY, &arrow, true, 4.0, 2.0, 3.5)); // notch
    }

    #[test]
    fn zero_size_object_is_grabbable_via_padded_bbox() {
        // RA3 (1): a collapsed (zero-size) object — a single local point. Its bbox is a
        // point; only the pad makes it a finite target. A point within the pad hits.
        let collapsed = vec![(0.0_f32, 0.0_f32)];
        assert!(hit_test_object_or_bbox(&IDENTITY, &collapsed, false, 4.0, 2.0, 2.0));
        assert!(!hit_test_object_or_bbox(&IDENTITY, &collapsed, false, 4.0, 10.0, 10.0));
    }

    #[test]
    fn swept_segment_crosses_filled_object_between_endpoints() {
        // RA3 (2): the unit rect [0,1]^2. A world segment from (-1,0.5) to (2,0.5)
        // passes THROUGH the rect though NEITHER endpoint is inside it — the swept
        // test must still hit (a point test at either endpoint would miss).
        let rect = unit_rect();
        assert!(!hit_test_object(&IDENTITY, &rect, -1.0, 0.5)); // endpoint A outside
        assert!(!hit_test_object(&IDENTITY, &rect, 2.0, 0.5)); // endpoint B outside
        assert!(swept_segment_hits_object(
            &IDENTITY, &rect, true, 0.0, -1.0, 0.5, 2.0, 0.5,
        ));
        // A parallel segment that misses the rect entirely stays a miss.
        assert!(!swept_segment_hits_object(
            &IDENTITY, &rect, true, 0.0, -1.0, 5.0, 2.0, 5.0,
        ));
    }

    #[test]
    fn swept_segment_crosses_open_stroke_via_bbox() {
        // RA3 (2): an open stroke local (0,0)->(10,0). A world segment crossing its
        // local bbox (a near-vertical line at x=5) hits via the bbox-crossing fallback.
        let stroke = vec![(0.0_f32, 0.0_f32), (10.0, 0.0)];
        assert!(swept_segment_hits_object(
            &IDENTITY, &stroke, false, 0.0, 5.0, -3.0, 5.0, 3.0,
        ));
        // A segment well clear of the bbox misses.
        assert!(!swept_segment_hits_object(
            &IDENTITY, &stroke, false, 0.0, 50.0, -3.0, 50.0, 3.0,
        ));
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
