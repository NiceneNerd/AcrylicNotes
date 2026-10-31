//! `vault-glass` — one trait for putting a real backdrop behind a window.
//!
//! A frontend asks for a [`GlassStyle`]; the platform backend applies the best
//! effect it can and reports back what actually happened, so the caller can
//! fall back to an opaque surface instead of guessing.
//!
//! # Platform notes
//!
//! * **Linux / Wayland** — the compositor owns the blur. The app only has to
//!   ask for it (`iced::window::enable_blur`, which the iced fork implements
//!   with the Wayland `ext-background-effect-v1` protocol) and keep its own
//!   surfaces translucent. COSMIC (>= 1.3), KDE Plasma (6.7) and GNOME (51)
//!   implement that protocol; other compositors ignore the request, which
//!   shows up as "no visible blur" rather than as an error.
//! * **Windows** — `window-vibrancy`: Mica (Windows 11, build 22000+) with
//!   Acrylic and plain blur as fallbacks.
//! * **macOS** — `window-vibrancy` vibrancy, Liquid Glass on macOS 26+, and
//!   the toolkit's own window blur as the generic fallback.
//!
//! The crate deliberately knows nothing about any GUI toolkit: it only needs a
//! raw window handle.

pub mod platform;

use std::fmt;

pub use raw_window_handle::HasWindowHandle;

/// A backdrop effect an application may ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlassStyle {
    /// No effect: paint an opaque surface.
    Opaque,
    /// Generic blurred backdrop (COSMIC frosted glass, macOS window blur).
    Blur,
    /// Windows 11 Mica.
    Mica,
    /// Windows Acrylic.
    Acrylic,
    /// macOS vibrancy (`NSVisualEffectView`).
    Vibrancy,
    /// macOS 26+ Liquid Glass.
    LiquidGlass,
}

impl GlassStyle {
    /// A short human readable name, used by the UI and in log lines.
    pub const fn name(self) -> &'static str {
        match self {
            GlassStyle::Opaque => "opaque",
            GlassStyle::Blur => "blur",
            GlassStyle::Mica => "mica",
            GlassStyle::Acrylic => "acrylic",
            GlassStyle::Vibrancy => "vibrancy",
            GlassStyle::LiquidGlass => "liquid glass",
        }
    }
}

impl fmt::Display for GlassStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a platform can offer, best first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// The style the platform uses when the caller has no preference.
    pub preferred: GlassStyle,
    /// Ordered fallbacks, tried when the preferred style is refused.
    pub fallbacks: Vec<GlassStyle>,
}

/// Why a backdrop could not be applied at all.
#[derive(Clone, Debug)]
pub enum GlassError {
    /// The native window handle could not be read.
    Handle(String),
    /// The platform refused an effect for a reason other than "not this OS".
    Platform(String),
}

impl fmt::Display for GlassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlassError::Handle(msg) => write!(f, "window handle error: {msg}"),
            GlassError::Platform(msg) => write!(f, "platform refused backdrop: {msg}"),
        }
    }
}

impl std::error::Error for GlassError {}

/// A platform backdrop.
pub trait Backdrop {
    /// What this platform can do right now.
    fn probe(&self) -> Capabilities;

    /// Apply `requested`, falling back when the platform refuses it.
    ///
    /// Returns the style that was actually applied. `GlassStyle::Opaque` means
    /// no effect could be used and the caller should paint an opaque surface.
    fn apply(
        &self,
        window: &dyn HasWindowHandle,
        requested: GlassStyle,
    ) -> Result<GlassStyle, GlassError>;
}

/// Try `requested` first, then the platform's fallbacks, then give up.
///
/// `attempt` is called once per candidate style, in order, and is never asked
/// to apply [`GlassStyle::Opaque`]: an opaque surface is always available, so
/// exhausting the chain succeeds with [`GlassStyle::Opaque`] instead of
/// failing.
pub fn negotiate<F>(
    requested: GlassStyle,
    capabilities: &Capabilities,
    mut attempt: F,
) -> Result<GlassStyle, GlassError>
where
    F: FnMut(GlassStyle) -> Result<(), GlassError>,
{
    if requested == GlassStyle::Opaque {
        return Ok(GlassStyle::Opaque);
    }

    let mut chain = vec![requested];
    for style in &capabilities.fallbacks {
        if !chain.contains(style) {
            chain.push(*style);
        }
    }
    // An opaque surface is always available, so it terminates the chain.
    if !chain.contains(&GlassStyle::Opaque) {
        chain.push(GlassStyle::Opaque);
    }

    let mut last_error = None;
    for style in chain {
        if style == GlassStyle::Opaque {
            return Ok(GlassStyle::Opaque);
        }
        match attempt(style) {
            Ok(()) => return Ok(style),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| GlassError::Platform("no candidate styles".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(preferred: GlassStyle, fallbacks: &[GlassStyle]) -> Capabilities {
        Capabilities {
            preferred,
            fallbacks: fallbacks.to_vec(),
        }
    }

    #[test]
    fn opaque_is_never_negotiated() {
        let mut tried = Vec::new();
        let applied = negotiate(GlassStyle::Opaque, &caps(GlassStyle::Mica, &[]), |style| {
            tried.push(style);
            Ok(())
        })
        .unwrap();
        assert_eq!(applied, GlassStyle::Opaque);
        assert!(tried.is_empty(), "opaque must not call the platform");
    }

    #[test]
    fn preferred_style_wins_when_it_works() {
        let mut tried = Vec::new();
        let applied = negotiate(
            GlassStyle::Mica,
            &caps(GlassStyle::Mica, &[GlassStyle::Acrylic, GlassStyle::Opaque]),
            |style| {
                tried.push(style);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(applied, GlassStyle::Mica);
        assert_eq!(tried, [GlassStyle::Mica]);
    }

    #[test]
    fn windows_falls_back_mica_to_acrylic_to_blur_to_opaque() {
        let capabilities = caps(
            GlassStyle::Mica,
            &[GlassStyle::Acrylic, GlassStyle::Blur, GlassStyle::Opaque],
        );

        let mut tried = Vec::new();
        let applied = negotiate(GlassStyle::Mica, &capabilities, |style| {
            tried.push(style);
            if style == GlassStyle::Mica {
                Err(GlassError::Platform("not Windows 11".into()))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(applied, GlassStyle::Acrylic);
        assert_eq!(tried, [GlassStyle::Mica, GlassStyle::Acrylic]);

        let mut tried = Vec::new();
        let applied = negotiate(GlassStyle::Mica, &capabilities, |style| {
            tried.push(style);
            match style {
                GlassStyle::Mica | GlassStyle::Acrylic => {
                    Err(GlassError::Platform("not supported".into()))
                }
                _ => Ok(()),
            }
        })
        .unwrap();
        assert_eq!(applied, GlassStyle::Blur);
        assert_eq!(
            tried,
            [GlassStyle::Mica, GlassStyle::Acrylic, GlassStyle::Blur]
        );
    }

    #[test]
    fn everything_refused_falls_back_to_opaque() {
        let capabilities = caps(GlassStyle::Mica, &[GlassStyle::Acrylic]);
        let mut tried = Vec::new();
        let applied = negotiate(GlassStyle::Mica, &capabilities, |style| {
            tried.push(style);
            Err(GlassError::Platform("nope".into()))
        })
        .unwrap();
        assert_eq!(applied, GlassStyle::Opaque);
        assert_eq!(tried, [GlassStyle::Mica, GlassStyle::Acrylic]);
    }

    #[test]
    fn requested_style_is_tried_before_the_platform_preference() {
        let capabilities = caps(GlassStyle::Mica, &[GlassStyle::Blur]);
        let mut tried = Vec::new();
        let applied = negotiate(GlassStyle::Blur, &capabilities, |style| {
            tried.push(style);
            Ok(())
        })
        .unwrap();
        assert_eq!(applied, GlassStyle::Blur);
        assert_eq!(tried, [GlassStyle::Blur]);
    }
}
