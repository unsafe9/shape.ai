//! Backdrop-blur gating: the host-testable decision of WHEN a panel's frosted
//! backdrop must be recomputed.
//!
//! Panels are screen-fixed (identity camera), so the only thing that changes the
//! world behind them is a camera move or an object feed. Recomputing a full
//! separable Gaussian every frame is the perf failure the redesign forbids; this
//! tracker collapses "did the world behind the panels change?" into one boolean so
//! the GPU half can skip the blur passes and reuse the prior blurred crop when the
//! answer is no.
//!
//! The decision lives here (not in the renderer's GPU path) so it is provable on a
//! GPU-less host: feed it camera poses + feed ticks, assert the dirty verdict.

use crate::model::CameraState;

/// The world-pose signature a frame's backdrop blur was computed against. Two
/// frames whose signatures match render the same world behind a screen-fixed
/// panel, so the blurred crop is reusable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBlurKey {
    /// Camera pose, bit-pattern compared so a sub-epsilon jitter still counts as a
    /// move (the world genuinely shifted). `f64::to_bits` makes the compare exact
    /// and total (no NaN-vs-NaN surprises: equal bits == equal key).
    camera_bits: [u64; 3],
    /// A monotonically bumped tick: every object/scene feed increments it, so a
    /// content change with an unchanged camera still invalidates the crop.
    feed_tick: u64,
}

impl WorldBlurKey {
    pub fn new(camera: &CameraState, feed_tick: u64) -> Self {
        Self {
            camera_bits: [camera.x.to_bits(), camera.y.to_bits(), camera.zoom.to_bits()],
            feed_tick,
        }
    }
}

/// Tracks the last-blurred world signature so the renderer can gate the panel
/// backdrop blur on a real change. `feed_tick` is owned here: bump it on every
/// object/UI feed; the camera flows in from the live frame.
#[derive(Clone, Debug, Default)]
pub struct BackdropBlurGate {
    /// `None` until the first blur is recorded — the first frame is always dirty.
    last: Option<WorldBlurKey>,
    feed_tick: u64,
}

impl BackdropBlurGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note that the world content changed (an object/UI scene feed). The next
    /// `is_dirty` is forced true even if the camera is identical.
    pub fn mark_feed(&mut self) {
        self.feed_tick = self.feed_tick.wrapping_add(1);
    }

    /// The current world-pose signature (live camera + accumulated feed tick).
    pub fn current_key(&self, camera: &CameraState) -> WorldBlurKey {
        WorldBlurKey::new(camera, self.feed_tick)
    }

    /// Whether the panel backdrop must be recomputed this frame: true when the
    /// world behind the (screen-fixed) panels differs from the last blurred frame,
    /// or no blur has been recorded yet.
    pub fn is_dirty(&self, camera: &CameraState) -> bool {
        match self.last {
            Some(prev) => prev != self.current_key(camera),
            None => true,
        }
    }

    /// Record that the blur was (re)computed against `camera` this frame, so a later
    /// frame with the same world reuses the crop. Call ONLY after the blur actually
    /// ran; skipping it keeps the panel dirty.
    pub fn record_blurred(&mut self, camera: &CameraState) {
        self.last = Some(self.current_key(camera));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(x: f64, y: f64, zoom: f64) -> CameraState {
        CameraState { x, y, zoom }
    }

    #[test]
    fn first_frame_is_always_dirty() {
        let gate = BackdropBlurGate::new();
        assert!(
            gate.is_dirty(&cam(0.0, 0.0, 1.0)),
            "a gate that has never recorded a blur must report dirty"
        );
    }

    #[test]
    fn a_static_world_reuses_the_crop_after_the_first_blur() {
        let mut gate = BackdropBlurGate::new();
        let camera = cam(10.0, -5.0, 2.0);
        gate.record_blurred(&camera);
        assert!(
            !gate.is_dirty(&camera),
            "same camera + no feed since the blur must reuse the crop (no per-frame reblur)"
        );
    }

    #[test]
    fn a_camera_move_marks_the_panel_dirty() {
        let mut gate = BackdropBlurGate::new();
        gate.record_blurred(&cam(0.0, 0.0, 1.0));
        assert!(gate.is_dirty(&cam(1.0, 0.0, 1.0)), "a pan invalidates the crop");
        assert!(gate.is_dirty(&cam(0.0, 1.0, 1.0)), "a pan invalidates the crop");
        assert!(gate.is_dirty(&cam(0.0, 0.0, 1.5)), "a zoom invalidates the crop");
    }

    #[test]
    fn a_subpixel_camera_jitter_still_counts_as_dirty() {
        let mut gate = BackdropBlurGate::new();
        let base = cam(0.0, 0.0, 1.0);
        gate.record_blurred(&base);
        let jittered = cam(f64::from_bits(base.x.to_bits() + 1), 0.0, 1.0);
        assert!(
            gate.is_dirty(&jittered),
            "any camera bit change is a world move (bit-exact compare)"
        );
    }

    #[test]
    fn a_feed_marks_dirty_even_with_a_static_camera() {
        let mut gate = BackdropBlurGate::new();
        let camera = cam(3.0, 4.0, 1.0);
        gate.record_blurred(&camera);
        assert!(!gate.is_dirty(&camera), "static precondition");
        gate.mark_feed();
        assert!(
            gate.is_dirty(&camera),
            "an object feed changes the world behind a screen-fixed panel"
        );
        gate.record_blurred(&camera);
        assert!(
            !gate.is_dirty(&camera),
            "re-recording after the feed-driven reblur clears the dirty bit"
        );
    }
}
