//! OB-3 object render model (OB3.R10): the renderer-core view of the single
//! `object` primitive (D1/D2/D4/D7) plus structural style defaults and the
//! selection/hover/focus visual resolution that used to live in the shell's
//! `renderScene.ts` (`defaultStyles`/`shapeStyleToken`).
//!
//! This is ADDITIVE: it does not touch the live `RenderGroup/RenderCard/RenderEdge`
//! pipeline in `model.rs`/`webgpu.rs`. The object pipeline is wired into the GPU
//! draw path at the OB-4 cutover, not here. All logic here is pure CPU and unit
//! tested in-file.
//!
//! Geometry coordinates are object-local quantized integers at 8 units/px (D2);
//! convert to f64 pixels with [`QUANT_PER_PX`] before any matrix math. The
//! transform is a 3x3 projective matrix in f64 (D7). The crate stays
//! pointer-width-agnostic: data fields use `i32` coords and `f64` matrices, never
//! `usize`.

use serde::{Deserialize, Serialize};

use crate::model::CameraState;

/// Quantization units per logical pixel for object-local geometry coordinates
/// (D2: agent-discretion default = 1/8 px). Divide an `i32` coord by this to get
/// f64 pixels for matrix math.
pub const QUANT_PER_PX: f64 = 8.0;

// ---------------------------------------------------------------------------
// Object render model (D1/D2/D4/D7)
// ---------------------------------------------------------------------------

/// A single canvas object as the renderer sees it (D1). Geometry is carried as a
/// path-string (`geometry_d`, D2) and parsed on demand via [`parse_path_d`];
/// style is inline (D4), there is no `styleKey`/palette lookup.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderObject {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// Fractional ordering key (string), see D1.
    pub order: String,
    /// 3x3 projective transform, row-major (D7). `screen = camera · M · local`.
    pub transform: [[f64; 3]; 3],
    /// SVG-subset path-string (M/L/C/Z, multi-subpath); object-local quantized
    /// `i32` coords at [`QUANT_PER_PX`] units/px (D2).
    #[serde(rename = "geometryD")]
    pub geometry_d: String,
    #[serde(default)]
    pub fill: Option<RFill>,
    #[serde(default)]
    pub stroke: Option<RStroke>,
    #[serde(default)]
    pub text: Option<RText>,
    /// Figma-style clip flag (D18). When true, children render clipped to this
    /// object's region/bounds.
    #[serde(default)]
    pub clip: bool,
}

/// Fill paint applied to the derived region, below stroke (D4).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RFill {
    pub paint: RPaint,
    /// Fill opacity in 0..=1. Defaults to fully opaque.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

/// Stroke for the outline / drawn line (D4): paint + width + opacity + dash/cap/join.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RStroke {
    pub paint: RPaint,
    /// Stroke width in logical pixels.
    pub width: f64,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    /// Dash pattern (on/off lengths in px); empty = solid.
    #[serde(default)]
    pub dash: Vec<f64>,
    #[serde(default)]
    pub cap: RStrokeCap,
    #[serde(default)]
    pub join: RStrokeJoin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RStrokeCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RStrokeJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Paint source for fill or stroke (D4). Colors are inline; there is no palette
/// or `styleKey` indirection.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RPaint {
    Solid {
        color: String,
    },
    Gradient {
        stops: Vec<RGradientStop>,
        /// Gradient angle in degrees.
        angle: f64,
    },
    Image {
        #[serde(rename = "contentRef")]
        content_ref: String,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RGradientStop {
    /// Stop offset in 0..=1.
    pub offset: f64,
    pub color: String,
}

/// Text content positioned relative to the derived region (D4): runs + align.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RText {
    pub runs: Vec<RTextRun>,
    #[serde(default)]
    pub align: RTextAlign,
    #[serde(default)]
    pub valign: RTextValign,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RTextRun {
    pub text: String,
    #[serde(default = "default_text_color")]
    pub color: String,
    #[serde(default = "default_text_size")]
    pub size: f64,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub font: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RTextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RTextValign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// The renderer-core scene view of the object substrate: scene id, camera (reused
/// from `model.rs`), the object list, the persisted single-anchor selection, and
/// the transient multi-select set.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderObjectScene {
    pub scene_id: String,
    pub camera: CameraState,
    pub objects: Vec<RenderObject>,
    /// Persisted single-anchor selection: the id of the selected object, if any.
    #[serde(default)]
    pub selection: Option<String>,
    /// Transient multi-select set, never persisted (mirrors `SceneSnapshot`).
    #[serde(default, skip)]
    pub multi_select: Vec<String>,
}

// ---------------------------------------------------------------------------
// Geometry path-string parse (D2)
// ---------------------------------------------------------------------------

/// A geometry node in object-local quantized i32 coords. Handles are stored
/// relative to the node (D2: `inHandle`/`outHandle`), so a node with no handles
/// is a straight (polyline) corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RNode {
    pub x: i32,
    pub y: i32,
    /// Incoming bezier handle, relative to (x, y). `None` = straight in.
    #[serde(default)]
    pub in_handle: Option<RHandle>,
    /// Outgoing bezier handle, relative to (x, y). `None` = straight out.
    #[serde(default)]
    pub out_handle: Option<RHandle>,
}

/// A bezier handle offset relative to its node, in quantized i32 coords.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RHandle {
    pub dx: i32,
    pub dy: i32,
}

/// One subpath (contour) of a parsed geometry: a node list plus a closed flag.
/// A multi-subpath path-string yields multiple `RSubPath`s (even-odd holes, D2).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RSubPath {
    pub closed: bool,
    pub nodes: Vec<RNode>,
}

/// Parse an SVG-subset path-string into subpaths (D2). Supported commands:
///
/// - `M x y`  — moveto, starts a new subpath at absolute (x, y).
/// - `L x y`  — lineto to absolute (x, y) (straight node).
/// - `C x1 y1 x2 y2 x y` — cubic to absolute (x, y); the absolute control points
///   `(x1,y1)`/`(x2,y2)` become the previous node's `out_handle` and this node's
///   `in_handle`, stored relative to their respective nodes.
/// - `Z`      — close the current subpath.
///
/// Coordinates are absolute integers (object-local quantized i32 at
/// [`QUANT_PER_PX`] units/px). Only the absolute (uppercase) forms are accepted,
/// mirroring the at-rest encoding. Whitespace and commas separate tokens.
pub fn parse_path_d(d: &str) -> Result<Vec<RSubPath>, String> {
    let mut tokens = Tokenizer::new(d);
    let mut subpaths: Vec<RSubPath> = Vec::new();
    let mut current: Option<RSubPath> = None;

    while let Some(tok) = tokens.next_command()? {
        match tok {
            'M' => {
                if let Some(sub) = current.take() {
                    subpaths.push(sub);
                }
                let x = tokens.next_int()?;
                let y = tokens.next_int()?;
                current = Some(RSubPath {
                    closed: false,
                    nodes: vec![RNode {
                        x,
                        y,
                        in_handle: None,
                        out_handle: None,
                    }],
                });
            }
            'L' => {
                let sub = current
                    .as_mut()
                    .ok_or_else(|| "L command before M".to_string())?;
                let x = tokens.next_int()?;
                let y = tokens.next_int()?;
                sub.nodes.push(RNode {
                    x,
                    y,
                    in_handle: None,
                    out_handle: None,
                });
            }
            'C' => {
                let sub = current
                    .as_mut()
                    .ok_or_else(|| "C command before M".to_string())?;
                let x1 = tokens.next_int()?;
                let y1 = tokens.next_int()?;
                let x2 = tokens.next_int()?;
                let y2 = tokens.next_int()?;
                let x = tokens.next_int()?;
                let y = tokens.next_int()?;
                // Absolute control points -> relative handles (D2).
                let prev = sub
                    .nodes
                    .last_mut()
                    .ok_or_else(|| "C command with no previous node".to_string())?;
                prev.out_handle = Some(RHandle {
                    dx: x1.checked_sub(prev.x).ok_or("out_handle overflow")?,
                    dy: y1.checked_sub(prev.y).ok_or("out_handle overflow")?,
                });
                sub.nodes.push(RNode {
                    x,
                    y,
                    in_handle: Some(RHandle {
                        dx: x2.checked_sub(x).ok_or("in_handle overflow")?,
                        dy: y2.checked_sub(y).ok_or("in_handle overflow")?,
                    }),
                    out_handle: None,
                });
            }
            'Z' => {
                let sub = current
                    .as_mut()
                    .ok_or_else(|| "Z command before M".to_string())?;
                sub.closed = true;
            }
            other => return Err(format!("unsupported path command '{other}'")),
        }
    }

    if let Some(sub) = current.take() {
        subpaths.push(sub);
    }
    if subpaths.is_empty() {
        return Err("empty path".to_string());
    }
    Ok(subpaths)
}

/// Minimal scanner over a path-string. Splits on whitespace and commas; commands
/// are single ASCII letters, coordinates are signed integers.
struct Tokenizer<'a> {
    rest: &'a str,
}

impl<'a> Tokenizer<'a> {
    fn new(d: &'a str) -> Self {
        Tokenizer { rest: d }
    }

    fn skip_separators(&mut self) {
        self.rest = self
            .rest
            .trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }

    /// Read the next command letter, or `None` at end of input.
    fn next_command(&mut self) -> Result<Option<char>, String> {
        self.skip_separators();
        match self.rest.chars().next() {
            None => Ok(None),
            Some(c) if c.is_ascii_alphabetic() => {
                self.rest = &self.rest[c.len_utf8()..];
                // Pass the letter through verbatim: only absolute (uppercase)
                // M/L/C/Z are accepted, so relative (lowercase) forms fall through
                // to the caller's unsupported-command error (D2 at-rest encoding).
                Ok(Some(c))
            }
            Some(c) => Err(format!("expected command, found '{c}'")),
        }
    }

    /// Read the next signed integer coordinate.
    fn next_int(&mut self) -> Result<i32, String> {
        self.skip_separators();
        let end = self
            .rest
            .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+'))
            .unwrap_or(self.rest.len());
        if end == 0 {
            return Err("expected integer coordinate".to_string());
        }
        let (num, rest) = self.rest.split_at(end);
        self.rest = rest;
        num.parse::<i32>()
            .map_err(|_| format!("invalid integer coordinate '{num}'"))
    }
}

// ---------------------------------------------------------------------------
// Style defaults + visual state resolution (OB3.R10)
// ---------------------------------------------------------------------------

/// Structural default fill applied when an object omits `fill` (OB3.R10). Mirrors
/// the shell's neutral surface; color is inline, no palette lookup.
pub fn default_fill() -> RFill {
    RFill {
        paint: RPaint::Solid {
            color: "#ffffff".to_string(),
        },
        opacity: 1.0,
    }
}

/// Structural default stroke applied when an object omits `stroke` (OB3.R10).
pub fn default_stroke() -> RStroke {
    RStroke {
        paint: RPaint::Solid {
            color: "#283644".to_string(),
        },
        width: 1.0,
        opacity: 1.0,
        dash: Vec::new(),
        cap: RStrokeCap::Butt,
        join: RStrokeJoin::Miter,
    }
}

/// Focus-ring color used when an object is selected/focused (OB3.R10), from the
/// shell's `focus` token (`#2f7ee6`).
pub const FOCUS_RING_COLOR: &str = "#2f7ee6";
/// Focus-ring width in logical pixels, from the shell's `strokeWidths.focusRing`.
pub const FOCUS_RING_WIDTH: f64 = 4.0;

/// Visual interaction state for an object (OB3.R10). The renderer owns this
/// instead of the shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualState {
    pub selected: bool,
    pub hovered: bool,
    pub focused: bool,
}

/// A focus ring to draw around an object's region.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusRing {
    pub color: String,
    pub width: f64,
}

/// The fully resolved draw style for an object: inline style over structural
/// defaults, plus any selection/focus ring (OB3.R10).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    pub fill: RFill,
    pub stroke: RStroke,
    pub focus_ring: Option<FocusRing>,
}

/// Resolve an object's draw style (OB3.R10): apply the object's inline `fill`/
/// `stroke` over the structural defaults, then add a focus ring when the visual
/// state is selected or focused. Hover currently carries no style change; it is
/// part of the owned state so the GPU path can react later without re-plumbing.
pub fn resolve_visual(obj: &RenderObject, state: VisualState) -> ResolvedStyle {
    let fill = obj.fill.clone().unwrap_or_else(default_fill);
    let stroke = obj.stroke.clone().unwrap_or_else(default_stroke);
    let focus_ring = if state.selected || state.focused {
        Some(FocusRing {
            color: FOCUS_RING_COLOR.to_string(),
            width: FOCUS_RING_WIDTH,
        })
    } else {
        None
    };
    ResolvedStyle {
        fill,
        stroke,
        focus_ring,
    }
}

fn default_opacity() -> f64 {
    1.0
}

fn default_text_color() -> String {
    "#111111".to_string()
}

fn default_text_size() -> f64 {
    16.0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(x: i32, y: i32) -> RNode {
        RNode {
            x,
            y,
            in_handle: None,
            out_handle: None,
        }
    }

    #[test]
    fn parse_rect_path() {
        // A closed unit-ish rect: M 0 0 L 80 0 L 80 40 L 0 40 Z
        let subs = parse_path_d("M 0 0 L 80 0 L 80 40 L 0 40 Z").expect("rect parses");
        assert_eq!(subs.len(), 1);
        let sub = &subs[0];
        assert!(sub.closed);
        assert_eq!(
            sub.nodes,
            vec![
                straight(0, 0),
                straight(80, 0),
                straight(80, 40),
                straight(0, 40),
            ]
        );
    }

    #[test]
    fn parse_cubic_path_absolute_controls_to_relative_handles() {
        // M 0 0 C 10 -20 90 -20 100 0
        // start node at (0,0), end node at (100,0).
        // abs control1 (10,-20) -> start.out_handle = (10, -20)
        // abs control2 (90,-20) -> end.in_handle    = (90-100, -20-0) = (-10, -20)
        let subs = parse_path_d("M 0 0 C 10 -20 90 -20 100 0").expect("cubic parses");
        assert_eq!(subs.len(), 1);
        let sub = &subs[0];
        assert!(!sub.closed);
        assert_eq!(sub.nodes.len(), 2);

        let start = &sub.nodes[0];
        assert_eq!((start.x, start.y), (0, 0));
        assert_eq!(start.in_handle, None);
        assert_eq!(start.out_handle, Some(RHandle { dx: 10, dy: -20 }));

        let end = &sub.nodes[1];
        assert_eq!((end.x, end.y), (100, 0));
        assert_eq!(end.in_handle, Some(RHandle { dx: -10, dy: -20 }));
        assert_eq!(end.out_handle, None);
    }

    #[test]
    fn parse_multi_subpath_with_commas() {
        // Two subpaths in one string, comma-separated coords (even-odd hole, D2).
        let subs =
            parse_path_d("M0,0 L40,0 L40,40 Z M10,10 L20,10 L20,20 Z").expect("multi parses");
        assert_eq!(subs.len(), 2);
        assert!(subs[0].closed);
        assert!(subs[1].closed);
        assert_eq!(subs[0].nodes.len(), 3);
        assert_eq!(subs[1].nodes[0], straight(10, 10));
    }

    #[test]
    fn parse_rejects_relative_and_unknown_commands() {
        assert!(parse_path_d("m 0 0 l 10 10").is_err());
        assert!(parse_path_d("M 0 0 Q 1 1 2 2").is_err());
        assert!(parse_path_d("L 0 0").is_err());
        assert!(parse_path_d("").is_err());
    }

    #[test]
    fn default_style_resolution_uses_defaults_when_unstyled() {
        let obj = RenderObject {
            id: "o1".to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity_transform(),
            geometry_d: "M 0 0 L 80 0 L 80 40 L 0 40 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            clip: false,
        };
        let resolved = resolve_visual(&obj, VisualState::default());
        match &resolved.fill.paint {
            RPaint::Solid { color } => assert_eq!(color, "#ffffff"),
            _ => panic!("expected solid default fill"),
        }
        match &resolved.stroke.paint {
            RPaint::Solid { color } => assert_eq!(color, "#283644"),
            _ => panic!("expected solid default stroke"),
        }
        assert_eq!(resolved.stroke.width, 1.0);
        assert_eq!(resolved.focus_ring, None);
    }

    #[test]
    fn inline_style_overrides_defaults() {
        let obj = RenderObject {
            id: "o2".to_string(),
            parent: None,
            order: "a1".to_string(),
            transform: identity_transform(),
            geometry_d: "M 0 0 L 10 0 L 10 10 Z".to_string(),
            fill: Some(RFill {
                paint: RPaint::Solid {
                    color: "#ff0000".to_string(),
                },
                opacity: 0.5,
            }),
            stroke: Some(RStroke {
                paint: RPaint::Solid {
                    color: "#00ff00".to_string(),
                },
                width: 3.0,
                opacity: 1.0,
                dash: vec![4.0, 2.0],
                cap: RStrokeCap::Round,
                join: RStrokeJoin::Round,
            }),
            text: None,
            clip: false,
        };
        let resolved = resolve_visual(&obj, VisualState::default());
        match &resolved.fill.paint {
            RPaint::Solid { color } => assert_eq!(color, "#ff0000"),
            _ => panic!("expected inline fill"),
        }
        assert_eq!(resolved.fill.opacity, 0.5);
        assert_eq!(resolved.stroke.width, 3.0);
        assert_eq!(resolved.stroke.dash, vec![4.0, 2.0]);
        assert_eq!(resolved.stroke.cap, RStrokeCap::Round);
    }

    #[test]
    fn selection_adds_focus_ring() {
        let obj = RenderObject {
            id: "o3".to_string(),
            parent: None,
            order: "a2".to_string(),
            transform: identity_transform(),
            geometry_d: "M 0 0 L 10 0 L 10 10 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            clip: false,
        };
        let selected = resolve_visual(
            &obj,
            VisualState {
                selected: true,
                ..VisualState::default()
            },
        );
        let ring = selected.focus_ring.expect("selection adds a focus ring");
        assert_eq!(ring.color, FOCUS_RING_COLOR);
        assert_eq!(ring.width, FOCUS_RING_WIDTH);

        // Focused-but-not-selected also rings; hover alone does not.
        let focused = resolve_visual(
            &obj,
            VisualState {
                focused: true,
                ..VisualState::default()
            },
        );
        assert!(focused.focus_ring.is_some());
        let hovered = resolve_visual(
            &obj,
            VisualState {
                hovered: true,
                ..VisualState::default()
            },
        );
        assert!(hovered.focus_ring.is_none());
    }

    #[test]
    fn deserializes_minimal_model_text_and_stroke() {
        // The model is the source of truth: the shell sends a minimal text shape
        // (runs without color/size, align as the model's `start` variant) and a
        // stroke. The renderer feed must tolerate it (FC-01).
        let json = r##"{
            "sceneId": "s1",
            "camera": { "x": 0, "y": 0, "zoom": 1 },
            "objects": [{
                "id": "o1",
                "order": "a0",
                "transform": [[1,0,0],[0,1,0],[0,0,1]],
                "geometryD": "M 0 0 L 80 0 L 80 40 L 0 40 Z",
                "stroke": { "paint": { "kind": "solid", "color": "#000000" }, "width": 8 },
                "text": { "runs": [{ "text": "Note" }], "align": "start", "valign": "top" }
            }]
        }"##;
        let scene: RenderObjectScene =
            serde_json::from_str(json).expect("minimal model text + stroke deserializes");
        let obj = &scene.objects[0];
        let text = obj.text.as_ref().expect("text present");
        assert_eq!(text.align, RTextAlign::Start);
        assert_eq!(text.valign, RTextValign::Top);
        let run = &text.runs[0];
        assert_eq!(run.color, "#111111");
        assert_eq!(run.size, 16.0);
        assert_eq!(obj.stroke.as_ref().expect("stroke present").width, 8.0);
    }

    #[test]
    fn quantized_coords_to_pixels() {
        // 80 quantized units at 8 units/px = 10 px.
        assert_eq!(80.0 / QUANT_PER_PX, 10.0);
    }

    fn identity_transform() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }
}
