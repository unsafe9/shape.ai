//! The audited narrowing-cast points for the renderer. The workspace denies
//! `clippy::cast_possible_truncation` to keep the cores pointer-width-agnostic
//! (see root `Cargo.toml`); these helpers concentrate the genuinely-intended
//! narrowings into one reasoned site each instead of scattering per-call
//! `#[allow]`s, so a stray un-audited `as` still trips the gate. Public so the
//! GPU half (`renderer-wgpu`, which depends on this crate by path) shares the
//! same single audited boundary rather than re-declaring its own `#[allow]`s.

/// Narrow an `f64` to the `f32` the renderer's geometry and GPU pipelines use.
/// Vertices, uniforms, and the hit-test geometry space are all `f32`; precision
/// loss at that boundary is the intended, lossy contract.
#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "renderer geometry/uniforms/vertices are f32; the f64->f32 narrowing at that boundary is the intended, lossy contract"
)]
pub fn narrow_f32(x: f64) -> f32 {
    x as f32
}

/// Round a non-negative `f32` pixel/cell measure to `u32`, clamping the result
/// into range so the cast is provably exact rather than merely silenced. NaN maps
/// to `0`. Used for atlas dimensions, glyph cell sizes, and px-size keys.
#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "clamped to [0, u32::MAX] then rounded; the value is an exact integer in range"
)]
pub fn round_u32(x: f32) -> u32 {
    if x.is_nan() {
        return 0;
    }
    x.round().clamp(0.0, u32::MAX as f32) as u32
}

/// Round an `f64` to `i32`, clamping the result into range. NaN maps to `0`.
#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "clamped to [i32::MIN, i32::MAX] then rounded; the value is an exact integer in range"
)]
pub fn round_i32(x: f64) -> i32 {
    if x.is_nan() {
        return 0;
    }
    x.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// A buffer element count as the `u32` the GPU index/range space uses. Vertex and
/// index buffers are addressed in `u32`; a mesh that overflowed `u32` could not be
/// drawn, so the count is in range by construction.
#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "GPU vertex/index ranges are u32; a buffer larger than u32::MAX is undrawable, so the element count is in range by construction"
)]
pub fn len_u32(n: usize) -> u32 {
    n as u32
}
