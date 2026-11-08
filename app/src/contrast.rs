//! Editor text contrast: the editor's colors must stay readable on any
//! COSMIC theme, including custom palettes that ship dim text colors.
//!
//! The editor's body text is the theme palette's `neutral_9`. The default
//! palettes make that near-black (light mode) or near-white (dark mode),
//! but a custom palette can set it to a mid-gray — measured at 2.15:1
//! WCAG against this app's editor surface, far below any readability
//! bar. Rather than trusting theme colors, every editor text color is
//! run through [`ensure_contrast`]: colors that already pass come back
//! unchanged (default themes render exactly as before), and colors that
//! fail are blended toward white or black — whichever side of the
//! surface has more contrast headroom — until they pass, preserving the
//! theme's hue while guaranteeing the ratio.

use cosmic::iced::Color;

/// Minimum WCAG 2.x contrast ratio for the editor's body text. 7:1 is
/// the "enhanced readability" tier: default themes sit between ~11:1 and
/// ~20:1, so this only lifts themes whose text color is genuinely dim.
pub const EDITOR_TEXT_MIN_CONTRAST: f32 = 7.0;

/// Minimum contrast for the editor's placeholder and syntax-token
/// colors (the standard AA bar for body text). Tokens keep their
/// designed hue but never drop below readable.
pub const EDITOR_SECONDARY_MIN_CONTRAST: f32 = 4.5;

/// Drawing alpha of the editor's placeholder text (kept from the old
/// hardcoded style; the contrast guarantee applies after compositing).
pub const EDITOR_PLACEHOLDER_ALPHA: f32 = 0.7;

/// WCAG 2.x relative luminance of an sRGB color.
fn relative_luminance(c: Color) -> f32 {
    fn channel(v: f32) -> f32 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG 2.x contrast ratio between two colors (1..=21). Alpha is
/// ignored: composite translucent colors first ([`composite_over`]).
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = (la.max(lb), la.min(lb));
    (hi + 0.05) / (lo + 0.05)
}

/// Source-over composite of `c` onto an opaque `bg` (sRGB-space
/// blending, matching how the pixels actually render).
pub fn composite_over(c: Color, bg: Color) -> Color {
    Color {
        r: c.r * c.a + bg.r * (1.0 - c.a),
        g: c.g * c.a + bg.g * (1.0 - c.a),
        b: c.b * c.a + bg.b * (1.0 - c.a),
        a: 1.0,
    }
}

/// Blend `c` toward `target` by `t` (0..=1) per channel; alpha carried.
fn blend_toward(c: Color, target: Color, t: f32) -> Color {
    Color {
        r: c.r + (target.r - c.r) * t,
        g: c.g + (target.g - c.g) * t,
        b: c.b + (target.b - c.b) * t,
        a: c.a,
    }
}

/// Nudge a text color until it reads on `surface`: the color drawn at
/// `alpha` over `surface` must reach the WCAG `target`. Colors that
/// already pass come back unchanged; failing colors are blended toward
/// white or black — whichever side of `surface` has more headroom — in
/// steps that converge on the pure endpoint. The blend preserves the
/// theme's hue while raising contrast.
pub fn ensure_contrast(text: Color, surface: Color, alpha: f32, target: f32) -> Color {
    let passes = |rgb: Color| {
        contrast_ratio(composite_over(Color { a: alpha, ..rgb }, surface), surface) >= target
    };
    let mut rgb = Color {
        r: text.r,
        g: text.g,
        b: text.b,
        a: 1.0,
    };
    if passes(rgb) {
        return Color { a: alpha, ..rgb };
    }
    let endpoint = if contrast_ratio(Color::WHITE, surface) >= contrast_ratio(Color::BLACK, surface)
    {
        Color::WHITE
    } else {
        Color::BLACK
    };
    // 0.25 per step reaches the endpoint in ~36 steps; the cap is a
    // backstop, and `passes` breaks early once the bar is met.
    for _ in 0..48 {
        rgb = blend_toward(rgb, endpoint, 0.25);
        if passes(rgb) {
            break;
        }
    }
    Color { a: alpha, ..rgb }
}

/// The color the editor's text effectively sits on: the theme's opaque
/// background base with the editor pane's dark overlay (black at 0.18)
/// composited on. The frosted-glass backdrop behind the window is
/// unknowable, so the theme's own background stands in for it.
pub fn editor_surface(theme: &cosmic::Theme) -> Color {
    let base = theme.cosmic().background(false).base;
    let bg = Color {
        r: base.red,
        g: base.green,
        b: base.blue,
        a: 1.0,
    };
    composite_over(
        Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.18,
        },
        bg,
    )
}

fn palette_color(c: palette::Srgba) -> Color {
    Color {
        r: c.red,
        g: c.green,
        b: c.blue,
        a: c.alpha,
    }
}

/// The editor's body-text color: the theme palette's `neutral_9`, made
/// opaque and contrast-checked against the editor surface.
pub fn editor_value_color(theme: &cosmic::Theme) -> Color {
    ensure_contrast(
        palette_color(theme.cosmic().palette.neutral_9),
        editor_surface(theme),
        1.0,
        EDITOR_TEXT_MIN_CONTRAST,
    )
}

/// The editor's placeholder color: `neutral_9` at reduced alpha, still
/// meeting the AA bar once composited over the surface.
pub fn editor_placeholder_color(theme: &cosmic::Theme) -> Color {
    ensure_contrast(
        palette_color(theme.cosmic().palette.neutral_9),
        editor_surface(theme),
        EDITOR_PLACEHOLDER_ALPHA,
        EDITOR_SECONDARY_MIN_CONTRAST,
    )
}

/// The `to_format` callback for the editor's highlighter: every token
/// color the syntax theme emits is contrast-checked against the editor
/// surface, so dim tokens (comments, headings on custom themes) are
/// lifted to the AA bar while strong ones pass through untouched.
pub fn editor_highlight_format(
    highlight: &cosmic::iced::highlighter::Highlight,
    theme: &cosmic::Theme,
) -> cosmic::iced::core::text::highlighter::Format<cosmic::iced::Font> {
    let mut format = highlight.to_format();
    if let Some(color) = format.color {
        format.color = Some(ensure_contrast(
            color,
            editor_surface(theme),
            1.0,
            EDITOR_SECONDARY_MIN_CONTRAST,
        ));
    }
    format
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's "windows-7-dark-mode" theme: a palette whose
    /// `neutral_9` (the editor's text color) is a mid-gray on a dark
    /// background — the case that shipped unreadable (~2.2:1).
    fn weak_theme_colors() -> (Color, Color) {
        // neutral_9 = rgba(0.39, 0.41, 0.43, 0.97); background.base =
        // rgba(0.15, 0.17, 0.19, 0.97); the editor surface is the base
        // with the pane's black 0.18 overlay composited on.
        let text = Color::from_rgba(0.39, 0.41, 0.43, 0.97);
        let surface = composite_over(
            Color::from_rgba(0.0, 0.0, 0.0, 0.18),
            Color::from_rgba(0.15, 0.17, 0.19, 1.0),
        );
        (text, surface)
    }

    #[test]
    fn weak_theme_body_text_is_lifted_to_easy_reading() {
        let (text, surface) = weak_theme_colors();
        assert!(
            contrast_ratio(composite_over(text, surface), surface) < 4.5,
            "the theme ships dim text"
        );
        let fixed = ensure_contrast(text, surface, 1.0, EDITOR_TEXT_MIN_CONTRAST);
        assert_eq!(fixed.a, 1.0, "body text is opaque");
        assert!(contrast_ratio(fixed, surface) >= EDITOR_TEXT_MIN_CONTRAST);
        // The lift lands on a tinted color, not a stark white.
        assert!(fixed.r < 1.0 && fixed.b < 1.0);
    }

    #[test]
    fn default_dark_theme_text_passes_untouched() {
        // Default dark palette: neutral_9 #DEDEDE on the derived dark
        // background — already well past the bar, so no blend runs.
        let text = Color::from_rgba(0.871, 0.871, 0.871, 0.97);
        let surface = Color::from_rgba(0.106, 0.106, 0.106, 1.0);
        let fixed = ensure_contrast(text, surface, 1.0, EDITOR_TEXT_MIN_CONTRAST);
        assert_eq!((fixed.r, fixed.g, fixed.b), (text.r, text.g, text.b));
        assert_eq!(fixed.a, 1.0);
    }

    #[test]
    fn placeholder_meets_aa_after_compositing() {
        let (text, surface) = weak_theme_colors();
        let fixed = ensure_contrast(
            text,
            surface,
            EDITOR_PLACEHOLDER_ALPHA,
            EDITOR_SECONDARY_MIN_CONTRAST,
        );
        assert_eq!(fixed.a, EDITOR_PLACEHOLDER_ALPHA);
        let drawn = composite_over(fixed, surface);
        assert!(contrast_ratio(drawn, surface) >= EDITOR_SECONDARY_MIN_CONTRAST);
    }

    #[test]
    fn dim_token_colors_are_lifted_and_strong_ones_pass() {
        let (_text, surface) = weak_theme_colors();
        // A dim comment gray (Base16Ocean's base03 sits near this on dark).
        let dim = Color::from_rgba(0.30, 0.32, 0.34, 1.0);
        let lifted = ensure_contrast(dim, surface, 1.0, EDITOR_SECONDARY_MIN_CONTRAST);
        assert!(contrast_ratio(lifted, surface) >= EDITOR_SECONDARY_MIN_CONTRAST);
        // A strong color passes through unchanged.
        let strong = Color::from_rgba(0.83, 0.91, 0.99, 1.0);
        let same = ensure_contrast(strong, surface, 1.0, EDITOR_SECONDARY_MIN_CONTRAST);
        assert_eq!((same.r, same.g, same.b), (strong.r, strong.g, strong.b));
    }

    #[test]
    fn light_backgrounds_blend_toward_black() {
        // A light custom theme with washed-out mid-gray text: the fix
        // must darken, not lighten.
        let surface = Color::from_rgba(0.92, 0.92, 0.92, 1.0);
        let text = Color::from_rgba(0.55, 0.55, 0.55, 1.0);
        let fixed = ensure_contrast(text, surface, 1.0, EDITOR_TEXT_MIN_CONTRAST);
        assert!(contrast_ratio(fixed, surface) >= EDITOR_TEXT_MIN_CONTRAST);
        assert!(fixed.r < text.r, "blended darker, not lighter");
    }

    #[test]
    fn the_real_default_theme_passes_the_guarantee() {
        let theme = cosmic::Theme::default();
        let surface = editor_surface(&theme);
        assert!(
            contrast_ratio(editor_value_color(&theme), surface) >= EDITOR_TEXT_MIN_CONTRAST,
            "body text"
        );
        let placeholder = editor_placeholder_color(&theme);
        assert!(
            contrast_ratio(composite_over(placeholder, surface), surface)
                >= EDITOR_SECONDARY_MIN_CONTRAST,
            "placeholder"
        );
    }
}
