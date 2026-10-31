//! Access to a note's source text, and Obsidian's own settings.
//!
//! Frontends and the markdown pipeline need the raw text of other notes
//! (transclusion). [`NoteSource`] is the read-only contract; [`Vault`]
//! implements it by reading from disk, so embeds always reflect the file
//! state without duplicating note bodies in memory.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::vault::Vault;

/// Read-only access to the raw text of notes in a vault.
pub trait NoteSource {
    /// The full raw text of the note at `path` (vault-relative, '/'-separated),
    /// or `None` if it does not exist or cannot be read.
    fn note_source(&self, path: &str) -> Option<String>;
}

impl NoteSource for Vault {
    fn note_source(&self, path: &str) -> Option<String> {
        // Refuse dot-paths the same way the scan does.
        if path.split('/').any(|c| c.starts_with('.') && c.len() > 1) {
            return None;
        }
        fs::read_to_string(self.root().join(path)).ok()
    }
}

/// The bits of Obsidian's `.obsidian/app.json` that change rendering.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObsidianSettings {
    /// Obsidian's "Strict line breaks" setting: when true, a single newline
    /// does NOT become a line break in reading view.
    #[serde(rename = "strictLineBreaks", default)]
    pub strict_line_breaks: bool,
    // Future: attachmentFolderPath, newLinkFormat, daily-notes settings.
}

impl ObsidianSettings {
    /// Load `.obsidian/app.json` from a vault root; missing or unreadable
    /// settings fall back to Obsidian's defaults.
    pub fn load(vault_root: impl AsRef<Path>) -> Self {
        let path = vault_root.as_ref().join(".obsidian").join("app.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn loads_strict_line_breaks() {
        let tmp = TempDir::new().unwrap();
        let obs = tmp.path().join(".obsidian");
        fs::create_dir(&obs).unwrap();
        fs::write(obs.join("app.json"), r#"{"strictLineBreaks": true}"#).unwrap();
        let settings = ObsidianSettings::load(tmp.path());
        assert!(settings.strict_line_breaks);
    }

    #[test]
    fn missing_settings_are_defaults() {
        let tmp = TempDir::new().unwrap();
        assert!(!ObsidianSettings::load(tmp.path()).strict_line_breaks);
    }
}
