//! Windows: Mica / Acrylic / plain blur through `window-vibrancy`.
//!
//! Mica needs Windows 11 (build 22000+). Acrylic is the fallback for older
//! Windows, plain blur the last resort before the window goes opaque.

use crate::{negotiate, Backdrop, Capabilities, GlassError, GlassStyle};
use raw_window_handle::HasWindowHandle;

/// The Windows backdrop.
pub struct NativeBackdrop;

impl Backdrop for NativeBackdrop {
    fn probe(&self) -> Capabilities {
        Capabilities {
            preferred: GlassStyle::Mica,
            fallbacks: vec![GlassStyle::Acrylic, GlassStyle::Blur, GlassStyle::Opaque],
        }
    }

    fn apply(
        &self,
        window: &dyn HasWindowHandle,
        requested: GlassStyle,
    ) -> Result<GlassStyle, GlassError> {
        window
            .window_handle()
            .map_err(|error| GlassError::Handle(error.to_string()))?;

        negotiate(requested, &self.probe(), |style| match style {
            GlassStyle::Mica => window_vibrancy::apply_mica(window, None).map_err(windows_error),
            GlassStyle::Acrylic => {
                window_vibrancy::apply_acrylic(window, None).map_err(windows_error)
            }
            GlassStyle::Blur => {
                window_vibrancy::apply_blur(window, Some((18, 18, 18, 125))).map_err(windows_error)
            }
            GlassStyle::Vibrancy | GlassStyle::LiquidGlass => {
                Err(GlassError::Platform("macOS-only effect".into()))
            }
            GlassStyle::Opaque => Ok(()),
        })
        .map_err(|error| GlassError::Platform(error.to_string()))
    }
}

/// Map a `window-vibrancy` error onto `GlassError::Platform`.
fn windows_error(error: window_vibrancy::Error) -> GlassError {
    GlassError::Platform(error.to_string())
}
