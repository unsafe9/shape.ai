//! Level-of-detail (LOD) policy for the scene core.
//!
//! T3.1: LOD is a *visual-only* detail ladder selected by a single core-owned
//! function of an object's apparent on-screen size. Every tier preserves object
//! id, position, bounds, selection identity, and hit-test identity — a far card
//! degrades into a simpler drawing of *the same card*, never a different object.
//!
//! This module is host-neutral and MCP/business-free: it reads only geometry
//! (`bounds` x `camera.zoom`) and never any business field (`status`,
//! `node_type`, `confidence`). It returns plain data, no `JsValue`.

use crate::model::{CameraState, WorldRect};

/// Visual-only detail tiers, ordered from richest (`Full`) to sparsest
/// (`Minimap`). Tiers add or remove *paint* of the same object; they never
/// substitute, merge, or re-identify objects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LodTier {
    /// Fill + gradient + stroke + shadow/glow + title + summary + detail +
    /// badges + ports + edge labels + selection handles.
    Full,
    /// Fill + stroke + title + 1-line summary; `states.compact` alphas applied.
    Compact,
    /// Fill + stroke only, at `styleKey` color; all text/badges/ports/glow off.
    ShapeOnly,
    /// Overview: a single filled rounded rect per object at reduced alpha;
    /// group hulls drawn as flat tint; edges as thin straight lines.
    Density,
    /// Extreme far zoom: the object contributes only to its owning group hull's
    /// aggregated density tint. An optional non-interactive positional dot may
    /// hint location, but the object is never replaced.
    Minimap,
}

/// Tier band edges in CSS pixels of the object's apparent longest on-screen
/// edge. Bands are half-open `[lo, hi)`; an object at or above
/// [`FULL_THRESHOLD_PX`] is [`LodTier::Full`].
///
/// These are **policy constants**, named so T3.2/T3.4 can tune them against
/// benchmark evidence rather than digging them out of a shader.
pub const FULL_THRESHOLD_PX: f64 = 220.0;
pub const COMPACT_THRESHOLD_PX: f64 = 120.0;
pub const SHAPE_ONLY_THRESHOLD_PX: f64 = 40.0;
pub const DENSITY_THRESHOLD_PX: f64 = 8.0;

/// Fraction of a band edge used as a symmetric dead-band for hysteresis. A tier
/// only changes once `apparent_px` clears the far side of the dead-band around
/// the boundary, preventing tier flicker during a continuous pan/zoom.
pub const HYSTERESIS_FRACTION: f64 = 0.10;

/// The object's longest on-screen edge in CSS pixels. LOD is selected by
/// apparent size, not raw `camera.zoom`, so a small note card and a wide frame
/// reach the same tier at different zooms.
pub fn apparent_px(bounds: &WorldRect, camera: &CameraState) -> f64 {
    let longest_edge = bounds.width.abs().max(bounds.height.abs());
    (longest_edge * camera.zoom).max(0.0)
}

/// Tier for an `apparent_px` ignoring hysteresis: the band the size falls in.
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

/// The band edge below which `tier` demotes to the next sparser tier, or `None`
/// for [`LodTier::Minimap`], which is the sparsest tier and has no lower edge.
fn lower_edge(tier: LodTier) -> Option<f64> {
    match tier {
        LodTier::Full => Some(FULL_THRESHOLD_PX),
        LodTier::Compact => Some(COMPACT_THRESHOLD_PX),
        LodTier::ShapeOnly => Some(SHAPE_ONLY_THRESHOLD_PX),
        LodTier::Density => Some(DENSITY_THRESHOLD_PX),
        LodTier::Minimap => None,
    }
}

/// The band edge at or above which `tier` promotes to the next richer tier, or
/// `None` for [`LodTier::Full`], which is the richest tier and has no upper edge.
fn upper_edge(tier: LodTier) -> Option<f64> {
    match tier {
        LodTier::Full => None,
        LodTier::Compact => Some(FULL_THRESHOLD_PX),
        LodTier::ShapeOnly => Some(COMPACT_THRESHOLD_PX),
        LodTier::Density => Some(SHAPE_ONLY_THRESHOLD_PX),
        LodTier::Minimap => Some(DENSITY_THRESHOLD_PX),
    }
}

/// Resolve the LOD tier for an object of size `apparent_px`, honoring the
/// previously assigned tier for hysteresis.
///
/// With no `previous` tier the raw band ([`base_tier`]) is used. When
/// `previous` is supplied and `apparent_px` sits inside the dead-band straddling
/// `previous`'s boundary, the previous tier is held — the tier only changes once
/// the size clears the far side of the dead-band. This keeps a card hovering on
/// a boundary from flickering between tiers during a pan.
pub fn lod_tier(apparent_px: f64, previous: Option<LodTier>) -> LodTier {
    let base = base_tier(apparent_px);
    let Some(previous) = previous else {
        return base;
    };
    if base == previous {
        return previous;
    }

    // Size grew past `previous`'s upper edge: promote only after clearing the
    // dead-band above that edge.
    if let Some(upper) = upper_edge(previous) {
        if apparent_px >= upper {
            if apparent_px >= upper * (1.0 + HYSTERESIS_FRACTION) {
                return base;
            }
            return previous;
        }
    }

    // Size shrank below `previous`'s lower edge: demote only after clearing the
    // dead-band below that edge.
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

        // A wide frame and a default card reach the same apparent size at
        // different zooms.
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
        // Previously Compact (below the 220 edge). Growing just past 220 but
        // still inside the +10% dead-band must hold Compact, not promote.
        assert_eq!(lod_tier(221.0, Some(LodTier::Compact)), LodTier::Compact);
        // Clearing the far side (220 * 1.10 = 242) promotes to Full.
        assert_eq!(lod_tier(243.0, Some(LodTier::Compact)), LodTier::Full);
    }

    #[test]
    fn lod_tier_holds_inside_dead_band_on_demotion() {
        // Previously Full. Shrinking just below 220 but still inside the -10%
        // dead-band (220 * 0.90 = 198) must hold Full.
        assert_eq!(lod_tier(219.0, Some(LodTier::Full)), LodTier::Full);
        assert_eq!(lod_tier(199.0, Some(LodTier::Full)), LodTier::Full);
        // Clearing the far side demotes to Compact.
        assert_eq!(lod_tier(197.0, Some(LodTier::Full)), LodTier::Compact);
    }

    #[test]
    fn lod_tier_is_stable_when_size_unchanged() {
        // Same tier in, same tier out, regardless of dead-band proximity.
        assert_eq!(lod_tier(150.0, Some(LodTier::Compact)), LodTier::Compact);
        assert_eq!(lod_tier(300.0, Some(LodTier::Full)), LodTier::Full);
        assert_eq!(lod_tier(4.0, Some(LodTier::Minimap)), LodTier::Minimap);
    }

    #[test]
    fn lod_tier_demotes_minimap_past_density_floor() {
        // Density floor is 8px; from Density, shrinking past 8 * 0.9 = 7.2
        // reaches Minimap.
        assert_eq!(lod_tier(7.5, Some(LodTier::Density)), LodTier::Density);
        assert_eq!(lod_tier(7.0, Some(LodTier::Density)), LodTier::Minimap);
    }

    #[test]
    fn lod_tier_promotes_minimap_only_past_dead_band() {
        // From Minimap, growing past the Density floor 8px but inside the +10%
        // dead-band (8.8) holds Minimap; clearing it promotes to Density.
        assert_eq!(lod_tier(8.5, Some(LodTier::Minimap)), LodTier::Minimap);
        assert_eq!(lod_tier(9.0, Some(LodTier::Minimap)), LodTier::Density);
    }
}
