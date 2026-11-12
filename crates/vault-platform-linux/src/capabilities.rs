//! Capability flags for the Linux adapters, used for graceful fallback.
//!
//! Honest by construction: only what this crate actually implements is
//! reported available. The clipboard/dialog/timer/window/notification/
//! file-drop adapters arrive with the phases that consume them (p7/p11),
//! and the reasons say so.

use vault_platform::{Capability, CapabilityStatus, PlatformCapabilities};

/// The Linux capability report.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxCapabilities;

impl PlatformCapabilities for LinuxCapabilities {
    fn status(&self, capability: Capability) -> CapabilityStatus {
        match capability {
            Capability::ExternalOpening => CapabilityStatus::Available,
            Capability::Backdrop => CapabilityStatus::Available,
            Capability::Clipboard
            | Capability::Dialogs
            | Capability::Timers
            | Capability::Notifications
            | Capability::FileDrop => CapabilityStatus::Unsupported {
                reason: "Linux adapter pending (thin-shell phase p7 / windows p11)".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implemented_adapters_are_available_and_the_rest_explain_themselves() {
        let capabilities = LinuxCapabilities;
        assert!(capabilities
            .status(Capability::ExternalOpening)
            .is_available());
        assert!(capabilities.status(Capability::Backdrop).is_available());

        for pending in [
            Capability::Clipboard,
            Capability::Dialogs,
            Capability::Timers,
            Capability::Notifications,
            Capability::FileDrop,
        ] {
            let status = capabilities.status(pending);
            assert!(!status.is_available(), "{pending:?} has no adapter yet");
            assert!(status.unsupported_reason().is_some());
        }
    }
}
