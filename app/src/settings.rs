//! Persistent app settings (M4a: last-used vault), stored as JSON in the XDG
//! config directory. A plain file — not cosmic-config — so the app also works
//! outside a full COSMIC session.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Vault opened last; restored on startup when it still exists.
    #[serde(default)]
    pub last_vault: Option<PathBuf>,
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
        };
        store_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path), Some(settings));

        // Corrupt file -> no settings, not a crash.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_from(&path), None);
    }
}
