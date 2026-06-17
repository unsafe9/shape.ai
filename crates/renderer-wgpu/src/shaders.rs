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

    const ALL: [(&str, &str); 7] = [
        ("object_fill", OBJECT_FILL_WGSL),
        ("object_shadow", OBJECT_SHADOW_WGSL),
        ("object_stroke", OBJECT_STROKE_WGSL),
        ("msdf_text", MSDF_TEXT_WGSL),
        ("clip", CLIP_WGSL),
        ("shadow_blur", SHADOW_BLUR_WGSL),
        ("shadow_composite", SHADOW_COMPOSITE_WGSL),
    ];

    #[test]
    fn object_pipeline_shaders_are_present() {
        for (name, src) in ALL {
            assert!(!src.trim().is_empty(), "{name} WGSL source is empty");
            assert!(
                src.contains("@vertex") && src.contains("@fragment"),
                "{name} WGSL is missing a vertex or fragment stage"
            );
        }
    }

    /// Parse + validate every embedded WGSL on the host (catches parse/type errors that
    /// a green wasm build never sees, since `cargo test --workspace` excludes this
    /// crate). naga is lenient on uniformity — the browser's Tint is the real gate for
    /// that — so the derivative-uniformity invariant is pinned separately below.
    #[test]
    fn object_pipeline_shaders_pass_naga_validation() {
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        for (name, src) in ALL {
            let module = naga::front::wgsl::parse_str(src)
                .unwrap_or_else(|e| panic!("{name} WGSL did not parse: {e:?}"));
            validator
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name} WGSL failed naga validation: {e:?}"));
        }
    }


    /// FALSIFIABLE: a derivative builtin (`fwidth`/`dpdx`/`dpdy`) reached only through
    /// control flow that branches on the per-fragment input (`input.…`) is non-uniform,
    /// which Tint rejects as a shader-creation error — it blanks the whole canvas (it
    /// bit us once: `fwidth` inside the `if (input.mode > 0.5)` branch of `msdf_text`).
    /// naga does NOT catch this and `cargo test --workspace` excludes this crate, so we
    /// pin it here. A derivative inside a branch on a UNIFORM (e.g. `stroke_params.…`)
    /// is fine — that is why this keys on `input.`, not on nesting depth.
    #[test]
    fn derivatives_not_gated_by_non_uniform_input() {
        for (name, src) in ALL {
            // Strip line comments so a `//` mention can't skew brace/condition scans.
            let code: String = src
                .lines()
                .map(|l| l.split("//").next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n");
            let tainted = input_branch_spans(&code);
            for builtin in ["fwidth(", "dpdx(", "dpdy("] {
                let mut from = 0;
                while let Some(rel) = code[from..].find(builtin) {
                    let at = from + rel;
                    assert!(
                        !tainted.iter().any(|&(s, e)| at >= s && at < e),
                        "{name}: `{builtin}` is inside an `if/else` on the per-fragment \
                         `input.…` (non-uniform) — Tint rejects this and blanks the \
                         canvas. Hoist the derivative to uniform control flow."
                    );
                    from = at + builtin.len();
                }
            }
        }
    }

    /// Source spans of every `if/while/for (… input.… ) { … }` construct, extended
    /// across trailing `else`/`else if` blocks — the whole chain is non-uniform, so a
    /// derivative anywhere inside it is the Tint-rejecting pattern. Branches on a
    /// UNIFORM (`stroke_params.…`, `text_params.…`) carry no `input.` header and are
    /// left out, so a legit uniform-gated derivative (e.g. dashed-stroke AA) is fine.
    fn input_branch_spans(code: &str) -> Vec<(usize, usize)> {
        let bytes = code.as_bytes();
        let mut spans = Vec::new();
        for kw in ["if (", "while (", "for ("] {
            let mut from = 0;
            while let Some(rel) = code[from..].find(kw) {
                let at = from + rel;
                from = at + kw.len();
                let Some(brace_rel) = code[at..].find('{') else { continue };
                let body_open = at + brace_rel;
                if !code[at..body_open].contains("input.") {
                    continue;
                }
                let mut end = match_block_end(bytes, body_open);
                loop {
                    let mut k = end;
                    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if code[k..].starts_with("else") {
                        if let Some(rel2) = code[k..].find('{') {
                            end = match_block_end(bytes, k + rel2);
                            continue;
                        }
                    }
                    break;
                }
                spans.push((body_open, end));
            }
        }
        spans
    }

    /// Index just past the `}` matching the `{` at `open`.
    fn match_block_end(bytes: &[u8], open: usize) -> usize {
        let mut depth = 0_i32;
        let mut i = open;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        bytes.len()
    }
}
