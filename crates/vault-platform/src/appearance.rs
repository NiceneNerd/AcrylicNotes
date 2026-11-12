//! Visual appearance: what backdrop the platform can offer.
//!
//! A neutral two-value contract is enough for every current consumer: the
//! app asks "does this platform prefer a blurred backdrop?", and the
//! toolkit request (Wayland `ext-background-effect-v1`, WinUI backdrop
//! materials) is native detail inside the adapter.

use serde::{Deserialize, Serialize};

/// The backdrop style the platform prefers for app windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backdrop {
    /// Paint an opaque surface; no backdrop effect requested.
    #[serde(rename = "opaque")]
    Opaque,
    /// Ask the platform/toolkit for a blurred backdrop. Whether the
    /// compositor actually blurs stays its decision (COSMIC's
    /// frosted-windows setting); lack of blur is never an error.
    #[serde(rename = "blur")]
    Blur,
}

/// Reports the platform's visual appearance preferences.
pub trait AppearanceService: Send + Sync {
    /// The backdrop the platform prefers right now.
    fn preferred_backdrop(&self) -> Backdrop;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrop_round_trips_through_the_wire_format() {
        for backdrop in [Backdrop::Opaque, Backdrop::Blur] {
            let json = serde_json::to_string(&backdrop).unwrap();
            assert_eq!(
                serde_json::from_str::<Backdrop>(&json).unwrap(),
                backdrop,
                "round trip failed for {json}"
            );
        }
    }
}
