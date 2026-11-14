//! Shared settings DTOs and choice enums.
//!
//! These are the persistence-neutral data types both frontends bind to:
//! choice enums (theme, pane, margins, tree order, link spelling), font
//! bounds, and the `AppSettings` document. Storage (config paths, atomic
//! writes) stays in the frontends/platform layer; this module only defines
//! the data and its serde names, which are stable across platforms.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which visual theme the app uses. `System` follows the desktop.
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

/// The `AppSettings` document persisted by each frontend.
///
/// Field order and serde names are part of the on-disk format: unknown
/// fields are ignored on load, missing fields take their defaults, and
/// older files keep loading (see the round-trip tests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Vault opened last; restored on startup when it still exists.
    #[serde(default)]
    pub last_vault: Option<PathBuf>,
    /// UI theme choice; `system` follows the desktop.
    #[serde(default)]
    pub theme: ThemeChoice,
    /// Whether to request a frosted (blur) backdrop from the platform.
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
    /// Insert closing brackets/quotes and type over them.
    #[serde(default = "default_true")]
    pub auto_pair: bool,
    /// Suggest note titles while typing `[[`.
    #[serde(default = "default_true")]
    pub wiki_autocomplete: bool,
    /// Soft-wrap long lines in the editor.
    #[serde(default = "default_true")]
    pub word_wrap: bool,
    /// Edit in the monospace font (off = the system UI font). Ignored
    /// when a custom editor font family is set.
    #[serde(default = "default_true")]
    pub editor_font_mono: bool,
    /// Custom editor font family name; empty follows the monospace
    /// toggle. An unknown name falls back to the default font.
    #[serde(default)]
    pub editor_font_family: String,
    /// Editor font size in pixels.
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: u16,
    /// Custom reading-view font family name; empty follows the system
    /// font. An unknown name falls back to the default font.
    #[serde(default)]
    pub reading_font_family: String,
    /// Reading-view font size in pixels. The source editor keeps its own
    /// size, so the two panes can be sized independently.
    #[serde(default = "default_reading_font_size")]
    pub reading_font_size: u16,
    /// The margin the reading view leaves around its text column.
    #[serde(default)]
    pub reading_margin: ReadingMargin,
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
    /// How the file tree orders its notes. Folders always sort by name.
    #[serde(default)]
    pub tree_sort: TreeSort,
}

/// How the file tree orders its notes.
///
/// Folders always come first, sorted by name; only the notes follow the
/// chosen order. The date orders need a modification time, which the
/// vault keeps for notes but not for attachments — an attachment sorts
/// last in every order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeSort {
    /// Notes by name, A to Z.
    #[default]
    #[serde(rename = "name-asc")]
    NameAsc,
    /// Notes by name, Z to A.
    #[serde(rename = "name-desc")]
    NameDesc,
    /// The most recently changed notes first.
    #[serde(rename = "newest")]
    Newest,
    /// The least recently changed notes first.
    #[serde(rename = "oldest")]
    Oldest,
}

impl TreeSort {
    /// Every order, in the order the menu lists them.
    pub const ALL: [TreeSort; 4] = [
        TreeSort::NameAsc,
        TreeSort::NameDesc,
        TreeSort::Newest,
        TreeSort::Oldest,
    ];

    /// The user-facing name of this order.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name A–Z",
            Self::NameDesc => "Name Z–A",
            Self::Newest => "Newest first",
            Self::Oldest => "Oldest first",
        }
    }

    /// The next order in the cycle (what the sort button's click does).
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::NameAsc => Self::NameDesc,
            Self::NameDesc => Self::Newest,
            Self::Newest => Self::Oldest,
            Self::Oldest => Self::NameAsc,
        }
    }

    /// Whether this order runs from the top down (the button's icon).
    #[must_use]
    pub fn descending(self) -> bool {
        matches!(self, Self::NameDesc | Self::Newest)
    }
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

/// How the note area is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaneMode {
    /// Live preview: blocks render like the reading pane; the block
    /// being edited is a raw-source text editor.
    #[default]
    #[serde(rename = "live")]
    Live,
    #[serde(rename = "split")]
    Split,
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "reading")]
    Reading,
}

/// How much margin the reading view leaves around its text column.
///
/// The margin is the space between the pane's edge and the text: a
/// narrow column reads badly against the window edge, a wide one wastes
/// the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingMargin {
    /// No margin: the text runs to the pane's edges.
    #[serde(rename = "none")]
    None,
    /// A tight margin (16 px).
    #[serde(rename = "compact")]
    Compact,
    /// A roomy margin (32 px) — the default.
    #[default]
    #[serde(rename = "comfortable")]
    Comfortable,
    /// A wide margin (56 px).
    #[serde(rename = "spacious")]
    Spacious,
}

impl ReadingMargin {
    /// Every margin, in the order the settings drawer lists them.
    pub const ALL: [ReadingMargin; 4] = [
        ReadingMargin::None,
        ReadingMargin::Compact,
        ReadingMargin::Comfortable,
        ReadingMargin::Spacious,
    ];

    /// The user-facing name of this margin.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Spacious => "Spacious",
        }
    }

    /// The padding (in pixels) the reading view leaves around its text.
    #[must_use]
    pub fn padding(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Compact => 16.0,
            Self::Comfortable => 32.0,
            Self::Spacious => 56.0,
        }
    }
}

/// The smallest font size either pane accepts.
pub const MIN_FONT_SIZE: u16 = 8;

/// The largest font size either pane accepts.
pub const MAX_FONT_SIZE: u16 = 72;

/// Clamp a font size to the range both panes accept.
#[must_use]
pub fn clamp_font_size(size: i32) -> u16 {
    size.clamp(i32::from(MIN_FONT_SIZE), i32::from(MAX_FONT_SIZE)) as u16
}

fn default_true() -> bool {
    true
}

fn default_editor_font_size() -> u16 {
    14
}

fn default_reading_font_size() -> u16 {
    16
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
            editor_font_family: String::new(),
            editor_font_size: default_editor_font_size(),
            reading_font_family: String::new(),
            reading_font_size: default_reading_font_size(),
            reading_margin: ReadingMargin::default(),
            sidebar_open: true,
            autosave_secs: default_autosave_secs(),
            default_pane: PaneMode::default(),
            attachment_extensions: Vec::new(),
            link_path_format: LinkPathFormat::default(),
            tree_sort: TreeSort::default(),
        }
    }
}

/// The platform service owning the settings document's storage.
///
/// The kernel decides *what* to persist (its snapshot, e.g.
/// `AppState::default_pane`); the store decides *where and how* (a JSON
/// file in the platform config dir, an in-memory fake in tests).
/// Persistence is load-modify-store at the call site — the runner loads,
/// applies the kernel's snapshot, and stores back — so fields this build
/// does not own survive every write.
pub trait SettingsStore {
    /// The stored document, or defaults when nothing valid is stored.
    fn load(&self) -> AppSettings;

    /// Persist the whole document. The real store writes atomically;
    /// failures are non-fatal (settings are a convenience), the store
    /// keeps serving its last good document on load.
    fn store(&mut self, settings: &AppSettings);
}

/// A deterministic in-memory [`SettingsStore`] for tests: it starts from
/// a known document and records every write so tests can assert exactly
/// what the runner persisted.
#[derive(Debug, Clone, Default)]
pub struct MemorySettingsStore {
    current: AppSettings,
    writes: usize,
}

impl MemorySettingsStore {
    /// A store already holding `settings` (e.g. a previous session's).
    #[must_use]
    pub fn starting_with(settings: AppSettings) -> Self {
        Self {
            current: settings,
            writes: 0,
        }
    }

    /// How many documents were stored.
    #[must_use]
    pub fn writes(&self) -> usize {
        self.writes
    }

    /// The document a load would return right now.
    #[must_use]
    pub fn last(&self) -> &AppSettings {
        &self.current
    }
}

impl SettingsStore for MemorySettingsStore {
    fn load(&self) -> AppSettings {
        self.current.clone()
    }

    fn store(&mut self, settings: &AppSettings) {
        self.current = settings.clone();
        self.writes += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_mode_serializes_as_lowercase_with_live_default() {
        // The persisted names are the user-facing spokes of the default-pane
        // choice; "live" joins them and old files' "split" keeps loading.
        assert_eq!(serde_json::to_string(&PaneMode::Live).unwrap(), r#""live""#);
        assert_eq!(
            serde_json::to_string(&PaneMode::Split).unwrap(),
            r#""split""#
        );
        assert_eq!(
            serde_json::from_str::<PaneMode>(r#""split""#).unwrap(),
            PaneMode::Split
        );
        assert_eq!(
            serde_json::from_str::<PaneMode>(r#""reading""#).unwrap(),
            PaneMode::Reading
        );
        assert_eq!(PaneMode::default(), PaneMode::Live);
    }

    #[test]
    fn the_tree_sort_persists_as_a_word_and_defaults_to_the_name() {
        assert_eq!(
            serde_json::to_string(&TreeSort::NameAsc).unwrap(),
            r#""name-asc""#
        );
        assert_eq!(
            serde_json::from_str::<TreeSort>(r#""name-desc""#).unwrap(),
            TreeSort::NameDesc
        );
        assert_eq!(
            serde_json::from_str::<TreeSort>(r#""newest""#).unwrap(),
            TreeSort::Newest
        );
        assert_eq!(
            serde_json::from_str::<TreeSort>(r#""oldest""#).unwrap(),
            TreeSort::Oldest
        );
        assert_eq!(TreeSort::default(), TreeSort::NameAsc);
    }

    #[test]
    fn the_tree_sort_cycles_and_names_its_orders() {
        // The button cycles through the four orders and comes back.
        let mut order = TreeSort::NameAsc;
        for _ in 0..4 {
            order = order.next();
        }
        assert_eq!(order, TreeSort::NameAsc);
        assert_eq!(TreeSort::NameAsc.next(), TreeSort::NameDesc);
        assert_eq!(TreeSort::NameDesc.next(), TreeSort::Newest);
        assert_eq!(TreeSort::Newest.next(), TreeSort::Oldest);
        assert_eq!(TreeSort::Oldest.next(), TreeSort::NameAsc);

        // Every order has a label, and the descending ones are marked.
        for order in TreeSort::ALL {
            assert!(!order.label().is_empty());
        }
        assert!(!TreeSort::NameAsc.descending());
        assert!(TreeSort::NameDesc.descending());
        assert!(TreeSort::Newest.descending());
        assert!(!TreeSort::Oldest.descending());
    }

    #[test]
    fn the_reading_margin_persists_as_a_word_and_widens_the_gutter() {
        assert_eq!(
            serde_json::to_string(&ReadingMargin::None).unwrap(),
            r#""none""#
        );
        assert_eq!(
            serde_json::from_str::<ReadingMargin>(r#""spacious""#).unwrap(),
            ReadingMargin::Spacious
        );
        assert_eq!(ReadingMargin::default(), ReadingMargin::Comfortable);

        // Each margin has a label and a padding, and the paddings grow
        // with the room they ask for.
        let mut previous = 0.0;
        for margin in ReadingMargin::ALL {
            assert!(!margin.label().is_empty());
            assert!(margin.padding() >= previous);
            previous = margin.padding();
        }
        assert_eq!(ReadingMargin::None.padding(), 0.0);
    }

    #[test]
    fn font_sizes_are_clamped_to_the_range_both_panes_accept() {
        assert_eq!(clamp_font_size(14), 14);
        assert_eq!(clamp_font_size(i32::from(MIN_FONT_SIZE) - 5), MIN_FONT_SIZE);
        assert_eq!(clamp_font_size(i32::from(MAX_FONT_SIZE) + 5), MAX_FONT_SIZE);
        // Every integer inside the range survives untouched.
        for size in MIN_FONT_SIZE..=MAX_FONT_SIZE {
            assert_eq!(clamp_font_size(i32::from(size)), size);
        }
    }

    #[test]
    fn settings_document_round_trips() {
        let settings = AppSettings {
            last_vault: Some(PathBuf::from("/home/nn/Notes")),
            theme: ThemeChoice::Dark,
            reading_margin: ReadingMargin::Spacious,
            default_pane: PaneMode::Reading,
            link_path_format: LinkPathFormat::Relative,
            tree_sort: TreeSort::Newest,
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let loaded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn removed_and_unknown_keys_are_ignored_on_load() {
        // A settings file written by another frontend (or an older one)
        // carries keys this version does not know; serde must ignore them
        // instead of failing the whole load.
        let text = r#"{"sidebar_width": 320.0, "sidebar_solid": true, "unknown": 1}"#;
        let loaded: AppSettings = serde_json::from_str(text).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn the_memory_settings_store_round_trips_and_counts_writes() {
        let mut store = MemorySettingsStore::starting_with(AppSettings {
            default_pane: PaneMode::Reading,
            ..AppSettings::default()
        });
        assert_eq!(store.load().default_pane, PaneMode::Reading);

        let mut document = store.load();
        document.default_pane = PaneMode::Split;
        store.store(&document);

        assert_eq!(store.writes(), 1);
        assert_eq!(store.last().default_pane, PaneMode::Split);
    }

    #[test]
    fn missing_fields_take_safe_defaults() {
        let loaded: AppSettings = serde_json::from_str(r#"{"last_vault": "/notes"}"#).unwrap();
        assert_eq!(loaded.last_vault, Some(PathBuf::from("/notes")));
        assert_eq!(loaded.theme, ThemeChoice::System);
        assert!(loaded.glass_enabled);
        assert!(loaded.hide_md_extension);
        assert!(loaded.scroll_sync);
        assert!(loaded.auto_pair);
        assert!(loaded.wiki_autocomplete);
        assert!(loaded.word_wrap);
        assert!(loaded.editor_font_mono);
        assert_eq!(loaded.editor_font_size, 14);
        assert!(loaded.editor_font_family.is_empty());
        assert!(loaded.reading_font_family.is_empty());
        assert_eq!(loaded.reading_font_size, 16);
        assert_eq!(loaded.reading_margin, ReadingMargin::Comfortable);
        assert!(loaded.sidebar_open);
        assert_eq!(loaded.autosave_secs, 2);
        assert_eq!(loaded.default_pane, PaneMode::Live);
    }
}
