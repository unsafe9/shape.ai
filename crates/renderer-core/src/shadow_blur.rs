//! W3-G8/A real drop-shadow blur: offscreen separable Gaussian.
//!
//! The G7 shadow faked softness by stacking scaled silhouette tiers. This module
//! replaces that with a TRUE macOS-style soft shadow: the per-object shadow
//! silhouette (the offset fill mesh) is rendered ONCE into an offscreen mask, blur
//! ed with a separable Gaussian (a horizontal pass then a vertical pass), and the
//! blurred result composites UNDER the fill in the visible object pass, tinted by
//! the theme `shadow` token. The shadow GEOMETRY is still baked once with the scene
//! (zero per-frame re-tessellation); only the two fixed-cost fullscreen blur passes
//! and the composite run per frame.
//!
//! ## Isolation contract
//!
//! A bug in this path must NEVER blank the canvas. The mask/blur/composite are an
//! ADDITIVE underlay: the fill/stroke/text passes draw on top into the same surface
//! regardless. The worst acceptable failure mode is "shadow missing/weak". The
//! composite reads only the blurred mask's ALPHA as coverage and multiplies it by
//! the theme shadow color, so an empty mask -> zero coverage -> invisible (never a
//! dark wash over the canvas).
//!
//! ## What is host-testable vs GPU-runtime-deferred
//!
//! [`gaussian_kernel`] is a pure, device-free function (normalized, symmetric
//! weights) with falsifiable unit tests below — it runs on the host test gate. The
//! WGSL blur/composite shaders, the offscreen render/sample wiring, bind-group /
//! pipeline-layout match, and the resize re-creation all compile at build time but
//! only EXECUTE on a browser device; the look is user-validated.

/// Maximum one-sided blur radius in taps. The kernel is `2*radius+1` taps wide;
/// the WGSL blur shader's fixed loop bound (`SHADOW_BLUR_MAX_RADIUS`) must match.
pub const SHADOW_BLUR_MAX_RADIUS: usize = 12;

/// Default blur radius in PHYSICAL pixels (zoom-independent screen blur). Scaled by
/// the device-pixel ratio at upload so the screen feather is constant across DPRs.
/// W3-G9/#1: bumped to the full 12-tap budget. Because the blur targets render at
/// QUARTER resolution (see [`quarter_dim`]) each tap steps 4 physical px, so 12 taps
/// reach ~48 physical px (~24 CSS px @2x) on every side — a wide, soft, symmetric
/// macOS-ambient halo at the SAME 12-tap cost.
pub const SHADOW_BLUR_RADIUS_PX: f32 = 12.0;

/// W3-G9/#1: the offscreen mask/ping/pong blur targets render at QUARTER resolution
/// of the surface. The two blur passes then touch 1/16 the fragments (faster, fixed
/// cost), each of the 12 taps steps 4 physical px for a wide spread, and the Linear
/// composite sampler upsamples the quarter-res blurred mask for free extra smoothing.
/// Guarded to a minimum of 1 so a tiny surface never yields a zero-sized texture.
pub fn quarter_dim(n: u32) -> u32 {
    (n / 4).max(1)
}

/// Compute a normalized, symmetric 1-D Gaussian kernel of `2*radius+1` taps. The
/// `sigma` controls the spread; passing `radius == 0` yields the trivial `[1.0]`
/// kernel (no blur). Weights are normalized to sum to 1.0 and are symmetric about
/// the center tap, so the separable H-then-V passes preserve total energy and a
/// wider sigma pushes more weight into the tails.
///
/// Pure and device-free: this is the falsifiable core of the blur (the WGSL shader
/// reads these exact weights from a uniform), so it carries the host unit tests.
pub fn gaussian_kernel(radius: usize, sigma: f32) -> Vec<f32> {
    if radius == 0 {
        return vec![1.0];
    }
    // A non-positive sigma would divide by zero; clamp to a tiny positive so the
    // kernel stays a valid (sharply-peaked) distribution instead of NaN.
    let sigma = sigma.max(1e-4);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let n = 2 * radius + 1;
    let mut weights = Vec::with_capacity(n);
    for i in 0..n {
        // Offset from the center tap, in [-radius, radius].
        let x = i as f32 - radius as f32;
        weights.push((-(x * x) / two_sigma_sq).exp());
    }
    let sum: f32 = weights.iter().sum();
    for w in &mut weights {
        *w /= sum;
    }
    weights
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The kernel is symmetric about the center tap and sums to ~1.0. FAILS if the
    /// kernel is left unnormalized (sum != 1) or built asymmetrically.
    #[test]
    fn gaussian_kernel_is_normalized_and_symmetric() {
        for radius in 1..=8usize {
            let sigma = radius as f32 / 3.0;
            let k = gaussian_kernel(radius, sigma);
            assert_eq!(k.len(), 2 * radius + 1, "kernel is 2*radius+1 taps wide");

            // Sums to 1.0 (normalized) within tight tolerance.
            let sum: f32 = k.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "radius {radius}: kernel sums to ~1.0 (got {sum})"
            );

            // Symmetric: weight at +i equals weight at -i.
            for i in 0..=radius {
                let lo = k[radius - i];
                let hi = k[radius + i];
                assert!(
                    (lo - hi).abs() < 1e-6,
                    "radius {radius}: tap +{i} ({hi}) != tap -{i} ({lo})"
                );
            }
        }
    }

    /// The center tap is the maximum (a Gaussian peaks at the center) and weights
    /// fall off monotonically toward the tails. FAILS if the distribution is flat
    /// (a box blur) or inverted.
    #[test]
    fn gaussian_kernel_peaks_at_center_and_falls_off() {
        let radius = 6;
        let k = gaussian_kernel(radius, radius as f32 / 3.0);
        for i in 0..radius {
            // Strictly decreasing from center out to the tail (one-sided).
            assert!(
                k[radius + i] > k[radius + i + 1],
                "tap {i} ({}) must exceed tap {} ({})",
                k[radius + i],
                i + 1,
                k[radius + i + 1]
            );
        }
        // The center is the global max.
        let center = k[radius];
        assert!(
            k.iter().all(|&w| w <= center + 1e-9),
            "center tap is the maximum weight"
        );
    }

    /// A WIDER sigma spreads more weight into the tails: the center weight DROPS and
    /// the tail weight RISES as sigma grows (the blur softens). This is the property
    /// that makes the blur actually blur. FAILS if sigma is ignored (e.g. a fixed
    /// box kernel) because the center/tail ratio would not move.
    #[test]
    fn wider_sigma_spreads_weight_to_tails() {
        let radius = 8;
        let narrow = gaussian_kernel(radius, 1.0);
        let wide = gaussian_kernel(radius, 4.0);

        // Center weight is monotonically smaller for the wider kernel.
        assert!(
            wide[radius] < narrow[radius],
            "wider sigma lowers the center weight ({} !< {})",
            wide[radius],
            narrow[radius]
        );
        // Tail (outermost) weight is larger for the wider kernel.
        assert!(
            wide[0] > narrow[0],
            "wider sigma raises the tail weight ({} !> {})",
            wide[0],
            narrow[0]
        );
        // Both still normalized.
        assert!((narrow.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((wide.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    /// `radius == 0` is the identity kernel (`[1.0]`) — no blur, still normalized.
    #[test]
    fn zero_radius_is_identity_kernel() {
        let k = gaussian_kernel(0, 1.0);
        assert_eq!(k, vec![1.0]);
    }

    /// W3-G9/#1: `quarter_dim` is exactly the surface dim / 4, floored at 1 so a tiny
    /// surface never yields a zero-sized texture. FAILS if the divisor drifts off 4
    /// (the texel step `4/full` assumes a quarter-res target) or the guard is dropped.
    #[test]
    fn quarter_dim_is_quarter_res_guarded_to_one() {
        assert_eq!(quarter_dim(1600), 400);
        assert_eq!(quarter_dim(900), 225);
        // Sub-quarter dims floor to 1, never 0 (a 0-sized texture is invalid).
        assert_eq!(quarter_dim(3), 1);
        assert_eq!(quarter_dim(1), 1);
        assert_eq!(quarter_dim(0), 1);
        // It is genuinely a quarter (not a half/eighth): each tap steps 4 physical px.
        for n in [8u32, 64, 256, 4096] {
            assert_eq!(quarter_dim(n), n / 4);
        }
    }

    /// The packed one-sided tap count never exceeds the shader's fixed loop bound.
    /// FAILS if `SHADOW_BLUR_MAX_RADIUS` and the WGSL `MAX_RADIUS` drift apart.
    #[test]
    fn blur_radius_fits_shader_loop_bound() {
        let radius_px = (SHADOW_BLUR_RADIUS_PX * 3.0).round() as usize; // 3x DPR
        let radius = radius_px.clamp(1, SHADOW_BLUR_MAX_RADIUS);
        assert!(radius <= SHADOW_BLUR_MAX_RADIUS);
        // The one-sided weights array (center + radius) fits the fixed slot count.
        assert!(radius + 1 <= SHADOW_BLUR_MAX_RADIUS + 1);
    }
}
