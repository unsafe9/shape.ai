//! CPU stroke expansion reference: turns a polyline into a filled triangle ribbon
//! of a given width. The GPU vertex-shader expansion is the fast path; this is the
//! CPU reference/fallback the test gate checks.
//!
//! Coordinates are CSS pixels as `f32` (quantized object-local i32 is `/ 8.0` before
//! reaching this module). `width` is the full stroke width; the ribbon extends
//! `± width / 2` from the centerline, with per-node width overriding when supplied.
//! Pointer-width-agnostic: indices are `u32`.

/// Line-cap style for the open ends of a stroke; ignored on closed paths.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    Butt,
    /// Extended by `width / 2` past the last node, squared off.
    Square,
    /// Rounded with a semicircular fan of [`ROUND_CAP_SEGMENTS`] segments.
    Round,
}

/// Line-join style between two segments at an interior node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Join {
    /// Extend the outer edges until they meet; falls back to a bevel when sharper
    /// than [`MITER_LIMIT`] (the spike runs away to infinity at a cusp).
    Miter,
    Bevel,
}

/// A triangle mesh, the same shape the live pipeline feeds the GPU.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn new() -> Self {
        Mesh {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Push a vertex via `u32::try_from` so a pathological overflow panics here
    /// rather than silently wrapping.
    fn push_vertex(&mut self, v: [f32; 2]) -> u32 {
        let idx = u32::try_from(self.vertices.len())
            .expect("stroke mesh vertex count exceeds u32");
        self.vertices.push(v);
        idx
    }

    /// Winding order is not enforced; the live pipeline does not cull stroke geometry.
    fn push_tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.push(a);
        self.indices.push(b);
        self.indices.push(c);
    }

    /// Two triangles wound `a, b, c` then `a, c, d` (a fan around `a`).
    fn push_quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.push_tri(a, b, c);
        self.push_tri(a, c, d);
    }
}

/// Segments in the semicircular fan approximating a round cap.
pub const ROUND_CAP_SEGMENTS: u32 = 8;

/// Miter ratio ceiling (miter length / stroke width); above this the miter
/// degenerates to a bevel. `4.0` matches the SVG/Canvas default `stroke-miterlimit`.
pub const MITER_LIMIT: f32 = 4.0;

/// Below this segment length (px) direction is undefined, so the segment is skipped.
const MIN_SEGMENT_LEN: f32 = 1.0e-6;

/// Expand `points` into a filled triangle ribbon of `width`, honoring per-node
/// width, joins, and caps. `closed` wraps the last node to the first and suppresses
/// end caps. `per_node_width`, when `Some`, must match `points` length; the
/// half-width along a segment is the mean of its endpoints' half-widths. Degenerate
/// inputs (< 2 distinct points, non-positive width) yield an empty mesh.
pub fn expand_stroke(
    points: &[(f32, f32)],
    closed: bool,
    width: f32,
    per_node_width: Option<&[f32]>,
    cap: Cap,
    join: Join,
) -> Mesh {
    let mut mesh = Mesh::new();
    if width <= 0.0 {
        return mesh;
    }

    // Collapse runs of coincident points and remember each survivor's index in
    // the original list so per-node width still lines up.
    let pts = dedup_points(points);
    if pts.len() < 2 {
        return mesh;
    }

    let half_at = |orig_index: usize| -> f32 {
        match per_node_width {
            Some(widths) if orig_index < widths.len() && widths[orig_index] > 0.0 => {
                widths[orig_index] * 0.5
            }
            _ => width * 0.5,
        }
    };

    // N-1 open segments, or N closed segments (the wrap-around included).
    let seg_count = if closed { pts.len() } else { pts.len() - 1 };
    let mut dirs: Vec<[f32; 2]> = Vec::with_capacity(seg_count);
    let mut normals: Vec<[f32; 2]> = Vec::with_capacity(seg_count);
    for s in 0..seg_count {
        let (ax, ay) = pts[s].point;
        let (bx, by) = pts[(s + 1) % pts.len()].point;
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (dx / len, dy / len);
        dirs.push([ux, uy]);
        // Left normal of direction (ux, uy) is (-uy, ux).
        normals.push([-uy, ux]);
    }

    for s in 0..seg_count {
        let i0 = s;
        let i1 = (s + 1) % pts.len();
        let (ax, ay) = pts[i0].point;
        let (bx, by) = pts[i1].point;
        let n = normals[s];
        let h0 = half_at(pts[i0].orig);
        let h1 = half_at(pts[i1].orig);

        let a_left = mesh.push_vertex([ax + n[0] * h0, ay + n[1] * h0]);
        let a_right = mesh.push_vertex([ax - n[0] * h0, ay - n[1] * h0]);
        let b_left = mesh.push_vertex([bx + n[0] * h1, by + n[1] * h1]);
        let b_right = mesh.push_vertex([bx - n[0] * h1, by - n[1] * h1]);
        mesh.push_quad(a_left, b_left, b_right, a_right);
    }

    // Closed: every node is interior (including wrap node 0). Open: nodes 1..=N-2.
    let join_nodes: Vec<usize> = if closed {
        (0..pts.len()).collect()
    } else {
        (1..pts.len() - 1).collect()
    };
    for &node in &join_nodes {
        let incoming = (node + seg_count - 1) % seg_count;
        let outgoing = node % seg_count;
        let (px, py) = pts[node].point;
        let h = half_at(pts[node].orig);
        add_join(
            &mut mesh,
            (px, py),
            h,
            normals[incoming],
            normals[outgoing],
            dirs[incoming],
            dirs[outgoing],
            join,
        );
    }

    if !closed {
        // Start cap: free end at node 0, pointing back along -dir[0].
        let (sx, sy) = pts[0].point;
        add_cap(
            &mut mesh,
            (sx, sy),
            half_at(pts[0].orig),
            normals[0],
            [-dirs[0][0], -dirs[0][1]],
            cap,
        );
        // End cap: free end at the last node, pointing forward along +dir[last].
        let last = pts.len() - 1;
        let last_seg = seg_count - 1;
        let (ex, ey) = pts[last].point;
        add_cap(
            &mut mesh,
            (ex, ey),
            half_at(pts[last].orig),
            normals[last_seg],
            dirs[last_seg],
            cap,
        );
    }

    mesh
}

/// A surviving point after coincident-run collapse; `orig` is its index in the
/// caller's original `points` so per-node width still maps correctly.
struct DedupPoint {
    point: (f32, f32),
    orig: usize,
}

/// Drop points coinciding with their predecessor (within [`MIN_SEGMENT_LEN`]).
fn dedup_points(points: &[(f32, f32)]) -> Vec<DedupPoint> {
    let mut out: Vec<DedupPoint> = Vec::with_capacity(points.len());
    for (orig, &(x, y)) in points.iter().enumerate() {
        if let Some(prev) = out.last() {
            let (dx, dy) = (x - prev.point.0, y - prev.point.1);
            if (dx * dx + dy * dy).sqrt() < MIN_SEGMENT_LEN {
                continue;
            }
        }
        out.push(DedupPoint { point: (x, y), orig });
    }
    out
}

/// Fill the outer wedge at an interior node where the incoming and outgoing
/// segments meet. `n_in`/`n_out` are the segments' left-normals, `d_in`/`d_out`
/// their unit directions. The turn direction picks which side is the outer
/// (convex) corner that needs filling; the inner side overlaps and needs nothing.
#[allow(clippy::too_many_arguments)]
fn add_join(
    mesh: &mut Mesh,
    center: (f32, f32),
    half: f32,
    n_in: [f32; 2],
    n_out: [f32; 2],
    d_in: [f32; 2],
    d_out: [f32; 2],
    join: Join,
) {
    // Cross of incoming/outgoing direction: sign tells turn handedness.
    let cross = d_in[0] * d_out[1] - d_in[1] * d_out[0];
    if cross.abs() < MIN_SEGMENT_LEN {
        // Collinear: the offset quads already abut, nothing to fill.
        return;
    }
    let (cx, cy) = center;
    // Outer side is opposite the turn (left turn -> right outer corner).
    let side = if cross > 0.0 { -1.0 } else { 1.0 };

    let p_in = [cx + n_in[0] * half * side, cy + n_in[1] * half * side];
    let p_out = [cx + n_out[0] * half * side, cy + n_out[1] * half * side];

    let c = mesh.push_vertex([cx, cy]);
    let a = mesh.push_vertex(p_in);
    let b = mesh.push_vertex(p_out);

    match join {
        Join::Bevel => {
            mesh.push_tri(c, a, b);
        }
        Join::Miter => {
            // Miter apex lies along the bisector of the outer normals at distance
            // half / cos(theta/2).
            let mut bisx = n_in[0] * side + n_out[0] * side;
            let mut bisy = n_in[1] * side + n_out[1] * side;
            let bis_len = (bisx * bisx + bisy * bisy).sqrt();
            if bis_len < MIN_SEGMENT_LEN {
                // Normals oppose (≈180° turn): no finite miter; bevel it.
                mesh.push_tri(c, a, b);
                return;
            }
            bisx /= bis_len;
            bisy /= bis_len;
            // cos(theta/2) = dot(outer_normal, bisector); guard against zero.
            let cos_half = n_in[0] * side * bisx + n_in[1] * side * bisy;
            if cos_half.abs() < MIN_SEGMENT_LEN {
                mesh.push_tri(c, a, b);
                return;
            }
            let miter_len = half / cos_half;
            if (miter_len / (half * 2.0)).abs() > MITER_LIMIT {
                // Spike too long: clip to a bevel.
                mesh.push_tri(c, a, b);
                return;
            }
            let apex = mesh.push_vertex([cx + bisx * miter_len, cy + bisy * miter_len]);
            mesh.push_tri(c, a, apex);
            mesh.push_tri(c, apex, b);
        }
    }
}

/// Close a free end with a cap. `out_dir` is the outward direction past the end.
fn add_cap(
    mesh: &mut Mesh,
    center: (f32, f32),
    half: f32,
    normal: [f32; 2],
    out_dir: [f32; 2],
    cap: Cap,
) {
    let (cx, cy) = center;
    let left = [cx + normal[0] * half, cy + normal[1] * half];
    let right = [cx - normal[0] * half, cy - normal[1] * half];

    match cap {
        Cap::Butt => {} // Flush end; the ribbon already ends here.
        Cap::Square => {
            let ext_left = [left[0] + out_dir[0] * half, left[1] + out_dir[1] * half];
            let ext_right = [right[0] + out_dir[0] * half, right[1] + out_dir[1] * half];
            let l = mesh.push_vertex(left);
            let r = mesh.push_vertex(right);
            let el = mesh.push_vertex(ext_left);
            let er = mesh.push_vertex(ext_right);
            mesh.push_quad(l, el, er, r);
        }
        Cap::Round => {
            let c = mesh.push_vertex([cx, cy]);
            let start = (normal[1].atan2(normal[0])) as f64;
            // Sweep 180° so the fan's midpoint points along `out_dir`. The cross sign
            // fixes the half-circle's sense (CCW when positive).
            let cross = normal[0] * out_dir[1] - normal[1] * out_dir[0];
            let sweep = if cross >= 0.0 {
                std::f64::consts::PI
            } else {
                -std::f64::consts::PI
            };
            let n = ROUND_CAP_SEGMENTS;
            let mut prev = mesh.push_vertex(left);
            for k in 1..=n {
                let t = f64::from(k) / f64::from(n);
                let ang = start + sweep * t;
                let p = [
                    cx + crate::cast::narrow_f32(ang.cos()) * half,
                    cy + crate::cast::narrow_f32(ang.sin()) * half,
                ];
                let cur = mesh.push_vertex(p);
                mesh.push_tri(c, prev, cur);
                prev = cur;
            }
        }
    }
}

/// Split a polyline into the "on" subpaths of a dash pattern. `dash` lengths are in
/// px and alternate on/off, repeating; an empty/all-zero pattern yields one subpath
/// (the whole polyline). Dashes are sampled so mid-segment boundaries land exactly.
pub fn dash_segments(points: &[(f32, f32)], dash: &[f32]) -> Vec<Vec<(f32, f32)>> {
    if points.len() < 2 {
        return Vec::new();
    }
    // Solid when there is no usable pattern.
    if dash.is_empty() || dash.iter().all(|&d| d <= 0.0) {
        return vec![points.to_vec()];
    }

    // Precompute cumulative arc length so we can walk dash boundaries.
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();

    let mut dash_idx = 0usize;
    let mut remaining = dash[0];
    // Advance past any leading zero-length spans so `remaining > 0`.
    while remaining <= 0.0 {
        dash_idx = (dash_idx + 1) % dash.len();
        remaining = dash[dash_idx];
    }
    let mut drawing = dash_idx % 2 == 0;

    if drawing {
        current.push(points[0]);
    }

    for s in 0..points.len() - 1 {
        let (ax, ay) = points[s];
        let (bx, by) = points[s + 1];
        let (dx, dy) = (bx - ax, by - ay);
        let seg_len = (dx * dx + dy * dy).sqrt();
        if seg_len < MIN_SEGMENT_LEN {
            continue;
        }
        let (ux, uy) = (dx / seg_len, dy / seg_len);
        let mut walked = 0.0f32; // distance consumed within this segment

        while seg_len - walked > remaining {
            walked += remaining;
            let bx_ = ax + ux * walked;
            let by_ = ay + uy * walked;

            // Collapse zero-length spans (they flip on/off without consuming
            // distance) and take the net drawing state at the boundary.
            let was_drawing = drawing;
            loop {
                drawing = !drawing;
                dash_idx = (dash_idx + 1) % dash.len();
                remaining = dash[dash_idx];
                if remaining > 0.0 {
                    break;
                }
            }

            if drawing == was_drawing {
                // Flipped back to the same state: the run continues unbroken.
                continue;
            }
            if was_drawing {
                current.push((bx_, by_));
                out.push(std::mem::take(&mut current));
            } else {
                current.push((bx_, by_));
            }
        }

        remaining -= seg_len - walked;
        if drawing {
            current.push((bx, by));
        }
    }

    if drawing && current.len() >= 2 {
        out.push(current);
    }

    // A boundary on the final point can leave a 1-point stub; drop non-polylines.
    out.retain(|sub| sub.len() >= 2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.0e-4
    }

    fn polyline_length(pts: &[(f32, f32)]) -> f32 {
        pts.windows(2)
            .map(|w| {
                let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }

    #[test]
    fn horizontal_segment_is_a_quad() {
        let mesh = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0)],
            false,
            2.0,
            None,
            Cap::Butt,
            Join::Miter,
        );
        assert_eq!(mesh.vertices.len(), 4, "one ribbon quad = 4 verts");
        assert_eq!(mesh.indices.len(), 6, "one quad = two triangles");

        let ys: Vec<f32> = mesh.vertices.iter().map(|v| v[1]).collect();
        let xs: Vec<f32> = mesh.vertices.iter().map(|v| v[0]).collect();
        let max_y = ys.iter().cloned().fold(f32::MIN, f32::max);
        let min_y = ys.iter().cloned().fold(f32::MAX, f32::min);
        let max_x = xs.iter().cloned().fold(f32::MIN, f32::max);
        let min_x = xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!(approx(max_y, 1.0) && approx(min_y, -1.0), "spans ±1 in y");
        assert!(approx(max_x, 10.0) && approx(min_x, 0.0), "spans the segment in x");
    }

    #[test]
    fn dash_4_4_over_length_16_yields_two_on_segments() {
        // dash [4,4] over length 16: on-spans [0,4] and [8,12].
        let subs = dash_segments(&[(0.0, 0.0), (16.0, 0.0)], &[4.0, 4.0]);
        assert_eq!(subs.len(), 2, "two on-dashes over length 16");
        assert!(approx(subs[0][0].0, 0.0) && approx(subs[0].last().unwrap().0, 4.0));
        assert!(approx(subs[1][0].0, 8.0) && approx(subs[1].last().unwrap().0, 12.0));
    }

    #[test]
    fn empty_dash_is_solid() {
        let line = [(0.0, 0.0), (5.0, 0.0), (5.0, 5.0)];
        let subs = dash_segments(&line, &[]);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], line.to_vec());
    }

    #[test]
    fn dash_crosses_vertex_correctly() {
        // L-shape (two legs of 4), dash [2,2]: on-spans [0,2] and [4,6] (corner-straddling).
        let line = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)];
        let subs = dash_segments(&line, &[2.0, 2.0]);
        assert_eq!(subs.len(), 2, "on at [0,2] and [4,6]");
        let second = &subs[1];
        assert!(
            second.iter().any(|p| approx(p.0, 4.0) && approx(p.1, 0.0)),
            "on-span crossing the corner includes the vertex {second:?}"
        );
        assert!(approx(polyline_length(second), 2.0));
    }

    #[test]
    fn dash_starts_on_at_origin() {
        let subs = dash_segments(&[(0.0, 0.0), (12.0, 0.0)], &[3.0, 3.0]);
        assert_eq!(subs.len(), 2);
        assert!(approx(subs[0][0].0, 0.0) && approx(subs[0].last().unwrap().0, 3.0));
        assert!(approx(subs[1][0].0, 6.0) && approx(subs[1].last().unwrap().0, 9.0));
    }

    #[test]
    fn dash_skips_zero_length_spans() {
        // Pattern [4, 0, 4]: the middle 0-length gap merges [0,4]+[4,8] into [0,8],
        // matching SVG dash semantics.
        let subs = dash_segments(&[(0.0, 0.0), (16.0, 0.0)], &[4.0, 0.0, 4.0]);
        assert_eq!(subs.len(), 1, "the zero gap merges two dashes into one");
        assert!(
            approx(subs[0][0].0, 0.0) && approx(subs[0].last().unwrap().0, 8.0),
            "merged on-span spans [0,8]: {:?}",
            subs[0]
        );
    }

    #[test]
    fn zero_width_is_empty() {
        let mesh = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0)],
            false,
            0.0,
            None,
            Cap::Butt,
            Join::Miter,
        );
        assert!(mesh.vertices.is_empty() && mesh.indices.is_empty());
    }

    #[test]
    fn fewer_than_two_points_is_empty() {
        let mesh = expand_stroke(&[(1.0, 1.0)], false, 2.0, None, Cap::Butt, Join::Miter);
        assert!(mesh.vertices.is_empty());
        assert!(dash_segments(&[(1.0, 1.0)], &[4.0, 4.0]).is_empty());
    }

    #[test]
    fn coincident_points_are_collapsed() {
        let mesh = expand_stroke(
            &[(0.0, 0.0), (0.0, 0.0), (10.0, 0.0)],
            false,
            2.0,
            None,
            Cap::Butt,
            Join::Miter,
        );
        // One real segment -> one quad.
        assert_eq!(mesh.vertices.len(), 4);
    }

    #[test]
    fn square_cap_extends_both_ends() {
        let butt = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0)],
            false,
            2.0,
            None,
            Cap::Butt,
            Join::Miter,
        );
        let square = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0)],
            false,
            2.0,
            None,
            Cap::Square,
            Join::Miter,
        );
        // Two extra cap quads (4 verts + 6 idx each) over the butt ribbon.
        assert_eq!(square.vertices.len(), butt.vertices.len() + 8);
        assert_eq!(square.indices.len(), butt.indices.len() + 12);
        let xs: Vec<f32> = square.vertices.iter().map(|v| v[0]).collect();
        let max_x = xs.iter().cloned().fold(f32::MIN, f32::max);
        let min_x = xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!(approx(max_x, 11.0), "end extended by half-width");
        assert!(approx(min_x, -1.0), "start extended by half-width");
    }

    #[test]
    fn round_cap_adds_two_fans() {
        let mesh = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0)],
            false,
            2.0,
            None,
            Cap::Round,
            Join::Miter,
        );
        // Body quad = 4 verts, 6 idx. Each round cap = center + start + N new
        // verts and N triangles.
        let n = ROUND_CAP_SEGMENTS;
        let cap_verts = 2 + n; // center, start endpoint, then N fan points
        let expected_verts = 4 + 2 * cap_verts;
        let expected_idx = 6 + 2 * (3 * n);
        assert_eq!(crate::cast::len_u32(mesh.vertices.len()), expected_verts);
        assert_eq!(crate::cast::len_u32(mesh.indices.len()), expected_idx);
        // No fan point exceeds half-width 1 from its cap center.
        let r_max = mesh
            .vertices
            .iter()
            .map(|v| {
                // Distance from whichever cap center (0,0) or (10,0).
                let d0 = (v[0] * v[0] + v[1] * v[1]).sqrt();
                let d1 = ((v[0] - 10.0).powi(2) + v[1] * v[1]).sqrt();
                d0.min(d1)
            })
            .fold(f32::MIN, f32::max);
        assert!(r_max <= 1.0 + 1.0e-4, "round cap within half-width radius");
    }

    #[test]
    fn miter_join_adds_apex_on_right_angle() {
        let bevel = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
            false,
            2.0,
            None,
            Cap::Butt,
            Join::Bevel,
        );
        let miter = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
            false,
            2.0,
            None,
            Cap::Butt,
            Join::Miter,
        );
        // Two ribbon quads = 8 verts, 12 idx in both.
        // Bevel join: center + 2 offset pts (3 verts), 1 triangle.
        assert_eq!(bevel.vertices.len(), 8 + 3);
        assert_eq!(bevel.indices.len(), 12 + 3);
        // Miter join: center + 2 offset pts + apex (4 verts), 2 triangles.
        assert_eq!(miter.vertices.len(), 8 + 4);
        assert_eq!(miter.indices.len(), 12 + 6);
    }

    #[test]
    fn miter_falls_back_to_bevel_on_sharp_cusp() {
        let spike = [(0.0, 0.0), (10.0, 0.0), (0.0, 0.2)];
        let miter = expand_stroke(&spike, false, 2.0, None, Cap::Butt, Join::Miter);
        let bevel = expand_stroke(&spike, false, 2.0, None, Cap::Butt, Join::Bevel);
        assert_eq!(
            miter.vertices.len(),
            bevel.vertices.len(),
            "sharp miter clips to a bevel"
        );
        assert_eq!(miter.indices.len(), bevel.indices.len());
    }

    #[test]
    fn closed_path_joins_every_node() {
        let tri = [(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)];
        let mesh = expand_stroke(&tri, true, 2.0, None, Cap::Butt, Join::Bevel);
        // 3 ribbon quads (12 verts, 18 idx) + 3 bevel joins (3 verts, 3 idx each).
        assert_eq!(mesh.vertices.len(), 12 + 3 * 3);
        assert_eq!(mesh.indices.len(), 18 + 3 * 3);
    }

    #[test]
    fn per_node_width_overrides_global() {
        // node 0 width 4 (half 2), node 1 width 2 (half 1).
        let mesh = expand_stroke(
            &[(0.0, 0.0), (10.0, 0.0)],
            false,
            8.0, // global ignored where per-node provided
            Some(&[4.0, 2.0]),
            Cap::Butt,
            Join::Miter,
        );
        let start_ys: Vec<f32> = mesh
            .vertices
            .iter()
            .filter(|v| approx(v[0], 0.0))
            .map(|v| v[1])
            .collect();
        let end_ys: Vec<f32> = mesh
            .vertices
            .iter()
            .filter(|v| approx(v[0], 10.0))
            .map(|v| v[1])
            .collect();
        assert!(start_ys.iter().any(|&y| approx(y, 2.0)));
        assert!(start_ys.iter().any(|&y| approx(y, -2.0)));
        assert!(end_ys.iter().any(|&y| approx(y, 1.0)));
        assert!(end_ys.iter().any(|&y| approx(y, -1.0)));
    }

    #[test]
    fn dash_pattern_repeats_across_long_line() {
        // dash [2,2] over length 20: 5 on-spans.
        let subs = dash_segments(&[(0.0, 0.0), (20.0, 0.0)], &[2.0, 2.0]);
        assert_eq!(subs.len(), 5);
        for (i, sub) in subs.iter().enumerate() {
            let start = (i as f32) * 4.0;
            assert!(approx(sub[0].0, start));
            assert!(approx(sub.last().unwrap().0, start + 2.0));
        }
    }
}
