//! Linux: the compositor performs the blur.
//!
//! The iced fork implements `iced::window::enable_blur` on Wayland with the
//! `ext-background-effect-v1` protocol, so the application only has to ask the
//! compositor and keep its own surfaces translucent. There is no per-window
//! native call to make from here.

use crate::{Backdrop, Capabilities, GlassError, GlassStyle};
use raw_window_handle::HasWindowHandle;

/// Backdrop for Wayland sessions, where the compositor owns the blur.
pub struct CompositorBackdrop;

impl Backdrop for CompositorBackdrop {
    fn probe(&self) -> Capabilities {
        if supports_compositor_blur() {
            Capabilities {
                preferred: GlassStyle::Blur,
                fallbacks: vec![GlassStyle::Opaque],
            }
        } else {
            Capabilities {
                preferred: GlassStyle::Opaque,
                fallbacks: vec![GlassStyle::Opaque],
            }
        }
    }

    fn apply(
        &self,
        _window: &dyn HasWindowHandle,
        requested: GlassStyle,
    ) -> Result<GlassStyle, GlassError> {
        // Nothing to call: the GUI toolkit asks the compositor for the blur.
        Ok(match requested {
            GlassStyle::Opaque => GlassStyle::Opaque,
            _ if supports_compositor_blur() => GlassStyle::Blur,
            _ => GlassStyle::Opaque,
        })
    }
}

/// Whether this session looks like a Wayland session that can blur.
///
/// The protocol is implemented by COSMIC (>= 1.3), KDE Plasma (6.7) and GNOME
/// (51). Compositors without it ignore the request, which shows up as "no
/// visible blur" rather than as an error.
pub fn supports_compositor_blur() -> bool {
    wayland_session_from(
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
    )
}

/// Pure helper, so the decision can be unit tested without touching the
/// process environment.
fn wayland_session_from(wayland_display: Option<&str>, session_type: Option<&str>) -> bool {
    wayland_display.is_some_and(|display| !display.is_empty())
        || session_type.is_some_and(|kind| kind.eq_ignore_ascii_case("wayland"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_session_detection() {
        assert!(wayland_session_from(Some("wayland-1"), None));
        assert!(wayland_session_from(None, Some("wayland")));
        assert!(wayland_session_from(None, Some("Wayland")));
        assert!(!wayland_session_from(None, Some("x11")));
        assert!(!wayland_session_from(Some(""), None));
        assert!(!wayland_session_from(None, None));
    }

    #[test]
    fn probe_follows_the_session() {
        let backdrop = CompositorBackdrop;
        let capabilities = backdrop.probe();
        if supports_compositor_blur() {
            assert_eq!(capabilities.preferred, GlassStyle::Blur);
        } else {
            assert_eq!(capabilities.preferred, GlassStyle::Opaque);
        }
        assert_eq!(capabilities.fallbacks, [GlassStyle::Opaque]);
    }
}
