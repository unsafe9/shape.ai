//! Derived outline / region for the object render model. Derives a fillable /
//! hittable / selectable "region" from an object's geometry path-string:
//!
//! * A closed contour's region is its flattened boundary polygon + AABB.
//! * An open contour's region is the concave hull of its stroke points.
//!
//! Pure CPU geometry: host-neutral, no GPU, no time/IO/threads.
//!
//! Coordinates: object geometry is stored as object-local quantized integers at 8
//! units/px; [`parse_path_string`] decodes to `f32` pixel nodes (`/8.0`) and
//! flattens cubics so every downstream stage works in one pixel space.
//! `derive_region` itself takes already-flattened subpaths.
//!
//! Concave hull: Andrew's monotone-chain convex hull, then an iterative dig-in pass
//! pulling in nearby interior points along long edges (a deliberately simple
//! concave hull; a principled alpha-shape is a follow-up). Degrades cleanly to the
//! convex hull when nothing can be pulled in.

#![allow(dead_code)]

use std::collections::HashMap;

/// A derived "shape" for one object's geometry, in pixel coordinates. `outline` is
/// a single boundary polygon (winding not guaranteed) with no repeated closing
/// vertex. `closed` records whether the source was a closed fill or an open stroke
/// hull. The AABB fields bound every outline vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub outline: Vec<(f32, f32)>,
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub closed: bool,
}

impl Region {
    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

/// Derive a [`Region`] from flattened `(closed, points)` subpaths. `flatness` is the
/// merge tolerance (px) for collapsing near-duplicate / near-collinear vertices.
///
/// Selection rule: if any subpath is closed, the region is the largest-area closed
/// subpath's boundary (inner subpaths are even-odd holes); else all open points are
/// pooled into a concave hull. `None` when there is too little geometry to bound.
pub fn derive_region(subpaths: &[(bool, Vec<(f32, f32)>)], flatness: f32) -> Option<Region> {
    let tol = flatness.max(0.0);

    // Prefer a closed contour: its flattened boundary *is* the region.
    let largest_closed = subpaths
        .iter()
        .filter(|(closed, pts)| *closed && pts.len() >= 3)
        .max_by(|a, b| {
            signed_area(&a.1)
                .abs()
                .partial_cmp(&signed_area(&b.1).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    if let Some((_, points)) = largest_closed {
        let outline = cleanup_polygon(points, tol);
        if outline.len() < 3 {
            return None;
        }
        return Some(bound(outline, true));
    }

    // Open contour(s): pool every open stroke's points into a concave hull. Closed
    // subpaths are excluded — a degenerate closed contour encloses no area and must
    // not be repurposed as an open stroke.
    let mut pool: Vec<(f32, f32)> = Vec::new();
    for (closed, pts) in subpaths {
        if !*closed {
            pool.extend_from_slice(pts);
        }
    }
    let pool = dedup_points(&pool, tol);
    if pool.len() < 2 {
        return None;
    }
    if pool.len() == 2 {
        // A degenerate two-point stroke: return the segment as a 2-vertex outline
        // so AABB/hit still work.
        return Some(bound(pool, false));
    }

    let hull = concave_hull(&pool, tol);
    if hull.len() < 3 {
        return None;
    }
    Some(bound(hull, false))
}

/// Build a [`Region`] from a finished outline by computing its AABB.
fn bound(outline: Vec<(f32, f32)>, closed: bool) -> Region {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &(x, y) in &outline {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Region {
        outline,
        min_x,
        min_y,
        max_x,
        max_y,
        closed,
    }
}

/// Signed area via the shoelace formula (sign = winding, magnitude = enclosed area).
fn signed_area(points: &[(f32, f32)]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for i in 0..points.len() {
        let (x0, y0) = points[i];
        let (x1, y1) = points[(i + 1) % points.len()];
        sum += x0 * y1 - x1 * y0;
    }
    sum * 0.5
}

/// Drop consecutive vertices closer than `tol`, a redundant closing vertex, then
/// near-collinear interior vertices (within `tol` of the chord).
fn cleanup_polygon(points: &[(f32, f32)], tol: f32) -> Vec<(f32, f32)> {
    let mut out = dedup_points(points, tol);
    // A closed polygon may repeat its first vertex at the end; drop it.
    if out.len() >= 2 && dist2(out[0], out[out.len() - 1]) <= tol * tol {
        out.pop();
    }
    if out.len() < 3 || tol <= 0.0 {
        return out;
    }
    let mut culled: Vec<(f32, f32)> = Vec::with_capacity(out.len());
    let n = out.len();
    for i in 0..n {
        let prev = out[(i + n - 1) % n];
        let cur = out[i];
        let next = out[(i + 1) % n];
        if point_segment_distance(cur, prev, next) > tol {
            culled.push(cur);
        }
    }
    if culled.len() >= 3 {
        culled
    } else {
        out
    }
}

/// Drop consecutive points closer than `tol` (squared compare). With `tol <= 0`
/// only exact-equal neighbors are merged. The first point is always kept.
fn dedup_points(points: &[(f32, f32)], tol: f32) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    let tol2 = tol * tol;
    for &p in points {
        match out.last() {
            Some(&last) if dist2(last, p) <= tol2 => {}
            _ => out.push(p),
        }
    }
    out
}

/// Andrew's monotone-chain convex hull. Returns hull vertices in counter-
/// clockwise order with no repeated endpoint. Input need not be sorted.
fn convex_hull(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut pts: Vec<(f32, f32)> = points.to_vec();
    pts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    pts.dedup();
    let n = pts.len();
    if n < 3 {
        return pts;
    }

    let mut hull: Vec<(f32, f32)> = Vec::with_capacity(n + 1);
    // Lower hull.
    for &p in &pts {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    // Upper hull.
    let lower_len = hull.len() + 1;
    for &p in pts.iter().rev() {
        while hull.len() >= lower_len && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
        {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop(); // last == first
    hull
}

/// Concave hull: convex hull, then a dig-in refinement pulling the boundary toward
/// nearby interior points along long edges. Degrades to the convex hull when no
/// point can be pulled in.
fn concave_hull(points: &[(f32, f32)], tol: f32) -> Vec<(f32, f32)> {
    let mut hull = convex_hull(points);
    if hull.len() < 3 {
        return hull;
    }

    // Threshold tied to mean nearest-neighbor spacing so the concavity scale tracks
    // the cloud's own resolution rather than a fixed magic number.
    let spacing = mean_nearest_neighbor(points).max(tol).max(f32::EPSILON);
    let dig_threshold = spacing * DIG_THRESHOLD_FACTOR;

    // Bound the passes so a pathological cloud cannot loop unboundedly.
    let max_passes = points.len().saturating_mul(2);
    for _ in 0..max_passes {
        let Some((edge_idx, insert_pt)) = best_dig_candidate(&hull, points, dig_threshold) else {
            break;
        };
        hull.insert(edge_idx + 1, insert_pt);
    }
    hull
}

/// Factor on mean point spacing above which a hull edge is long enough to dig into.
/// Larger = smoother (nearer convex); smaller = more concave.
const DIG_THRESHOLD_FACTOR: f32 = 2.5;

/// Find the hull edge most worth digging into and the interior point to insert: an
/// off-hull point projecting onto the edge interior whose insertion shortens the
/// longest edge the most. Returns `(edge_index, point)` or `None`.
fn best_dig_candidate(
    hull: &[(f32, f32)],
    points: &[(f32, f32)],
    dig_threshold: f32,
) -> Option<(usize, (f32, f32))> {
    let on_hull = |p: (f32, f32)| hull.iter().any(|&h| h == p);
    let mut best: Option<(usize, (f32, f32), f32)> = None;
    let n = hull.len();
    for i in 0..n {
        let a = hull[i];
        let b = hull[(i + 1) % n];
        let edge_len = dist(a, b);
        if edge_len < dig_threshold {
            continue;
        }
        for &p in points {
            if on_hull(p) {
                continue;
            }
            // Must project onto the edge interior, not past an end.
            let t = project_t(p, a, b);
            if !(0.05..=0.95).contains(&t) {
                continue;
            }
            let d = point_segment_distance(p, a, b);
            // A real concavity: closer to the edge than the edge is long.
            if d >= edge_len {
                continue;
            }
            // Prefer the deepest dent on the longest edge.
            let score = edge_len - d;
            if best.map(|(_, _, s)| score > s).unwrap_or(true) {
                best = Some((i, p, score));
            }
        }
    }
    best.map(|(i, p, _)| (i, p))
}

/// Mean distance from each point to its nearest other point (O(n^2), run once per
/// derive). Returns 0 for fewer than two points.
fn mean_nearest_neighbor(points: &[(f32, f32)]) -> f32 {
    let n = points.len();
    if n < 2 {
        return 0.0;
    }
    let mut total = 0.0_f32;
    for i in 0..n {
        let mut nearest = f32::INFINITY;
        for j in 0..n {
            if i == j {
                continue;
            }
            nearest = nearest.min(dist2(points[i], points[j]));
        }
        total += nearest.sqrt();
    }
    total / n as f32
}

/// Parse a quantized-integer SVG-subset path-string (`M`/`L`/`C`/`Z`, absolute
/// integer coords at 8 units/px) into flattened pixel subpaths for [`derive_region`].
/// `C` carries absolute control points. Cubics are flattened to within `flatness` px
/// of chord error; coords are converted to pixels by `/ QUANT_UNITS_PER_PX`. Returns
/// `(closed, points)` per subpath, or `None` on a malformed string.
pub fn parse_path_string(path: &str, flatness: f32) -> Option<Vec<(bool, Vec<(f32, f32)>)>> {
    let mut tokens = PathLexer::new(path);
    let mut subpaths: Vec<(bool, Vec<(f32, f32)>)> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut current_closed = false;
    let mut cursor = (0.0_f32, 0.0_f32);
    let mut start = (0.0_f32, 0.0_f32);
    let mut have_subpath = false;
    let tol = flatness.max(MIN_FLATNESS);

    let flush = |subpaths: &mut Vec<(bool, Vec<(f32, f32)>)>,
                 current: &mut Vec<(f32, f32)>,
                 closed: bool| {
        if !current.is_empty() {
            subpaths.push((closed, std::mem::take(current)));
        }
    };

    while let Some(cmd) = tokens.next_command() {
        match cmd {
            b'M' => {
                if have_subpath {
                    flush(&mut subpaths, &mut current, current_closed);
                }
                let x = tokens.next_coord()?;
                let y = tokens.next_coord()?;
                cursor = (x, y);
                start = cursor;
                current = vec![cursor];
                current_closed = false;
                have_subpath = true;
            }
            b'L' => {
                if !have_subpath {
                    return None;
                }
                let x = tokens.next_coord()?;
                let y = tokens.next_coord()?;
                cursor = (x, y);
                current.push(cursor);
            }
            b'C' => {
                if !have_subpath {
                    return None;
                }
                let c1x = tokens.next_coord()?;
                let c1y = tokens.next_coord()?;
                let c2x = tokens.next_coord()?;
                let c2y = tokens.next_coord()?;
                let ex = tokens.next_coord()?;
                let ey = tokens.next_coord()?;
                flatten_cubic(
                    cursor,
                    (c1x, c1y),
                    (c2x, c2y),
                    (ex, ey),
                    tol,
                    &mut current,
                );
                cursor = (ex, ey);
            }
            b'Z' => {
                if !have_subpath {
                    return None;
                }
                current_closed = true;
                cursor = start;
                flush(&mut subpaths, &mut current, true);
                have_subpath = false;
            }
            _ => return None,
        }
    }
    if have_subpath {
        flush(&mut subpaths, &mut current, current_closed);
    }
    if tokens.errored {
        return None;
    }
    if subpaths.is_empty() {
        return None;
    }
    Some(subpaths)
}

/// Object-local geometry stores 8 integer units per pixel.
pub const QUANT_UNITS_PER_PX: f32 = 8.0;

/// Floor on flattening tolerance so a zero/negative `flatness` still terminates.
const MIN_FLATNESS: f32 = 0.05;

/// Tokenizer for the M/L/C/Z path grammar over quantized integers; coords are
/// divided by [`QUANT_UNITS_PER_PX`] to pixels as read.
struct PathLexer<'a> {
    bytes: &'a [u8],
    pos: usize,
    errored: bool,
}

impl<'a> PathLexer<'a> {
    fn new(s: &'a str) -> Self {
        PathLexer {
            bytes: s.as_bytes(),
            pos: 0,
            errored: false,
        }
    }

    fn skip_separators(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' || b == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Advance to the next command byte (M/L/C/Z, case-insensitive, uppercased).
    fn next_command(&mut self) -> Option<u8> {
        self.skip_separators();
        if self.pos >= self.bytes.len() {
            return None;
        }
        let b = self.bytes[self.pos];
        let upper = b.to_ascii_uppercase();
        if matches!(upper, b'M' | b'L' | b'C' | b'Z') {
            self.pos += 1;
            Some(upper)
        } else {
            self.errored = true;
            None
        }
    }

    /// Read one signed integer coordinate, converted to pixels.
    fn next_coord(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.pos;
        if self.pos < self.bytes.len() && (self.bytes[self.pos] == b'-' || self.bytes[self.pos] == b'+') {
            self.pos += 1;
        }
        let digit_start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos == digit_start {
            self.errored = true;
            return None;
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        let units: i64 = slice.parse().ok()?;
        Some(units as f32 / QUANT_UNITS_PER_PX)
    }
}

/// Flatten a cubic Bézier to within `tol` of the chord, pushing endpoints
/// (excluding the start, already held as the cursor) onto `out`.
fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tol: f32,
    out: &mut Vec<(f32, f32)>,
) {
    subdivide_cubic(p0, p1, p2, p3, tol, out, 0);
    out.push(p3);
}

/// Max cubic subdivision depth, a hard recursion guard independent of `tol`.
const MAX_CUBIC_DEPTH: u32 = 16;

fn subdivide_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tol: f32,
    out: &mut Vec<(f32, f32)>,
    depth: u32,
) {
    // Flatness measure: max distance of the two control points to the chord.
    let d1 = point_segment_distance(p1, p0, p3);
    let d2 = point_segment_distance(p2, p0, p3);
    if depth >= MAX_CUBIC_DEPTH || d1.max(d2) <= tol {
        return;
    }
    // de Casteljau split at t = 0.5.
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let mid_pt = mid(p012, p123);
    subdivide_cubic(p0, p01, p012, mid_pt, tol, out, depth + 1);
    out.push(mid_pt);
    subdivide_cubic(mid_pt, p123, p23, p3, tol, out, depth + 1);
}

fn mid(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// `> 0` if `c` is left of the directed segment `a->b`, `< 0` if right, `0` if
/// collinear (twice the signed triangle area).
fn cross(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    dx * dx + dy * dy
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    dist2(a, b).sqrt()
}

/// Parametric position `t` of `p`'s projection onto the infinite line `a->b`,
/// clamped to `[0, 1]` for a finite segment. Degenerate (`a == b`) yields 0.
pub fn project_t(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let abx = b.0 - a.0;
    let aby = b.1 - a.1;
    let len2 = abx * abx + aby * aby;
    if len2 <= f32::EPSILON {
        return 0.0;
    }
    let apx = p.0 - a.0;
    let apy = p.1 - a.1;
    ((apx * abx + apy * aby) / len2).clamp(0.0, 1.0)
}

/// Shortest distance from point `p` to the finite segment `a->b`.
pub fn point_segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let t = project_t(p, a, b);
    let proj = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    dist(p, proj)
}

/// Nearest point on a polyline outline to `(px, py)`, returning the projected point
/// and its SQUARED distance (anchor snapping). When `closed` is true the implicit
/// closing edge `[last, 0]` is included, so a closed silhouette snaps to its full
/// boundary while an open polyline never snaps to an edge it does not draw. A
/// single-vertex outline returns that vertex; an empty outline returns `None`.
pub fn nearest_point_on_polyline(
    outline: &[(f32, f32)],
    closed: bool,
    px: f32,
    py: f32,
) -> Option<((f32, f32), f32)> {
    let n = outline.len();
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some((outline[0], dist2((px, py), outline[0])));
    }
    let p = (px, py);
    let mut best_pt = outline[0];
    let mut best_d2 = f32::INFINITY;
    let last = if closed { n } else { n - 1 };
    for i in 0..last {
        let a = outline[i];
        let b = outline[(i + 1) % n];
        let t = project_t(p, a, b);
        let proj = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let d2 = dist2(p, proj);
        if d2 < best_d2 {
            best_d2 = d2;
            best_pt = proj;
        }
    }
    Some((best_pt, best_d2))
}

/// A revision-keyed LRU of derived regions (mirrors [`crate::render_cache::RenderDataCache`]):
/// an object whose geometry revision is unchanged is a hit and skips re-derivation;
/// bounded by an entry cap with LRU eviction.
#[derive(Clone, Debug)]
pub struct RegionCache {
    entries: HashMap<String, RegionEntry>,
    capacity: usize,
    frame: u64,
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
}

#[derive(Clone, Debug)]
struct RegionEntry {
    revision: u64,
    last_used: u64,
    region: Region,
}

/// Default cap on cached regions (matches the render-data cache budget).
pub const REGION_CACHE_LIMIT: usize = 8192;

impl RegionCache {
    pub fn new() -> Self {
        Self::with_capacity(REGION_CACHE_LIMIT)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        RegionCache {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            frame: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Advance the logical frame clock for LRU recency.
    pub fn begin_frame(&mut self) -> u64 {
        self.frame += 1;
        self.frame
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up the region for `id` at `revision`, invoking `derive` only on a miss
    /// or stale revision. `None` (uncached) when `derive` yields no region.
    pub fn get_or_derive<F>(&mut self, id: &str, revision: u64, derive: F) -> Option<&Region>
    where
        F: FnOnce() -> Option<Region>,
    {
        let frame = self.frame;
        let fresh = matches!(self.entries.get(id), Some(e) if e.revision == revision);
        if fresh {
            self.hits += 1;
            let entry = self.entries.get_mut(id).expect("entry present on hit");
            entry.last_used = frame;
            return Some(&self.entries.get(id).expect("present").region);
        }
        self.misses += 1;
        let region = derive()?;
        self.entries.insert(
            id.to_string(),
            RegionEntry {
                revision,
                last_used: frame,
                region,
            },
        );
        self.evict_if_needed(id);
        Some(&self.entries.get(id).expect("present after insert").region)
    }

    pub fn cached_revision(&self, id: &str) -> Option<u64> {
        self.entries.get(id).map(|e| e.revision)
    }

    /// Drop the cached region for `id` (dirty-id invalidation path).
    pub fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn evict_if_needed(&mut self, protect: &str) {
        while self.entries.len() > self.capacity {
            let victim = self
                .entries
                .iter()
                .filter(|(id, _)| id.as_str() != protect)
                .min_by_key(|(_, e)| e.last_used)
                .map(|(id, _)| id.clone());
            match victim {
                Some(id) => {
                    self.entries.remove(&id);
                    self.evictions += 1;
                }
                None => break,
            }
        }
    }
}

impl Default for RegionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aabb_contains(region: &Region, pts: &[(f32, f32)]) -> bool {
        pts.iter().all(|&(x, y)| {
            x >= region.min_x - 1e-3
                && x <= region.max_x + 1e-3
                && y >= region.min_y - 1e-3
                && y <= region.max_y + 1e-3
        })
    }

    #[test]
    fn closed_rect_region_has_boundary_and_aabb() {
        let rect = vec![(true, vec![(0.0, 0.0), (10.0, 0.0), (10.0, 6.0), (0.0, 6.0)])];
        let region = derive_region(&rect, 0.5).expect("rect yields a region");

        assert!(region.closed, "a closed contour produces a closed region");
        assert!(
            region.outline.len() >= 4,
            "rect boundary keeps its 4 corners, got {}",
            region.outline.len()
        );
        assert_eq!((region.min_x, region.min_y), (0.0, 0.0));
        assert_eq!((region.max_x, region.max_y), (10.0, 6.0));
    }

    #[test]
    fn closed_rect_with_repeated_closing_vertex_is_not_double_counted() {
        // Boundary that repeats the first vertex at the end (explicit Z-style).
        let rect = vec![(
            true,
            vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0), (0.0, 0.0)],
        )];
        let region = derive_region(&rect, 0.0).expect("region");
        assert_eq!(region.outline.len(), 4, "redundant closing vertex dropped");
        assert!(region.closed);
    }

    #[test]
    fn closed_region_picks_largest_subpath_as_silhouette() {
        // Outer square + inner hole: the silhouette's AABB is the outer one's.
        let big = vec![(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
        let hole = vec![(8.0, 8.0), (12.0, 8.0), (12.0, 12.0), (8.0, 12.0)];
        let subpaths = vec![(true, hole), (true, big)];
        let region = derive_region(&subpaths, 0.0).expect("region");
        assert_eq!((region.min_x, region.min_y), (0.0, 0.0));
        assert_eq!((region.max_x, region.max_y), (20.0, 20.0));
    }

    #[test]
    fn open_polyline_of_five_points_yields_hull_bounding_all() {
        let pts = vec![
            (0.0, 0.0),
            (4.0, 8.0),
            (8.0, 1.0),
            (12.0, 9.0),
            (16.0, 2.0),
        ];
        let subpaths = vec![(false, pts.clone())];
        let region = derive_region(&subpaths, 0.25).expect("open polyline yields a region");

        assert!(!region.closed, "an open contour produces an open region");
        assert!(
            region.outline.len() >= 3,
            "hull is a polygon, got {} pts",
            region.outline.len()
        );
        assert!(
            aabb_contains(&region, &pts),
            "hull AABB must bound every input point: {region:?}"
        );
        assert_eq!((region.min_x, region.max_x), (0.0, 16.0));
        assert_eq!((region.min_y, region.max_y), (0.0, 9.0));
    }

    #[test]
    fn convex_hull_of_square_with_interior_point_drops_interior() {
        let pts = vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (5.0, 5.0), // interior, must not be on the hull
        ];
        let hull = convex_hull(&pts);
        assert_eq!(hull.len(), 4, "interior point excluded from convex hull");
        assert!(!hull.contains(&(5.0, 5.0)));
    }

    #[test]
    fn concave_hull_digs_into_a_deep_dent() {
        // A notched cloud (rectangle with a dent in the top edge): the concave hull
        // should pull a boundary vertex into the dent where the convex hull cannot.
        let mut pts = Vec::new();
        // bottom edge
        for i in 0..=10 {
            pts.push((i as f32, 0.0));
        }
        // left & right walls
        for i in 1..=5 {
            pts.push((0.0, i as f32));
            pts.push((10.0, i as f32));
        }
        // top edge with a dent dipping to y=1 around x=5
        for i in 0..=10 {
            let x = i as f32;
            let y = if (4..=6).contains(&i) { 1.0 } else { 5.0 };
            pts.push((x, y));
        }
        let convex = convex_hull(&pts);
        let concave = concave_hull(&pts, 0.5);
        assert!(
            concave.len() > convex.len(),
            "concave hull should add boundary vertices vs convex ({} vs {})",
            concave.len(),
            convex.len()
        );
        let region = bound(concave, false);
        assert!(aabb_contains(&region, &pts));
    }

    #[test]
    fn empty_input_yields_no_region() {
        assert!(derive_region(&[], 0.5).is_none());
        assert!(derive_region(&[(false, vec![(1.0, 1.0)])], 0.5).is_none());
        assert!(derive_region(&[(true, vec![(0.0, 0.0), (1.0, 1.0)])], 0.5).is_none());
    }

    #[test]
    fn two_point_open_stroke_keeps_segment_outline() {
        let region = derive_region(&[(false, vec![(0.0, 0.0), (4.0, 3.0)])], 0.5).expect("region");
        assert_eq!(region.outline.len(), 2);
        assert!(!region.closed);
        assert_eq!((region.min_x, region.min_y, region.max_x, region.max_y), (0.0, 0.0, 4.0, 3.0));
    }

    #[test]
    fn parse_path_string_decodes_closed_rect_in_quantized_units() {
        // 8 units/px: (0,0)->(80,0)->(80,48)->(0,48) close == px (0,0)..(10,6).
        let path = "M 0 0 L 80 0 L 80 48 L 0 48 Z";
        let subpaths = parse_path_string(path, 0.5).expect("valid path");
        assert_eq!(subpaths.len(), 1);
        assert!(subpaths[0].0, "Z closes the subpath");
        assert_eq!(subpaths[0].1[0], (0.0, 0.0));
        assert_eq!(subpaths[0].1[1], (10.0, 0.0));
        assert_eq!(subpaths[0].1[2], (10.0, 6.0));

        let region = derive_region(&subpaths, 0.5).expect("region");
        assert!(region.closed);
        assert_eq!((region.min_x, region.min_y, region.max_x, region.max_y), (0.0, 0.0, 10.0, 6.0));
    }

    #[test]
    fn parse_path_string_flattens_cubic_within_tolerance() {
        let path = "M 0 0 C 0 80 80 80 80 0";
        let subpaths = parse_path_string(path, 0.1).expect("valid cubic path");
        let pts = &subpaths[0].1;
        assert_eq!(pts.first(), Some(&(0.0, 0.0)));
        assert_eq!(pts.last(), Some(&(10.0, 0.0)));
        assert!(pts.len() > 2, "a bowed cubic flattens to several segments");
        // Apex of this symmetric cubic is at (5, 7.5)px; some flattened vertex
        // should be near the curve's peak height.
        let peak = pts.iter().map(|&(_, y)| y).fold(f32::NEG_INFINITY, f32::max);
        assert!((peak - 7.5).abs() < 0.5, "peak {peak} ~ 7.5px");
    }

    #[test]
    fn parse_path_string_handles_multi_subpath_and_negative_coords() {
        let path = "M -16 -16 L 16 -16 L 16 16 Z M 0 80 L 80 80";
        let subpaths = parse_path_string(path, 0.5).expect("valid multi-subpath");
        assert_eq!(subpaths.len(), 2);
        assert!(subpaths[0].0, "first subpath closed by Z");
        assert!(!subpaths[1].0, "second subpath left open");
        assert_eq!(subpaths[0].1[0], (-2.0, -2.0));
        assert_eq!(subpaths[1].1, vec![(0.0, 10.0), (10.0, 10.0)]);
    }

    #[test]
    fn parse_path_string_rejects_malformed_input() {
        assert!(parse_path_string("L 0 0", 0.5).is_none(), "L before M");
        assert!(parse_path_string("M 0", 0.5).is_none(), "missing operand");
        assert!(parse_path_string("M 0 0 X 1 1", 0.5).is_none(), "unknown command");
        assert!(parse_path_string("", 0.5).is_none(), "empty string");
    }

    #[test]
    fn region_cache_hit_skips_rederive_and_revision_invalidates() {
        let mut cache = RegionCache::new();
        let mut derive_calls = 0;
        let geom = vec![(true, vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])];

        cache.begin_frame();
        let r = cache
            .get_or_derive("obj-1", 1, || {
                derive_calls += 1;
                derive_region(&geom, 0.5)
            })
            .cloned();
        assert!(r.is_some());

        cache.begin_frame();
        cache.get_or_derive("obj-1", 1, || {
            derive_calls += 1;
            derive_region(&geom, 0.5)
        });
        assert_eq!(derive_calls, 1, "unchanged revision is a hit, no re-derive");
        assert_eq!(cache.hits, 1);

        cache.begin_frame();
        cache.get_or_derive("obj-1", 2, || {
            derive_calls += 1;
            derive_region(&geom, 0.5)
        });
        assert_eq!(derive_calls, 2, "bumped revision re-derives");
        assert_eq!(cache.cached_revision("obj-1"), Some(2));
    }

    #[test]
    fn region_cache_lru_evicts_least_recently_used() {
        let mut cache = RegionCache::with_capacity(2);
        let rect = |o: f32| vec![(true, vec![(o, o), (o + 1.0, o), (o + 1.0, o + 1.0), (o, o + 1.0)])];

        cache.begin_frame();
        cache.get_or_derive("a", 1, || derive_region(&rect(0.0), 0.0));
        cache.begin_frame();
        cache.get_or_derive("b", 1, || derive_region(&rect(2.0), 0.0));
        // Touch "a" so "b" is the LRU victim.
        cache.begin_frame();
        cache.get_or_derive("a", 1, || derive_region(&rect(0.0), 0.0));
        cache.begin_frame();
        cache.get_or_derive("c", 1, || derive_region(&rect(4.0), 0.0));

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.evictions, 1);
        assert_eq!(cache.cached_revision("b"), None, "LRU victim evicted");
        assert_eq!(cache.cached_revision("a"), Some(1));
        assert_eq!(cache.cached_revision("c"), Some(1));
    }

    #[test]
    fn nearest_point_on_polyline_closing_edge_only_when_closed() {
        // 3 sides of a unit square; the query (-0.1, 0.5) sits left of the missing
        // closing edge's midpoint.
        let outline = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let q = (-0.1_f32, 0.5_f32);

        // Open: no closing edge, so the nearest point is a drawn endpoint, not (0, 0.5).
        let (open_pt, _open_d2) =
            nearest_point_on_polyline(&outline, false, q.0, q.1).expect("open nearest");
        assert!(
            (open_pt.0 - 0.0).abs() < 1e-4 && (open_pt.1 - 0.0).abs() < 1e-4
                || (open_pt.0 - 0.0).abs() < 1e-4 && (open_pt.1 - 1.0).abs() < 1e-4,
            "open polyline snaps to a drawn endpoint, got {open_pt:?}"
        );

        // Closed: the implicit edge [last, 0] is included; nearest is (0, 0.5).
        let (closed_pt, closed_d2) =
            nearest_point_on_polyline(&outline, true, q.0, q.1).expect("closed nearest");
        assert!(
            (closed_pt.0 - 0.0).abs() < 1e-4 && (closed_pt.1 - 0.5).abs() < 1e-4,
            "closed polyline snaps to the closing edge, got {closed_pt:?}"
        );
        assert!((closed_d2 - 0.01).abs() < 1e-4, "d2 = 0.1^2, got {closed_d2}");
    }

    #[test]
    fn nearest_point_on_polyline_handles_degenerate_outlines() {
        assert!(nearest_point_on_polyline(&[], false, 0.0, 0.0).is_none());
        let (pt, d2) =
            nearest_point_on_polyline(&[(3.0, 4.0)], false, 0.0, 0.0).expect("single vertex");
        assert_eq!(pt, (3.0, 4.0));
        assert!((d2 - 25.0).abs() < 1e-4);
    }

    #[test]
    fn region_cache_does_not_cache_degenerate_geometry() {
        let mut cache = RegionCache::new();
        cache.begin_frame();
        let r = cache.get_or_derive("bad", 1, || derive_region(&[(true, vec![(0.0, 0.0)])], 0.5));
        assert!(r.is_none(), "degenerate geometry yields no region");
        assert_eq!(cache.cached_revision("bad"), None, "nothing cached on None");
    }
}
