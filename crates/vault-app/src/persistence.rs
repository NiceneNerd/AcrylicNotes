//! Persistence decision types shared by both frontends.
//!
//! Destructive and persistence decisions must live in shared workflows,
//! not in duplicated native click handlers (restructure plan §21). The
//! conflict banner's three choices are the first such decision type.

use serde::{Deserialize, Serialize};

/// What to do when a tab's note changed on disk while it had unsaved
/// edits (the conflict banner's choices).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictDecision {
    /// Discard the local edits and reload the note from disk (undo
    /// history is dropped too).
    #[serde(rename = "reload-from-disk")]
    ReloadFromDisk,
    /// Keep the local edits in the buffer without saving.
    #[serde(rename = "keep-mine")]
    KeepMine,
    /// Save the local edits, deliberately overwriting the external
    /// change (a fresh disk snapshot becomes the save's sentinel).
    #[serde(rename = "save-mine-anyway")]
    SaveMineAnyway,
}

impl ConflictDecision {
    /// Every decision, in the order the banner lists them.
    pub const ALL: [ConflictDecision; 3] = [
        ConflictDecision::ReloadFromDisk,
        ConflictDecision::KeepMine,
        ConflictDecision::SaveMineAnyway,
    ];

    /// The user-facing label of this choice.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ReloadFromDisk => "Reload from disk",
            Self::KeepMine => "Keep mine",
            Self::SaveMineAnyway => "Save mine anyway",
        }
    }
}

/// What to do with links that would break after a rename/move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkUpdateDecision {
    /// Rewrite the affected wikilinks/embeds in every referrer.
    #[serde(rename = "update-links")]
    UpdateLinks,
    /// Leave the links pointing at their old spelling.
    #[serde(rename = "leave-as-is")]
    LeaveAsIs,
}

impl LinkUpdateDecision {
    /// The user-facing label of this choice.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::UpdateLinks => "Update links",
            Self::LeaveAsIs => "Leave as is",
        }
    }
}

/// What the watcher poll decides to do about one externally changed tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalChangeAction {
    /// Content-identical change (e.g. an mtime-only `touch`): nothing to do.
    #[serde(rename = "none")]
    None,
    /// Real content change with no local edits: auto-reload + info banner.
    #[serde(rename = "reloaded")]
    Reloaded,
    /// Real content change with unsaved edits: conflict banner.
    #[serde(rename = "conflict")]
    Conflict,
}

/// Classify an external change against a tab's base text: identical
/// content is silent; otherwise dirty tabs conflict and clean tabs reload.
#[must_use]
pub fn classify_external_change(disk: &str, base: &str, tab_dirty: bool) -> ExternalChangeAction {
    if disk == base {
        ExternalChangeAction::None
    } else if tab_dirty {
        ExternalChangeAction::Conflict
    } else {
        ExternalChangeAction::Reloaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_round_trip_and_have_labels() {
        for decision in ConflictDecision::ALL {
            let json = serde_json::to_string(&decision).unwrap();
            assert_eq!(
                serde_json::from_str::<ConflictDecision>(&json).unwrap(),
                decision
            );
            assert!(!decision.label().is_empty());
        }
        assert!(!LinkUpdateDecision::UpdateLinks.label().is_empty());
        assert_eq!(
            serde_json::to_string(&LinkUpdateDecision::LeaveAsIs).unwrap(),
            r#""leave-as-is""#
        );
    }

    #[test]
    fn external_changes_classify_by_content_and_dirtiness() {
        use ExternalChangeAction::{Conflict, None as Silent, Reloaded};
        // Identical content is always silent (an mtime-only touch).
        assert_eq!(classify_external_change("same", "same", false), Silent);
        assert_eq!(classify_external_change("same", "same", true), Silent);
        // Real change: clean tabs reload, dirty tabs conflict.
        assert_eq!(classify_external_change("disk", "base", false), Reloaded);
        assert_eq!(classify_external_change("disk", "base", true), Conflict);
    }
}
