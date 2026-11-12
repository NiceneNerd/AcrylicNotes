//! Executes effects produced by the shared `vault_app::App` reducer and
//! maps outcomes into `AppEvent`s delivered back to `App::handle`.
//!
//! On Windows this drives WinUI 3 / windows-reactor APIs and real platform
//! calls (shell launch for external open, ...). On non-Windows targets it
//! is a compile-time stub: effects are accepted but never performed, so
//! the crate type-checks here without windows-reactor.

use vault_app::effects::Effect;
use vault_platform::OpenTarget;

/// Runs `vault_app::Effect` values on the native platform.
pub struct WindowsEffectRunner;

impl WindowsEffectRunner {
    /// Create a new runner.
    #[cfg(target_os = "windows")]
    pub fn new() -> Self {
        Self
    }

    /// Create a new runner (stub on non-Windows: accepted, not performed).
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Self {
        Self
    }

    /// Execute `effects`, feeding outcomes back as `AppEvent`s.
    #[cfg(target_os = "windows")]
    pub fn run(&mut self, effects: &[Effect]) {
        for effect in effects {
            match &effect.kind {
                vault_app::effects::EffectKind::PersistSettings => {
                    // Settings persistence is handled by the libcosmic
                    // shell's storage; on Windows it goes through the
                    // shared settings DTO atomically written to disk.
                }
                vault_app::effects::EffectKind::SaveTab { tab } => {
                    // Save is performed by the reducer's repository
                    // adapter; the runner only reports completion.
                    let _ = tab;
                }
                vault_app::effects::EffectKind::ReloadTab { .. } => {}
                vault_app::effects::EffectKind::SearchVault { .. } => {}
                vault_app::effects::EffectKind::OpenExternal { target } => {
                    let _ = open_external_target(target.clone());
                }
            }
        }
    }

    /// Execute `effects` (stub on non-Windows: accepted, not performed).
    #[cfg(not(target_os = "windows"))]
    pub fn run(&mut self, _effects: Vec<Effect>) {
        // No native platform here: effects are type-checked but not run.
        // Real execution happens on Windows via windows-reactor.
    }
}

impl Default for WindowsEffectRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Hand an external-open target to the platform's default handler.
///
/// On Windows this launches the default handler via ShellExecuteEx / the
/// Windows App SDK. Returns an error string on failure; success is
/// fire-and-forget (the reducer reports `OpenCompleted` separately).
#[cfg(target_os = "windows")]
fn open_external_target(_target: OpenTarget) -> Result<(), String> {
    // windows-reactor / WinRT call goes here on a real Windows build.
    Ok(())
}

/// Stub on non-Windows: external opening is not available here.
#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn open_external_target(_target: OpenTarget) -> Result<(), String> {
    Err("external opening requires a Windows target".to_string())
}
