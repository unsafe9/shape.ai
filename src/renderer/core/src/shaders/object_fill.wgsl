// OB-3 object fill shader (OB3.R3 + D7 projective transform + OB3.R8 analytic AA).
//
// Pipeline contract (compiled by wgpu at the OB-4 cutover; structurally
// validated only here — there is no device in the CPU test environment):
//
//   - Camera lives in @group(0) and mirrors the legacy affine convention used by
//     the live pipeline: `camera = vec4(translate.x, translate.y, zoom, _)` and
//     `viewport = vec4(px_w, px_h, _, _)`. World pixels map to clip space the
//     same way the legacy `fs_main` path does, so the two pipelines share a
//     coordinate frame during the cutover.
//   - Per-object transform (D1/D4) is a 3x3 *projective* matrix carried as three
//     instance-step vec3 columns. Object-local vertex positions are in CSS px
//     (the i32 geometry, quantized at 8 units/px, is converted to px by /8 on the
//     CPU before upload). world = M * vec3(pos, 1); the .z carries the projective
//     term, so a perspective divide (world.xy / world.z) is required — this is
//     why we cannot fold M into the affine camera.
//   - Fill color (D7) is inline per-instance (solid paint). Gradient/image fills
//     resolve to this same color slot on the CPU for the first cutover; richer
//     paints get their own bind group later without touching this VS.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

// Per-vertex: object-local position (CSS px) and a signed edge-distance helper.
// `edge` is a per-vertex scalar that is +1 at the triangle's silhouette edge
// vertex and 0 at interior vertices; interpolated, |edge| approaches 0 at the
// shape boundary, giving us a cheap analytic coverage term without a full SDF.
// Tessellation (lyon) fills this on the CPU; interior fans get edge = 0.
struct VertexIn {
  @location(0) position: vec2<f32>,
  @location(1) edge: f32,
  // Instance-step: three columns of the per-object 3x3 projective matrix and the
  // inline fill color. mat3x3 as a vertex attribute is awkward across backends,
  // so we pass three vec3 columns and rebuild the matrix in the VS.
  @location(2) m0: vec3<f32>,
  @location(3) m1: vec3<f32>,
  @location(4) m2: vec3<f32>,
  @location(5) fill: vec4<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) fill: vec4<f32>,
  // Signed edge coverage helper carried to the FS for analytic AA.
  @location(1) edge: f32,
};

fn world_from_local(local: vec2<f32>, m0: vec3<f32>, m1: vec3<f32>, m2: vec3<f32>) -> vec2<f32> {
  // Column-major reconstruction: M = [m0 | m1 | m2].
  let m = mat3x3<f32>(m0, m1, m2);
  let h = m * vec3<f32>(local, 1.0);
  // D4 projective divide. Guard the degenerate w ~= 0 so a malformed matrix
  // produces a finite (off-screen) point rather than a NaN that poisons the
  // whole primitive.
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
  // OB3.R8 analytic anti-aliasing. `edge` is a signed distance proxy that is ~0
  // at the silhouette boundary and grows toward the interior; fwidth gives the
  // per-pixel screen-space rate of change so the soft band is exactly one pixel
  // wide at any zoom (the projective divide already varies edge non-linearly
  // across the primitive, which fwidth tracks for free).
  //
  // MSAA is the simpler alternative: enabling a multisampled color target and
  // dropping this coverage term would hand antialiasing to fixed-function
  // hardware. We keep the analytic path so object edges stay crisp without
  // paying for a multisampled attachment, and so dashed strokes/text can share
  // the same distance-based treatment.
  let aa = fwidth(input.edge);
  let coverage = smoothstep(0.0, aa, input.edge);
  return vec4<f32>(input.fill.rgb, input.fill.a * coverage);
}
