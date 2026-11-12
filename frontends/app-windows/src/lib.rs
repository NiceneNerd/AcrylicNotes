//! Native WinUI 3 frontend for VaultNotes (windows-reactor).
//!
//! Compiles to a skeleton on non-Windows targets; native Windows UI is
//! user-side (no Windows runner in this session).

pub mod app;
pub mod windows_effect_runner;
pub mod windows_message;

pub use app::WindowsApp;
