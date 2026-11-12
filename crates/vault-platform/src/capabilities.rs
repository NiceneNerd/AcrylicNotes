//! Platform capability flags with reasons.
//!
//! Used for graceful fallback: a frontend asks up front what its platform
//! can do through the traits and adjusts its UI (visible fallbacks per
//! plan §18), instead of failing per operation.

use serde::{Deserialize, Serialize};

/// A platform service the app may rely on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "external-opening")]
    ExternalOpening,
    #[serde(rename = "clipboard")]
    Clipboard,
    #[serde(rename = "dialogs")]
    Dialogs,
    #[serde(rename = "timers")]
    Timers,
    #[serde(rename = "notifications")]
    Notifications,
    #[serde(rename = "file-drop")]
    FileDrop,
    #[serde(rename = "backdrop")]
    Backdrop,
}

/// Whether a capability is usable, and why not when it is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityStatus {
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "unsupported")]
    Unsupported { reason: String },
}

impl CapabilityStatus {
    /// Whether the capability can be used.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    /// The reason a capability is unavailable, if it is.
    #[must_use]
    pub fn unsupported_reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

/// Reports what the platform can do through the service traits.
pub trait PlatformCapabilities: Send + Sync {
    fn status(&self, capability: Capability) -> CapabilityStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_status_reports_availability_and_reasons() {
        let available = CapabilityStatus::Available;
        assert!(available.is_available());
        assert_eq!(available.unsupported_reason(), None);

        let unsupported = CapabilityStatus::Unsupported {
            reason: "no file manager integration".into(),
        };
        assert!(!unsupported.is_available());
        assert_eq!(
            unsupported.unsupported_reason(),
            Some("no file manager integration")
        );
    }

    #[test]
    fn capabilities_round_trip_through_the_wire_format() {
        for capability in [
            Capability::ExternalOpening,
            Capability::Clipboard,
            Capability::Dialogs,
            Capability::Timers,
            Capability::Notifications,
            Capability::FileDrop,
            Capability::Backdrop,
        ] {
            let json = serde_json::to_string(&capability).unwrap();
            assert_eq!(
                serde_json::from_str::<Capability>(&json).unwrap(),
                capability
            );
        }
    }
}
