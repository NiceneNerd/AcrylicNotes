//! Nonfatal user-facing messages.
//!
//! The app's dismissible error line is the in-app surface (SAVE-012);
//! this contract exists so a frontend can additionally route important
//! notices to OS-level notifications when one makes sense.

/// Surfaces nonfatal information to the user.
pub trait NotificationService: Send + Sync {
    /// An informational notice (e.g. "reloaded from disk").
    fn info(&self, message: &str);

    /// A failure notice (e.g. "save failed: …").
    fn error(&self, message: &str);
}
