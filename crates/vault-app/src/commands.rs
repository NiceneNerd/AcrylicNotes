//! User intents as serializable commands (restructure plan §5.2).
//!
//! Frontends convert native widget events into `UiCommand` values and hand
//! them to [`crate::App::dispatch`]. The command set grows slice by slice as
//! behavior moves out of the cosmic frontend; every variant must stay
//! toolkit-neutral (no widget ids, no colors, no native handles).

use serde::{Deserialize, Serialize};

use crate::ids::TabId;
use crate::navigation::NavSection;
use crate::persistence::ConflictDecision;
use crate::settings::PaneMode;

/// One user intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiCommand {
    /// Choose the pane layout (header segmented control).
    #[serde(rename = "set-pane-mode")]
    SetPaneMode(PaneMode),
    /// Show or hide the sidebar.
    #[serde(rename = "set-sidebar-visible")]
    SetSidebarVisible(bool),
    /// Switch the sidebar's active panel.
    #[serde(rename = "set-nav-section")]
    SetNavSection(NavSection),
    /// Open a note (focuses its existing tab instead of duplicating it).
    #[serde(rename = "open-note")]
    OpenNote { path: String },
    /// Activate the tab at a display position.
    #[serde(rename = "activate-tab")]
    ActivateTab { position: usize },
    /// Close the tab at a display position (the close-save workflow
    /// arrives with the persistence slice; until then the tab closes
    /// without a save step).
    #[serde(rename = "close-tab")]
    CloseTab { position: usize },
    /// Save the active tab's note.
    #[serde(rename = "save-active-tab")]
    SaveActiveTab,
    /// Resolve an external-change conflict for a tab.
    #[serde(rename = "resolve-conflict")]
    ResolveConflict {
        tab: TabId,
        decision: ConflictDecision,
    },
}
