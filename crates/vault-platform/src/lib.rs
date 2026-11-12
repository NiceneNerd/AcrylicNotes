//! Platform service contracts shared by all VaultNotes frontends.
//!
//! The shared application layer (`vault-app`) may request an operation
//! through these traits, but it never calls `xdg-open`, WinRT, COM, shell
//! APIs, dialogs, clipboard APIs, or window APIs directly (restructure
//! plan §3). Each frontend selects its platform's adapters at startup and
//! executes reducer effects through them.
//!
//! Contract rules:
//!
//! * DTOs are plain data — no native handles, no WinRT objects, nothing
//!   toolkit-specific crosses these traits.
//! * Every fallible operation returns an explicit `Result` with a
//!   domain-safe, `Display`-able error.
//! * Signatures are synchronous; frontends wrap them in their own async
//!   machinery where the toolkit demands it (e.g. iced clipboard tasks).
//! * Adapters for services without a consumer yet arrive with the phase
//!   that consumes them (p7 thin-cosmic, p11 windows services); the
//!   trait surface here is the stable contract.

pub mod appearance;
pub mod capabilities;
pub mod clipboard;
pub mod dialog;
pub mod file_drop;
pub mod notification;
pub mod opener;
pub mod testing;
pub mod timer;
pub mod window;

pub use appearance::{AppearanceService, Backdrop};
pub use capabilities::{Capability, CapabilityStatus, PlatformCapabilities};
pub use clipboard::{ClipboardError, ClipboardService};
pub use dialog::{ConfirmDecision, ConfirmPrompt, DialogError, DialogService, FileFilter};
pub use file_drop::{DroppedItem, FileDropError, FileDropService};
pub use notification::NotificationService;
pub use opener::{ExternalOpener, OpenError, OpenTarget};
pub use timer::{TimerHandle, TimerService};
pub use window::WindowService;
