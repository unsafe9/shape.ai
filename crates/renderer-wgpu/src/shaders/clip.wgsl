// Clip region shader: nested stencil clipping. An object with clip:true masks its
// descendants to its arbitrary tessellated region (a scissor rect cannot express a
// post-transform path), so this stencil-WRITE pass rasterizes the clipper's filled
// region (the same tessellation object_fill.wgsl draws) writing only the stencil.
//
// The binding pipeline sets stencil front/back { compare: Equal, pass_op:
// IncrementClamp, fail/depth_fail: Keep } and color write_mask NONE, rendered with
// stencil_reference = parent_depth — so the increment lands only inside the parent
// clip, marking the intersection. Descendants draw with compare = Equal + Keep, so
// fragments outside fail; on leaving the subtree a matching DecrementClamp restores
// the parent depth.
//
// Camera + projective transform are identical to object_fill.wgsl so the clip
// rasterizes to the same pixels as the object's fill.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

struct VertexIn {
  @location(0) position: vec2<f32>,
  @location(1) m0: vec3<f32>,
  @location(2) m1: vec3<f32>,
  @location(3) m2: vec3<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
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
  return out;
}

// Stencil-only pass: the color attachment is masked off, so the returned value is
// never written; the fragment still runs so the stencil op covers the region.
@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
