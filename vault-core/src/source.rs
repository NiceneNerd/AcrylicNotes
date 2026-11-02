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

/// The bits of Obsidian's daily-notes plugin config
/// (`.obsidian/daily-notes.json`) that decide where daily notes live and how
/// new ones are seeded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyNotesSettings {
    /// Vault-relative folder for daily notes (Obsidian key: `folder`).
    #[serde(rename = "folder", default)]
    pub folder: Option<String>,
    /// Moment-style date format for the file name (Obsidian key: `format`).
    #[serde(rename = "format", default)]
    pub format: Option<String>,
    /// Vault-relative path of the template note (Obsidian key: `template`).
    #[serde(rename = "template", default)]
    pub template: Option<String>,
}

/// The fallback used when `.obsidian/daily-notes.json` is missing entirely:
/// `Journal/YYYY/MM/DD.md`.
impl DailyNotesSettings {
    fn fallback() -> Self {
        Self {
            folder: Some("Journal".to_string()),
            format: Some("YYYY/MM/DD".to_string()),
            template: None,
        }
    }

    /// Load `.obsidian/daily-notes.json`; a missing or unparsable file yields
    /// the fallback configuration.
    pub fn load(vault_root: impl AsRef<Path>) -> Self {
        let path = vault_root
            .as_ref()
            .join(".obsidian")
            .join("daily-notes.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(Self::fallback)
    }

    /// The vault-relative folder daily notes live in.
    pub fn folder(&self) -> &str {
        self.folder.as_deref().unwrap_or("Journal")
    }

    /// The moment-style date format for daily note names. When a settings
    /// file exists but omits the format, this is Obsidian's plugin default
    /// (`YYYY-MM-DD`); with no settings file at all the whole fallback shape
    /// is `Journal/YYYY/MM/DD.md`.
    pub fn format(&self) -> &str {
        self.format.as_deref().unwrap_or("YYYY-MM-DD")
    }
}

/// Translate the moment.js format tokens Obsidian uses into `chrono`
/// specifiers. Longest tokens are matched first; `[...]` escapes pass their
/// contents through literally; anything unknown is copied verbatim.
pub fn moment_to_chrono(format: &str) -> String {
    const TOKENS: &[(&str, &str)] = &[
        ("YYYY", "%Y"),
        ("YY", "%y"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("MM", "%m"),
        ("M", "%-m"),
        ("DD", "%d"),
        ("D", "%-d"),
        ("dddd", "%A"),
        ("ddd", "%a"),
        ("HH", "%H"),
        ("H", "%-H"),
        ("hh", "%I"),
        ("h", "%-I"),
        ("mm", "%M"),
        ("m", "%-M"),
        ("ss", "%S"),
        ("s", "%-S"),
        ("X", "%s"),
    ];

    let mut out = String::with_capacity(format.len());
    let mut chars = format.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '[' {
            // Literal escape: copy until the closing bracket.
            for (_, inner) in chars.by_ref() {
                if inner == ']' {
                    break;
                }
                out.push(inner);
            }
            continue;
        }
        let rest = &format[i..];
        let matched = TOKENS.iter().find(|(token, _)| rest.starts_with(token));
        if let Some((token, spec)) = matched {
            out.push_str(spec);
            // Skip the rest of the token's characters (the first was consumed).
            for _ in 1..token.chars().count() {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Fill a daily-notes template the way Obsidian's daily-notes plugin does:
/// `{{date}}` and `{{time}}` (optionally `{{date:FORMAT}}` with a moment
/// format), and `{{title}}`.
pub fn render_daily_template(
    template: &str,
    date: chrono::NaiveDate,
    title: &str,
    now: chrono::NaiveTime,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let placeholder = &after[..end];
        rest = &after[end + 2..];
        if let Some(format) = placeholder.strip_prefix("date:") {
            out.push_str(&date.format(&moment_to_chrono(format)).to_string());
        } else if let Some(format) = placeholder.strip_prefix("time:") {
            out.push_str(&now.format(&moment_to_chrono(format)).to_string());
        } else {
            match placeholder.trim() {
                "date" => out.push_str(&date.format("%Y-%m-%d").to_string()),
                "time" => out.push_str(&now.format("%H:%M").to_string()),
                "title" => out.push_str(title),
                // Unknown placeholder: keep it verbatim.
                other => {
                    out.push_str("{{");
                    out.push_str(other);
                    out.push_str("}}");
                }
            }
        }
    }
    out.push_str(rest);
    out
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

    mod daily {
        use super::*;
        use chrono::NaiveDate;

        fn date(y: i32, m: u32, d: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, d).unwrap()
        }

        #[test]
        fn missing_daily_notes_file_falls_back_to_journal_yyyy_mm_dd() {
            let tmp = TempDir::new().unwrap();
            let settings = DailyNotesSettings::load(tmp.path());
            assert_eq!(settings.folder(), "Journal");
            assert_eq!(settings.format(), "YYYY/MM/DD");
            assert_eq!(settings.template, None);
        }

        #[test]
        fn daily_notes_file_parses_obsidian_keys() {
            let tmp = TempDir::new().unwrap();
            let obs = tmp.path().join(".obsidian");
            fs::create_dir(&obs).unwrap();
            fs::write(
                obs.join("daily-notes.json"),
                r#"{"folder": "Days", "format": "YYYY-MM-DD", "template": "Templates/Daily.md"}"#,
            )
            .unwrap();
            let settings = DailyNotesSettings::load(tmp.path());
            assert_eq!(settings.folder(), "Days");
            assert_eq!(settings.format(), "YYYY-MM-DD");
            assert_eq!(settings.template.as_deref(), Some("Templates/Daily.md"));
        }

        #[test]
        fn daily_notes_file_without_format_uses_obsidian_default() {
            let tmp = TempDir::new().unwrap();
            let obs = tmp.path().join(".obsidian");
            fs::create_dir(&obs).unwrap();
            fs::write(obs.join("daily-notes.json"), r#"{"folder": "Days"}"#).unwrap();
            let settings = DailyNotesSettings::load(tmp.path());
            assert_eq!(settings.folder(), "Days");
            assert_eq!(settings.format(), "YYYY-MM-DD");
        }

        #[test]
        fn moment_formats_translate_to_chrono() {
            assert_eq!(moment_to_chrono("YYYY-MM-DD"), "%Y-%m-%d");
            assert_eq!(moment_to_chrono("YYYY/MM/DD"), "%Y/%m/%d");
            assert_eq!(moment_to_chrono("YY MMM D"), "%y %b %-d");
            assert_eq!(moment_to_chrono("dddd HH:mm"), "%A %H:%M");
            assert_eq!(moment_to_chrono("MMMM"), "%B");
            // Longest token wins: YYYY before YY, MMMM before MMM.
            assert_eq!(moment_to_chrono("YYYYYY"), "%Y%y");
        }

        #[test]
        fn moment_bracket_escapes_pass_through() {
            assert_eq!(moment_to_chrono("[Week] WW"), "Week WW");
            assert_eq!(moment_to_chrono("YYYY[Q]"), "%YQ");
        }

        #[test]
        fn moment_unknown_tokens_survive() {
            assert_eq!(moment_to_chrono("wo"), "wo");
        }

        #[test]
        fn template_placeholders_fill() {
            let d = date(2026, 11, 2);
            let t = chrono::NaiveTime::from_hms_opt(9, 5, 0).unwrap();
            assert_eq!(
                render_daily_template("# {{date}}\n\n{{title}}", d, "2026-11-02", t),
                "# 2026-11-02\n\n2026-11-02"
            );
            assert_eq!(
                render_daily_template("{{date:YYYY/MM/DD}} at {{time}}", d, "x", t),
                "2026/11/02 at 09:05"
            );
            assert_eq!(render_daily_template("{{time:HH:mm}}", d, "x", t), "09:05");
        }

        #[test]
        fn unknown_and_unclosed_placeholders_survive() {
            let d = date(2026, 11, 2);
            let t = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap();
            assert_eq!(render_daily_template("{{tags}}", d, "x", t), "{{tags}}");
            assert_eq!(render_daily_template("{{date", d, "x", t), "{{date");
        }
    }
}
