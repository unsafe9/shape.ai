//! Renderer-side semantic theme token table — an intentional MIRROR of the
//! `scene-core::object::theme` token contract (same kebab-case names, same
//! light/dark RGBA pairs), held here so token paints resolve at draw time without
//! crossing the crate boundary. Keep names + values in lock-step with scene-core,
//! or the wire contract `{"kind":"token","name":"<kebab>"}` breaks. Pure: a static
//! light/dark RGBA lookup; flipping the `dark` bit re-resolves with zero re-tessellation.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeToken {
    CanvasBg,
    Surface,
    SurfaceMuted,
    DefaultFill,
    DefaultStroke,
    Text,
    Shadow,
    SelectionRing,
    Material,
    Hairline,
    Hover,
    AccentSoft,
    TextSecondary,
}

pub const ALL_TOKENS: [ThemeToken; 13] = [
    ThemeToken::CanvasBg,
    ThemeToken::Surface,
    ThemeToken::SurfaceMuted,
    ThemeToken::DefaultFill,
    ThemeToken::DefaultStroke,
    ThemeToken::Text,
    ThemeToken::Shadow,
    ThemeToken::SelectionRing,
    ThemeToken::Material,
    ThemeToken::Hairline,
    ThemeToken::Hover,
    ThemeToken::AccentSoft,
    ThemeToken::TextSecondary,
];

impl ThemeToken {
    /// The kebab-case wire name (matches the `RPaint::Token { name }` wire).
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
            ThemeToken::Material => "material",
            ThemeToken::Hairline => "hairline",
            ThemeToken::Hover => "hover",
            ThemeToken::AccentSoft => "accent-soft",
            ThemeToken::TextSecondary => "text-secondary",
        }
    }

    pub fn from_name(name: &str) -> Option<ThemeToken> {
        ALL_TOKENS.into_iter().find(|t| t.name() == name)
    }

    /// RGBA `[r, g, b, a]`, 0..=255. Byte-identical to the scene-core table.
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

            (ThemeToken::Material, false) => [0xf7, 0xf7, 0xf9, 0xe6],
            (ThemeToken::Material, true) => [0x2c, 0x2c, 0x2e, 0xe6],

            (ThemeToken::Hairline, false) => [0x00, 0x00, 0x00, 0x1f],
            (ThemeToken::Hairline, true) => [0xff, 0xff, 0xff, 0x26],

            (ThemeToken::Hover, false) => [0x00, 0x00, 0x00, 0x14],
            (ThemeToken::Hover, true) => [0xff, 0xff, 0xff, 0x1f],

            (ThemeToken::AccentSoft, false) => [0x00, 0x7a, 0xff, 0x26],
            (ThemeToken::AccentSoft, true) => [0x0a, 0x84, 0xff, 0x3d],

            (ThemeToken::TextSecondary, false) => [0x8a, 0x8a, 0x8e, 0xff],
            (ThemeToken::TextSecondary, true) => [0x98, 0x98, 0x9d, 0xff],
        }
    }

    /// RGBA as renderer floats `[f32; 4]`, each channel 0..=1.
    pub fn rgba_f32(self, dark: bool) -> [f32; 4] {
        u8_to_f32(self.rgba(dark))
    }
}

pub fn resolve_token(name: &str, dark: bool) -> Option<[u8; 4]> {
    ThemeToken::from_name(name).map(|t| t.rgba(dark))
}

pub fn resolve_token_f32(name: &str, dark: bool) -> Option<[f32; 4]> {
    ThemeToken::from_name(name).map(|t| t.rgba_f32(dark))
}

fn u8_to_f32([r, g, b, a]: [u8; 4]) -> [f32; 4] {
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

/// The renderer's active theme: a single dark/light bit, the value the theme
/// uniform carries. Flipping it re-resolves every token color without touching
/// tessellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Theme {
    pub dark: bool,
}

impl Theme {
    pub const fn light() -> Self {
        Theme { dark: false }
    }

    pub const fn dark() -> Self {
        Theme { dark: true }
    }

    pub fn token_f32(self, token: ThemeToken) -> [f32; 4] {
        token.rgba_f32(self.dark)
    }

    pub fn canvas_bg(self) -> [f32; 4] {
        self.token_f32(ThemeToken::CanvasBg)
    }

    pub fn selection_ring(self) -> [f32; 4] {
        self.token_f32(ThemeToken::SelectionRing)
    }

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
        // Pin the exact RGBA so any drift from the scene-core table fails here.
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
        // Dark shadow is a white veil with alpha high enough to read over the
        // near-black canvas through the wide quarter-res blur.
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

    /// Pin the macOS-material token RGBA byte-for-byte against the scene-core
    /// table contract; any drift between the two mirrors fails here.
    #[test]
    fn material_language_tokens_mirror_scene_core() {
        assert_eq!(resolve_token("material", false), Some([0xf7, 0xf7, 0xf9, 0xe6]));
        assert_eq!(resolve_token("material", true), Some([0x2c, 0x2c, 0x2e, 0xe6]));
        assert_eq!(resolve_token("hairline", false), Some([0x00, 0x00, 0x00, 0x1f]));
        assert_eq!(resolve_token("hairline", true), Some([0xff, 0xff, 0xff, 0x26]));
        assert_eq!(resolve_token("hover", false), Some([0x00, 0x00, 0x00, 0x14]));
        assert_eq!(resolve_token("hover", true), Some([0xff, 0xff, 0xff, 0x1f]));
        assert_eq!(resolve_token("accent-soft", false), Some([0x00, 0x7a, 0xff, 0x26]));
        assert_eq!(resolve_token("accent-soft", true), Some([0x0a, 0x84, 0xff, 0x3d]));
        assert_eq!(resolve_token("text-secondary", false), Some([0x8a, 0x8a, 0x8e, 0xff]));
        assert_eq!(resolve_token("text-secondary", true), Some([0x98, 0x98, 0x9d, 0xff]));
        // Translucent material fills keep their frosted alpha.
        assert!(resolve_token("material", false).unwrap()[3] < 0xff);
        assert!(resolve_token("material", true).unwrap()[3] < 0xff);
    }

    #[test]
    fn theme_chrome_flips_with_the_bit() {
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
