//! WGSL sources for the object render pipeline. WGSL compiles at RUNTIME on a
//! live device, so these are only structurally validated here; real compilation
//! happens under the `wgpu-probe` feature.
//!
//! Shared coordinate convention (mirrors the legacy affine camera):
//!   * `view.camera = vec4(translate.x, translate.y, zoom, _)`
//!   * `view.viewport = vec4(px_w, px_h, _, _)`
//! Every object carries a 3x3 projective matrix passed as three instance-step
//! `vec3` columns; the vertex stages compute `world = M * vec3(local, 1)` then
//! perspective-divide before mapping world px to clip space — which is why the
//! object transform cannot be folded into the affine camera uniform.
//!
//! Object-local geometry is the quantized i32 path (8 units/px) converted to
//! `f32` px on the CPU before upload; the shaders see plain px.

pub const OBJECT_FILL_WGSL: &str = include_str!("shaders/object_fill.wgsl");

pub const OBJECT_SHADOW_WGSL: &str = include_str!("shaders/object_shadow.wgsl");

pub const OBJECT_STROKE_WGSL: &str = include_str!("shaders/object_stroke.wgsl");

pub const MSDF_TEXT_WGSL: &str = include_str!("shaders/msdf_text.wgsl");

pub const CLIP_WGSL: &str = include_str!("shaders/clip.wgsl");

pub const SHADOW_BLUR_WGSL: &str = include_str!("shaders/shadow_blur.wgsl");

pub const SHADOW_COMPOSITE_WGSL: &str = include_str!("shaders/shadow_composite.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_pipeline_shaders_are_present() {
        for (name, src) in [
            ("object_fill", OBJECT_FILL_WGSL),
            ("object_shadow", OBJECT_SHADOW_WGSL),
            ("object_stroke", OBJECT_STROKE_WGSL),
            ("msdf_text", MSDF_TEXT_WGSL),
            ("clip", CLIP_WGSL),
            ("shadow_blur", SHADOW_BLUR_WGSL),
            ("shadow_composite", SHADOW_COMPOSITE_WGSL),
        ] {
            assert!(!src.trim().is_empty(), "{name} WGSL source is empty");
            assert!(
                src.contains("@vertex") && src.contains("@fragment"),
                "{name} WGSL is missing a vertex or fragment stage"
            );
        }
    }
}
