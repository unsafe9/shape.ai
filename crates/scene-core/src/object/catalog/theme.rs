//! Semantic theme token table — the single source of truth for which tokens
//! exist and what they resolve to. Pure: a static light/dark RGBA lookup over a
//! fixed set of kebab-case names. The wire convention is `Paint::Token { name }`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Token {
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

pub const ALL_TOKENS: [Token; 13] = [
    Token::CanvasBg,
    Token::Surface,
    Token::SurfaceMuted,
    Token::DefaultFill,
    Token::DefaultStroke,
    Token::Text,
    Token::Shadow,
    Token::SelectionRing,
    Token::Material,
    Token::Hairline,
    Token::Hover,
    Token::AccentSoft,
    Token::TextSecondary,
];

impl Token {
    pub const fn name(self) -> &'static str {
        match self {
            Token::CanvasBg => "canvas-bg",
            Token::Surface => "surface",
            Token::SurfaceMuted => "surface-muted",
            Token::DefaultFill => "default-fill",
            Token::DefaultStroke => "default-stroke",
            Token::Text => "text",
            Token::Shadow => "shadow",
            Token::SelectionRing => "selection-ring",
            Token::Material => "material",
            Token::Hairline => "hairline",
            Token::Hover => "hover",
            Token::AccentSoft => "accent-soft",
            Token::TextSecondary => "text-secondary",
        }
    }

    pub fn from_name(name: &str) -> Option<Token> {
        ALL_TOKENS.into_iter().find(|t| t.name() == name)
    }

    /// RGBA (`[r, g, b, a]`, 0..=255) in the given mode.
    pub const fn rgba(self, dark: bool) -> [u8; 4] {
        match (self, dark) {
            (Token::CanvasBg, false) => [0xf5, 0xf5, 0xf7, 0xff],
            (Token::CanvasBg, true) => [0x1e, 0x1e, 0x20, 0xff],

            (Token::Surface, false) => [0xff, 0xff, 0xff, 0xff],
            (Token::Surface, true) => [0x2c, 0x2c, 0x2e, 0xff],

            (Token::SurfaceMuted, false) => [0xe9, 0xe9, 0xeb, 0xff],
            (Token::SurfaceMuted, true) => [0x3a, 0x3a, 0x3c, 0xff],

            (Token::DefaultFill, false) => [0xff, 0xff, 0xff, 0xff],
            (Token::DefaultFill, true) => [0x2c, 0x2c, 0x2e, 0xff],

            (Token::DefaultStroke, false) => [0xc6, 0xc6, 0xc8, 0xff],
            (Token::DefaultStroke, true) => [0x54, 0x54, 0x56, 0xff],

            (Token::Text, false) => [0x1d, 0x1d, 0x1f, 0xff],
            (Token::Text, true) => [0xf5, 0xf5, 0xf7, 0xff],

            (Token::Shadow, false) => [0x00, 0x00, 0x00, 0x40],
            (Token::Shadow, true) => [0xff, 0xff, 0xff, 0x66],

            (Token::SelectionRing, false) => [0x00, 0x7a, 0xff, 0xff],
            (Token::SelectionRing, true) => [0x0a, 0x84, 0xff, 0xff],

            (Token::Material, false) => [0xf7, 0xf7, 0xf9, 0xe6],
            (Token::Material, true) => [0x2c, 0x2c, 0x2e, 0xe6],

            (Token::Hairline, false) => [0x00, 0x00, 0x00, 0x1f],
            (Token::Hairline, true) => [0xff, 0xff, 0xff, 0x26],

            (Token::Hover, false) => [0x00, 0x00, 0x00, 0x14],
            (Token::Hover, true) => [0xff, 0xff, 0xff, 0x1f],

            (Token::AccentSoft, false) => [0x00, 0x7a, 0xff, 0x26],
            (Token::AccentSoft, true) => [0x0a, 0x84, 0xff, 0x3d],

            (Token::TextSecondary, false) => [0x8a, 0x8a, 0x8e, 0xff],
            (Token::TextSecondary, true) => [0x98, 0x98, 0x9d, 0xff],
        }
    }
}

pub fn resolve_token(name: &str, dark: bool) -> Option<[u8; 4]> {
    Token::from_name(name).map(|t| t.rgba(dark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_is_non_empty_and_names_are_unique() {
        assert!(!ALL_TOKENS.is_empty());
        let mut names: Vec<&str> = ALL_TOKENS.iter().map(|t| t.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "token names must be unique");
    }

    #[test]
    fn name_round_trips_through_from_name() {
        for t in ALL_TOKENS {
            assert_eq!(Token::from_name(t.name()), Some(t));
        }
    }

    #[test]
    fn resolve_known_tokens_differ_by_mode() {
        for name in ["canvas-bg", "text", "shadow"] {
            let light = resolve_token(name, false).expect("light");
            let dark = resolve_token(name, true).expect("dark");
            assert_ne!(light, dark, "{name} must differ light vs dark");
        }
    }

    #[test]
    fn shadow_is_translucent_in_both_modes() {
        assert!(resolve_token("shadow", false).unwrap()[3] < 0xff);
        assert!(resolve_token("shadow", true).unwrap()[3] < 0xff);
    }

    #[test]
    fn selection_ring_is_a_blue_accent_in_both_modes() {
        for dark in [false, true] {
            let [r, g, b, a] = resolve_token("selection-ring", dark).unwrap();
            assert_eq!(a, 0xff, "ring is opaque");
            assert!(b > r && b > g, "blue dominates in {} mode", if dark { "dark" } else { "light" });
        }
    }

    #[test]
    fn unknown_token_resolves_to_none() {
        assert_eq!(resolve_token("not-a-token", false), None);
        assert_eq!(resolve_token("not-a-token", true), None);
        assert_eq!(Token::from_name("surface "), None);
    }

    /// Pin the macOS-material token RGBA so any drift from the renderer-core
    /// mirror (byte-identical contract) fails here, not silently at draw time.
    #[test]
    fn material_language_tokens_have_locked_rgba() {
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
        // The two translucent fills must stay translucent (frosted material reads).
        assert!(resolve_token("material", false).unwrap()[3] < 0xff);
        assert!(resolve_token("material", true).unwrap()[3] < 0xff);
    }
}
