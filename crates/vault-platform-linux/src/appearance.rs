//! The compositor's backdrop preference, via the `vault-glass` probe.
//!
//! `vault-glass` keeps the per-platform detection (on Linux: Wayland
//! session detection — `WAYLAND_DISPLAY`/`XDG_SESSION_TYPE`; X11 reports
//! opaque). This adapter translates its answer into the neutral
//! [`Backdrop`] contract; the mapping stays lossless because the cosmic
//! frontend only distinguishes blur from opaque today.

use vault_glass::GlassStyle;
use vault_platform::{AppearanceService, Backdrop};

/// The Linux appearance service.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlassAppearance;

impl AppearanceService for GlassAppearance {
    fn preferred_backdrop(&self) -> Backdrop {
        let preferred = vault_glass::platform::native().probe().preferred;
        match preferred {
            GlassStyle::Blur => Backdrop::Blur,
            _ => Backdrop::Opaque,
        }
    }
}

/// Map a `vault-glass` style to the neutral contract (exposed for tests
/// and future frontends).
#[must_use]
pub fn to_backdrop(style: GlassStyle) -> Backdrop {
    match style {
        GlassStyle::Blur => Backdrop::Blur,
        _ => Backdrop::Opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_blur_maps_to_a_blur_backdrop() {
        assert_eq!(to_backdrop(GlassStyle::Blur), Backdrop::Blur);
        for opaque_like in [
            GlassStyle::Opaque,
            GlassStyle::Mica,
            GlassStyle::Acrylic,
            GlassStyle::Vibrancy,
            GlassStyle::LiquidGlass,
        ] {
            assert_eq!(to_backdrop(opaque_like), Backdrop::Opaque);
        }
    }

    #[test]
    fn the_probe_runs_and_returns_a_neutral_answer() {
        // The probe is environment-dependent (Wayland vs X11 in CI), so
        // assert the shape, not the value.
        let backdrop = GlassAppearance.preferred_backdrop();
        assert!(matches!(backdrop, Backdrop::Opaque | Backdrop::Blur));
    }
}
