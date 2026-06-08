// W3-G8/A drop-shadow composite: draw the blurred shadow mask under the fill.
//
// A fullscreen-triangle pass that samples the twice-blurred shadow mask and tints
// it by the theme `shadow` token color, src-over onto the visible surface. It is
// recorded FIRST in the visible object pass (right after the clear, before fill),
// so fill/stroke/text draw on top.
//
// The mask's ALPHA is the blurred silhouette COVERAGE. The silhouette was rendered
// with the theme `shadow` token's OWN translucent alpha, so the coverage ALREADY
// encodes the shadow strength (peak ~= the token alpha, feathered to 0 at the rim).
// The composite therefore takes the token's RGB from `tint` and uses the coverage
// DIRECTLY as the output alpha (the token alpha is applied exactly once, via the
// mask). Its rgb channel is discarded so no premultiplied dark-edge bleed reaches
// the surface. An empty mask -> zero coverage -> fully transparent output: the
// shadow simply disappears, never a wash.

struct CompositeParams {
  // The theme `shadow` token color (rgb + translucent a), resolved on the CPU
  // through the token path and refreshed on a theme flip.
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
  // Coverage already carries the token alpha (the silhouette was rendered with the
  // translucent shadow color), so use it DIRECTLY as the output alpha and take only
  // the token RGB from the tint. The pipeline uses straight (non-premultiplied)
  // ALPHA_BLENDING, so emit straight rgb with the coverage as alpha.
  let coverage = textureSample(mask_tex, mask_sampler, input.uv).a;
  return vec4<f32>(params.tint.rgb, coverage);
}
