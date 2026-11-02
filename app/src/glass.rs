//! Glass backdrop integration (M4c).
//!
//! The platform decides what backdrop it can offer; `vault-glass` probes it
//! and this module turns the answer into a libcosmic request. On Linux the
//! compositor owns the blur, so "applying" it means asking the toolkit
//! (`Action::BlurEnabled` → `iced::window::enable_blur`, the Wayland
//! `ext-background-effect-v1` protocol) and keeping our own surfaces
//! translucent — the editor overlay and dialog alpha do that. Whether the
//! compositor actually blurs is up to it (COSMIC's frosted-windows setting);
//! a compositor without support simply shows the translucent surfaces
//! unblurred, never an error.

use vault_glass::GlassStyle;

/// What to request from the platform: its preferred style when glass is
/// enabled, otherwise opaque.
///
/// On Linux, [`Backdrop::apply`] deliberately reports blur as
/// not-applicable (the toolkit delegates to the compositor), so the probe's
/// *preferred* capability is the honest answer here.
pub fn plan(glass_enabled: bool) -> GlassStyle {
    if !glass_enabled {
        return GlassStyle::Opaque;
    }
    vault_glass::platform::native().probe().preferred
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_glass_is_opaque() {
        assert_eq!(plan(false), GlassStyle::Opaque);
    }
}
