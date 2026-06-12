// Separable Gaussian blur for the offscreen drop-shadow mask: one fullscreen-
// triangle pass blurring the source along `direction`, run twice (H: mask -> ping,
// V: ping -> pong). Taps are a normalized one-sided Gaussian kernel (CPU-computed)
// read from `params.weights`; the negative side is the mirror. Bilinear ClampToEdge.
//
// MAX_RADIUS must match `SHADOW_BLUR_MAX_RADIUS` in `shadow_blur.rs` (the weights
// uniform is sized `MAX_RADIUS + 1`).

const MAX_RADIUS: i32 = 12;

struct BlurParams {
  // Sample step axis, TEXEL units: (1,0) H, (0,1) V.
  direction: vec2<f32>,
  // (1/width, 1/height): one device pixel in UV space.
  texel: vec2<f32>,
  // x = active one-sided tap count; yzw unused.
  params: vec4<f32>,
  // One-sided + center Gaussian weights, one per vec4 slot (.x carries the value).
  // weights[0] = center; weights[k] = tap k px out (mirrored both sides).
  weights: array<vec4<f32>, 13>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: BlurParams;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

// Fullscreen triangle covering the viewport, UVs derived from the vertex index
// (no vertex buffer).
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOut {
  var out: VertexOut;
  let x = f32((vid << 1u) & 2u);
  let y = f32(vid & 2u);
  out.uv = vec2<f32>(x, y);
  out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let step = params.direction * params.texel;
  let radius = i32(params.params.x);
  var acc: vec4<f32> = textureSample(src_tex, src_sampler, input.uv) * params.weights[0].x;
  // Symmetric tails: each one-sided weight applied at +k and -k.
  for (var k: i32 = 1; k <= MAX_RADIUS; k = k + 1) {
    if (k > radius) {
      break;
    }
    let w = params.weights[k].x;
    let offset = step * f32(k);
    acc = acc + textureSample(src_tex, src_sampler, input.uv + offset) * w;
    acc = acc + textureSample(src_tex, src_sampler, input.uv - offset) * w;
  }
  return acc;
}
