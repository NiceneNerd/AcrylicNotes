//! The COSMIC application model: state, messages, update, subscriptions.
//!
//! Working rules (M4): all vault logic goes through `vault-core`/`vault-md`;
//! every write is bracketed by the safe save path; the pieces that need
//! testing without a GUI (save planning, new-note names) are pure functions.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use cosmic::app::{Core, Task};
use cosmic::iced::core::widget::Id;
use cosmic::iced::keyboard::{key::Named, Key, Modifiers};
use cosmic::iced::widget::image::Handle as ImageHandle;
use cosmic::iced::widget::text_editor::Content;
use cosmic::iced::Color;

use vault_core::vault::VaultError;
use vault_core::{NoteSource, ObsidianSettings, Vault, VaultIndex};
use vault_md::{parse_note, Doc};

use crate::settings;
use crate::tree::{self, TreeNode};

/// How the note area is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneMode {
    #[default]
    Split,
    Source,
    Reading,
}

/// Which panel the sidebar shows (M4b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidePanel {
    #[default]
    Files,
    Search,
    Tags,
    Links,
    Outline,
}

impl SidePanel {
    pub const ALL: [SidePanel; 5] = [
        SidePanel::Files,
        SidePanel::Search,
        SidePanel::Tags,
        SidePanel::Links,
        SidePanel::Outline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SidePanel::Files => "Files",
            SidePanel::Search => "Search",
            SidePanel::Tags => "Tags",
            SidePanel::Links => "Links",
            SidePanel::Outline => "Outline",
        }
    }
}

/// The quick switcher: a fuzzy overlay over all note paths (M4b, nucleo).
#[derive(Debug, Default)]
pub struct Switcher {
    pub open: bool,
    pub query: String,
    /// Ranked note paths (already materialized from the index).
    pub items: Vec<String>,
    pub selected: usize,
}

/// Id of the switcher's text input (so it can be focused on open).
pub const SWITCHER_INPUT: &str = "switcher-input";

/// Cap on vault-wide search results shown.
pub const SEARCH_MAX_HITS: usize = 200;

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
    /// Headings for the outline panel, in sync with `doc`.
    pub outline: Vec<crate::reading::OutlineEntry>,
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
            outline: Vec::new(),
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
            self.outline = crate::reading::outline(&self.doc);
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
    /// Open (creating if needed) today's daily note.
    DailyNote,
    /// Switch between dark and light themes (persisted).
    ToggleTheme,
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
    // --- M4b: navigation panels, quick switcher, search ---
    SidePanel(SidePanel),
    SearchInput(String),
    SearchSubmitted,
    SearchDone(Vec<crate::search::SearchHit>),
    /// Open search hit `i` and scroll its reading view to the match.
    SearchOpen(usize),
    TagSelected(String),
    /// Scroll the active tab's reading view to top-level block `idx`.
    JumpToBlock(usize),
    SwitcherToggle,
    SwitcherInput(String),
    SwitcherMove(i32),
    SwitcherActivate,
    /// Open switcher item `i` directly (mouse click).
    SwitcherSelect(usize),
    /// Raw key events; update decides what matters (switcher, shortcuts).
    KeyPressed(Modifiers, Key),
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
    // --- M4b ---
    pub panel: SidePanel,
    pub search_query: String,
    pub search_hits: Vec<crate::search::SearchHit>,
    pub selected_tag: Option<String>,
    pub switcher: Switcher,
    // --- M4c ---
    /// When the editor last saw an edit action; drives autosave-on-idle.
    pub last_edit: Option<std::time::Instant>,
    /// The backdrop style the platform offered at startup (glass).
    pub glass_style: vault_glass::GlassStyle,
    /// The theme choice the user made (persisted; `System` follows desktop).
    pub theme: settings::ThemeChoice,
}

/// Save the active note ~2 s after its last edit (M4 decision, user-tuned).
const AUTOSAVE_IDLE: std::time::Duration = std::time::Duration::from_secs(2);

/// Pure autosave decision: an idle period measured since the last edit is
/// due for a save. Unit-testable without an `Instant`.
fn autosave_due(idle_since_last_edit: Option<std::time::Duration>) -> bool {
    idle_since_last_edit.is_some_and(|d| d >= AUTOSAVE_IDLE)
}

/// The libcosmic task that applies a stored theme choice (M4c).
fn theme_task(choice: settings::ThemeChoice) -> Task<Message> {
    let theme = match choice {
        settings::ThemeChoice::System => cosmic::theme::system_preference(),
        settings::ThemeChoice::Dark => cosmic::theme::Theme::dark(),
        settings::ThemeChoice::Light => cosmic::theme::Theme::light(),
    };
    Task::done(cosmic::Action::Cosmic(cosmic::app::Action::AppThemeChange(
        theme,
    )))
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
                self.vault = Some(vault);
                self.rebuild_index();
                // Start with every folder collapsed (user preference).
                self.collapsed = tree::folder_paths(&self.tree).into_iter().collect();
                self.core
                    .set_header_title(format!("Vault Notes — {}", self.vault_name));
                // Load-modify-store so theme and glass choices survive.
                let mut persisted = settings::load();
                persisted.last_vault = Some(path);
                settings::store(&persisted);
            }
            Err(error) => {
                self.error = Some(format!("cannot open {}: {error}", path.display()));
            }
        }
        Task::none()
    }

    /// The tab currently shown, if any.
    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|idx| self.tabs.get(idx))
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

    /// Open today's daily note, creating it from the configured template.
    fn open_daily_note(&mut self) -> Task<Message> {
        let today = chrono::Local::now().date_naive();
        let Some(vault) = self.vault.as_mut() else {
            return Task::none();
        };
        match vault.open_or_create_daily_note(today) {
            Ok((path, _created)) => {
                self.rebuild_index();
                self.open_tab(path)
            }
            Err(error) => {
                self.error = Some(format!("cannot open daily note: {error}"));
                Task::none()
            }
        }
    }

    /// Re-rank the switcher's items from the vault's note paths.
    fn rerank_switcher(&mut self) {
        let query = self.switcher.query.clone();
        if let Some(vault) = self.vault.as_ref() {
            let paths: Vec<String> = vault
                .notes_sorted()
                .iter()
                .map(|note| note.path.clone())
                .collect();
            self.switcher.items = crate::fuzzy::rank(&query, &paths, 20)
                .into_iter()
                .map(|ranked| paths[ranked.index].clone())
                .collect();
        } else {
            self.switcher.items.clear();
        }
        if self.switcher.selected >= self.switcher.items.len() {
            self.switcher.selected = 0;
        }
    }

    /// Scroll the active tab's reading view to top-level block `idx`,
    /// switching out of source-only mode so the jump is visible.
    fn jump_to_block(&mut self, idx: usize) -> Task<Message> {
        if self.pane == PaneMode::Source {
            self.pane = PaneMode::Split;
        }
        // No frame deferral needed even when the pane just switched: the
        // iced runner rebuilds all user interfaces *before* running the
        // actions returned by update, so the measure op below always sees
        // the freshly built tree (iced/winit/src/lib.rs: build_user_interfaces
        // precedes run_action!).
        crate::scroll::to_block::<Message>(idx, 12.0)
    }

    /// Save the active tab through the safe write path: body-only edits
    /// reuse the original frontmatter bytes verbatim; frontmatter edits go
    /// through `save_note_full` with the full text.
    fn save_active(&mut self) -> Task<Message> {
        if let Some(idx) = self.active {
            self.save_tab(idx);
        }
        Task::none()
    }

    /// Save tab `idx` through the guarded write path. Returns whether the
    /// tab is now clean; on refusal (external change) the conflict banner is
    /// raised and the tab stays dirty.
    fn save_tab(&mut self, idx: usize) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            return false;
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
                true
            }
            Err(VaultError::Write(vault_core::WriteError::ExternallyModified)) => {
                // The file changed under us: offer resolution in the banner.
                let path = tab.path.clone();
                self.banner = Some(Banner {
                    reloaded: Vec::new(),
                    conflicts: vec![path],
                });
                false
            }
            Err(error) => {
                self.error = Some(format!("save failed: {error}"));
                false
            }
        }
    }

    /// Close a tab, saving unsaved edits first. A refused save (external
    /// change) keeps the tab open so no edits are lost.
    fn close_tab(&mut self, idx: usize) -> Task<Message> {
        if idx >= self.tabs.len() {
            return Task::none();
        }
        if self.tabs[idx].is_dirty() && !self.save_tab(idx) {
            return Task::none();
        }
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
        Task::none()
    }

    /// Poll the watcher: re-index externally changed paths, auto-refresh
    /// clean tabs, and flag dirty ones in the banner (M4 decision 7).
    /// Save the active tab if it was edited and has been idle for
    /// `AUTOSAVE_IDLE`. Goes through the guarded save path, so an external
    /// change refuses the write and raises the conflict banner instead of
    /// silently clobbering.
    fn autosave_tick(&mut self) -> Task<Message> {
        let idle = self.last_edit.map(|t| t.elapsed());
        if autosave_due(idle) && self.active.is_some_and(|idx| self.tabs[idx].is_dirty()) {
            return self.save_active();
        }
        Task::none()
    }

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
            panel: SidePanel::default(),
            search_query: String::new(),
            search_hits: Vec::new(),
            selected_tag: None,
            switcher: Switcher::default(),
            last_edit: None,
            glass_style: vault_glass::GlassStyle::Opaque,
            theme: settings::ThemeChoice::default(),
        };
        app.core.set_header_title("Vault Notes".to_string());

        // Restore preferences (M4c): theme choice, then the glass request,
        // then the last-used vault (M4 decision 1) when it still exists.
        let settings = settings::load();
        app.theme = settings.theme;
        app.glass_style = crate::glass::plan(settings.glass_enabled);

        let mut tasks = Vec::new();
        tasks.push(theme_task(settings.theme));
        if app.glass_style == vault_glass::GlassStyle::Blur {
            // Ask the compositor for the frosted window; visibility still
            // depends on the theme's frosted-windows setting.
            tasks.push(Task::done(cosmic::Action::Cosmic(
                cosmic::app::Action::BlurEnabled,
            )));
        }
        tasks.push(
            settings
                .last_vault
                .filter(|path| path.is_dir())
                .map(|path| app.open_vault(path))
                .unwrap_or_else(Task::none),
        );

        (app, Task::batch(tasks))
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
            Message::WatchTick => {
                let autosave = self.autosave_tick();
                let poll = self.poll_watcher();
                Task::batch([autosave, poll])
            }
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
            Message::DailyNote => self.open_daily_note(),
            Message::ToggleTheme => {
                // System starts the cycle at dark; dark and light alternate.
                let next = match self.theme {
                    settings::ThemeChoice::Dark | settings::ThemeChoice::System => {
                        settings::ThemeChoice::Light
                    }
                    settings::ThemeChoice::Light => settings::ThemeChoice::Dark,
                };
                self.theme = next;
                let mut persisted = settings::load();
                persisted.theme = next;
                settings::store(&persisted);
                theme_task(next)
            }
            Message::TabClose(idx) => self.close_tab(idx),
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
                    // Any editor activity (edits and cursor moves alike)
                    // resets the idle timer: the user is still working here.
                    self.last_edit = Some(std::time::Instant::now());
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
                    // Same-document anchor: jump to the matching heading.
                    "scroll" => {
                        let wanted = rest.trim().to_lowercase();
                        let idx = self.active_tab().and_then(|tab| {
                            tab.outline
                                .iter()
                                .find(|entry| entry.text.to_lowercase() == wanted)
                                .map(|entry| entry.block)
                        });
                        match idx {
                            Some(idx) => self.jump_to_block(idx),
                            None => Task::none(),
                        }
                    }
                    // Footnote reference: jump to the definition.
                    "fnref" => {
                        let wanted = rest.trim();
                        let idx = self.active_tab().and_then(|tab| {
                            tab.doc.blocks.iter().position(|block| match block {
                                vault_md::Block::FootnoteDef { label, .. } => label == wanted,
                                _ => false,
                            })
                        });
                        match idx {
                            Some(idx) => self.jump_to_block(idx),
                            None => Task::none(),
                        }
                    }
                    // `tag:` routes to the tag panel (M4b).
                    "tag" => {
                        self.panel = SidePanel::Tags;
                        self.selected_tag = Some(rest.to_string());
                        Task::none()
                    }
                    _ => Task::none(),
                }
            }
            Message::SidePanel(panel) => {
                self.panel = panel;
                Task::none()
            }
            Message::SearchInput(query) => {
                self.search_query = query;
                Task::none()
            }
            Message::SearchSubmitted => {
                let Some(vault) = self.vault.as_ref() else {
                    return Task::none();
                };
                let root = vault.root().to_path_buf();
                let query = self.search_query.clone();
                // Off the UI thread (tokio worker): keeps typing responsive
                // on large vaults. Sync file IO inside is fine at vault scale.
                Task::perform(
                    async move {
                        crate::search::search(&root, &query, SEARCH_MAX_HITS).unwrap_or_default()
                    },
                    |hits| cosmic::Action::App(Message::SearchDone(hits)),
                )
            }
            Message::SearchDone(hits) => {
                self.search_hits = hits;
                Task::none()
            }
            Message::SearchOpen(i) => {
                let Some(hit) = self.search_hits.get(i) else {
                    return Task::none();
                };
                let path = hit.path.clone();
                let query = self.search_query.clone();
                let open_task = self.open_tab(path);
                if self.pane == PaneMode::Source {
                    self.pane = PaneMode::Split;
                }
                // Scroll to the first block whose text contains the query.
                // No deferral needed: the runner rebuilds the view (with the
                // new tab and Split panes) *before* dispatching the widget
                // operation, so the measure op sees the fresh tree.
                let jump = self
                    .active_tab()
                    .and_then(|tab| crate::reading::find_block_with_text(&tab.doc, &query))
                    .map(|idx| self.jump_to_block(idx))
                    .unwrap_or_else(Task::none);
                Task::batch([open_task, jump])
            }
            Message::TagSelected(tag) => {
                // Clicking the selected tag again clears the filter.
                if self.selected_tag.as_deref() == Some(tag.as_str()) {
                    self.selected_tag = None;
                } else {
                    self.selected_tag = Some(tag);
                }
                Task::none()
            }
            Message::JumpToBlock(idx) => self.jump_to_block(idx),
            Message::SwitcherToggle => {
                if self.vault.is_none() {
                    // No vault: the overlay would render nowhere (empty state).
                    return Task::none();
                }
                self.switcher.open = !self.switcher.open;
                if self.switcher.open {
                    self.switcher.query.clear();
                    self.switcher.selected = 0;
                    self.rerank_switcher();
                    return cosmic::widget::text_input::focus::<Message>(Id::new(SWITCHER_INPUT))
                        .map(cosmic::Action::App);
                }
                Task::none()
            }
            Message::SwitcherInput(query) => {
                self.switcher.query = query;
                self.switcher.selected = 0;
                self.rerank_switcher();
                Task::none()
            }
            Message::SwitcherMove(delta) => {
                if !self.switcher.items.is_empty() {
                    let len = self.switcher.items.len() as i32;
                    let next = (self.switcher.selected as i32 + delta).rem_euclid(len);
                    self.switcher.selected = next as usize;
                }
                Task::none()
            }
            Message::SwitcherActivate => {
                let Some(path) = self.switcher.items.get(self.switcher.selected).cloned() else {
                    return Task::none();
                };
                self.switcher.open = false;
                self.open_tab(path)
            }
            Message::SwitcherSelect(i) => {
                let Some(path) = self.switcher.items.get(i).cloned() else {
                    return Task::none();
                };
                self.switcher.open = false;
                self.open_tab(path)
            }
            Message::KeyPressed(modifiers, key) => {
                if self.switcher.open {
                    match key {
                        Key::Named(Named::ArrowUp) => self.update(Message::SwitcherMove(-1)),
                        Key::Named(Named::ArrowDown) => self.update(Message::SwitcherMove(1)),
                        Key::Named(Named::Escape) => {
                            self.switcher.open = false;
                            Task::none()
                        }
                        _ => Task::none(),
                    }
                } else if modifiers.control() {
                    match &key {
                        Key::Character(c) if c == "p" => self.update(Message::SwitcherToggle),
                        Key::Character(c) if c == "s" => self.save_active(),
                        Key::Character(c) if c == "d" => self.update(Message::DailyNote),
                        // Shift+F arrives as either case depending on the
                        // platform's key mapping.
                        Key::Character(c) if c == "f" || c == "F" => {
                            self.update(Message::SidePanel(SidePanel::Search))
                        }
                        Key::Character(c) if c == "w" => match self.active {
                            Some(idx) => self.update(Message::TabClose(idx)),
                            None => Task::none(),
                        },
                        _ => Task::none(),
                    }
                } else {
                    Task::none()
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
        // Key events always flow (quick switcher + Ctrl+S work everywhere);
        // update filters by app state, so unused keys are cheap noise.
        let keys = cosmic::iced::event::listen_with(|event, _status, _window| match event {
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                ..
            }) => Some(Message::KeyPressed(modifiers, key)),
            _ => None,
        });
        match &self.vault {
            Some(_) => cosmic::iced::Subscription::batch([
                keys,
                cosmic::iced::time::every(std::time::Duration::from_secs(2))
                    .map(|_| Message::WatchTick),
            ]),
            None => keys,
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

    #[test]
    fn autosave_fires_only_after_the_idle_window() {
        use std::time::Duration;
        // No edit recorded: never save.
        assert!(!autosave_due(None));
        // Recent edit: not yet.
        assert!(!autosave_due(Some(Duration::from_millis(500))));
        // Idle past the window: save.
        assert!(autosave_due(Some(Duration::from_secs(2))));
        assert!(autosave_due(Some(Duration::from_secs(10))));
    }
}
