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
    /// Insert closing brackets/quotes and type over them (M4d).
    #[serde(default = "default_true")]
    pub auto_pair: bool,
    /// Suggest note titles while typing `[[` (M4d).
    #[serde(default = "default_true")]
    pub wiki_autocomplete: bool,
    /// Soft-wrap long lines in the editor.
    #[serde(default = "default_true")]
    pub word_wrap: bool,
    /// Edit in the monospace font (off = the system UI font).
    #[serde(default = "default_true")]
    pub editor_font_mono: bool,
    /// Editor font size in pixels.
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: u16,
    /// Whether the sidebar (file tree and other panels) is shown. A
    /// toggle button in the titlebar flips it.
    #[serde(default = "default_true")]
    pub sidebar_open: bool,
    /// Seconds of editor idle before autosave fires.
    #[serde(default = "default_autosave_secs")]
    pub autosave_secs: u64,
    /// The pane layout a fresh session starts in.
    #[serde(default)]
    pub default_pane: PaneMode,
    /// Non-markdown file extensions shown in the file tree (lowercase,
    /// without the dot); empty means every attachment shows.
    #[serde(default)]
    pub attachment_extensions: Vec<String>,
    /// How generated wikilinks spell their target (Obsidian's "new
    /// link format" setting).
    #[serde(default)]
    pub link_path_format: LinkPathFormat,
}

/// How generated wikilinks refer to their target, mirroring Obsidian's
/// "new link format" preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkPathFormat {
    /// The bare title; the full vault path only when several notes
    /// share the title (Obsidian's default).
    #[default]
    #[serde(rename = "shortest")]
    Shortest,
    /// The path from the current note's folder (`../folder/note`).
    #[serde(rename = "relative")]
    Relative,
    /// The full vault path.
    #[serde(rename = "absolute")]
    Absolute,
}

/// How the note area is laid out. Lives here (not app.rs) because it
/// persists; app.rs re-exports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaneMode {
    #[default]
    #[serde(rename = "split")]
    Split,
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "reading")]
    Reading,
}

fn default_true() -> bool {
    true
}

fn default_editor_font_size() -> u16 {
    14
}

fn default_autosave_secs() -> u64 {
    2
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_vault: None,
            theme: ThemeChoice::default(),
            glass_enabled: true,
            hide_md_extension: true,
            scroll_sync: true,
            auto_pair: true,
            wiki_autocomplete: true,
            word_wrap: true,
            editor_font_mono: true,
            editor_font_size: default_editor_font_size(),
            sidebar_open: true,
            autosave_secs: default_autosave_secs(),
            default_pane: PaneMode::default(),
            attachment_extensions: Vec::new(),
            link_path_format: LinkPathFormat::default(),
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
            auto_pair: false,
            wiki_autocomplete: true,
            word_wrap: false,
            editor_font_mono: true,
            editor_font_size: 18,
            sidebar_open: false,
            autosave_secs: 5,
            default_pane: PaneMode::Reading,
            attachment_extensions: vec!["png".into(), "pdf".into()],
            link_path_format: LinkPathFormat::Relative,
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
        // Phase G fields all default sensibly.
        assert!(loaded.auto_pair);
        assert!(loaded.wiki_autocomplete);
        assert!(loaded.word_wrap);
        assert!(loaded.editor_font_mono);
        assert_eq!(loaded.editor_font_size, 14);
        // The sidebar starts open for files that predate the field.
        assert!(loaded.sidebar_open);
        assert_eq!(loaded.autosave_secs, 2);
        assert_eq!(loaded.default_pane, PaneMode::Split);

        // A glass_enabled=false must survive the roundtrip.
        let path2 = tmp.path().join("settings2.json");
        let settings = AppSettings {
            glass_enabled: false,
            ..AppSettings::default()
        };
        store_to(&path2, &settings).unwrap();
        assert!(!load_from(&path2).unwrap().glass_enabled);
    }

    #[test]
    fn removed_width_and_solid_keys_are_ignored_on_load() {
        // A settings file written while the sidebar was resizable carries
        // sidebar_width/sidebar_solid keys; serde must ignore them instead
        // of failing the whole load.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, r#"{"sidebar_width": 320.0, "sidebar_solid": true}"#).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }
}
