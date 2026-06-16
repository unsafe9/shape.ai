//! The ONE token→`#rrggbb` resolver for UI text. renderer-core's
//! `object_theme::resolve_token` returns RGBA bytes, NOT the `#rrggbb` string
//! `RTextRun.color` needs, and ui-core text references only a small token subset
//! — so this mirrors the SUBSET BY VALUE from
//! `renderer-core/src/object_theme.rs` (kept in lock-step;
//! `text_paint_token_matches_object_theme` pins them equal). It is NOT a copy of
//! the whole palette — object_theme.rs is the source of truth.

use crate::widget::TextPaint;

/// `Token` → `#rrggbb` using the same light/dark values as object_theme.rs;
/// `Hex` passes through. Unknown token → the `text` color (safe default).
pub(crate) fn resolve_text_paint(paint: &TextPaint, theme_dark: bool) -> String {
    match paint {
        TextPaint::Hex(hex) => hex.clone(),
        TextPaint::Token(name) => token_hex(name, theme_dark),
    }
}

/// `token` → `#rrggbb`. Mirrors the SUBSET ui-core widgets reference; unknown
/// token falls back to `text`. Alpha is dropped (UI paints are opaque).
pub(crate) fn token_hex(token: &str, theme_dark: bool) -> String {
    // RGB byte triples copied verbatim from object_theme.rs:51-77 (light, dark).
    let (r, g, b) = match token {
        "surface" => {
            if theme_dark {
                (0x2c, 0x2c, 0x2e)
            } else {
                (0xff, 0xff, 0xff)
            }
        }
        "surface-muted" => {
            if theme_dark {
                (0x3a, 0x3a, 0x3c)
            } else {
                (0xe9, 0xe9, 0xeb)
            }
        }
        "default-fill" => {
            if theme_dark {
                (0x2c, 0x2c, 0x2e)
            } else {
                (0xff, 0xff, 0xff)
            }
        }
        "default-stroke" => {
            if theme_dark {
                (0x54, 0x54, 0x56)
            } else {
                (0xc6, 0xc6, 0xc8)
            }
        }
        "selection-ring" => {
            if theme_dark {
                (0x0a, 0x84, 0xff)
            } else {
                (0x00, 0x7a, 0xff)
            }
        }
        "text-secondary" => {
            if theme_dark {
                (0x98, 0x98, 0x9d)
            } else {
                (0x8a, 0x8a, 0x8e)
            }
        }
        // "text" and every unknown token resolve to the text color.
        _ => {
            if theme_dark {
                (0xf5, 0xf5, 0xf7)
            } else {
                (0x1d, 0x1d, 0x1f)
            }
        }
    };
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_renderer_core::object_theme::resolve_token;

    /// A theme-token text color flips with the theme; a literal hex stays fixed.
    /// FAILS if a white-on-white regression slips back in (the resolved light
    /// `text` is dark `#1d1d1f`, never `#ffffff` the surface token resolves to).
    #[test]
    fn text_paint_token_flips_with_theme() {
        let tok = TextPaint::Token("text".to_string());
        let light = resolve_text_paint(&tok, false);
        let dark = resolve_text_paint(&tok, true);
        assert_eq!(light, "#1d1d1f");
        assert_eq!(dark, "#f5f5f7");
        assert_ne!(light, dark);
        // Contrast pin: light text never equals the light surface fill.
        assert_ne!(light, token_hex("surface", false));

        let hex = TextPaint::Hex("#abcdef".to_string());
        assert_eq!(resolve_text_paint(&hex, false), "#abcdef");
        assert_eq!(resolve_text_paint(&hex, true), "#abcdef");
    }

    /// Pin the mirror: ui-core token_hex RGB equals object_theme resolve_token's
    /// first three bytes for every UI-referenced token in both themes. Drift in
    /// object_theme.rs fails HERE, not silently downstream.
    #[test]
    fn text_paint_token_matches_object_theme() {
        for token in [
            "text",
            "surface",
            "surface-muted",
            "default-stroke",
            "default-fill",
            "selection-ring",
            "text-secondary",
        ] {
            for dark in [false, true] {
                let rgba = resolve_token(token, dark).expect("known token");
                let expected = format!("#{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2]);
                assert_eq!(token_hex(token, dark), expected, "token {token} dark={dark}");
            }
        }
    }
}
