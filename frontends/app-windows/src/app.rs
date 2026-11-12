//! Native WinUI 3 frontend for VaultNotes (windows-reactor).
//!
//! This crate compiles to a compile-time skeleton on non-Windows targets so
//! the workspace gates stay green here. Native Windows compilation — and
//! verification — is user-side (no Windows runner in this session; never
//! claimed via Wine). See restructure plan phase 9.

#![cfg_attr(not(target_os = "windows"), allow(dead_code, unused_variables))]

use vault_app::{effects::Effect, App};

use crate::windows_effect_runner::WindowsEffectRunner;
use crate::windows_message::WindowsMessage;

/// The WinUI 3 application host.
///
/// Mirrors the libcosmic `VaultNotes` shell but driven by the shared
/// `vault_app::App` kernel: native events become `UiCommand`s dispatched
/// through `App::dispatch`, the resulting `Effect`s run through
/// `WindowsEffectRunner`, and outcomes are fed back as `AppEvent`s.
///
/// On non-Windows targets this is a skeleton: the type exists and compiles,
/// but no window is created.
pub struct WindowsApp {
    /// The shared application kernel (state + reducer).
    pub shared: App,
    /// Executes effects produced by the reducer and maps outcomes into
    /// `AppEvent`s delivered back to `App::handle`.
    pub runner: WindowsEffectRunner,
}

impl WindowsApp {
    /// Construct a new Windows application for `vault`.
    #[cfg(target_os = "windows")]
    pub fn new() -> Self {
        Self {
            shared: App::default(),
            runner: WindowsEffectRunner::new(),
        }
    }

    /// Construct a new Windows application for `vault`.
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Self {
        Self {
            shared: App::default(),
            runner: WindowsEffectRunner,
        }
    }

    /// Handle a native Windows event, translating it into a `UiCommand`
    /// and running the resulting effects.
    #[cfg(target_os = "windows")]
    pub fn handle_native(&mut self, message: WindowsMessage) -> Vec<Effect> {
        let effects = match message {
            WindowsMessage::Command(cmd) => self.shared.dispatch(cmd),
            WindowsMessage::EffectCompleted { id, event } => {
                self.shared.handle(event);
                Vec::new()
            }
            WindowsMessage::Native(_) => Vec::new(),
        };
        self.runner.run(effects);
        effects
    }

    /// Handle a native Windows event (skeleton on non-Windows).
    #[cfg(not(target_os = "windows"))]
    pub fn handle_native(&mut self, _message: WindowsMessage) -> Vec<Effect> {
        Vec::new()
    }
}

impl Default for WindowsApp {
    fn default() -> Self {
        Self::new()
    }
}
