//! Fill tessellation, mesh caching, and megabuffer batching for the OB-3 object
//! render model (OB3.R1, design D3/D10/P3/P4).
//!
//! Every OB-3 object carries a `geometry` path-string (SVG-subset M/L/C/Z,
//! multi-subpath, object-local quantized i32 at 8 units/px). The truth is the
//! vector path; the screen is a *derived, cached* GPU mesh (P3). This module
//! turns a flattened set of contours into a triangle [`Mesh`] via the `lyon`
//! [`FillTessellator`], memoizes those meshes per object so a transform-only edit
//! never re-tessellates (P4, zero-lag), and merges many tiny unique sketch meshes
//! into one vertex/index buffer for batched draws (D10 megabuffer).
//!
//! ADDITIVE: this is new, standalone object-pipeline groundwork. It does not touch
//! the legacy `RenderGroup/RenderCard/RenderEdge` draw path in `webgpu.rs`; the
//! OB-4 cutover wires it into the GPU draw path. `lib.rs` / `Cargo.toml` (the lyon
//! dep) are wired in the Integrate phase, not here.
//!
//! Pure CPU and host-neutral: no time, randomness, threads, I/O, or `JsValue`.
//! Pointer-width-agnostic — geometry coords are `i32`, ranges/indices are `u32`.

#![allow(dead_code)]

use std::collections::HashMap;

use lyon_tessellation::geom::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};

/// Quantization of object-local geometry coordinates: 8 integer units per CSS
/// pixel (D2). A quantized `i32` converts to an `f32` pixel coordinate by
/// `/ 8.0`. Centralized so the parser and any future stroke/outline code share
/// one source of truth.
pub const GEOMETRY_UNITS_PER_PX: f32 = 8.0;

/// A triangulated fill mesh: a flat vertex array of `[x, y]` positions in
/// object-local **pixel** space, plus a triangle index list into it.
///
/// Positions only — no per-vertex color. Per-object fill color does not live in
/// the mesh: a mesh is shared/cached/batched across draws (cache hits on
/// transform-only edits, megabuffer merging of many objects), so color travels
/// separately at draw time via an instance attribute or a per-draw uniform. The
/// object's 3x3 transform is likewise applied downstream in the vertex shader
/// (`screen = camera * M * local`), never baked into these positions, so a
/// transform-only edit is a cache hit (P4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Number of triangles (each is three indices).
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Per-vertex silhouette flags for analytic fill AA (OB3.R8 / D4): `1.0` for a
    /// boundary (silhouette) vertex, `0.0` for an interior one, index-aligned with
    /// [`vertices`](Mesh::vertices).
    ///
    /// A triangle edge that belongs to exactly one triangle lies on the mesh
    /// silhouette; an edge shared by two triangles is interior. Every endpoint of a
    /// silhouette edge is a boundary vertex. The shader treats this normalized
    /// "distance" (1 at the boundary, 0 inside) as the analytic-AA coverage helper
    /// — it fades the last screen pixel before the silhouette, so interior fans
    /// (all `0.0`) stay fully opaque. Pure topology over the tessellated index
    /// buffer: no float thresholds, no allocation on the GPU hot path (built once
    /// with the mesh).
    pub fn boundary_flags(&self) -> Vec<f32> {
        let mut flags = vec![0.0f32; self.vertices.len()];
        // Count how many triangles each undirected edge (min,max vertex index)
        // borders. A count of 1 means a silhouette edge. A BTreeMap keeps this
        // randomness-free (CLAUDE.md pure-core rule) and deterministic.
        let mut edge_counts: std::collections::BTreeMap<(u32, u32), u32> =
            std::collections::BTreeMap::new();
        for tri in self.indices.chunks_exact(3) {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                let key = if p <= q { (p, q) } else { (q, p) };
                *edge_counts.entry(key).or_insert(0) += 1;
            }
        }
        for (&(p, q), &count) in &edge_counts {
            if count == 1 {
                flags[p as usize] = 1.0;
                flags[q as usize] = 1.0;
            }
        }
        flags
    }
}

/// Which winding rule decides what counts as "inside" when a contour set
/// self-overlaps or nests (D2: multi-subpath holes use even-odd). Mirrors
/// `lyon`'s [`FillRule`] without leaking the dependency into callers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FillRuleKind {
    /// Non-zero winding (the SVG/Canvas default for a single solid contour).
    NonZero,
    /// Even-odd — nested subpaths punch holes (a donut, a glyph counter).
    EvenOdd,
}

impl FillRuleKind {
    fn to_lyon(self) -> FillRule {
        match self {
            FillRuleKind::NonZero => FillRule::NonZero,
            FillRuleKind::EvenOdd => FillRule::EvenOdd,
        }
    }
}

/// Tessellate a closed-fill region from already-flattened contours into a
/// triangle [`Mesh`] using the `lyon` [`FillTessellator`].
///
/// `subpaths` is one entry per contour: `(closed, points)` where `points` are
/// flattened polyline vertices in object-local **pixel** space (curves already
/// reduced to line segments by the caller's LOD flattener). A subpath flagged
/// `closed` is sealed back to its first point; a contour with fewer than three
/// points contributes nothing. Multiple subpaths are tessellated together so the
/// `fill_rule` (even-odd vs non-zero) decides holes/overlaps across the whole set
/// (D2 multi-subpath). Open contours are still closed for fill purposes — fill is
/// a closed-region operation; open-path stroking is a separate concern.
pub fn tessellate_fill(subpaths: &[(bool, Vec<(f32, f32)>)], fill_rule: FillRuleKind) -> Mesh {
    let mut builder = Path::builder();
    let mut any = false;
    for (_closed, pts) in subpaths {
        if pts.len() < 3 {
            // A point or single segment has no fillable area; skip it so lyon
            // never sees a degenerate sub-path begin/end.
            continue;
        }
        let (fx, fy) = pts[0];
        builder.begin(point(fx, fy));
        for &(x, y) in &pts[1..] {
            builder.line_to(point(x, y));
        }
        // Fill always treats a contour as closed: lyon seals begin->end itself
        // when we pass `close = true`, so we never duplicate the first point.
        builder.end(true);
        any = true;
    }
    let path = builder.build();
    if !any {
        return Mesh::default();
    }

    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();
    let options = FillOptions::default().with_fill_rule(fill_rule.to_lyon());
    let result = tessellator.tessellate_path(
        &path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
            let p = vertex.position();
            [p.x, p.y]
        }),
    );
    if result.is_err() {
        // A tessellation failure (e.g. a pathological self-intersection) yields an
        // empty mesh rather than a panic: the object simply draws no fill this
        // frame, which is recoverable, instead of taking down the render loop.
        return Mesh::default();
    }

    Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

/// One cached mesh tagged with the geometry revision it was baked at and the
/// frame it was last used (for LRU recency).
#[derive(Clone, Debug)]
struct CachedMesh {
    revision: u64,
    last_used: u64,
    mesh: Mesh,
}

/// Default cap on cached meshes. Bounds the resident working set to the visible +
/// prefetch set for realistic viewports rather than the whole (unbounded) scene;
/// mirrors `render_cache::RENDER_DATA_CACHE_LIMIT`.
pub const TESS_CACHE_LIMIT: usize = 8192;

/// A revision-keyed LRU cache of tessellated fill meshes, keyed by
/// `(object_id, geometry_revision)`.
///
/// The key insight for zero-lag (P4): the geometry revision bumps *only* on a
/// geometry edit, never on a transform/style edit. So dragging, scaling, or
/// rotating an object — which changes its 3x3 matrix but not its path — leaves the
/// revision unchanged and yields a cache HIT with zero re-tessellation. A
/// geometry edit bumps the revision, producing a re-bake of that object alone.
///
/// This is a fresh, simpler reimplementation of the idea in `render_cache.rs`
/// (`RenderDataCache`), specialized to [`Mesh`] so the OB-3 object pipeline owns
/// its tessellation cache without coupling to the legacy card/edge cache.
#[derive(Clone, Debug)]
pub struct TessCache {
    entries: HashMap<String, CachedMesh>,
    capacity: usize,
    frame: u64,
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
}

impl TessCache {
    pub fn new() -> Self {
        Self::with_capacity(TESS_CACHE_LIMIT)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        TessCache {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            frame: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Advance the logical frame clock so `last_used` reflects the frame an entry
    /// was actually requested, giving LRU eviction a meaningful recency order.
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

    /// The revision an entry is cached at, or `None` if absent.
    pub fn cached_revision(&self, id: &str) -> Option<u64> {
        self.entries.get(id).map(|e| e.revision)
    }

    /// Return the cached mesh for `id` at `rev`, baking it via `bake` on a miss or
    /// a stale (different) revision. `bake` runs only when a re-tessellation is
    /// actually needed, so a transform-only edit (same `rev`) is a hit and never
    /// rebakes (P4, 0-rebake).
    pub fn get_or_insert<F>(&mut self, id: &str, rev: u64, bake: F) -> &Mesh
    where
        F: FnOnce() -> Mesh,
    {
        let frame = self.frame;
        let hit = matches!(self.entries.get(id), Some(e) if e.revision == rev);
        if hit {
            self.hits += 1;
            let entry = self.entries.get_mut(id).expect("entry present on hit");
            entry.last_used = frame;
        } else {
            self.misses += 1;
            let mesh = bake();
            self.entries.insert(
                id.to_string(),
                CachedMesh {
                    revision: rev,
                    last_used: frame,
                    mesh,
                },
            );
            self.evict_if_needed(id);
        }
        &self
            .entries
            .get(id)
            .expect("entry present after insert")
            .mesh
    }

    /// Drop the cached mesh for `id`, forcing a re-bake on its next request even
    /// at the same revision. Used by the dirty-id invalidation path.
    pub fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }

    /// Invalidate every id in `dirty_ids` — the dirty set the patch path already
    /// collects on a geometry edit.
    pub fn invalidate_all<I, S>(&mut self, dirty_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for id in dirty_ids {
            self.entries.remove(id.as_ref());
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Evict the least-recently-used entry while over capacity, never evicting
    /// `protect` (the entry just inserted this frame).
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

impl Default for TessCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Where one object's geometry lives inside a [`MegaBuffer`]'s merged index
/// array: the half-open range `[start, end)` of `indices` to issue as one draw.
/// Indices in that range already point at the object's vertices inside the merged
/// vertex array (the merge rebased them), so a single bound buffer plus this
/// range draws exactly this object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DrawRange {
    pub start: u32,
    pub end: u32,
}

impl DrawRange {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Vertex-count threshold below which a unique mesh is considered "small" and
/// worth merging into the megabuffer rather than drawn alone. Many tiny sketch
/// meshes (freehand scribbles) each cost a draw call on their own; merging them
/// into one buffer collapses that to one draw with per-object ranges (D10).
/// Larger meshes are better drawn individually (instancing / their own buffer),
/// so they are not merged. Tunable against benchmark evidence.
pub const MEGABUFFER_MERGE_THRESHOLD: usize = 256;

/// A merge of many small unique meshes into a single vertex array and a single
/// index array, with a [`DrawRange`] per pushed mesh (OB3.R1 / D10 megabuffer
/// batching).
///
/// Each [`push`](MegaBuffer::push) appends a mesh's vertices to the shared vertex
/// array and its indices — rebased by the running vertex offset — to the shared
/// index array, returning the index range covering just that mesh. After merging
/// many scribbles, the renderer uploads `vertices`/`indices` once and issues one
/// draw per range (or, when ranges are contiguous and share state, fewer), instead
/// of one buffer + one draw per object.
///
/// As with [`Mesh`], positions are object-local and carry no color; per-object
/// color/transform is supplied at draw time via the range's instance/uniform.
#[derive(Clone, Debug, Default)]
pub struct MegaBuffer {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// One range per `push`, in push order, indexing into `indices`.
    pub ranges: Vec<DrawRange>,
}

impl MegaBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `mesh` is small enough to be worth merging here, per
    /// [`MEGABUFFER_MERGE_THRESHOLD`]. The caller routes larger meshes to their
    /// own buffer/instanced draw instead.
    pub fn should_merge(mesh: &Mesh) -> bool {
        mesh.vertices.len() < MEGABUFFER_MERGE_THRESHOLD
    }

    /// Merge `mesh` into the buffers and return its [`DrawRange`] in the merged
    /// index array. Indices are rebased by the current vertex count so they point
    /// at this mesh's vertices within the merged vertex array. An empty mesh
    /// yields an empty range at the current index offset (still recorded, so per-
    /// object range bookkeeping stays 1:1 with pushes).
    pub fn push(&mut self, mesh: &Mesh) -> DrawRange {
        let base = u32::try_from(self.vertices.len())
            .expect("megabuffer vertex count exceeds u32 index space");
        let start = u32::try_from(self.indices.len())
            .expect("megabuffer index count exceeds u32 range");

        self.vertices.extend_from_slice(&mesh.vertices);
        self.indices.reserve(mesh.indices.len());
        for &i in &mesh.indices {
            // Rebase each local index into the merged vertex array. `checked_add`
            // keeps the conversion non-lossy if the merged buffer ever overflows
            // u32 rather than silently wrapping.
            let rebased = base
                .checked_add(i)
                .expect("megabuffer rebased index exceeds u32 range");
            self.indices.push(rebased);
        }

        let end = u32::try_from(self.indices.len())
            .expect("megabuffer index count exceeds u32 range");
        let range = DrawRange { start, end };
        self.ranges.push(range);
        range
    }

    /// Total merged vertices.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Number of meshes merged so far (one per `push`).
    pub fn range_count(&self) -> usize {
        self.ranges.len()
    }
}

/// One drawing command parsed from a geometry path-string: a minimal SVG-subset
/// (M/L/C/Z) over object-local quantized `i32` coordinates (D2). Absolute coords;
/// `Cubic` carries absolute control points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathCommand {
    /// Move-to: begin a new subpath at this absolute quantized point.
    MoveTo { x: i32, y: i32 },
    /// Line-to: straight segment to this absolute quantized point.
    LineTo { x: i32, y: i32 },
    /// Cubic Bezier with two absolute control points and an absolute end point.
    Cubic {
        c1x: i32,
        c1y: i32,
        c2x: i32,
        c2y: i32,
        x: i32,
        y: i32,
    },
    /// Close the current subpath back to its move-to point.
    Close,
}

/// A parsed subpath: its commands plus whether a `Z` closed it. Coordinates stay
/// quantized `i32`; convert to pixels with [`quantized_to_px`] before flattening.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedSubpath {
    pub commands: Vec<PathCommand>,
    pub closed: bool,
}

/// Convert one object-local quantized integer coordinate to `f32` pixels (D2:
/// 8 units/px). Done in `f64` then narrowed, so large quantized magnitudes keep
/// full integer precision before the final `f32` narrowing.
pub fn quantized_to_px(q: i32) -> f32 {
    (f64::from(q) / f64::from(GEOMETRY_UNITS_PER_PX)) as f32
}

/// Parse an SVG-subset path-string into subpaths (D2). Supports `M`/`L`/`C`/`Z`
/// (case-insensitive), absolute integer coordinates only; `C` takes two absolute
/// control points then the end point. Whitespace and commas separate tokens. An
/// unrecognized command or a malformed/short coordinate run ends parsing at that
/// point, returning what parsed cleanly so far (forgiving, never panics).
///
/// This is a small local parser by design: the renderer crate stays standalone
/// and must not depend on scene-core. It mirrors scene-core's encoding (absolute
/// integer coords; cubic control points absolute; node handles relative to nodes
/// are resolved into absolute control points before this string is produced).
pub fn parse_path(path: &str) -> Vec<ParsedSubpath> {
    let mut subpaths: Vec<ParsedSubpath> = Vec::new();
    let mut current: Option<ParsedSubpath> = None;
    let mut tokens = PathTokens::new(path);

    while let Some(cmd) = tokens.next_command() {
        match cmd {
            'M' | 'm' => {
                if let (Some(x), Some(y)) = (tokens.next_int(), tokens.next_int()) {
                    if let Some(sub) = current.take() {
                        subpaths.push(sub);
                    }
                    current = Some(ParsedSubpath {
                        commands: vec![PathCommand::MoveTo { x, y }],
                        closed: false,
                    });
                } else {
                    break;
                }
            }
            'L' | 'l' => {
                if let (Some(x), Some(y)) = (tokens.next_int(), tokens.next_int()) {
                    match current.as_mut() {
                        Some(sub) => sub.commands.push(PathCommand::LineTo { x, y }),
                        None => break,
                    }
                } else {
                    break;
                }
            }
            'C' | 'c' => {
                let coords = (
                    tokens.next_int(),
                    tokens.next_int(),
                    tokens.next_int(),
                    tokens.next_int(),
                    tokens.next_int(),
                    tokens.next_int(),
                );
                if let (Some(c1x), Some(c1y), Some(c2x), Some(c2y), Some(x), Some(y)) = coords {
                    match current.as_mut() {
                        Some(sub) => sub.commands.push(PathCommand::Cubic {
                            c1x,
                            c1y,
                            c2x,
                            c2y,
                            x,
                            y,
                        }),
                        None => break,
                    }
                } else {
                    break;
                }
            }
            'Z' | 'z' => match current.as_mut() {
                Some(sub) => {
                    sub.commands.push(PathCommand::Close);
                    sub.closed = true;
                }
                None => break,
            },
            _ => break,
        }
    }
    if let Some(sub) = current.take() {
        subpaths.push(sub);
    }
    subpaths
}

/// A forgiving tokenizer over a path-string: yields command letters and signed
/// integers, treating whitespace and commas as separators.
struct PathTokens<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> PathTokens<'a> {
    fn new(s: &'a str) -> Self {
        PathTokens {
            bytes: s.as_bytes(),
            pos: 0,
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

    /// Advance to and consume the next command letter, or `None` at end of input.
    fn next_command(&mut self) -> Option<char> {
        self.skip_separators();
        if self.pos >= self.bytes.len() {
            return None;
        }
        let b = self.bytes[self.pos];
        if b.is_ascii_alphabetic() {
            self.pos += 1;
            Some(b as char)
        } else {
            // A stray non-letter where a command is expected: stop cleanly.
            None
        }
    }

    /// Parse the next signed integer token, or `None` if the next token is not a
    /// valid integer (e.g. a command letter or end of input).
    fn next_int(&mut self) -> Option<i32> {
        self.skip_separators();
        let start = self.pos;
        if self.pos < self.bytes.len() && (self.bytes[self.pos] == b'-' || self.bytes[self.pos] == b'+') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos == digits_start {
            // No digits consumed: not an integer. Rewind so a command letter is
            // still seen by the next `next_command`.
            self.pos = start;
            return None;
        }
        // Safe: the slice is ASCII sign + digits by construction.
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        text.trim_start_matches('+').parse::<i32>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit (well, 100px) rect tessellates to a non-empty mesh with at least two
    /// triangles (>= 6 indices) — the minimum for a filled quad.
    #[test]
    fn tessellate_rect_yields_two_triangles() {
        let rect = vec![(
            true,
            vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        )];
        let mesh = tessellate_fill(&rect, FillRuleKind::NonZero);

        assert!(!mesh.is_empty(), "rect must tessellate to a non-empty mesh");
        assert!(
            mesh.vertices.len() >= 4,
            "a quad needs at least its 4 corners, got {}",
            mesh.vertices.len()
        );
        assert!(
            mesh.indices.len() >= 6,
            ">= 2 triangles expected (>= 6 indices), got {}",
            mesh.indices.len()
        );
        assert_eq!(
            mesh.indices.len() % 3,
            0,
            "indices must form whole triangles"
        );
        assert!(
            mesh.triangle_count() >= 2,
            "rect is at least 2 triangles, got {}",
            mesh.triangle_count()
        );
        // Every index must address a real vertex.
        let vcount = mesh.vertices.len() as u32;
        assert!(
            mesh.indices.iter().all(|&i| i < vcount),
            "every index addresses a real vertex"
        );
    }

    /// A degenerate contour (< 3 points) and an empty contour set both yield an
    /// empty mesh rather than garbage or a panic.
    #[test]
    fn tessellate_degenerate_is_empty() {
        let empty = tessellate_fill(&[], FillRuleKind::NonZero);
        assert!(empty.is_empty());

        let two_points = tessellate_fill(&[(true, vec![(0.0, 0.0), (10.0, 0.0)])], FillRuleKind::NonZero);
        assert!(two_points.is_empty(), "a single segment has no fill area");
    }

    /// Even-odd nesting punches a hole: a small square inside a big square fills
    /// only the ring, so it produces fewer covered triangles than the same outer
    /// square alone under the same rule. We assert the hole changes the result.
    #[test]
    fn even_odd_punches_hole() {
        let outer_only = tessellate_fill(
            &[(
                true,
                vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
            )],
            FillRuleKind::EvenOdd,
        );
        let donut = tessellate_fill(
            &[
                (
                    true,
                    vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
                ),
                (
                    true,
                    vec![(25.0, 25.0), (75.0, 25.0), (75.0, 75.0), (25.0, 75.0)],
                ),
            ],
            FillRuleKind::EvenOdd,
        );
        assert!(!outer_only.is_empty());
        assert!(!donut.is_empty(), "the ring must still fill");
        assert_ne!(
            outer_only.indices.len(),
            donut.indices.len(),
            "even-odd hole must change the triangulation vs the solid square"
        );
    }

    /// A second request at the same revision is a cache HIT and never re-bakes —
    /// the transform-only-edit / zero-rebake property (P4). A counter proves the
    /// bake closure ran exactly once.
    #[test]
    fn cache_hit_returns_same_mesh_without_recompute() {
        let mut cache = TessCache::new();
        let mut bake_calls = 0;

        let bake = |calls: &mut i32| {
            *calls += 1;
            tessellate_fill(
                &[(
                    true,
                    vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
                )],
                FillRuleKind::NonZero,
            )
        };

        cache.begin_frame();
        let first = cache.get_or_insert("obj-1", 1, || bake(&mut bake_calls)).clone();
        assert!(!first.is_empty());

        // Frame 2: same object, same geometry revision (a transform-only edit).
        cache.begin_frame();
        let second = cache.get_or_insert("obj-1", 1, || bake(&mut bake_calls)).clone();

        assert_eq!(bake_calls, 1, "stable revision must not re-tessellate (P4)");
        assert_eq!(first, second, "hit returns the identical cached mesh");
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 1);
    }

    /// Bumping the geometry revision forces a re-bake of that object alone.
    #[test]
    fn bumped_revision_rebakes() {
        let mut cache = TessCache::new();
        let mut bake_calls = 0;
        let mut bake = || {
            bake_calls += 1;
            tessellate_fill(
                &[(true, vec![(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 5.0)])],
                FillRuleKind::NonZero,
            )
        };

        cache.begin_frame();
        cache.get_or_insert("obj-1", 1, &mut bake);
        cache.begin_frame();
        cache.get_or_insert("obj-1", 2, &mut bake);

        assert_eq!(bake_calls, 2, "geometry edit (rev bump) re-bakes");
        assert_eq!(cache.cached_revision("obj-1"), Some(2));
        assert_eq!(cache.misses, 2);
        assert_eq!(cache.hits, 0);
    }

    /// OB5.1 zero-lag perf gate (DONE: "1만 object 드래그 = 재tessellation 0").
    ///
    /// Bake 10_000 distinct object meshes once (the initial tessellation = 10_000
    /// misses), then simulate a sustained DRAG: re-request every object many
    /// frames at the SAME geometry revision (a transform-only edit does not bump
    /// the revision). The gate is that the drag adds ZERO further misses — every
    /// request is a cache hit, so the dragged frames re-tessellate nothing (P4).
    /// Finally a single real geometry edit (one revision bump) re-bakes exactly
    /// that one object and nothing else.
    #[test]
    fn perf_gate_drag_10k_objects_zero_retessellation() {
        const N: usize = 10_000;
        const DRAG_FRAMES: usize = 60;

        // Capacity must hold the whole working set: at the default 8192 cap the
        // LRU would evict during the initial bake, and an evicted object would
        // re-bake on the next drag frame and falsely count as a miss. Size the
        // cache to the full set so 0-rebake measures the cache, not eviction.
        let mut cache = TessCache::with_capacity(N);

        // Each object gets a unique id and a unique geometry. A shared bake-call
        // counter (the ground-truth tessellation count) lets us assert the bake
        // closure ran exactly N times across the whole scenario.
        let ids: Vec<String> = (0..N).map(|i| format!("obj-{i}")).collect();
        let mut bake_calls = 0usize;
        let bake_one = |calls: &mut usize, i: usize| {
            *calls += 1;
            let f = i as f32;
            tessellate_fill(
                &[(
                    true,
                    vec![(f, f), (f + 10.0, f), (f + 10.0, f + 10.0), (f, f + 10.0)],
                )],
                FillRuleKind::NonZero,
            )
        };

        // --- Initial bake: 10_000 distinct (id, rev=1) -> 10_000 misses. -------
        cache.begin_frame();
        for (i, id) in ids.iter().enumerate() {
            cache.get_or_insert(id, 1, || bake_one(&mut bake_calls, i));
        }
        assert_eq!(cache.misses, N, "initial bake is one miss per distinct object");
        assert_eq!(cache.hits, 0, "nothing cached before the initial bake");
        assert_eq!(cache.evictions, 0, "capacity holds the whole set, no eviction");
        assert_eq!(bake_calls, N, "tessellated each object exactly once");
        let misses_after_bake = cache.misses;

        // --- Drag: re-request all 10_000 at the SAME rev for many frames. ------
        // A transform-only edit (drag) does not bump the geometry revision, so
        // every one of these N * DRAG_FRAMES requests must be a cache hit.
        for _frame in 0..DRAG_FRAMES {
            cache.begin_frame();
            for (i, id) in ids.iter().enumerate() {
                // The bake closure must NEVER run during the drag; if it does, the
                // shared counter moves past N and the final assert fails. The
                // additional-misses assert below is the primary 0-rebake gate.
                cache.get_or_insert(id, 1, || bake_one(&mut bake_calls, i));
            }
        }

        // THE 0-REBAKE GATE: the drag added zero misses. Every re-request at the
        // unchanged revision was a hit, so re-tessellation during the drag == 0.
        assert_eq!(
            cache.misses - misses_after_bake,
            0,
            "1만 object 드래그 = 재tessellation 0: a transform-only drag must not re-tessellate (P4)"
        );
        assert_eq!(
            bake_calls, N,
            "the bake closure never ran during the drag — still exactly N total bakes"
        );
        assert_eq!(
            cache.hits,
            N * DRAG_FRAMES,
            "every dragged request across every frame was a cache hit"
        );
        assert_eq!(cache.evictions, 0, "no eviction perturbed the resident set");

        // --- One real geometry edit: bump ONE object's revision -> exactly 1 ---
        // additional miss (only that object rebakes, P4), the other 9_999 stay
        // hits.
        let misses_before_edit = cache.misses;
        let hits_before_edit = cache.hits;
        cache.begin_frame();
        for (i, id) in ids.iter().enumerate() {
            // Object 0 gets a bumped revision (a genuine geometry edit); the rest
            // stay at rev 1.
            let rev = if i == 0 { 2 } else { 1 };
            cache.get_or_insert(id, rev, || bake_one(&mut bake_calls, i));
        }
        assert_eq!(
            cache.misses - misses_before_edit,
            1,
            "a single geometry edit re-bakes exactly one object, not the scene"
        );
        assert_eq!(
            cache.hits - hits_before_edit,
            N - 1,
            "the other 9_999 unchanged objects are still hits"
        );
        assert_eq!(bake_calls, N + 1, "exactly one extra bake for the one edited object");
        assert_eq!(cache.cached_revision("obj-0"), Some(2), "edited object cached at new rev");
    }

    /// Explicit dirty-id invalidation re-bakes even at the same revision.
    #[test]
    fn invalidate_forces_rebake_at_same_revision() {
        let mut cache = TessCache::new();
        let mut bake_calls = 0;
        let mut bake = || {
            bake_calls += 1;
            tessellate_fill(
                &[(true, vec![(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 5.0)])],
                FillRuleKind::NonZero,
            )
        };

        cache.begin_frame();
        cache.get_or_insert("obj-1", 1, &mut bake);
        cache.invalidate_all(["obj-1", "absent"]);
        assert_eq!(cache.cached_revision("obj-1"), None);

        cache.begin_frame();
        cache.get_or_insert("obj-1", 1, &mut bake);
        assert_eq!(bake_calls, 2, "invalidated entry re-bakes at same revision");
    }

    /// LRU eviction drops the least-recently-used mesh over capacity, never the
    /// one just inserted this frame.
    #[test]
    fn lru_eviction_over_capacity() {
        let mut cache = TessCache::with_capacity(2);
        let empty = || Mesh::default();

        cache.begin_frame();
        cache.get_or_insert("a", 1, empty);
        cache.begin_frame();
        cache.get_or_insert("b", 1, empty);
        // Touch "a" so "b" is now LRU.
        cache.begin_frame();
        cache.get_or_insert("a", 1, empty);
        // Insert "c": over capacity, "b" is evicted.
        cache.begin_frame();
        cache.get_or_insert("c", 1, empty);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.evictions, 1);
        assert_eq!(cache.cached_revision("b"), None, "LRU victim evicted");
        assert_eq!(cache.cached_revision("a"), Some(1));
        assert_eq!(cache.cached_revision("c"), Some(1));
    }

    /// The megabuffer merges 3 meshes into one vertex/index buffer with 3 correct
    /// ranges. Each range covers exactly its mesh's indices, ranges are contiguous
    /// and cover the whole index array, and indices are rebased so they address
    /// each mesh's own vertices inside the merged vertex array.
    #[test]
    fn megabuffer_merges_three_meshes_with_correct_ranges() {
        // Three distinct triangles, so we can check rebasing precisely.
        let tri = |ox: f32| Mesh {
            vertices: vec![[ox, 0.0], [ox + 1.0, 0.0], [ox, 1.0]],
            indices: vec![0, 1, 2],
        };
        let m0 = tri(0.0);
        let m1 = tri(10.0);
        let m2 = tri(20.0);

        let mut mega = MegaBuffer::new();
        let r0 = mega.push(&m0);
        let r1 = mega.push(&m1);
        let r2 = mega.push(&m2);

        assert_eq!(mega.range_count(), 3, "one range per push");
        assert_eq!(mega.vertex_count(), 9, "3 triangles * 3 verts merged");
        assert_eq!(mega.indices.len(), 9, "3 triangles * 3 indices merged");
        assert_eq!(mega.ranges, vec![r0, r1, r2]);

        // Ranges are contiguous and tile the whole index array.
        assert_eq!(r0, DrawRange { start: 0, end: 3 });
        assert_eq!(r1, DrawRange { start: 3, end: 6 });
        assert_eq!(r2, DrawRange { start: 6, end: 9 });
        assert_eq!(r0.end, r1.start);
        assert_eq!(r1.end, r2.start);

        // Indices are rebased: mesh 1's indices point at verts 3..6, mesh 2's at
        // 6..9, not all at 0..3.
        assert_eq!(&mega.indices[0..3], &[0, 1, 2]);
        assert_eq!(&mega.indices[3..6], &[3, 4, 5]);
        assert_eq!(&mega.indices[6..9], &[6, 7, 8]);

        // The merged vertices preserve each mesh's positions in push order.
        assert_eq!(mega.vertices[0], [0.0, 0.0]);
        assert_eq!(mega.vertices[3], [10.0, 0.0]);
        assert_eq!(mega.vertices[6], [20.0, 0.0]);
    }

    /// Pushing an empty mesh still records a (zero-length) range so per-object
    /// bookkeeping stays 1:1 with pushes.
    #[test]
    fn megabuffer_push_empty_records_zero_range() {
        let mut mega = MegaBuffer::new();
        let r = mega.push(&Mesh::default());
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(mega.range_count(), 1);
    }

    /// Analytic-AA boundary detection (D4): a vertex on a silhouette edge (one
    /// bordering triangle) flags 1.0; a purely interior vertex stays 0.0. A square
    /// fanned from a center point gives a known interior vertex (the center, all of
    /// whose spoke edges are shared by two triangles) and four boundary corners (on
    /// the perimeter edges, each bordering a single triangle). Fails if the flags
    /// stay all-zero or if the interior center is wrongly marked.
    #[test]
    fn boundary_flags_mark_silhouette_not_interior() {
        let mesh = Mesh {
            // 0..3 = corners, 4 = center.
            vertices: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [1.0, 1.0]],
            indices: vec![0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4],
        };
        let flags = mesh.boundary_flags();
        assert_eq!(flags.len(), mesh.vertices.len());
        // Every perimeter corner is a boundary vertex.
        assert_eq!(&flags[0..4], &[1.0, 1.0, 1.0, 1.0], "corners are silhouette");
        // The fan center borders only shared spokes -> interior.
        assert_eq!(flags[4], 0.0, "fan center stays interior");
        assert!(flags.iter().any(|&e| e != 0.0), "not all-zero");
    }

    /// A real lyon-tessellated fill flags silhouette vertices (so the FS can AA the
    /// edge) without marking the whole mesh — interior fans stay 0.0.
    #[test]
    fn boundary_flags_on_tessellated_fill_is_mixed() {
        // A plus/cross polygon tessellates with both boundary and interior verts.
        let cross = vec![(
            true,
            vec![
                (1.0, 0.0), (2.0, 0.0), (2.0, 1.0), (3.0, 1.0), (3.0, 2.0),
                (2.0, 2.0), (2.0, 3.0), (1.0, 3.0), (1.0, 2.0), (0.0, 2.0),
                (0.0, 1.0), (1.0, 1.0),
            ],
        )];
        let mesh = tessellate_fill(&cross, FillRuleKind::NonZero);
        assert!(!mesh.is_empty(), "cross must tessellate");
        let flags = mesh.boundary_flags();
        assert_eq!(flags.len(), mesh.vertices.len());
        assert!(flags.iter().any(|&e| e != 0.0), "some boundary vertices flagged");
    }

    /// `should_merge` routes small meshes into the megabuffer and large ones away.
    #[test]
    fn should_merge_respects_threshold() {
        let small = Mesh {
            vertices: vec![[0.0, 0.0]; 4],
            indices: vec![],
        };
        let large = Mesh {
            vertices: vec![[0.0, 0.0]; MEGABUFFER_MERGE_THRESHOLD + 1],
            indices: vec![],
        };
        assert!(MegaBuffer::should_merge(&small));
        assert!(!MegaBuffer::should_merge(&large));
    }

    /// The path parser handles a closed rect with absolute integer coords and Z.
    #[test]
    fn parse_path_closed_rect() {
        // 100px square at 8 units/px -> 800 quantized units.
        let subs = parse_path("M0 0 L800 0 L800 800 L0 800 Z");
        assert_eq!(subs.len(), 1);
        let sub = &subs[0];
        assert!(sub.closed);
        assert_eq!(
            sub.commands,
            vec![
                PathCommand::MoveTo { x: 0, y: 0 },
                PathCommand::LineTo { x: 800, y: 0 },
                PathCommand::LineTo { x: 800, y: 800 },
                PathCommand::LineTo { x: 0, y: 800 },
                PathCommand::Close,
            ]
        );
        // Quantized -> px conversion at 8 units/px.
        assert_eq!(quantized_to_px(800), 100.0);
        assert_eq!(quantized_to_px(-8), -1.0);
    }

    /// Multi-subpath, cubic commands, negatives, and comma separators all parse.
    #[test]
    fn parse_path_multi_subpath_and_cubic() {
        let subs = parse_path("M0 0 C10 -20 30 40 50 50 Z M100,100 L120,140");
        assert_eq!(subs.len(), 2, "two subpaths split on the second M");

        assert!(subs[0].closed);
        assert_eq!(
            subs[0].commands,
            vec![
                PathCommand::MoveTo { x: 0, y: 0 },
                PathCommand::Cubic {
                    c1x: 10,
                    c1y: -20,
                    c2x: 30,
                    c2y: 40,
                    x: 50,
                    y: 50,
                },
                PathCommand::Close,
            ]
        );

        assert!(!subs[1].closed);
        assert_eq!(
            subs[1].commands,
            vec![
                PathCommand::MoveTo { x: 100, y: 100 },
                PathCommand::LineTo { x: 120, y: 140 },
            ]
        );
    }

    /// A malformed tail (a command missing its coordinates) is dropped, but the
    /// clean prefix still parses — forgiving, never panics.
    #[test]
    fn parse_path_truncated_is_forgiving() {
        let subs = parse_path("M0 0 L10 10 L");
        assert_eq!(subs.len(), 1);
        assert_eq!(
            subs[0].commands,
            vec![
                PathCommand::MoveTo { x: 0, y: 0 },
                PathCommand::LineTo { x: 10, y: 10 },
            ],
            "the trailing bare L is dropped"
        );

        assert!(parse_path("").is_empty());
    }
}
