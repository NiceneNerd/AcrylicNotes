//! Platforms without any backdrop API: everything is opaque.

use crate::{Backdrop, Capabilities, GlassError, GlassStyle};
use raw_window_handle::HasWindowHandle;

/// The fallback backdrop for platforms we cannot blur.
pub struct NativeBackdrop;

impl Backdrop for NativeBackdrop {
    fn probe(&self) -> Capabilities {
        Capabilities {
            preferred: GlassStyle::Opaque,
            fallbacks: vec![GlassStyle::Opaque],
        }
    }

    fn apply(
        &self,
        _window: &dyn HasWindowHandle,
        requested: GlassStyle,
    ) -> Result<GlassStyle, GlassError> {
        Ok(match requested {
            GlassStyle::Opaque => GlassStyle::Opaque,
            _ => GlassStyle::Opaque,
        })
    }
}
