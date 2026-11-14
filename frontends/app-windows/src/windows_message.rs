//! Envelope for events coming from the native Windows UI.
//!
//! Each message is either a command for the shared reducer, an effect
//! outcome reported back by the runner (late, real async I/O), or a raw
//! native event the shell translates itself. The shape is identical on
//! every target, so `WindowsApp::handle_native` is written once and
//! unit-tested on Linux.

use vault_app::commands::UiCommand;
use vault_app::effects::AppEvent;

/// A message from the native Windows UI.
pub enum WindowsMessage {
    /// A command produced by a UI interaction (button press, selection,
    /// keyboard shortcut). The kernel decides everything.
    Command(UiCommand),
    /// The outcome of a previously-issued effect. `AppEvent` carries the
    /// `EffectId` it belongs to, so no separate id is needed here (the
    /// kernel rejects stale ids itself).
    EffectCompleted(AppEvent),
    /// A raw native event awaiting the shell's translation.
    Native(NativeEvent),
}

/// A raw native event with no direct `UiCommand` equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEvent {
    /// The user asked to close the window. Dirty buffers are flushed
    /// through the same guarded-save path the tab-close workflow uses
    /// (WIN-108 semantics: closing never silently discards edits).
    WindowClosing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_envelope_carries_each_kind() {
        let messages = [
            WindowsMessage::Command(UiCommand::SaveActiveTab),
            WindowsMessage::EffectCompleted(AppEvent::SaveCompleted {
                effect: vault_app::ids::EffectId::new(7),
            }),
            WindowsMessage::Native(NativeEvent::WindowClosing),
        ];
        // A smoke pass over the variants; behavior lives in app.rs tests.
        assert!(matches!(messages[0], WindowsMessage::Command(_)));
        assert!(matches!(messages[1], WindowsMessage::EffectCompleted(_)));
        assert!(matches!(messages[2], WindowsMessage::Native(_)));
    }
}
