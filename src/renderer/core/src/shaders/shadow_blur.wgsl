// W3-G8/A separable Gaussian blur for the offscreen drop-shadow mask.
//
// One fullscreen-triangle pass that blurs the bound source texture along a single
// axis (`direction`). Run twice per frame: horizontal (mask -> ping) then vertical
// (ping -> pong). The taps are a normalized one-sided Gaussian kernel (center +
// positive side) computed on the CPU (`shadow_blur.rs::gaussian_kernel`) and read
// from `params.weights`; the negative side is the mirror, so each non-center tap is
// applied symmetrically. Sampling is bilinear with ClampToEdge, so the silhouette
// feathers uniformly without wrapping.
//
// MAX_RADIUS must match `SHADOW_BLUR_MAX_RADIUS` in `shadow_blur.rs` (the weights
// uniform is sized `MAX_RADIUS + 1`). The active tap count rides in `params.x`.

const MAX_RADIUS: i32 = 12;

struct BlurParams {
  // Sample step axis in TEXEL units: (1,0) horizontal, (0,1) vertical.
  direction: vec2<f32>,
  // (1/width, 1/height): one device pixel in UV space.
  texel: vec2<f32>,
  // x = active one-sided tap count (radius); yzw unused.
  params: vec4<f32>,
  // One-sided + center Gaussian weights, one per vec4 slot (.x carries the value).
  // weights[0] = center tap; weights[k] = tap k pixels out (mirrored both sides).
  weights: array<vec4<f32>, 13>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: BlurParams;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

// Fullscreen triangle: three clip-space verts that cover the viewport, with UVs in
// [0,1]. No vertex buffer — positions/UVs are derived from the vertex index.
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
  // Center tap.
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
