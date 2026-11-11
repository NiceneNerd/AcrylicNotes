//! Effects the reducer requests from the platform or persistence layer.
//!
//! The reducer never performs I/O itself: it returns [`Effect`] values and
//! the frontend's effect runner executes them, feeding results back as
//! [`AppEvent`]s. Effects that can race (saves, reloads) carry a stable
//! [`EffectId`] so a late completion can be checked against the current
//! state before it is applied (restructure plan §5.3).

use serde::{Deserialize, Serialize};

use crate::ids::{EffectId, TabId};

/// What the effect runner should perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    /// Persist `default_pane` (and whatever else the settings DTO now
    /// holds) after a settings-affecting command.
    #[serde(rename = "persist-settings")]
    PersistSettings,
    /// Save the note shown in `tab`. The runner reports the outcome with
    /// `AppEvent::SaveCompleted` / `SaveFailed` carrying the same id.
    #[serde(rename = "save-tab")]
    SaveTab { tab: TabId },
    /// Re-read the note shown in `tab` from disk and feed its text back
    /// with `AppEvent::ReloadCompleted` (the reducer applies it only when
    /// the tab still exists).
    #[serde(rename = "reload-tab")]
    ReloadTab { tab: TabId },
    /// Execute the vault-wide search for `query`, reporting the hit list
    /// with `AppEvent::SearchCompleted` keyed by `request`. Stale
    /// requests are rejected by the shared session, not the runner.
    #[serde(rename = "search-vault")]
    SearchVault {
        request: crate::ids::SearchRequestId,
        query: String,
    },
}

/// One requested side effect, identified so late completions are checkable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub id: EffectId,
    pub kind: EffectKind,
}

impl Effect {
    pub(crate) fn new(id: EffectId, kind: EffectKind) -> Self {
        Self { id, kind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_and_events_round_trip_through_the_wire_format() {
        // The Windows bridge serializes effects and events the same way
        // the cosmic runner passes them in-process; both must round trip.
        let effect = Effect::new(
            EffectId::new(9),
            EffectKind::SearchVault {
                request: crate::ids::SearchRequestId::new(3),
                query: "needle".into(),
            },
        );
        let json = serde_json::to_string(&effect).unwrap();
        assert_eq!(serde_json::from_str::<Effect>(&json).unwrap(), effect);

        let events = vec![
            AppEvent::SaveCompleted {
                effect: EffectId::new(1),
            },
            AppEvent::SaveFailed {
                effect: EffectId::new(2),
                error: "changed on disk".into(),
            },
            AppEvent::ReloadCompleted {
                effect: EffectId::new(3),
                text: "fresh".into(),
            },
            AppEvent::SearchCompleted {
                request: crate::ids::SearchRequestId::new(3),
                hits: vec![crate::search::SearchHitView {
                    path: "a.md".into(),
                    line_no: 4,
                    line: "needle".into(),
                }],
            },
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(
                serde_json::from_str::<AppEvent>(&json).unwrap(),
                event,
                "round trip failed for {json}"
            );
        }
    }
}

/// Results of executed effects, fed back into the reducer by the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppEvent {
    /// The save for `effect` succeeded.
    #[serde(rename = "save-completed")]
    SaveCompleted { effect: EffectId },
    /// The save for `effect` failed (refused as externally modified, I/O
    /// error, ...). The message is user-facing text for the error line.
    #[serde(rename = "save-failed")]
    SaveFailed { effect: EffectId, error: String },
    /// The reload for `effect` read the note's current disk text.
    #[serde(rename = "reload-completed")]
    ReloadCompleted { effect: EffectId, text: String },
    /// A vault-wide search finished. Keyed by the search request (not an
    /// effect id): the session drops results from older requests.
    #[serde(rename = "search-completed")]
    SearchCompleted {
        request: crate::ids::SearchRequestId,
        hits: Vec<crate::search::SearchHitView>,
    },
}
