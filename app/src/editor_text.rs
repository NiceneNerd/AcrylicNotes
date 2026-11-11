//! Editor text presentation: contrast-guaranteed colors and
//! font-consistent highlighting, on any COSMIC theme and font choice.
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
//!
//! The same treatment applies to fonts: iced's syntect `Highlight`
//! hardcodes its base family to `Monospace`, so a highlighted italic
//! stretch renders monospaced even when the editor's font is the system
//! UI font (or a custom one). [`editor_highlight_format`] re-roots each
//! span on the editor's actual font, keeping only the weight/style the
//! syntax theme asked for.

use cosmic::iced::core::font::{Family, Style, Weight};
use cosmic::iced::Color;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex, RwLock};

/// Minimum WCAG 2.x contrast ratio for the editor's body text. 10:1
/// puts the editor on par with the reading pane (the theme's own
/// text-on-background color measures ~9.7:1 there) while leaving room
/// before pure white. Default themes sit between ~11:1 and ~20:1, so
/// this only lifts themes whose text color is genuinely dim.
pub const EDITOR_TEXT_MIN_CONTRAST: f32 = 10.0;

/// Minimum contrast for the editor's placeholder and syntax-token
/// colors (a notch above the 4.5:1 AA bar for body text). Tokens keep
/// their designed hue but never drop below readable.
pub const EDITOR_SECONDARY_MIN_CONTRAST: f32 = 5.5;

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
    // 0.1 per step keeps the guarantee tight: the color that comes out
    // lands within a few percent of the bar rather than a whole step
    // below it. 0.9^80 is ~0.0002, so the cap reaches the endpoint.
    for _ in 0..80 {
        rgb = blend_toward(rgb, endpoint, 0.1);
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

/// The editor's font: the custom family when one is set (it wins over
/// the monospace toggle), else the monospace/system choice the toggle
/// makes. An unknown family name falls back to the default font at
/// render time, so a typo degrades instead of breaking.
pub fn editor_font(mono: bool, custom_family: &str) -> cosmic::iced::Font {
    let family = custom_family.trim();
    if family.is_empty() {
        return if mono {
            cosmic::font::mono()
        } else {
            cosmic::font::default()
        };
    }
    cosmic::iced::Font {
        family: Family::Name(intern_family(family)),
        ..cosmic::iced::Font::DEFAULT
    }
}

/// The reading view's font: the custom family when one is set, else the
/// system UI font. An unknown family name falls back to the default
/// font at render time, so a typo degrades instead of breaking.
///
/// The reading view has its own family: the editor's monospace toggle
/// says nothing about how a note should read.
pub fn reading_font(custom_family: &str) -> cosmic::iced::Font {
    let family = custom_family.trim();
    if family.is_empty() {
        return cosmic::font::default();
    }
    cosmic::iced::Font {
        family: Family::Name(intern_family(family)),
        ..cosmic::iced::Font::DEFAULT
    }
}

/// `Family::Name` holds a `&'static str`, so a family typed at runtime
/// is interned once and leaked; libcosmic's own `FontConfig` conversion
/// interns font names the same way. Typing in the settings field leaks
/// one short string per distinct prefix — bounded by keystrokes.
fn intern_family(family: &str) -> &'static str {
    static FAMILY_MAP: LazyLock<RwLock<HashSet<&'static str>>> = LazyLock::new(RwLock::default);
    let mut map = FAMILY_MAP.write().unwrap();
    if let Some(name) = map.get(family).copied() {
        return name;
    }
    let leaked: &'static str = Box::leak(family.to_owned().into_boxed_str());
    map.insert(leaked);
    leaked
}

/// The font family the editor's highlighted spans must use. iced's
/// `to_format` callback is a plain fn pointer with no channel for app
/// state, so the editor's resolved font is parked here on each view
/// build and read per highlighted span at render time.
static EDITOR_BASE_FONT: Mutex<cosmic::iced::Font> = Mutex::new(cosmic::iced::Font::MONOSPACE);

/// Park the editor's current font for `editor_highlight_format` to read.
pub fn set_editor_base_font(font: cosmic::iced::Font) {
    *EDITOR_BASE_FONT.lock().unwrap() = font;
}

fn editor_base_font() -> cosmic::iced::Font {
    *EDITOR_BASE_FONT.lock().unwrap()
}

/// The font for one highlighted span: the syntax theme may ask for
/// bold/italic, but iced's `Highlight::font()` hardcodes the base
/// family to `Monospace` — keep its weight and style, swap in the
/// editor's actual font. A span that asks for only italics must not
/// clobber the base font's weight (and vice versa).
pub fn span_font(span: cosmic::iced::Font, base: cosmic::iced::Font) -> cosmic::iced::Font {
    cosmic::iced::Font {
        family: base.family,
        stretch: base.stretch,
        weight: if span.weight == Weight::Bold {
            span.weight
        } else {
            base.weight
        },
        style: if span.style == Style::Italic {
            span.style
        } else {
            base.style
        },
    }
}

/// The `to_format` callback for the editor's highlighter: every token
/// color the syntax theme emits is contrast-checked against the editor
/// surface, so dim tokens (comments, headings on custom themes) are
/// lifted to the AA bar while strong ones pass through untouched. Each
/// span is also re-rooted on the editor's actual font (see `span_font`).
///
/// A wikilink range overrides the markdown color: the target half wears
/// the theme's accent, the custom-label half its green — the two sides
/// of `[[target|label]]` read apart at a glance.
pub fn editor_highlight_format(
    highlight: &crate::highlight::Highlight,
    theme: &cosmic::Theme,
) -> cosmic::iced::core::text::highlighter::Format<cosmic::iced::Font> {
    use crate::highlight::WikiPart;
    let mut format = highlight.base;
    let color = match highlight.wiki {
        Some(WikiPart::Target) => Some(palette_color(theme.cosmic().accent.base)),
        Some(WikiPart::Alias) => Some(palette_color(theme.cosmic().success.base)),
        None => format.color,
    };
    if let Some(color) = color {
        format.color = Some(ensure_contrast(
            color,
            editor_surface(theme),
            1.0,
            EDITOR_SECONDARY_MIN_CONTRAST,
        ));
    }
    if let Some(span) = format.font {
        format.font = Some(span_font(span, editor_base_font()));
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
        let ratio = contrast_ratio(fixed, surface);
        assert!(ratio >= EDITOR_TEXT_MIN_CONTRAST);
        // The guarantee is tight: the blend lands just above the bar,
        // not a whole step below it.
        assert!(
            ratio < EDITOR_TEXT_MIN_CONTRAST * 1.15,
            "landed at {ratio:.2}, too far above the bar"
        );
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

    #[test]
    fn highlighted_spans_use_the_editor_font_not_monospace() {
        // iced's syntect Highlight hardcodes its base family to
        // Monospace; a span must re-root on the editor's font while
        // keeping what the syntax theme asked for.
        let base = cosmic::iced::Font::with_name("DejaVu Sans");
        let span = cosmic::iced::Font {
            style: cosmic::iced::core::font::Style::Italic,
            ..cosmic::iced::Font::MONOSPACE
        };
        let fixed = span_font(span, base);
        assert_eq!(
            fixed.family, base.family,
            "span re-rooted on the editor font"
        );
        assert_eq!(
            fixed.style,
            cosmic::iced::core::font::Style::Italic,
            "italics survive"
        );
        assert_eq!(
            fixed.weight, base.weight,
            "an italic-only span keeps the base weight"
        );

        let bold = cosmic::iced::Font {
            weight: cosmic::iced::core::font::Weight::Bold,
            ..cosmic::iced::Font::MONOSPACE
        };
        let fixed = span_font(bold, base);
        assert_eq!(
            fixed.weight,
            cosmic::iced::core::font::Weight::Bold,
            "bold survives"
        );
        assert_eq!(
            fixed.style, base.style,
            "a bold-only span keeps the base style"
        );
    }

    #[test]
    fn wikilinks_get_two_distinct_readable_colors() {
        use crate::highlight::WikiPart;
        use cosmic::iced::core::text::highlighter::Format;

        let theme = cosmic::Theme::default();
        let base = Format {
            color: None,
            font: None,
        };
        let target = editor_highlight_format(
            &crate::highlight::Highlight {
                base,
                wiki: Some(WikiPart::Target),
            },
            &theme,
        );
        let alias = editor_highlight_format(
            &crate::highlight::Highlight {
                base,
                wiki: Some(WikiPart::Alias),
            },
            &theme,
        );
        let target = target.color.expect("the target half is colored");
        let alias = alias.color.expect("the label half is colored");
        assert_ne!(
            (target.r, target.g, target.b),
            (alias.r, alias.g, alias.b),
            "the two sides of a wikilink read apart"
        );
        // Both meet the syntax-token bar on the editor surface.
        let surface = editor_surface(&theme);
        for color in [target, alias] {
            assert!(
                contrast_ratio(composite_over(color, surface), surface)
                    >= EDITOR_SECONDARY_MIN_CONTRAST
            );
        }
    }

    #[test]
    fn the_editor_font_follows_the_settings() {
        // No custom family: the monospace toggle decides.
        let mono = editor_font(true, "");
        assert_eq!(mono.family, cosmic::font::mono().family);
        let ui = editor_font(false, "  ");
        assert_eq!(ui.family, cosmic::font::default().family);
        // A custom family wins over the toggle (and surrounding spaces
        // are trimmed).
        let custom = editor_font(true, " Fira Code ");
        assert_eq!(
            custom.family,
            cosmic::iced::core::font::Family::Name("Fira Code")
        );
        // Interning is stable: the same name resolves to the same
        // leaked static on repeated calls.
        assert_eq!(editor_font(false, "Fira Code").family, custom.family);
    }

    #[test]
    fn the_reading_font_is_the_system_font_unless_a_family_is_named() {
        // The reading view has its own family: the editor's monospace
        // toggle says nothing about how a note should read.
        assert_eq!(
            reading_font("").family,
            cosmic::font::default().family,
            "no custom family follows the system font"
        );
        assert_eq!(
            reading_font("   ").family,
            cosmic::font::default().family,
            "a blank family is no family"
        );
        let custom = reading_font(" Iowan Old Style ");
        assert_eq!(
            custom.family,
            cosmic::iced::core::font::Family::Name("Iowan Old Style")
        );
        // Interning is stable here too.
        assert_eq!(reading_font("Iowan Old Style").family, custom.family);
    }
}
