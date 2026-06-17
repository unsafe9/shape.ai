//! The retained, declarative widget tree. All coords are SCREEN px
//! (identity-camera space). A `Container` is either absolute (`Axis::None`,
//! P1 back-compat) or flex/stack (`Horizontal`/`Vertical`).

/// Stable widget id; `String` matches the `RenderObject.id` wire type.
pub type WidgetId = String;

/// A paint source: a kebab-case theme token (resolved downstream by the renderer)
/// or a literal `#rrggbb`.
#[derive(Clone, Debug)]
pub enum Paint {
    Token(String),
    Solid(String),
}

/// Text color source: a theme token resolved to hex at render() (RTextRun.color
/// is a FIXED hex per renderer-core, never a token), or a literal `#rrggbb`.
#[derive(Clone, Debug, PartialEq)]
pub enum TextPaint {
    Token(String),
    Hex(String),
}

/// Rect visual style. `corner_radius == 0.0` ⇒ sharp rect, `> 0.0` ⇒ rounded.
#[derive(Clone, Debug)]
pub struct RectStyle {
    pub fill: Option<Paint>,
    /// `(paint, width_px)`.
    pub stroke: Option<(Paint, f64)>,
    pub corner_radius: f64,
    /// Fill opacity in [0,1]. `RPaint::Solid` carries no alpha, so a translucent
    /// SOLID rect (e.g. a soft-shadow underlay) sets this below 1.0.
    pub opacity: f64,
}

impl Default for RectStyle {
    fn default() -> Self {
        RectStyle {
            fill: None,
            stroke: None,
            corner_radius: 0.0,
            opacity: 1.0,
        }
    }
}

/// Main-axis layout direction of a `Container`. `None` == today's absolute layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    None,
    Horizontal,
    Vertical,
}

/// Cross-axis alignment of a flex container's children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CrossAlign {
    Start,
    Center,
    End,
}

/// Main-axis distribution of a flex container's children. `Start` packs them at the
/// leading edge with `spacing` between (today's behavior); `SpaceBetween` pins the
/// first child to the leading edge and the last to the trailing edge, splitting the
/// slack equally between the gaps — the label|value row layout (a left label, a
/// right-pinned control) that otherwise needs `PANEL_W - LABEL_W - …` cursor math.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MainAlign {
    Start,
    SpaceBetween,
}

/// Per-side insets (left/top/right/bottom).
#[derive(Clone, Copy, Debug)]
pub struct Edges {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

impl Edges {
    pub fn all(v: f64) -> Self {
        Edges {
            l: v,
            t: v,
            r: v,
            b: v,
        }
    }
}

/// Interaction-driven visual phase a composite widget picks tokens against. The
/// token choice lives here / in render — never branched in the shell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VisualState {
    Normal,
    Hover,
    Pressed,
    Focused,
}

/// What the shell's IME library (NEXT slice) will fulfil: mount an OS editing
/// surface at this rect with this value/style. ui-architecture decision #5.
#[derive(Clone, Debug, PartialEq)]
pub struct EditRequest {
    pub id: WidgetId,
    pub rect: (f64, f64, f64, f64),
    pub value: String,
    pub size_px: f64,
}

/// A filled/stroked rectangle. `hoverable` marks an interactive body (a borderless
/// icon-button) that takes a `hover` background while pointed at — the projection
/// swaps its resting fill, so ui-core stays prefix-free (the caller, which knows the
/// body is interactive, sets the flag). A plain decorative rect leaves it `false`.
#[derive(Clone, Debug)]
pub struct Rect {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub style: RectStyle,
    pub hoverable: bool,
}

/// A line-art glyph. `d` is an SVG-subset path authored in a 24×24 box
/// (absolute `M`/`L`/`C`/`Z` only); `emit_icon` scales it to the icon box and
/// quantizes. Stroked icons (SF-Symbols feel) carry `stroke = (paint, width_px)`;
/// filled icons carry `fill`. Emits ONE RenderObject (`{id}`).
#[derive(Clone, Debug)]
pub struct Icon {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub d: String,
    pub fill: Option<Paint>,
    /// `(paint, width_px)` — stroke width is in ICON-BOX px (post-scale).
    pub stroke: Option<(Paint, f64)>,
}

/// A non-interactive text label, optionally centered within its box. Folds the
/// "Label" role: `align_center == false` is a left-aligned label.
#[derive(Clone, Debug)]
pub struct Text {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub label: String,
    pub size_px: f64,
    pub color: TextPaint,
    pub align_center: bool,
}

/// A labeled pill: a rounded `Rect` body + a centered `Text`. Emits TWO
/// RenderObjects (`{id}` body, `{id}::label` text) but hit-tests on the body id.
#[derive(Clone, Debug)]
pub struct Button {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub label: String,
    pub style: RectStyle,
    pub label_size_px: f64,
    pub label_color: TextPaint,
}

/// A single color chip; a selection ring when picked. Emits ONE rect (`{id}`).
#[derive(Clone, Debug)]
pub struct Swatch {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub fill: Paint,
    pub selected: bool,
}

/// An on/off pill. Emits a track (`{id}`) + a knob (`{id}::knob`); hits the track.
#[derive(Clone, Debug)]
pub struct Toggle {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub on: bool,
}

/// A horizontal value bar, `value` in [0,1]. Emits track (`{id}`) + filled
/// (`{id}::fill`) + knob (`{id}::knob`); hits the track.
#[derive(Clone, Debug)]
pub struct Slider {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub value: f64,
}

/// A segmented control. Emits a track (`{id}`) + selected-cell fill (`{id}::sel`)
/// + a per-cell centered Text (`{id}::seg{n}::label`); hits the track.
#[derive(Clone, Debug)]
pub struct Segment {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub labels: Vec<String>,
    pub selected: usize,
    pub label_size_px: f64,
    pub label_color: TextPaint,
}

/// A single-line text field. Emits a bordered body (`{id}`) + a left-aligned
/// value/placeholder Text (`{id}::value`) + (focused) an end-of-text caret
/// (`{id}::caret`); hits the body.
#[derive(Clone, Debug)]
pub struct TextInput {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub value: String,
    pub focused: bool,
    pub size_px: f64,
    pub color: TextPaint,
    pub placeholder: String,
}

/// A layout group: `Axis::None` offsets each child by the container origin
/// (P1 absolute), `Horizontal`/`Vertical` flow children with padding/spacing.
#[derive(Clone, Debug)]
pub struct Container {
    pub id: WidgetId,
    pub x: f64,
    pub y: f64,
    /// Explicit own size — needed for cross-align + a predictable hit box.
    pub w: f64,
    pub h: f64,
    /// `Axis::None` == absolute (back-compat).
    pub direction: Axis,
    /// Main-axis gap between children (the gap `MainAlign::Start` packs with; ignored
    /// under `SpaceBetween`, which derives the gap from the slack).
    pub spacing: f64,
    /// Main-axis distribution. `Start` is the default packed layout.
    pub main_align: MainAlign,
    pub padding: Edges,
    /// Cross-axis alignment of children.
    pub align: CrossAlign,
    /// Clip children to this container's box (Figma-style clip-to-region). `false`
    /// is the default for every existing surface; a capped panel that must not let
    /// overflowing sections paint past its bottom edge sets it `true`.
    pub clip: bool,
    pub children: Vec<Widget>,
}

#[derive(Clone, Debug)]
pub enum Widget {
    Container(Container),
    Rect(Rect),
    Icon(Icon),
    Text(Text),
    Button(Button),
    Swatch(Swatch),
    Toggle(Toggle),
    Slider(Slider),
    Segment(Segment),
    TextInput(TextInput),
}
