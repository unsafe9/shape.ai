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
}

pub const ALL_TOKENS: [Token; 8] = [
    Token::CanvasBg,
    Token::Surface,
    Token::SurfaceMuted,
    Token::DefaultFill,
    Token::DefaultStroke,
    Token::Text,
    Token::Shadow,
    Token::SelectionRing,
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
            (Token::Shadow, true) => [0xff, 0xff, 0xff, 0xa8],

            (Token::SelectionRing, false) => [0x00, 0x7a, 0xff, 0xff],
            (Token::SelectionRing, true) => [0x0a, 0x84, 0xff, 0xff],
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
}
