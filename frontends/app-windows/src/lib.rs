//! Native WinUI 3 frontend for VaultNotes (windows-reactor).
//!
//! Compiles to a skeleton on non-Windows targets; native Windows UI is
//! user-side (no Windows runner in this session).

pub mod app;
pub mod file_tree;
pub mod reading;
pub mod source_mode;
pub mod split_mode;
pub mod windows_effect_runner;
pub mod windows_message;

pub use app::WindowsApp;

// Exercise the pinned windows-reactor dependency even before the native
// window loop lands: if the pin breaks (version, features, target support),
// the cross-target check below fails instead of the breakage surfacing on a
// real Windows machine. The cross-check is `cargo xtask check-windows-ui`.
#[cfg(target_os = "windows")]
use windows_reactor as _;
