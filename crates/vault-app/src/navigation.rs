//! Navigation view-model types shared by both frontends.

use serde::{Deserialize, Serialize};

/// Which sidebar panel is active. Rendering and panel-specific widgets are
/// native; the active section itself is shared state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavSection {
    /// The file tree.
    #[default]
    #[serde(rename = "files")]
    Files,
    /// Vault-wide text search.
    #[serde(rename = "search")]
    Search,
    /// Tag list with counts and per-tag note lists.
    #[serde(rename = "tags")]
    Tags,
    /// Backlinks of the active note.
    #[serde(rename = "links")]
    Links,
    /// Headings of the active note.
    #[serde(rename = "outline")]
    Outline,
}

impl NavSection {
    /// Every section, in the order the sidebar lists them.
    pub const ALL: [NavSection; 5] = [
        NavSection::Files,
        NavSection::Search,
        NavSection::Tags,
        NavSection::Links,
        NavSection::Outline,
    ];

    /// The user-facing name of this section.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Search => "Search",
            Self::Tags => "Tags",
            Self::Links => "Links",
            Self::Outline => "Outline",
        }
    }
}
