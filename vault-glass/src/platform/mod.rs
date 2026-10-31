//! Platform selection for the backdrop implementation.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

use crate::Backdrop;

/// The backdrop implementation for the platform this binary was built for.
pub fn native() -> Box<dyn Backdrop> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::CompositorBackdrop)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::NativeBackdrop)
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::NativeBackdrop)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Box::new(unsupported::NativeBackdrop)
    }
}
