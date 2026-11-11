//! Shared application kernel for the VaultNotes frontends.
//!
//! This crate owns user-visible behavior without any native widgets. Frontends
//! convert native events into [`ids`]-keyed commands, a deterministic reducer
//! mutates shared state, and the reducer emits effects for platform or
//! persistence adapters. See the cross-platform GUI restructure plan.
//!
//! Layering rule (restructure plan §3.2): this crate must never depend on
//! libcosmic, iced, windows, windows-reactor, HWND/COM types, or any frontend
//! message enum.

pub mod ids;
pub mod navigation;
pub mod persistence;
pub mod settings;
pub mod tabs;
