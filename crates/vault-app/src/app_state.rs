//! The shared application state both frontends render.
//!
//! This is the single source of truth for everything the thin frontends
//! display: open tabs and their order, the active pane and sidebar
//! section, and which tabs have unsaved edits conflicting with disk.
//! Native widgets hold no duplicate decisions (restructure plan §5.1).

use crate::ids::TabId;
use crate::navigation::NavSection;
use crate::settings::PaneMode;
use crate::tabs::Tabs;

/// The kernel's application state.
#[derive(Debug, Default)]
pub struct AppState {
    /// The ordered open tabs with one active tab.
    pub tabs: Tabs,
    /// The pane layout currently rendered (not the startup default).
    pub pane: PaneMode,
    /// Whether the sidebar is shown.
    pub sidebar_open: bool,
    /// Which sidebar panel is active.
    pub panel: NavSection,
    /// Tabs whose note changed on disk while they had unsaved edits
    /// (conflict banner instances).
    pub conflicts: Vec<TabId>,
    /// The last error to surface on the dismissible error line.
    pub last_error: Option<String>,
}

impl AppState {
    /// Whether `tab` currently has an unresolved external-change conflict.
    #[must_use]
    pub fn is_conflicted(&self, tab: TabId) -> bool {
        self.conflicts.contains(&tab)
    }
}
