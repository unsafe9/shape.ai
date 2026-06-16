//! The one px→quantized convention for ui-core geometry coords. EXACT mirror of
//! scene-core `authoring/primitives.rs::q` (JS `Math.round` = `floor(x + 0.5)`);
//! re-defined locally so ui-core carries no scene-core dep. `QUANT_PER_PX` mirrors
//! the renderer-core wire constant by value (a wire constant, not an import).

pub(crate) const QUANT_PER_PX: f64 = 8.0;

/// Quantize logical px to object-local integer units, round half toward +∞.
pub(crate) fn q(px: f64) -> i32 {
    if px.is_nan() {
        return 0;
    }
    let units = (px * QUANT_PER_PX + 0.5).floor();
    let clamped = units.clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the value is an exact integer in range"
    )]
    let q = clamped as i32;
    q
}

/// JS `Math.round` for an already-product f64 (the kappa control offset). Mirrors
/// scene-core `primitives.rs::js_round`.
pub(crate) fn js_round(x: f64) -> i32 {
    if x.is_nan() {
        return 0;
    }
    let r = (x + 0.5).floor();
    let clamped = r.clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the value is an exact integer in range"
    )]
    let v = clamped as i32;
    v
}
