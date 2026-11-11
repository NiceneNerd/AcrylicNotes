//! Persistent app settings (M4a: last-used vault; M4c: theme + glass),
//! stored as JSON in the XDG config directory. A plain file — not
//! cosmic-config — so the app also works outside a full COSMIC session.
//!
//! The settings *data* (the DTO and choice enums) lives in `vault-app`
//! so both frontends bind to the same document; this module owns this
//! platform's storage (XDG path, atomic tmp+rename writes).

use std::fs;
use std::path::{Path, PathBuf};

pub use vault_app::settings::{
    clamp_font_size, AppSettings, LinkPathFormat, PaneMode, ReadingMargin, ThemeChoice, TreeSort,
};

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
            editor_font_family: "Fira Code".to_string(),
            editor_font_size: 18,
            reading_font_family: "Iowan Old Style".to_string(),
            reading_font_size: 21,
            reading_margin: ReadingMargin::Spacious,
            sidebar_open: false,
            autosave_secs: 5,
            default_pane: PaneMode::Reading,
            attachment_extensions: vec!["png".into(), "pdf".into()],
            link_path_format: LinkPathFormat::Relative,
            tree_sort: TreeSort::Newest,
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
        assert!(loaded.editor_font_family.is_empty(), "no custom font");
        // The reading view has its own font family and size; both start
        // out empty/default for files that predate the fields.
        assert!(loaded.reading_font_family.is_empty());
        assert_eq!(loaded.reading_font_size, 16);
        assert_eq!(loaded.reading_margin, ReadingMargin::Comfortable);
        // The sidebar starts open for files that predate the field.
        assert!(loaded.sidebar_open);
        assert_eq!(loaded.autosave_secs, 2);
        // Live preview is the new default (PaneMode::Live has `#[default]`);
        // files older than the field open into it.
        assert_eq!(loaded.default_pane, PaneMode::Live);

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
