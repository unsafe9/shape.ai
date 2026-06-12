// Drop-shadow composite: a fullscreen-triangle pass sampling the twice-blurred
// shadow mask and tinting it by the theme `shadow` token, src-over onto the
// surface. Recorded FIRST in the visible object pass (right after the clear) so
// fill/stroke/text draw on top.
//
// The mask ALPHA is the blurred silhouette COVERAGE, already encoding the shadow
// strength (the silhouette was rendered with the token's own translucent alpha), so
// the composite takes only the token RGB from `tint` and uses coverage DIRECTLY as
// the output alpha. An empty mask -> transparent output, never a wash.

struct CompositeParams {
  // The theme `shadow` token color, resolved on the CPU and refreshed on a flip.
  tint: vec4<f32>,
};

@group(0) @binding(0) var mask_tex: texture_2d<f32>;
@group(0) @binding(1) var mask_sampler: sampler;
@group(0) @binding(2) var<uniform> params: CompositeParams;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

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
  // Coverage already carries the token alpha, so use it directly as the output
  // alpha with the token RGB (straight, non-premultiplied ALPHA_BLENDING).
  let coverage = textureSample(mask_tex, mask_sampler, input.uv).a;
  return vec4<f32>(params.tint.rgb, coverage);
}
