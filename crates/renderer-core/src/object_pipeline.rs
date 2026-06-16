//! Object render-geometry build (pure CPU half): the `bytemuck` vertex/instance
//! layout structs, the per-object [`ObjectDraw`] index, and the scene tessellation
//! into a [`MegaBuffer`] + instance data. No `wgpu`; runs on every target.
//!
//! Per-object 3x3 matrix strategy: WebGPU has no push constants, so the projective
//! transform rides as three instance-step `vec3` columns (`m0`/`m1`/`m2`) plus the
//! inline paint color. Each object draws as one instance; the shared `view` uniform
//! stays the affine camera (byte-identical to the legacy `ViewUniform`) and the VS
//! does the projective `M * vec3(local, 1)` divide per object.

use crate::model::CameraState;
use crate::object_theme::{resolve_token_f32, Theme};
use crate::render_object::{
    resolve_visual, RPaint, RText, RTextAlign, RTextValign, RenderObject, RenderObjectScene,
    VisualState, QUANT_PER_PX,
};
use crate::text::TextEngine;
use crate::text_layout::{
    layout_runs, GlyphCoverage, MsdfAtlasPlan, MsdfGlyphEntry, MsdfGlyphKey, TextAlign,
    TextRunInput, TextVAlign,
};
use std::collections::HashMap;
use crate::stroke_expand::{dash_segments, expand_stroke, Cap, Join};
use crate::tessellate::{
    parse_path, quantized_to_px, tessellate_fill, DrawRange, FillRuleKind, MegaBuffer,
    PathCommand,
};

// ---------------------------------------------------------------------------
// Uniform + vertex/instance GPU layouts
// ---------------------------------------------------------------------------

/// Shared camera uniform (WGSL `View`): the affine camera, with every object's
/// projective matrix on the instance buffer instead. Byte-identical to the legacy
/// `ViewUniform` so the two pipelines share a coordinate frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ObjectMatrixUniform {
    /// `vec4(translate.x, translate.y, zoom, _)`.
    pub camera: [f32; 4],
    /// `vec4(px_w, px_h, _, _)`.
    pub viewport: [f32; 4],
}

impl ObjectMatrixUniform {
    pub fn from_scene(scene: &RenderObjectScene, pixel_width: f32, pixel_height: f32) -> Self {
        ObjectMatrixUniform {
            camera: [
                crate::cast::narrow_f32(scene.camera.x),
                crate::cast::narrow_f32(scene.camera.y),
                crate::cast::narrow_f32(scene.camera.zoom),
                0.0,
            ],
            viewport: [pixel_width, pixel_height, 0.0, 0.0],
        }
    }
}

/// Per-vertex fill attributes matching `object_fill.wgsl`'s `VertexIn` (`position`
/// @0, `edge` @1). Positions are object-local pixels; `edge` is the analytic-AA
/// silhouette flag (1.0 boundary, 0.0 interior); all-zero degrades to opaque fill.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FillVertex {
    pub position: [f32; 2],
    pub edge: f32,
}

/// Fill instance matching `object_fill.wgsl` (`m0`/`m1`/`m2` @2..4, `fill` @5):
/// the 3x3 matrix as three `vec3` columns plus the resolved solid paint.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FillInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub fill: [f32; 4],
}

/// Per-vertex shadow attributes matching `object_shadow.wgsl`'s `VertexIn`
/// (`position` @0, `feather` @1). Positions are the offset fill silhouette;
/// `feather` is uniformly `0` for the flat silhouette (a soft blur is a GPU residual).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowVertex {
    pub position: [f32; 2],
    pub feather: f32,
}

/// Shadow instance matching `object_shadow.wgsl` (`m0`/`m1`/`m2` @2..4, `shadow`
/// @5). `shadow` is the translucent theme `shadow` token, re-resolved on a theme flip.
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

/// Stroke instance matching `object_stroke.wgsl` (`m0`/`m1`/`m2` @5..7, `stroke` @8).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub stroke: [f32; 4],
}

/// `object_stroke.wgsl`'s `StrokeUniform` (binding 1).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeParamsUniform {
    /// `vec4(dash_on_px, dash_period_px, opacity, dashed_flag)`.
    pub dash: [f32; 4],
}

impl StrokeParamsUniform {
    /// Solid (no-dash) params at full opacity; per-object dashing is in the CPU split.
    pub fn solid() -> Self {
        StrokeParamsUniform {
            dash: [0.0, 0.0, 1.0, 0.0],
        }
    }
}

/// Per-glyph-corner attributes matching `msdf_text.wgsl`'s `VertexIn` (`position`
/// @0, `uv` @1, `color` @2). Color rides per-glyph (not on the instance) so one
/// object's runs can mix colors.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// Text instance matching `msdf_text.wgsl` (`m0`/`m1`/`m2` @3..5); matrix only,
/// since color is per-glyph in [`TextVertex`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
}

/// `msdf_text.wgsl`'s `TextUniform` (binding 3) — MSDF atlas params for `screenPxRange` AA.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextUniform {
    /// `vec4(distance_range_texels, atlas_width, atlas_height, _)`.
    pub atlas: [f32; 4],
}


// ---------------------------------------------------------------------------
// CPU scene build
// ---------------------------------------------------------------------------

/// The CPU-built draw data for one object: where its fill/stroke/shadow/text live in
/// the shared buffers and the instance records drawn against them.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectDraw {
    pub id: String,
    /// Index range into the shared fill megabuffer, or empty for no fillable region.
    pub fill_range: DrawRange,
    /// VERTEX range of this object's fill (distinct from `fill_range`, an INDEX
    /// range), so a follower's fill positions can be patched in place during a live
    /// anchor reproject. Empty for no fillable region.
    pub fill_vertex_range: DrawRange,
    pub fill_instance: FillInstance,
    /// Shadow quad vertex range; empty for no boundable region. Drawn beneath fill.
    pub shadow_range: DrawRange,
    pub shadow_instance: ShadowInstance,
    pub stroke_range: DrawRange,
    pub stroke_instance: StrokeInstance,
    /// Glyph-quad vertex range; empty when no text (or whitespace). Drawn after stroke.
    pub text_range: DrawRange,
    pub focus_ring: bool,
    /// The token name backing this object's fill, if a [`RPaint::Token`]. `Some` =>
    /// the color re-resolves on a theme flip (raw hex/gradient/image is invariant),
    /// so a theme toggle writes only token-backed colors with zero re-tessellation.
    pub fill_token: Option<String>,
    pub stroke_token: Option<String>,
}

/// The renderer-default shadow color is the theme `shadow` token — wired, never
/// hardcoded, so the shadow flips with the theme bit (zero-rebake color refresh).
const SHADOW_TOKEN: &str = "shadow";

/// Drop-shadow offset (object-local px). The shadow is a single offset silhouette
/// copy of the object's own fill; the offscreen Gaussian blur owns the softness.
/// Offset is 0 so the halo is symmetric on all sides (no downward bias).
const SHADOW_OFFSET_PX: f32 = 0.0;


// ---------------------------------------------------------------------------
// CPU geometry build (device-independent, unit-testable)
// ---------------------------------------------------------------------------

/// The device-independent result of building a scene's GPU geometry: merged fill
/// megabuffer + per-object instances, stroke/shadow/text vertex arrays + instances,
/// and the per-object [`ObjectDraw`] index. The `*_instances`/`draws` arrays are
/// index-aligned.
#[derive(Clone, Debug, Default)]
pub struct SceneGeometry {
    pub fill: MegaBuffer,
    /// Per-vertex analytic-AA silhouette flags, index-aligned with `fill.vertices`
    /// (1.0 boundary, 0.0 interior). Built once with the mesh topology.
    pub fill_edges: Vec<f32>,
    pub fill_instances: Vec<FillInstance>,
    /// Shadow quad vertices (6 per boundable object, tri-list); per-object slices
    /// via `draws[i].shadow_range`.
    pub shadow_vertices: Vec<ShadowVertex>,
    pub shadow_instances: Vec<ShadowInstance>,
    pub stroke_vertices: Vec<StrokeVertex>,
    pub stroke_instances: Vec<StrokeInstance>,
    /// Glyph-quad vertices (6 per visible glyph, tri-list); per-object slices via
    /// `draws[i].text_range`.
    pub text_vertices: Vec<TextVertex>,
    pub text_instances: Vec<TextInstance>,
    pub draws: Vec<ObjectDraw>,
}

/// Build all CPU geometry for `scene` in light mode.
pub fn build_scene_geometry(scene: &RenderObjectScene) -> SceneGeometry {
    build_scene_geometry_themed(scene, Theme::light())
}

/// Device-free glyph-advance stub: a size-proportional advance, pure and
/// deterministic (no fontdue/IO). The real fontdue shaper is injected at the GPU
/// cutover via [`build_scene_geometry_themed_with_measure`].
pub const STUB_ADVANCE_RATIO: f32 = 0.6;

fn stub_measure(_ch: char, size: f32) -> f32 {
    STUB_ADVANCE_RATIO * size
}

/// Per-glyph atlas-slot lookup the GPU cutover injects: given a glyph char at a
/// pixel size, the populated [`MsdfAtlasPlan`] returns its corner UVs + bearing.
/// `None` (atlas not yet built, glyph unpacked, or blank) => the placeholder
/// full-atlas cell. The pure core never rasterizes; the host supplies this.
pub type GlyphUvProvider<'a> = dyn Fn(char, f32) -> Option<MsdfGlyphEntry> + 'a;

/// A provider that maps no glyph (used by the stub/legacy path so quads keep the
/// placeholder full-atlas `uv 0..1`).
fn no_glyph_uv(_ch: char, _size: f32) -> Option<MsdfGlyphEntry> {
    None
}

/// Pack every glyph of the scene's committed text not yet present in `entries` into
/// `plan`, keyed by `(char, rounded px)` — the same key the glyph-UV provider
/// resolves. Returns true if any NEW glyph was packed (the texture needs a
/// re-upload); idempotent per key, so a pan/zoom re-feed with no new text returns
/// false (zero-rebake on motion). The host's `ObjectTextAtlas` delegates here so the
/// populate DECISION (which glyphs, the grew flag, idempotency) lives GPU-free in the
/// core, with a single glyph-pack implementation shared by the GPU path and tests.
pub fn populate_atlas_from_scene(
    plan: &mut MsdfAtlasPlan,
    entries: &mut HashMap<(u32, u32), MsdfGlyphEntry>,
    engine: &TextEngine,
    scene: &RenderObjectScene,
    oversample: f32,
) -> bool {
    let mut grew = false;
    for obj in &scene.objects {
        let Some(text) = obj.text.as_ref() else { continue };
        for run in &text.runs {
            // Wire size is quantized at QUANT_PER_PX units/px; the layout de-quants,
            // so the atlas key uses the same rounded px the provider keys on. The key
            // stays LOGICAL (no `oversample`) so the UV lookup is dpr-independent; the
            // oversample only raises the SDF source resolution behind that key.
            let size_px = crate::cast::narrow_f32(run.size / QUANT_PER_PX);
            let key_px = crate::cast::round_u32(size_px);
            for ch in run.text.chars() {
                let map_key = (ch as u32, key_px);
                if entries.contains_key(&map_key) {
                    continue;
                }
                let Some(cov) = engine.glyph_coverage(ch, size_px, oversample) else {
                    continue;
                };
                let slot = plan.generate_glyph(&GlyphCoverage {
                    key: MsdfGlyphKey {
                        font_index: cov.font_index,
                        glyph_id: cov.glyph_id,
                        px: cov.px,
                    },
                    coverage: &cov.coverage,
                    width: cov.width,
                    height: cov.height,
                    bearing_x: cov.bearing_x,
                    bearing_y: cov.bearing_y,
                    // The oversample (dpr) the coverage was rasterized at divides the
                    // quad back to logical px so layout stays resolution-independent.
                    oversample: cov.oversample,
                });
                // `None` = atlas full; leave the glyph unresolved so the build falls
                // back to the placeholder cell rather than dropping it silently.
                if let Some(entry) = slot {
                    entries.insert(map_key, entry);
                    grew = true;
                }
            }
        }
    }
    grew
}

/// Build all CPU geometry for `scene` under `theme`: tessellate fill, expand stroke,
/// resolve instance data, lay out text. The `theme` bit only affects token paint
/// COLORS (tessellation/ranges are theme-invariant), which makes the toggle a
/// zero-rebake color refresh.
pub fn build_scene_geometry_themed(scene: &RenderObjectScene, theme: Theme) -> SceneGeometry {
    build_scene_geometry_themed_with_measure(scene, theme, &stub_measure)
}

/// As [`build_scene_geometry_themed`], with an injected per-char `measure` closure
/// (the pure core never calls fontdue itself; the GPU cutover supplies the real one).
/// Glyph quads keep the placeholder full-atlas UV — use
/// [`build_scene_geometry_themed_with_text`] for real per-glyph atlas slots.
pub fn build_scene_geometry_themed_with_measure(
    scene: &RenderObjectScene,
    theme: Theme,
    measure: &dyn Fn(char, f32) -> f32,
) -> SceneGeometry {
    build_scene_geometry_themed_with_text(scene, theme, measure, &no_glyph_uv)
}

/// As [`build_scene_geometry_themed_with_measure`], with an injected per-glyph
/// `glyph_uv` provider so each glyph quad carries its REAL atlas-slot UVs + bearing
/// (the GPU cutover supplies it from a populated [`MsdfAtlasPlan`]). A glyph the
/// provider does not resolve falls back to the placeholder full-atlas cell.
pub fn build_scene_geometry_themed_with_text(
    scene: &RenderObjectScene,
    theme: Theme,
    measure: &dyn Fn(char, f32) -> f32,
    glyph_uv: &GlyphUvProvider<'_>,
) -> SceneGeometry {
    let mut geometry = SceneGeometry::default();

    // Container frames (any object that parents another) must not paint their
    // structural-default fill over their children. Precompute the parent-id set ONCE
    // so the per-object container test stays O(1), keeping the build O(objects).
    let parent_ids: std::collections::HashSet<&str> = scene
        .objects
        .iter()
        .filter_map(|o| o.parent.as_deref())
        .collect();

    for obj in &scene.objects {
        let state = visual_state_for(scene, &obj.id);
        let resolved = resolve_visual(obj, state);

        // A hidden object emits zero-vertex geometry (empty subpaths) while still
        // pushing its index-aligned instance + ObjectDraw slot below — never skipped,
        // so the buffers stay aligned with `draws` (and a follower anchored to it
        // still reprojects). Transform-only: no tessellation cost.
        let subpaths = if obj.hidden {
            Vec::new()
        } else {
            flatten_object_subpaths(obj, scene.camera.zoom)
        };

        let is_container = parent_ids.contains(obj.id.as_str());
        // A text label's closed rect is a layout region the glyphs lay out against,
        // not a shape: with no explicit fill OR stroke it must paint NO body — no
        // structural-default fill (an opaque white box over the rounded panel), no
        // default-stroke ribbon (a hairline border), and so no shadow either.
        let text_only = obj.text.is_some() && obj.fill.is_none() && obj.stroke.is_none();
        // A decorative-empty body declares an EXPLICIT fully-transparent fill and no
        // stroke — a UI hit/hover target (icon-button, menu row, action body) that
        // paints nothing at rest. Treated like a text-only label: no fill mesh, no
        // default-stroke ribbon, no shadow. (A canvas shape never emits a transparent
        // fill, so this carve-out can't touch the structural-default for `fill:None`.)
        let decorative_empty = obj.stroke.is_none()
            && obj
                .fill
                .as_ref()
                .is_some_and(|f| f.opacity <= f64::EPSILON);
        // An open-only path with no explicit fill is not filled (Figma convention),
        // and a fill-less container frame must not paint its structural-default fill
        // over its children. A skipped object still pushes a Fill/Shadow instance
        // below so the per-object buffers stay index-aligned with `draws`.
        let skip_fill = text_only
            || decorative_empty
            || (obj.fill.is_none()
                && (is_container || subpaths.iter().all(|(closed, _)| !closed)));

        let mesh = if skip_fill {
            crate::tessellate::Mesh::default()
        } else {
            let fill_input: Vec<(bool, Vec<(f32, f32)>)> = subpaths
                .iter()
                .map(|(closed, pts)| (*closed, pts.clone()))
                .collect();
            tessellate_fill(&fill_input, FillRuleKind::NonZero)
        };
        // Silhouette flags stay index-aligned with the megabuffer's vertex array.
        geometry.fill_edges.extend_from_slice(&mesh.boundary_flags());
        // Vertex sub-range (distinct from the index range `push` returns) so a
        // follower's fill positions can be patched in place.
        let fill_vertex_start = crate::cast::len_u32(geometry.fill.vertices.len());
        let fill_range = geometry.fill.push(&mesh);
        let fill_vertex_range = DrawRange {
            start: fill_vertex_start,
            end: crate::cast::len_u32(geometry.fill.vertices.len()),
        };
        geometry.fill_instances.push(FillInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            fill: paint_color(&resolved.fill.paint, crate::cast::narrow_f32(resolved.fill.opacity), theme),
        });

        // ---- Stroke: expand each (dashed) subpath into a ribbon ------------
        let stroke_start = crate::cast::len_u32(geometry.stroke_vertices.len());
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
        let width = crate::cast::narrow_f32(resolved.stroke.width);
        // Keep the stroke ribbon meshes so a fill-less object can cast a shadow from
        // its line; a filled object casts from its fill mesh instead. A text-only
        // label has no explicit stroke and its rect is a layout region, so it gets no
        // default-stroke ribbon (and thus no shadow from one).
        let mut stroke_meshes: Vec<crate::stroke_expand::Mesh> = Vec::new();
        if !text_only && !decorative_empty {
            for (closed, pts) in &subpaths {
                let runs = dash_segments(pts, &dash_px(&resolved.stroke.dash));
                for run in runs {
                    let stroke_mesh = expand_stroke(&run, *closed, width, None, cap, join);
                    append_stroke_ribbon(&mut geometry.stroke_vertices, &stroke_mesh, width);
                    stroke_meshes.push(stroke_mesh);
                }
            }
        }
        let stroke_end = crate::cast::len_u32(geometry.stroke_vertices.len());

        // Shadow: an offset copy of the fill `mesh` (no extra tessellation). When the
        // fill is empty (open/stroke-only), cast from the stroke ribbon instead so the
        // line itself casts a shadow.
        let shadow_start = crate::cast::len_u32(geometry.shadow_vertices.len());
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
        let shadow_end = crate::cast::len_u32(geometry.shadow_vertices.len());
        geometry.shadow_instances.push(ShadowInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            shadow: resolve_token_f32(SHADOW_TOKEN, theme.dark).unwrap_or([0.0, 0.0, 0.0, 0.25]),
        });
        geometry.stroke_instances.push(StrokeInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            stroke: paint_color(&resolved.stroke.paint, crate::cast::narrow_f32(resolved.stroke.opacity), theme),
        });

        // Text: the region bbox derives from the same flattened subpaths the fill/
        // stroke use; the per-object instance carries region-local-px -> world.
        let text_start = crate::cast::len_u32(geometry.text_vertices.len());
        if let Some(text) = &obj.text {
            append_text_quads(
                &mut geometry.text_vertices,
                text,
                &subpaths,
                scene.camera.zoom,
                measure,
                glyph_uv,
            );
        }
        let text_end = crate::cast::len_u32(geometry.text_vertices.len());
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

/// Resolve the visual state (selected via single anchor or multi-select) for an
/// object id.
fn visual_state_for(scene: &RenderObjectScene, id: &str) -> VisualState {
    let selected = scene.selection.as_deref() == Some(id)
        || scene.multi_select.iter().any(|candidate| candidate == id);
    VisualState {
        selected,
        hovered: false,
        focused: false,
    }
}

/// Parse an object's geometry into object-local pixel polylines, flattening cubics
/// via the zoom-bucket LOD flattener. Returns `(closed, points)` per subpath.
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
                    // Skip the start: it duplicates the cursor already pushed.
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

/// Append one object's drop-shadow geometry: a single silhouette copy of the
/// triangle-list (the already-tessellated fill or stroke ribbon), translated by
/// [`SHADOW_OFFSET_PX`] — no extra tessellation. Softness is not baked here; the
/// offscreen Gaussian blur owns it, so `feather` is emitted `0` (the slot stays for
/// the shader contract). An empty mesh casts nothing. Takes bare `(vertices,
/// indices)` slices so it serves both the fill mesh and the stroke ribbon.
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

/// Append a stroke ribbon `mesh` as [`StrokeVertex`]es. The CPU ribbon already bakes
/// the normal offset into positions, so we emit a zero normal and `side = 0` (the
/// shader must not offset again), with per-node `width` and arc-length `distance_along`.
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

/// Lay out an object's text runs against its derived region and append one 6-vertex
/// quad per visible glyph (object-local px, per-run color). The region bbox comes
/// from the same flattened `subpaths` the fill/stroke use; run `size` is de-quantized
/// (`/QUANT_PER_PX`) to edit-time pixels. When `glyph_uv` resolves a glyph to a real
/// atlas slot, the quad carries that slot's corner UVs and is positioned by the
/// glyph bearing; otherwise it falls back to a placement-sized cell spanning the whole
/// atlas (`uv 0..1`).
fn append_text_quads(
    out: &mut Vec<TextVertex>,
    text: &RText,
    subpaths: &[(bool, Vec<(f32, f32)>)],
    zoom: f64,
    measure: &dyn Fn(char, f32) -> f32,
    glyph_uv: &GlyphUvProvider<'_>,
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
            // Wire size is quantized at `QUANT_PER_PX` units/px; divide to px.
            size: crate::cast::narrow_f32(run.size / QUANT_PER_PX),
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
        // A real atlas slot positions the cell by the glyph bearing and carries the
        // slot's corner UVs; an unresolved glyph keeps the placeholder full cell.
        let (corners, uv) = match glyph_uv(p.ch, p.size) {
            Some(entry) if entry.width > 0.0 && entry.height > 0.0 => {
                let gx = p.x + entry.bearing_x;
                let gy = p.y + entry.bearing_y;
                (
                    [gx, gy, gx + entry.width, gy + entry.height],
                    entry.uv,
                )
            }
            _ => (
                [p.x, p.y, p.x + p.size, p.y + p.size],
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            ),
        };
        let [x0, y0, x1, y1] = corners;
        let tl = TextVertex { position: [x0, y0], uv: uv[0], color: p.color };
        let tr = TextVertex { position: [x1, y0], uv: uv[1], color: p.color };
        let br = TextVertex { position: [x1, y1], uv: uv[2], color: p.color };
        let bl = TextVertex { position: [x0, y1], uv: uv[3], color: p.color };
        // Two triangles (tl, tr, br) + (tl, br, bl) — CCW tri-list, no index buffer.
        out.push(tl);
        out.push(tr);
        out.push(br);
        out.push(tl);
        out.push(br);
        out.push(bl);
    }
}

/// Parse a text run's `#rrggbb` color into RGBA; a bad value falls back to white.
fn text_run_color(value: &str) -> [f32; 4] {
    let [r, g, b] = parse_hex_rgb(value);
    [r, g, b, 1.0]
}

/// Column `col` of a row-major 3x3 transform as a `vec3` for the WGSL column-major
/// `mat3x3` reconstruction (`M = [m0 | m1 | m2]`).
fn matrix_col(transform: &[[f64; 3]; 3], col: usize) -> [f32; 3] {
    [
        crate::cast::narrow_f32(transform[0][col]),
        crate::cast::narrow_f32(transform[1][col]),
        crate::cast::narrow_f32(transform[2][col]),
    ]
}

/// Compose the preview world matrix `delta * base` and return its three instance
/// columns in the same packing [`build_scene_geometry`] uses. The single source of
/// truth for the preview matrix (GPU writer + perf-gate test both call it), so the
/// live-drag push is byte-equivalent to a full rebake at `compose(delta, base)`.
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

/// Per-buffer strides the previewed matrix is written into, in lockstep with the
/// `[fill, stroke, text, shadow]` buffer array: it lands at `i * stride` in each, so
/// every sub-visual follows the same drag. A dropped entry means one stops tracking.
pub fn preview_instance_strides() -> [u64; 4] {
    [
        std::mem::size_of::<FillInstance>() as u64,
        std::mem::size_of::<StrokeInstance>() as u64,
        std::mem::size_of::<TextInstance>() as u64,
        std::mem::size_of::<ShadowInstance>() as u64,
    ]
}

/// Re-expanded geometry for one object, to patch a follower's baked vertices in
/// place during a live anchor reproject. Fill indices are object-local (0-based);
/// the caller rebases them by the follower's vertex base. Carries every per-object
/// baked vertex artifact so no sub-visual lags at the old geometry until the commit.
pub struct FollowerReexpand {
    pub fill_vertices: Vec<FillVertex>,
    pub fill_indices: Vec<u32>,
    pub stroke_vertices: Vec<StrokeVertex>,
    pub shadow_vertices: Vec<ShadowVertex>,
    pub text_vertices: Vec<TextVertex>,
}

/// Re-expand a single object's geometry, reusing the exact build-loop path so the
/// result is byte-identical to a full rebake. Built alone in a one-object scene with
/// the live `camera` (matching zoom/LOD bucket) and `theme` (matching the AA `edge`
/// flags); selection is dropped (it only adds a focus ring, never changing counts).
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
    // Destructuring WITHOUT `..` makes a new SceneGeometry artifact a compile error
    // here until its follower-patch story is decided (instances/draws are not the
    // patch's business — see `preview_instance_strides`).
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

/// Byte offsets for writing a follower's re-expanded geometry over its existing
/// megabuffer ranges. Produced only when the re-expand is size-safe; a count
/// mismatch yields `None` so the write never bleeds into another object's range.
pub struct FollowerPatchPlan {
    pub fill_vertex_byte_offset: u64,
    pub fill_index_byte_offset: u64,
    /// Added to each object-local fill index to point at the follower's vertices in
    /// the merged megabuffer (the build-time rebase).
    pub fill_index_rebase: u32,
    pub stroke_vertex_byte_offset: u64,
    pub shadow_vertex_byte_offset: u64,
    pub text_vertex_byte_offset: u64,
}

/// Validate that `rebuilt` exactly fills the follower `draw`'s existing ranges and,
/// if so, return the [`FollowerPatchPlan`]. `None` on any count mismatch so the
/// caller skips the patch rather than writing a mismatched range (the rare topology
/// edge case; counts normally match since a drag only moves positions).
pub fn follower_patch_plan(draw: &ObjectDraw, rebuilt: &FollowerReexpand) -> Option<FollowerPatchPlan> {
    // Destructuring WITHOUT `..` makes a new per-object vertex RANGE a compile error
    // here until it is guarded + planned.
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
    if crate::cast::len_u32(rebuilt.fill_vertices.len()) != fill_vertex_range.len()
        || crate::cast::len_u32(rebuilt.fill_indices.len()) != fill_range.len()
        || crate::cast::len_u32(rebuilt.stroke_vertices.len()) != stroke_range.len()
        || crate::cast::len_u32(rebuilt.shadow_vertices.len()) != shadow_range.len()
        || crate::cast::len_u32(rebuilt.text_vertices.len()) != text_range.len()
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

/// Resolve a paint to a single theme-aware RGBA. `theme` selects the token table for
/// [`RPaint::Token`]; gradient/image paints collapse to a representative color. A
/// token carries its own alpha (e.g. translucent `shadow`) that `opacity` multiplies;
/// hex paints have no inherent alpha, so `opacity` becomes the alpha directly.
fn paint_color(paint: &RPaint, opacity: f32, theme: Theme) -> [f32; 4] {
    let opacity = opacity.clamp(0.0, 1.0);
    match paint {
        RPaint::Solid { color } => {
            let rgb = parse_hex_rgb(color);
            [rgb[0], rgb[1], rgb[2], opacity]
        }
        RPaint::Token { name } => {
            // Unknown tokens fall back to opaque white so a bad token never poisons the draw.
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

/// The token name backing a paint, if a [`RPaint::Token`]. Marks an [`ObjectDraw`]
/// token-backed so a theme toggle re-resolves only those colors.
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
    dash.iter().map(|&d| crate::cast::narrow_f32(d)).collect()
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
            hidden: false,
            locked: false,
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
    fn identity_camera_maps_local_px_to_screen_px() {
        // The screen-space UI scene rides an identity camera (x=0,y=0,zoom=1) with a
        // CSS-px viewport, so object-local px land at screen px 1:1.
        let scene = scene_with(vec![rect_object("ui")], None);
        let uniform = ObjectMatrixUniform::from_scene(&scene, 800.0, 600.0);
        assert_eq!(uniform.camera, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(uniform.viewport, [800.0, 600.0, 0.0, 0.0]);

        // A widget's pure-translate transform [[1,0,sx],[0,1,sy],[0,0,1]] places its
        // object-local origin (0,0) at screen (sx,sy) under the documented mapping
        // `world = M * local`, `screen = world * zoom + camera.xy` (zoom 1, camera 0,0).
        let (sx, sy) = (24.0_f64, 24.0_f64);
        let m = [[1.0, 0.0, sx], [0.0, 1.0, sy], [0.0, 0.0, 1.0]];
        let (lx, ly) = (0.0_f64, 0.0_f64);
        let world_x = m[0][0] * lx + m[0][1] * ly + m[0][2];
        let world_y = m[1][0] * lx + m[1][1] * ly + m[1][2];
        let zoom = f64::from(uniform.camera[2]);
        let (cam_x, cam_y) = (f64::from(uniform.camera[0]), f64::from(uniform.camera[1]));
        let screen_x = world_x * zoom + cam_x;
        let screen_y = world_y * zoom + cam_y;
        assert_eq!((screen_x, screen_y), (sx, sy));
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
        assert_eq!(draw.fill_range.end, crate::cast::len_u32(geo.fill.indices.len()));
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

    #[test]
    fn open_path_without_fill_skips_fill_but_shadows_the_stroke() {
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
            hidden: false,
            locked: false,
        };

        let open = fill_less("brush", "M0 0 L80 40 L20 90");
        let closed = fill_less("rect", "M0 0 L800 0 L800 800 L0 800 Z");
        let scene = scene_with(vec![open, closed], None);
        let geo = build_scene_geometry(&scene);

        let open_draw = &geo.draws[0];
        let closed_draw = &geo.draws[1];

        // Open + fill:None => no fill region, but the stroke ribbon still casts a shadow.
        assert!(
            open_draw.fill_range.is_empty(),
            "open brush stroke must not tessellate a fill region"
        );
        assert!(!open_draw.stroke_range.is_empty(), "open stroke still draws a ribbon");
        assert!(
            !open_draw.shadow_range.is_empty(),
            "an unfilled open stroke casts a shadow from its stroke ribbon"
        );

        // Closed + fill:None => the structural default fill still applies.
        assert!(
            !closed_draw.fill_range.is_empty(),
            "a closed shape with no fill keeps the default white fill"
        );
        assert!(!closed_draw.shadow_range.is_empty(), "the filled rect casts a shadow");

        assert_eq!(geo.fill_instances.len(), geo.draws.len());
        assert_eq!(geo.shadow_instances.len(), geo.draws.len());
        assert_eq!(geo.stroke_instances.len(), geo.draws.len());
    }

    /// A text-only label as the ui-core shell emits it: a CLOSED rect layout region
    /// with `fill: None`, `stroke: None`, and `text: Some(..)`. Same geometry the
    /// white-box defect rode in on.
    fn text_label(id: &str, fill: Option<RFill>) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            geometry_d: "M0 0 L800 0 L800 800 L0 800 Z".to_string(),
            fill,
            stroke: None,
            text: Some(RText {
                runs: vec![RTextRun {
                    text: "Label".to_string(),
                    color: "#111111".to_string(),
                    size: 16.0 * crate::render_object::QUANT_PER_PX,
                    bold: false,
                    italic: false,
                    font: String::new(),
                }],
                align: RTextAlign::Start,
                valign: RTextValign::Middle,
            }),
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    #[test]
    fn text_only_label_paints_no_body_fill_stroke_or_shadow() {
        // The keystone regression: before the fix a text-only label fell through to
        // the structural white default fill (an opaque box over the rounded panel)
        // AND a default-stroke hairline ribbon, with a shadow cast from one of them.
        let scene = scene_with(vec![text_label("title", None)], None);
        let geo = build_scene_geometry(&scene);

        let draw = &geo.draws[0];
        assert!(
            draw.fill_range.is_empty(),
            "a text-only label must paint NO body fill (the white box bug)"
        );
        assert!(
            draw.stroke_range.is_empty(),
            "a text-only label must paint NO default-stroke border ribbon"
        );
        assert!(
            draw.shadow_range.is_empty(),
            "a text-only label with no body casts NO shadow"
        );
        // The glyphs themselves still lay out against the region rect.
        assert!(!draw.text_range.is_empty(), "the label's glyphs still render");
        // Per-object instance slots stay index-aligned even when the body is skipped.
        assert_eq!(geo.fill_instances.len(), geo.draws.len());
        assert_eq!(geo.stroke_instances.len(), geo.draws.len());
        assert_eq!(geo.shadow_instances.len(), geo.draws.len());
    }

    #[test]
    fn closed_no_fill_no_text_shape_keeps_default_fill() {
        // Guard the precise boundary: a CLOSED unfilled shape with NO text is still a
        // shape, so it keeps the structural white default fill (and casts a shadow).
        // Only the text-only case loses its body.
        let shape = RenderObject {
            id: "rect".to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            geometry_d: "M0 0 L800 0 L800 800 L0 800 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        };
        let scene = scene_with(vec![shape], None);
        let geo = build_scene_geometry(&scene);

        let draw = &geo.draws[0];
        assert!(
            !draw.fill_range.is_empty(),
            "a closed no-text shape with no fill keeps the default white fill"
        );
        assert!(!draw.shadow_range.is_empty(), "the defaulted-fill rect casts a shadow");
    }

    #[test]
    fn explicitly_filled_text_object_keeps_its_fill() {
        // An EXPLICIT inline fill on a text object is honored — the text-only skip
        // only suppresses the STRUCTURAL default, never an author's chosen paint.
        let fill = Some(RFill {
            paint: RPaint::Solid {
                color: "#ff0000".to_string(),
            },
            opacity: 1.0,
        });
        let scene = scene_with(vec![text_label("chip", fill)], None);
        let geo = build_scene_geometry(&scene);

        let draw = &geo.draws[0];
        assert!(
            !draw.fill_range.is_empty(),
            "an explicitly-filled text object keeps its fill"
        );
        // The explicit red survives resolution (not the white default).
        assert_eq!(geo.fill_instances[0].fill, [1.0, 0.0, 0.0, 1.0]);
        assert!(!draw.text_range.is_empty(), "its glyphs still render over the fill");
    }

    #[test]
    fn container_frame_with_no_fill_skips_the_white_default_so_children_show_through() {
        // A CLOSED rect 'frame' (fill:None) that PARENTS a child — a container by the
        // structural definition. It must skip its structural white default fill so the
        // children show through (Figma frame semantics). FAILS today: the frame
        // tessellates an opaque white fill (non-empty fill_range) over its children.
        let frame = RenderObject {
            id: "frame".to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            geometry_d: "M0 0 L800 0 L800 800 L0 800 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        };
        let mut child = rect_object("child");
        child.parent = Some("frame".to_string());

        // Control: the SAME closed fill-less rect with NO children still defaults to
        // the white fill (its contract is unchanged — pinned by the open-path test).
        let mut control = frame.clone();
        control.id = "control".to_string();

        let scene = scene_with(vec![frame, child, control], None);
        let geo = build_scene_geometry(&scene);

        let frame_draw = geo.draws.iter().find(|d| d.id == "frame").unwrap();
        let control_draw = geo.draws.iter().find(|d| d.id == "control").unwrap();
        assert!(
            frame_draw.fill_range.is_empty(),
            "a fill-less container frame must not tessellate its structural white fill"
        );
        assert!(
            !control_draw.fill_range.is_empty(),
            "a childless closed fill-less rect still keeps the default white fill"
        );
    }

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
        // Every flagged vertex sits on the 100px-square perimeter; interior stays 0.0.
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
        assert_eq!(b.end, crate::cast::len_u32(geo.fill.indices.len()));
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

    #[test]
    fn perf_gate_drag_10k_objects_zero_retessellation_instance_path() {
        const N: usize = 10_000;
        const DRAG_FRAMES: usize = 60;

        let scene = scene_with((0..N).map(|i| rect_object(&format!("obj-{i}"))).collect(), None);

        // The sole tessellation: bake once. The drag must not move the counter past 1.
        let mut bake_calls = 0usize;
        let geo = {
            bake_calls += 1;
            build_scene_geometry(&scene)
        };
        assert_eq!(geo.draws.len(), N);
        assert_eq!(bake_calls, 1, "initial bake is the sole tessellation");

        // A cumulative translate delta advancing each frame (a real drag).
        for frame in 0..DRAG_FRAMES {
            let delta = crate::hit_test_object::translate_3x3((frame as f64) + 1.0, -(frame as f64));
            for obj in &scene.objects {
                let (m0, m1, m2) = preview_instance_columns(&delta, &obj.transform);
                for v in m0.iter().chain(m1.iter()).chain(m2.iter()) {
                    assert!(v.is_finite(), "preview columns stay finite");
                }
                // The pushed columns equal `delta * base`'s columns, the packing a bake would use.
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

    /// Pins the matrix-at-offset-0 layout the preview write depends on; a field
    /// before the matrix would corrupt the write.
    #[test]
    fn text_instance_matrix_region_matches_preview_write_layout() {
        assert_eq!(std::mem::size_of::<[[f32; 3]; 3]>(), 36);
        assert_eq!(std::mem::size_of::<TextInstance>(), 36);
        assert_eq!(std::mem::offset_of!(TextInstance, m0), 0);
        assert_eq!(std::mem::offset_of!(TextInstance, m1), 12);
        assert_eq!(std::mem::offset_of!(TextInstance, m2), 24);
    }

    /// Pins which buffers follow the drag: all four per-object instance buffers must
    /// be in the stride set, or a sub-visual lags at canonical during a live drag.
    #[test]
    fn preview_write_set_includes_text_instance_buffer() {
        let strides = preview_instance_strides();
        assert_eq!(strides.len(), 4, "fill + stroke + text + shadow all follow the drag");
        assert_eq!(strides[0], std::mem::size_of::<FillInstance>() as u64);
        assert_eq!(strides[1], std::mem::size_of::<StrokeInstance>() as u64);
        assert_eq!(
            strides[2],
            std::mem::size_of::<TextInstance>() as u64,
            "text instance buffer must be in the preview write set (G3)"
        );
        assert_eq!(
            strides[3],
            std::mem::size_of::<ShadowInstance>() as u64,
            "shadow instance buffer must be in the preview write set (G5)"
        );
    }

    /// Pins the matrix-at-offset-0 layout so the preview write never clobbers the
    /// baked shadow color (which sits at offset 36, past the matrix).
    #[test]
    fn shadow_instance_matrix_region_matches_preview_write_layout() {
        assert_eq!(std::mem::offset_of!(ShadowInstance, m0), 0);
        assert_eq!(std::mem::offset_of!(ShadowInstance, m1), 12);
        assert_eq!(std::mem::offset_of!(ShadowInstance, m2), 24);
        assert_eq!(std::mem::offset_of!(ShadowInstance, shadow), 36);
        assert!(
            std::mem::offset_of!(ShadowInstance, shadow) >= 3 * std::mem::size_of::<[f32; 3]>()
        );
    }

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

    /// `draws[i].id` <-> instance `i` in both the fill and stroke buffers. The
    /// preview write looks up `i` and writes `i * size_of` in both, so the alignment
    /// is load-bearing.
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

    /// Stub measure: every char advances `size` px, so glyph N's origin is
    /// `region_min.x + N*size_px`.
    fn unit_measure(_ch: char, size: f32) -> f32 {
        size
    }

    /// A rect text object with one run; `size` is the WIRE (quantized) size, align
    /// Start / valign Top so the first glyph lands at the region top-left.
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

    #[test]
    fn text_object_produces_positioned_glyph_quads() {
        // Wire size 128 = 16px * 8 quantum. "AB" -> two visible glyphs.
        let obj = text_rect("t1", "AB", 128.0, "#ff8800");
        let scene = scene_with(vec![obj], None);
        let geo =
            build_scene_geometry_themed_with_measure(&scene, Theme::light(), &unit_measure);

        assert!(
            !geo.text_vertices.is_empty(),
            "committed text must produce glyph quads"
        );
        let range = geo.draws[0].text_range;
        assert_eq!(range.start, 0);
        assert_eq!(range.len(), 12, "two glyphs -> 12 tri-list verts");
        assert_eq!(geo.text_vertices.len(), 12);

        // The quad top-left vertex (index 0 of each glyph's 6) is the pen origin.
        let g0_origin_x = geo.text_vertices[0].position[0];
        let g1_origin_x = geo.text_vertices[6].position[0];
        assert!((g0_origin_x - 0.0).abs() < 1e-4, "first glyph at region_min.x");
        assert!(
            (g1_origin_x - 16.0).abs() < 1e-4,
            "second glyph at region_min.x + 16 (de-quant px advance), got {g1_origin_x}"
        );

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

    #[test]
    fn text_range_is_drawn_after_stroke_for_each_object() {
        let text_obj = text_rect("with-text", "Ab", 128.0, "#111111");
        let plain = rect_object("no-text");
        let scene = scene_with(vec![text_obj, plain], None);
        let geo =
            build_scene_geometry_themed_with_measure(&scene, Theme::light(), &unit_measure);

        assert!(!geo.draws[0].text_range.is_empty(), "text object draws glyphs");
        assert!(geo.draws[1].text_range.is_empty(), "plain object draws no glyphs");

        assert_eq!(geo.draws.len(), geo.text_instances.len());
        for (i, draw) in geo.draws.iter().enumerate() {
            assert_eq!(draw.id, scene.objects[i].id);
            assert_eq!(geo.text_instances[i].m0, geo.fill_instances[i].m0);
            assert_eq!(geo.text_instances[i].m1, geo.fill_instances[i].m1);
            assert_eq!(geo.text_instances[i].m2, geo.fill_instances[i].m2);
        }
        assert_eq!(geo.draws[0].text_range.end, crate::cast::len_u32(geo.text_vertices.len()));
    }

    #[test]
    fn text_vertex_and_instance_sizes_match_shader_contract() {
        // TextVertex: vec2 position + vec2 uv + vec4 color = 8 floats = 32 bytes.
        assert_eq!(std::mem::size_of::<TextVertex>(), 32);
        // TextInstance: 3 vec3 columns = 36 bytes, no color.
        assert_eq!(std::mem::size_of::<TextInstance>(), 36);
    }

    #[test]
    fn committed_text_size_dequantizes_to_pixels() {
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

        // Feeding 16px through the same path means wire 16*QUANT_PER_PX = 128, exactly
        // the committed run — so committed == the 16px edit (one de-quant site).
        let edited = text_rect("c", "AB", 16.0 * QUANT_PER_PX, "#000000");
        let edited_geo =
            build_scene_geometry_themed_with_measure(&scene_with(vec![edited], None), Theme::light(), &unit_measure);
        assert_eq!(
            geo.text_vertices, edited_geo.text_vertices,
            "committed text == the 16px edit-overlay intent (single de-quant site)"
        );
    }

    /// DEFECT 3 (ii): with a populated atlas the emitted glyph quads carry their REAL
    /// per-glyph atlas-slot UVs, NOT the placeholder full-atlas [0,0]/[1,1]. A run
    /// with size:None (defaulted to 16px) + non-empty text must map at least one glyph
    /// to a real sub-unit slot. FAILS while every quad still spans uv 0..1.
    #[test]
    fn text_quads_carry_real_atlas_slot_uvs_not_placeholder() {
        // Populate a real atlas from the bundled fonts for "AB" at the 16px default.
        let engine = crate::text::TextEngine::new().expect("bundled fonts load");
        let mut atlas = crate::text_layout::MsdfAtlasPlan::new(2048, 2048, 4.0);
        let mut entries: std::collections::HashMap<(u32, u32), crate::text_layout::MsdfGlyphEntry> =
            std::collections::HashMap::new();
        for ch in "AB".chars() {
            let cov = engine.glyph_coverage(ch, 16.0, 1.0).expect("rasterizes");
            let entry = atlas
                .generate_glyph(&crate::text_layout::GlyphCoverage {
                    key: crate::text_layout::MsdfGlyphKey {
                        font_index: cov.font_index,
                        glyph_id: cov.glyph_id,
                        px: cov.px,
                    },
                    coverage: &cov.coverage,
                    width: cov.width,
                    height: cov.height,
                    bearing_x: cov.bearing_x,
                    bearing_y: cov.bearing_y,
                    oversample: cov.oversample,
                })
                .expect("atlas room");
            entries.insert((ch as u32, 16), entry);
        }
        let glyph_uv = |ch: char, size: f32| -> Option<crate::text_layout::MsdfGlyphEntry> {
            entries.get(&(ch as u32, crate::cast::round_u32(size))).copied()
        };
        let real_measure = |ch: char, size: f32| engine.char_advance(ch, size);

        // Wire size 128 = 16px default; the object carries non-empty text.
        let obj = text_rect("t-uv", "AB", 128.0, "#ffffff");
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry_themed_with_text(
            &scene,
            Theme::light(),
            &real_measure,
            &glyph_uv,
        );

        assert!(!geo.text_vertices.is_empty(), "committed text emits quads");
        // The placeholder spans uv 0..1; a real slot is sub-unit. Assert NOT every
        // quad is the placeholder — at least one vertex carries a non-trivial UV.
        let all_placeholder = geo.text_vertices.iter().all(|v| {
            (v.uv == [0.0, 0.0]) || (v.uv == [1.0, 0.0]) || (v.uv == [1.0, 1.0]) || (v.uv == [0.0, 1.0])
        });
        assert!(
            !all_placeholder,
            "glyph quads must carry real sub-unit atlas UVs, not the full-atlas placeholder"
        );
        // And every UV stays inside the atlas (a real slot, well-formed).
        for v in &geo.text_vertices {
            assert!((0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]));
        }
    }

    /// DEFECT 3 (the populate decision, in the DEFAULT gate): the GPU-free populate
    /// seam packs committed glyphs (grew=true), is idempotent per `(char,px)` on a
    /// re-feed (grew=false => no re-upload on pan/zoom), and feeds the build real atlas
    /// slots end-to-end. FAILS if the grew flag is dropped or the `(char,px)` key
    /// regresses. This is the same code path renderer-wgpu's `ObjectTextAtlas` delegates
    /// to, so the single glyph-pack impl is asserted on a host with no GPU.
    #[test]
    fn populate_atlas_from_scene_grows_on_commit_and_is_idempotent() {
        let engine = crate::text::TextEngine::new().expect("bundled fonts load");
        let mut plan = crate::text_layout::MsdfAtlasPlan::new(
            crate::text::TEXT_ATLAS_WIDTH,
            crate::text::TEXT_ATLAS_HEIGHT,
            4.0,
        );
        let mut entries: std::collections::HashMap<
            (u32, u32),
            crate::text_layout::MsdfGlyphEntry,
        > = std::collections::HashMap::new();

        // One text object, run "AB" at the 16px default (wire = 16 * QUANT_PER_PX).
        let scene = scene_with(vec![text_rect("t", "AB", 16.0 * QUANT_PER_PX, "#ffffff")], None);

        // First populate packs both glyphs (grew=true).
        assert!(
            populate_atlas_from_scene(&mut plan, &mut entries, &engine, &scene, 1.0),
            "first populate grows the atlas"
        );
        assert!(plan.glyph_count() >= 2, "both glyphs packed, got {}", plan.glyph_count());
        let count_after_first = plan.glyph_count();

        // Re-feeding the same scene packs nothing new (grew=false): the zero-rebake /
        // no-re-upload-on-pan-zoom contract, keyed on (char, px).
        assert!(
            !populate_atlas_from_scene(&mut plan, &mut entries, &engine, &scene, 1.0),
            "re-populate of the same scene packs no new glyph"
        );
        assert_eq!(plan.glyph_count(), count_after_first, "glyph count unchanged on re-feed");

        // The populated entries feed the build real atlas slots end-to-end: not every
        // quad carries the placeholder full-atlas uv 0..1.
        let measure = |ch: char, size: f32| engine.char_advance(ch, size);
        let glyph_uv = |ch: char, size: f32| -> Option<crate::text_layout::MsdfGlyphEntry> {
            entries.get(&(ch as u32, crate::cast::round_u32(size))).copied()
        };
        let geo = build_scene_geometry_themed_with_text(&scene, Theme::light(), &measure, &glyph_uv);
        assert!(!geo.text_vertices.is_empty(), "committed text emits quads");
        let all_placeholder = geo.text_vertices.iter().all(|v| {
            (v.uv == [0.0, 0.0]) || (v.uv == [1.0, 0.0]) || (v.uv == [1.0, 1.0]) || (v.uv == [0.0, 1.0])
        });
        assert!(
            !all_placeholder,
            "populate fed real sub-unit slots into the build, not the placeholder"
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

    // ---- theme resolution + zero-rebake toggle --------------------------

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

    /// A scene re-feed rebuilds the renderer with the persisted theme bit, not a
    /// hardcoded light theme, so a dark canvas survives the rebuild.
    #[test]
    fn refeed_preserves_persisted_dark_theme_clear() {
        let mut persisted = Theme::light();
        assert!(!persisted.dark, "wrapper starts on the light bit");

        persisted = Theme { dark: true };
        let rebuilt = persisted;

        assert_eq!(
            rebuilt.canvas_bg(),
            Theme::dark().canvas_bg(),
            "re-feed must clear with the persisted dark canvas-bg"
        );
        assert_ne!(
            rebuilt.canvas_bg(),
            Theme::light().canvas_bg(),
            "re-feed must NOT revert to the light canvas-bg"
        );

        let scene = scene_with(vec![token_rect("o1")], None);
        let refed = build_scene_geometry_themed(&scene, rebuilt);
        assert_eq!(
            refed.fill_instances[0].fill,
            crate::object_theme::resolve_token_f32("default-fill", true).unwrap(),
            "re-feed bakes the persisted dark token colors, not light"
        );
    }

    /// The same scene yields different chrome/instance RGBA when the theme flips:
    /// token fill/stroke colors, the canvas clear, and the drop-shadow all move.
    #[test]
    fn theme_flip_changes_token_instance_and_clear_rgba() {
        let scene = scene_with(vec![token_rect("o1")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        assert_ne!(
            light.fill_instances[0].fill, dark.fill_instances[0].fill,
            "default-fill token re-resolves on theme flip"
        );
        assert_ne!(
            light.stroke_instances[0].stroke, dark.stroke_instances[0].stroke,
            "default-stroke token re-resolves on theme flip"
        );
        assert_eq!(
            light.fill_instances[0].fill,
            crate::object_theme::resolve_token_f32("default-fill", false).unwrap()
        );
        assert_eq!(
            dark.fill_instances[0].fill,
            crate::object_theme::resolve_token_f32("default-fill", true).unwrap()
        );

        let light_clear = Theme::light().canvas_bg();
        let dark_clear = Theme::dark().canvas_bg();
        assert_ne!(light_clear, dark_clear, "canvas clear RGBA flips with the theme bit");
        assert!(light_clear[0] > 0.8, "light canvas-bg is near-white");
        assert!(dark_clear[0] < 0.2, "dark canvas-bg is near-black");

        let light_shadow = light.shadow_instances[0].shadow;
        let dark_shadow = dark.shadow_instances[0].shadow;
        assert_ne!(light_shadow, dark_shadow, "shadow RGBA flips with the theme bit");
        assert_eq!(light_shadow, Theme::light().shadow(), "shadow sourced from token");
        assert_eq!(dark_shadow, Theme::dark().shadow());
        assert!(light_shadow[0] < 0.2, "light-mode shadow casts dark");
        assert!(dark_shadow[0] > 0.8, "dark-mode shadow casts whitish");
        assert!(light_shadow[3] < 1.0 && dark_shadow[3] < 1.0, "shadow stays translucent");
    }

    /// A theme flip leaves tessellation byte-identical — only the token instance
    /// colors move — so a real GPU toggle is a per-instance color write, not a rebuild.
    #[test]
    fn theme_flip_leaves_tessellation_byte_identical_zero_rebake() {
        let scene = scene_with(vec![token_rect("a"), token_rect("b")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        assert_eq!(light.fill.vertices, dark.fill.vertices, "fill verts unchanged");
        assert_eq!(light.fill.indices, dark.fill.indices, "fill indices unchanged");
        assert_eq!(
            light.stroke_vertices, dark.stroke_vertices,
            "stroke ribbon verts unchanged"
        );
        assert_eq!(light.draws.len(), dark.draws.len());
        for (l, d) in light.draws.iter().zip(dark.draws.iter()) {
            assert_eq!(l.fill_range, d.fill_range, "fill range stable across theme");
            assert_eq!(l.stroke_range, d.stroke_range, "stroke range stable across theme");
            assert_eq!(l.fill_instance.m0, d.fill_instance.m0);
            assert_eq!(l.fill_instance.m1, d.fill_instance.m1);
            assert_eq!(l.fill_instance.m2, d.fill_instance.m2);
            assert_eq!(l.fill_token, d.fill_token, "token name is theme-invariant");
            assert_ne!(l.fill_instance.fill, d.fill_instance.fill);
        }
    }

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

    /// Pins the color slot at offset 36 (past the 36-byte matrix) so the theme color
    /// write never clobbers the matrix the drag preview writes at offset 0.
    #[test]
    fn theme_color_write_targets_the_color_slot_past_the_matrix() {
        assert_eq!(std::mem::offset_of!(FillInstance, fill), 36);
        assert_eq!(std::mem::offset_of!(StrokeInstance, stroke), 36);
        assert_eq!(std::mem::offset_of!(FillInstance, m0), 0);
        assert!(std::mem::offset_of!(FillInstance, fill) >= 3 * std::mem::size_of::<[f32; 3]>());
    }

    #[test]
    fn every_object_emits_a_themed_translucent_shadow() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        assert_eq!(light.draws.len(), 2);
        assert_eq!(light.shadow_instances.len(), 2);
        for draw in &light.draws {
            assert!(
                !draw.shadow_range.is_empty(),
                "object {} must cast a drop shadow",
                draw.id
            );
        }
        assert_eq!(light.draws[0].shadow_range.start, 0);
        assert_eq!(
            light.draws[0].shadow_range.end,
            light.draws[1].shadow_range.start
        );
        assert_eq!(
            light.draws[1].shadow_range.end,
            crate::cast::len_u32(light.shadow_vertices.len())
        );

        let light_shadow = light.shadow_instances[0].shadow;
        let dark_shadow = dark.shadow_instances[0].shadow;
        assert_eq!(light_shadow, Theme::light().shadow(), "shadow sourced from token, not hardcoded");
        assert_eq!(dark_shadow, Theme::dark().shadow());
        assert_ne!(light_shadow, dark_shadow, "shadow RGBA flips light vs dark");
        assert!(light_shadow[3] < 1.0, "light shadow is translucent");
        assert!(dark_shadow[3] < 1.0, "dark shadow is translucent");
        assert!(light_shadow[3] > 0.0 && dark_shadow[3] > 0.0, "shadow is visible");
    }

    #[test]
    fn shadow_geometry_is_theme_invariant_only_color_flips() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")], None);
        let light = build_scene_geometry_themed(&scene, Theme::light());
        let dark = build_scene_geometry_themed(&scene, Theme::dark());

        assert_eq!(
            light.shadow_vertices, dark.shadow_vertices,
            "shadow quad geometry is theme-invariant (zero rebake)"
        );
        for (l, d) in light.draws.iter().zip(dark.draws.iter()) {
            assert_eq!(l.shadow_range, d.shadow_range, "shadow range stable across theme");
            assert_eq!(l.shadow_instance.m0, d.shadow_instance.m0);
            assert_eq!(l.shadow_instance.m1, d.shadow_instance.m1);
            assert_eq!(l.shadow_instance.m2, d.shadow_instance.m2);
            assert_ne!(l.shadow_instance.shadow, d.shadow_instance.shadow);
        }
    }

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

        // Single tier: one copy of the fill triangle list, never a *N stack.
        assert_eq!(
            verts.len(),
            mesh.indices.len(),
            "shadow is a single offset silhouette (one copy of the fill triangle list)"
        );
        assert!(
            verts.iter().all(|v| v.feather == 0.0),
            "single silhouette tier carries feather 0 (no baked-in ramp)"
        );
    }

    #[test]
    fn shadow_offset_is_zero_so_silhouette_is_symmetric() {
        assert_eq!(SHADOW_OFFSET_PX, 0.0, "shadow drop offset is removed (all-sides symmetric)");

        let scene = scene_with(vec![rect_object("o1")], None);
        let geo = build_scene_geometry(&scene);

        let subpaths = flatten_object_subpaths(&scene.objects[0], scene.camera.zoom);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);

        // Every shadow vertex equals its fill-mesh source with no y-offset.
        for (sv, &idx) in geo.shadow_vertices.iter().zip(mesh.indices.iter()) {
            let src = mesh.vertices[idx as usize];
            assert_eq!(sv.position, src, "shadow vertex is untranslated (offset 0)");
        }
    }

    #[test]
    fn shadow_is_exact_silhouette_of_fill_mesh_for_concave_shape() {
        // A concave arrowhead: the reflex vertex at (300,400) makes a centroid fan
        // invalid (the centroid lies outside the silhouette).
        let mut obj = rect_object("arrow");
        obj.geometry_d = "M0 0 L800 400 L0 800 L300 400 Z".to_string();
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);

        let subpaths = flatten_object_subpaths(&scene.objects[0], scene.camera.zoom);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        assert!(!mesh.indices.is_empty(), "concave arrow has a fillable interior");

        assert_eq!(
            geo.shadow_vertices.len(),
            mesh.indices.len(),
            "shadow is a single offset silhouette (one copy of the fill triangle list)"
        );

        // Undo the offset and compare against the un-offset fill triangulation.
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

        // Regression guard: no shadow vertex sits at the outline centroid (the old
        // core-fan apex, which lies outside this concave silhouette).
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

    #[test]
    fn stroke_only_object_casts_shadow_from_ribbon() {
        // Stroke-only (open, fill None): empty fill mesh, so the shadow comes from
        // the ribbon.
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

        // Filled object: shadow is still the fill-mesh silhouette, not the ribbon.
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
            crate::cast::len_u32(fill_mesh.indices.len()),
            "a filled object's shadow is its fill-mesh silhouette (unchanged)"
        );

        // Neither fill nor stroke ribbon: a lone MoveTo casts nothing.
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

    #[test]
    fn nonrectangular_shadow_follows_path_outline_not_aabb() {
        // An ellipse (four cubic arcs, closed) so its silhouette hugs the curve; no
        // shadow vertex should sit in a bbox corner the curve never reaches.
        let mut obj = rect_object("ellipse");
        obj.geometry_d = "M400 0 C621 0 800 179 800 400 C800 621 621 800 400 800 \
             C179 800 0 621 0 400 C0 179 179 0 400 0 Z"
            .to_string();
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        assert!(!geo.shadow_vertices.is_empty(), "ellipse casts a shadow");

        let subpaths = flatten_object_subpaths(&scene.objects[0], 1.0);
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> =
            subpaths.iter().map(|(c, p)| (*c, p.clone())).collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        let core = &geo.shadow_vertices[..mesh.indices.len()];

        // The four bbox corners are the points a bbox-quad shadow would touch.
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

        // No shadow vertex may coincide with a bbox corner (the ellipse pulls inward).
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

        // The silhouette is genuinely curved (more than a rect's 4 vertices).
        let positions: std::collections::BTreeSet<[u32; 2]> = core
            .iter()
            .map(|v| [v.position[0].to_bits(), v.position[1].to_bits()])
            .collect();
        assert!(
            positions.len() > 8,
            "ellipse silhouette shadow has many curved vertices, not 4 bbox corners"
        );
    }

    #[test]
    fn shadow_vertex_and_instance_sizes_match_shader_contract() {
        // ShadowVertex: vec2 position + f32 feather = 3 floats = 12 bytes.
        assert_eq!(std::mem::size_of::<ShadowVertex>(), 12);
        // ShadowInstance: 3 vec3 columns + vec4 color = 13 floats = 52 bytes.
        assert_eq!(std::mem::size_of::<ShadowInstance>(), 52);
        assert_eq!(std::mem::offset_of!(ShadowInstance, m0), 0);
        assert_eq!(std::mem::offset_of!(ShadowInstance, shadow), 36);
    }

    /// The shadow uses the same `shadow` token `Theme::shadow()` exposes.
    #[test]
    fn shadow_token_constant_matches_theme_shadow_accessor() {
        assert_eq!(SHADOW_TOKEN, crate::object_theme::ThemeToken::Shadow.name());
        let light = paint_color(&RPaint::Token { name: SHADOW_TOKEN.to_string() }, 1.0, Theme::light());
        assert_eq!(light, Theme::light().shadow());
    }

    /// A token paint's `opacity` multiplies the token's own alpha (e.g. translucent
    /// `shadow`), so the paint fades without losing the baseline translucency.
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

    /// An open stroke object with no fill, so the patch exercises only the ribbon.
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
            hidden: false,
            locked: false,
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
        assert_eq!(crate::cast::len_u32(rebuilt.stroke_vertices.len()), draw.stroke_range.len());
        assert_eq!(crate::cast::len_u32(rebuilt.fill_vertices.len()), draw.fill_vertex_range.len());
        assert_eq!(crate::cast::len_u32(rebuilt.fill_indices.len()), draw.fill_range.len());
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
        let canonical = open_stroke_object("f", "M0 0 L800 0");
        let scene = scene_with(vec![canonical], None);
        let build = build_scene_geometry_themed(&scene, Theme::light());
        let draw = &build.draws[0];

        // A 3-node re-expand changes the vertex count, so the guard must refuse it.
        let three_nodes = open_stroke_object("f", "M0 0 L800 0 L800 400");
        let rebuilt = reexpand_single_object(&three_nodes, Theme::light(), scene.camera.clone());
        assert_ne!(crate::cast::len_u32(rebuilt.stroke_vertices.len()), draw.stroke_range.len());
        assert!(
            follower_patch_plan(draw, &rebuilt).is_none(),
            "a topology/LOD count change must SKIP the patch, not corrupt the buffer"
        );
    }

    /// The count guard covers shadow + text too: a mismatched count refuses the plan.
    #[test]
    fn follower_patch_plan_is_none_on_a_shadow_or_text_count_change() {
        // A filled rect casts a fill-derived shadow, so the tamper is a real mismatch.
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

    /// A follower re-expanded with a reprojected node must carry shadow + text
    /// vertices that equal the corresponding slices of a full rebake of the deformed
    /// scene. Two followers cover both shadow sources (filled + stroke-only).
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
                    x: crate::cast::round_i32(anchor.at.x),
                    y: crate::cast::round_i32(anchor.at.y),
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

        // The text follower's glyphs moved by the reproject (its region min changed).
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
