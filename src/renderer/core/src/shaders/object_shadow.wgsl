// OB-3 object drop-shadow shader (RB3 #11 + D2 + D7 projective transform).
//
// A soft macOS-style drop shadow drawn BENEATH every object's fill. The shadow is
// a feathered quad covering the object's region bbox, expanded by a blur radius
// and offset by the drop-shadow offset on the CPU (see `object_pipeline.rs`). The
// shadow COLOR is the theme `shadow` token (RB1) carried inline per-instance, so a
// theme flip is a per-instance color refresh, never a re-tessellation.
//
// Pipeline contract (compiled by wgpu at the OB-4 cutover; structurally validated
// only here — there is no device in the CPU test environment). Shares the camera
// uniform + projective per-object matrix convention with `object_fill.wgsl`.
//
//   - `feather` is a per-vertex 0..1 term: 0 at the inner (solid) edge of the
//     shadow's core, 1 at the outer soft edge. The FS turns it into a smooth blur
//     falloff, approximating a Gaussian drop shadow without a separate blur pass.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

struct VertexIn {
  @location(0) position: vec2<f32>,
  // Per-vertex blur falloff: 0 at the shadow core, 1 at the soft outer edge.
  @location(1) feather: f32,
  // Instance-step: three columns of the per-object 3x3 projective matrix and the
  // inline shadow color (the theme `shadow` token, translucent).
  @location(2) m0: vec3<f32>,
  @location(3) m1: vec3<f32>,
  @location(4) m2: vec3<f32>,
  @location(5) shadow: vec4<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) shadow: vec4<f32>,
  @location(1) feather: f32,
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

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
  let world = world_from_local(input.position, input.m0, input.m1, input.m2);
  var out: VertexOut;
  out.position = vec4<f32>(world_to_clip(world), 0.0, 1.0);
  out.shadow = input.shadow;
  out.feather = input.feather;
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  // Soft blur falloff: full shadow alpha at the core (feather=0), fading to zero
  // at the outer edge (feather=1). A squared falloff approximates the Gaussian
  // tail of a real blur cheaply.
  let t = clamp(input.feather, 0.0, 1.0);
  let falloff = (1.0 - t) * (1.0 - t);
  return vec4<f32>(input.shadow.rgb, input.shadow.a * falloff);
}
