//! Linux adapters for the `vault-platform` service contracts.
//!
//! * [`external_opener`] — `xdg-open`, the freedesktop default handler.
//!   Spawn failures (missing program, refused exec) become typed
//!   [`OpenError`]s; the child process's own outcome stays asynchronous
//!   with the OS handler, as before.
//! * [`appearance`] — the compositor's backdrop preference, delegated to
//!   the `vault-glass` Wayland probe (COSMIC/KDE/GNOME
//!   `ext-background-effect-v1` detection; X11 reports opaque).
//! * [`capabilities`] — honest flags: only adapters that exist here are
//!   reported available.

pub mod appearance;
pub mod capabilities;
pub mod opener;

pub use appearance::GlassAppearance;
pub use capabilities::LinuxCapabilities;
pub use opener::XdgOpen;

/// The platform's external opener (xdg-open).
#[must_use]
pub fn external_opener() -> Box<dyn vault_platform::ExternalOpener> {
    Box::new(XdgOpen)
}

/// The platform's appearance service (vault-glass compositor probe).
#[must_use]
pub fn appearance() -> Box<dyn vault_platform::AppearanceService> {
    Box::new(GlassAppearance)
}

/// The platform's capability report.
#[must_use]
pub fn capabilities() -> Box<dyn vault_platform::PlatformCapabilities> {
    Box::new(LinuxCapabilities)
}
