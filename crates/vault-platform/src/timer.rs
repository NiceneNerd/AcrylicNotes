//! Timers for debounced and periodic background work.
//!
//! Callback-style: the cosmic frontend's autosave tick and watcher poll
//! arrive as toolkit subscriptions today; when they move behind this
//! service (autosave slice), the adapter bridges callbacks to messages.
//! The signature may gain a cancellation context then; the operation
//! names are the contract.

use std::time::Duration;

/// Cancels a scheduled periodic timer.
pub struct TimerHandle {
    cancel: Box<dyn FnOnce() + Send>,
}

impl TimerHandle {
    /// Wrap the adapter's cancellation closure.
    pub fn new(cancel: Box<dyn FnOnce() + Send>) -> Self {
        Self { cancel }
    }

    /// Stop the periodic callback. Calling twice is harmless (the
    /// closure runs once; the second call is a no-op by construction).
    pub fn cancel(self) {
        (self.cancel)()
    }
}

impl std::fmt::Debug for TimerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TimerHandle")
    }
}

/// Schedules deferred and periodic work on the platform.
pub trait TimerService: Send + Sync {
    /// Run `callback` once after `after` elapses.
    fn schedule_once(&self, after: Duration, callback: Box<dyn FnOnce() + Send>);

    /// Run `callback` every `interval` until the returned handle cancels.
    fn schedule_periodic(&self, interval: Duration, callback: Box<dyn Fn() + Send>) -> TimerHandle;
}
