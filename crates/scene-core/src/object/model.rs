//! OB1.1 keystone — the single `object` substrate (D1/P2).
//!
//! Every shape, scribble, edge, group, and text node on the canvas is an
//! [`Object`]. There is no shape/type discriminant: a single `geometry` value
//! (a path substrate) plus optional style/text/anchors/layout expresses all of
//! them. This module is authored alongside the legacy `model.rs` during the
//! OB-1..OB-3 parallel run; the model cutover (OB4.1) swaps consumers over and
//! deletes `SceneGroup/SceneNode/SceneEdge`.
//!
//! Conventions (CLAUDE.md): serde camelCase to match the wire; platform-pure
//! (no time/rng/IO); pointer-width-agnostic (no `usize` in any serialized or
//! addressing field — coords are i32, codes i64/u64); strict workspace lints
//! (no lossy `as` casts).
//!
//! **Geometry layering (D2/D9/D11):** the canonical at-rest + wire encoding is
//! an SVG-subset path-string (`d`); the parsed contour list (`subpaths`) is the
//! runtime form and is never serialized (`#[serde(skip)]`). Coordinates inside
//! the string are object-local quantized integers at [`GEOMETRY_QUANTUM_PER_PX`]
//! units per logical pixel.

use serde::{Deserialize, Serialize};

/// Stable string id. Objects, tags, comments, and anchor targets all use string
/// ids (matches the existing model.rs / `Record.id`); never an array index that
/// could shift, which is why nodes are index-addressed *within* one geometry.
pub type ObjectId = String;

/// Free-form metadata bag, preserved verbatim (mirrors legacy `ObjectMeta`).
pub type ObjectMeta = serde_json::Map<String, serde_json::Value>;

// ---------------------------------------------------------------------------
// D2 — geometry: single path substrate, object-local quantized i32 @ 1/8px.
// ---------------------------------------------------------------------------

/// Quantization unit: this many quantized units == 1.0 logical px. Global const
/// (not per-canvas) per the locked decision — keeps storage + golden vectors
/// stable. Stored coords are i32 so the schema is pointer-width-agnostic and
/// bit-exact across Rust/wasm/golden.
pub const GEOMETRY_QUANTUM_PER_PX: i32 = 8;

/// Bezier handle offset, object-local quantized i32 relative to its owning node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlePoint {
    pub dx: i32,
    pub dy: i32,
}

/// One path node, **index-addressable within its subpath** (the node index is
/// the addressing key for edit-geometry / anchors — never a separate node id).
/// `in_handle`/`out_handle` absent => straight segment (polyline/sketch),
/// present => cubic bezier control points relative to the node. `width` is the
/// per-node stroke/pressure slot (D4/D13, behavior deferred). Coords object-local
/// quantized i32 (D2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// A corner node (straight segments on both sides).
    pub fn corner(x: i32, y: i32) -> Self {
        PathNode { x, y, in_handle: None, out_handle: None, width: None }
    }
}

/// One contour. `closed` + presence-of-handles + multi-subpath define topology;
/// there is no shape/type discriminant (P2/D2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubPath {
    pub closed: bool,
    pub nodes: Vec<PathNode>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillRule {
    #[default]
    EvenOdd,
    NonZero,
}

/// Geometry (D2/D9). The serialized form carries the SVG-subset path-string `d`
/// and the fill rule only; `subpaths` is the parsed runtime mirror and is
/// reconstructed by [`Geometry::ensure_parsed`] / [`Geometry::parse`]. This
/// makes the path-string the single source of truth at rest + on the wire
/// (no duality / divergence risk).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    /// At-rest / wire SVG-subset path-string (M/L/C/Z, multi-subpath). Absolute
    /// integer coordinates in object-local quantized units (D2).
    #[serde(rename = "d", default, skip_serializing_if = "String::is_empty")]
    pub path_string: String,
    #[serde(default)]
    pub fill_rule: FillRule,
    /// Parsed runtime contours — never serialized; derived from `path_string`.
    #[serde(skip)]
    pub subpaths: Vec<SubPath>,
}

impl Geometry {
    /// Build geometry from runtime contours, encoding the canonical path-string.
    pub fn from_subpaths(subpaths: Vec<SubPath>, fill_rule: FillRule) -> Self {
        let path_string = path_string::serialize(&subpaths);
        Geometry { path_string, fill_rule, subpaths }
    }

    /// Parse `path_string` into `subpaths`, replacing any current parse. Returns
    /// an error string on malformed input (never panics).
    pub fn parse(&mut self) -> Result<(), String> {
        self.subpaths = path_string::parse(&self.path_string)?;
        Ok(())
    }

    /// Populate `subpaths` from `path_string` if it is currently empty but a
    /// path-string is present (the post-deserialize hydration step).
    pub fn ensure_parsed(&mut self) -> Result<(), String> {
        if self.subpaths.is_empty() && !self.path_string.is_empty() {
            self.parse()?;
        }
        Ok(())
    }

    /// Re-encode `path_string` from the current `subpaths` (call after editing
    /// the parsed form so the canonical encoding stays in sync).
    pub fn reencode(&mut self) {
        self.path_string = path_string::serialize(&self.subpaths);
    }
}

/// SVG-subset path-string codec (D2). Grammar: `M x y` (moveto, starts a
/// subpath), `L x y` (lineto), `C x1 y1 x2 y2 x y` (absolute cubic bezier),
/// `Z` (close). Coordinates are integers (object-local quantized units). Bezier
/// handles are stored relative to their node; the codec converts to/from the
/// absolute control points SVG expects.
pub mod path_string {
    use super::{HandlePoint, PathNode, SubPath};
    use core::fmt::Write as _;

    /// Encode contours into an SVG-subset path-string.
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
                    // Closing segment back to the first node carries its curve
                    // when the endpoints define handles.
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

    /// Parse an SVG-subset path-string into contours. Tolerates extra
    /// whitespace and commas; rejects unknown commands and short arg lists.
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

// ---------------------------------------------------------------------------
// D7 — transform: 3x3 projective matrix (TRS + shear + perspective), 0-rebake.
// ---------------------------------------------------------------------------

/// Row-major 3x3 projective matrix `[[a,b,c],[d,e,f],[g,h,i]]`; affine is the
/// case `g=h=0,i=1`. Pipeline: `screen = camera · m · warp(local)` (D7). f64 so
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

    /// Matrix product `self · rhs`.
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

    /// Apply to an object-local point, returning a world point (perspective
    /// divide included). Input/output in logical px (not quantized).
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

/// FFD warp grid slot (D7 nonlinear bend/envelope/text-on-path). Schema-present,
/// implementation deferred — control point grid in object-local quantized units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warp {
    pub cols: i32,
    pub rows: i32,
    /// `cols*rows` control points, row-major, object-local quantized i32.
    pub points: Vec<PathNode>,
}

// ---------------------------------------------------------------------------
// D4 — style: fill / stroke / text(runs[]).
// ---------------------------------------------------------------------------

fn default_opacity() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Paint {
    Solid { color: String },
    Gradient { stops: Vec<GradientStop>, angle: f64 },
    /// content/embed channel hedge slot (external file/image). `contentRef` is a
    /// content-addressed handle resolved out-of-band; type-per-kind deferred.
    Image { content_ref: String },
    /// Semantic theme token (D-token contract). `name` is a kebab-case token id
    /// from [`super::theme`]; resolution to RGBA is deferred to the renderer
    /// (light/dark aware) — treated as an opaque color source until then.
    Token { name: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub offset: f64,
    pub color: String,
}

/// Paint applied to the derived region (D6 render order: fill below stroke).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub paint: Paint,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Stroke paints the path outline (open + closed) above fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub paint: Paint,
    /// Default stroke width in quantized units (per-node `PathNode.width` wins).
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
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextVAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Font size in quantized units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i32>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

/// D4 — text is a runs array (styled segments), positioned relative to the
/// derived region (D6). Markdown/silhouette-flow deferred; runs present.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub runs: Vec<TextRun>,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub valign: TextVAlign,
}

// ---------------------------------------------------------------------------
// D5 — anchors: per-node optional attachment; edges are absorbed into this.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPoint {
    pub x: i32,
    pub y: i32,
}

/// Per-node attachment. `node_index` addresses a node in *this* object's
/// geometry; `target` is another object's id; `at` is a local point on the
/// target's derived outline (re-projected when the target's geometry edits).
/// The pair of `target`s on two anchors defines the connection graph (D5/D6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub node_index: i32,
    pub target: ObjectId,
    pub at: LocalPoint,
}

// ---------------------------------------------------------------------------
// D3 / D18 / D20 — children grouping, clip, comments, tags.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutDirection {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutAlign {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutSizing {
    Hug,
    Fixed,
    Fill,
}

/// Auto-layout inputs on a children group (D3 tier-3, OB3.A1). Output positions
/// are derived (not stored/synced); these are the inputs. Schema-present.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub direction: LayoutDirection,
    pub gap: i32,
    pub padding: i32,
    pub align: LayoutAlign,
    pub sizing: LayoutSizing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommentAnchor {
    Node { node_index: i32 },
    Point { at: LocalPoint },
}

/// D20 — comment on an object, optionally anchored to a node or local point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// content/embed channel hedge slot (external file/embed). `kind` stays an open
/// string and `contentRef` a content-addressed handle so adding behavior later
/// is non-breaking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEmbed {
    pub kind: String,
    pub content_ref: String,
}

// ---------------------------------------------------------------------------
// D1 — the single Object. Replaces SceneGroup/SceneNode/SceneEdge entirely.
// ---------------------------------------------------------------------------

/// The one canvas primitive (D1/P2). Optional fields use `skip_serializing_if`
/// so an empty object is minimal on the wire and at rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Object {
    pub id: ObjectId,
    /// Parent object id (children-group containment, D3). `None` = canvas root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ObjectId>,
    /// Fractional z-order key (base-62, `fractional.rs`); sorts by plain str Ord.
    pub order: String,
    #[serde(default)]
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
    /// Per-node attachments (D5). Edges live here, not as a separate type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// D18 — clip children to this object's region/bounds (Figma frame clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    /// D20 — tag ids (name/color registry lives in `ObjectScene.tags`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Hedge: `componentOf` — id of a component this object instantiates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_of: Option<ObjectId>,
    /// Hedge: object-level content/embed channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentEmbed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

    /// Hydrate the parsed geometry after deserialization.
    pub fn ensure_parsed(&mut self) -> Result<(), String> {
        self.geometry.ensure_parsed()
    }
}

// ---------------------------------------------------------------------------
// Scene wrapper (FOLD-IN replace of model.rs Scene): drop the 3 arrays, add
// `objects`. comments + tags are now object fields (D20); the scene keeps a
// canvas-level tag registry (id -> name/color).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDef {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// Selection union (preserves the legacy `SceneSelection` shape: `tag = "kind"`).
/// `multi` is ephemeral shell-only; the canonical persisted selection is single.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[serde(rename_all = "camelCase")]
pub struct ObjectScene {
    #[serde(default)]
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
    /// Find an object by id.
    pub fn get(&self, id: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    /// Mutable lookup by id.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    /// Hydrate every object's parsed geometry (post-deserialize).
    pub fn ensure_parsed(&mut self) -> Result<(), String> {
        for o in &mut self.objects {
            o.ensure_parsed()?;
        }
        Ok(())
    }
}
