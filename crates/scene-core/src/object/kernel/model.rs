//! The single `object` substrate. Every shape, scribble, edge, group, and text
//! node is an [`Object`] — no shape/type discriminant; one `geometry` value plus
//! optional style/text/anchors/layout expresses all of them.
//!
//! Conventions: serde camelCase to match the wire; platform-pure; pointer-width-
//! agnostic (coords i32, codes i64/u64; no `usize` in serialized/addressing
//! fields); no lossy `as` casts.
//!
//! Geometry layering: the canonical at-rest + wire encoding is an SVG-subset
//! path-string (`d`); the parsed contour list (`subpaths`) is the runtime form,
//! never serialized. Coordinates are object-local quantized integers at
//! [`GEOMETRY_QUANTUM_PER_PX`] units per logical pixel.

use serde::{Deserialize, Serialize};

/// Stable string id — never an array index that could shift, which is why nodes
/// are index-addressed *within* one geometry.
pub type ObjectId = String;

pub type ObjectMeta = serde_json::Map<String, serde_json::Value>;

/// Quantized units per 1.0 logical px. Global (not per-canvas) so storage +
/// golden vectors stay stable; i32 so the schema is bit-exact across Rust/wasm.
pub const GEOMETRY_QUANTUM_PER_PX: i32 = 8;

/// Bezier handle offset, object-local quantized i32 relative to its owning node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct HandlePoint {
    pub dx: i32,
    pub dy: i32,
}

/// Index-addressable within its subpath (the node index is the addressing key
/// for edit-geometry / anchors — never a separate node id). Handles absent =>
/// straight segment, present => cubic bezier control points relative to the node.
/// Coords object-local quantized i32.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    pub x: i32,
    pub y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_handle: Option<HandlePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_handle: Option<HandlePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
}

impl PathNode {
    pub fn corner(x: i32, y: i32) -> Self {
        PathNode { x, y, in_handle: None, out_handle: None, width: None }
    }
}

/// One contour. `closed` + presence-of-handles + multi-subpath define topology;
/// there is no shape/type discriminant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct SubPath {
    pub closed: bool,
    pub nodes: Vec<PathNode>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum FillRule {
    #[default]
    EvenOdd,
    NonZero,
}

/// The serialized form carries the path-string `d` + fill rule only; `subpaths`
/// is the parsed runtime mirror, reconstructed by `ensure_parsed`/`parse`. The
/// path-string is the single source of truth at rest + on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    /// Path-string (M/L/C/Z, multi-subpath); absolute integer coordinates in
    /// object-local quantized units.
    #[serde(rename = "d", default, skip_serializing_if = "String::is_empty")]
    pub path_string: String,
    #[serde(default)]
    pub fill_rule: FillRule,
    #[serde(skip)]
    pub subpaths: Vec<SubPath>,
}

impl Geometry {
    pub fn from_subpaths(subpaths: Vec<SubPath>, fill_rule: FillRule) -> Self {
        let path_string = path_string::serialize(&subpaths);
        Geometry { path_string, fill_rule, subpaths }
    }

    /// Parse `path_string` into `subpaths`, replacing any current parse. `Err` on
    /// malformed input; never panics.
    pub fn parse(&mut self) -> Result<(), String> {
        self.subpaths = path_string::parse(&self.path_string)?;
        Ok(())
    }

    /// Post-deserialize hydration: populate `subpaths` from `path_string` when
    /// currently empty.
    pub fn ensure_parsed(&mut self) -> Result<(), String> {
        if self.subpaths.is_empty() && !self.path_string.is_empty() {
            self.parse()?;
        }
        Ok(())
    }

    /// Re-encode `path_string` after editing the parsed form.
    pub fn reencode(&mut self) {
        self.path_string = path_string::serialize(&self.subpaths);
    }
}

/// SVG-subset path-string codec. Grammar: `M x y`, `L x y`, `C x1 y1 x2 y2 x y`
/// (absolute cubic), `Z`. Integer coordinates in object-local quantized units.
/// Bezier handles are stored relative to their node; the codec converts to/from
/// SVG's absolute control points.
pub mod path_string {
    use super::{HandlePoint, PathNode, SubPath};
    use core::fmt::Write as _;

    pub fn serialize(subpaths: &[SubPath]) -> String {
        let mut out = String::new();
        for sp in subpaths {
            if sp.nodes.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            let first = &sp.nodes[0];
            let _ = write!(out, "M {} {}", first.x, first.y);
            for win in sp.nodes.windows(2) {
                emit_segment(&mut out, &win[0], &win[1]);
            }
            if sp.closed {
                if let (Some(last), Some(first)) = (sp.nodes.last(), sp.nodes.first()) {
                    // Closing segment carries its curve when endpoints define handles.
                    if last.out_handle.is_some() || first.in_handle.is_some() {
                        emit_segment(&mut out, last, first);
                    }
                }
                out.push_str(" Z");
            }
        }
        out
    }

    fn emit_segment(out: &mut String, from: &PathNode, to: &PathNode) {
        match (from.out_handle, to.in_handle) {
            (None, None) => {
                let _ = write!(out, " L {} {}", to.x, to.y);
            }
            (out_h, in_h) => {
                let c1x = from.x + out_h.map_or(0, |h| h.dx);
                let c1y = from.y + out_h.map_or(0, |h| h.dy);
                let c2x = to.x + in_h.map_or(0, |h| h.dx);
                let c2y = to.y + in_h.map_or(0, |h| h.dy);
                let _ = write!(out, " C {c1x} {c1y} {c2x} {c2y} {} {}", to.x, to.y);
            }
        }
    }

    /// Tolerates extra whitespace and commas; rejects unknown commands and short
    /// arg lists.
    pub fn parse(s: &str) -> Result<Vec<SubPath>, String> {
        let mut tokens = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .peekable();
        let mut subpaths: Vec<SubPath> = Vec::new();
        let mut current: Option<SubPath> = None;

        fn num(t: Option<&str>) -> Result<i32, String> {
            t.ok_or_else(|| "unexpected end of path".to_string())?
                .parse::<i32>()
                .map_err(|e| format!("bad coordinate: {e}"))
        }

        while let Some(tok) = tokens.next() {
            match tok {
                "M" | "m" => {
                    if let Some(sp) = current.take() {
                        subpaths.push(sp);
                    }
                    let x = num(tokens.next())?;
                    let y = num(tokens.next())?;
                    current = Some(SubPath { closed: false, nodes: vec![PathNode::corner(x, y)] });
                }
                "L" | "l" => {
                    let x = num(tokens.next())?;
                    let y = num(tokens.next())?;
                    let sp = current.as_mut().ok_or("L before M")?;
                    sp.nodes.push(PathNode::corner(x, y));
                }
                "C" | "c" => {
                    let c1x = num(tokens.next())?;
                    let c1y = num(tokens.next())?;
                    let c2x = num(tokens.next())?;
                    let c2y = num(tokens.next())?;
                    let x = num(tokens.next())?;
                    let y = num(tokens.next())?;
                    let sp = current.as_mut().ok_or("C before M")?;
                    if let Some(prev) = sp.nodes.last_mut() {
                        prev.out_handle = Some(HandlePoint { dx: c1x - prev.x, dy: c1y - prev.y });
                    }
                    sp.nodes.push(PathNode {
                        x,
                        y,
                        in_handle: Some(HandlePoint { dx: c2x - x, dy: c2y - y }),
                        out_handle: None,
                        width: None,
                    });
                }
                "Z" | "z" => {
                    let sp = current.as_mut().ok_or("Z before M")?;
                    sp.closed = true;
                }
                other => return Err(format!("unsupported path command: {other}")),
            }
        }
        if let Some(sp) = current.take() {
            subpaths.push(sp);
        }
        Ok(subpaths)
    }
}

/// Row-major 3x3 projective matrix `[[a,b,c],[d,e,f],[g,h,i]]`; affine is the
/// case `g=h=0,i=1`. Pipeline: `screen = camera · m · warp(local)`. f64 so
/// hit-test inverse + perspective divide stay numerically faithful.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Transform3x3 {
    pub m: [[f64; 3]; 3],
}

impl Default for Transform3x3 {
    fn default() -> Self {
        Transform3x3 { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }
    }
}

impl Transform3x3 {
    pub const IDENTITY: Transform3x3 =
        Transform3x3 { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };

    /// Pure translation in logical px.
    pub fn translate(tx: f64, ty: f64) -> Self {
        Transform3x3 { m: [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]] }
    }

    pub fn mul(&self, rhs: &Transform3x3) -> Transform3x3 {
        let a = &self.m;
        let b = &rhs.m;
        let mut out = [[0.0f64; 3]; 3];
        for (r, out_row) in out.iter_mut().enumerate() {
            for (c, cell) in out_row.iter_mut().enumerate() {
                *cell = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            }
        }
        Transform3x3 { m: out }
    }

    /// Object-local point -> world point (perspective divide included). I/O in
    /// logical px (not quantized).
    pub fn apply_point(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.m;
        let wx = m[0][0] * x + m[0][1] * y + m[0][2];
        let wy = m[1][0] * x + m[1][1] * y + m[1][2];
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        if w.abs() > f64::EPSILON {
            (wx / w, wy / w)
        } else {
            (wx, wy)
        }
    }
}

/// FFD warp grid slot (nonlinear bend/envelope/text-on-path), implementation
/// deferred — control point grid in object-local quantized units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Warp {
    pub cols: i32,
    pub rows: i32,
    /// `cols*rows` control points, row-major, object-local quantized i32.
    pub points: Vec<PathNode>,
}

fn default_opacity() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Paint {
    Solid { color: String },
    Gradient { stops: Vec<GradientStop>, angle: f64 },
    /// `contentRef` is a content-addressed handle resolved out-of-band.
    Image { content_ref: String },
    /// `name` is a kebab-case token id from [`crate::object::theme`]; RGBA
    /// resolution is deferred to the renderer (light/dark aware).
    Token { name: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub offset: f64,
    pub color: String,
}

/// Paint applied to the derived region (render order: fill below stroke).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub paint: Paint,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub paint: Paint,
    /// Default width in quantized units (per-node `PathNode.width` wins).
    pub width: i32,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    /// Dash on/off run lengths in quantized units; empty => solid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<i32>,
    #[serde(default)]
    pub cap: LineCap,
    #[serde(default)]
    pub join: LineJoin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum TextVAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Quantized units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i32>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

/// Runs array (styled segments) positioned relative to the derived region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub runs: Vec<TextRun>,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub valign: TextVAlign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct LocalPoint {
    pub x: i32,
    pub y: i32,
}

/// Per-node attachment. `node_index` addresses a node in *this* object's
/// geometry; `target` is another object's id; `at` is a local point on the
/// target's derived outline (re-projected when the target's geometry edits).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub node_index: i32,
    pub target: ObjectId,
    pub at: LocalPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum LayoutDirection {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum LayoutAlign {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum LayoutSizing {
    Hug,
    Fixed,
    Fill,
}

/// Auto-layout inputs on a children group; output positions are derived (not
/// stored/synced).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub direction: LayoutDirection,
    pub gap: i32,
    pub padding: i32,
    pub align: LayoutAlign,
    pub sizing: LayoutSizing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommentAnchor {
    Node { node_index: i32 },
    Point { at: LocalPoint },
}

/// Comment on an object, optionally anchored to a node or local point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<CommentAnchor>,
    #[serde(default)]
    pub resolved: bool,
}

/// `kind` stays an open string and `contentRef` a content-addressed handle so
/// adding behavior later is non-breaking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ContentEmbed {
    pub kind: String,
    pub content_ref: String,
}

/// The one canvas primitive. Optional fields use `skip_serializing_if` so an
/// empty object is minimal on the wire and at rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Object {
    pub id: ObjectId,
    /// `None` = canvas root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ObjectId>,
    /// Fractional z-order key (base-62, `fractional.rs`); sorts by plain str Ord.
    pub order: String,
    // `Transform3x3` is `#[serde(transparent)]` (a bare array on the wire); ts-rs
    // ignores `transparent` and can't impl `TS`, so emit the matrix AS its inner
    // `[[f64; 3]; 3]`.
    #[serde(default)]
    #[cfg_attr(feature = "ts-gen", ts(as = "[[f64; 3]; 3]"))]
    pub transform: Transform3x3,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp: Option<Warp>,
    pub geometry: Geometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    /// Edges live here, not as a separate type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// Clip children to this object's region/bounds (Figma frame clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    /// Tag ids; name/color registry lives in `ObjectScene.tags`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_of: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentEmbed>,
    // Typed as the matching index signature rather than dragging ts-rs's
    // `JsonValue` into the surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts-gen", ts(type = "Record<string, unknown>"))]
    pub meta: Option<ObjectMeta>,
}

impl Object {
    /// Minimal object: id + order + geometry, identity transform, no style.
    pub fn new(id: impl Into<ObjectId>, order: impl Into<String>, geometry: Geometry) -> Self {
        Object {
            id: id.into(),
            parent: None,
            order: order.into(),
            transform: Transform3x3::default(),
            warp: None,
            geometry,
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            layout: None,
            clip: None,
            comments: Vec::new(),
            tags: Vec::new(),
            component_of: None,
            content: None,
            meta: None,
        }
    }

    pub fn ensure_parsed(&mut self) -> Result<(), String> {
        self.geometry.ensure_parsed()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct TagDef {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// `multi` is ephemeral shell-only; the canonical persisted selection is single.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ObjectSelection {
    Canvas,
    Object { id: ObjectId },
    Multi { ids: Vec<ObjectId> },
}

impl Default for ObjectSelection {
    fn default() -> Self {
        ObjectSelection::Canvas
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ObjectScene {
    // i64 on the wire is a plain JSON number, not a `bigint` (ts-rs default).
    #[serde(default)]
    #[cfg_attr(feature = "ts-gen", ts(type = "number"))]
    pub scene_version: i64,
    /// Insertion order preserved; canonical paint order is `order` (fractional).
    #[serde(default)]
    pub objects: Vec<Object>,
    #[serde(default)]
    pub tags: Vec<TagDef>,
    #[serde(default)]
    pub selection: ObjectSelection,
    #[serde(default)]
    pub updated_at: String,
}

impl ObjectScene {
    pub fn get(&self, id: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn ensure_parsed(&mut self) -> Result<(), String> {
        for o in &mut self.objects {
            o.ensure_parsed()?;
        }
        Ok(())
    }
}
