//! Viewport windowing decisions (port of the windowing logic in
//! `runtime/sceneClient.ts`).
//!
//! This crate owns only the windowing *decisions* — the region math, the margin,
//! and whether a re-subscribe is warranted. Debounce timing stays shell-side: the
//! shell runs its own timer and, when it fires, asks [`WindowState::set_window`]
//! whether the new window actually differs (a no-op move returns `false`, so no
//! `subscribe` frame is emitted).
//!
//! Windowing is the data-layer WINDOW the client subscribes to — distinct from
//! renderer culling: windowing controls which objects the client HOLDS at all;
//! renderer culling decides which of the held objects to draw each frame.

use shape_scene_core::model::Bounds;

/// A camera-derived viewport / window rect in WORLD coordinates.
pub type Bbox = Bounds;

/// Default fraction the viewport is grown on each side to form the window.
pub const DEFAULT_VIEWPORT_MARGIN: f64 = 0.5;

/// Grow a viewport bbox by `margin` of its size on each side. This is the
/// data-layer WINDOW the client subscribes to.
pub fn window_from_viewport(viewport: Bbox, margin: f64) -> Bbox {
    let pad_x = viewport.width * margin;
    let pad_y = viewport.height * margin;
    Bbox {
        x: viewport.x - pad_x,
        y: viewport.y - pad_y,
        width: viewport.width + pad_x * 2.0,
        height: viewport.height + pad_y * 2.0,
    }
}

/// True when two bboxes are equal enough that a re-subscribe would be a no-op.
/// `None` is whole-canvas; two `None`s are equal, a `None`/`Some` pair is not.
pub fn bbox_equals(a: Option<&Bbox>, b: Option<&Bbox>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
        }
        (None, None) => true,
        _ => false,
    }
}

/// The subscribed-window state machine. Holds the currently-subscribed window
/// (`None` = whole canvas) and decides, on each requested change, whether the
/// shell must emit a `subscribe` frame. The shell owns the debounce timer and the
/// transport; this only computes the decision.
#[derive(Clone, Debug, Default)]
pub struct WindowState {
    window: Option<Bbox>,
    margin: f64,
}

impl WindowState {
    /// A window state seeded from the connect region's bbox (`None` = whole
    /// canvas), using [`DEFAULT_VIEWPORT_MARGIN`].
    pub fn new(seed: Option<Bbox>) -> Self {
        Self {
            window: seed,
            margin: DEFAULT_VIEWPORT_MARGIN,
        }
    }

    /// A window state with an explicit margin (the web shell wires a configurable
    /// `viewportMargin`).
    pub fn with_margin(seed: Option<Bbox>, margin: f64) -> Self {
        Self {
            window: seed,
            margin,
        }
    }

    /// The window bbox currently subscribed, or `None` for whole-canvas.
    pub fn current_window(&self) -> Option<&Bbox> {
        self.window.as_ref()
    }

    /// The window-margin fraction this state grows viewports by.
    pub fn margin(&self) -> f64 {
        self.margin
    }

    /// Grow a camera `viewport` into the window bbox (margin applied), then decide
    /// whether to re-aim to it (the shell calls this when its debounce fires).
    /// Returns the bbox to `subscribe` to, or `None` when the window is unchanged
    /// (no frame needed).
    pub fn on_viewport(&mut self, viewport: Bbox) -> Option<Bbox> {
        let next = window_from_viewport(viewport, self.margin);
        self.set_window(next)
    }

    /// Re-aim the window to `bbox` directly (no margin). Returns the bbox to
    /// `subscribe` to, or `None` when the window is unchanged.
    pub fn set_window(&mut self, bbox: Bbox) -> Option<Bbox> {
        if bbox_equals(self.window.as_ref(), Some(&bbox)) {
            return None;
        }
        self.window = Some(bbox);
        Some(bbox)
    }

    /// Drop the window: decide whether to re-subscribe to the whole canvas.
    /// Returns true when a whole-canvas `subscribe` (no bbox) must be sent; false
    /// when already whole-canvas (no frame needed).
    pub fn subscribe_whole_canvas(&mut self) -> bool {
        if self.window.is_none() {
            return false;
        }
        self.window = None;
        true
    }
}
