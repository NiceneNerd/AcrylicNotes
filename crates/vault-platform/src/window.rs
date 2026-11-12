//! Window-level requests: title, focus, close, backdrop.
//!
//! The cosmic frontend reaches these through libcosmic's `Core`/tasks
//! today; the adapter wraps them when views thin out (p7). The Windows
//! frontend maps them to app-window APIs (p9).

use crate::appearance::Backdrop;

/// Requests window-level changes from the platform.
pub trait WindowService: Send + Sync {
    /// Set the window/desktop title ("note — vault" per MISC-100).
    fn set_title(&self, title: &str);

    /// Bring the window into focus.
    fn focus(&self);

    /// Ask the platform to close the window (the shared close workflow
    /// must have finished saving/prompting first).
    fn request_close(&self);

    /// Request a backdrop effect (delegates to the toolkit/compositor).
    fn set_backdrop(&self, backdrop: Backdrop);
}
