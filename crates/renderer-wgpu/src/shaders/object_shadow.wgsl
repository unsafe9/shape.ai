// Object drop-shadow shader: the object's own fill silhouette, offset on the CPU,
// drawn BENEATH the fill. The shadow color is the theme `shadow` token carried
// inline per-instance, so a theme flip is a color refresh, not a re-tessellation.
// Shares the camera + projective matrix convention with `object_fill.wgsl`.
// `feather` is a per-vertex 0..1 term (uniformly 0 for the flat silhouette today,
// so the FS falloff is 1); the slot stays for a future soft-blur pass.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

struct VertexIn {
  @location(0) position: vec2<f32>,
  // Blur falloff: 0 at the core, 1 at the soft outer edge.
  @location(1) feather: f32,
  // Instance-step projective matrix columns + inline (translucent) shadow color.
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
  // Soft blur falloff: full alpha at the core (feather=0) to zero at the edge
  // (feather=1). The squared falloff cheaply approximates a Gaussian tail.
  let t = clamp(input.feather, 0.0, 1.0);
  let falloff = (1.0 - t) * (1.0 - t);
  return vec4<f32>(input.shadow.rgb, input.shadow.a * falloff);
}
