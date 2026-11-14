//! File-backed settings storage for the native frontend.
//!
//! The settings *document* and its DTO live in `vault-app` (shared with
//! the Linux shell); this module owns this platform's storage: a JSON
//! file in the platform config directory, written atomically (tmp +
//! rename) so a crash cannot truncate it. Semantics mirror the Linux
//! adapter (`app/src/settings.rs`): a missing, unreadable, or corrupt
//! file falls back to defaults, and unknown keys are ignored by the
//! shared serde contract.
//!
//! Everything here is plain cross-platform code, so the store is fully
//! tested on Linux (tempdir round trips); the only Windows-specific
//! part is where the config directory lives, which `dirs` resolves.

use std::fs;
use std::path::{Path, PathBuf};

use vault_app::settings::{AppSettings, SettingsStore};

/// A [`SettingsStore`] backed by one JSON file.
pub struct FileSettingsStore {
    path: PathBuf,
}

impl FileSettingsStore {
    /// A store backed by `path` (the file need not exist yet).
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default per-user location:
    /// `%APPDATA%\vault-handoff\settings.json` on Windows (and
    /// `$XDG_CONFIG_HOME/vault-handoff/settings.json` elsewhere, the
    /// same directory the Linux shell uses).
    #[must_use]
    pub fn in_config_dir() -> Option<Self> {
        dirs::config_dir().map(|dir| Self::at(dir.join("vault-handoff").join("settings.json")))
    }

    /// The file this store reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the document atomically (tmp + rename).
    fn write(&self, settings: &AppSettings) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(settings)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)
    }
}

impl SettingsStore for FileSettingsStore {
    fn load(&self) -> AppSettings {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn store(&mut self, settings: &AppSettings) {
        // Non-fatal by contract: settings are a convenience, and the
        // last good document keeps loading.
        let _ = self.write(settings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_app::settings::{PaneMode, ThemeChoice};

    #[test]
    fn settings_round_trip_through_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let mut store = FileSettingsStore::at(&path);

        let document = AppSettings {
            theme: ThemeChoice::Dark,
            default_pane: PaneMode::Reading,
            ..AppSettings::default()
        };
        store.store(&document);

        assert!(path.is_file(), "the nested parent directory was created");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "the atomic tmp file was renamed into place"
        );
        let loaded = store.load();
        assert_eq!(loaded.theme, ThemeChoice::Dark);
        assert_eq!(loaded.default_pane, PaneMode::Reading);
        // Fields this build does not touch keep their defaults.
        assert_eq!(loaded.reading_font_size, 16);
    }

    #[test]
    fn a_missing_file_loads_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = FileSettingsStore::at(dir.path().join("settings.json"));
        assert_eq!(store.load(), AppSettings::default());
    }

    #[test]
    fn a_corrupt_file_loads_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        let store = FileSettingsStore::at(&path);
        assert_eq!(store.load(), AppSettings::default());
    }

    #[test]
    fn load_modify_store_preserves_fields_the_caller_does_not_own() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = FileSettingsStore::at(dir.path().join("settings.json"));
        store.store(&AppSettings {
            theme: ThemeChoice::Light,
            default_pane: PaneMode::Live,
            ..AppSettings::default()
        });

        // The runner's pattern: load, change one field, store back.
        let mut document = store.load();
        document.default_pane = PaneMode::Split;
        store.store(&document);

        let loaded = store.load();
        assert_eq!(loaded.default_pane, PaneMode::Split);
        assert_eq!(loaded.theme, ThemeChoice::Light, "the other field survives");
    }
}
