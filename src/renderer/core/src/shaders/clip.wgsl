// OB-3 clip region shader (OB3.R7 nested stencil clipping).
//
// An object with clip:true masks its descendants to its own filled region. We
// implement this with the stencil buffer rather than a rectangular scissor,
// because an object's clip region is its arbitrary tessellated path (after the
// projective transform), not an axis-aligned rect.
//
// This shader is the *stencil-write* pass: it rasterizes the clipper's filled
// region (the same tessellation object_fill.wgsl draws) but writes no color —
// only the stencil value. The render pipeline that binds this shader sets:
//
//   depth_stencil.stencil.front/back = {
//     compare:        Equal,           // only write where parent clip already holds
//     pass_op:        IncrementClamp,  // nest: child region = parent + 1
//     fail_op / depth_fail_op: Keep,
//   }
//   color target write_mask = NONE     // stencil-only, no color
//
// and renders with stencil_reference = parent_depth. Drawing the clipper this
// way increments the stencil only inside the parent's already-clipped area, so
// the new reference value marks exactly the intersection (parent region AND this
// object's region) — that is how nested clips intersect (OB3.R7).
//
// Children then draw with their normal pipelines but with stencil compare =
// Equal against their clip depth and pass_op = Keep, so fragments outside the
// accumulated region fail the stencil test and are discarded. On leaving the
// clip subtree the controller issues a matching DecrementClamp pass (or restores
// via a saved reference) so sibling subtrees see the correct parent depth.
//
// Camera + projective transform are identical to object_fill.wgsl: the clip
// region must rasterize to the exact same pixels as the object's fill.

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

// Stencil-only pass: the color attachment is masked off by the pipeline, so the
// returned value is never written. We still emit a fragment so rasterization
// (and therefore the stencil op) runs over the clipper's region.
@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
