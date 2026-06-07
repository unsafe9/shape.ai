//! WGSL sources for the OB-3 object render pipeline.
//!
//! These shaders implement the projective, path-based object model (D1/D2/D4/D7)
//! and are *additive*: they do not touch the legacy `RenderGroup`/`RenderCard`/
//! `RenderEdge` draw path or its `ViewUniform`. They are compiled by `wgpu` at
//! the OB-4 cutover. There is no GPU device in the CPU test environment, so they
//! are only structurally validated here (the [`tests`] module asserts the
//! sources are present and non-empty); real shader-module compilation happens
//! under the `wgpu-probe` feature on a device.
//!
//! All four shaders share one coordinate convention, deliberately mirroring the
//! legacy pipeline's affine camera so the two can coexist during the cutover:
//!
//!   * `view.camera = vec4(translate.x, translate.y, zoom, _)`
//!   * `view.viewport = vec4(px_w, px_h, _, _)`
//!
//! On top of that affine camera, every object carries a 3x3 *projective* matrix
//! (D1/D4) passed as three instance-step `vec3` columns. The vertex stages
//! compute `world = M * vec3(local, 1)` and perform the perspective divide
//! `world.xy / world.z` before mapping world px to clip space — which is why the
//! object transform cannot be folded into the affine camera uniform.
//!
//! Object-local geometry is the quantized i32 path (8 units/px) converted to
//! `f32` px on the CPU (divide by 8) before upload; the shaders see plain px.
//!
//! # OB3.R7 nested clip plan (stencil)
//!
//! [`CLIP_WGSL`] is the *stencil-write* pass for `clip:true` objects. A scissor
//! rect cannot express an arbitrary post-transform path, so clipping uses the
//! stencil buffer:
//!
//! 1. Before drawing a clipper's subtree, draw the clipper's tessellated region
//!    with [`CLIP_WGSL`] using a stencil pipeline configured `compare = Equal`
//!    against the current clip depth and `pass_op = IncrementClamp`, with the
//!    color write mask set to `NONE`. This raises the stencil value by one only
//!    inside the area already permitted by the parent clip, so the new reference
//!    value marks the *intersection* of this object's region with its ancestors'
//!    — that is how nested clips compose.
//! 2. Descendants render with their normal pipelines (`object_fill` /
//!    `object_stroke` / `msdf_text`) but with `stencil.compare = Equal` against
//!    the accumulated clip depth and `pass_op = Keep`, so fragments outside the
//!    region fail the stencil test and are discarded.
//! 3. On leaving the subtree, restore the parent depth (a matching
//!    `DecrementClamp` pass over the same region, or a saved stencil reference)
//!    so sibling subtrees see the correct parent clip.
//!
//! The clipper region rasterizes through the same camera + projective transform
//! as [`OBJECT_FILL_WGSL`], guaranteeing the mask covers exactly the object's
//! filled pixels.

/// Object fill VS/FS: projective transform + inline solid fill with analytic,
/// `fwidth`-based edge anti-aliasing (OB3.R3 / D7 / OB3.R8).
pub const OBJECT_FILL_WGSL: &str = include_str!("shaders/object_fill.wgsl");

/// Object stroke VS/FS: per-vertex ribbon expansion along the normal with
/// per-node width, dash gating, and analytic ribbon-edge AA (OB3.R2 / OB3.R8).
pub const OBJECT_STROKE_WGSL: &str = include_str!("shaders/object_stroke.wgsl");

/// MSDF text VS/FS: median-of-3 signed distance with `screenPxRange` AA for
/// zoom-stable crispness; per-glyph quads from the run layout (OB3.R8 / OB3.R9 /
/// D19).
pub const MSDF_TEXT_WGSL: &str = include_str!("shaders/msdf_text.wgsl");

/// Clip region stencil-write VS/FS used to implement nested clipping (OB3.R7).
/// See the module docs for the full stencil compose plan.
pub const CLIP_WGSL: &str = include_str!("shaders/clip.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    /// No device here to compile WGSL against, so the structural guarantee is
    /// just that each source is embedded and non-empty. Device-side compilation
    /// is exercised under the `wgpu-probe` feature at the OB-4 cutover.
    #[test]
    fn object_pipeline_shaders_are_present() {
        for (name, src) in [
            ("object_fill", OBJECT_FILL_WGSL),
            ("object_stroke", OBJECT_STROKE_WGSL),
            ("msdf_text", MSDF_TEXT_WGSL),
            ("clip", CLIP_WGSL),
        ] {
            assert!(!src.trim().is_empty(), "{name} WGSL source is empty");
            assert!(
                src.contains("@vertex") && src.contains("@fragment"),
                "{name} WGSL is missing a vertex or fragment stage"
            );
        }
    }
}
