//! Renderer-side semantic theme token table (RB1, decision D1).
//!
//! The renderer is a SEPARATE crate from `scene-core` and does NOT import it, so
//! this is an intentional MIRROR of the C1 token contract
//! (`scene-core::object::theme`): the exact same kebab-case token names and the
//! exact same light/dark RGBA pairs, held renderer-side so token paints
//! (`RPaint::Token { name }`) resolve at draw time without crossing the crate
//! boundary. Keep the names + values in lock-step with C1 — the wire contract
//! (`{"kind":"token","name":"<kebab>"}`) only holds if both tables agree.
//!
//! Pure: no time, randomness, threads, or I/O — a static light/dark RGBA lookup.
//! The theme toggle is a single `dark: bool` bit (see [`Theme`]); flipping it
//! re-resolves token colors with ZERO re-tessellation (P4).

/// A semantic theme token. Mirrors `scene-core::object::theme::Token`: each
/// variant maps to a stable kebab-case wire name and a light/dark RGBA pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeToken {
    /// Infinite-canvas backdrop behind all objects (canvas clear color).
    CanvasBg,
    /// Raised surface (cards, panels) sitting on the canvas.
    Surface,
    /// Muted/secondary surface (subtle backgrounds, hover wells).
    SurfaceMuted,
    /// Default object fill.
    DefaultFill,
    /// Default object stroke / hairline border.
    DefaultStroke,
    /// Primary text / foreground.
    Text,
    /// Translucent drop-shadow color (RB3 sources its shadow paint here).
    Shadow,
    /// Selection highlight ring (blue accent; AP4 focus/selection chrome).
    SelectionRing,
}

/// Every token, in a stable order (mirrors C1 `ALL_TOKENS`).
pub const ALL_TOKENS: [ThemeToken; 8] = [
    ThemeToken::CanvasBg,
    ThemeToken::Surface,
    ThemeToken::SurfaceMuted,
    ThemeToken::DefaultFill,
    ThemeToken::DefaultStroke,
    ThemeToken::Text,
    ThemeToken::Shadow,
    ThemeToken::SelectionRing,
];

impl ThemeToken {
    /// The kebab-case wire name (matches the `RPaint::Token { name }` / C1 wire).
    pub const fn name(self) -> &'static str {
        match self {
            ThemeToken::CanvasBg => "canvas-bg",
            ThemeToken::Surface => "surface",
            ThemeToken::SurfaceMuted => "surface-muted",
            ThemeToken::DefaultFill => "default-fill",
            ThemeToken::DefaultStroke => "default-stroke",
            ThemeToken::Text => "text",
            ThemeToken::Shadow => "shadow",
            ThemeToken::SelectionRing => "selection-ring",
        }
    }

    /// Parse a kebab-case wire name back into a [`ThemeToken`], or `None`.
    pub fn from_name(name: &str) -> Option<ThemeToken> {
        ALL_TOKENS.into_iter().find(|t| t.name() == name)
    }

    /// The RGBA (`[r, g, b, a]`, 0..=255) this token resolves to in the given
    /// mode. Values are byte-identical to C1 (`scene-core::object::theme`).
    pub const fn rgba(self, dark: bool) -> [u8; 4] {
        match (self, dark) {
            (ThemeToken::CanvasBg, false) => [0xf5, 0xf5, 0xf7, 0xff],
            (ThemeToken::CanvasBg, true) => [0x1e, 0x1e, 0x20, 0xff],

            (ThemeToken::Surface, false) => [0xff, 0xff, 0xff, 0xff],
            (ThemeToken::Surface, true) => [0x2c, 0x2c, 0x2e, 0xff],

            (ThemeToken::SurfaceMuted, false) => [0xe9, 0xe9, 0xeb, 0xff],
            (ThemeToken::SurfaceMuted, true) => [0x3a, 0x3a, 0x3c, 0xff],

            (ThemeToken::DefaultFill, false) => [0xff, 0xff, 0xff, 0xff],
            (ThemeToken::DefaultFill, true) => [0x2c, 0x2c, 0x2e, 0xff],

            (ThemeToken::DefaultStroke, false) => [0xc6, 0xc6, 0xc8, 0xff],
            (ThemeToken::DefaultStroke, true) => [0x54, 0x54, 0x56, 0xff],

            (ThemeToken::Text, false) => [0x1d, 0x1d, 0x1f, 0xff],
            (ThemeToken::Text, true) => [0xf5, 0xf5, 0xf7, 0xff],

            (ThemeToken::Shadow, false) => [0x00, 0x00, 0x00, 0x55],
            (ThemeToken::Shadow, true) => [0xff, 0xff, 0xff, 0xa8],

            (ThemeToken::SelectionRing, false) => [0x00, 0x7a, 0xff, 0xff],
            (ThemeToken::SelectionRing, true) => [0x0a, 0x84, 0xff, 0xff],
        }
    }

    /// The token's RGBA as renderer floats (`[f32; 4]`, each channel 0..=1) in
    /// the given mode — the form the GPU paint slot / clear color consume.
    pub fn rgba_f32(self, dark: bool) -> [f32; 4] {
        u8_to_f32(self.rgba(dark))
    }
}

/// Resolve a kebab-case token `name` to its RGBA (`[u8; 4]`) in the given mode,
/// or `None` for an unknown name (mirrors C1 `resolve_token`).
pub fn resolve_token(name: &str, dark: bool) -> Option<[u8; 4]> {
    ThemeToken::from_name(name).map(|t| t.rgba(dark))
}

/// Resolve a kebab-case token `name` to renderer floats (`[f32; 4]`, 0..=1) in
/// the given mode, or `None` for an unknown name.
pub fn resolve_token_f32(name: &str, dark: bool) -> Option<[f32; 4]> {
    ThemeToken::from_name(name).map(|t| t.rgba_f32(dark))
}

/// Convert a `[u8; 4]` 0..=255 RGBA to renderer floats `[f32; 4]` 0..=1.
fn u8_to_f32([r, g, b, a]: [u8; 4]) -> [f32; 4] {
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

/// The renderer's active theme: a single dark/light bit. This is the value the
/// theme UNIFORM carries — flipping it re-resolves every token color GPU-bound
/// without touching tessellation (D2/P4). Chrome (canvas clear, selection ring,
/// drop shadow) is sourced from tokens through this bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Theme {
    /// `true` = dark mode, `false` = light mode.
    pub dark: bool,
}

impl Theme {
    /// Light mode (the default at load).
    pub const fn light() -> Self {
        Theme { dark: false }
    }

    /// Dark mode.
    pub const fn dark() -> Self {
        Theme { dark: true }
    }

    /// Resolve a token to renderer floats in this theme.
    pub fn token_f32(self, token: ThemeToken) -> [f32; 4] {
        token.rgba_f32(self.dark)
    }

    /// The canvas clear color (`canvas-bg`) for this theme as renderer floats.
    pub fn canvas_bg(self) -> [f32; 4] {
        self.token_f32(ThemeToken::CanvasBg)
    }

    /// The selection/focus ring color (`selection-ring`) for this theme.
    pub fn selection_ring(self) -> [f32; 4] {
        self.token_f32(ThemeToken::SelectionRing)
    }

    /// The drop-shadow color (`shadow`, translucent) for this theme. RB3's
    /// default shadow pass reads its paint from here, NOT a hardcoded color.
    pub fn shadow(self) -> [f32; 4] {
        self.token_f32(ThemeToken::Shadow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = ALL_TOKENS.iter().map(|t| t.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "token names must be unique");
    }

    #[test]
    fn name_round_trips_through_from_name() {
        for t in ALL_TOKENS {
            assert_eq!(ThemeToken::from_name(t.name()), Some(t));
        }
    }

    #[test]
    fn mirrors_c1_canonical_values() {
        // Pin the exact light/dark RGBA the C1 contract specifies so a drift
        // between the two crates' tables fails here (the wire contract depends
        // on both agreeing).
        assert_eq!(resolve_token("canvas-bg", false), Some([0xf5, 0xf5, 0xf7, 0xff]));
        assert_eq!(resolve_token("canvas-bg", true), Some([0x1e, 0x1e, 0x20, 0xff]));
        assert_eq!(resolve_token("default-fill", false), Some([0xff, 0xff, 0xff, 0xff]));
        assert_eq!(resolve_token("default-fill", true), Some([0x2c, 0x2c, 0x2e, 0xff]));
        assert_eq!(resolve_token("default-stroke", false), Some([0xc6, 0xc6, 0xc8, 0xff]));
        assert_eq!(resolve_token("default-stroke", true), Some([0x54, 0x54, 0x56, 0xff]));
        assert_eq!(resolve_token("text", false), Some([0x1d, 0x1d, 0x1f, 0xff]));
        assert_eq!(resolve_token("text", true), Some([0xf5, 0xf5, 0xf7, 0xff]));
        assert_eq!(resolve_token("shadow", false), Some([0x00, 0x00, 0x00, 0x55]));
        assert_eq!(resolve_token("shadow", true), Some([0xff, 0xff, 0xff, 0xa8]));
        assert_eq!(resolve_token("selection-ring", false), Some([0x00, 0x7a, 0xff, 0xff]));
        assert_eq!(resolve_token("selection-ring", true), Some([0x0a, 0x84, 0xff, 0xff]));
    }

    #[test]
    fn shadow_is_translucent_in_both_modes() {
        assert!(resolve_token("shadow", false).unwrap()[3] < 0xff);
        assert!(resolve_token("shadow", true).unwrap()[3] < 0xff);
    }

    #[test]
    fn dark_shadow_is_a_visible_white_halo() {
        // W3-G10/#1: dark-mode shadow must be a WHITE veil with enough alpha that
        // the wide quarter-res blur still reads over the near-black canvas. The old
        // 0x33 (~0.20) was imperceptible; pin a substantially raised alpha so a
        // regression back to that faint value fails here.
        let [r, g, b, a] = resolve_token("shadow", true).unwrap();
        assert_eq!([r, g, b], [0xff, 0xff, 0xff], "dark shadow must be white");
        assert!(a >= 0x99, "dark shadow alpha must be raised (got {a:#x})");
        assert!(a < 0xff, "dark shadow stays translucent");
    }

    #[test]
    fn unknown_token_resolves_to_none() {
        assert_eq!(resolve_token("not-a-token", false), None);
        assert_eq!(resolve_token_f32("not-a-token", true), None);
    }

    #[test]
    fn theme_chrome_flips_with_the_bit() {
        // The canvas clear, selection ring, and shadow each differ light vs dark
        // — the falsifiable "chrome RGBA flips with the theme bit" property.
        assert_ne!(Theme::light().canvas_bg(), Theme::dark().canvas_bg());
        assert_ne!(Theme::light().selection_ring(), Theme::dark().selection_ring());
        assert_ne!(Theme::light().shadow(), Theme::dark().shadow());
    }

    #[test]
    fn f32_resolution_matches_u8_table() {
        let [r, g, b, a] = ThemeToken::CanvasBg.rgba(false);
        assert_eq!(
            ThemeToken::CanvasBg.rgba_f32(false),
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
        );
    }
}
