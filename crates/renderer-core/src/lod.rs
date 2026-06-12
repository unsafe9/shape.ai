//! Level-of-detail (LOD) policy for the scene core. A visual-only detail ladder
//! selected from an object's apparent on-screen size. Every tier preserves object
//! id, position, bounds, selection identity, and hit-test identity — a far card
//! degrades into a simpler drawing of *the same card*, never a different object.
//! Reads only geometry (`bounds` x `camera.zoom`), never a business field.

use crate::model::{CameraState, WorldRect};

/// Visual-only detail tiers, richest (`Full`) to sparsest (`Minimap`). Tiers add or
/// remove *paint* of the same object; they never substitute, merge, or re-identify.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LodTier {
    Full,
    Compact,
    ShapeOnly,
    Density,
    Minimap,
}

/// Tier band edges in CSS pixels of the object's apparent longest on-screen edge.
/// Bands are half-open `[lo, hi)`; at or above [`FULL_THRESHOLD_PX`] is [`LodTier::Full`].
pub const FULL_THRESHOLD_PX: f64 = 220.0;
pub const COMPACT_THRESHOLD_PX: f64 = 120.0;
pub const SHAPE_ONLY_THRESHOLD_PX: f64 = 40.0;
pub const DENSITY_THRESHOLD_PX: f64 = 8.0;

/// Symmetric dead-band fraction for hysteresis: a tier changes only once
/// `apparent_px` clears the far side of the band, preventing flicker during pan/zoom.
pub const HYSTERESIS_FRACTION: f64 = 0.10;

/// The object's longest on-screen edge in CSS pixels (longest world edge x zoom).
pub fn apparent_px(bounds: &WorldRect, camera: &CameraState) -> f64 {
    let longest_edge = bounds.width.abs().max(bounds.height.abs());
    (longest_edge * camera.zoom).max(0.0)
}

fn base_tier(apparent_px: f64) -> LodTier {
    if apparent_px >= FULL_THRESHOLD_PX {
        LodTier::Full
    } else if apparent_px >= COMPACT_THRESHOLD_PX {
        LodTier::Compact
    } else if apparent_px >= SHAPE_ONLY_THRESHOLD_PX {
        LodTier::ShapeOnly
    } else if apparent_px >= DENSITY_THRESHOLD_PX {
        LodTier::Density
    } else {
        LodTier::Minimap
    }
}

fn lower_edge(tier: LodTier) -> Option<f64> {
    match tier {
        LodTier::Full => Some(FULL_THRESHOLD_PX),
        LodTier::Compact => Some(COMPACT_THRESHOLD_PX),
        LodTier::ShapeOnly => Some(SHAPE_ONLY_THRESHOLD_PX),
        LodTier::Density => Some(DENSITY_THRESHOLD_PX),
        LodTier::Minimap => None,
    }
}

fn upper_edge(tier: LodTier) -> Option<f64> {
    match tier {
        LodTier::Full => None,
        LodTier::Compact => Some(FULL_THRESHOLD_PX),
        LodTier::ShapeOnly => Some(COMPACT_THRESHOLD_PX),
        LodTier::Density => Some(SHAPE_ONLY_THRESHOLD_PX),
        LodTier::Minimap => Some(DENSITY_THRESHOLD_PX),
    }
}

/// Resolve the LOD tier for `apparent_px`, holding `previous` while the size sits
/// inside the dead-band straddling its boundary (hysteresis).
pub fn lod_tier(apparent_px: f64, previous: Option<LodTier>) -> LodTier {
    let base = base_tier(apparent_px);
    let Some(previous) = previous else {
        return base;
    };
    if base == previous {
        return previous;
    }

    if let Some(upper) = upper_edge(previous) {
        if apparent_px >= upper {
            if apparent_px >= upper * (1.0 + HYSTERESIS_FRACTION) {
                return base;
            }
            return previous;
        }
    }

    if let Some(lower) = lower_edge(previous) {
        if apparent_px < lower {
            if apparent_px < lower * (1.0 - HYSTERESIS_FRACTION) {
                return base;
            }
            return previous;
        }
    }

    base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(zoom: f64) -> CameraState {
        CameraState {
            x: 0.0,
            y: 0.0,
            zoom,
        }
    }

    fn rect(width: f64, height: f64) -> WorldRect {
        WorldRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    #[test]
    fn apparent_px_uses_longest_edge_times_zoom() {
        let bounds = rect(390.0, 200.0);
        assert_eq!(apparent_px(&bounds, &camera(1.0)), 390.0);
        assert_eq!(apparent_px(&bounds, &camera(0.5)), 195.0);

        let wide = rect(2000.0, 600.0);
        let card = rect(390.0, 390.0);
        assert_eq!(apparent_px(&wide, &camera(0.1)), 200.0);
        assert!((apparent_px(&card, &camera(0.5128)) - 200.0).abs() < 1.0);
    }

    #[test]
    fn base_tier_maps_each_band() {
        assert_eq!(base_tier(500.0), LodTier::Full);
        assert_eq!(base_tier(220.0), LodTier::Full); // band edge is inclusive lo
        assert_eq!(base_tier(219.99), LodTier::Compact);
        assert_eq!(base_tier(120.0), LodTier::Compact);
        assert_eq!(base_tier(119.99), LodTier::ShapeOnly);
        assert_eq!(base_tier(40.0), LodTier::ShapeOnly);
        assert_eq!(base_tier(39.99), LodTier::Density);
        assert_eq!(base_tier(8.0), LodTier::Density);
        assert_eq!(base_tier(7.99), LodTier::Minimap);
        assert_eq!(base_tier(0.0), LodTier::Minimap);
    }

    #[test]
    fn lod_tier_without_previous_uses_raw_band() {
        assert_eq!(lod_tier(300.0, None), LodTier::Full);
        assert_eq!(lod_tier(150.0, None), LodTier::Compact);
        assert_eq!(lod_tier(60.0, None), LodTier::ShapeOnly);
        assert_eq!(lod_tier(20.0, None), LodTier::Density);
        assert_eq!(lod_tier(4.0, None), LodTier::Minimap);
    }

    #[test]
    fn lod_tier_holds_inside_dead_band_on_promotion() {
        assert_eq!(lod_tier(221.0, Some(LodTier::Compact)), LodTier::Compact);
        assert_eq!(lod_tier(243.0, Some(LodTier::Compact)), LodTier::Full);
    }

    #[test]
    fn lod_tier_holds_inside_dead_band_on_demotion() {
        assert_eq!(lod_tier(219.0, Some(LodTier::Full)), LodTier::Full);
        assert_eq!(lod_tier(199.0, Some(LodTier::Full)), LodTier::Full);
        assert_eq!(lod_tier(197.0, Some(LodTier::Full)), LodTier::Compact);
    }

    #[test]
    fn lod_tier_is_stable_when_size_unchanged() {
        assert_eq!(lod_tier(150.0, Some(LodTier::Compact)), LodTier::Compact);
        assert_eq!(lod_tier(300.0, Some(LodTier::Full)), LodTier::Full);
        assert_eq!(lod_tier(4.0, Some(LodTier::Minimap)), LodTier::Minimap);
    }

    #[test]
    fn lod_tier_demotes_minimap_past_density_floor() {
        assert_eq!(lod_tier(7.5, Some(LodTier::Density)), LodTier::Density);
        assert_eq!(lod_tier(7.0, Some(LodTier::Density)), LodTier::Minimap);
    }

    #[test]
    fn lod_tier_promotes_minimap_only_past_dead_band() {
        assert_eq!(lod_tier(8.5, Some(LodTier::Minimap)), LodTier::Minimap);
        assert_eq!(lod_tier(9.0, Some(LodTier::Minimap)), LodTier::Density);
    }
}
