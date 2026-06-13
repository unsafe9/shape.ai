//! Object render model: the renderer-core view of the single `object` primitive
//! plus structural style defaults and selection/hover/focus visual resolution.
//!
//! Geometry coordinates are object-local quantized integers at 8 units/px; convert
//! to f64 pixels with [`QUANT_PER_PX`] before any matrix math. The transform is a
//! 3x3 projective matrix in f64. Pointer-width-agnostic: `i32` coords and `f64`
//! matrices, never `usize`.

use serde::{Deserialize, Serialize};

use shape_scene_core::object::model::{
    self, GEOMETRY_QUANTUM_PER_PX, Object, ObjectScene, ObjectSelection,
};

use crate::model::CameraState;

/// Quantization units per logical pixel for object-local geometry coords (1/8 px).
pub const QUANT_PER_PX: f64 = 8.0;

/// A single canvas object as the renderer sees it. Geometry is a path-string
/// (`geometry_d`) parsed on demand; style is inline, no `styleKey`/palette lookup.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderObject {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub order: String,
    /// 3x3 projective transform, row-major. `screen = camera · M · local`.
    pub transform: [[f64; 3]; 3],
    /// SVG-subset path-string (M/L/C/Z); object-local quantized i32 coords.
    #[serde(rename = "geometryD")]
    pub geometry_d: String,
    #[serde(default)]
    pub fill: Option<RFill>,
    #[serde(default)]
    pub stroke: Option<RStroke>,
    #[serde(default)]
    pub text: Option<RText>,
    /// Per-node attachments binding this object's geometry nodes to a target. The
    /// bindings graph inverts these into Reproject edges so a moved target
    /// reprojects its followers.
    #[serde(default)]
    pub anchors: Vec<RAnchor>,
    /// Figma-style clip flag: children render clipped to this object's region/bounds.
    #[serde(default)]
    pub clip: bool,
}

/// Per-node attachment binding node `node_index` to `target` at target-local `at`.
/// Wire: `{ nodeIndex, target, at: { x, y } }`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RAnchor {
    pub node_index: usize,
    pub target: String,
    pub at: RLocalPoint,
}

/// A target-local attachment point in object-local pixels.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RLocalPoint {
    pub x: f64,
    pub y: f64,
}

/// Fill paint applied to the derived region, below stroke.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RFill {
    pub paint: RPaint,
    /// Fill opacity in 0..=1; defaults to fully opaque.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

/// Stroke for the outline / drawn line: paint + width + opacity + dash/cap/join.
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

/// Paint source for fill or stroke; colors inline, no palette indirection.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RPaint {
    Solid {
        color: String,
    },
    /// Kebab-case theme token resolved to RGBA at draw time (see
    /// [`crate::object_theme`]). Wire: `{"kind":"token","name":"<kebab>"}`.
    Token {
        name: String,
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

/// Text content positioned relative to the derived region: runs + align.
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
    #[serde(alias = "left")]
    Start,
    Center,
    #[serde(alias = "right")]
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

/// The renderer-core scene view of the object substrate.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderObjectScene {
    pub scene_id: String,
    pub camera: CameraState,
    pub objects: Vec<RenderObject>,
    #[serde(default)]
    pub selection: Option<String>,
    /// Transient, shell-owned, never persisted — but the shell sends it on the live
    /// wire (`multiSelect`), so it must deserialize; the draw path reads it.
    #[serde(default, rename = "multiSelect")]
    pub multi_select: Vec<String>,
}

impl RenderObjectScene {
    /// Project a canonical [`ObjectScene`] into the renderer-core view. Owns the
    /// identity-transform default, the selection/multi-select flattening, the
    /// anchors/clip defaults, and the stroke-width/dash de-quantization the shell
    /// hand-built in TS. `selection` is the live (transient) selection the shell
    /// drives, not `scene.selection`. `scene_id` is the renderer's stable scene
    /// tag (`object-scene-v{scene_version}` in the shell).
    pub fn from_object_scene(
        scene: &ObjectScene,
        camera: CameraState,
        selection: &ObjectSelection,
        scene_id: impl Into<String>,
    ) -> Self {
        let (selection, multi_select) = match selection {
            ObjectSelection::Object { id } => (Some(id.clone()), Vec::new()),
            ObjectSelection::Multi { ids } => (None, ids.clone()),
            ObjectSelection::Canvas => (None, Vec::new()),
        };
        RenderObjectScene {
            scene_id: scene_id.into(),
            camera,
            objects: scene.objects.iter().map(render_object_from).collect(),
            selection,
            multi_select,
        }
    }
}

/// De-quantize a quantized geometry length into logical px (matches the shell's
/// `/ GEOMETRY_QUANTUM_PER_PX`).
fn dequantize(quantized: i32) -> f64 {
    f64::from(quantized) / f64::from(GEOMETRY_QUANTUM_PER_PX)
}

fn render_object_from(object: &Object) -> RenderObject {
    RenderObject {
        id: object.id.clone(),
        parent: object.parent.clone(),
        order: object.order.clone(),
        transform: object.transform.m,
        geometry_d: object.geometry.path_string.clone(),
        fill: object.fill.as_ref().map(rfill_from),
        stroke: object.stroke.as_ref().map(rstroke_from),
        text: object.text.as_ref().map(rtext_from),
        anchors: object.anchors.iter().map(ranchor_from).collect(),
        clip: object.clip.unwrap_or(false),
    }
}

fn rpaint_from(paint: &model::Paint) -> RPaint {
    match paint {
        model::Paint::Solid { color } => RPaint::Solid { color: color.clone() },
        model::Paint::Gradient { stops, angle } => RPaint::Gradient {
            stops: stops
                .iter()
                .map(|s| RGradientStop { offset: s.offset, color: s.color.clone() })
                .collect(),
            angle: *angle,
        },
        model::Paint::Image { content_ref } => {
            RPaint::Image { content_ref: content_ref.clone() }
        }
        model::Paint::Token { name } => RPaint::Token { name: name.clone() },
    }
}

fn rfill_from(fill: &model::Fill) -> RFill {
    RFill { paint: rpaint_from(&fill.paint), opacity: fill.opacity }
}

/// Stroke width and dash run lengths are stored quantized in the model; the
/// renderer treats `RStroke.width`/`dash` as logical px, so de-quantize both —
/// the lone style field the renderer does not de-quantize at draw time.
fn rstroke_from(stroke: &model::Stroke) -> RStroke {
    RStroke {
        paint: rpaint_from(&stroke.paint),
        width: dequantize(stroke.width),
        opacity: stroke.opacity,
        dash: stroke.dash.iter().map(|d| dequantize(*d)).collect(),
        cap: match stroke.cap {
            model::LineCap::Butt => RStrokeCap::Butt,
            model::LineCap::Round => RStrokeCap::Round,
            model::LineCap::Square => RStrokeCap::Square,
        },
        join: match stroke.join {
            model::LineJoin::Miter => RStrokeJoin::Miter,
            model::LineJoin::Round => RStrokeJoin::Round,
            model::LineJoin::Bevel => RStrokeJoin::Bevel,
        },
    }
}

fn rtext_from(text: &model::Text) -> RText {
    RText {
        runs: text
            .runs
            .iter()
            .map(|run| RTextRun {
                text: run.text.clone(),
                // Absent color/size in the model resolve to the same structural
                // defaults the renderer's serde fills in for the raw passthrough.
                color: run.color.clone().unwrap_or_else(default_text_color),
                size: run.size.map(f64::from).unwrap_or_else(default_text_size),
                bold: run.bold,
                italic: run.italic,
                font: run.font.clone().unwrap_or_default(),
            })
            .collect(),
        align: match text.align {
            model::TextAlign::Start => RTextAlign::Start,
            model::TextAlign::Center => RTextAlign::Center,
            model::TextAlign::End => RTextAlign::End,
            model::TextAlign::Justify => RTextAlign::Justify,
        },
        valign: match text.valign {
            model::TextVAlign::Top => RTextValign::Top,
            model::TextVAlign::Middle => RTextValign::Middle,
            model::TextVAlign::Bottom => RTextValign::Bottom,
        },
    }
}

fn ranchor_from(anchor: &model::Anchor) -> RAnchor {
    RAnchor {
        node_index: usize::try_from(anchor.node_index).unwrap_or(0),
        target: anchor.target.clone(),
        at: RLocalPoint { x: f64::from(anchor.at.x), y: f64::from(anchor.at.y) },
    }
}

/// A geometry node in object-local quantized i32 coords. Handles are stored
/// relative to the node, so a node with no handles is a straight (polyline) corner.
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

/// One subpath (contour): a node list plus a closed flag. Multiple subpaths form
/// even-odd holes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RSubPath {
    pub closed: bool,
    pub nodes: Vec<RNode>,
}

/// Parse an SVG-subset path-string (`M`/`L`/`C`/`Z`) into subpaths. For `C`, the
/// absolute control points become the previous node's `out_handle` and this node's
/// `in_handle`, stored relative. Coords are absolute quantized i32; only the
/// uppercase (absolute) forms are accepted.
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

    fn next_command(&mut self) -> Result<Option<char>, String> {
        self.skip_separators();
        match self.rest.chars().next() {
            None => Ok(None),
            Some(c) if c.is_ascii_alphabetic() => {
                self.rest = &self.rest[c.len_utf8()..];
                // Verbatim: lowercase (relative) forms fall through to the caller's
                // unsupported-command error.
                Ok(Some(c))
            }
            Some(c) => Err(format!("expected command, found '{c}'")),
        }
    }

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

/// Structural default fill applied when an object omits `fill`.
pub fn default_fill() -> RFill {
    RFill {
        paint: RPaint::Solid {
            color: "#ffffff".to_string(),
        },
        opacity: 1.0,
    }
}

/// Structural default stroke applied when an object omits `stroke`.
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

pub const FOCUS_RING_COLOR: &str = "#2f7ee6";
/// Focus-ring width in logical pixels.
pub const FOCUS_RING_WIDTH: f64 = 4.0;

/// Visual interaction state for an object; renderer-owned.
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

/// The fully resolved draw style: inline style over structural defaults, plus any
/// focus ring.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    pub fill: RFill,
    pub stroke: RStroke,
    pub focus_ring: Option<FocusRing>,
}

/// Resolve an object's draw style: inline `fill`/`stroke` over structural defaults,
/// plus a focus ring when selected or focused. Hover carries no style change yet.
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

/// Default font size in WIRE-quantized units (8 units/px), so a defaulted run is
/// 16px after the layout de-quant (`/QUANT_PER_PX`). Returning raw px would de-quant
/// to 2px.
fn default_text_size() -> f64 {
    16.0 * QUANT_PER_PX
}

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
        // abs control2 (90,-20) -> end.in_handle = (90-100, -20-0) = (-10, -20)
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
            anchors: Vec::new(),
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
            anchors: Vec::new(),
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
            anchors: Vec::new(),
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
        // Minimal text shape (runs without color/size) + stroke; the feed must tolerate it.
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
        // `size` is wire-quantized (8 u/px): 16px * 8 = 128, de-quantizing to 16px.
        assert_eq!(run.size, 16.0 * QUANT_PER_PX);
        assert_eq!(obj.stroke.as_ref().expect("stroke present").width, 8.0);
    }

    #[test]
    fn quantized_coords_to_pixels() {
        assert_eq!(80.0 / QUANT_PER_PX, 10.0);
    }

    fn identity_transform() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    #[test]
    fn multi_select_deserializes_from_the_wire() {
        let json = r##"{
            "sceneId": "s1",
            "camera": { "x": 0, "y": 0, "zoom": 1 },
            "objects": [],
            "multiSelect": ["a", "b"]
        }"##;
        let scene: RenderObjectScene =
            serde_json::from_str(json).expect("scene with multiSelect deserializes");
        assert_eq!(scene.multi_select, vec!["a".to_string(), "b".to_string()]);
    }

    // Golden: `from_object_scene` must emit the BYTE-IDENTICAL `RenderObjectScene`
    // wire the shell's TS `objectSceneToRenderObjectScene` produced — identity
    // transform default, selection/multiSelect flatten, anchors/clip defaults, and
    // stroke width/dash de-quantized to logical px (the lone style field the
    // renderer does not de-quantize at draw time).
    #[test]
    fn from_object_scene_matches_ts_projection_wire() {
        // Canonical model scene: quantized stroke (width 24 q -> 3px, dash 16/8 q ->
        // 2/1px), a text run WITHOUT color/size (must resolve to renderer defaults),
        // an anchor, clip true, and an object with NO transform (identity default).
        let scene_json = r##"{
            "sceneVersion": 7,
            "objects": [
                {
                    "id": "o1",
                    "order": "a0",
                    "geometry": { "d": "M 0 0 L 80 0 L 80 40 L 0 40 Z" },
                    "stroke": {
                        "paint": { "kind": "solid", "color": "#283644" },
                        "width": 24,
                        "dash": [16, 8],
                        "cap": "round",
                        "join": "bevel"
                    },
                    "text": { "runs": [{ "text": "Note" }], "align": "center", "valign": "bottom" },
                    "anchors": [{ "nodeIndex": 2, "target": "o2", "at": { "x": 10, "y": 20 } }],
                    "clip": true
                }
            ]
        }"##;
        let scene: ObjectScene =
            serde_json::from_str(scene_json).expect("canonical model scene deserializes");

        let selection = ObjectSelection::Multi {
            ids: vec!["o1".to_string(), "o2".to_string()],
        };
        let camera = CameraState { x: 5.0, y: -3.0, zoom: 2.0 };
        let projected = RenderObjectScene::from_object_scene(
            &scene,
            camera,
            &selection,
            format!("object-scene-v{}", scene.scene_version),
        );

        // The exact wire the TS projection emits: identity transform, geometryD,
        // stroke in logical px, text run defaulted (color #111111, size 16*8=128),
        // anchor with f64 `at`, clip true, and the Multi selection flattened to
        // `selection: null` + `multiSelect: [...]`.
        let expected: serde_json::Value = serde_json::json!({
            "sceneId": "object-scene-v7",
            "camera": { "x": 5.0, "y": -3.0, "zoom": 2.0 },
            "selection": null,
            "multiSelect": ["o1", "o2"],
            "objects": [
                {
                    "id": "o1",
                    "parent": null,
                    "order": "a0",
                    "transform": [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],
                    "geometryD": "M 0 0 L 80 0 L 80 40 L 0 40 Z",
                    "fill": null,
                    "stroke": {
                        "paint": { "kind": "solid", "color": "#283644" },
                        "width": 3.0,
                        "opacity": 1.0,
                        "dash": [2.0, 1.0],
                        "cap": "round",
                        "join": "bevel"
                    },
                    "text": {
                        "runs": [{
                            "text": "Note",
                            "color": "#111111",
                            "size": 128.0,
                            "bold": false,
                            "italic": false,
                            "font": ""
                        }],
                        "align": "center",
                        "valign": "bottom"
                    },
                    "anchors": [{ "nodeIndex": 2, "target": "o2", "at": { "x": 10.0, "y": 20.0 } }],
                    "clip": true
                }
            ]
        });

        let actual = serde_json::to_value(&projected).expect("projected scene serializes");
        assert_eq!(actual, expected);
        // Stroke is the load-bearing de-quant: prove logical px, not raw quantized.
        assert_eq!(projected.objects[0].stroke.as_ref().unwrap().width, 3.0);
    }

    // Selection flatten: a single-object selection becomes `selection: Some(id)` with
    // an empty multiSelect; Canvas becomes neither.
    #[test]
    fn from_object_scene_flattens_selection_variants() {
        let scene: ObjectScene = serde_json::from_str(r##"{ "objects": [] }"##).unwrap();
        let cam = CameraState { x: 0.0, y: 0.0, zoom: 1.0 };

        let single = RenderObjectScene::from_object_scene(
            &scene,
            cam.clone(),
            &ObjectSelection::Object { id: "o9".to_string() },
            "s",
        );
        assert_eq!(single.selection.as_deref(), Some("o9"));
        assert!(single.multi_select.is_empty());

        let canvas =
            RenderObjectScene::from_object_scene(&scene, cam, &ObjectSelection::Canvas, "s");
        assert_eq!(canvas.selection, None);
        assert!(canvas.multi_select.is_empty());
    }
}
