// OB-3 object stroke shader (OB3.R2 stroke ribbon + OB3.R8 analytic AA).
//
// The stroke is uploaded as a triangle ribbon along the path. Each ribbon vertex
// carries the on-path position plus the unit normal at that point, a `side` flag
// (+1 / -1) picking which bank of the ribbon it belongs to, and the per-node
// half-extent width — so variable-width strokes (D2 stroke.width can taper per
// node) are expressed entirely in vertex data. The VS offsets each vertex along
// its normal by `side * width * 0.5` to give the ribbon its thickness; the FS
// paints the stroke color, applies dashing by discarding gaps, and softens both
// the long ribbon edges and the dash ends analytically.
//
// Camera + projective transform mirror object_fill.wgsl exactly so fills and
// strokes of the same object land in the same coordinate frame. Width is offset
// in object-local space *before* the projective divide, so a sheared/perspective
// transform thickens the stroke consistently with the body it outlines.

struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

struct VertexIn {
  // On-path position in object-local CSS px.
  @location(0) position: vec2<f32>,
  // Unit normal at this path point (object-local).
  @location(1) normal: vec2<f32>,
  // +1 / -1: which bank of the ribbon this vertex offsets toward.
  @location(2) side: f32,
  // Per-node stroke width (full width in px); half is applied along the normal.
  @location(3) width: f32,
  // Arc length from the start of the (sub)path to this point, in px. Drives the
  // dash pattern; monotonically increasing along the ribbon.
  @location(4) distance_along: f32,
  // Instance-step per-object projective matrix columns + stroke paint.
  @location(5) m0: vec3<f32>,
  @location(6) m1: vec3<f32>,
  @location(7) m2: vec3<f32>,
  @location(8) stroke: vec4<f32>,
};

struct StrokeUniform {
  // x: dash on-length (px), y: dash period (on + off) (px), z: stroke opacity,
  // w: 0 => no dash (solid), 1 => dashed.
  dash: vec4<f32>,
};

@group(0) @binding(1)
var<uniform> stroke_params: StrokeUniform;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) stroke: vec4<f32>,
  @location(1) distance_along: f32,
  // Signed across-ribbon coordinate in px: -half..+half. |edge_pos| near the
  // half-width is the ribbon silhouette, used for analytic AA along the banks.
  @location(2) edge_pos: f32,
  @location(3) half_width: f32,
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
  let half = input.width * 0.5;
  // OB3.R2: expand the ribbon by offsetting along the per-vertex normal.
  let local = input.position + input.normal * (input.side * half);
  let world = world_from_local(local, input.m0, input.m1, input.m2);

  var out: VertexOut;
  out.position = vec4<f32>(world_to_clip(world), 0.0, 1.0);
  out.stroke = input.stroke;
  out.distance_along = input.distance_along;
  out.edge_pos = input.side * half;
  out.half_width = half;
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  // Dash gating (D2 stroke.dash). With a period p and on-length on, a fragment is
  // painted when its arc-length position falls inside the on-segment. We soften
  // the dash boundary by one pixel of arc length so dash ends do not shimmer.
  var dash_coverage = 1.0;
  if (stroke_params.dash.w > 0.5) {
    let period = max(stroke_params.dash.y, 1e-4);
    let phase = fract(input.distance_along / period) * period;
    let on_len = stroke_params.dash.x;
    let daa = fwidth(input.distance_along);
    // Rising edge at phase=0, falling edge at phase=on_len.
    let rise = smoothstep(0.0, daa, phase);
    let fall = 1.0 - smoothstep(on_len - daa, on_len, phase);
    dash_coverage = rise * fall;
    if (dash_coverage <= 0.0) {
      discard;
    }
  }

  // OB3.R8 analytic AA across the ribbon banks: coverage falls off in the last
  // pixel before |edge_pos| reaches the half width.
  let eaa = fwidth(input.edge_pos);
  let edge_coverage = 1.0 - smoothstep(input.half_width - eaa, input.half_width, abs(input.edge_pos));

  let alpha = input.stroke.a * stroke_params.dash.z * dash_coverage * edge_coverage;
  return vec4<f32>(input.stroke.rgb, alpha);
}
