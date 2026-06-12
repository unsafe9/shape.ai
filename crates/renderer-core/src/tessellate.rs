//! Fill tessellation, mesh caching, and megabuffer batching for the object render
//! model. Turns flattened contours into a triangle [`Mesh`] via `lyon`, memoizes
//! per object so a transform-only edit never re-tessellates (zero-lag), and merges
//! many tiny sketch meshes into one vertex/index buffer for batched draws.
//!
//! Pure CPU: no time, randomness, threads, I/O. Pointer-width-agnostic — geometry
//! coords are `i32`, ranges/indices are `u32`.

#![allow(dead_code)]

use std::collections::HashMap;

use lyon_tessellation::geom::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};

/// 8 integer units per CSS pixel; a quantized `i32` converts to f32 px by `/ 8.0`.
pub const GEOMETRY_UNITS_PER_PX: f32 = 8.0;

/// A triangulated fill mesh: `[x, y]` positions in object-local pixel space plus a
/// triangle index list. Positions only — color and transform travel separately at
/// draw time, never baked in, so a transform-only edit stays a cache hit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Per-vertex silhouette flags for analytic fill AA: `1.0` for a boundary
    /// vertex, `0.0` interior, index-aligned with [`vertices`](Mesh::vertices). An
    /// edge bordering exactly one triangle is a silhouette edge; both its endpoints
    /// are boundary vertices. Pure topology, built once with the mesh.
    pub fn boundary_flags(&self) -> Vec<f32> {
        let mut flags = vec![0.0f32; self.vertices.len()];
        // BTreeMap keeps the edge-count fold deterministic (pure-core rule).
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

/// Winding rule for what counts as "inside" when contours overlap or nest. Mirrors
/// `lyon`'s [`FillRule`] without leaking the dependency to callers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FillRuleKind {
    NonZero,
    /// Nested subpaths punch holes (a donut, a glyph counter).
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

/// Tessellate a closed-fill region from already-flattened `(closed, points)`
/// contours (object-local pixel space) into a triangle [`Mesh`] via `lyon`. All
/// contours are filled together so `fill_rule` decides holes/overlaps across the
/// set; open contours are still closed for fill purposes.
pub fn tessellate_fill(subpaths: &[(bool, Vec<(f32, f32)>)], fill_rule: FillRuleKind) -> Mesh {
    let mut builder = Path::builder();
    let mut any = false;
    for (_closed, pts) in subpaths {
        if pts.len() < 3 {
            // No fillable area; skip so lyon never sees a degenerate begin/end.
            continue;
        }
        let (fx, fy) = pts[0];
        builder.begin(point(fx, fy));
        for &(x, y) in &pts[1..] {
            builder.line_to(point(x, y));
        }
        // `close = true`: lyon seals begin->end itself, so we never duplicate the first point.
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
        // A tessellation failure yields an empty mesh rather than a panic: the
        // object draws no fill this frame (recoverable) instead of crashing.
        return Mesh::default();
    }

    Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

#[derive(Clone, Debug)]
struct CachedMesh {
    revision: u64,
    last_used: u64,
    mesh: Mesh,
}

pub const TESS_CACHE_LIMIT: usize = 8192;

/// A revision-keyed LRU cache of fill meshes keyed by `(object_id, geometry_revision)`.
/// The revision bumps only on a geometry edit, never on a transform/style edit, so
/// a drag/scale/rotate leaves it unchanged and yields a cache HIT with zero
/// re-tessellation; a geometry edit re-bakes that object alone.
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

    /// Advance the logical frame clock so `last_used` gives LRU a recency order.
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

    /// Return the cached mesh for `id` at `rev`, invoking `bake` only on a miss or
    /// stale revision, so a transform-only edit (same `rev`) never rebakes.
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

    /// Drop the cached mesh for `id` so it re-bakes on its next request even at the
    /// same revision (dirty-id invalidation path).
    pub fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }

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

    /// Evict the LRU entry while over capacity, never evicting `protect`.
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

/// The half-open range `[start, end)` of a [`MegaBuffer`]'s merged `indices` to
/// issue as one draw; the merge rebased them to point at this object's vertices.
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

/// Vertex-count threshold below which a unique mesh is small enough to merge into
/// the megabuffer (one draw, per-object ranges) rather than drawn alone.
pub const MEGABUFFER_MERGE_THRESHOLD: usize = 256;

/// A merge of many small meshes into a single vertex + index array with a
/// [`DrawRange`] per pushed mesh. Positions carry no color; per-object color/transform
/// is supplied at draw time.
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

    /// Whether `mesh` is small enough to merge (per [`MEGABUFFER_MERGE_THRESHOLD`]).
    pub fn should_merge(mesh: &Mesh) -> bool {
        mesh.vertices.len() < MEGABUFFER_MERGE_THRESHOLD
    }

    /// Merge `mesh` and return its [`DrawRange`]; indices are rebased by the current
    /// vertex count. An empty mesh records an empty range (1:1 with pushes).
    pub fn push(&mut self, mesh: &Mesh) -> DrawRange {
        let base = u32::try_from(self.vertices.len())
            .expect("megabuffer vertex count exceeds u32 index space");
        let start = u32::try_from(self.indices.len())
            .expect("megabuffer index count exceeds u32 range");

        self.vertices.extend_from_slice(&mesh.vertices);
        self.indices.reserve(mesh.indices.len());
        for &i in &mesh.indices {
            // `checked_add` so a u32-overflowing merge panics rather than wrapping.
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

/// One path-string command over object-local quantized `i32` coords; absolute, with
/// `Cubic`'s control points absolute too.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathCommand {
    MoveTo { x: i32, y: i32 },
    LineTo { x: i32, y: i32 },
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
/// quantized `i32`; convert with [`quantized_to_px`] before flattening.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedSubpath {
    pub commands: Vec<PathCommand>,
    pub closed: bool,
}

/// Convert one quantized integer coordinate to `f32` pixels (8 units/px). Done in
/// `f64` then narrowed, so large magnitudes keep full integer precision.
pub fn quantized_to_px(q: i32) -> f32 {
    (f64::from(q) / f64::from(GEOMETRY_UNITS_PER_PX)) as f32
}

/// Parse an SVG-subset path-string (`M`/`L`/`C`/`Z`, case-insensitive, absolute
/// integer coords; `C` takes two control points then the end) into subpaths. A
/// malformed command/coordinate run ends parsing, returning what parsed so far
/// (forgiving, never panics).
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
            // Not an integer; rewind so the next `next_command` still sees the token.
            self.pos = start;
            return None;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        text.trim_start_matches('+').parse::<i32>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let vcount = mesh.vertices.len() as u32;
        assert!(
            mesh.indices.iter().all(|&i| i < vcount),
            "every index addresses a real vertex"
        );
    }

    #[test]
    fn tessellate_degenerate_is_empty() {
        let empty = tessellate_fill(&[], FillRuleKind::NonZero);
        assert!(empty.is_empty());

        let two_points = tessellate_fill(&[(true, vec![(0.0, 0.0), (10.0, 0.0)])], FillRuleKind::NonZero);
        assert!(two_points.is_empty(), "a single segment has no fill area");
    }

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

        // Same object + revision (a transform-only edit).
        cache.begin_frame();
        let second = cache.get_or_insert("obj-1", 1, || bake(&mut bake_calls)).clone();

        assert_eq!(bake_calls, 1, "stable revision must not re-tessellate (P4)");
        assert_eq!(first, second, "hit returns the identical cached mesh");
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 1);
    }

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

    #[test]
    fn perf_gate_drag_10k_objects_zero_retessellation() {
        const N: usize = 10_000;
        const DRAG_FRAMES: usize = 60;

        // Capacity holds the whole set so eviction never re-bakes an object and
        // false-counts a miss; 0-rebake then measures the cache, not eviction.
        let mut cache = TessCache::with_capacity(N);

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

        cache.begin_frame();
        for (i, id) in ids.iter().enumerate() {
            cache.get_or_insert(id, 1, || bake_one(&mut bake_calls, i));
        }
        assert_eq!(cache.misses, N, "initial bake is one miss per distinct object");
        assert_eq!(cache.hits, 0, "nothing cached before the initial bake");
        assert_eq!(cache.evictions, 0, "capacity holds the whole set, no eviction");
        assert_eq!(bake_calls, N, "tessellated each object exactly once");
        let misses_after_bake = cache.misses;

        // Drag: re-request all N at the same rev for many frames (no revision bump).
        for _frame in 0..DRAG_FRAMES {
            cache.begin_frame();
            for (i, id) in ids.iter().enumerate() {
                cache.get_or_insert(id, 1, || bake_one(&mut bake_calls, i));
            }
        }

        // The drag added zero misses: every re-request was a hit, re-tessellation == 0.
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

        // One real geometry edit: bump object 0's revision -> exactly 1 extra miss.
        let misses_before_edit = cache.misses;
        let hits_before_edit = cache.hits;
        cache.begin_frame();
        for (i, id) in ids.iter().enumerate() {
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

    #[test]
    fn megabuffer_merges_three_meshes_with_correct_ranges() {
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

    #[test]
    fn megabuffer_push_empty_records_zero_range() {
        let mut mega = MegaBuffer::new();
        let r = mega.push(&Mesh::default());
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(mega.range_count(), 1);
    }

    /// A square fanned from a center point: the center is interior (shared spokes),
    /// the four corners are boundary (perimeter edges).
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
        assert_eq!(quantized_to_px(800), 100.0);
        assert_eq!(quantized_to_px(-8), -1.0);
    }

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
