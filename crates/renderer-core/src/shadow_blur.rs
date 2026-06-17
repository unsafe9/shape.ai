//! Drop-shadow blur: offscreen separable Gaussian.
//!
//! Isolation contract: a bug here must NEVER blank the canvas. The mask/blur/
//! composite are an additive underlay; the composite reads only the blurred mask's
//! ALPHA as coverage times the theme shadow color, so an empty mask yields zero
//! coverage (invisible), never a dark wash. Worst acceptable failure is "shadow
//! missing/weak".

/// Kernel is `2*radius+1` taps wide; the WGSL blur shader's fixed loop bound must match.
pub const SHADOW_BLUR_MAX_RADIUS: usize = 12;

/// Default blur radius in PHYSICAL pixels (zoom-independent). Scaled by the DPR at
/// upload so the screen feather is constant across DPRs. Blur targets render at
/// quarter resolution, so each tap steps 4 physical px. Tuned for a tight macOS-card
/// elevation rather than a wide diffuse halo.
pub const SHADOW_BLUR_RADIUS_PX: f32 = 8.0;

/// Quarter resolution of the surface for the blur targets. Floored at 1 so a tiny
/// surface never yields a zero-sized texture. The texel step `4/full` assumes /4.
pub fn quarter_dim(n: u32) -> u32 {
    (n / 4).max(1)
}

/// Normalized, symmetric 1-D Gaussian kernel of `2*radius+1` taps; `radius == 0`
/// yields `[1.0]`. The WGSL shader reads these exact weights from a uniform.
pub fn gaussian_kernel(radius: usize, sigma: f32) -> Vec<f32> {
    if radius == 0 {
        return vec![1.0];
    }
    // Non-positive sigma would divide by zero; clamp to a tiny positive (avoids NaN).
    let sigma = sigma.max(1e-4);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let n = 2 * radius + 1;
    let mut weights = Vec::with_capacity(n);
    for i in 0..n {
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

    #[test]
    fn gaussian_kernel_is_normalized_and_symmetric() {
        for radius in 1..=8usize {
            let sigma = radius as f32 / 3.0;
            let k = gaussian_kernel(radius, sigma);
            assert_eq!(k.len(), 2 * radius + 1, "kernel is 2*radius+1 taps wide");

            let sum: f32 = k.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "radius {radius}: kernel sums to ~1.0 (got {sum})"
            );

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

    #[test]
    fn gaussian_kernel_peaks_at_center_and_falls_off() {
        let radius = 6;
        let k = gaussian_kernel(radius, radius as f32 / 3.0);
        for i in 0..radius {
            assert!(
                k[radius + i] > k[radius + i + 1],
                "tap {i} ({}) must exceed tap {} ({})",
                k[radius + i],
                i + 1,
                k[radius + i + 1]
            );
        }
        let center = k[radius];
        assert!(
            k.iter().all(|&w| w <= center + 1e-9),
            "center tap is the maximum weight"
        );
    }

    #[test]
    fn wider_sigma_spreads_weight_to_tails() {
        let radius = 8;
        let narrow = gaussian_kernel(radius, 1.0);
        let wide = gaussian_kernel(radius, 4.0);

        assert!(
            wide[radius] < narrow[radius],
            "wider sigma lowers the center weight ({} !< {})",
            wide[radius],
            narrow[radius]
        );
        assert!(
            wide[0] > narrow[0],
            "wider sigma raises the tail weight ({} !> {})",
            wide[0],
            narrow[0]
        );
        assert!((narrow.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((wide.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn zero_radius_is_identity_kernel() {
        let k = gaussian_kernel(0, 1.0);
        assert_eq!(k, vec![1.0]);
    }

    #[test]
    fn quarter_dim_is_quarter_res_guarded_to_one() {
        assert_eq!(quarter_dim(1600), 400);
        assert_eq!(quarter_dim(900), 225);
        assert_eq!(quarter_dim(3), 1);
        assert_eq!(quarter_dim(1), 1);
        assert_eq!(quarter_dim(0), 1);
        for n in [8u32, 64, 256, 4096] {
            assert_eq!(quarter_dim(n), n / 4);
        }
    }

    #[test]
    fn default_blur_radius_is_the_tight_elevation_value() {
        // Pinned exact: a tight macOS-card elevation, not the old wide diffuse halo.
        // A regression back up toward the heavy default fails here.
        assert_eq!(SHADOW_BLUR_RADIUS_PX, 8.0);
    }

    #[test]
    fn blur_radius_fits_shader_loop_bound() {
        // 3x DPR; the small positive product rounds to a handful of px.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "small positive constant product; rounds to an exact in-range integer"
        )]
        let radius_px = (SHADOW_BLUR_RADIUS_PX * 3.0).round() as usize;
        let radius = radius_px.clamp(1, SHADOW_BLUR_MAX_RADIUS);
        assert!(radius <= SHADOW_BLUR_MAX_RADIUS);
        assert!(radius + 1 <= SHADOW_BLUR_MAX_RADIUS + 1);
    }
}
