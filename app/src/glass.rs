//! Glass backdrop integration (M4c).
//!
//! The platform decides what backdrop it can offer; the appearance
//! service (`vault-platform` trait, Linux adapter delegates to the
//! `vault-glass` Wayland probe) reports its preference, and this module
//! turns the answer into a libcosmic request. On Linux the compositor
//! owns the blur, so "applying" it means asking the toolkit
//! (`Action::BlurEnabled` → `iced::window::enable_blur`, the Wayland
//! `ext-background-effect-v1` protocol) and keeping our own surfaces
//! translucent — the editor overlay and dialog alpha do that. Whether
//! the compositor actually blurs is up to it (COSMIC's frosted-windows
//! setting); a compositor without support simply shows the translucent
//! surfaces unblurred, never an error.

use vault_glass::GlassStyle;
use vault_platform::Backdrop;

/// What to request from the platform: the appearance service's preferred
/// style when glass is enabled, otherwise opaque.
///
/// On Linux, [`Backdrop::apply`-style blur requests] deliberately report
/// blur as not-applicable (the toolkit delegates to the compositor), so
/// the adapter's *preferred* capability is the honest answer here.
#[must_use]
pub fn plan(glass_enabled: bool, preferred: Backdrop) -> GlassStyle {
    if !glass_enabled {
        return GlassStyle::Opaque;
    }
    match preferred {
        Backdrop::Blur => GlassStyle::Blur,
        Backdrop::Opaque => GlassStyle::Opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_glass_is_opaque() {
        assert_eq!(plan(false, Backdrop::Blur), GlassStyle::Opaque);
    }

    #[test]
    fn enabled_glass_follows_the_appearance_service() {
        assert_eq!(plan(true, Backdrop::Blur), GlassStyle::Blur);
        assert_eq!(plan(true, Backdrop::Opaque), GlassStyle::Opaque);
    }
}
