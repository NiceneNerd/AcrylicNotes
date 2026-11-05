//! Persistent app settings (M4a: last-used vault; M4c: theme + glass),
//! stored as JSON in the XDG config directory. A plain file — not
//! cosmic-config — so the app also works outside a full COSMIC session.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Which visual theme the app uses (M4c). `System` follows the desktop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "dark")]
    Dark,
    #[serde(rename = "light")]
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Vault opened last; restored on startup when it still exists.
    #[serde(default)]
    pub last_vault: Option<PathBuf>,
    /// UI theme choice; `system` follows the desktop (M4c).
    #[serde(default)]
    pub theme: ThemeChoice,
    /// Whether to request a frosted (blur) backdrop from the platform (M4c).
    #[serde(default = "default_true")]
    pub glass_enabled: bool,
    /// Hide the `.md` extension on note rows in the file tree (Obsidian's
    /// default presentation).
    #[serde(default = "default_true")]
    pub hide_md_extension: bool,
    /// Follow the editor caret in the reading pane while it is visible
    /// (Split mode): the preview scrolls to the block containing the
    /// caret's line.
    #[serde(default = "default_true")]
    pub scroll_sync: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_vault: None,
            theme: ThemeChoice::default(),
            glass_enabled: true,
            hide_md_extension: true,
            scroll_sync: true,
        }
    }
}

/// `$XDG_CONFIG_HOME/vault-handoff/settings.json`.
fn config_file() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("vault-handoff").join("settings.json"))
}

/// Load settings, falling back to defaults when missing or unreadable.
pub fn load() -> AppSettings {
    config_file()
        .and_then(|path| load_from(&path))
        .unwrap_or_default()
}

/// Persist settings; failures are non-fatal (settings are a convenience).
pub fn store(settings: &AppSettings) {
    if let Some(path) = config_file() {
        let _ = store_to(&path, settings);
    }
}

fn load_from(path: &Path) -> Option<AppSettings> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write atomically (tmp + rename) so a crash cannot truncate the file.
fn store_to(path: &Path, settings: &AppSettings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_corruption_fallback() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("settings.json");

        // Missing file -> no settings yet.
        assert_eq!(load_from(&path), None);

        // Roundtrip.
        let settings = AppSettings {
            last_vault: Some(PathBuf::from("/home/nn/Notes")),
            theme: ThemeChoice::Dark,
            glass_enabled: true,
            hide_md_extension: true,
            scroll_sync: false,
        };
        store_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path), Some(settings));

        // Corrupt file -> no settings, not a crash.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_from(&path), None);
    }

    #[test]
    fn missing_fields_take_safe_defaults() {
        // A pre-M4c settings file has only last_vault: glass stays enabled
        // and the theme follows the system.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, r#"{"last_vault": "/notes"}"#).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.last_vault, Some(PathBuf::from("/notes")));
        assert_eq!(loaded.theme, ThemeChoice::System);
        assert!(loaded.glass_enabled);
        // The `.md`-hiding default applies to files that predate the field.
        assert!(loaded.hide_md_extension);
        // Scroll sync defaults on for files that predate the field.
        assert!(loaded.scroll_sync);

        // A glass_enabled=false must survive the roundtrip.
        let path2 = tmp.path().join("settings2.json");
        let settings = AppSettings {
            glass_enabled: false,
            ..AppSettings::default()
        };
        store_to(&path2, &settings).unwrap();
        assert!(!load_from(&path2).unwrap().glass_enabled);
    }
}
