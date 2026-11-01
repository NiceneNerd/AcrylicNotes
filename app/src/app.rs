//! The COSMIC application model: state, messages, update, subscriptions.
//!
//! Working rules (M4): all vault logic goes through `vault-core`/`vault-md`;
//! every write is bracketed by the safe save path; the pieces that need
//! testing without a GUI (save planning, new-note names) are pure functions.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use cosmic::app::{Core, Task};
use cosmic::iced::widget::image::Handle as ImageHandle;
use cosmic::iced::widget::text_editor::Content;
use cosmic::iced::Color;

use vault_core::vault::VaultError;
use vault_core::{NoteSource, ObsidianSettings, Vault, VaultIndex};
use vault_md::{parse_note, Doc};

use crate::settings::{self, AppSettings};
use crate::tree::{self, TreeNode};

/// How the note area is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneMode {
    #[default]
    Split,
    Source,
    Reading,
}

/// Colors derived once per view pass from the active cosmic theme, so the
/// reading renderer and views stay theme-agnostic (M4 decision 6: the
/// foundation for later custom themes).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    /// Subtle surface for code blocks / callout backgrounds.
    pub surface: Color,
    pub is_dark: bool,
}

impl Palette {
    pub fn from(theme: &cosmic::Theme) -> Self {
        let cosmic = theme.cosmic();
        let color = |c: palette::Srgba| Color {
            r: c.red,
            g: c.green,
            b: c.blue,
            a: c.alpha,
        };
        let text = color(cosmic.background(theme.transparent).on);
        let muted = Color {
            a: text.a * 0.6,
            ..text
        };
        Self {
            text,
            muted,
            accent: color(cosmic.accent.base),
            success: color(cosmic.success.base),
            warning: color(cosmic.warning.base),
            danger: color(cosmic.destructive.base),
            surface: color(cosmic.background(theme.transparent).base),
            is_dark: cosmic.is_dark,
        }
    }
}

/// One open note: editor buffer plus its derived reading document.
pub struct Tab {
    pub path: String,
    pub content: Content<cosmic::Renderer>,
    /// The text as loaded from / saved to disk; the dirty test compares
    /// against it.
    base_text: String,
    /// The parsed document behind the reading view, refreshed only when the
    /// editor text actually changes (one parse per edit, not per frame).
    pub doc: Doc,
    doc_text: String,
    /// Callouts the user folded by hand (ids from the reading renderer).
    pub folds: HashSet<u64>,
}

impl Tab {
    fn open(path: String, source: &str) -> Self {
        let mut tab = Self {
            content: Content::with_text(source),
            base_text: source.to_string(),
            path,
            doc: Doc {
                frontmatter_raw: None,
                blocks: Vec::new(),
            },
            doc_text: String::new(),
            folds: HashSet::new(),
        };
        tab.refresh_doc();
        tab
    }

    pub fn is_dirty(&self) -> bool {
        self.content.text() != self.base_text
    }

    /// Re-parse the reading document if the editor text changed.
    fn refresh_doc(&mut self) {
        let text = self.content.text();
        if text != self.doc_text {
            self.doc = parse_note(&text);
            self.doc_text = text;
            // Ids are positional; after an edit they may point at different
            // callouts, so fold state resets with the content.
            self.folds.clear();
        }
    }

    /// The label shown in the tab bar and tree: file stem.
    pub fn title(&self) -> String {
        title_of(&self.path)
    }
}

pub(crate) fn title_of(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string()
}

#[derive(Clone, Debug)]
pub enum Message {
    VaultInput(String),
    VaultOpenSubmitted,
    VaultOpen(PathBuf),
    /// Periodic watcher poll.
    WatchTick,
    BannerReload,
    /// Conflict resolution: deliberately overwrite the external changes
    /// with the local buffer.
    BannerOverwrite,
    BannerDismiss,
    ErrorDismiss,
    TabOpen(String),
    TabClose(usize),
    TabSelect(usize),
    EditorAction(cosmic::iced::widget::text_editor::Action),
    Save,
    PaneMode(PaneMode),
    ToggleFolder(String),
    ToggleFold(u64),
    /// A link span was clicked; the URI scheme routes it.
    OpenUri(String),
    /// An http(s) link: hand it to the OS opener.
    OpenExternal(String),
}

pub struct VaultNotes {
    core: Core,
    pub vault: Option<Vault>,
    /// Snapshot of the derived index; rebuilt after every open/save/reload.
    pub index: VaultIndex,
    pub vault_name: String,
    pub tree: Vec<TreeNode>,
    pub collapsed: HashSet<String>,
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    pub pane: PaneMode,
    /// Paths of open tabs that changed on disk while dirty.
    pub banner: Option<Banner>,
    pub error: Option<String>,
    pub vault_input: String,
    pub strict_breaks: bool,
    pub images: RefCell<std::collections::HashMap<String, ImageHandle>>,
}

impl VaultNotes {
    /// Open (or switch to) a vault; persists it as the last-used one.
    fn open_vault(&mut self, path: PathBuf) -> Task<Message> {
        match Vault::open(&path) {
            Ok(vault) => {
                self.strict_breaks = ObsidianSettings::load(vault.root()).strict_line_breaks;
                self.vault_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                self.tabs.clear();
                self.active = None;
                self.banner = None;
                self.error = None;
                self.collapsed.clear();
                self.vault = Some(vault);
                self.rebuild_index();
                self.core
                    .set_header_title(format!("Vault Notes — {}", self.vault_name));
                settings::store(&AppSettings {
                    last_vault: Some(path),
                });
            }
            Err(error) => {
                self.error = Some(format!("cannot open {}: {error}", path.display()));
            }
        }
        Task::none()
    }

    /// Path of the tab currently shown, if any.
    pub fn active_tab_path(&self) -> Option<&str> {
        self.active
            .and_then(|idx| self.tabs.get(idx))
            .map(|tab| tab.path.as_str())
    }

    /// Whether any open tab for `path` has unsaved edits.
    pub fn is_tab_dirty(&self, path: &str) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.path == path && tab.is_dirty())
    }

    fn rebuild_index(&mut self) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        self.index = vault.index();
        let paths: Vec<String> = vault
            .notes_sorted()
            .iter()
            .map(|note| note.path.clone())
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        self.tree = tree::build(&refs);
    }

    fn open_tab(&mut self, path: String) -> Task<Message> {
        if let Some(i) = self.tabs.iter().position(|tab| tab.path == path) {
            self.active = Some(i);
            return Task::none();
        }
        let source = self
            .vault
            .as_ref()
            .and_then(|vault| vault.note_source(&path))
            .unwrap_or_default();
        self.tabs.push(Tab::open(path, &source));
        self.active = Some(self.tabs.len() - 1);
        Task::none()
    }

    /// Create a note for an unresolved link and open it.
    fn create_note(&mut self, name: &str, from: &str) -> Task<Message> {
        let Some(clean) = sanitize_new_name(name) else {
            self.error = Some(format!("cannot create a note named {name:?}"));
            return Task::none();
        };
        let path = format!("{clean}.md");
        // If the name resolves after all (a note we did not consider), just
        // open it instead of duplicating.
        if let Some(existing) = self.index.resolve(&clean, from) {
            let existing = existing.to_string();
            return self.open_tab(existing);
        }
        let Some(vault) = self.vault.as_mut() else {
            return Task::none();
        };
        match vault.save_note(&path, "", "") {
            Ok(_) => {
                self.rebuild_index();
                self.open_tab(path)
            }
            Err(error) => {
                self.error = Some(format!("cannot create {path}: {error}"));
                Task::none()
            }
        }
    }

    /// Save the active tab through the safe write path: body-only edits
    /// reuse the original frontmatter bytes verbatim; frontmatter edits go
    /// through `save_note_full` with the full text.
    fn save_active(&mut self) -> Task<Message> {
        let (Some(idx), Some(vault)) = (self.active, self.vault.as_mut()) else {
            return Task::none();
        };
        let tab = &mut self.tabs[idx];
        // The sentinel is the base text our edits were made against: an
        // external change since then refuses the save instead of clobbering.
        let base = tab.base_text.clone();
        match write_tab(vault, tab, &base) {
            Ok(_) => {
                tab.base_text = tab.content.text();
                tab.refresh_doc();
                self.error = None;
                self.rebuild_index();
            }
            Err(VaultError::Write(vault_core::WriteError::ExternallyModified)) => {
                // The file changed under us: offer resolution in the banner.
                let path = tab.path.clone();
                self.banner = Some(Banner {
                    reloaded: Vec::new(),
                    conflicts: vec![path],
                });
            }
            Err(error) => {
                self.error = Some(format!("save failed: {error}"));
            }
        }
        Task::none()
    }

    /// Poll the watcher: re-index externally changed paths, auto-refresh
    /// clean tabs, and flag dirty ones in the banner (M4 decision 7).
    fn poll_watcher(&mut self) -> Task<Message> {
        let dirty = {
            let Some(vault) = self.vault.as_mut() else {
                return Task::none();
            };
            let dirty = vault.take_dirty();
            if dirty.is_empty() {
                return Task::none();
            }
            let _ = vault.reload(&dirty);
            dirty
        };
        self.rebuild_index();

        let mut banner = Banner::default();
        if let Some(vault) = self.vault.as_ref() {
            for tab in &mut self.tabs {
                if !dirty.iter().any(|path| path == &tab.path) {
                    continue;
                }
                let disk = vault.note_source(&tab.path).unwrap_or_default();
                match external_action(&disk, &tab.base_text, tab.is_dirty()) {
                    ExternalAction::None => {}
                    ExternalAction::Reloaded => {
                        tab.content = Content::with_text(&disk);
                        tab.base_text = disk;
                        tab.refresh_doc();
                        banner.reloaded.push(tab.path.clone());
                    }
                    ExternalAction::Conflict => {
                        banner.conflicts.push(tab.path.clone());
                    }
                }
            }
        }
        if !banner.is_empty() {
            self.banner = Some(banner);
        }
        Task::none()
    }
}

impl cosmic::Application for VaultNotes {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "dev.vault-handoff.Notes";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let mut app = Self {
            core,
            vault: None,
            index: VaultIndex::new(Vec::<String>::new()),
            vault_name: String::new(),
            tree: Vec::new(),
            collapsed: HashSet::new(),
            tabs: Vec::new(),
            active: None,
            pane: PaneMode::default(),
            banner: None,
            error: None,
            vault_input: String::new(),
            strict_breaks: false,
            images: RefCell::new(std::collections::HashMap::new()),
        };
        app.core.set_header_title("Vault Notes".to_string());

        // Restore the last-used vault (M4 decision 1) when it still exists.
        let restored = settings::load().last_vault.filter(|path| path.is_dir());
        let task = restored
            .map(|path| app.open_vault(path))
            .unwrap_or_else(Task::none);

        (app, task)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::VaultInput(input) => {
                self.vault_input = input;
                Task::none()
            }
            Message::VaultOpenSubmitted => {
                let raw = self.vault_input.trim().to_string();
                let path = expand_tilde(&raw).unwrap_or(PathBuf::from(raw));
                self.update(Message::VaultOpen(path))
            }
            Message::VaultOpen(path) => self.open_vault(path),
            Message::WatchTick => self.poll_watcher(),
            Message::BannerReload => {
                // Discard local edits in the conflicting tabs; reload disk.
                let conflicts = self
                    .banner
                    .take()
                    .map(|banner| banner.conflicts)
                    .unwrap_or_default();
                if let Some(vault) = self.vault.as_ref() {
                    for tab in &mut self.tabs {
                        if !conflicts.iter().any(|p| p == &tab.path) {
                            continue;
                        }
                        let source = vault.note_source(&tab.path).unwrap_or_default();
                        tab.content = Content::with_text(&source);
                        tab.base_text = source;
                        tab.refresh_doc();
                    }
                }
                self.error = None;
                Task::none()
            }
            Message::BannerOverwrite => {
                // Deliberate resolution: write the local buffer over the
                // external change (fresh disk text as the sentinel).
                let conflicts = self
                    .banner
                    .take()
                    .map(|banner| banner.conflicts)
                    .unwrap_or_default();
                if let Some(vault) = self.vault.as_mut() {
                    for tab in &mut self.tabs {
                        if !conflicts.iter().any(|p| p == &tab.path) {
                            continue;
                        }
                        let disk = vault.note_source(&tab.path).unwrap_or_default();
                        match write_tab(vault, tab, &disk) {
                            Ok(_) => tab.base_text = tab.content.text(),
                            Err(error) => {
                                self.error = Some(format!("overwrite failed: {error}"));
                            }
                        }
                    }
                }
                self.rebuild_index();
                Task::none()
            }
            Message::BannerDismiss => {
                self.banner = None;
                Task::none()
            }
            Message::ErrorDismiss => {
                self.error = None;
                Task::none()
            }
            Message::TabOpen(path) => self.open_tab(path),
            Message::TabClose(idx) => {
                if idx < self.tabs.len() {
                    self.tabs.remove(idx);
                    self.active = match self.active {
                        Some(active) if active >= self.tabs.len() => {
                            if self.tabs.is_empty() {
                                None
                            } else {
                                Some(self.tabs.len() - 1)
                            }
                        }
                        other => other,
                    };
                }
                Task::none()
            }
            Message::TabSelect(idx) => {
                if idx < self.tabs.len() {
                    self.active = Some(idx);
                }
                Task::none()
            }
            Message::EditorAction(action) => {
                if let Some(idx) = self.active {
                    let is_edit = action.is_edit();
                    let tab = &mut self.tabs[idx];
                    tab.content.perform(action);
                    if is_edit {
                        tab.refresh_doc();
                    }
                }
                Task::none()
            }
            Message::Save => self.save_active(),
            Message::PaneMode(pane) => {
                self.pane = pane;
                Task::none()
            }
            Message::ToggleFolder(path) => {
                if !self.collapsed.remove(&path) {
                    self.collapsed.insert(path);
                }
                Task::none()
            }
            Message::ToggleFold(id) => {
                if let Some(idx) = self.active {
                    let folds = &mut self.tabs[idx].folds;
                    if !folds.remove(&id) {
                        folds.insert(id);
                    }
                }
                Task::none()
            }
            Message::OpenUri(uri) => {
                let (scheme, rest) = match uri.split_once(':') {
                    Some(split) => split,
                    None => return self.update(Message::OpenExternal(uri)),
                };
                match scheme {
                    "note" => self.open_tab(rest.to_string()),
                    "wiki" | "create" => {
                        let (note, from) = rest.split_once('|').unwrap_or((rest, ""));
                        match scheme {
                            "wiki" => {
                                if let Some(path) = self.index.resolve(note, from) {
                                    let path = path.to_string();
                                    self.open_tab(path)
                                } else {
                                    self.create_note(note, from)
                                }
                            }
                            _ => self.create_note(note, from),
                        }
                    }
                    "open" => self.update(Message::OpenExternal(rest.to_string())),
                    // `tag:` routes to the tag pane in M4b; `fnref:` and
                    // `scroll:` need scroll infrastructure (M4b). Inert now.
                    _ => Task::none(),
                }
            }
            Message::OpenExternal(url) => {
                #[cfg(target_os = "linux")]
                {
                    use std::process::{Command, Stdio};
                    let _ = Command::new("xdg-open")
                        .arg(&url)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                }
                #[cfg(not(target_os = "linux"))]
                let _ = url;
                Task::none()
            }
        }
    }

    fn view(&self) -> cosmic::Element<'_, Self::Message> {
        crate::view::view(self)
    }

    fn header_start(&self) -> Vec<cosmic::Element<'_, Self::Message>> {
        crate::view::header_start(self)
    }

    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        if self.vault.is_some() {
            cosmic::iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::WatchTick)
        } else {
            cosmic::iced::Subscription::none()
        }
    }
}

/// What the watcher poll decides to do about one externally changed tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalAction {
    /// Content-identical change (e.g. an mtime-only `touch`): nothing to do.
    None,
    /// Real content change with no local edits: auto-reload + info banner.
    Reloaded,
    /// Real content change with unsaved edits: conflict banner.
    Conflict,
}

/// Classify an external change against the tab's base text (M4 decision 7):
/// identical content is silent; otherwise dirty tabs conflict and clean
/// tabs reload.
pub(crate) fn external_action(disk: &str, base: &str, tab_dirty: bool) -> ExternalAction {
    if disk == base {
        ExternalAction::None
    } else if tab_dirty {
        ExternalAction::Conflict
    } else {
        ExternalAction::Reloaded
    }
}

/// The non-blocking external-change banner: what was reloaded under us, and
/// which tabs have unsaved edits conflicting with disk.
#[derive(Debug, Default)]
pub struct Banner {
    pub reloaded: Vec<String>,
    pub conflicts: Vec<String>,
}

impl Banner {
    fn is_empty(&self) -> bool {
        self.reloaded.is_empty() && self.conflicts.is_empty()
    }
}

/// How the editor buffer should be written: body-only (frontmatter bytes are
/// provably untouched) or full text (the user edited the frontmatter).
pub(crate) enum SavePlan {
    Body(String),
    Full(String),
}

/// Write a tab's buffer through the safe save path.
///
/// `original` is the on-disk text this write is based on. Saves normally
/// pass the tab's `base_text`, so an external change since load is REFUSED
/// (`ExternallyModified`) instead of silently clobbered; the conflict
/// banner's "save mine anyway" passes the fresh disk text to overwrite
/// deliberately.
pub(crate) fn write_tab(
    vault: &mut Vault,
    tab: &mut Tab,
    original: &str,
) -> Result<vault_core::WriteResult, VaultError> {
    let text = tab.content.text();
    match save_plan(&text, original) {
        SavePlan::Body(body) => vault.save_note(&tab.path, original, &body),
        SavePlan::Full(full) => vault.save_note_full(&tab.path, original, &full),
    }
}

pub(crate) fn save_plan(editor: &str, disk: &str) -> SavePlan {
    let editor_split = vault_core::split_frontmatter(editor);
    let disk_split = vault_core::split_frontmatter(disk);
    if editor_split.bom == disk_split.bom && editor_split.raw == disk_split.raw {
        SavePlan::Body(editor_split.body)
    } else {
        SavePlan::Full(editor.to_string())
    }
}

/// Sanitize a new note name from an unresolved link target: no traversal, no
/// dot-directories. Returns the cleaned name (without extension).
pub(crate) fn sanitize_new_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    for component in name.split('/') {
        if component.is_empty()
            || component == ".."
            || component.starts_with('.')
            || component.contains('\\')
        {
            return None;
        }
    }
    Some(name.to_string())
}

fn expand_tilde(raw: &str) -> Option<PathBuf> {
    if raw == "~" {
        dirs::home_dir()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        Some(dirs::home_dir()?.join(rest))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_edits_keep_the_body_path() {
        let disk = "---\ntitle: T\n---\n\nold body\n";
        let editor = "---\ntitle: T\n---\n\nnew body\n";
        match save_plan(editor, disk) {
            SavePlan::Body(body) => assert_eq!(body, "\nnew body\n"),
            SavePlan::Full(_) => panic!("frontmatter untouched; body path expected"),
        }
    }

    #[test]
    fn frontmatter_edits_take_the_full_text_path() {
        let disk = "---\ntitle: T\n---\n\nbody\n";
        let editor = "---\ntitle: Edited\ntags: [x]\n---\n\nbody\n";
        match save_plan(editor, disk) {
            SavePlan::Body(_) => panic!("frontmatter edited; full-text path expected"),
            SavePlan::Full(full) => assert_eq!(full, editor),
        }
    }

    #[test]
    fn adding_or_removing_frontmatter_takes_the_full_text_path() {
        assert!(matches!(
            save_plan("---\na: 1\n---\n\nb\n", "b\n"),
            SavePlan::Full(_)
        ));
        assert!(matches!(
            save_plan("b\n", "---\na: 1\n---\n\nb\n"),
            SavePlan::Full(_)
        ));
    }

    #[test]
    fn bom_differences_take_the_full_text_path() {
        let disk = "---\na: 1\n---\n\nb\n";
        let editor = "\u{feff}---\na: 1\n---\n\nb\n";
        assert!(matches!(save_plan(editor, disk), SavePlan::Full(_)));
    }

    #[test]
    fn new_note_names_reject_traversal_and_dotfiles() {
        assert_eq!(sanitize_new_name("Glass"), Some("Glass".to_string()));
        assert_eq!(
            sanitize_new_name("Notes/Glass"),
            Some("Notes/Glass".to_string())
        );
        assert_eq!(sanitize_new_name("../evil"), None);
        assert_eq!(sanitize_new_name(".obsidian/x"), None);
        assert_eq!(sanitize_new_name(""), None);
        assert_eq!(sanitize_new_name("  "), None);
    }

    #[test]
    fn tab_titles_are_file_stems() {
        assert_eq!(title_of("Journal/2026/04/14.md"), "14");
        assert_eq!(title_of("Glass.md"), "Glass");
    }

    #[test]
    fn external_changes_classify_by_content_then_dirtiness() {
        use ExternalAction::*;
        // Content-identical (an mtime-only `touch`): silent either way.
        assert_eq!(external_action("same", "same", false), None);
        assert_eq!(external_action("same", "same", true), None);
        // Real change: clean tabs reload, dirty tabs conflict.
        assert_eq!(external_action("new", "old", false), Reloaded);
        assert_eq!(external_action("new", "old", true), Conflict);
    }

    #[test]
    fn tab_save_refuses_to_clobber_external_changes() {
        use cosmic::iced::widget::text_editor::{Action, Edit};

        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("n.md"), "hello\n").unwrap();
        let mut vault = Vault::open(tmp.path()).unwrap();
        let mut tab = Tab::open("n.md".to_string(), "hello\n");

        // Local edit + save against our base: goes through the safe path.
        tab.content.perform(Action::Edit(Edit::Insert('X')));
        let base = tab.base_text.clone();
        write_tab(&mut vault, &mut tab, &base).unwrap();
        assert_eq!(
            vault.note_source("n.md").as_deref(),
            Some(tab.content.text().as_str())
        );

        // External change after our base: refused, disk keeps its content.
        std::fs::write(tmp.path().join("n.md"), "external\n").unwrap();
        let base = tab.base_text.clone();
        let err = write_tab(&mut vault, &mut tab, &base);
        assert!(matches!(
            err,
            Err(VaultError::Write(
                vault_core::WriteError::ExternallyModified
            ))
        ));
        assert_eq!(vault.note_source("n.md").as_deref(), Some("external\n"));

        // The banner's "save mine anyway": fresh disk text as the sentinel.
        let fresh = vault.note_source("n.md").unwrap();
        write_tab(&mut vault, &mut tab, &fresh).unwrap();
        assert_eq!(
            vault.note_source("n.md").as_deref(),
            Some(tab.content.text().as_str())
        );
    }
}
