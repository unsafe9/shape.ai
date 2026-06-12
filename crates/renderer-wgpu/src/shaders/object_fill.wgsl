// Object fill shader: projective transform + inline solid fill + analytic AA.
//
// Per-object transform is a 3x3 projective matrix carried as three instance-step
// vec3 columns; the .z carries the projective term, so a perspective divide is
// required — which is why M cannot be folded into the affine camera. Object-local
// positions are CSS px (i32 geometry at 8 units/px, converted /8 on the CPU).
// Fill color is inline per-instance; gradient/image fills resolve to this slot.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

// `edge` is 1 at a boundary vertex and 0 at interior vertices; interpolated, it
// gives a cheap analytic coverage term without a full SDF. The CPU fills it from
// the mesh topology; interior fans get edge = 0.
struct VertexIn {
  @location(0) position: vec2<f32>,
  @location(1) edge: f32,
  // Instance-step matrix columns (mat3x3 attributes are awkward across backends,
  // so pass three vec3 columns and rebuild in the VS) + inline fill color.
  @location(2) m0: vec3<f32>,
  @location(3) m1: vec3<f32>,
  @location(4) m2: vec3<f32>,
  @location(5) fill: vec4<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) fill: vec4<f32>,
  @location(1) edge: f32,
};

fn world_from_local(local: vec2<f32>, m0: vec3<f32>, m1: vec3<f32>, m2: vec3<f32>) -> vec2<f32> {
  let m = mat3x3<f32>(m0, m1, m2);
  let h = m * vec3<f32>(local, 1.0);
  // Projective divide. Guard the degenerate w ~= 0 so a malformed matrix yields a
  // finite off-screen point rather than a NaN that poisons the primitive.
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

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
  let world = world_from_local(input.position, input.m0, input.m1, input.m2);
  var out: VertexOut;
  out.position = vec4<f32>(world_to_clip(world), 0.0, 1.0);
  out.fill = input.fill;
  out.edge = input.edge;
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  // Analytic AA: `edge` rises 0 (interior) -> 1 (boundary); fwidth gives the
  // screen-space rate so the soft band is one pixel wide at any zoom. An all-zero-
  // edge mesh degrades to a fully opaque fill. We keep this over MSAA so edges stay
  // crisp without a multisampled attachment and strokes/text share the treatment.
  let aa = fwidth(input.edge);
  let coverage = 1.0 - smoothstep(1.0 - aa, 1.0, input.edge);
  return vec4<f32>(input.fill.rgb, input.fill.a * coverage);
}
