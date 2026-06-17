//! The ONE spacing scale + shared surface metrics every surface lays out against,
//! so a gap is a named step on a single 4px grid instead of a per-file magic
//! number. The values are the ones the surfaces already used (4 / 8 / 12 / 16);
//! collapsing them here is what removes the irregular spacing — a surface now asks
//! for `SPACE_SM` rather than re-deriving `8.0` (or `9.0`, the old separator drift).

/// The base grid every step is a multiple of. A gap not on this grid is the
/// defect this scale exists to prevent.
pub const GRID: f64 = 4.0;

/// 4px — the tightest gap (icon-to-icon, chip-to-chip in a dense row).
pub const SPACE_XS: f64 = GRID;
/// 8px — the default gap between a label and its control, and between stacked rows.
pub const SPACE_SM: f64 = GRID * 2.0;
/// 12px — the gap between sections of a panel.
pub const SPACE_MD: f64 = GRID * 3.0;
/// 16px — a panel's inner margin (content inset from the body edge).
pub const SPACE_LG: f64 = GRID * 4.0;

/// A standard control/row height — a text field, a labeled row, a unit pill.
pub const ROW_H: f64 = 28.0;

/// The corner radius shared by every elevated panel body (inspector, settings,
/// toolbar tray).
pub const PANEL_RADIUS: f64 = 14.0;
