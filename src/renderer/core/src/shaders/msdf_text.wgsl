// OB-3 MSDF text shader (OB3.R8 / OB3.R9 crispness at any zoom + D19 text runs).
//
// Glyphs are drawn as per-glyph quads positioned by the run layout (computed on
// the CPU: shaped runs -> glyph quads with atlas UVs). Each quad samples an MSDF
// (multi-channel signed distance field) atlas. The median of the three color
// channels reconstructs the true signed distance to the glyph outline, which
// stays sharp under arbitrary scaling — the whole point of MSDF over a single
// fontdue raster, which blurs when zoomed past its baked size (OB3.R9).
//
// screenPxRange-based AA (the canonical MSDF technique): the atlas bakes a fixed
// distance range in *texels*; we convert that to a screen-pixel range using the
// derivative of the texture coordinates, then smoothstep one screen pixel around
// the 0.5 threshold. This keeps the edge exactly one pixel soft regardless of
// camera zoom, so text neither aliases when zoomed in nor fattens when zoomed
// out.
//
// Camera + per-object projective transform mirror object_fill.wgsl so a text run
// inherits the same world placement as the object that owns it.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

@group(0) @binding(1)
var msdf_atlas: texture_2d<f32>;

@group(0) @binding(2)
var msdf_sampler: sampler;

struct TextUniform {
  // x: pxRange baked into the atlas (in atlas texels). y: atlas width in texels.
  // z: atlas height in texels. w: unused.
  atlas: vec4<f32>,
};

@group(0) @binding(3)
var<uniform> text_params: TextUniform;

struct VertexIn {
  // Per-glyph-quad corner, object-local CSS px (already laid out by the run).
  @location(0) position: vec2<f32>,
  // Atlas UV (0..1) for this corner.
  @location(1) uv: vec2<f32>,
  // Per-run text color (D19 run.color), inline.
  @location(2) color: vec4<f32>,
  // Instance-step per-object projective matrix columns.
  @location(3) m0: vec3<f32>,
  @location(4) m1: vec3<f32>,
  @location(5) m2: vec3<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) uv: vec2<f32>,
};

fn world_from_local(local: vec2<f32>, m0: vec3<f32>, m1: vec3<f32>, m2: vec3<f32>) -> vec2<f32> {
  let m = mat3x3<f32>(m0, m1, m2);
  let h = m * vec3<f32>(local, 1.0);
  let w = select(h.z, 1.0, abs(h.z) < 1e-6);
  return h.xy / w;
}

fn world_to_clip(world: vec2<f32>) -> vec2<f32> {
  let screen = world * view.camera.z + view.camera.xy;
  return vec2<f32>(
    (screen.x / view.viewport.x) * 2.0 - 1.0,
    1.0 - (screen.y / view.viewport.y) * 2.0
  );
}

fn median3(v: vec3<f32>) -> f32 {
  return max(min(v.r, v.g), min(max(v.r, v.g), v.b));
}

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
  let world = world_from_local(input.position, input.m0, input.m1, input.m2);
  var out: VertexOut;
  out.position = vec4<f32>(world_to_clip(world), 0.0, 1.0);
  out.color = input.color;
  out.uv = input.uv;
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let msd = textureSample(msdf_atlas, msdf_sampler, input.uv).rgb;
  // Reconstruct the signed distance (0.5 == on the outline).
  let sd = median3(msd);

  // screenPxRange: convert the atlas's baked texel range into screen pixels at
  // this fragment. unitRange is the pxRange expressed in UV units; its length in
  // screen space (via fwidth of uv) tells us how many screen pixels one unit of
  // signed distance spans here.
  let atlas_size = vec2<f32>(text_params.atlas.y, text_params.atlas.z);
  let unit_range = vec2<f32>(text_params.atlas.x) / atlas_size;
  let screen_tex_size = vec2<f32>(1.0) / fwidth(input.uv);
  let screen_px_range = max(0.5 * dot(unit_range, screen_tex_size), 1.0);

  let screen_dist = screen_px_range * (sd - 0.5);
  let coverage = clamp(screen_dist + 0.5, 0.0, 1.0);

  return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
