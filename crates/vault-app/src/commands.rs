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
use vault_platform::OpenTarget;

/// One user intent.
///
/// Internally tagged (`{"type": "save-active-tab"}`) so the wire format is
/// self-describing: the Windows bridge can log, forward, and inspect
/// commands without matching every variant. Every variant is a struct
/// variant (serde's internal tag cannot wrap bare primitives).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UiCommand {
    /// Choose the pane layout (header segmented control).
    #[serde(rename = "set-pane-mode")]
    SetPaneMode { mode: PaneMode },
    /// Show or hide the sidebar.
    #[serde(rename = "set-sidebar-visible")]
    SetSidebarVisible { open: bool },
    /// Switch the sidebar's active panel.
    #[serde(rename = "set-nav-section")]
    SetNavSection { section: NavSection },
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
    /// Save the notes of the listed tabs (the shell decides which: it
    /// owns the dirty buffers; the kernel tracks the effects). The
    /// window-close and close-all flows flush dirty tabs through this.
    #[serde(rename = "save-tabs")]
    SaveTabs { tabs: Vec<TabId> },
    /// Resolve an external-change conflict for a tab.
    #[serde(rename = "resolve-conflict")]
    ResolveConflict {
        tab: TabId,
        decision: ConflictDecision,
    },
    /// Record the vault-wide search query being typed.
    #[serde(rename = "set-search-query")]
    SetSearchQuery { query: String },
    /// Run the vault-wide search for a query (the runner executes the
    /// scan and reports `AppEvent::SearchCompleted`).
    #[serde(rename = "search-vault")]
    SearchVault { query: String },
    /// Change the pane mode new sessions start in (the settings
    /// snapshot), emitting `EffectKind::PersistSettings`. Unlike
    /// [`UiCommand::SetPaneMode`] this does not change the currently
    /// rendered pane — the Linux shell's `SettingsDefaultPane` split.
    #[serde(rename = "set-default-pane")]
    SetDefaultPane { mode: crate::settings::PaneMode },
    /// Hand a URL or file to the platform's default handler
    /// (`EffectKind::OpenExternal`, reported via `AppEvent::Open…`).
    #[serde(rename = "open-external")]
    OpenExternal { target: OpenTarget },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_round_trips_through_its_persisted_names() {
        let commands = vec![
            UiCommand::SetPaneMode {
                mode: crate::settings::PaneMode::Split,
            },
            UiCommand::SetSidebarVisible { open: false },
            UiCommand::SetNavSection {
                section: crate::navigation::NavSection::Outline,
            },
            UiCommand::OpenNote {
                path: "notes/a.md".into(),
            },
            UiCommand::ActivateTab { position: 2 },
            UiCommand::CloseTab { position: 0 },
            UiCommand::SaveActiveTab,
            UiCommand::SaveTabs {
                tabs: vec![crate::ids::TabId::new(9), crate::ids::TabId::new(11)],
            },
            UiCommand::ResolveConflict {
                tab: crate::ids::TabId::new(4),
                decision: crate::persistence::ConflictDecision::ReloadFromDisk,
            },
            UiCommand::SetSearchQuery {
                query: "needle".into(),
            },
            UiCommand::SearchVault {
                query: "needle".into(),
            },
            UiCommand::OpenExternal {
                target: vault_platform::OpenTarget::Url("https://example.com".into()),
            },
        ];
        for command in commands {
            let json = serde_json::to_string(&command).unwrap();
            assert_eq!(
                serde_json::from_str::<UiCommand>(&json).unwrap(),
                command,
                "round trip failed for {json}"
            );
            // The wire names are kebab-case tags, not enum variant names.
            assert!(json.starts_with('{'), "tagged enum shape expected: {json}");
        }
    }
}
