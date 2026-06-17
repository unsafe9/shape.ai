// Object text shader: per-glyph quads (laid out on the CPU) sampling the shared atlas.
//
// Two modes, branched per glyph on the inline `mode` flag:
//   mode 0 (SDF, world/canvas): the atlas holds a signed distance field; median3
//     reconstructs the distance and screenPxRange AA keeps the edge one screen pixel
//     soft at any zoom, staying sharp under arbitrary scaling where a raster blurs.
//   mode 1 (coverage, screen-space UI): the atlas holds a raw device-resolution
//     coverage raster; we sample it straight as the AA alpha — the browser-blit
//     behavior — so fixed small UI text is crisp instead of distance-field-blurred.
//
// Camera + per-object projective transform mirror object_fill.wgsl.

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
  // x: atlas pxRange (texels), y: atlas width (texels), z: atlas height (texels), w: unused.
  atlas: vec4<f32>,
};

@group(0) @binding(3)
var<uniform> text_params: TextUniform;

struct VertexIn {
  // Glyph-quad corner, object-local CSS px (laid out by the run).
  @location(0) position: vec2<f32>,
  @location(1) uv: vec2<f32>,
  // Per-run text color, inline.
  @location(2) color: vec4<f32>,
  // Atlas-sampling mode: 0 = SDF (world/canvas), 1 = raw coverage (screen-space UI).
  @location(3) mode: f32,
  // Instance-step projective matrix columns.
  @location(4) m0: vec3<f32>,
  @location(5) m1: vec3<f32>,
  @location(6) m2: vec3<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) @interpolate(flat) mode: f32,
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
  out.mode = input.mode;
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let sample = textureSample(msdf_atlas, msdf_sampler, input.uv);
  // Derivatives must be evaluated in UNIFORM control flow (WGSL/Tint rejects them
  // inside the per-vertex `mode` branch below, which blanks the canvas). Compute the
  // screen-texel size for every fragment here at the top; the coverage branch ignores
  // it, the SDF branch uses it.
  let screen_tex_size = vec2<f32>(1.0) / fwidth(input.uv);

  var coverage: f32;
  if (input.mode > 0.5) {
    // Coverage mode (screen-space UI): the atlas holds raw alpha at device
    // resolution, so sample it straight — no distance math — and it stays crisp.
    coverage = sample.a;
  } else {
    // SDF mode (world/canvas): reconstruct the distance and screenPxRange-AA it.
    let sd = median3(sample.rgb);
    let atlas_size = vec2<f32>(text_params.atlas.y, text_params.atlas.z);
    let unit_range = vec2<f32>(text_params.atlas.x) / atlas_size;
    let screen_px_range = max(0.5 * dot(unit_range, screen_tex_size), 1.0);
    let screen_dist = screen_px_range * (sd - 0.5);
    coverage = clamp(screen_dist + 0.5, 0.0, 1.0);
  }

  return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
