//! Envelope for events coming from the native Windows UI.
//!
//! Each event is either a command for the shared reducer, an effect
//! completion from the runner, or a raw native event awaiting
//! translation. The variant shape is identical on every target so
//! `WindowsApp::handle_native` can be written once.

use vault_app::effects::AppEvent;
use vault_app::ids::EffectId;

/// A message from the native Windows UI.
///
/// On Windows, `Command` carries a `UiCommand` produced by a WinUI 3
/// interaction and `Native` wraps the windows-reactor event that hasn't
/// been translated yet. On non-Windows targets this is a type-level stub.
pub enum WindowsMessage {
    /// A command produced by a UI interaction (e.g. a button press).
    Command(vault_app::commands::UiCommand),
    /// The outcome of a previously-issued effect, keyed by its id.
    EffectCompleted { id: EffectId, event: AppEvent },
    /// A raw native event awaiting translation.
    Native(EffectId),
}
