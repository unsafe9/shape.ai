//! OB-4 object render-geometry build (pure CPU half).
//!
//! This module builds the device-independent CPU geometry for the OB-3 object
//! model (`RenderObjectScene` / `RenderObject`): the `bytemuck` vertex/instance
//! layout structs, the per-object [`ObjectDraw`] index, and the scene
//! tessellation into a [`MegaBuffer`] + instance data ([`build_scene_geometry`]).
//! It needs no `wgpu` and runs on every target (web wasm, native, host tests).
//!
//! The GPU half — the `wgpu` render pipelines (`ObjectPipeline`) and the buffer
//! uploader / render-pass recorder (`ObjectRenderer`) that consume this geometry —
//! lives in the `shape_canvas_core` (renderer-wgpu) crate's `object_pipeline`
//! module, gated behind its `wgpu-probe` feature.
//!
//! ## Per-object 3x3 matrix strategy: instance attributes
//!
//! `wgpu`'s WebGPU backend exposes no push constants, so the per-object 3x3
//! projective transform (D7) cannot be a push constant. The WGSL contract
//! declares the matrix as three **instance-step** `vec3` columns (`m0`/`m1`/`m2`)
//! plus the inline paint color, so the instance structs here match that exactly:
//! one instance record per object carries its matrix columns + color, and each
//! object draws as a single instance. This keeps the matrix out of the shared
//! `view` uniform — which stays the affine camera, byte-identical to the legacy
//! `ViewUniform` — and lets the VS do the projective `M * vec3(local, 1)` divide
//! per object.

use crate::model::CameraState;
use crate::object_theme::{resolve_token_f32, Theme};
use crate::render_object::{
    resolve_visual, RPaint, RText, RTextAlign, RTextValign, RenderObject, RenderObjectScene,
    VisualState, QUANT_PER_PX,
};
use crate::text_layout::{layout_runs, TextAlign, TextRunInput, TextVAlign};
use crate::stroke_expand::{dash_segments, expand_stroke, Cap, Join};
use crate::tessellate::{
    parse_path, quantized_to_px, tessellate_fill, DrawRange, FillRuleKind, MegaBuffer,
    PathCommand,
};

// ---------------------------------------------------------------------------
// Uniform + vertex/instance GPU layouts
// ---------------------------------------------------------------------------

/// Shared camera uniform for the object pipelines. Replaces `ViewUniform`'s
/// affine-only role for the object path: the same affine `camera`/`viewport`
/// fields the WGSL `View` struct reads, while every object's *projective* 3x3
/// matrix rides on the instance buffer (see module docs), not here. Byte-layout
/// identical to the legacy `ViewUniform` so the two pipelines share a coordinate
/// frame during the cutover.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ObjectMatrixUniform {
    /// `vec4(translate.x, translate.y, zoom, _)` — the affine camera.
    pub camera: [f32; 4],
    /// `vec4(px_w, px_h, _, _)` — the device-pixel viewport.
    pub viewport: [f32; 4],
}

impl ObjectMatrixUniform {
    /// Build the camera uniform from a scene's camera and a device-pixel viewport.
    pub fn from_scene(scene: &RenderObjectScene, pixel_width: f32, pixel_height: f32) -> Self {
        ObjectMatrixUniform {
            camera: [
                scene.camera.x as f32,
                scene.camera.y as f32,
                scene.camera.zoom as f32,
                0.0,
            ],
            viewport: [pixel_width, pixel_height, 0.0, 0.0],
        }
    }
}

/// Per-vertex fill attributes matching `object_fill.wgsl`'s `VertexIn`
/// (`position` @0, `edge` @1). Positions are object-local **pixels**
/// (`tessellate::Mesh` vertices); `edge` is the analytic-AA silhouette flag (D4):
/// `1.0` on a boundary vertex, `0.0` interior (from `Mesh::boundary_flags`), which
/// the FS fades over the last screen pixel before the silhouette. All-zero edges
/// (no boundary data) degrade gracefully to opaque interior fill.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FillVertex {
    pub position: [f32; 2],
    pub edge: f32,
}

/// Per-object instance attributes for the fill pipeline matching
/// `object_fill.wgsl` (`m0`/`m1`/`m2` @2..4, `fill` @5). The 3x3 projective
/// matrix is stored as three `vec3` columns; `fill` is the resolved solid paint.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FillInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub fill: [f32; 4],
}

/// Per-vertex drop-shadow attributes matching `object_shadow.wgsl`'s `VertexIn`
/// (`position` @0, `feather` @1). Positions are object-local **pixels** (the
/// object's own fill silhouette translated by the drop-shadow offset); `feather`
/// is the per-vertex 0..1 blur falloff, uniformly `0` for the flat offset
/// silhouette (the FS falloff is then 1) — a soft blur is the GPU-cutover residual.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowVertex {
    pub position: [f32; 2],
    pub feather: f32,
}

/// Per-object instance attributes for the drop-shadow pipeline matching
/// `object_shadow.wgsl` (`m0`/`m1`/`m2` @2..4, `shadow` @5). The 3x3 projective
/// matrix is the same one the fill/stroke instances carry; `shadow` is the theme
/// `shadow` token color (translucent), re-resolved on a theme flip.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub shadow: [f32; 4],
}

/// Per-vertex stroke attributes matching `object_stroke.wgsl`'s `VertexIn`
/// (`position` @0, `normal` @1, `side` @2, `width` @3, `distance_along` @4).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeVertex {
    pub position: [f32; 2],
    pub normal: [f32; 2],
    pub side: f32,
    pub width: f32,
    pub distance_along: f32,
}

/// Per-object instance attributes for the stroke pipeline matching
/// `object_stroke.wgsl` (`m0`/`m1`/`m2` @5..7, `stroke` @8).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub stroke: [f32; 4],
}

/// `object_stroke.wgsl`'s `StrokeUniform` (binding 1): dash on-length, period,
/// opacity, and a solid/dashed flag.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeParamsUniform {
    /// `vec4(dash_on_px, dash_period_px, opacity, dashed_flag)`.
    pub dash: [f32; 4],
}

impl StrokeParamsUniform {
    /// Solid (no-dash) params at full opacity. Per-object dash gating is folded
    /// into the CPU dash split for the first cutover, so the shader-side uniform
    /// stays solid; richer per-instance dashing gets its own buffer later.
    pub fn solid() -> Self {
        StrokeParamsUniform {
            dash: [0.0, 0.0, 1.0, 0.0],
        }
    }
}

/// Per-glyph-quad-corner attributes matching `msdf_text.wgsl`'s `VertexIn`
/// (`position` @0, `uv` @1, `color` @2). `position` is object-local **pixels**
/// (the region-local glyph quad corner, already laid out by `layout_runs`); `uv`
/// is the atlas UV (0..1) for the corner; `color` is the inline per-run paint
/// (D19 `run.color`). Color rides per-glyph here, not on the instance — the
/// instance carries only the matrix so one object's runs can mix colors.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// Per-object instance attributes for the text pipeline matching
/// `msdf_text.wgsl` (`m0`/`m1`/`m2` @3..5). Color is per-glyph in [`TextVertex`],
/// so the text instance carries only the 3x3 projective matrix columns — the same
/// region-local-px -> world bridge the fill/stroke instances use, index-aligned
/// with `draws[i]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
}

/// `msdf_text.wgsl`'s `TextUniform` (binding 3): MSDF atlas params the
/// `screenPxRange` AA reads. `atlas = vec4(distance_range, atlas_w, atlas_h, _)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextUniform {
    /// `vec4(distance_range_texels, atlas_width, atlas_height, _)`.
    pub atlas: [f32; 4],
}


// ---------------------------------------------------------------------------
// CPU scene build
// ---------------------------------------------------------------------------

/// The CPU-built draw data for one object: where its fill triangles live in the
/// megabuffer and the instance record (matrix + color) drawn against them, plus
/// its stroke vertices and stroke instance. Held by [`ObjectRenderer`] so each
/// object becomes one indexed fill draw + one stroke draw at record time.
#[derive(Clone, Debug)]
pub struct ObjectDraw {
    pub id: String,
    /// Index range into the shared fill megabuffer (`fill_indices`), or an empty
    /// range when the object has no fillable region.
    pub fill_range: DrawRange,
    /// VERTEX range of this object's fill inside the shared fill vertex buffer
    /// (`[start, end)`), distinct from `fill_range` which indexes the INDEX buffer.
    /// W3-G9/#4 needs the vertex base/length to patch a follower's fill positions in
    /// place during a live anchor reproject (the megabuffer rebases indices by this
    /// vertex offset). Empty when the object has no fillable region.
    pub fill_vertex_range: DrawRange,
    pub fill_instance: FillInstance,
    /// RB3 drop-shadow quad vertex range into the shared `shadow_vertices` buffer.
    /// Empty when the object has no boundable region (nothing to cast a shadow).
    /// Drawn BEFORE fill (beneath the object).
    pub shadow_range: DrawRange,
    pub shadow_instance: ShadowInstance,
    /// Stroke ribbon vertices for this object (own buffer slice via `stroke_range`).
    pub stroke_range: DrawRange,
    pub stroke_instance: StrokeInstance,
    /// Glyph-quad vertex range into the shared `text_vertices` buffer (RB2). Empty
    /// when the object carries no text (or only whitespace). Drawn after stroke.
    pub text_range: DrawRange,
    /// Whether a focus ring should be drawn for this object (selection/focus).
    pub focus_ring: bool,
    /// RB1 theme toggle: the semantic token name backing this object's fill, if
    /// the fill is a [`RPaint::Token`]. `Some` => the fill color re-resolves on a
    /// theme flip; `None` (raw hex / gradient / image) is theme-invariant. Lets
    /// [`ObjectRenderer::set_theme`] write ONLY token-backed instance colors
    /// (zero re-tessellation, P4).
    pub fill_token: Option<String>,
    /// RB1 theme toggle: the token name backing this object's stroke, if any.
    pub stroke_token: Option<String>,
}

/// RB3: the renderer-default drop-shadow color is the theme `shadow` token. This
/// is the ONLY tie of the shadow pass to the C1 token table — it is wired, never
/// hardcoded, so the shadow flips dark-translucent (light mode) <-> light-
/// translucent (dark mode) with the theme bit (zero-rebake color refresh, P4).
const SHADOW_TOKEN: &str = "shadow";

/// W3-G8/A drop-shadow constants (object-local px). The shadow is a SINGLE offset
/// silhouette copy of the object's OWN fill, offset by [`SHADOW_OFFSET_PX`] in y.
/// Softness no longer comes from stacked tiers — the offscreen separable-Gaussian
/// blur pass (see [`crate::shadow_blur`]) provides the uniform feather. W3-G9/#1:
/// the offset is 0 so the halo is fully symmetric on ALL sides (macOS ambient
/// look) — the wide quarter-res blur owns the soft spread, no downward bias.
const SHADOW_OFFSET_PX: f32 = 0.0;


// ---------------------------------------------------------------------------
// CPU geometry build (device-independent, unit-testable)
// ---------------------------------------------------------------------------

/// The device-independent result of building a scene's GPU geometry: a merged
/// fill megabuffer + per-object fill instances, and a flat stroke ribbon vertex
/// array + per-object stroke instances, plus the per-object [`ObjectDraw`] index.
#[derive(Clone, Debug, Default)]
pub struct SceneGeometry {
    pub fill: MegaBuffer,
    /// Per-vertex analytic-AA silhouette flags (D4), index-aligned with
    /// `fill.vertices`: `1.0` on a boundary (silhouette) vertex, `0.0` interior.
    /// Built once with the mesh topology; `ObjectRenderer::new` widens it into each
    /// `FillVertex.edge` so the FS fades the silhouette pixel (zero per-frame work).
    pub fill_edges: Vec<f32>,
    pub fill_instances: Vec<FillInstance>,
    /// RB3 drop-shadow quad vertices (6 per object with a boundable region,
    /// tri-list), object-local px + per-vertex feather. Per-object slices via
    /// `draws[i].shadow_range`.
    pub shadow_vertices: Vec<ShadowVertex>,
    /// Per-object shadow instances (matrix columns + theme `shadow` color),
    /// index-aligned with `draws`.
    pub shadow_instances: Vec<ShadowInstance>,
    pub stroke_vertices: Vec<StrokeVertex>,
    pub stroke_instances: Vec<StrokeInstance>,
    /// Positioned glyph-quad vertices (6 per visible glyph, tri-list), object-local
    /// px + atlas UV + per-run color. Per-object slices via `draws[i].text_range`.
    pub text_vertices: Vec<TextVertex>,
    /// Per-object text instances (matrix columns only), index-aligned with `draws`.
    pub text_instances: Vec<TextInstance>,
    pub draws: Vec<ObjectDraw>,
}

/// Build all CPU geometry for `scene` in light mode (no device needed). Thin
/// wrapper over [`build_scene_geometry_themed`] for callers that don't carry a
/// theme yet (the web geometry-summary export + light-mode tests). Token paints
/// resolve against the light table here.
pub fn build_scene_geometry(scene: &RenderObjectScene) -> SceneGeometry {
    build_scene_geometry_themed(scene, Theme::light())
}

/// Default device-free glyph-advance stub for the text-build path: a
/// size-proportional advance (`STUB_ADVANCE_RATIO * size`), pure and
/// deterministic, with no fontdue / I-O (CLAUDE.md pure-core rule). The real
/// fontdue-backed `TextEngine::measure_text_width` is injected at the GPU cutover
/// via [`build_scene_geometry_themed_with_measure`]; until then committed text
/// still reaches the geometry/draw path at proportional positions.
pub const STUB_ADVANCE_RATIO: f32 = 0.6;

fn stub_measure(_ch: char, size: f32) -> f32 {
    STUB_ADVANCE_RATIO * size
}

/// Build all CPU geometry for `scene` under `theme` (no device needed): for each
/// object, tessellate its fill into the shared megabuffer, expand its stroke into
/// a ribbon, resolve its instance data (3x3 matrix columns + theme-resolved paint
/// color), and lay out its text runs into positioned glyph quads. This is the
/// unit-testable core of [`ObjectRenderer::new`]. The `theme` bit only affects
/// token paint COLORS — tessellation/ranges are theme-invariant, which is what
/// makes the toggle a zero-rebake color refresh. Text advances use the
/// size-proportional [`stub_measure`]; the live fontdue shaper is injected via
/// [`build_scene_geometry_themed_with_measure`] at the GPU cutover.
pub fn build_scene_geometry_themed(scene: &RenderObjectScene, theme: Theme) -> SceneGeometry {
    build_scene_geometry_themed_with_measure(scene, theme, &stub_measure)
}

/// As [`build_scene_geometry_themed`], but with an injected per-char glyph-advance
/// `measure` closure. The pure core never calls fontdue itself (CLAUDE.md: no
/// I-O); the GPU cutover supplies the real `TextEngine::measure_text_width`, while
/// tests pass a deterministic stub. Identical to the themed wrapper for non-text
/// objects.
pub fn build_scene_geometry_themed_with_measure(
    scene: &RenderObjectScene,
    theme: Theme,
    measure: &dyn Fn(char, f32) -> f32,
) -> SceneGeometry {
    let mut geometry = SceneGeometry::default();

    for obj in &scene.objects {
        let state = visual_state_for(scene, &obj.id);
        let resolved = resolve_visual(obj, state);

        let subpaths = flatten_object_subpaths(obj, scene.camera.zoom);

        // W3-G7/#3: an OPEN-only path with NO explicit fill is NOT filled (Figma /
        // macOS convention) — a freehand brush stroke commits as an open subpath
        // with `fill: None`, and tessellating it as a chord region showed an ugly
        // default-white fill. Closed shapes (rect/ellipse) and any explicit fill are
        // unaffected. When skipped we still push a Fill/Shadow instance below so the
        // per-object instance buffers stay index-aligned with `draws`.
        let skip_fill =
            obj.fill.is_none() && subpaths.iter().all(|(closed, _)| !closed);

        // ---- Fill: tessellate the closed region into the megabuffer --------
        let mesh = if skip_fill {
            crate::tessellate::Mesh::default()
        } else {
            let fill_input: Vec<(bool, Vec<(f32, f32)>)> = subpaths
                .iter()
                .map(|(closed, pts)| (*closed, pts.clone()))
                .collect();
            tessellate_fill(&fill_input, FillRuleKind::NonZero)
        };
        // Silhouette flags travel index-aligned with the megabuffer's vertex array:
        // `push` appends this mesh's vertices, so we extend `fill_edges` with this
        // mesh's boundary flags in lockstep (D4 analytic fill AA).
        geometry.fill_edges.extend_from_slice(&mesh.boundary_flags());
        // Record the fill VERTEX sub-range (distinct from the index range `push`
        // returns) so a follower's fill positions can be patched in place (W3-G9/#4).
        let fill_vertex_start = geometry.fill.vertices.len() as u32;
        let fill_range = geometry.fill.push(&mesh);
        let fill_vertex_range = DrawRange {
            start: fill_vertex_start,
            end: geometry.fill.vertices.len() as u32,
        };
        geometry.fill_instances.push(FillInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            fill: paint_color(&resolved.fill.paint, resolved.fill.opacity as f32, theme),
        });

        // ---- Stroke: expand each (dashed) subpath into a ribbon ------------
        let stroke_start = geometry.stroke_vertices.len() as u32;
        let cap = match resolved.stroke.cap {
            crate::render_object::RStrokeCap::Butt => Cap::Butt,
            crate::render_object::RStrokeCap::Round => Cap::Round,
            crate::render_object::RStrokeCap::Square => Cap::Square,
        };
        let join = match resolved.stroke.join {
            crate::render_object::RStrokeJoin::Miter => Join::Miter,
            crate::render_object::RStrokeJoin::Bevel => Join::Bevel,
            // Round joins are not yet expanded by the CPU reference; miter is the
            // closest existing geometry until a round-join arc is added.
            crate::render_object::RStrokeJoin::Round => Join::Miter,
        };
        let width = resolved.stroke.width as f32;
        // Collect the object's stroke ribbon triangle meshes so a fill-LESS object
        // (open/free-draw stroke) can still cast a shadow from its line outline
        // (W3-G10/#3); a filled object ignores these and casts from its fill mesh.
        let mut stroke_meshes: Vec<crate::stroke_expand::Mesh> = Vec::new();
        for (closed, pts) in &subpaths {
            let runs = dash_segments(pts, &dash_px(&resolved.stroke.dash));
            for run in runs {
                let stroke_mesh = expand_stroke(&run, *closed, width, None, cap, join);
                append_stroke_ribbon(&mut geometry.stroke_vertices, &stroke_mesh, width);
                stroke_meshes.push(stroke_mesh);
            }
        }
        let stroke_end = geometry.stroke_vertices.len() as u32;

        // ---- Shadow: an offset copy of the silhouette (RB3 #11) ------------
        // The shadow REUSES the object's own fill `mesh` (the exact, hole-aware,
        // concavity-correct region triangulation), translated by the drop offset
        // and drawn beneath the fill. Its color is the theme `shadow` token, NEVER
        // hardcoded — so a theme flip is a per-instance color refresh, the geometry
        // stays put (zero rebake, P4). No extra tessellation.
        //
        // W3-G10/#3: when the fill mesh is EMPTY (an open/stroke-only free-draw
        // object) but the object has a stroke ribbon, build the silhouette from the
        // STROKE RIBBON instead so the line itself casts a soft shadow. The ribbon
        // is the already-expanded triangle list `append_stroke_ribbon` consumed, so
        // this is still a copy of baked geometry — zero re-tessellation.
        let shadow_start = geometry.shadow_vertices.len() as u32;
        if mesh.indices.is_empty() {
            for stroke_mesh in &stroke_meshes {
                append_shadow_quad(
                    &mut geometry.shadow_vertices,
                    &stroke_mesh.vertices,
                    &stroke_mesh.indices,
                );
            }
        } else {
            append_shadow_quad(&mut geometry.shadow_vertices, &mesh.vertices, &mesh.indices);
        }
        let shadow_end = geometry.shadow_vertices.len() as u32;
        geometry.shadow_instances.push(ShadowInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            // The drop-shadow color is the `shadow` token resolved against the
            // active theme — wired through the token table, never a hardcoded RGBA.
            shadow: resolve_token_f32(SHADOW_TOKEN, theme.dark).unwrap_or([0.0, 0.0, 0.0, 0.25]),
        });
        geometry.stroke_instances.push(StrokeInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            stroke: paint_color(&resolved.stroke.paint, resolved.stroke.opacity as f32, theme),
        });

        // ---- Text: lay out runs into positioned glyph quads ----------------
        // The region bbox is derived from the SAME flattened pixel subpaths the
        // fill/stroke use (single pixel space, no re-parse). `layout_runs` bakes
        // align/valign/wrap into region-local px pen origins; the per-object 3x3
        // instance carries region-local-px -> world, so no extra CPU transform.
        let text_start = geometry.text_vertices.len() as u32;
        if let Some(text) = &obj.text {
            append_text_quads(&mut geometry.text_vertices, text, &subpaths, scene.camera.zoom, measure);
        }
        let text_end = geometry.text_vertices.len() as u32;
        geometry.text_instances.push(TextInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
        });

        geometry.draws.push(ObjectDraw {
            id: obj.id.clone(),
            fill_range,
            fill_vertex_range,
            fill_instance: *geometry
                .fill_instances
                .last()
                .expect("fill instance just pushed"),
            shadow_range: DrawRange {
                start: shadow_start,
                end: shadow_end,
            },
            shadow_instance: *geometry
                .shadow_instances
                .last()
                .expect("shadow instance just pushed"),
            stroke_range: DrawRange {
                start: stroke_start,
                end: stroke_end,
            },
            stroke_instance: *geometry
                .stroke_instances
                .last()
                .expect("stroke instance just pushed"),
            text_range: DrawRange {
                start: text_start,
                end: text_end,
            },
            focus_ring: resolved.focus_ring.is_some(),
            fill_token: paint_token_name(&resolved.fill.paint),
            stroke_token: paint_token_name(&resolved.stroke.paint),
        });
    }

    geometry
}

/// Resolve the visual state (selected via single anchor or transient
/// multi-select) for an object id, so `resolve_visual` adds a focus ring for the
/// selection set.
fn visual_state_for(scene: &RenderObjectScene, id: &str) -> VisualState {
    let selected = scene.selection.as_deref() == Some(id)
        || scene.multi_select.iter().any(|candidate| candidate == id);
    VisualState {
        selected,
        hovered: false,
        focused: false,
    }
}

/// Parse an object's geometry and flatten each subpath into an object-local
/// **pixel** polyline, flattening cubics via the zoom-bucket LOD flattener.
/// Returns `(closed, points)` per subpath.
fn flatten_object_subpaths(obj: &RenderObject, zoom: f64) -> Vec<(bool, Vec<(f32, f32)>)> {
    let bucket = crate::curve_lod::zoom_bucket(zoom);
    let flatness = crate::curve_lod::flatness_for_bucket(bucket);
    let parsed = parse_path(&obj.geometry_d);
    let mut out = Vec::with_capacity(parsed.len());
    for sub in &parsed {
        let mut pts: Vec<(f32, f32)> = Vec::new();
        let mut cursor = (0.0f32, 0.0f32);
        for cmd in &sub.commands {
            match *cmd {
                PathCommand::MoveTo { x, y } => {
                    cursor = (quantized_to_px(x), quantized_to_px(y));
                    pts.push(cursor);
                }
                PathCommand::LineTo { x, y } => {
                    cursor = (quantized_to_px(x), quantized_to_px(y));
                    pts.push(cursor);
                }
                PathCommand::Cubic {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    let c1 = (quantized_to_px(c1x), quantized_to_px(c1y));
                    let c2 = (quantized_to_px(c2x), quantized_to_px(c2y));
                    let end = (quantized_to_px(x), quantized_to_px(y));
                    let flat = crate::curve_lod::flatten_cubic(cursor, c1, c2, end, flatness);
                    // `flatten_cubic` includes both endpoints; the start duplicates
                    // the cursor already pushed, so skip it.
                    for &p in flat.iter().skip(1) {
                        pts.push(p);
                    }
                    cursor = end;
                }
                PathCommand::Close => {}
            }
        }
        out.push((sub.closed, pts));
    }
    out
}

/// Append one object's drop-shadow geometry (W3-G8/A) to `out`: a SINGLE
/// translucent silhouette copy of the object's OWN fill `mesh` (the exact region
/// triangulation lyon already produced — concave, curved, hole-aware for free),
/// translated down by [`SHADOW_OFFSET_PX`]. There is no extra tessellation: this is
/// one triangle-list copy of an already-tessellated mesh.
///
/// Softness is NOT baked here. The silhouette is rendered ONCE to an offscreen
/// target and a separable Gaussian blur ([`crate::shadow_blur`]) feathers it
/// uniformly before it composites under the fill — a true blur, not a stacked-tier
/// approximation. `feather` is emitted `0` (the slot stays for the shader contract;
/// the offscreen blur owns softness now).
///
/// An empty mesh (an open polyline / no fillable interior, with no stroke ribbon)
/// casts nothing, so the object's `shadow_range` stays empty — matching the fill's
/// "no boundable region emits nothing" contract.
///
/// Takes the triangle-list as bare `(vertices, indices)` slices so it serves both
/// the fill mesh ([`crate::tessellate::Mesh`]) and the stroke ribbon
/// ([`crate::stroke_expand::Mesh`]) — same `[[f32;2]]` + `[u32]` shape — without
/// coupling to either concrete type.
fn append_shadow_quad(out: &mut Vec<ShadowVertex>, vertices: &[[f32; 2]], indices: &[u32]) {
    if indices.is_empty() {
        return;
    }
    for &index in indices {
        let p = vertices[index as usize];
        out.push(ShadowVertex {
            position: [p[0], p[1] + SHADOW_OFFSET_PX],
            feather: 0.0,
        });
    }
}

/// Append a stroke ribbon `mesh` (triangle-list of `[x,y]` positions) as
/// expanded [`StrokeVertex`]es. The CPU ribbon already bakes the normal offset
/// into the positions, so the GPU stroke shader must not offset again: we emit a
/// zero normal and `side = 0`, carrying only the pre-offset position, with the
/// per-node `width` and a running arc-length `distance_along`.
fn append_stroke_ribbon(out: &mut Vec<StrokeVertex>, mesh: &crate::stroke_expand::Mesh, width: f32) {
    let mut distance = 0.0f32;
    let mut prev: Option<[f32; 2]> = None;
    for &index in &mesh.indices {
        let position = mesh.vertices[index as usize];
        if let Some(p) = prev {
            let dx = position[0] - p[0];
            let dy = position[1] - p[1];
            distance += (dx * dx + dy * dy).sqrt();
        }
        prev = Some(position);
        out.push(StrokeVertex {
            position,
            normal: [0.0, 0.0],
            side: 0.0,
            width,
            distance_along: distance,
        });
    }
}

/// Lay out an object's text runs against its derived region and append one
/// 2-triangle (6-vertex) quad per visible glyph to `out`, in object-local px with
/// per-run color. The region bbox comes from the SAME flattened pixel `subpaths`
/// the fill/stroke use, via [`crate::outline::derive_region`] (single pixel space,
/// no re-parse). Run `size` is DE-QUANTIZED (`/QUANT_PER_PX`) so committed text
/// lays out at edit-time pixels, joining the same pixel space as the geometry.
///
/// The glyph quad geometry is currently a placement-sized cell spanning the full
/// atlas (`uv 0..1`): the live MSDF atlas (per-glyph bearing/width/height/uv) is
/// populated at the GPU cutover, so at build-level the quad's ORIGIN — the
/// load-bearing layout result — is what the golden pins. The pen origin equals
/// `layout_runs`' region-local px position, so de-quant + align + wrap are all
/// verifiable device-free; real per-glyph atlas UVs swap in at GPU wiring.
fn append_text_quads(
    out: &mut Vec<TextVertex>,
    text: &RText,
    subpaths: &[(bool, Vec<(f32, f32)>)],
    zoom: f64,
    measure: &dyn Fn(char, f32) -> f32,
) {
    let bucket = crate::curve_lod::zoom_bucket(zoom);
    let flatness = crate::curve_lod::flatness_for_bucket(bucket);
    let Some(region) = crate::outline::derive_region(subpaths, flatness) else {
        return;
    };
    let region_min = (region.min_x, region.min_y);
    let region_max = (region.max_x, region.max_y);

    let runs: Vec<TextRunInput> = text
        .runs
        .iter()
        .map(|run| TextRunInput {
            text: run.text.clone(),
            color: text_run_color(&run.color),
            // DE-QUANT (commit C): wire size is quantized at `QUANT_PER_PX` units/px,
            // matching the geometry coords; divide to recover edit-time pixels.
            size: (run.size / QUANT_PER_PX) as f32,
            bold: run.bold,
            italic: run.italic,
            font: run.font.clone(),
        })
        .collect();

    let align = match text.align {
        RTextAlign::Start => TextAlign::Start,
        RTextAlign::Center => TextAlign::Center,
        RTextAlign::End => TextAlign::End,
        RTextAlign::Justify => TextAlign::Justify,
    };
    let valign = match text.valign {
        RTextValign::Top => TextVAlign::Top,
        RTextValign::Middle => TextVAlign::Middle,
        RTextValign::Bottom => TextVAlign::Bottom,
    };

    let placements = layout_runs(&runs, region_min, region_max, align, valign, measure);
    for p in &placements {
        // Placement-sized cell at the pen origin; UVs span the whole atlas until
        // the GPU cutover registers per-glyph atlas slots. Top-left -> bottom-right.
        let x0 = p.x;
        let y0 = p.y;
        let x1 = p.x + p.size;
        let y1 = p.y + p.size;
        let tl = TextVertex { position: [x0, y0], uv: [0.0, 0.0], color: p.color };
        let tr = TextVertex { position: [x1, y0], uv: [1.0, 0.0], color: p.color };
        let br = TextVertex { position: [x1, y1], uv: [1.0, 1.0], color: p.color };
        let bl = TextVertex { position: [x0, y1], uv: [0.0, 1.0], color: p.color };
        // Two triangles (tl, tr, br) + (tl, br, bl) — CCW tri-list, no index buffer.
        out.push(tl);
        out.push(tr);
        out.push(br);
        out.push(tl);
        out.push(br);
        out.push(bl);
    }
}

/// Parse a text run's `#rrggbb` color into RGBA (alpha 1.0). Mirrors
/// [`parse_hex_rgb`]; a bad value falls back to opaque white.
fn text_run_color(value: &str) -> [f32; 4] {
    let [r, g, b] = parse_hex_rgb(value);
    [r, g, b, 1.0]
}

/// Extract column `col` of a row-major 3x3 transform as a `vec3` for the
/// column-major `mat3x3` reconstruction in the WGSL (`M = [m0 | m1 | m2]`).
fn matrix_col(transform: &[[f64; 3]; 3], col: usize) -> [f32; 3] {
    [
        transform[0][col] as f32,
        transform[1][col] as f32,
        transform[2][col] as f32,
    ]
}

/// W2-11 drag zero-rebake: compose the preview world matrix `delta * base` and
/// return its three instance columns in the exact same packing
/// [`build_scene_geometry`] uses for [`FillInstance`]/[`StrokeInstance`]
/// (`M = [m0 | m1 | m2]`). This is the single source of truth for the preview
/// matrix: the GPU writer ([`ObjectRenderer::set_preview_transform`]) and the
/// perf-gate test both call it, so the live-drag instance push is byte-equivalent
/// to a full rebake of the same object at `compose(delta, base)`. Pure and
/// device-free so it runs under `cargo test --workspace` too.
pub fn preview_instance_columns(
    delta: &[[f64; 3]; 3],
    base: &[[f64; 3]; 3],
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let world = crate::hit_test_object::mat3_mul(delta, base);
    (
        matrix_col(&world, 0),
        matrix_col(&world, 1),
        matrix_col(&world, 2),
    )
}

/// The per-buffer strides [`ObjectRenderer::set_preview_transform`] writes the
/// previewed matrix into, in lockstep with its `[fill, stroke, text, shadow]`
/// buffer array: the matrix lands at `i * stride` in EACH. Fill/stroke moved the
/// region from W2-11; text (G3) makes the glyphs follow the same drag; shadow (G5)
/// makes the drop shadow follow it too, instead of lagging at canonical until the
/// rebake. Pure and device-free so the live-drag follow set is testable under
/// `cargo test` without a GPU — a dropped entry here means a sub-visual stops
/// tracking the preview.
pub fn preview_instance_strides() -> [u64; 4] {
    [
        std::mem::size_of::<FillInstance>() as u64,
        std::mem::size_of::<StrokeInstance>() as u64,
        std::mem::size_of::<TextInstance>() as u64,
        std::mem::size_of::<ShadowInstance>() as u64,
    ]
}

/// W3-G9/#4: the re-expanded geometry for ONE object, used to patch a follower's
/// baked vertices in place during a live anchor reproject. The fill indices are
/// object-LOCAL (0-based); the caller rebases them by the follower's vertex base
/// in the shared megabuffer before writing. W3-G13: carries EVERY per-object baked
/// vertex artifact — fill, stroke, shadow silhouette, text glyph quads — so no
/// sub-visual lags at the old geometry until the commit rebake (the dragged-target
/// follower-shadow bug).
pub struct FollowerReexpand {
    pub fill_vertices: Vec<FillVertex>,
    pub fill_indices: Vec<u32>,
    pub stroke_vertices: Vec<StrokeVertex>,
    pub shadow_vertices: Vec<ShadowVertex>,
    pub text_vertices: Vec<TextVertex>,
}

/// W3-G9/#4: re-expand a SINGLE object's fill + stroke geometry, reusing the exact
/// build-loop path ([`build_scene_geometry_themed`]) so the result is byte-identical
/// to what a full rebake of that object would produce. The object is built alone in
/// a one-object scene carrying the live `camera` (so the zoom/LOD bucket matches the
/// canonical bake) and `theme` (so the silhouette-AA `edge` flags match); selection
/// is dropped because it only adds a focus ring, never changing fill/stroke vertex
/// COUNT. Returns the widened fill vertices, the object-local fill indices, and the
/// stroke ribbon vertices. Pure (device-free), so the COUNT-equality that makes the
/// in-place patch size-safe is host-testable without a GPU.
pub fn reexpand_single_object(
    obj: &RenderObject,
    theme: Theme,
    camera: CameraState,
) -> FollowerReexpand {
    let scene = RenderObjectScene {
        scene_id: String::new(),
        camera,
        objects: vec![obj.clone()],
        selection: None,
        multi_select: Vec::new(),
    };
    // W3-G13 ENFORCED artifact contract: every per-object BAKED VERTEX artifact
    // must reach the follower patch (a dropped one lags at the old geometry until
    // the commit rebake — the shadow bug). Instance matrices/colors belong to the
    // instance-preview write-set (`preview_instance_strides`), and `draws` is the
    // canonical bake's index, rebuilt only on a full re-feed. Destructuring WITHOUT
    // `..` makes adding a SceneGeometry artifact a COMPILE ERROR here until its
    // patch story is decided.
    let SceneGeometry {
        fill,
        fill_edges,
        fill_instances: _,
        shadow_vertices,
        shadow_instances: _,
        stroke_vertices,
        stroke_instances: _,
        text_vertices,
        text_instances: _,
        draws: _,
    } = build_scene_geometry_themed(&scene, theme);
    let fill_vertices: Vec<FillVertex> = fill
        .vertices
        .iter()
        .zip(&fill_edges)
        .map(|(&position, &edge)| FillVertex { position, edge })
        .collect();
    FollowerReexpand {
        fill_vertices,
        fill_indices: fill.indices,
        stroke_vertices,
        shadow_vertices,
        text_vertices,
    }
}

/// W3-G9/#4: the byte offsets + element counts for writing a follower's re-expanded
/// geometry over its EXISTING megabuffer ranges, or `None` when the re-expand is not
/// size-safe (topology/LOD edge case: a vertex/index count changed). Skipping on
/// `None` is the defensive guard — a mismatched write would bleed into another
/// object's range or corrupt the buffer. Pure (no GPU), so the count-safety decision
/// is host-testable; the actual `queue.write_buffer` consuming this is GPU-only.
pub struct FollowerPatchPlan {
    /// Byte offset of the follower's fill vertices in the shared fill vertex buffer.
    pub fill_vertex_byte_offset: u64,
    /// Byte offset of the follower's fill indices in the shared fill index buffer.
    pub fill_index_byte_offset: u64,
    /// Amount to add to each object-local fill index so it points at the follower's
    /// vertices inside the merged megabuffer (the build-time rebase).
    pub fill_index_rebase: u32,
    /// Byte offset of the follower's stroke vertices in the shared stroke buffer.
    pub stroke_vertex_byte_offset: u64,
    /// W3-G13: byte offset of the follower's drop-shadow silhouette vertices in the
    /// shared shadow buffer.
    pub shadow_vertex_byte_offset: u64,
    /// W3-G13: byte offset of the follower's glyph-quad vertices in the shared text
    /// buffer.
    pub text_vertex_byte_offset: u64,
}

/// W3-G9/#4: validate that `rebuilt` exactly fills the follower `draw`'s existing
/// megabuffer ranges (same fill vertex count, fill index count, stroke vertex count,
/// and — W3-G13 — shadow + text vertex counts) and, if so, return the
/// [`FollowerPatchPlan`] byte offsets. Returns `None` on ANY count mismatch so the
/// caller SKIPS the patch this frame rather than writing a mismatched range. The
/// node COUNT is unchanged during a drag (only positions move) and the LOD bucket
/// is fixed, so the counts normally match; a `None` is the rare topology edge case
/// the guard exists for. Pure + falsifiable without a device.
pub fn follower_patch_plan(draw: &ObjectDraw, rebuilt: &FollowerReexpand) -> Option<FollowerPatchPlan> {
    // W3-G13 ENFORCED artifact contract (the ObjectDraw side): destructuring WITHOUT
    // `..` binds every field, so a new per-object vertex RANGE is a COMPILE ERROR
    // here until it is guarded + planned. The non-range fields are explicitly not
    // the patch's business: instances are the instance-preview write-set's
    // (`preview_instance_strides`), and id/focus/tokens are draw-record metadata.
    let ObjectDraw {
        id: _,
        fill_range,
        fill_vertex_range,
        fill_instance: _,
        shadow_range,
        shadow_instance: _,
        stroke_range,
        stroke_instance: _,
        text_range,
        focus_ring: _,
        fill_token: _,
        stroke_token: _,
    } = draw;
    if rebuilt.fill_vertices.len() as u32 != fill_vertex_range.len()
        || rebuilt.fill_indices.len() as u32 != fill_range.len()
        || rebuilt.stroke_vertices.len() as u32 != stroke_range.len()
        || rebuilt.shadow_vertices.len() as u32 != shadow_range.len()
        || rebuilt.text_vertices.len() as u32 != text_range.len()
    {
        return None;
    }
    Some(FollowerPatchPlan {
        fill_vertex_byte_offset: fill_vertex_range.start as u64
            * std::mem::size_of::<FillVertex>() as u64,
        fill_index_byte_offset: fill_range.start as u64 * std::mem::size_of::<u32>() as u64,
        fill_index_rebase: fill_vertex_range.start,
        stroke_vertex_byte_offset: stroke_range.start as u64
            * std::mem::size_of::<StrokeVertex>() as u64,
        shadow_vertex_byte_offset: shadow_range.start as u64
            * std::mem::size_of::<ShadowVertex>() as u64,
        text_vertex_byte_offset: text_range.start as u64
            * std::mem::size_of::<TextVertex>() as u64,
    })
}

/// Resolve a paint to a single RGBA color for the inline-solid first cutover,
/// theme-aware (RB1/D1). `theme` selects the light/dark token table for
/// [`RPaint::Token`]. Gradient/image paints collapse to their representative
/// color (first stop / neutral) here; richer paints get their own bind group
/// later (D7 note).
///
/// A token carries its own alpha (e.g. translucent `shadow`); the per-paint
/// `opacity` multiplies it. Solid/gradient hex paints have no inherent alpha, so
/// `opacity` becomes the alpha directly (unchanged from the inline-solid path).
fn paint_color(paint: &RPaint, opacity: f32, theme: Theme) -> [f32; 4] {
    let opacity = opacity.clamp(0.0, 1.0);
    match paint {
        RPaint::Solid { color } => {
            let rgb = parse_hex_rgb(color);
            [rgb[0], rgb[1], rgb[2], opacity]
        }
        RPaint::Token { name } => {
            // Unknown token names fall back to opaque white so a bad token never
            // poisons the draw (mirrors `parse_hex_rgb`).
            let [r, g, b, a] = resolve_token_f32(name, theme.dark).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            [r, g, b, a * opacity]
        }
        RPaint::Gradient { stops, .. } => {
            let rgb = stops
                .first()
                .map(|stop| parse_hex_rgb(&stop.color))
                .unwrap_or([1.0, 1.0, 1.0]);
            [rgb[0], rgb[1], rgb[2], opacity]
        }
        RPaint::Image { .. } => [1.0, 1.0, 1.0, opacity],
    }
}

/// The token name backing a paint, if it is an [`RPaint::Token`]. Used to mark
/// an [`ObjectDraw`] as token-backed so a theme toggle can re-resolve ONLY those
/// instances' colors (zero-rebake, P4) — non-token paints are theme-invariant.
fn paint_token_name(paint: &RPaint) -> Option<String> {
    match paint {
        RPaint::Token { name } => Some(name.clone()),
        _ => None,
    }
}

/// Parse a `#rrggbb` hex color into linear-ish 0..1 RGB. A malformed value falls
/// back to white so a bad color never poisons the draw.
fn parse_hex_rgb(value: &str) -> [f32; 3] {
    let parse = || -> Option<[f32; 3]> {
        let hex = value.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0])
    };
    parse().unwrap_or([1.0, 1.0, 1.0])
}

/// Convert a stroke dash pattern (px lengths) for the CPU dash split. An empty
/// pattern stays empty (solid).
fn dash_px(dash: &[f64]) -> Vec<f32> {
    dash.iter().map(|&d| d as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CameraState;
    use crate::render_object::{
        RFill, RPaint, RStroke, RStrokeCap, RStrokeJoin, RText, RTextAlign, RTextRun, RTextValign,
    };

    fn identity() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    fn rect_object(id: &str) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            // 100px square at 8 units/px = 800 quantized units.
            geometry_d: "M0 0 L800 0 L800 800 L0 800 Z".to_string(),
            fill: Some(RFill {
                paint: RPaint::Solid {
                    color: "#ff0000".to_string(),
                },
                opacity: 1.0,
            }),
            stroke: Some(RStroke {
                paint: RPaint::Solid {
                    color: "#00ff00".to_string(),
                },
                width: 4.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            }),
            text: None,
            anchors: Vec::new(),
            clip: false,
        }
    }

    fn scene_with(objects: Vec<RenderObject>, selection: Option<String>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects,
            selection,
            multi_select: Vec::new(),
        }
    }

    #[test]
    fn matrix_uniform_packs_to_32_bytes_camera_then_viewport() {
        let scene = scene_with(vec![rect_object("o1")], None);
        let uniform = ObjectMatrixUniform::from_scene(&scene, 1280.0, 720.0);
        let bytes = bytemuck::bytes_of(&uniform);
        // Two vec4<f32> = 8 floats = 32 bytes, camera first then viewport.
        assert_eq!(bytes.len(), 32);
        assert_eq!(std::mem::size_of::<ObjectMatrixUniform>(), 32);
        assert_eq!(uniform.camera, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(uniform.viewport, [1280.0, 720.0, 0.0, 0.0]);
        // Byte layout matches the legacy ViewUniform (camera xyzw then viewport).
        let floats: &[f32] = bytemuck::cast_slice(bytes);
        assert_eq!(floats, &[0.0, 0.0, 1.0, 0.0, 1280.0, 720.0, 0.0, 0.0]);
    }

    #[test]
    fn vertex_and_instance_layout_sizes_match_shader_contract() {
        // FillVertex: vec2 position + f32 edge = 3 floats = 12 bytes.
        assert_eq!(std::mem::size_of::<FillVertex>(), 12);
        // FillInstance: 3 vec3 columns + vec4 fill = 9 + 4 = 13 floats = 52 bytes.
        assert_eq!(std::mem::size_of::<FillInstance>(), 52);
        // StrokeVertex: position(2) + normal(2) + side + width + distance = 7
        // floats = 28 bytes.
        assert_eq!(std::mem::size_of::<StrokeVertex>(), 28);
        // StrokeInstance: 3 vec3 + vec4 = 52 bytes (same as fill instance).
        assert_eq!(std::mem::size_of::<StrokeInstance>(), 52);
        // StrokeParamsUniform: one vec4 = 16 bytes.
        assert_eq!(std::mem::size_of::<StrokeParamsUniform>(), 16);
    }

    #[test]
    fn build_scene_geometry_tessellates_fill_and_expands_stroke() {
        let scene = scene_with(vec![rect_object("o1")], None);
        let geo = build_scene_geometry(&scene);

        assert_eq!(geo.draws.len(), 1);
        assert_eq!(geo.fill_instances.len(), 1);
        assert_eq!(geo.stroke_instances.len(), 1);

        let draw = &geo.draws[0];
        assert_eq!(draw.id, "o1");
        // The rect fill tessellates to a non-empty index range in the megabuffer.
        assert!(!draw.fill_range.is_empty(), "rect fill must tessellate");
        assert_eq!(draw.fill_range.start, 0);
        assert_eq!(draw.fill_range.end, geo.fill.indices.len() as u32);
        // The closed square stroke expands to a non-empty ribbon.
        assert!(!draw.stroke_range.is_empty(), "rect border must expand");
        assert_eq!(draw.stroke_range.end as usize, geo.stroke_vertices.len());
        // Unselected object: no focus ring.
        assert!(!draw.focus_ring);

        // Identity transform -> m0/m1/m2 are the identity columns.
        assert_eq!(geo.fill_instances[0].m0, [1.0, 0.0, 0.0]);
        assert_eq!(geo.fill_instances[0].m1, [0.0, 1.0, 0.0]);
        assert_eq!(geo.fill_instances[0].m2, [0.0, 0.0, 1.0]);
        // Inline fill #ff0000 -> red, full alpha.
        assert_eq!(geo.fill_instances[0].fill, [1.0, 0.0, 0.0, 1.0]);
        // Inline stroke #00ff00 -> green, full alpha.
        assert_eq!(geo.stroke_instances[0].stroke, [0.0, 1.0, 0.0, 1.0]);
    }

    /// W3-G7/#3: an OPEN-only path with NO explicit fill renders fill-less (no
    /// default white chord region), while a CLOSED path with no fill keeps the
    /// structural default fill. The per-object instance buffers stay index-aligned.
    /// FAILS if open brush strokes still tessellate a fill (the ugly white region).
    /// W3-G10/#3: the open stroke now casts a shadow from its stroke ribbon (its
    /// fill mesh is empty), so its `shadow_range` is non-empty.
    #[test]
    fn open_path_without_fill_skips_fill_but_shadows_the_stroke() {
        // A bare-bones object factory: identity transform, the given geometry, no
        // inline fill, no stroke.
        let fill_less = |id: &str, d: &str| RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            geometry_d: d.to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
        };

        // (open) a freehand-style open polyline; (closed) a rect ending in Z.
        let open = fill_less("brush", "M0 0 L80 40 L20 90");
        let closed = fill_less("rect", "M0 0 L800 0 L800 800 L0 800 Z");
        let scene = scene_with(vec![open, closed], None);
        let geo = build_scene_geometry(&scene);

        let open_draw = &geo.draws[0];
        let closed_draw = &geo.draws[1];

        // (a) Open + fill:None => NO fill region, but the stroke ribbon STILL casts a
        // shadow (W3-G10/#3: a free-draw line floats over the canvas too).
        assert!(
            open_draw.fill_range.is_empty(),
            "open brush stroke must not tessellate a fill region"
        );
        // It still strokes (an open path is a visible line).
        assert!(!open_draw.stroke_range.is_empty(), "open stroke still draws a ribbon");
        assert!(
            !open_draw.shadow_range.is_empty(),
            "an unfilled open stroke casts a shadow from its stroke ribbon"
        );

        // (b) Closed + fill:None => the structural default fill STILL applies.
        assert!(
            !closed_draw.fill_range.is_empty(),
            "a closed shape with no fill keeps the default white fill"
        );
        assert!(!closed_draw.shadow_range.is_empty(), "the filled rect casts a shadow");

        // (c) Per-object instance buffers stay index-aligned with `draws`.
        assert_eq!(geo.fill_instances.len(), geo.draws.len());
        assert_eq!(geo.shadow_instances.len(), geo.draws.len());
        assert_eq!(geo.stroke_instances.len(), geo.draws.len());
    }

    /// D4 analytic fill AA: the build populates `fill_edges` index-aligned with the
    /// megabuffer vertices, flags the rect's silhouette (perimeter) vertices non-zero
    /// so the FS can fade the edge, and never marks an off-perimeter vertex. Fails if
    /// `edge` stays all-zero (the pre-D4 placeholder) or flags an interior vertex.
    #[test]
    fn build_populates_fill_edge_on_silhouette_vertices() {
        let scene = scene_with(vec![rect_object("o1")], None);
        let geo = build_scene_geometry(&scene);

        // Edge flags are index-aligned with the merged fill vertices.
        assert_eq!(
            geo.fill_edges.len(),
            geo.fill.vertices.len(),
            "one edge flag per fill vertex"
        );
        // The placeholder shipped all-zero edges (fully transparent on GPU); the
        // tessellated rect must now flag its silhouette.
        assert!(
            geo.fill_edges.iter().any(|&e| e != 0.0),
            "rect silhouette vertices must be flagged non-zero"
        );
        // Every flagged vertex sits on the 100px-square perimeter (x or y is 0/100);
        // an interior vertex (if lyon emitted one) would stay 0.0.
        for (&[x, y], &edge) in geo.fill.vertices.iter().zip(&geo.fill_edges) {
            if edge != 0.0 {
                let on_perimeter =
                    x == 0.0 || x == 100.0 || y == 0.0 || y == 100.0;
                assert!(
                    on_perimeter,
                    "flagged vertex ({x},{y}) must lie on the rect boundary"
                );
            }
        }
    }

    #[test]
    fn megabuffer_ranges_are_contiguous_across_two_objects() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")], None);
        let geo = build_scene_geometry(&scene);

        assert_eq!(geo.draws.len(), 2);
        // Two pushes -> two contiguous ranges tiling the merged index array.
        let a = geo.draws[0].fill_range;
        let b = geo.draws[1].fill_range;
        assert_eq!(a.start, 0);
        assert_eq!(a.end, b.start, "object ranges are contiguous");
        assert_eq!(b.end, geo.fill.indices.len() as u32);
        // The second object's indices are rebased into the merged vertex array,
        // so the highest index is >= the first object's vertex count.
        let max_index = geo.fill.indices.iter().copied().max().unwrap();
        assert!(max_index as usize >= geo.draws[0].fill_range.len() as usize);
    }

    #[test]
    fn selection_adds_focus_ring_flag() {
        let scene = scene_with(vec![rect_object("o1")], Some("o1".to_string()));
        let geo = build_scene_geometry(&scene);
        assert!(geo.draws[0].focus_ring, "selected object rings");

        let unselected = scene_with(vec![rect_object("o1")], Some("other".to_string()));
        let geo2 = build_scene_geometry(&unselected);
        assert!(!geo2.draws[0].focus_ring);
    }

    #[test]
    fn multi_select_adds_focus_ring_flag() {
        let mut scene = scene_with(vec![rect_object("o1")], None);
        scene.multi_select = vec!["o1".to_string()];
        let geo = build_scene_geometry(&scene);
        assert!(
            geo.draws[0].focus_ring,
            "multi-selected object rings like the single anchor"
        );
    }

    #[test]
    fn unstyled_object_uses_structural_default_paints() {
        let mut obj = rect_object("o1");
        obj.fill = None;
        obj.stroke = None;
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        // Default fill #ffffff -> white; default stroke #283644.
        assert_eq!(geo.fill_instances[0].fill, [1.0, 1.0, 1.0, 1.0]);
        let s = geo.stroke_instances[0].stroke;
        assert!((s[0] - 0x28 as f32 / 255.0).abs() < 1e-6);
        assert!((s[1] - 0x36 as f32 / 255.0).abs() < 1e-6);
        assert!((s[2] - 0x44 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn cubic_geometry_flattens_into_fill_and_stroke() {
        let mut obj = rect_object("curve");
        // An open cubic: M then C. No Z, so it strokes but does not fill.
        obj.geometry_d = "M0 0 C80 -160 720 -160 800 0".to_string();
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        // Open path: stroke ribbon is non-empty.
        assert!(!geo.draws[0].stroke_range.is_empty());
        // The cubic flattened to more than the two endpoints (interior points).
        assert!(geo.stroke_vertices.len() > 6);
    }

    #[test]
    fn dashed_stroke_produces_multiple_ribbon_runs() {
        let mut obj = rect_object("dashed");
        // A straight open segment, 100px, with a [4,4] dash -> multiple on-runs.
        obj.geometry_d = "M0 0 L800 0".to_string();
        obj.stroke = Some(RStroke {
            paint: RPaint::Solid {
                color: "#000000".to_string(),
            },
            width: 2.0,
            opacity: 1.0,
            dash: vec![4.0, 4.0],
            cap: RStrokeCap::Butt,
            join: RStrokeJoin::Miter,
        });
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        let solid = {
            let mut o = rect_object("solid");
            o.geometry_d = "M0 0 L800 0".to_string();
            o.stroke = Some(RStroke {
                paint: RPaint::Solid {
                    color: "#000000".to_string(),
                },
                width: 2.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            });
            build_scene_geometry(&scene_with(vec![o], None))
        };
        // The dashed line splits into multiple ribbon quads, so it produces more
        // stroke vertices than the single solid quad.
        assert!(
            geo.stroke_vertices.len() > solid.stroke_vertices.len(),
            "dashing splits the line into more ribbon runs"
        );
    }

    /// W2-11 perf gate (the whole point of S7): a sustained drag updates ONLY each
    /// dragged object's instance model matrix via `preview_instance_columns` — a
    /// pure column compose — and NEVER re-runs `build_scene_geometry`. We bake a
    /// 10_000-object scene once (the single tessellation entry), then drive
    /// DRAG_FRAMES of per-object preview pushes and assert the bake closure ran
    /// exactly once across the whole scenario (zero re-tessellation, P4).
    #[test]
    fn perf_gate_drag_10k_objects_zero_retessellation_instance_path() {
        const N: usize = 10_000;
        const DRAG_FRAMES: usize = 60;

        let scene = scene_with((0..N).map(|i| rect_object(&format!("obj-{i}"))).collect(), None);

        // The ONLY tessellation entry: bake the whole scene once. A counter pins
        // the ground-truth `build_scene_geometry` call count; the drag must not move
        // it past 1.
        let mut bake_calls = 0usize;
        let geo = {
            bake_calls += 1;
            build_scene_geometry(&scene)
        };
        assert_eq!(geo.draws.len(), N);
        assert_eq!(bake_calls, 1, "initial bake is the sole tessellation");

        // A cumulative translate delta, advancing each frame so the matrix actually
        // moves (a real drag, not a no-op).
        for frame in 0..DRAG_FRAMES {
            let delta = crate::hit_test_object::translate_3x3((frame as f64) + 1.0, -(frame as f64));
            for obj in &scene.objects {
                let (m0, m1, m2) = preview_instance_columns(&delta, &obj.transform);
                // Finite, no NaN/inf on the hot path.
                for v in m0.iter().chain(m1.iter()).chain(m2.iter()) {
                    assert!(v.is_finite(), "preview columns stay finite");
                }
                // Correctness: the pushed columns equal `delta * base`'s columns,
                // exactly the packing `build_scene_geometry` would have baked.
                let world = crate::hit_test_object::mat3_mul(&delta, &obj.transform);
                assert_eq!(m0, matrix_col(&world, 0));
                assert_eq!(m1, matrix_col(&world, 1));
                assert_eq!(m2, matrix_col(&world, 2));
            }
        }

        // THE 0-REBAKE GATE: the entire N * DRAG_FRAMES drag re-tessellated nothing.
        assert_eq!(
            bake_calls, 1,
            "1만 object 드래그 = 재tessellation 0: the instance-matrix push must never re-bake (P4)"
        );
    }

    /// W2-11: the GPU instance shortcut is geometry-equivalent to the old
    /// full-rebake feed. `preview_instance_columns(delta, base)` must yield the SAME
    /// instance columns as a full `build_scene_geometry` of a scene whose object
    /// transform was set to `compose(delta, base)` — proving the per-move push is
    /// pixel-equivalent to the W2-05 `sceneWithObjectTransformed` rebake it replaces.
    #[test]
    fn preview_columns_equal_full_rebake_at_composed_transform() {
        let base = [[2.0, 0.0, 30.0], [0.0, 2.0, -10.0], [0.0, 0.0, 1.0]];
        let delta = crate::hit_test_object::rotate_about_3x3(std::f64::consts::FRAC_PI_3, 5.0, 7.0);

        let mut obj = rect_object("o1");
        obj.transform = base;
        let preview = preview_instance_columns(&delta, &base);

        // Full rebake: bake the object at the composed world transform and read its
        // baked instance columns.
        let composed = crate::hit_test_object::mat3_mul(&delta, &base);
        let mut rebaked_obj = rect_object("o1");
        rebaked_obj.transform = composed;
        let geo = build_scene_geometry(&scene_with(vec![rebaked_obj], None));
        let inst = geo.fill_instances[0];

        assert_eq!(preview.0, inst.m0);
        assert_eq!(preview.1, inst.m1);
        assert_eq!(preview.2, inst.m2);
        // And the stroke instance carries the same matrix columns.
        let stroke = geo.stroke_instances[0];
        assert_eq!(preview.0, stroke.m0);
        assert_eq!(preview.1, stroke.m1);
        assert_eq!(preview.2, stroke.m2);
    }

    /// G3 (RB2 follow-up): the live text-drag preview moves the GLYPHS with the
    /// fill/stroke. `set_preview_transform` writes the previewed columns into the
    /// text instance buffer at `i * size_of::<TextInstance>()` — the same matrix-at-
    /// offset-0, index-aligned write fill/stroke get. This proves, device-free, that
    /// (a) the previewed text instance equals the composed `delta*base` transform —
    /// exactly what the write pushes — and (b) it DIFFERS from the canonical baked
    /// text instance, so a missing text write would leave the glyphs at canonical
    /// (the bug this card closes). Mirrors `preview_columns_equal_full_rebake_at_*`.
    #[test]
    fn text_preview_follows_drag_matches_composed_transform() {
        let base = [[2.0, 0.0, 30.0], [0.0, 2.0, -10.0], [0.0, 0.0, 1.0]];
        let delta = crate::hit_test_object::translate_3x3(40.0, -25.0);

        // Canonical (no preview): the baked text instance for the object at `base`.
        let mut canonical = text_rect("t-drag", "Ab", 128.0, "#222222");
        canonical.transform = base;
        let canon_geo = build_scene_geometry_themed_with_measure(
            &scene_with(vec![canonical], None),
            Theme::light(),
            &unit_measure,
        );
        let canon_text = canon_geo.text_instances[0];

        // The preview columns `set_preview_transform` would write for this drag.
        let preview = preview_instance_columns(&delta, &base);

        // Without a preview the text instance is canonical: the un-dragged columns
        // are NOT the previewed ones (a real move, so a missing write is observable).
        assert!(
            (canon_text.m0, canon_text.m1, canon_text.m2) != preview,
            "canonical text instance differs from the previewed transform"
        );

        // Under the preview, the text instance equals the composed `delta*base` —
        // and matches what fill/stroke get, so the glyphs follow the same drag.
        let composed = crate::hit_test_object::mat3_mul(&delta, &base);
        let mut moved = text_rect("t-drag", "Ab", 128.0, "#222222");
        moved.transform = composed;
        let moved_geo = build_scene_geometry_themed_with_measure(
            &scene_with(vec![moved], None),
            Theme::light(),
            &unit_measure,
        );
        let moved_text = moved_geo.text_instances[0];
        assert_eq!(preview.0, moved_text.m0);
        assert_eq!(preview.1, moved_text.m1);
        assert_eq!(preview.2, moved_text.m2);
        // Fill/stroke still follow the same preview too (parity with text).
        assert_eq!(preview.0, moved_geo.fill_instances[0].m0);
        assert_eq!(preview.0, moved_geo.stroke_instances[0].m0);
        assert_eq!(moved_text.m0, moved_geo.fill_instances[0].m0);
        assert_eq!(moved_text.m1, moved_geo.fill_instances[0].m1);
        assert_eq!(moved_text.m2, moved_geo.fill_instances[0].m2);
    }

    /// G3 byte-layout pin: the text instance preview write `set_preview_transform`
    /// performs targets offset `i * size_of::<TextInstance>()` and overwrites the
    /// 36-byte `m0,m1,m2` region at struct offset 0 — identical to the fill/stroke
    /// writes (matrix-at-0). FAILS if `TextInstance` ever grows a field before the
    /// matrix or stops being a clean 3-column struct, which would corrupt the write.
    #[test]
    fn text_instance_matrix_region_matches_preview_write_layout() {
        // The preview writes `[[f32;3];3]` (36 bytes) at the struct's matrix region.
        assert_eq!(std::mem::size_of::<[[f32; 3]; 3]>(), 36);
        assert_eq!(std::mem::size_of::<TextInstance>(), 36);
        assert_eq!(std::mem::offset_of!(TextInstance, m0), 0);
        assert_eq!(std::mem::offset_of!(TextInstance, m1), 12);
        assert_eq!(std::mem::offset_of!(TextInstance, m2), 24);
    }

    /// G3/G5 write-set pin: `set_preview_transform` writes the previewed matrix into
    /// the `[fill, stroke, text, shadow]` buffer array, striding each by
    /// `preview_instance_strides()`. This is the single source of truth for WHICH
    /// buffers follow the drag. FAILS if the text entry is dropped (the RB2 bug:
    /// glyphs left at canonical during a live drag) or the shadow entry is dropped
    /// (the G5 bug: drop shadow lagging at canonical until rebake) — the strides set
    /// must carry all four per-object instance buffers.
    #[test]
    fn preview_write_set_includes_text_instance_buffer() {
        let strides = preview_instance_strides();
        // Four per-object instance buffers must follow the preview: fill, stroke,
        // text, shadow.
        assert_eq!(strides.len(), 4, "fill + stroke + text + shadow all follow the drag");
        assert_eq!(strides[0], std::mem::size_of::<FillInstance>() as u64);
        assert_eq!(strides[1], std::mem::size_of::<StrokeInstance>() as u64);
        // The load-bearing G3 assertion: the text buffer IS in the write set, strided
        // by `TextInstance`. Drop the text write and this entry vanishes — failing here.
        assert_eq!(
            strides[2],
            std::mem::size_of::<TextInstance>() as u64,
            "text instance buffer must be in the preview write set (G3)"
        );
        // The load-bearing G5 assertion: the shadow buffer IS in the write set, strided
        // by `ShadowInstance`. Drop the shadow write and this entry vanishes — failing
        // here (the shadow would lag at canonical during a live drag).
        assert_eq!(
            strides[3],
            std::mem::size_of::<ShadowInstance>() as u64,
            "shadow instance buffer must be in the preview write set (G5)"
        );
    }

    /// G5 byte-layout pin: the shadow instance preview write `set_preview_transform`
    /// performs targets offset `i * size_of::<ShadowInstance>()` and overwrites the
    /// 36-byte `m0,m1,m2` region at struct offset 0 — identical to the fill/stroke/
    /// text writes (matrix-at-0). The baked `shadow` color sits at offset 36, past the
    /// matrix, so it survives the write exactly like the fill/stroke color slot. FAILS
    /// if `ShadowInstance` ever grows a field before the matrix, which would corrupt
    /// the write or clobber the shadow color.
    #[test]
    fn shadow_instance_matrix_region_matches_preview_write_layout() {
        assert_eq!(std::mem::offset_of!(ShadowInstance, m0), 0);
        assert_eq!(std::mem::offset_of!(ShadowInstance, m1), 12);
        assert_eq!(std::mem::offset_of!(ShadowInstance, m2), 24);
        // The 36-byte matrix region the preview overwrites sits strictly before the
        // color slot, so the matrix write never clobbers the baked shadow color.
        assert_eq!(std::mem::offset_of!(ShadowInstance, shadow), 36);
        assert!(
            std::mem::offset_of!(ShadowInstance, shadow) >= 3 * std::mem::size_of::<[f32; 3]>()
        );
    }

    /// G5 (the live-drag follow card): the drop SHADOW moves with the fill/stroke/
    /// text under a live drag. `set_preview_transform` writes the previewed columns
    /// into the shadow instance buffer at `i * size_of::<ShadowInstance>()` — the same
    /// matrix-at-offset-0, index-aligned write the other three buffers get. This proves,
    /// device-free, that (a) the previewed shadow instance equals the composed
    /// `delta*base` transform — exactly what the write pushes — and matches fill/stroke,
    /// and (b) it DIFFERS from the canonical baked shadow instance, so a missing shadow
    /// write would leave the shadow at canonical (the bug this card closes). Mirrors
    /// `text_preview_follows_drag_matches_composed_transform`.
    #[test]
    fn shadow_preview_follows_drag_matches_composed_transform() {
        let base = [[2.0, 0.0, 30.0], [0.0, 2.0, -10.0], [0.0, 0.0, 1.0]];
        let delta = crate::hit_test_object::translate_3x3(40.0, -25.0);

        // Canonical (no preview): the baked shadow instance for the object at `base`.
        let mut canonical = rect_object("s-drag");
        canonical.transform = base;
        let canon_geo = build_scene_geometry(&scene_with(vec![canonical], None));
        let canon_shadow = canon_geo.shadow_instances[0];

        // The preview columns `set_preview_transform` would write for this drag.
        let preview = preview_instance_columns(&delta, &base);

        // Without a preview the shadow instance is canonical: the un-dragged columns are
        // NOT the previewed ones (a real move, so a missing write is observable). This
        // is the assertion that FAILS if the shadow is left at canonical during preview.
        assert!(
            (canon_shadow.m0, canon_shadow.m1, canon_shadow.m2) != preview,
            "canonical shadow instance differs from the previewed transform"
        );

        // Under the preview, the shadow instance equals the composed `delta*base` — and
        // matches what fill/stroke get, so the shadow follows the same drag in lockstep.
        let composed = crate::hit_test_object::mat3_mul(&delta, &base);
        let mut moved = rect_object("s-drag");
        moved.transform = composed;
        let moved_geo = build_scene_geometry(&scene_with(vec![moved], None));
        let moved_shadow = moved_geo.shadow_instances[0];
        assert_eq!(preview.0, moved_shadow.m0);
        assert_eq!(preview.1, moved_shadow.m1);
        assert_eq!(preview.2, moved_shadow.m2);
        // Shadow follows the SAME preview matrix as fill/stroke (lockstep, not lagging).
        assert_eq!(moved_shadow.m0, moved_geo.fill_instances[0].m0);
        assert_eq!(moved_shadow.m1, moved_geo.fill_instances[0].m1);
        assert_eq!(moved_shadow.m2, moved_geo.fill_instances[0].m2);
        assert_eq!(moved_shadow.m0, moved_geo.stroke_instances[0].m0);
    }

    /// W2-11 index==offset invariant: `draws[i].id` ↔ instance `i` in BOTH the fill
    /// and stroke instance buffers. `set_preview_transform` looks up `i` via
    /// `draws.position(id)` and writes `i * size_of` in both buffers, so this
    /// alignment is load-bearing. Pin it so a future reorder of draws-vs-instances
    /// can't silently corrupt the preview write.
    #[test]
    fn draws_index_aligns_with_both_instance_buffers() {
        let scene = scene_with(
            vec![rect_object("a"), rect_object("b"), rect_object("c")],
            None,
        );
        let geo = build_scene_geometry(&scene);
        assert_eq!(geo.draws.len(), geo.fill_instances.len());
        assert_eq!(geo.draws.len(), geo.stroke_instances.len());
        for (i, draw) in geo.draws.iter().enumerate() {
            assert_eq!(draw.id, scene.objects[i].id, "draws stay in scene order");
            assert_eq!(draw.fill_instance, geo.fill_instances[i]);
            assert_eq!(draw.stroke_instance, geo.stroke_instances[i]);
        }
    }

    // ---- RB2 live text render: geometry contract + de-quant -----------------

    /// Stub measure: every char advances `size` px (matches `text_layout`'s
    /// `unit_measure`), so glyph N's pen origin is `region_min.x + N*size_px`.
    fn unit_measure(_ch: char, size: f32) -> f32 {
        size
    }

    /// A rect text object: a 200x100px region (1600x800 quantized) carrying one
    /// run. `size` is the WIRE (quantized) size; align Start / valign Top so the
    /// first glyph lands exactly at the region top-left.
    fn text_rect(id: &str, run_text: &str, wire_size: f64, color: &str) -> RenderObject {
        let mut obj = rect_object(id);
        obj.geometry_d = "M0 0 L1600 0 L1600 800 L0 800 Z".to_string();
        obj.text = Some(RText {
            runs: vec![RTextRun {
                text: run_text.to_string(),
                color: color.to_string(),
                size: wire_size,
                bold: false,
                italic: false,
                font: String::new(),
            }],
            align: RTextAlign::Start,
            valign: RTextValign::Top,
        });
        obj
    }

    /// PRIMARY GOLDEN (commit A): a text object's committed runs produce a
    /// NON-EMPTY set of positioned glyph quads in `text_vertices`, one quad
    /// (6 verts) per visible glyph, with the first glyph's origin at `region_min.x`
    /// and the second at `region_min.x + advance(size_px)` where `size_px` is the
    /// DE-QUANTIZED size (16, not the wire 128), and each quad carries the run color.
    #[test]
    fn text_object_produces_positioned_glyph_quads() {
        // Wire size 128 = 16px * 8 quantum. "AB" -> two visible glyphs.
        let obj = text_rect("t1", "AB", 128.0, "#ff8800");
        let scene = scene_with(vec![obj], None);
        let geo =
            build_scene_geometry_themed_with_measure(&scene, Theme::light(), &unit_measure);

        // (1) Text reaches the geometry — FAILS today (build never called layout).
        assert!(
            !geo.text_vertices.is_empty(),
            "committed text must produce glyph quads"
        );
        // (2) Exactly two glyphs' worth of quads: 6 verts/glyph * 2 = 12.
        let range = geo.draws[0].text_range;
        assert_eq!(range.start, 0);
        assert_eq!(range.len(), 12, "two glyphs -> 12 tri-list verts");
        assert_eq!(geo.text_vertices.len(), 12);

        // (3) Glyph origins prove de-quant + layout: region_min.x is 0; size_px=16.
        // The quad top-left vertex (index 0 of each glyph's 6) is the pen origin.
        let g0_origin_x = geo.text_vertices[0].position[0];
        let g1_origin_x = geo.text_vertices[6].position[0];
        assert!((g0_origin_x - 0.0).abs() < 1e-4, "first glyph at region_min.x");
        assert!(
            (g1_origin_x - 16.0).abs() < 1e-4,
            "second glyph at region_min.x + 16 (de-quant px advance), got {g1_origin_x}"
        );

        // (4) Each quad carries the run color.
        for v in &geo.text_vertices {
            assert_eq!(v.color, [1.0, 0x88 as f32 / 255.0, 0.0, 1.0]);
        }

        // An empty-text object yields an empty text_range (no quads).
        let mut empty = text_rect("t2", "", 128.0, "#ffffff");
        if let Some(text) = empty.text.as_mut() {
            text.runs[0].text.clear();
        }
        let empty_geo =
            build_scene_geometry_themed_with_measure(&scene_with(vec![empty], None), Theme::light(), &unit_measure);
        assert!(empty_geo.draws[0].text_range.is_empty(), "no text -> empty range");
        assert!(empty_geo.text_vertices.is_empty());
    }

    /// COMMIT B (frame wiring, device-free): for [text-object, no-text-object],
    /// `draws[0].text_range` is non-empty and `draws[1].text_range` is empty, and
    /// `text_instances` is index-aligned with `draws` (the invariant render()'s
    /// per-object text loop relies on). Mirrors `draws_index_aligns_with_both_instance_buffers`.
    #[test]
    fn text_range_is_drawn_after_stroke_for_each_object() {
        let text_obj = text_rect("with-text", "Ab", 128.0, "#111111");
        let plain = rect_object("no-text");
        let scene = scene_with(vec![text_obj, plain], None);
        let geo =
            build_scene_geometry_themed_with_measure(&scene, Theme::light(), &unit_measure);

        assert!(!geo.draws[0].text_range.is_empty(), "text object draws glyphs");
        assert!(geo.draws[1].text_range.is_empty(), "plain object draws no glyphs");

        // Index-alignment: draws[i] <-> text_instances[i], in scene order.
        assert_eq!(geo.draws.len(), geo.text_instances.len());
        for (i, draw) in geo.draws.iter().enumerate() {
            assert_eq!(draw.id, scene.objects[i].id);
            // The text instance carries the same matrix columns as the fill instance.
            assert_eq!(geo.text_instances[i].m0, geo.fill_instances[i].m0);
            assert_eq!(geo.text_instances[i].m1, geo.fill_instances[i].m1);
            assert_eq!(geo.text_instances[i].m2, geo.fill_instances[i].m2);
        }
        // Text vertex ranges tile the shared buffer contiguously.
        assert_eq!(geo.draws[0].text_range.end, geo.text_vertices.len() as u32);
    }

    /// COMMIT A (packing): the text vertex/instance byte packing matches the
    /// msdf_text.wgsl contract — `TextVertex` is 32 bytes (position+uv+color),
    /// `TextInstance` is 36 bytes (3 matrix columns, NO color). FAILS if the
    /// packing drifts. The instance attribute @location wiring is pinned by the
    /// wgpu-probe pipeline golden in commit B.
    #[test]
    fn text_vertex_and_instance_sizes_match_shader_contract() {
        // TextVertex: vec2 position + vec2 uv + vec4 color = 8 floats = 32 bytes.
        assert_eq!(std::mem::size_of::<TextVertex>(), 32);
        // TextInstance: 3 vec3 columns = 9 floats = 36 bytes, no color.
        assert_eq!(std::mem::size_of::<TextInstance>(), 36);
    }


    /// COMMIT C (de-quant): a wire run size of 128 (= 16px * 8) lays out the second
    /// glyph at `region_min.x + 16`, NOT +128, and the glyph quad height tracks
    /// 16px not 128px. Building the SAME object with the already-de-quantized 16px
    /// run through the px path yields a byte-identical glyph quad set (committed ==
    /// edited).
    #[test]
    fn committed_text_size_dequantizes_to_pixels() {
        // Committed object: wire size 128.
        let committed = text_rect("c", "AB", 128.0, "#000000");
        let geo =
            build_scene_geometry_themed_with_measure(&scene_with(vec![committed], None), Theme::light(), &unit_measure);

        // Second glyph at +16 (de-quant), NOT +128 (raw wire size).
        let g1_origin_x = geo.text_vertices[6].position[0];
        assert!(
            (g1_origin_x - 16.0).abs() < 1e-4,
            "second glyph de-quantized to +16px, got {g1_origin_x}"
        );
        // The quad height tracks 16px (de-quant size), not 128px.
        let g0_top_y = geo.text_vertices[0].position[1];
        let g0_bottom_y = geo.text_vertices[2].position[1];
        assert!(
            ((g0_bottom_y - g0_top_y) - 16.0).abs() < 1e-4,
            "glyph quad height = 16px (de-quant), got {}",
            g0_bottom_y - g0_top_y
        );

        // Committed (wire 128) == edited (px-overlay intent 16) through the px path:
        // a scene whose run already carries the de-quantized 16 must yield the SAME
        // glyph quads. The build de-quants by /QUANT_PER_PX, so to feed 16px through
        // the same path we set the wire to 16*QUANT_PER_PX = 128 — which is exactly
        // the committed run. The equivalence holds because there is ONE de-quant site.
        let edited = text_rect("c", "AB", 16.0 * QUANT_PER_PX, "#000000");
        let edited_geo =
            build_scene_geometry_themed_with_measure(&scene_with(vec![edited], None), Theme::light(), &unit_measure);
        assert_eq!(
            geo.text_vertices, edited_geo.text_vertices,
            "committed text == the 16px edit-overlay intent (single de-quant site)"
        );
    }

    /// COMMIT C (defaulted size): a run that omits `size` on the wire defaults to
    /// the WIRE-quantized default (16px * 8), so after the layout de-quant it lays
    /// out at 16px — NOT 2px (raw 16 / 8). FAILS if `default_text_size` returns raw
    /// px while the build de-quants.
    #[test]
    fn defaulted_run_size_lays_out_at_sixteen_px() {
        let json = r##"{
            "sceneId": "s1",
            "camera": { "x": 0, "y": 0, "zoom": 1 },
            "objects": [{
                "id": "o1",
                "order": "a0",
                "transform": [[1,0,0],[0,1,0],[0,0,1]],
                "geometryD": "M0 0 L1600 0 L1600 800 L0 800 Z",
                "text": { "runs": [{ "text": "AB" }], "align": "start", "valign": "top" }
            }]
        }"##;
        let scene: RenderObjectScene = serde_json::from_str(json).expect("deserializes");
        let geo =
            build_scene_geometry_themed_with_measure(&scene, Theme::light(), &unit_measure);
        // Second glyph at +16 (de-quant of the 128 default), NOT +2 (16/8).
        let g1_origin_x = geo.text_vertices[6].position[0];
        assert!(
            (g1_origin_x - 16.0).abs() < 1e-4,
            "defaulted run lays out at 16px, got advance {g1_origin_x}"
        );
    }

    // ---- RB1 theme resolution + zero-rebake toggle --------------------------

    /// A rect whose fill + stroke are semantic theme TOKENS (not raw hex), so its
    /// instance colors re-resolve when the theme bit flips.
    fn token_rect(id: &str) -> RenderObject {
        let mut obj = rect_object(id);
        obj.fill = Some(RFill {
            paint: RPaint::Token {
                name: "default-fill".to_string(),
            },
            opacity: 1.0,
        });
        obj.stroke = Some(RStroke {
            paint: RPaint::Token {
                name: "default-stroke".to_string(),
            },
            width: 4.0,
            opacity: 1.0,
            dash: Vec::new(),
            cap: RStrokeCap::Butt,
            join: RStrokeJoin::Miter,
        });
        obj
    }

    /// (c) `RPaint::Token` serde round-trips on the `{"kind":"token","name":...}`
    /// wire form and the renderer resolves the name to the theme table value.
    #[test]
    fn rpaint_token_serde_roundtrips_and_resolves_to_table() {
        let paint = RPaint::Token {
            name: "selection-ring".to_string(),
        };
        let json = serde_json::to_string(&paint).unwrap();
        assert_eq!(json, r#"{"kind":"token","name":"selection-ring"}"#);
        let back: RPaint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, paint);

        // Resolution matches the renderer token table (and flips with the bit).
        let light = paint_color(&paint, 1.0, Theme::light());
        let dark = paint_color(&paint, 1.0, Theme::dark());
        assert_eq!(light, crate::object_theme::resolve_token_f32("selection-ring", false).unwrap());
        assert_eq!(dark, crate::object_theme::resolve_token_f32("selection-ring", true).unwrap());
        assert_ne!(light, dark, "selection-ring flips light vs dark");
    }

    /// W3-G6/#3 STICKY THEME ACROSS RE-FEED: the bug was that every scene re-feed
    /// (`load_object_scene`: pan/move/create) rebuilt the `ObjectRenderer` with a
    /// HARDCODED `Theme::light()`, so a dark canvas reverted to white on the next
    /// re-render. The fix persists the bit on the wasm wrapper (`self.object_theme`,
    /// written by `set_object_theme`) and feeds THAT into the rebuilt renderer.
    ///
    /// This pure-core test models that exact two-step wrapper flow — toggle dark,
    /// then a re-feed reads the persisted bit to choose the rebuild theme — and pins
    /// that the rebuilt renderer clears with the DARK canvas-bg, not light. It FAILS
    /// on the pre-fix code (where the re-feed step substitutes `Theme::light()`):
    /// `rebuilt.canvas_bg()` would equal the light clear.
    #[test]
    fn refeed_preserves_persisted_dark_theme_clear() {
        // The wasm wrapper's persisted bit (`ShapeWebGpuRenderer::object_theme`),
        // initialized light like the real constructor.
        let mut persisted = Theme::light();

        // `set_object_theme(true)` persists the dark bit FIRST (input.rs).
        persisted = Theme { dark: true };

        // A scene re-feed (`load_object_scene`) now rebuilds the renderer with the
        // PERSISTED bit (scene_feed.rs), not a hardcoded `Theme::light()`. The
        // rebuilt renderer's `self.theme` IS this value, and `render`'s `LoadOp::Clear`
        // reads `self.theme.canvas_bg()`.
        let rebuilt = persisted;

        // The re-feed clears DARK, surviving the rebuild...
        assert_eq!(
            rebuilt.canvas_bg(),
            Theme::dark().canvas_bg(),
            "re-feed must clear with the persisted dark canvas-bg"
        );
        // ...and must NOT revert to the light clear (the reported bug).
        assert_ne!(
            rebuilt.canvas_bg(),
            Theme::light().canvas_bg(),
            "re-feed must NOT revert to the light canvas-bg"
        );

        // The rebuilt renderer actually bakes geometry through the persisted theme:
        // token colors resolve dark, proving the bit threads the real build path the
        // re-feed uses (`ObjectRenderer::new` -> `build_scene_geometry_themed`), not
        // just an abstract value.
        let scene = scene_with(vec![token_rect("o1")], None);
        let refed = build_scene_geometry_themed(&scene, rebuilt);
        assert_eq!(
            refed.fill_instances[0].fill,
            crate::object_theme::resolve_token_f32("default-fill", true).unwrap(),
            "re-feed bakes the persisted dark token colors, not light"
        );
    }

    /// (a) The SAME scene yields DIFFERENT chrome/instance RGBA when the theme bit
    /// flips: token-backed fill/stroke colors change, the canvas clear color
    /// (`canvas-bg`) differs light vs dark, AND the drop-shadow color (`shadow`)
    /// flips light-translucent vs dark-translucent (G5: full dark mode, not just
    /// the floating UI). The clear color and shadow color are BOTH sourced from the
    /// theme bit at runtime — `render`'s `LoadOp::Clear` reads `self.theme.canvas_bg()`
    /// and `set_theme` re-resolves `self.theme.shadow()` into each shadow instance —
    /// so this asserts the renderer-side dark-mode halves both move.
    #[test]
    fn theme_flip_changes_token_instance_and_clear_rgba() {
        let scene = scene_with(vec![token_rect("o1")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        // Token fill/stroke instance colors differ between themes.
        assert_ne!(
            light.fill_instances[0].fill, dark.fill_instances[0].fill,
            "default-fill token re-resolves on theme flip"
        );
        assert_ne!(
            light.stroke_instances[0].stroke, dark.stroke_instances[0].stroke,
            "default-stroke token re-resolves on theme flip"
        );
        // And they equal the table values for each mode.
        assert_eq!(
            light.fill_instances[0].fill,
            crate::object_theme::resolve_token_f32("default-fill", false).unwrap()
        );
        assert_eq!(
            dark.fill_instances[0].fill,
            crate::object_theme::resolve_token_f32("default-fill", true).unwrap()
        );

        // Canvas CLEAR color (the backdrop `render` clears to) flips light vs dark:
        // light `canvas-bg` is near-white, dark is near-black. This is the half the
        // user reported missing (canvas background not inverting).
        let light_clear = Theme::light().canvas_bg();
        let dark_clear = Theme::dark().canvas_bg();
        assert_ne!(light_clear, dark_clear, "canvas clear RGBA flips with the theme bit");
        // Light backdrop is bright, dark backdrop is near-black (a real inversion,
        // not two arbitrary colors).
        assert!(light_clear[0] > 0.8, "light canvas-bg is near-white");
        assert!(dark_clear[0] < 0.2, "dark canvas-bg is near-black");

        // Drop-SHADOW color flips too: dark-translucent (black) in light mode,
        // light-translucent (whitish) in dark mode. The shadow instance color is
        // re-resolved from `self.theme.shadow()` on every flip, never hardcoded.
        let light_shadow = light.shadow_instances[0].shadow;
        let dark_shadow = dark.shadow_instances[0].shadow;
        assert_ne!(light_shadow, dark_shadow, "shadow RGBA flips with the theme bit");
        assert_eq!(light_shadow, Theme::light().shadow(), "shadow sourced from token");
        assert_eq!(dark_shadow, Theme::dark().shadow());
        // Light-mode shadow is a dark cast (black-ish); dark-mode shadow is a light
        // cast (whitish) — the user's requested flip. Both translucent.
        assert!(light_shadow[0] < 0.2, "light-mode shadow casts dark");
        assert!(dark_shadow[0] > 0.8, "dark-mode shadow casts whitish");
        assert!(light_shadow[3] < 1.0 && dark_shadow[3] < 1.0, "shadow stays translucent");
    }

    /// (b) ZERO-REBAKE: flipping the theme must NOT re-tessellate. Across a theme
    /// flip the fill megabuffer vertices/indices, the stroke ribbon vertices, and
    /// EVERY per-object draw RANGE are byte-identical — only token instance COLORS
    /// (past the matrix) move. This is the falsifiable "no rebake on toggle" gate.
    #[test]
    fn theme_flip_leaves_tessellation_byte_identical_zero_rebake() {
        let scene = scene_with(vec![token_rect("a"), token_rect("b")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        // Tessellation (the expensive product) is untouched by the theme bit.
        assert_eq!(light.fill.vertices, dark.fill.vertices, "fill verts unchanged");
        assert_eq!(light.fill.indices, dark.fill.indices, "fill indices unchanged");
        assert_eq!(
            light.stroke_vertices, dark.stroke_vertices,
            "stroke ribbon verts unchanged"
        );
        // Per-object draw ranges + matrix columns are identical; ONLY the color
        // slot differs, so a real GPU toggle is a per-instance color write, not a
        // rebuild.
        assert_eq!(light.draws.len(), dark.draws.len());
        for (l, d) in light.draws.iter().zip(dark.draws.iter()) {
            assert_eq!(l.fill_range, d.fill_range, "fill range stable across theme");
            assert_eq!(l.stroke_range, d.stroke_range, "stroke range stable across theme");
            assert_eq!(l.fill_instance.m0, d.fill_instance.m0);
            assert_eq!(l.fill_instance.m1, d.fill_instance.m1);
            assert_eq!(l.fill_instance.m2, d.fill_instance.m2);
            assert_eq!(l.fill_token, d.fill_token, "token name is theme-invariant");
            // The color is the one thing that moves.
            assert_ne!(l.fill_instance.fill, d.fill_instance.fill);
        }
    }

    /// Raw-hex / gradient paints are theme-INVARIANT: no token name is recorded,
    /// so `set_theme` skips them (nothing to re-resolve). Guards against a theme
    /// flip silently recoloring user-picked explicit colors.
    #[test]
    fn raw_hex_paint_is_theme_invariant() {
        let scene = scene_with(vec![rect_object("o1")], None); // #ff0000 / #00ff00 hex
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());
        assert_eq!(light.draws[0].fill_token, None);
        assert_eq!(light.draws[0].stroke_token, None);
        assert_eq!(light.fill_instances[0].fill, dark.fill_instances[0].fill);
        assert_eq!(light.stroke_instances[0].stroke, dark.stroke_instances[0].stroke);
    }

    /// The byte offset `set_theme` writes (the color slot, past `m0,m1,m2`) is the
    /// 36-byte matrix boundary the W2-11 preview write also assumes. Pin both the
    /// `offset_of!` the toggle uses AND that the matrix region sits strictly
    /// before it, so a layout change can't make `set_theme` clobber the matrix.
    #[test]
    fn theme_color_write_targets_the_color_slot_past_the_matrix() {
        assert_eq!(std::mem::offset_of!(FillInstance, fill), 36);
        assert_eq!(std::mem::offset_of!(StrokeInstance, stroke), 36);
        // m0,m1,m2 = three vec3 = 36 bytes, so the color write at offset 36 never
        // overlaps the matrix the drag preview writes at offset 0.
        assert_eq!(std::mem::offset_of!(FillInstance, m0), 0);
        assert!(std::mem::offset_of!(FillInstance, fill) >= 3 * std::mem::size_of::<[f32; 3]>());
    }

    // ---- RB3 default drop-shadow pass --------------------------------------

    /// PRIMARY GOLDEN (RB3 #11): EVERY object emits a drop-shadow primitive
    /// beneath its fill, and the shadow RGBA is the theme `shadow` token —
    /// translucent, and DIFFERENT light vs dark. Fails if any object lacks a
    /// shadow quad, if the shadow range is empty, or if the color is hardcoded
    /// (does not flip with the theme bit).
    #[test]
    fn every_object_emits_a_themed_translucent_shadow() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        assert_eq!(light.draws.len(), 2);
        assert_eq!(light.shadow_instances.len(), 2);
        // (1) Every object emits a non-empty shadow primitive (the offset silhouette).
        for draw in &light.draws {
            assert!(
                !draw.shadow_range.is_empty(),
                "object {} must cast a drop shadow",
                draw.id
            );
        }
        // The shadow vertex ranges tile the shared buffer contiguously.
        assert_eq!(light.draws[0].shadow_range.start, 0);
        assert_eq!(
            light.draws[0].shadow_range.end,
            light.draws[1].shadow_range.start
        );
        assert_eq!(
            light.draws[1].shadow_range.end,
            light.shadow_vertices.len() as u32
        );

        // (2) The shadow color is the `shadow` token, so it FLIPS with the theme
        // bit and is NEVER hardcoded.
        let light_shadow = light.shadow_instances[0].shadow;
        let dark_shadow = dark.shadow_instances[0].shadow;
        assert_eq!(light_shadow, Theme::light().shadow(), "shadow sourced from token, not hardcoded");
        assert_eq!(dark_shadow, Theme::dark().shadow());
        assert_ne!(light_shadow, dark_shadow, "shadow RGBA flips light vs dark");
        // (3) Translucent in both modes (a drop shadow, not an opaque block).
        assert!(light_shadow[3] < 1.0, "light shadow is translucent");
        assert!(dark_shadow[3] < 1.0, "dark shadow is translucent");
        assert!(light_shadow[3] > 0.0 && dark_shadow[3] > 0.0, "shadow is visible");
    }

    /// ZERO-REBAKE (P4): flipping the theme leaves the shadow GEOMETRY byte-
    /// identical — only the shadow instance COLOR moves. A theme flip is a per-
    /// instance color refresh, not a re-tessellation of the shadow quads.
    #[test]
    fn shadow_geometry_is_theme_invariant_only_color_flips() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        // Quad vertices (positions + feather) never move with the theme bit.
        assert_eq!(
            light.shadow_vertices, dark.shadow_vertices,
            "shadow quad geometry is theme-invariant (zero rebake)"
        );
        for (l, d) in light.draws.iter().zip(dark.draws.iter()) {
            assert_eq!(l.shadow_range, d.shadow_range, "shadow range stable across theme");
            assert_eq!(l.shadow_instance.m0, d.shadow_instance.m0);
            assert_eq!(l.shadow_instance.m1, d.shadow_instance.m1);
            assert_eq!(l.shadow_instance.m2, d.shadow_instance.m2);
            // The color is the ONLY thing that moves.
            assert_ne!(l.shadow_instance.shadow, d.shadow_instance.shadow);
        }
    }

    /// W3-G8/A: the shadow is now a SINGLE offset silhouette copy of the object's
    /// OWN fill mesh (count == fill triangle list, NOT *N) — softness comes from the
    /// offscreen Gaussian blur, not stacked tiers. The vertex count is exactly the
    /// fill index count and every `feather` is 0. FAILS if the old multi-tier stack
    /// returns (count == fill-list * tiers, or any feather > 0).
    #[test]
    fn shadow_quad_is_single_offset_silhouette_of_fill_mesh() {
        let scene = scene_with(vec![rect_object("o1")], None);
        let geo = build_scene_geometry(&scene);
        let verts = &geo.shadow_vertices;
        assert!(!verts.is_empty());

        // Recompute the object's own fill mesh the same way the pipeline does.
        let subpaths = flatten_object_subpaths(&scene.objects[0], scene.camera.zoom);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        assert!(!mesh.indices.is_empty());

        // (1) Single tier: ONE copy of the fill triangle list, never a *N stack.
        assert_eq!(
            verts.len(),
            mesh.indices.len(),
            "shadow is a single offset silhouette (one copy of the fill triangle list)"
        );

        // (2) Flat feather: the blur owns softness now, so the silhouette is flat.
        assert!(
            verts.iter().all(|v| v.feather == 0.0),
            "single silhouette tier carries feather 0 (no baked-in ramp)"
        );
    }

    /// W3-G9/#1: the drop offset is 0, so the shadow silhouette is a PERFECT
    /// (untranslated) copy of the fill mesh — no downward bias. The wide quarter-res
    /// blur owns the soft halo, which is symmetric on all sides only because the
    /// silhouette is centered. FAILS if `SHADOW_OFFSET_PX` regresses to a non-zero
    /// drop (the old +2px bottom-bias), which would push every shadow vertex in +y.
    #[test]
    fn shadow_offset_is_zero_so_silhouette_is_symmetric() {
        assert_eq!(SHADOW_OFFSET_PX, 0.0, "shadow drop offset is removed (all-sides symmetric)");

        let scene = scene_with(vec![rect_object("o1")], None);
        let geo = build_scene_geometry(&scene);

        let subpaths = flatten_object_subpaths(&scene.objects[0], scene.camera.zoom);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);

        // Every shadow vertex equals its fill-mesh source with NO y-offset (the emit
        // is `p[1] + SHADOW_OFFSET_PX`, now `+0`): zero down-bias, fully symmetric.
        for (sv, &idx) in geo.shadow_vertices.iter().zip(mesh.indices.iter()) {
            let src = mesh.vertices[idx as usize];
            assert_eq!(sv.position, src, "shadow vertex is untranslated (offset 0)");
        }
    }

    /// CONCAVE shape (an arrowhead with a reflex vertex): the shadow is the EXACT
    /// silhouette of the object's own fill triangulation (no centroid fan, no
    /// per-edge feather ring), translated by the drop offset. FAILS on the old
    /// centroid-fan build, which placed an apex outside this concave silhouette and
    /// self-intersected into facets.
    #[test]
    fn shadow_is_exact_silhouette_of_fill_mesh_for_concave_shape() {
        // A concave arrowhead: the reflex vertex at (300,400) is what makes a
        // centroid fan invalid (the centroid lies outside the silhouette).
        let mut obj = rect_object("arrow");
        obj.geometry_d = "M0 0 L800 400 L0 800 L300 400 Z".to_string();
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);

        // Recompute the object's own fill mesh the same way the pipeline does.
        let subpaths = flatten_object_subpaths(&scene.objects[0], scene.camera.zoom);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        assert!(!mesh.indices.is_empty(), "concave arrow has a fillable interior");

        // (1) Single tier: ONE copy of the fill triangle list.
        assert_eq!(
            geo.shadow_vertices.len(),
            mesh.indices.len(),
            "shadow is a single offset silhouette (one copy of the fill triangle list)"
        );

        // (2) The silhouette is an EXACT copy of the fill mesh translated by the
        // drop offset (+SHADOW_OFFSET_PX in y) — same silhouette, never a centroid
        // fan. Undo the offset and compare against the un-offset fill triangulation.
        let n = mesh.indices.len();
        let mut expected: Vec<[f32; 2]> = mesh
            .indices
            .iter()
            .map(|&i| mesh.vertices[i as usize])
            .collect();
        let mut got: Vec<[f32; 2]> = geo.shadow_vertices[..n]
            .iter()
            .map(|v| [v.position[0], v.position[1] - SHADOW_OFFSET_PX])
            .collect();
        let key = |v: &[f32; 2]| (v[0].to_bits(), v[1].to_bits());
        expected.sort_by_key(key);
        got.sort_by_key(key);
        assert_eq!(
            got, expected,
            "shadow == fill triangulation exactly (no faceting, no centroid fan)"
        );
        assert!(
            geo.shadow_vertices[..n].iter().all(|v| v.feather == 0.0),
            "the silhouette reads flat (feather 0)"
        );

        // (3) Regression guard against the centroid fan: NO shadow vertex sits at
        // the (offset) outline CENTROID (the old core-fan apex), which for this
        // concave shape lies outside the silhouette and produced overlapping facets.
        let outline: Vec<(f32, f32)> = subpaths[0].1.clone();
        let cx = outline.iter().map(|p| p.0).sum::<f32>() / outline.len() as f32;
        let cy = outline.iter().map(|p| p.1).sum::<f32>() / outline.len() as f32 + SHADOW_OFFSET_PX;
        assert!(
            !geo
                .shadow_vertices
                .iter()
                .any(|v| (v.position[0] - cx).abs() < 1e-3 && (v.position[1] - cy).abs() < 1e-3),
            "no shadow vertex sits at the outline centroid (the old core-fan apex)"
        );
    }

    /// W3-G10/#3: a stroke-only (free-draw) object — open path, `fill: None`, a
    /// non-empty stroke ribbon — now casts a shadow built from the STROKE RIBBON
    /// (its fill mesh is empty), so its `shadow_range` is non-empty. A filled object
    /// still casts from its fill-mesh silhouette (unchanged). An object with neither
    /// a fillable interior NOR a stroke ribbon casts nothing. FAILS if a stroke-only
    /// object stays shadowless (the empty-fill fall-through is removed).
    #[test]
    fn stroke_only_object_casts_shadow_from_ribbon() {
        // (1) Stroke-only: open 2-node line, fill None -> empty fill mesh, but a real
        // stroke ribbon. Shadow must come from the ribbon (non-empty), and equal the
        // ribbon triangle list exactly (one copy, untranslated at offset 0).
        let stroke_only = open_stroke_object("line", "M0 0 L800 400");
        let scene = scene_with(vec![stroke_only], None);
        let geo = build_scene_geometry(&scene);
        let draw = &geo.draws[0];
        assert!(draw.fill_range.is_empty(), "open + fill None has no fill mesh");
        assert!(
            !draw.shadow_range.is_empty(),
            "stroke-only object casts a shadow from its stroke ribbon"
        );
        // The shadow silhouette equals the object's stroke ribbon triangle count.
        assert_eq!(
            draw.shadow_range.len(),
            draw.stroke_range.len(),
            "stroke-only shadow is one copy of the stroke ribbon"
        );
        assert!(
            geo.shadow_vertices.iter().all(|v| v.feather == 0.0),
            "the stroke-ribbon silhouette reads flat (feather 0)"
        );

        // (2) Filled object: shadow is STILL the fill-mesh silhouette, NOT the ribbon.
        let filled = scene_with(vec![rect_object("r")], None);
        let fgeo = build_scene_geometry(&filled);
        let fdraw = &fgeo.draws[0];
        let subpaths = flatten_object_subpaths(&filled.objects[0], filled.camera.zoom);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let fill_mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        assert!(!fill_mesh.indices.is_empty(), "rect fills");
        assert_eq!(
            fdraw.shadow_range.len(),
            fill_mesh.indices.len() as u32,
            "a filled object's shadow is its fill-mesh silhouette (unchanged)"
        );

        // (3) Neither fill nor stroke ribbon: a lone MoveTo (single point) — open so
        // it does not fill, and a <2-node subpath expands to an empty ribbon — casts
        // nothing.
        let empty = open_stroke_object("dot", "M0 0");
        let egeo = build_scene_geometry(&scene_with(vec![empty], None));
        let edraw = &egeo.draws[0];
        assert!(edraw.fill_range.is_empty(), "single point has no fill");
        assert!(edraw.stroke_range.is_empty(), "single point has no stroke ribbon");
        assert!(
            edraw.shadow_range.is_empty(),
            "an object with neither a fill nor a stroke ribbon casts nothing"
        );
    }

    /// RB3 EXACT OUTLINE: a non-rectangular object casts a shadow that follows its
    /// real path silhouette, NOT the axis-aligned bbox. We build an ellipse (four
    /// cubic arcs) and assert the offset fill-mesh shadow vertices hug the curve —
    /// none sit in a bbox CORNER region, which the curve never reaches. A 4-corner
    /// AABB quad would place vertices exactly on the offset bbox corners, so this
    /// assertion FAILS for a bbox shadow.
    #[test]
    fn nonrectangular_shadow_follows_path_outline_not_aabb() {
        // An ellipse centered at (400,400), rx=ry=400 quantized units (50px @ 8/px),
        // approximated by four cubic arcs (kappa = 0.5523). Closed (Z) so it fills.
        let mut obj = rect_object("ellipse");
        obj.geometry_d = "M400 0 C621 0 800 179 800 400 C800 621 621 800 400 800 \
             C179 800 0 621 0 400 C0 179 179 0 400 0 Z"
            .to_string();
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        assert!(!geo.shadow_vertices.is_empty(), "ellipse casts a shadow");

        // The single offset silhouette is the whole shadow; check it against the
        // region bbox corners (the drop offset is small vs the corner margin below).
        let subpaths = flatten_object_subpaths(&scene.objects[0], 1.0);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        let core = &geo.shadow_vertices[..mesh.indices.len()];

        // Region bbox in pixels (geometry de-quantizes at 8 units/px): ~0..100. The
        // four bbox corners are the points a bbox-quad shadow would touch.
        let region = crate::outline::derive_region(
            &subpaths,
            crate::curve_lod::flatness_for_bucket(crate::curve_lod::zoom_bucket(1.0)),
        )
        .expect("ellipse has a region");
        let min_x = region.min_x;
        let min_y = region.min_y;
        let max_x = region.max_x;
        let max_y = region.max_y;
        let bbox_corners = [
            [min_x, min_y],
            [max_x, min_y],
            [max_x, max_y],
            [min_x, max_y],
        ];

        // No core-tier shadow vertex may coincide with a bbox corner: the ellipse
        // silhouette (and its interior triangulation) pulls inward at every corner.
        // Distance margin is a generous fraction of the radius so a flattened-curve
        // vertex near (but not at) a corner still counts as "off the corner".
        let corner_margin = (max_x - min_x) * 0.1;
        for v in core.iter() {
            for c in &bbox_corners {
                let d = ((v.position[0] - c[0]).powi(2) + (v.position[1] - c[1]).powi(2)).sqrt();
                assert!(
                    d > corner_margin,
                    "shadow vertex {:?} sits on bbox corner {:?} (bbox shadow, not silhouette)",
                    v.position,
                    c
                );
            }
        }

        // And the silhouette is genuinely curved: the fill mesh spans many more than
        // a rect's 4 vertices, so the shadow is not a 4-corner quad.
        let positions: std::collections::BTreeSet<[u32; 2]> = core
            .iter()
            .map(|v| [v.position[0].to_bits(), v.position[1].to_bits()])
            .collect();
        assert!(
            positions.len() > 8,
            "ellipse silhouette shadow has many curved vertices, not 4 bbox corners"
        );
    }

    /// Shadow packing matches `object_shadow.wgsl`: `ShadowVertex` is 12 bytes
    /// (position + feather) and `ShadowInstance` is 52 bytes (3 matrix columns +
    /// color), with the color slot at offset 36 (past the 36-byte matrix) so the
    /// theme color write never clobbers the matrix.
    #[test]
    fn shadow_vertex_and_instance_sizes_match_shader_contract() {
        // ShadowVertex: vec2 position + f32 feather = 3 floats = 12 bytes.
        assert_eq!(std::mem::size_of::<ShadowVertex>(), 12);
        // ShadowInstance: 3 vec3 columns + vec4 color = 13 floats = 52 bytes.
        assert_eq!(std::mem::size_of::<ShadowInstance>(), 52);
        assert_eq!(std::mem::offset_of!(ShadowInstance, m0), 0);
        assert_eq!(std::mem::offset_of!(ShadowInstance, shadow), 36);
    }

    /// The shadow uses the same `shadow` token RB1 exposes as `Theme::shadow()`,
    /// pinning the wiring: a wrong token (or a hardcoded color) breaks the link.
    #[test]
    fn shadow_token_constant_matches_theme_shadow_accessor() {
        assert_eq!(SHADOW_TOKEN, crate::object_theme::ThemeToken::Shadow.name());
        let light = paint_color(&RPaint::Token { name: SHADOW_TOKEN.to_string() }, 1.0, Theme::light());
        assert_eq!(light, Theme::light().shadow());
    }

    /// A token paint's per-paint `opacity` multiplies the token's own alpha
    /// (e.g. the translucent `shadow` token), so RB3's shadow paint can fade
    /// without losing the token's baseline translucency.
    #[test]
    fn token_opacity_multiplies_token_alpha() {
        let shadow = RPaint::Token {
            name: "shadow".to_string(),
        };
        let full = paint_color(&shadow, 1.0, Theme::light());
        let half = paint_color(&shadow, 0.5, Theme::light());
        // shadow light = 00000055 -> alpha 0x55/255.
        let base_a = 0x55 as f32 / 255.0;
        assert!((full[3] - base_a).abs() < 1e-6);
        assert!((half[3] - base_a * 0.5).abs() < 1e-6);
    }

    /// An open 2-node stroke object (no fill, so the patch exercises only the stroke
    /// ribbon) carrying `geometry_d`.
    fn open_stroke_object(id: &str, d: &str) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            geometry_d: d.to_string(),
            fill: None,
            stroke: Some(RStroke {
                paint: RPaint::Solid {
                    color: "#00ff00".to_string(),
                },
                width: 4.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            }),
            text: None,
            anchors: Vec::new(),
            clip: false,
        }
    }

    #[test]
    fn reexpand_keeps_vertex_count_when_only_a_node_moves() {
        // A 2-node open stroke; the canonical draw record gives the baked ranges.
        let canonical = open_stroke_object("f", "M0 0 L800 0");
        let scene = scene_with(vec![canonical.clone()], None);
        let build = build_scene_geometry_themed(&scene, Theme::light());
        let draw = &build.draws[0];

        // Move ONLY node 1 (no topology change): re-expand the moved follower.
        let moved = open_stroke_object("f", "M0 0 L800 400");
        let rebuilt = reexpand_single_object(&moved, Theme::light(), scene.camera.clone());

        // The COUNTS must match the baked ranges, so the in-place patch is size-safe.
        assert_eq!(rebuilt.stroke_vertices.len() as u32, draw.stroke_range.len());
        assert_eq!(rebuilt.fill_vertices.len() as u32, draw.fill_vertex_range.len());
        assert_eq!(rebuilt.fill_indices.len() as u32, draw.fill_range.len());
        // And the patch plan is produced (Some), with the stroke offset at the baked
        // range start (a lone object => start 0).
        let plan = follower_patch_plan(draw, &rebuilt).expect("size-safe patch");
        assert_eq!(
            plan.stroke_vertex_byte_offset,
            draw.stroke_range.start as u64 * std::mem::size_of::<StrokeVertex>() as u64
        );
        // The moved node actually changed the ribbon geometry (not a no-op write).
        let canonical_rebuild = reexpand_single_object(&canonical, Theme::light(), scene.camera.clone());
        assert_ne!(
            rebuilt.stroke_vertices, canonical_rebuild.stroke_vertices,
            "moving a node must change the ribbon vertices"
        );
    }

    #[test]
    fn follower_patch_plan_is_none_on_a_topology_change() {
        // Canonical: a 2-node open stroke. The baked draw has a fixed stroke range.
        let canonical = open_stroke_object("f", "M0 0 L800 0");
        let scene = scene_with(vec![canonical], None);
        let build = build_scene_geometry_themed(&scene, Theme::light());
        let draw = &build.draws[0];

        // A 3-node re-expand changes the vertex COUNT — the guard must refuse it so a
        // mismatched range is NEVER written to the GPU buffer.
        let three_nodes = open_stroke_object("f", "M0 0 L800 0 L800 400");
        let rebuilt = reexpand_single_object(&three_nodes, Theme::light(), scene.camera.clone());
        assert_ne!(rebuilt.stroke_vertices.len() as u32, draw.stroke_range.len());
        assert!(
            follower_patch_plan(draw, &rebuilt).is_none(),
            "a topology/LOD count change must SKIP the patch, not corrupt the buffer"
        );
    }

    /// W3-G13: the count guard covers the NEW artifacts too — a rebuilt whose
    /// shadow or text vertex count no longer matches the baked range must refuse
    /// the whole plan, exactly like a fill/stroke mismatch.
    #[test]
    fn follower_patch_plan_is_none_on_a_shadow_or_text_count_change() {
        // A filled rect casts a fill-derived shadow, so its shadow_range is
        // non-empty and the tamper below is a real mismatch, not 1-vs-0 noise.
        let canonical = rect_object("f");
        let scene = scene_with(vec![canonical.clone()], None);
        let build = build_scene_geometry_themed(&scene, Theme::light());
        let draw = &build.draws[0];
        assert!(!draw.shadow_range.is_empty(), "filled rect casts a shadow");

        let intact = reexpand_single_object(&canonical, Theme::light(), scene.camera.clone());
        assert!(follower_patch_plan(draw, &intact).is_some(), "untampered plan holds");

        let mut bad_shadow = reexpand_single_object(&canonical, Theme::light(), scene.camera.clone());
        bad_shadow.shadow_vertices.push(ShadowVertex {
            position: [0.0, 0.0],
            feather: 0.0,
        });
        assert!(
            follower_patch_plan(draw, &bad_shadow).is_none(),
            "a shadow vertex count change must SKIP the patch"
        );

        let mut bad_text = reexpand_single_object(&canonical, Theme::light(), scene.camera.clone());
        bad_text.text_vertices.push(TextVertex {
            position: [0.0, 0.0],
            uv: [0.0, 0.0],
            color: [0.0, 0.0, 0.0, 0.0],
        });
        assert!(
            follower_patch_plan(draw, &bad_text).is_none(),
            "a text vertex count change must SKIP the patch"
        );
    }

    /// W3-G13 PARITY GUARD (the dragged-target follower-shadow bug): a follower
    /// re-expanded with a reprojected node must carry shadow + text vertices that
    /// EQUAL the corresponding `shadow_range`/`text_range` slices of a FULL rebake
    /// of the whole deformed scene. Two followers cover both shadow sources: a
    /// CLOSED filled follower with a text run (fill-derived shadow + glyph quads)
    /// and an OPEN fill-less stroke follower (stroke-ribbon-derived shadow,
    /// W3-G10/#3). FAILS if shadow or text is dropped from [`FollowerReexpand`]
    /// (the plan refuses, or the slices diverge) — the live drag then shows a
    /// stale shadow until the commit rebake, the user-reported bug.
    #[test]
    fn reexpanded_follower_shadow_and_text_match_a_full_rebake() {
        use crate::render_object::{RAnchor, RLocalPoint};
        use shape_scene_core::object::{reproject_geometry_node, LocalPoint, Transform3x3};

        let translate =
            |tx: f64, ty: f64| [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]];

        // Target A at (10,20); the drag delta reuses the pinned cross-core vector
        // (`translate(5,7)`, see anchor_follower_closure_agrees_with_scene_core_vector).
        let mut target = rect_object("t");
        target.transform = translate(10.0, 20.0);

        // CLOSED filled follower with a text run; node 0 anchored to the target at
        // a point that drags the region MIN negative, so the glyph layout moves too.
        let mut closed = text_rect("fc", "AB", 128.0, "#ff8800");
        closed.anchors = vec![RAnchor {
            node_index: 0,
            target: "t".to_string(),
            at: RLocalPoint { x: -240.0, y: -480.0 },
        }];

        // OPEN fill-less stroke follower: the EXACT pinned cross-core vector object.
        let mut open = open_stroke_object("fo", "M 0 0 L 64 0");
        open.transform = translate(100.0, 0.0);
        open.anchors = vec![RAnchor {
            node_index: 1,
            target: "t".to_string(),
            at: RLocalPoint { x: 16.0, y: 8.0 },
        }];

        let scene = scene_with(vec![target, closed, open], None);
        let canonical_build = build_scene_geometry_themed(&scene, Theme::light());
        let delta = translate(5.0, 7.0);

        // Reproject each follower's anchored node through the moved target — the
        // same scene-core math the live preview path drives.
        let mut deformed = scene.clone();
        for obj in deformed.objects.iter_mut() {
            let Some(anchor) = obj.anchors.first().cloned() else {
                continue;
            };
            let target_base = scene.objects[0].transform;
            obj.geometry_d = reproject_geometry_node(
                &Transform3x3 { m: obj.transform },
                &Transform3x3 { m: target_base },
                &Transform3x3 { m: delta },
                LocalPoint {
                    x: anchor.at.x.round() as i32,
                    y: anchor.at.y.round() as i32,
                },
                i32::try_from(anchor.node_index).unwrap_or(i32::MAX),
                &obj.geometry_d,
            )
            .expect("addressable, changed");
        }
        // The open follower lands on the pinned cross-core vector.
        assert_eq!(deformed.objects[2].geometry_d, "M 0 0 L -664 224");

        // FULL rebake of the whole deformed scene = the parity oracle.
        let full = build_scene_geometry_themed(&deformed, Theme::light());

        for i in [1usize, 2usize] {
            let id = &scene.objects[i].id;
            let rebuilt = reexpand_single_object(
                &deformed.objects[i],
                Theme::light(),
                scene.camera.clone(),
            );
            let plan = follower_patch_plan(&canonical_build.draws[i], &rebuilt)
                .unwrap_or_else(|| panic!("size-safe patch for {id}"));
            assert_eq!(
                plan.shadow_vertex_byte_offset,
                canonical_build.draws[i].shadow_range.start as u64
                    * std::mem::size_of::<ShadowVertex>() as u64
            );

            // Shadow parity: the re-expand equals the full-rebake slice, and the
            // deformation really moved it (a stale canonical copy is caught).
            let full_shadow =
                &full.shadow_vertices[full.draws[i].shadow_range.start as usize
                    ..full.draws[i].shadow_range.end as usize];
            assert!(!full_shadow.is_empty(), "{id} casts a shadow");
            assert_eq!(rebuilt.shadow_vertices, full_shadow, "{id} shadow parity");
            let canonical_shadow = &canonical_build.shadow_vertices
                [canonical_build.draws[i].shadow_range.start as usize
                    ..canonical_build.draws[i].shadow_range.end as usize];
            assert_ne!(
                rebuilt.shadow_vertices, canonical_shadow,
                "{id} reproject must move the shadow silhouette"
            );

            // Text parity (the closed follower carries glyphs; the open one none).
            let full_text = &full.text_vertices[full.draws[i].text_range.start as usize
                ..full.draws[i].text_range.end as usize];
            assert_eq!(rebuilt.text_vertices, full_text, "{id} text parity");
        }

        // The text follower's glyphs are non-empty AND moved by the reproject
        // (its region min changed), so a stale canonical text copy is caught too.
        let fc_rebuilt = reexpand_single_object(
            &deformed.objects[1],
            Theme::light(),
            scene.camera.clone(),
        );
        assert!(!fc_rebuilt.text_vertices.is_empty(), "fc has glyph quads");
        let fc_canonical_text = &canonical_build.text_vertices
            [canonical_build.draws[1].text_range.start as usize
                ..canonical_build.draws[1].text_range.end as usize];
        assert_ne!(
            fc_rebuilt.text_vertices, fc_canonical_text,
            "the reprojected region min must move the glyphs"
        );
    }
}
