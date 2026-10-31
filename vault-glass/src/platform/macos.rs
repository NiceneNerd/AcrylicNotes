//! macOS: vibrancy and Liquid Glass through `window-vibrancy`.
//!
//! Liquid Glass needs macOS 26+; older systems fall back to vibrancy
//! (`NSVisualEffectView`). The generic window blur is applied by the GUI
//! toolkit itself (`winit`'s `set_blur`), so it is reported as a success here
//! without a native call.

use crate::{negotiate, Backdrop, Capabilities, GlassError, GlassStyle};
use raw_window_handle::HasWindowHandle;
use window_vibrancy::{NSGlassEffectViewStyle, NSVisualEffectMaterial};

/// The macOS backdrop.
pub struct NativeBackdrop;

impl Backdrop for NativeBackdrop {
    fn probe(&self) -> Capabilities {
        Capabilities {
            preferred: GlassStyle::LiquidGlass,
            fallbacks: vec![GlassStyle::Vibrancy, GlassStyle::Blur, GlassStyle::Opaque],
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
            GlassStyle::LiquidGlass => window_vibrancy::apply_liquid_glass(
                window,
                window_vibrancy::LiquidGlassOptions::new(NSGlassEffectViewStyle::Regular),
            ),
            GlassStyle::Vibrancy => window_vibrancy::apply_vibrancy(
                window,
                NSVisualEffectMaterial::ContentBackground,
                None,
                None,
            ),
            GlassStyle::Blur => Ok(()),
            GlassStyle::Mica | GlassStyle::Acrylic => {
                Err(GlassError::Platform("Windows-only effect".into()))
            }
            GlassStyle::Opaque => Ok(()),
        })
        .map_err(|error| GlassError::Platform(error.to_string()))
    }
}
