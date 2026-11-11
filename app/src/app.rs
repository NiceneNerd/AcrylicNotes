//! The COSMIC application model: state, messages, update, subscriptions.
//!
//! Working rules (M4): all vault logic goes through `vault-core`/`vault-md`;
//! every write is bracketed by the safe save path; the pieces that need
//! testing without a GUI (save planning, new-note names) are pure functions.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use cosmic::app::{Core, Task};
use cosmic::iced::core::widget::Id;
use cosmic::iced::keyboard::{key::Named, Key, Modifiers};
use cosmic::iced::widget::image::Handle as ImageHandle;
use cosmic::iced::widget::text_editor::{self, Action, Content, Cursor, Edit, Motion, Position};
use cosmic::iced::Color;
use cosmic::widget::{icon, nav_bar, segmented_button};
use unicode_segmentation::UnicodeSegmentation;

use vault_core::vault::VaultError;
use vault_core::{NoteSource, ObsidianSettings, Vault, VaultIndex};
use vault_md::{parse_note, Doc};

use crate::live;
use crate::settings;
use crate::tree::{self, TreeNode};

/// How the note area is laid out. Defined in settings (it persists) and
/// re-exported here.
pub use crate::settings::PaneMode;

/// How generated wikilinks spell their target. Defined in settings (it
/// persists) and re-exported here.
pub use crate::settings::LinkPathFormat;

/// How the file tree orders its notes. Defined in settings (it persists)
/// and re-exported here.
pub use crate::settings::TreeSort;

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

    /// XDG icon name for the nav bar (verified present in the Cosmic/Pop
    /// icon themes).
    pub fn icon(self) -> &'static str {
        match self {
            SidePanel::Files => "folder-symbolic",
            SidePanel::Search => "edit-find-symbolic",
            SidePanel::Tags => "edit-tag-symbolic",
            SidePanel::Links => "link-symbolic",
            SidePanel::Outline => "view-list-symbolic",
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

/// How many notes the quick switcher shows at most.
pub const SWITCHER_MAX_ITEMS: usize = 20;

/// Id of the find bar's text input (focused when find opens).
pub const FIND_INPUT: &str = "find-input";

/// State of the in-note find bar (Ctrl+F).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Find {
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    /// Index of the selected match (clamped against the live match list).
    pub current: usize,
}

/// Wikilink suggestions for the `[[` currently being typed in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikiComplete {
    /// Ranked note paths (the rows show titles, dimmed parents).
    pub items: Vec<String>,
    pub selected: usize,
}

/// The live preview's editing state (`Live` pane mode): one open block
/// region and its raw-source buffer.
pub struct LiveEdit {
    /// Index into the note's `live::slices` table.
    pub block: usize,
    /// The region's source. The focused text editor renders exactly this
    /// buffer; edits land here and are spliced back into the note text.
    pub editor: Content<cosmic::Renderer>,
}

/// How many wikilink suggestions to show at once.
const WIKI_MAX_ITEMS: usize = 8;

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
    /// Undo snapshots: the text and cursor before each edit, newest last.
    undo: Vec<(String, Cursor)>,
    /// Redo snapshots, filled by undo, consumed by redo.
    redo: Vec<(String, Cursor)>,
    /// Cached status-bar counts (refreshed with the doc).
    pub words: usize,
    pub chars: usize,
    /// The editor cursor's file line when preview scroll sync last ran, so
    /// sync only fires when the caret changes line.
    synced_line: Option<usize>,
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
                spans: Vec::new(),
            },
            doc_text: String::new(),
            outline: Vec::new(),
            folds: HashSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            words: 0,
            chars: 0,
            synced_line: None,
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
            let new_doc = parse_note(&text);
            // Fold ids are positional callout indexes: they survive an
            // edit only while the foldable inventory is unchanged (same
            // count ⇒ same id assignment, so typing inside a callout
            // keeps its fold). Adding or removing one shifts every later
            // id — the set then points at different blocks and resets.
            let old_foldables = crate::reading::foldable_count(&self.doc);
            if crate::reading::foldable_count(&new_doc) != old_foldables {
                self.folds.clear();
            }
            self.doc = new_doc;
            self.words = count_words(&text);
            self.chars = text.chars().count();
            self.doc_text = text;
            self.outline = crate::reading::outline(&self.doc);
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

/// The directory part of a vault-relative path ("Notes/Journal" for
/// "Notes/Journal/Daily.md"), or "" for root-level files.
pub(crate) fn parent_of(path: &str) -> String {
    match path.rfind('/') {
        Some(pos) => path[..pos].to_string(),
        None => String::new(),
    }
}

/// A note row's label in the file tree: the file name, with the `.md`
/// extension hidden when the setting asks for it (Obsidian's default).
pub(crate) fn tree_note_label(name: &str, hide_md_extension: bool) -> String {
    if hide_md_extension {
        name.trim_end_matches(".md").to_string()
    } else {
        name.to_string()
    }
}

/// Inline markdown formatting a shortcut can apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Strike,
    Code,
    Highlight,
    Link,
}

impl Format {
    /// The markdown markers wrapped around the selection.
    fn wrap(self) -> &'static str {
        match self {
            Format::Bold => "**",
            Format::Italic => "*",
            Format::Strike => "~~",
            Format::Code => "`",
            Format::Highlight => "==",
            // Links are built from brackets, not symmetric markers.
            Format::Link => "",
        }
    }
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
    /// A context menu opened (records the node it belongs to) or closed.
    TreeMenuTarget(Option<String>),
    /// A tree context-menu command for the last `TreeMenuTarget(Some(..))`.
    TreeMenu(TreeMenuAction),
    /// Keystrokes for the inline tree-edit input.
    TreeEditInput(String),
    TreeEditSubmit,
    TreeEditCancel,
    /// Wayland surface actions for context-menu popups (routed to libcosmic).
    Surface(cosmic::surface::Action<Message>),
    TabClose(usize),
    /// A nav-bar (sidebar panel) item was activated; the entity carries the
    /// panel via the model's data.
    NavSelect(nav_bar::Id),
    /// A segmented-control item (Source/Reading/Split) was activated.
    PaneActivated(segmented_button::Entity),
    /// A tab-bar tab was activated; the payload is the tab index.
    TabActivated(usize),
    /// A tab-bar close button was pressed; the payload is the tab index.
    TabCloseRequested(usize),
    EditorAction(Action),
    /// Undo the last edit in the active tab (Ctrl+Z).
    EditorUndo,
    /// Redo an undone edit (Ctrl+Shift+Z / Ctrl+Y).
    EditorRedo,
    /// Apply inline markdown formatting around the selection (or insert
    /// empty markers when nothing is selected).
    EditorFormat(Format),
    /// Insert a footnote reference at the cursor and append its definition.
    EditorFootnote,
    // --- find in current note (Ctrl+F) ---
    /// Open (or refocus) the find bar, prefilling from the selection.
    FindOpen,
    FindInput(String),
    FindClose,
    FindNext,
    FindPrev,
    FindToggleCase,
    FindReplaceInput(String),
    /// Replace the selected match.
    FindReplace,
    /// Replace every match in the active tab.
    FindReplaceAll,
    // --- wikilink autocomplete ---
    /// Accept the highlighted suggestion (Tab or click on a row).
    WikiAccept,
    /// Accept the suggestion at `index` (a dropdown row was clicked).
    WikiSelect(usize),
    /// Move the highlight (Ctrl+Tab / Ctrl+Shift+Tab).
    WikiCycle(i32),
    /// Dismiss the dropdown (Esc).
    WikiDismiss,
    // --- editor context menu ---
    /// Copy the selection to the clipboard.
    EditorCopy,
    /// Cut: copy the selection, then delete it.
    EditorCut,
    /// Read the clipboard (the paste entry point; result is
    /// `EditorPasteText`).
    EditorPasteAsk,
    /// Insert the clipboard text at the cursor (replacing a selection).
    EditorPasteText(Option<String>),
    /// Select the entire buffer.
    EditorSelectAll,
    /// Open a non-markdown vault file with the platform handler.
    OpenAttachment(String),
    // --- settings drawer ---
    /// Open or close the settings context drawer.
    SettingsToggle,
    SettingsAutoPair(bool),
    SettingsWikiComplete(bool),
    SettingsWordWrap(bool),
    SettingsFontMono(bool),
    /// Set the custom editor font family (live from the settings
    /// input; empty follows the monospace toggle).
    SettingsEditorFontFamily(String),
    /// Nudge the source editor's font size (clamped).
    SettingsEditorFontSize(i32),
    /// Set the custom reading-view font family (empty follows the
    /// system font).
    SettingsReadingFontFamily(String),
    /// Nudge the reading view's font size (clamped).
    SettingsReadingFontSize(i32),
    /// Choose the margin the reading view leaves around its text.
    SettingsReadingMargin(settings::ReadingMargin),
    /// Zoom the pane in focus (Ctrl+= / Ctrl+scroll up).
    ZoomIn,
    /// Zoom the pane in focus out (Ctrl+- / Ctrl+scroll down).
    ZoomOut,
    // --- link updates after rename/move ---
    /// Apply the pending wikilink rewrites (banner confirmation).
    LinkUpdatesApply,
    /// Leave the invalidated links as they are.
    LinkUpdatesDismiss,
    // --- sidebar collapse ---
    /// Show or hide the sidebar (file tree and other panels).
    ToggleSidebar,
    // --- tree drag-and-drop ---
    /// The cursor moved (global mouse subscription).
    MouseMoved(cosmic::iced::Point),
    /// The left button went down (global mouse subscription).
    MousePressed,
    /// The left button came up (global mouse subscription).
    MouseReleased,
    /// Fresh tree-row bounds from the measure operation.
    TreeBounds(crate::drag::TreeBounds),
    // --- window and zoom ---
    /// The mouse wheel turned (global mouse subscription); the value is
    /// the vertical delta in lines (up is positive).
    MouseWheel(f32),
    /// The window was resized (global window subscription); the value is
    /// the new width in pixels.
    WindowResized(f32),
    /// The reading view was clicked: the zoom shortcuts now act on it.
    ReadingClicked,
    SettingsTheme(settings::ThemeChoice),
    SettingsDefaultPane(PaneMode),
    SettingsHideMdExt(bool),
    /// Nudge the autosave delay in seconds (clamped 1..=60).
    SettingsAutosaveDelta(i32),
    SettingsScrollSync(bool),
    // --- attachment file-type filter ---
    /// Typed into the drawer's extension input.
    AttachmentInput(String),
    /// Add the normalized extension from the input to the tree filter.
    AttachmentAdd,
    /// Remove an extension from the tree filter.
    AttachmentRemove(String),
    /// How generated wikilinks spell their target.
    SettingsLinkFormat(LinkPathFormat),
    // --- Unicode codepoint entry (Ctrl+Shift+U, GTK-style) ---
    /// Begin codepoint entry in the focused editor.
    UnicodeStart,
    /// A character typed while entry is open (only hex digits count).
    UnicodeHex(char),
    /// Remove the last hex digit; exits when none remain.
    UnicodeBackspace,
    /// Insert the collected codepoint and return to editing.
    UnicodeCommit,
    /// A key that does nothing during entry (e.g. Tab).
    UnicodeIgnore,
    /// Abandon codepoint entry.
    UnicodeCancel,
    /// Delete the word before (false) or after (true) the cursor.
    EditorDeleteWord(bool),
    /// A bracket/quote was typed: pair it around the selection, or insert
    /// the pair with the cursor in the middle.
    EditorAutoPair(char),
    Save,
    ToggleFolder(String),
    /// Set the file tree's sort order (persisted).
    TreeSort(TreeSort),
    /// Collapse every folder of the file tree.
    TreeCollapseAll,
    /// Expand every folder of the file tree.
    TreeExpandAll,
    /// Fold every folder away except the ones on the current note's
    /// path, which stay open so the note remains visible.
    TreeCollapseOthers,
    /// Open the folders on the current note's path, if they are folded.
    TreeRevealCurrent,
    ToggleFold(u64),
    /// A link span was clicked; the URI scheme routes it.
    OpenUri(String),
    /// An http(s) link: hand it to the OS opener.
    OpenExternal(String),
    // --- M4b: navigation panels, quick switcher, search ---
    SidePanel(SidePanel),
    SearchInput(String),
    SearchSubmitted,
    SearchDone(Vec<vault_core::search::SearchHit>),
    /// Open search hit `i` and scroll its reading view to the match.
    SearchOpen(usize),
    TagSelected(String),
    /// Scroll the active tab's reading view to top-level block `idx`.
    JumpToBlock(usize),
    // --- live preview (`Live` pane mode) ---
    /// Open block region `idx` in the live preview for editing.
    LiveFocus(usize),
    /// An arrow key at a region boundary crossed into the neighbor.
    LiveUp,
    LiveDown,
    /// Backspace at a region's start: delete the newline before it.
    LiveBackspace,
    /// Delete at a region's end: delete the newline after it.
    LiveDelete,
    /// Ctrl+Backspace / Ctrl+Delete: delete the word before (or after)
    /// the caret.
    LiveDeleteWord(bool),
    /// Commit the open region and let the blocks render again (Escape).
    LiveDismiss,
    /// Apply markdown formatting to the focused region's selection.
    LiveFormat(Format),
    SwitcherToggle,
    SwitcherInput(String),
    SwitcherMove(i32),
    SwitcherActivate,
    /// Open switcher item `i` directly (mouse click).
    SwitcherSelect(usize),
    /// Raw key events; update decides what matters (switcher, shortcuts).
    /// A keyboard event reached the global subscription, with the status
    /// telling whether a widget already captured it (editor-scoped
    /// bindings capture; we must not double-fire global shortcuts then).
    KeyPressed(cosmic::iced::event::Status, Modifiers, Key),
    /// A key was released: the modifiers held are updated, so Ctrl+scroll
    /// stops zooming once Ctrl is let go.
    KeyReleased(Key),
}

/// Which pane the zoom shortcuts (Ctrl+= / Ctrl+scroll) act on.
///
/// The source editor and the rendered view have independent font sizes,
/// so the shortcut has to pick one. It follows the pane in focus: the
/// editor while typing, the reading view while reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomTarget {
    /// The source editor (and the live pane's focused block).
    #[default]
    Source,
    /// The rendered reading view.
    Reading,
}

/// The window width (in pixels) below which the split layout folds down
/// to the source pane: two panes side by side need more room than one.
pub const NARROW_WINDOW_WIDTH: f32 = 480.0;

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
    /// Links invalidated by the last rename/move, awaiting the user's
    /// banner choice (rewrite or leave).
    pub pending_links: Option<PendingLinkUpdates>,
    pub error: Option<String>,
    pub vault_input: String,
    pub strict_breaks: bool,
    pub images: RefCell<std::collections::HashMap<String, ImageHandle>>,
    // --- M4b ---
    pub panel: SidePanel,
    pub search_query: String,
    pub search_hits: Vec<vault_core::search::SearchHit>,
    pub selected_tag: Option<String>,
    pub switcher: Switcher,
    // --- M4c ---
    /// When the editor last saw an edit action; drives autosave-on-idle.
    pub last_edit: Option<std::time::Instant>,
    /// The backdrop style the platform offered at startup (glass).
    pub glass_style: vault_glass::GlassStyle,
    /// The theme choice the user made (persisted; `System` follows desktop).
    pub theme: settings::ThemeChoice,
    /// Hide the `.md` extension on note rows in the file tree.
    pub hide_md_extension: bool,
    /// Follow the editor caret in the visible reading pane.
    pub scroll_sync: bool,
    /// Insert closing brackets/quotes and type over them.
    pub auto_pair: bool,
    /// Suggest note titles while typing `[[`.
    pub wiki_autocomplete: bool,
    /// Soft-wrap long lines in the editor.
    pub word_wrap: bool,
    /// Edit in the monospace font (off = the system UI font). Ignored
    /// when `editor_font_family` is set.
    pub editor_font_mono: bool,
    /// Custom editor font family name; empty follows the monospace
    /// toggle.
    pub editor_font_family: String,
    /// Editor font size in pixels.
    pub editor_font_size: u16,
    /// Custom reading-view font family name; empty follows the system
    /// font.
    pub reading_font_family: String,
    /// Reading-view font size in pixels. The source editor keeps its own
    /// size, so the two panes are sized independently.
    pub reading_font_size: u16,
    /// The margin the reading view leaves around its text column.
    pub reading_margin: settings::ReadingMargin,
    /// Which pane the zoom shortcuts act on (the source editor or the
    /// rendered view). Tracked from the last interaction, so the split
    /// pane zooms the pane the user is working in.
    pub zoom_target: ZoomTarget,
    /// The modifiers held at the last key event (Ctrl+scroll needs to
    /// know whether Ctrl is down).
    modifiers: Modifiers,
    /// The split layout was folded away by a narrow window; restore it
    /// once the window grows again.
    restore_split: bool,
    /// Whether the sidebar (file tree and other panels) is shown;
    /// toggled from the titlebar button.
    pub sidebar_open: bool,
    // --- tree drag-and-drop (drag.rs) ---
    /// An armed or active tree drag: press position, source row, hover
    /// target. `None` when no left-press is pending a possible drag.
    pub tree_drag: Option<TreeDrag>,
    /// The last seen cursor position (presses carry no position).
    pub last_cursor: Option<cosmic::iced::Point>,
    /// Measured tree-row bounds for drag hit-testing.
    pub tree_bounds: crate::drag::TreeBounds,
    /// A bounds-measure task is in flight (don't queue another).
    bounds_pending: bool,
    /// Seconds of editor idle before autosave fires.
    pub autosave_secs: u64,
    /// The pane layout a fresh session starts in.
    pub default_pane: PaneMode,
    /// Non-markdown file extensions shown in the file tree; empty = all.
    pub attachment_extensions: Vec<String>,
    /// How generated wikilinks spell their target.
    pub link_path_format: LinkPathFormat,
    /// How the file tree orders its notes (persisted).
    pub tree_sort: TreeSort,
    /// Draft text of the drawer's extension input.
    pub attachment_input: String,
    /// Hex digits collected so far during Unicode codepoint entry
    /// (Ctrl+Shift+U); `None` while the mode is off.
    pub unicode_input: Option<String>,
    /// Pending inline tree edit (new note / new folder / rename), if any.
    pub tree_edit: Option<tree::TreeEdit>,
    /// The in-note find bar state (Ctrl+F), when open.
    pub find: Option<Find>,
    /// Wikilink suggestions for the `[[` being typed, when active.
    pub wiki_complete: Option<WikiComplete>,
    /// The live preview's open editing region (`Live` pane mode), if any.
    pub live: Option<LiveEdit>,
    /// The node path the open context menu belongs to (`None` when closed).
    pub tree_menu_target: Option<String>,
    // --- COSMIC chrome models ---
    /// Sidebar panel switcher (nav bar): one item per `SidePanel`.
    pub nav_model: nav_bar::Model,
    /// Header pane-mode segmented control (Source/Reading/Split).
    pub pane_model: segmented_button::Model<segmented_button::SingleSelect>,
}

/// Id of the inline tree-edit text input (focused when the edit starts).
pub const TREE_EDIT_INPUT: &str = "tree-edit-input";

/// The sidebar panel's fixed width in pixels (no resize handle; the
/// panel is shown or hidden whole).
pub const SIDEBAR_WIDTH: f32 = 250.0;

/// The sidebar panel's background opacity: the theme's opaque background
/// base drawn back this far, so the frosted backdrop still reads through
/// faintly. 1.0 would be a fully solid surface.
pub const SIDEBAR_OPACITY: f32 = 0.9;

/// Id of the settings drawer's file-extension input.
pub const ATTACHMENT_INPUT: &str = "attachment-ext-input";

/// Id of the settings drawer's custom editor-font input.
pub const EDITOR_FONT_INPUT: &str = "editor-font-input";

/// Id of the settings drawer's custom reading-font input.
pub const READING_FONT_INPUT: &str = "reading-font-input";

/// Id of the note editor itself. A *custom* id makes iced persist the
/// editor's widget state (its focus) in the UI cache and restore it by
/// name, so structural view changes — pane-mode switches, the quick
/// switcher overlay, popups — cannot strip the editor's focus and leave
/// it deaf to the keyboard.
pub const EDITOR_INPUT: &str = "note-editor";

/// Commands of the sidebar tree's right-click menu. `Copy` because cosmic's
/// `menu::Action` requires it; the target node rides separately via
/// `Message::TreeMenuTarget`, recorded when the menu opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeMenuAction {
    NewNote,
    NewFolder,
    Rename,
    Delete,
}

impl cosmic::widget::menu::Action for TreeMenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        Message::TreeMenu(*self)
    }
}

/// The file tree's sort menu: pick the order the notes follow.
/// `Copy` because cosmic's `menu::Action` requires it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortMenuAction(pub TreeSort);

impl cosmic::widget::menu::Action for SortMenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        Message::TreeSort(self.0)
    }
}

/// Pure autosave decision: an idle period measured since the last edit is
/// due for a save. Unit-testable without an `Instant`.
/// Pure autosave decision: an idle period measured since the last edit is
/// at least the configured delay.
fn autosave_due(idle_since_last_edit: Option<std::time::Duration>, delay_secs: u64) -> bool {
    idle_since_last_edit.is_some_and(|d| d >= std::time::Duration::from_secs(delay_secs))
}

/// Persist one settings change: load-modify-store so every other field
/// survives.
fn persist_setting(mutate: impl FnOnce(&mut settings::AppSettings)) {
    let mut persisted = settings::load();
    mutate(&mut persisted);
    settings::store(&persisted);
}

/// A lock for the tests that write the real settings file.
///
/// Cargo runs the tests in parallel and the settings are stored with a
/// load-modify-store, so two tests writing at once would lose a field.
/// The guard is taken for the whole test.
#[cfg(test)]
pub(crate) static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the settings lock, ignoring a poisoned one (a test that failed
/// already reported itself).
#[cfg(test)]
pub(crate) fn settings_guard() -> std::sync::MutexGuard<'static, ()> {
    SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Normalize a user-typed file extension: trim, drop leading dots,
/// lowercase. `None` when nothing remains.
fn normalize_extension(raw: &str) -> Option<String> {
    let ext = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    (!ext.is_empty()).then_some(ext)
}

/// Whether an attachment path passes the tree's extension filter. An
/// empty list shows everything (the default); otherwise the file's
/// extension (case-insensitive) must be listed.
fn attachment_shown(path: &str, extensions: &[String]) -> bool {
    if extensions.is_empty() {
        return true;
    }
    path.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .is_some_and(|ext| extensions.contains(&ext))
}

// --- Unicode codepoint entry (Ctrl+Shift+U) --------------------------------

/// GTK caps hex entry at 6 digits (`0x10FFFF` is the last codepoint).
const UNICODE_MAX_DIGITS: usize = 6;

/// Parse collected hex digits into a codepoint, rejecting surrogates
/// and out-of-range values.
pub(crate) fn unicode_char(hex: &str) -> Option<char> {
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
}

// --- editor commands (undo, formatting, footnotes, word delete) ------------

/// Undo snapshots kept per tab; older entries are dropped beyond this.
const UNDO_LIMIT: usize = 100;

/// Snapshot the tab's text and cursor for undo, discarding any redo
/// history (a new edit after undo branches the timeline).
fn push_undo(tab: &mut Tab) {
    let text = tab.content.text();
    let cursor = tab.content.cursor();
    tab.undo.push((text, cursor));
    if tab.undo.len() > UNDO_LIMIT {
        tab.undo.remove(0);
    }
    tab.redo.clear();
}

/// Pop the last undo snapshot into the tab. Returns whether anything was
/// restored (the caller refreshes derived state).
fn undo_tab(tab: &mut Tab) -> bool {
    match tab.undo.pop() {
        Some((text, cursor)) => {
            let current = (tab.content.text(), tab.content.cursor());
            tab.redo.push(current);
            tab.content = Content::with_text(&text);
            tab.content.move_to(cursor);
            true
        }
        None => false,
    }
}

/// Pop the last redo snapshot into the tab (the inverse of `undo_tab`).
fn redo_tab(tab: &mut Tab) -> bool {
    match tab.redo.pop() {
        Some((text, cursor)) => {
            let current = (tab.content.text(), tab.content.cursor());
            tab.undo.push(current);
            tab.content = Content::with_text(&text);
            tab.content.move_to(cursor);
            true
        }
        None => false,
    }
}

/// The editor actions that apply `format`: wrapping a non-empty selection
/// with the markers, or inserting empty markers and stepping the cursor
/// between them.
fn format_actions(selection: Option<String>, format: Format) -> Vec<Action> {
    let sel = selection.filter(|s| !s.is_empty());
    match format {
        Format::Link => match sel {
            Some(s) => paste_then_left(format!("[{s}]()"), 1),
            None => paste_then_left("[]()".to_string(), 3),
        },
        other => {
            let wrap = other.wrap();
            match sel {
                Some(s) => vec![Action::Edit(Edit::Paste(Arc::new(format!(
                    "{wrap}{s}{wrap}"
                ))))],
                None => paste_then_left(format!("{wrap}{wrap}"), wrap.chars().count()),
            }
        }
    }
}

/// Paste `text`, then move the cursor `left` positions back (to land
/// inside inserted markers).
fn paste_then_left(text: String, left: usize) -> Vec<Action> {
    let mut actions = vec![Action::Edit(Edit::Paste(Arc::new(text)))];
    actions.extend((0..left).map(|_| Action::Move(Motion::Left)));
    actions
}

/// Insert a `[^N]` reference at the cursor and a `[^N]: ` definition at
/// the end of the document; N is the first number without a definition.
///
/// The definitions are normalized on the way: every top-level
/// `[^label]:` line gets exactly one blank line before it. Markdown
/// needs that blank line — without it the definition is a lazy
/// continuation of the paragraph above and never becomes a footnote.
///
/// Returns the new text and the cursor's place in it (right after the
/// inserted reference).
fn insert_footnote(text: &str, cursor: Position, n: u32) -> (String, Position) {
    let lines: Vec<&str> = text.split('\n').collect();
    let (mut lines, cursor_line) = normalise_footnote_definitions(lines, cursor.line);
    // The reference goes in at the cursor.
    let line = cursor_line.min(lines.len().saturating_sub(1));
    let reference = format!("[^{n}]");
    let column = char_boundary(&lines[line], cursor.column);
    lines[line].insert_str(column, &reference);
    let cursor = Position {
        line,
        column: column + reference.len(),
    };
    // The definition goes at the end, under one blank line.
    if lines.last().is_some_and(|last| !last.is_empty()) {
        lines.push(String::new());
    }
    lines.push(format!("[^{n}]: "));
    (lines.join("\n"), cursor)
}

/// Give every top-level footnote definition exactly one blank line
/// before it.
///
/// The cursor's line is tracked: a blank line removed above it moves it
/// up, one added moves it down.
fn normalise_footnote_definitions(lines: Vec<&str>, cursor_line: usize) -> (Vec<String>, usize) {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut cursor = cursor_line;
    let mut fence: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        // A definition inside a code fence is code, not a definition.
        let in_code = match fence_of(line) {
            Some((character, length)) => match fence {
                Some((open, open_len)) if open == character && length >= open_len => {
                    // The fence closes here.
                    fence = None;
                    false
                }
                Some(_) => true,
                None => {
                    fence = Some((character, length));
                    true
                }
            },
            None => fence.is_some(),
        };
        if !in_code && is_footnote_def(line) {
            // Drop the blank lines that are there…
            let mut removed = 0;
            while out.last().is_some_and(|last| last.trim().is_empty()) {
                out.pop();
                removed += 1;
            }
            // …and put exactly one back.
            let added = usize::from(!out.is_empty());
            if added == 1 {
                out.push(String::new());
            }
            if i < cursor_line {
                cursor = cursor + added - removed;
            }
        }
        out.push((*line).to_string());
    }
    (out, cursor)
}

/// Whether a line starts a footnote definition: `[^label]:`.
fn is_footnote_def(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("[^") else {
        return false;
    };
    let Some(end) = rest.find(']') else {
        return false;
    };
    end > 0 && rest[end + 1..].starts_with(':')
}

/// The code fence a line opens or closes, if it is one: the character
/// and the run length.
fn fence_of(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let character = rest.chars().next()?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = rest.chars().take_while(|&c| c == character).count();
    (length >= 3).then_some((character, length))
}

/// The largest character boundary at or before `column`.
fn char_boundary(line: &str, column: usize) -> usize {
    let mut at = column.min(line.len());
    while at > 0 && !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The first footnote number that has no `[^N]:` definition in `text`.
fn next_footnote_number(text: &str) -> u32 {
    let mut used = HashSet::new();
    let mut rest = text;
    while let Some(pos) = rest.find("[^") {
        let after = &rest[pos + 2..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            if let Ok(n) = digits.parse::<u32>() {
                // Only definitions reserve a number; bare references do not.
                if after[digits.len()..].starts_with("]:") {
                    used.insert(n);
                }
            }
        }
        rest = after;
    }
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

/// Delete one word: with a selection the edit alone removes it; without
/// one, select back/forward a word first (Backspace/Delete then drop it).
fn word_delete_actions(has_selection: bool, right: bool) -> Vec<Action> {
    let edit = if right { Edit::Delete } else { Edit::Backspace };
    if has_selection {
        vec![Action::Edit(edit)]
    } else {
        vec![
            Action::Select(if right {
                Motion::WordRight
            } else {
                Motion::WordLeft
            }),
            Action::Edit(edit),
        ]
    }
}

/// The byte range of the word before the caret, the way the editor's own
/// word motion finds it: from the start of the last word before the
/// caret up to the caret, or — when the caret sits at the start of a
/// line — the line break.
///
/// The live editor needs this at file level: its buffer holds one
/// region, so the word can live in the neighbouring one.
fn word_before(text: &str, offset: usize) -> Option<(usize, usize)> {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let prefix = &before[line_start..];
    if !prefix.is_empty() {
        // The last word that starts before the caret; without one, the
        // whole prefix goes (the whitespace in front of the caret).
        let start = prefix
            .unicode_word_indices()
            .rev()
            .map(|(i, _)| i)
            .find(|&i| i < prefix.len())
            .unwrap_or(0);
        return Some((line_start + start, offset));
    }
    // The caret is at the start of a line: the motion reaches back to
    // the end of the line before it.
    let previous_end = line_start.checked_sub(1)?;
    Some((previous_end, offset))
}

/// The byte range of the word after the caret: the word's characters
/// (with the whitespace in front of them), or — at the end of a line —
/// the line break.
fn word_after(text: &str, offset: usize) -> Option<(usize, usize)> {
    let rest = &text[offset..];
    let line_end = rest.find('\n').map_or(text.len(), |i| offset + i);
    let suffix = &text[offset..line_end];
    if !suffix.is_empty() {
        // The first word that ends after the caret; without one, the
        // rest of the line goes.
        let end = suffix
            .unicode_word_indices()
            .map(|(i, word)| i + word.len())
            .find(|&i| i > 0)
            .unwrap_or(suffix.len());
        return Some((offset, offset + end));
    }
    // The caret is at the end of a line: the motion steps over the line
    // break.
    (line_end < text.len()).then_some((offset, line_end + 1))
}

/// The byte offset of a caret position (0-based line, byte column) in
/// the note text.
fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let mut offset = 0;
    for (i, current) in text.split('\n').enumerate() {
        if i == line {
            let mut at = offset + column.min(current.len());
            while at > offset && !text.is_char_boundary(at) {
                at -= 1;
            }
            return at;
        }
        offset += current.len() + 1;
    }
    text.len()
}

/// The caret position (0-based line, byte column) of a byte offset in
/// the note text.
fn position_of(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let column = offset - before.rfind('\n').map_or(0, |i| i + 1);
    (line, column)
}

/// Word count for the status bar: whitespace-separated runs.
pub(crate) fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The status bar's label: the note's totals, and the marked words and
/// characters when there is a selection.
pub(crate) fn status_label(tab: &Tab, selection: Option<&str>) -> String {
    let mut label = format!("{} words · {} characters", tab.words, tab.chars);
    if let Some(selection) = selection.filter(|text| !text.is_empty()) {
        let words = count_words(selection);
        let chars = selection.chars().count();
        label.push_str(&format!(" ({words} words · {chars} characters)"));
    }
    label
}

/// Whether the global keyboard subscription may act on a key event.
/// Captured events are skipped (a widget — usually the text editor's own
/// bindings — already consumed them), unless one of the app's own text
/// inputs has focus: their keys are exactly what the global handler routes.
fn global_key_allowed(status: cosmic::iced::event::Status, overlay_input_open: bool) -> bool {
    !matches!(status, cosmic::iced::event::Status::Captured) || overlay_input_open
}

// --- find in current note ---------------------------------------------------

/// One query hit: a line index and byte columns within that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Every occurrence of `query` in `text`, in order. Queries are matched
/// within single lines (a newline in the query matches nothing); columns
/// are byte offsets, like the editor's cursor columns.
pub(crate) fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<TextMatch> {
    if query.is_empty() || query.contains('\n') {
        return Vec::new();
    }
    let query_chars: Vec<char> = query.chars().collect();
    let mut out = Vec::new();
    for (line_no, line) in text.split('\n').enumerate() {
        let mut at = 0usize;
        while at <= line.len() {
            if !line.is_char_boundary(at) {
                at += 1;
                continue;
            }
            let mut probe = at;
            let mut hit = true;
            for &expected in &query_chars {
                match line[probe..].chars().next() {
                    Some(c) if char_matches(c, expected, case_sensitive) => {
                        probe += c.len_utf8();
                    }
                    _ => {
                        hit = false;
                        break;
                    }
                }
            }
            if hit {
                out.push(TextMatch {
                    line: line_no,
                    start: at,
                    end: probe,
                });
                at = probe;
            } else {
                at += line[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    out
}

/// Character equality under the find bar's case sensitivity. Only ASCII
/// case folding is applied, which keeps byte offsets stable.
fn char_matches(a: char, b: char, case_sensitive: bool) -> bool {
    a == b || (!case_sensitive && a.eq_ignore_ascii_case(&b))
}

/// Index of the first match at or after a cursor position (0 when the
/// cursor is past every match).
fn first_at_or_after(matches: &[TextMatch], line: usize, column: usize) -> usize {
    matches
        .iter()
        .position(|m| (m.line, m.start) >= (line, column))
        .unwrap_or(0)
}

/// The next (or previous) match index, wrapping around.
fn next_index(len: usize, current: usize, forward: bool) -> usize {
    if len == 0 {
        0
    } else if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

/// Splice `replacement` over every span (all within single lines,
/// non-overlapping, in order).
fn replace_spans(text: &str, spans: &[TextMatch], replacement: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        let mut copied = 0usize;
        let mut touched = false;
        for span in spans.iter().filter(|m| m.line == i) {
            out.push_str(&line[copied..span.start]);
            out.push_str(replacement);
            copied = span.end;
            touched = true;
        }
        if touched {
            out.push_str(&line[copied..]);
        } else {
            out.push_str(line);
        }
        if i + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

// --- wikilink autocomplete --------------------------------------------------

/// The wikilink being typed at a cursor position, if any: the byte column
/// of the opening `[[` in the cursor's line and the partial link text.
/// Suggesting stops when the link is closed (`]`), aliases are being typed
/// (`|`), or the `[[` sits on an earlier line.
pub(crate) fn wiki_query_at(text: &str, cursor: Position) -> Option<(usize, String)> {
    let line = text.split('\n').nth(cursor.line)?;
    let end = line.len().min(cursor.column);
    if !line.is_char_boundary(end) {
        return None;
    }
    let before = &line[..end];
    let open = before.rfind("[[")?;
    let query = &before[open + 2..];
    if query.contains(']') || query.contains('|') {
        return None;
    }
    Some((open, query.to_string()))
}

/// The closing brackets the auto-pair left directly after the caret.
///
/// Accepting a wiki completion replaces the `[[query` the user typed.
/// With auto-pair on, the caret also sits in front of the `]]` the
/// pairer inserted; those must be consumed with the rest, or the
/// accepted link ends up with four closing brackets.
pub(crate) fn wiki_close_run(text: &str, cursor: Position) -> usize {
    let Some(line) = text.split('\n').nth(cursor.line) else {
        return 0;
    };
    let end = line.len().min(cursor.column);
    if !line.is_char_boundary(end) {
        return 0;
    }
    line[end..]
        .chars()
        .take(2)
        .take_while(|c| *c == ']')
        .count()
}

/// The custom label of the wikilink being typed, if it has one.
///
/// The label follows the bar: `[[target|label]]`. While the caret sits
/// in the target part the label is *after* it, so accepting a
/// suggestion must keep it: `[[|]]` becomes `[[Target|]]`, not
/// `[[Target]]|]]`. Returns the label (without the bar and without the
/// closing brackets) and the column where the replacement ends — the
/// link's own `]]` stays where it is.
pub(crate) fn wiki_alias_at(text: &str, cursor: Position) -> Option<(String, usize)> {
    let line = text.split('\n').nth(cursor.line)?;
    let end = line.len().min(cursor.column);
    if !line.is_char_boundary(end) {
        return None;
    }
    let after = &line[end..];
    let bar = after.find('|')?;
    // The label runs to the link's closing brackets. A link that is not
    // closed on this line has no label worth preserving.
    let close = after[bar + 1..].find("]]")?;
    let label = after[bar + 1..bar + 1 + close].to_string();
    Some((label, end + bar + 1 + close))
}

/// The edit that accepts a wikilink suggestion at `cursor`.
///
/// The partial `[[query` is replaced by the link to `target`. A custom
/// label that follows the caret is kept, and the caret lands right
/// after the bar — in front of the label, ready to type it.
struct WikiReplacement {
    /// The column where the partial link starts.
    open: usize,
    /// The column where the replacement ends.
    end: usize,
    /// The text that replaces the partial link.
    text: String,
    /// Where the caret lands afterwards, when that is not the end of
    /// the pasted text.
    caret: Option<usize>,
}

/// The replacement for the partial wikilink at `cursor`, linking to
/// `target`. `None` when no wikilink is being typed there.
fn wiki_replacement(text: &str, cursor: Position, target: &str) -> Option<WikiReplacement> {
    let (open, _) = wiki_query_at(text, cursor)?;
    match wiki_alias_at(text, cursor) {
        Some((label, end)) => Some(WikiReplacement {
            open,
            end,
            // The link's own closing brackets are already in the text
            // and stay there.
            text: format!("[[{target}|{label}"),
            caret: Some(open + 2 + target.len() + 1),
        }),
        None => {
            let close = wiki_close_run(text, cursor);
            Some(WikiReplacement {
                open,
                end: cursor.column + close,
                text: format!("[[{target}]]"),
                caret: None,
            })
        }
    }
}

/// Rank note paths against a partial wikilink query by title (empty
/// queries pass everything through, like the quick switcher). Items are
/// paths so duplicate-titled notes stay distinguishable.
pub(crate) fn wiki_paths(query: &str, paths: &[String]) -> Vec<String> {
    let titles: Vec<String> = paths.iter().map(|p| title_of(p)).collect();
    crate::fuzzy::rank(query, &titles, WIKI_MAX_ITEMS)
        .into_iter()
        .map(|ranked| paths[ranked.index].clone())
        .collect()
}

// --- generated link targets (Obsidian's "new link format") -----------------

/// The wikilink text that refers to the note at `target`, as seen from
/// the note at `from`, given every note path (for duplicate titles).
/// All forms omit `.md` (wikilink convention) and resolve through
/// `VaultIndex`:
///
/// * `Shortest`: the bare title, unless several notes share it — then
///   the full vault path disambiguates.
/// * `Relative`: the path from `from`'s folder (`../folder/note`).
/// * `Absolute`: the full vault path.
pub(crate) fn link_target(
    from: &str,
    target: &str,
    notes: &[String],
    format: LinkPathFormat,
) -> String {
    fn stem(p: &str) -> &str {
        p.strip_suffix(".md").unwrap_or(p)
    }
    match format {
        LinkPathFormat::Absolute => stem(target).to_string(),
        LinkPathFormat::Shortest => {
            let title = title_of(target);
            let shared = notes.iter().filter(|note| title_of(note) == title).count();
            if shared <= 1 {
                title
            } else {
                stem(target).to_string()
            }
        }
        LinkPathFormat::Relative => relative_link_target(from, target),
    }
}

/// The folder-relative wikilink target from the note `from` to `target`
/// (both vault-relative): `..` steps out of the source folder, then the
/// path down to the target. A target in the source's own folder is just
/// its stem, and a root note linking into the vault spells the full
/// (relative) path.
fn relative_link_target(from: &str, target: &str) -> String {
    fn stem(p: &str) -> &str {
        p.strip_suffix(".md").unwrap_or(p)
    }
    let from_dir: Vec<&str> = from
        .rsplit_once('/')
        .map_or(Vec::new(), |(dir, _)| dir.split('/').collect());
    let to_parts: Vec<&str> = stem(target).split('/').collect();
    let (to_dirs, file) = to_parts.split_at(to_parts.len() - 1);
    let file = file[0];
    let mut common = 0;
    while common < from_dir.len() && common < to_dirs.len() && from_dir[common] == to_dirs[common] {
        common += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..from_dir.len() {
        parts.push("..".to_string());
    }
    parts.extend(to_dirs[common..].iter().map(|part| part.to_string()));
    parts.push(file.to_string());
    parts.join("/")
}

// --- auto-pairs -----------------------------------------------------------

/// The bracket/quote pairs that are typed as a pair. `None` for every
/// other character. Both members map to the same pair, so a closing
/// bracket can be recognised (and stepped over).
pub(crate) fn pair_for(typed: char) -> Option<(char, char)> {
    match typed {
        '(' | ')' => Some(('(', ')')),
        '[' | ']' => Some(('[', ']')),
        '{' | '}' => Some(('{', '}')),
        '"' => Some(('"', '"')),
        '\'' => Some(('\'', '\'')),
        '`' => Some(('`', '`')),
        _ => None,
    }
}

/// The character at a cursor position. Iced cursor columns are byte
/// offsets within the line (cosmic-text glyph indices).
fn char_at(text: &str, pos: Position) -> Option<char> {
    let line = text.split('\n').nth(pos.line)?;
    line.get(pos.column..)?.chars().next()
}

/// The character just before a cursor position.
fn char_before(text: &str, pos: Position) -> Option<char> {
    let line = text.split('\n').nth(pos.line)?;
    line.get(..pos.column)?.chars().next_back()
}

/// The byte offset of a cursor position in `text`.
///
/// Columns are byte offsets within their line (the editor's own
/// convention), so the offset is the sum of the lines before it.
fn offset_at(text: &str, pos: Position) -> usize {
    let mut offset = 0;
    for (line, text_line) in text.split('\n').enumerate() {
        if line == pos.line {
            return offset + pos.column.min(text_line.len());
        }
        offset += text_line.len() + 1;
    }
    offset
}

/// The cursor position at a byte offset in `text`.
fn position_at(text: &str, offset: usize) -> Position {
    let mut rest = offset;
    for (line, text_line) in text.split('\n').enumerate() {
        if rest <= text_line.len() {
            return Position { line, column: rest };
        }
        rest -= text_line.len() + 1;
    }
    // Past the end: the end of the last line.
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    Position {
        line: last,
        column: lines[last].len(),
    }
}

/// The text a cursor's selection covers, with its bounds in document
/// order: the start position, the end position, and the text between.
fn selected_text(text: &str, cursor: Cursor) -> Option<(Position, Position, String)> {
    let anchor = cursor.selection?;
    let (start, end) =
        if (anchor.line, anchor.column) <= (cursor.position.line, cursor.position.column) {
            (anchor, cursor.position)
        } else {
            (cursor.position, anchor)
        };
    let from = offset_at(text, start);
    let to = offset_at(text, end);
    Some((start, end, text.get(from..to)?.to_string()))
}

/// What an auto-pair keystroke does: the editor actions to perform, and
/// the cursor to restore once they have run.
///
/// A wrapped selection stays selected — the text between the markers —
/// so typing another opener pairs again: select "My Link", type "[",
/// type "[" again, and the link reads "[[My Link]]".
pub(crate) struct AutoPair {
    /// The editor actions to perform.
    pub actions: Vec<Action>,
    /// The cursor to restore after the actions ran: the caret at the
    /// end of the wrapped text, the selection still on it.
    pub select: Option<Cursor>,
}

/// The editor actions for a typed bracket/quote:
/// - a selection is wrapped in the pair, and stays selected,
/// - typing the closing character right before the same character steps
///   over it instead of duplicating it,
/// - quotes only pair at word boundaries, so an apostrophe inside a word
///   stays a plain character ("don't" must not become "don''t"),
/// - otherwise the pair is inserted with the cursor between the markers.
fn autopair_actions(text: &str, cursor: Cursor, typed: char) -> AutoPair {
    let Some((open, close)) = pair_for(typed) else {
        return AutoPair {
            actions: Vec::new(),
            select: None,
        };
    };
    let is_opener = typed == open;

    // A selection is wrapped in the pair (openers only). The wrapped
    // text stays selected: the caret lands after it, the selection
    // covers the text between the markers.
    if let Some((start, end, sel)) = selected_text(text, cursor) {
        if !sel.is_empty() {
            if !is_opener {
                // A closing bracket over a selection just replaces it.
                return AutoPair {
                    actions: vec![Action::Edit(Edit::Paste(Arc::new(typed.to_string())))],
                    select: None,
                };
            }
            let from = offset_at(text, start);
            let to = offset_at(text, end);
            let wrapped = format!("{open}{sel}{close}");
            // The inner text starts after the opener and ends before the
            // closer, in the text the paste produces.
            let inner_start = from + open.len_utf8();
            let inner_end = inner_start + sel.len();
            let mut new_text = String::with_capacity(text.len() + wrapped.len());
            new_text.push_str(&text[..from]);
            new_text.push_str(&wrapped);
            new_text.push_str(&text[to..]);
            return AutoPair {
                actions: vec![Action::Edit(Edit::Paste(Arc::new(wrapped)))],
                select: Some(Cursor {
                    position: position_at(&new_text, inner_end),
                    selection: Some(position_at(&new_text, inner_start)),
                }),
            };
        }
    }

    // Typing the closing character right before the same character steps
    // over it instead of duplicating it.
    if typed == close && char_at(text, cursor.position) == Some(close) {
        return AutoPair {
            actions: vec![Action::Move(Motion::Right)],
            select: None,
        };
    }

    // A closing bracket on its own is just the character.
    if !is_opener {
        return AutoPair {
            actions: vec![Action::Edit(Edit::Insert(typed))],
            select: None,
        };
    }

    // Quotes only pair at word boundaries, so an apostrophe inside a word
    // stays a plain character ("don't" must not become "don''t").
    if open == close && char_before(text, cursor.position).is_some_and(|c| c.is_alphanumeric()) {
        return AutoPair {
            actions: vec![Action::Edit(Edit::Insert(typed))],
            select: None,
        };
    }

    // Otherwise insert the pair with the cursor between the markers.
    AutoPair {
        actions: vec![
            Action::Edit(Edit::Paste(Arc::new(format!("{open}{close}")))),
            Action::Move(Motion::Left),
        ],
        select: None,
    }
}

/// Map an editor key press to a shortcut message, or `None` to fall
/// through (the caller then defers to the editor's default bindings).
/// Only fires while the editor is focused, so quick-switcher and global
/// shortcuts keep working elsewhere.
pub(crate) fn editor_shortcut(press: text_editor::KeyPress) -> Option<Message> {
    if !matches!(press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    let ctrl = press.modifiers.command();
    let shift = press.modifiers.shift();
    let alt = press.modifiers.alt();
    match &press.key {
        cosmic::iced::keyboard::Key::Character(c) if ctrl => {
            let lower = c.to_lowercase();
            match (lower.as_str(), shift, alt) {
                ("z", _, false) => Some(if shift {
                    Message::EditorRedo
                } else {
                    Message::EditorUndo
                }),
                ("y", false, false) => Some(Message::EditorRedo),
                ("b", _, false) => Some(Message::EditorFormat(Format::Bold)),
                ("i", _, false) => Some(Message::EditorFormat(Format::Italic)),
                ("k", _, false) => Some(Message::EditorFormat(Format::Link)),
                ("`", _, false) => Some(Message::EditorFormat(Format::Code)),
                ("x", true, false) => Some(Message::EditorFormat(Format::Strike)),
                ("h", true, false) => Some(Message::EditorFormat(Format::Highlight)),
                // Footnote lives on Ctrl+Alt+F: Ctrl+Shift+F belongs to the
                // global vault search.
                ("f", _, true) => Some(Message::EditorFootnote),
                // GTK-style Unicode codepoint entry.
                ("u", true, false) => Some(Message::UnicodeStart),
                _ => None,
            }
        }
        cosmic::iced::keyboard::Key::Named(Named::Backspace) if ctrl => {
            Some(Message::EditorDeleteWord(false))
        }
        cosmic::iced::keyboard::Key::Named(Named::Delete) if ctrl => {
            Some(Message::EditorDeleteWord(true))
        }
        cosmic::iced::keyboard::Key::Character(_) => {
            // A plainly typed bracket/quote: pair it. The produced text
            // (not the raw key) tells us which character was typed, so
            // shifted symbols work on any layout.
            let typed = press
                .text
                .as_deref()
                .and_then(|t| t.chars().find(|c| !c.is_control()))?;
            pair_for(typed)?;
            Some(Message::EditorAutoPair(typed))
        }
        _ => None,
    }
}

/// The markdown formatting shortcut the live block editor shares with
/// the source editor: Ctrl+B/I/K/`, Ctrl+Shift+X/H.
///
/// Footnote (Ctrl+Alt+F) stays source-only on purpose: its definition
/// belongs at the end of the *note*, while the live editor's buffer
/// holds one region's source.
pub(crate) fn live_format_shortcut(press: text_editor::KeyPress) -> Option<Format> {
    if !matches!(press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    if !press.modifiers.command() {
        return None;
    }
    let shift = press.modifiers.shift();
    let alt = press.modifiers.alt();
    let cosmic::iced::keyboard::Key::Character(c) = &press.key else {
        return None;
    };
    match (c.to_lowercase().as_str(), shift, alt) {
        ("b", _, false) => Some(Format::Bold),
        ("i", _, false) => Some(Format::Italic),
        ("k", _, false) => Some(Format::Link),
        ("`", _, false) => Some(Format::Code),
        ("x", true, false) => Some(Format::Strike),
        ("h", true, false) => Some(Format::Highlight),
        _ => None,
    }
}

/// While Unicode codepoint entry is open, map an editor key press:
/// typed characters feed the hex buffer (the handler keeps only hex
/// digits), Space/Enter commit, Backspace unravels, Esc cancels, and
/// Tab is ignored — GTK's behavior. Chords with Ctrl or Alt fall
/// through (`None`) so shortcuts keep working, as do keys like the
/// arrow keys (the commit inserts wherever the caret ends up).
pub(crate) fn unicode_entry_binding(
    press: &text_editor::KeyPress,
) -> Option<text_editor::Binding<Message>> {
    if press.modifiers.command() || press.modifiers.alt() {
        return None;
    }
    match &press.key {
        cosmic::iced::keyboard::Key::Character(c) => {
            let mut chars = c.chars();
            let first = chars.next()?;
            // Space arrives as a character key (there is no Named::Space
            // in this iced); like GTK, it commits the entry.
            if first == ' ' && chars.next().is_none() {
                return Some(text_editor::Binding::Custom(Message::UnicodeCommit));
            }
            (chars.next().is_none())
                .then(|| text_editor::Binding::Custom(Message::UnicodeHex(first)))
        }
        cosmic::iced::keyboard::Key::Named(named) => match named {
            Named::Enter => Some(text_editor::Binding::Custom(Message::UnicodeCommit)),
            Named::Backspace => Some(text_editor::Binding::Custom(Message::UnicodeBackspace)),
            Named::Escape => Some(text_editor::Binding::Custom(Message::UnicodeCancel)),
            Named::Tab => Some(text_editor::Binding::Custom(Message::UnicodeIgnore)),
            _ => None,
        },
        _ => None,
    }
}

/// The sidebar nav bar's model: one item per `SidePanel`, in `ALL` order,
/// Files active. Pure so tests can inspect it without a `Core`.
fn build_nav_model() -> nav_bar::Model {
    let mut model = nav_bar::Model::default();
    for panel in SidePanel::ALL {
        model
            .insert()
            .icon(icon::from_name(panel.icon()))
            .text(panel.label())
            .data(panel);
    }
    model.activate_position(0);
    model
}

/// The header pane-mode segmented control: Live / Split / Source /
/// Reading, with `active` highlighted.
///
/// The highlighted button must follow the pane that is *rendered*, which
/// at startup is the persisted default layout — not `PaneMode::default()`.
/// Highlighting the default instead left "Live" lit while the persisted
/// "Split" layout rendered.
fn build_pane_model(active: PaneMode) -> segmented_button::Model<segmented_button::SingleSelect> {
    let mut model = segmented_button::Model::default();
    for (i, (mode, label)) in [
        (PaneMode::Live, "Live"),
        (PaneMode::Split, "Split"),
        (PaneMode::Source, "Source"),
        (PaneMode::Reading, "Reading"),
    ]
    .into_iter()
    .enumerate()
    {
        model.insert().text(label).data(mode);
        if mode == active {
            model.activate_position(i as u16);
        }
    }
    model
}

/// The two parts of an open tab's label: the note's title (with a dot
/// in front while it has unsaved edits) and the folder the note sits in
/// (empty for a note at the vault root).
///
/// The tab strip shows the folder dimmed beside the title, so two notes
/// with the same name can be told apart at a glance.
pub(crate) fn tab_label_parts(tab: &Tab) -> (String, String) {
    let mut title = tab.title();
    if tab.is_dirty() {
        title = format!("• {title}");
    }
    (title, parent_of(&tab.path))
}

/// Activate the model item whose data equals `data` (nav bar / segmented
/// control). Returns whether an item matched.
fn activate_entity_with<Data: 'static + PartialEq>(
    model: &mut segmented_button::Model<segmented_button::SingleSelect>,
    data: &Data,
) -> bool {
    let target = model
        .iter()
        .find(|id| model.data::<Data>(*id) == Some(data));
    match target {
        Some(id) => {
            model.activate(id);
            true
        }
        None => false,
    }
}

/// Mirrors vault-core's dot-directory rule: reject names that would sneak a
/// write into `.obsidian/` or friends.
fn contains_hidden_component(name: &str) -> bool {
    name.split('/').any(|c| c.starts_with('.') && c != ".")
}

/// The libcosmic task that applies a stored theme choice (M4c).
///
/// Dark and light load the *system* theme configs (`system_dark()` /
/// `system_light()`), not the built-in defaults: the runner recomputes
/// window blur on theme change from the theme's `frosted_windows` flag
/// (and the accent from the theme itself), so a synthetic default theme
/// would silently turn frosted windows off.
fn theme_task(choice: settings::ThemeChoice) -> Task<Message> {
    let theme = match choice {
        settings::ThemeChoice::System => cosmic::theme::system_preference(),
        settings::ThemeChoice::Dark => cosmic::theme::system_dark(),
        settings::ThemeChoice::Light => cosmic::theme::system_light(),
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

    /// The text the user has selected, if any.
    ///
    /// The live pane edits one block region at a time, so its selection
    /// lives in the region's editor; the panes that show the source
    /// editor read it from there. The reading pane shows no editor, so
    /// nothing is selected there. An empty selection counts as none.
    pub fn selection_text(&self) -> Option<String> {
        let selection = match self.pane {
            PaneMode::Live => self.live.as_ref().and_then(|live| live.editor.selection()),
            PaneMode::Source | PaneMode::Split => {
                self.active_tab().and_then(|tab| tab.content.selection())
            }
            PaneMode::Reading => None,
        };
        selection.filter(|text| !text.is_empty())
    }

    /// The desktop window title (what the task switcher shows): the open
    /// note within the vault, the vault alone, or the app name.
    fn window_title(&self) -> String {
        match self.active_tab() {
            Some(tab) => format!("{} — {}", tab.title(), self.vault_name),
            None if !self.vault_name.is_empty() => self.vault_name.clone(),
            None => "Vault Notes".to_string(),
        }
    }

    /// Whether one of the app's own text inputs (quick switcher, inline
    /// tree rename, find bar) currently owns the keyboard.
    fn overlay_input_open(&self) -> bool {
        self.switcher.open
            || self.tree_edit.is_some()
            || self.find.is_some()
            || self.wiki_complete.is_some()
    }

    /// Push the current window title to the platform.
    fn sync_window_title(&self) -> Task<Message> {
        self.core.set_title(None, self.window_title())
    }

    /// The path being renamed inline in the sidebar tree, if any.
    pub fn tree_rename_target(&self) -> Option<&str> {
        match &self.tree_edit {
            Some(tree::TreeEdit {
                kind: tree::TreeEditKind::Rename { old },
                ..
            }) => Some(old),
            _ => None,
        }
    }

    /// The folder receiving an inline new-note/new-folder input, if any.
    pub fn tree_folder_edit_target(&self) -> Option<&str> {
        match &self.tree_edit {
            Some(tree::TreeEdit {
                kind: tree::TreeEditKind::NewNote { folder },
                ..
            }) => Some(folder),
            Some(tree::TreeEdit {
                kind: tree::TreeEditKind::NewFolder { folder },
                ..
            }) => Some(folder),
            _ => None,
        }
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

    /// Close any editor-attached overlays (wikilink dropdown, Unicode
    /// codepoint entry). Called whenever the editing context changes.
    fn dismiss_editor_overlays(&mut self) {
        self.wiki_complete = None;
        self.unicode_input = None;
    }

    /// Resolve a tree attachment path (vault-relative) to an absolute path
    /// for the platform handler; falls back to the raw path with no vault.
    fn attachment_path(&self, path: &str) -> PathBuf {
        self.vault
            .as_ref()
            .map(|vault| vault.root().join(path))
            .unwrap_or_else(|| PathBuf::from(path))
    }

    fn rebuild_index(&mut self) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        self.index = vault.index();
        // The notes carry their modification time, so the tree can order
        // them by age; the attachments have none.
        let notes: Vec<tree::Entry<'_>> = vault
            .notes_sorted()
            .iter()
            .map(|note| tree::Entry::new(&note.path, Some(note.modified)))
            .collect();
        // Empty folders materialize too, so they can receive new notes.
        let folders: Vec<String> = vault.folders();
        let folder_refs: Vec<&str> = folders.iter().map(String::as_str).collect();
        // Attachments show in the tree as well (click opens them with the
        // platform handler), filtered by the user's extension list; the
        // switcher and search stay note-only.
        let extensions = self.attachment_extensions.clone();
        let attachments: Vec<tree::Entry<'_>> = vault
            .attachments_sorted()
            .into_iter()
            .filter(|path| attachment_shown(path, &extensions))
            .map(|path| tree::Entry::new(path, None))
            .collect();
        let mut entries = notes;
        entries.extend(attachments);
        self.tree = tree::build_with_folders(&entries, &folder_refs, self.tree_sort);
    }

    fn open_tab(&mut self, path: String) -> Task<Message> {
        self.dismiss_editor_overlays();
        let was = self.active;
        if let Some(i) = self.tabs.iter().position(|tab| tab.path == path) {
            self.active = Some(i);
            if was != self.active {
                // Live-editing state belongs to the previous tab's text.
                self.live = None;
            }
            let focus = self.ensure_live_focus();
            return Task::batch([self.sync_window_title(), focus]);
        }
        let source = self
            .vault
            .as_ref()
            .and_then(|vault| vault.note_source(&path))
            .unwrap_or_default();
        self.tabs.push(Tab::open(path, &source));
        self.active = Some(self.tabs.len() - 1);
        self.live = None;
        let focus = self.ensure_live_focus();
        Task::batch([self.sync_window_title(), focus])
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
            let mut notes = vault.notes_sorted();
            if query.trim().is_empty() {
                // No query: the most recently touched notes first (the
                // ones you are working on), ties alphabetical. A plain
                // path dump buries them under whatever sorts first.
                notes.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
                self.switcher.items = notes
                    .iter()
                    .take(SWITCHER_MAX_ITEMS)
                    .map(|note| note.path.clone())
                    .collect();
            } else {
                let paths: Vec<String> = notes.iter().map(|note| note.path.clone()).collect();
                self.switcher.items = crate::fuzzy::rank_paths(&query, &paths, SWITCHER_MAX_ITEMS)
                    .into_iter()
                    .map(|ranked| paths[ranked.index].clone())
                    .collect();
            }
        } else {
            self.switcher.items.clear();
        }
        if self.switcher.selected >= self.switcher.items.len() {
            self.switcher.selected = 0;
        }
    }

    /// Switch the rendered pane mode, keeping the pane buttons in step:
    /// the highlighted button must always name the pane on screen.
    fn set_pane(&mut self, mode: PaneMode) {
        self.pane = mode;
        let target = self
            .pane_model
            .iter()
            .find(|id| self.pane_model.data::<PaneMode>(*id) == Some(&mode));
        if let Some(id) = target {
            self.pane_model.activate(id);
        }
    }

    // --- font sizes and zoom ---

    /// Set the source editor's font size (clamped) and persist it.
    fn set_editor_font_size(&mut self, size: i32) {
        let size = settings::clamp_font_size(size);
        self.editor_font_size = size;
        persist_setting(|s| s.editor_font_size = size);
    }

    /// Set the reading view's font size (clamped) and persist it.
    fn set_reading_font_size(&mut self, size: i32) {
        let size = settings::clamp_font_size(size);
        self.reading_font_size = size;
        persist_setting(|s| s.reading_font_size = size);
    }

    /// The pane the zoom shortcuts act on.
    ///
    /// The source editor and the rendered view carry independent font
    /// sizes, so the shortcut has to pick one. It follows the pane in
    /// focus: the editor while a note is being edited, the reading view
    /// while it is being read. The split pane shows both, so it follows
    /// the last one touched (`zoom_target`).
    fn zoom_target(&self) -> ZoomTarget {
        match self.pane {
            PaneMode::Reading => ZoomTarget::Reading,
            PaneMode::Source => ZoomTarget::Source,
            // In the live pane the focused block is a source editor; the
            // blocks around it are rendered.
            PaneMode::Live => {
                if self.live.is_some() {
                    ZoomTarget::Source
                } else {
                    ZoomTarget::Reading
                }
            }
            PaneMode::Split => self.zoom_target,
        }
    }

    /// Zoom the pane in focus by `delta` steps (one step = one pixel).
    fn zoom_font(&mut self, delta: i32) {
        match self.zoom_target() {
            ZoomTarget::Source => self.set_editor_font_size(self.editor_font_size as i32 + delta),
            ZoomTarget::Reading => {
                self.set_reading_font_size(self.reading_font_size as i32 + delta)
            }
        }
    }

    /// Fold the split layout away in a narrow window (and bring it back
    /// when the window grows again).
    ///
    /// Two panes side by side need more room than one: below
    /// `NARROW_WINDOW_WIDTH` the split view collapses to the source
    /// pane. The layout returns when the window is wide enough again —
    /// unless the user picked another pane in the meantime.
    fn apply_window_width(&mut self, width: f32) {
        let narrow = width < NARROW_WINDOW_WIDTH;
        if narrow && self.pane == PaneMode::Split {
            self.restore_split = true;
            self.set_pane(PaneMode::Source);
        } else if !narrow && self.restore_split && self.pane == PaneMode::Source {
            self.restore_split = false;
            self.set_pane(PaneMode::Split);
        }
    }

    /// Scroll the active tab's reading view to top-level block `idx`,
    /// switching out of source-only mode so the jump is visible.
    fn jump_to_block(&mut self, idx: usize) -> Task<Message> {
        // Live preview focuses the block instead of scrolling a pane.
        if self.pane == PaneMode::Live {
            let Some(tab_idx) = self.active else {
                return Task::none();
            };
            let region = self
                .live_slices(tab_idx)
                .iter()
                .position(|slice| slice.block == Some(idx));
            if let Some(region) = region {
                return self.live_focus(region);
            }
        }
        if self.pane == PaneMode::Source {
            self.set_pane(PaneMode::Split);
        }
        // No frame deferral needed even when the pane just switched: the
        // iced runner rebuilds all user interfaces *before* running the
        // actions returned by update, so the measure op below always sees
        // the freshly built tree (iced/winit/src/lib.rs: build_user_interfaces
        // precedes run_action!).
        crate::scroll::to_block::<Message>(idx, 12.0)
    }

    // --- live preview (`Live` pane mode) ---

    /// The live preview's slice table for a tab.
    fn live_slices(&self, tab_idx: usize) -> Vec<live::BlockSlice> {
        let tab = &self.tabs[tab_idx];
        live::slices(&tab.content.text(), &tab.doc)
    }

    /// Where the live caret is in absolute file coordinates, so undo and
    /// redo can re-aim the block editor after the text is restored.
    fn live_aim_marker(&self, tab_idx: usize) -> Option<(usize, usize)> {
        let live = self.live.as_ref()?;
        let slice = self.live_slices(tab_idx).get(live.block).copied()?;
        let cursor = live.editor.cursor().position;
        Some((slice.start_line - 1 + cursor.line, cursor.column))
    }

    /// Focus the live editor's text input once the view has rebuilt with
    /// it mounted at the focused region's (possibly new) position. The
    /// target id is per-region (`scroll::live_editor_id`), matching the
    /// id the view tags the focused editor with.
    fn live_editor_focus(region: usize) -> Task<Message> {
        cosmic::iced::runtime::task::widget::<Message>(
            cosmic::iced::core::widget::operation::focusable::focus::<Message>(
                crate::scroll::live_editor_id(region),
            ),
        )
        .map(cosmic::Action::from)
    }

    /// When the live preview is showing a note with no markdown blocks
    /// (fresh or emptied), focus its body region so the user can just
    /// type. Anything else leaves the focus alone.
    fn ensure_live_focus(&mut self) -> Task<Message> {
        if self.pane != PaneMode::Live || self.live.is_some() {
            return Task::none();
        }
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        if !self.tabs[tab_idx].doc.blocks.is_empty() {
            return Task::none();
        }
        let slices = self.live_slices(tab_idx);
        match slices.len().checked_sub(1) {
            Some(last) => self.live_focus(last),
            None => Task::none(),
        }
    }

    /// Open block region `idx` for editing in the live preview: its raw
    /// source becomes the buffer, with the caret at the end of the
    /// region's content (its trailing blank lines are the gap to the
    /// next region, not text).
    fn live_focus(&mut self, idx: usize) -> Task<Message> {
        if self.pane != PaneMode::Live {
            return Task::none();
        }
        if self.live.as_ref().is_some_and(|state| state.block == idx) {
            return Task::none();
        }
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let slices = self.live_slices(tab_idx);
        let Some(slice) = slices.get(idx).copied() else {
            return Task::none();
        };
        let (line, column) = live::content_end(&slice.source(&self.tabs[tab_idx].content.text()));
        self.live_jump_to(idx, line, column)
    }

    /// Move the live editor to region `idx` with the caret at
    /// `line`/`column` (clamped inside), no selection. Rebuilding the
    /// buffer resets the widget's focus, so a focus task rides along for
    /// the post-rebuild tree (the runner rebuilds views before running
    /// update's actions).
    fn live_jump_to(&mut self, idx: usize, line: usize, column: usize) -> Task<Message> {
        self.live_jump_to_caret(idx, line, column, None)
    }

    /// The same jump with an optional selection anchor (region-local
    /// line/column), for find matches that arrive already selected.
    fn live_jump_to_caret(
        &mut self,
        idx: usize,
        line: usize,
        column: usize,
        selection: Option<Position>,
    ) -> Task<Message> {
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let slices = self.live_slices(tab_idx);
        let Some(slice) = slices.get(idx).copied() else {
            return Task::none();
        };
        let tab = &self.tabs[tab_idx];
        let source = slice.source(&tab.content.text());
        let (line, column) = live::clamp_position(&source, line, column);
        let selection = selection.map(|s| {
            let (s_line, s_column) = live::clamp_position(&source, s.line, s.column);
            Position {
                line: s_line,
                column: s_column,
            }
        });
        let mut editor = Content::with_text(&source);
        editor.move_to(Cursor {
            position: Position { line, column },
            selection,
        });
        self.live = Some(LiveEdit { block: idx, editor });
        // The runner rebuilds views before running these actions, so the
        // measure op sees the region mounted in the fresh tree (the same
        // ordering the reading pane's `to_block` relies on).
        let scroll = crate::scroll::to_live_region::<Message>(idx, 12.0);
        let focus = Self::live_editor_focus(idx);
        Task::batch([scroll, focus])
    }

    /// A batch of editor actions on the live editor's buffer. Like
    /// `run_editor_command` for the source editor, but splices the
    /// buffer into the note text once at the end (the region's slice
    /// range is captured before anything moves the boundaries), then
    /// re-aims the buffer at the caret and re-evaluates the wiki
    /// dropdown against the region text.
    fn live_run(&mut self, actions: Vec<Action>) -> Task<Message> {
        self.live_run_with(actions, None)
    }

    /// As `live_run`, but restoring a cursor in the region's buffer
    /// afterwards (the auto-pair keeps a wrapped selection selected).
    ///
    /// The restore happens before the re-aim: when the edit changed the
    /// region's shape, the buffer is rebuilt and the selection is
    /// dropped with it.
    fn live_run_with(&mut self, actions: Vec<Action>, select: Option<Cursor>) -> Task<Message> {
        if actions.is_empty() {
            return Task::none();
        }
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let Some(live_block) = self.live.as_ref().map(|live| live.block) else {
            return Task::none();
        };
        let range = {
            let slice = self.live_slices(tab_idx).get(live_block).copied();
            match slice {
                Some(slice) => (slice.start_line, slice.end_line),
                None => return Task::none(),
            }
        };
        let is_edit = actions.iter().any(|action| action.is_edit());
        // Snapshot before the edit so undo can restore it: the whole
        // document stays the undo unit across regions.
        if is_edit {
            push_undo(&mut self.tabs[tab_idx]);
        }
        if let Some(live) = self.live.as_mut() {
            for action in actions {
                live.editor.perform(action);
            }
            // The auto-pair wraps a selection and keeps it selected: the
            // caret moves back inside the markers, the selection covers
            // the wrapped text.
            if let Some(cursor) = select {
                live.editor.move_to(cursor);
            }
        }
        if is_edit {
            let source = self
                .live
                .as_ref()
                .map_or_else(String::new, |live| live.editor.text());
            let tab = &mut self.tabs[tab_idx];
            let new_text = live::replace_lines(&tab.content.text(), range.0, range.1, &source);
            tab.content = Content::with_text(&new_text);
            tab.refresh_doc();
            // Any editor activity resets the idle timer: the user is
            // still working here.
            self.last_edit = Some(std::time::Instant::now());
        }
        let (line, column) = self
            .live
            .as_ref()
            .map(|live| {
                let cursor = live.editor.cursor().position;
                (range.0 - 1 + cursor.line, cursor.column)
            })
            .unwrap_or((0, 0));
        let aim_task = self.live_aim(line, column);
        // A region edit can open, move through, or close an unclosed
        // `[[` in the buffer.
        self.update_wiki_complete();
        aim_task
    }

    /// One action on the live editor: perform it on the focused region's
    /// buffer, and for edits splice the buffer back into the note text
    /// and re-aim at the region now containing the caret (a split or a
    /// text growth can move region boundaries).
    fn live_action(&mut self, action: Action) -> Task<Message> {
        self.live_run(vec![action])
    }

    /// Perform one edit action on the live editor.
    fn live_edit(&mut self, edit: Edit) -> Task<Message> {
        self.live_action(Action::Edit(edit))
    }

    /// Re-aim the live editor at the region containing `line` (0-based
    /// file line): re-slice the note text, and when the buffer no longer
    /// matches its region's source (the edit split or merged blocks),
    /// rebuild the buffer with the caret clamped inside. Returns a focus
    /// task when the buffer is rebuilt so the editor keeps focus across
    /// region boundary changes.
    fn live_aim(&mut self, line: usize, column: usize) -> Task<Message> {
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let tab = &self.tabs[tab_idx];
        let text = tab.content.text();
        let slices = live::slices(&text, &tab.doc);
        if slices.is_empty() {
            return Task::none();
        }
        let idx = live::block_containing(&slices, line);
        let source = slices[idx].source(&text);
        let Some(live) = self.live.as_mut() else {
            return Task::none();
        };
        if live.editor.text() == source {
            // Content identical (the common case: the region's source
            // simply re-tiled): keep the buffer, keep the caret.
            live.block = idx;
            return Task::none();
        }
        let (line, column) = live::clamp_position(
            &source,
            line.saturating_sub(slices[idx].start_line - 1),
            column,
        );
        live.editor = Content::with_text(&source);
        live.editor.move_to(Cursor {
            position: Position { line, column },
            selection: None,
        });
        live.block = idx;
        // Buffer rebuilt: request focus on the new region's editor id.
        Self::live_editor_focus(idx)
    }

    /// Arrow keys in the live editor. At a region's boundary they cross
    /// into the neighboring region (the caret lands in the matching
    /// place, with its column preserved where the target line is long
    /// enough); inside a region they are plain motions.
    fn live_navigate(&mut self, up: bool) -> Task<Message> {
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let motion = if up { Motion::Up } else { Motion::Down };
        let Some(live_block) = self.live.as_ref().map(|live| live.block) else {
            return Task::none();
        };
        let Some(cursor) = self.live.as_ref().map(|live| live.editor.cursor()) else {
            return Task::none();
        };
        // A selection stays inside the region: the motion collapses it.
        if cursor.selection.is_some() {
            if let Some(live) = self.live.as_mut() {
                live.editor.perform(Action::Move(motion));
            }
            return Task::none();
        }
        let slices = self.live_slices(tab_idx);
        if live_block >= slices.len() {
            return Task::none();
        }
        let text = self.tabs[tab_idx].content.text();
        // The editor's own motion is visual-line aware: it walks the
        // wrapped rows of a long line before it would leave the region.
        // Only a motion that cannot move the caret (the caret is on the
        // region's first or last visual row) crosses into the neighbor.
        let before = cursor.position;
        if let Some(live) = self.live.as_mut() {
            live.editor.perform(Action::Move(motion));
        }
        let after = self.live.as_ref().map(|live| live.editor.cursor().position);
        if after != Some(before) {
            // The caret moved inside the region — possibly from one
            // wrapped row of a long line to the next.
            return Task::none();
        }
        if up {
            if live_block == 0 {
                return Task::none();
            }
            let prev = slices[live_block - 1];
            let prev_source = prev.source(&text);
            let (line, _) = live::content_end(&prev_source);
            self.live_jump_to(live_block - 1, line, cursor.position.column)
        } else if live_block + 1 < slices.len() {
            self.live_jump_to(live_block + 1, 0, cursor.position.column)
        } else {
            Task::none()
        }
    }

    /// Backspace in the live editor. At a region's start it deletes the
    /// newline before it — one character at file level — anywhere else
    /// it is a plain backspace.
    fn live_backspace(&mut self) -> Task<Message> {
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let at_start = self.live.as_ref().is_some_and(|live| {
            let cursor = live.editor.cursor();
            cursor.selection.is_none() && cursor.position.line == 0 && cursor.position.column == 0
        });
        if !at_start {
            return self.live_edit(Edit::Backspace);
        }
        let Some(live_block) = self.live.as_ref().map(|live| live.block) else {
            return Task::none();
        };
        if live_block == 0 {
            return Task::none();
        }
        let slices = self.live_slices(tab_idx);
        let Some(current) = slices.get(live_block).copied() else {
            return Task::none();
        };
        let text = self.tabs[tab_idx].content.text();
        let Some(new_text) = live::delete_newline_before(&text, current.start_line) else {
            return Task::none();
        };
        let tab = &mut self.tabs[tab_idx];
        push_undo(tab);
        tab.content = Content::with_text(&new_text);
        tab.refresh_doc();
        self.last_edit = Some(std::time::Instant::now());
        // One newline before the caret is gone: the caret absolute file
        // line moves up by one while its column stays at the line start.
        self.live_aim(current.start_line - 2, 0)
    }

    /// Delete in the live editor. At a region's end it deletes the
    /// newline after it — one character at file level — anywhere else it
    /// is a plain delete.
    fn live_delete(&mut self) -> Task<Message> {
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let Some(live_block) = self.live.as_ref().map(|live| live.block) else {
            return Task::none();
        };
        let Some(cursor) = self.live.as_ref().map(|live| live.editor.cursor()) else {
            return Task::none();
        };
        let at_end = {
            let source = self
                .live_slices(tab_idx)
                .get(live_block)
                .map(|slice| slice.source(&self.tabs[tab_idx].content.text()))
                .unwrap_or_default();
            let (line, column) = live::clamp_position(&source, usize::MAX, usize::MAX);
            cursor.selection.is_none()
                && cursor.position.line == line
                && cursor.position.column == column
        };
        if !at_end {
            return self.live_edit(Edit::Delete);
        }
        let slices = self.live_slices(tab_idx);
        let Some(current) = slices.get(live_block).copied() else {
            return Task::none();
        };
        let (line, column) = (cursor.position.line, cursor.position.column);
        let text = self.tabs[tab_idx].content.text();
        // Caret at the region's last line end: the next character is the
        // newline joining it to whatever line follows the region.
        let Some(new_text) = live::delete_newline_before(&text, current.end_line + 1) else {
            return Task::none();
        };
        let tab = &mut self.tabs[tab_idx];
        push_undo(tab);
        tab.content = Content::with_text(&new_text);
        tab.refresh_doc();
        self.last_edit = Some(std::time::Instant::now());
        // The newline is deleted after the caret: its absolute file line
        // and column are unchanged, and the region still starts here.
        self.live_aim(current.start_line - 1 + line, column)
    }

    /// Ctrl+Backspace / Ctrl+Delete in the live editor: delete the word
    /// before (or after) the caret, the way the source editor does.
    ///
    /// Inside a region the editor's own word motion does the work. At a
    /// region's edge the word can live in the neighbouring region, so
    /// the motion runs over the whole note text instead.
    fn live_delete_word(&mut self, right: bool) -> Task<Message> {
        let Some(tab_idx) = self.active else {
            return Task::none();
        };
        let Some(live_block) = self.live.as_ref().map(|live| live.block) else {
            return Task::none();
        };
        let Some(cursor) = self.live.as_ref().map(|live| live.editor.cursor()) else {
            return Task::none();
        };
        // A selection is deleted as it is.
        if cursor.selection.is_some() {
            return self.live_run(word_delete_actions(true, right));
        }
        let slices = self.live_slices(tab_idx);
        let Some(current) = slices.get(live_block).copied() else {
            return Task::none();
        };
        let text = self.tabs[tab_idx].content.text();
        let source = current.source(&text);
        let (line, column) = (cursor.position.line, cursor.position.column);
        // The region's last line and its end: the caret there can only
        // reach the next region at file level. The column is a byte
        // offset, so the line's end is measured in bytes too.
        let lines: Vec<&str> = source.split('\n').collect();
        let last_line = lines.len() - 1;
        let last_column = lines[last_line].len();
        let at_edge = if right {
            line == last_line && column >= last_column
        } else {
            line == 0 && column == 0
        };
        if !at_edge {
            return self.live_run(word_delete_actions(false, right));
        }
        // File level: the word motion runs over the whole note, so the
        // word can come from the neighbouring region.
        let offset = offset_of(&text, current.start_line - 1 + line, column);
        let word = if right {
            word_after(&text, offset)
        } else {
            word_before(&text, offset)
        };
        let Some((start, end)) = word else {
            return Task::none();
        };
        let mut new_text = String::with_capacity(text.len());
        new_text.push_str(&text[..start]);
        new_text.push_str(&text[end..]);
        let tab = &mut self.tabs[tab_idx];
        push_undo(tab);
        tab.content = Content::with_text(&new_text);
        tab.refresh_doc();
        self.last_edit = Some(std::time::Instant::now());
        // The caret lands where the deleted word started.
        let (line, column) = position_of(&new_text, start);
        let aim_task = self.live_aim(line, column);
        // A word can carry an unclosed `[[` with it.
        self.update_wiki_complete();
        aim_task
    }

    /// Scroll-sync (Phase F): in Split mode, follow the editor caret — the
    /// reading pane scrolls to the top-level block containing the caret's
    /// line. Fires only when the caret changes line, and never in Source
    /// mode (no preview) or when the setting is off. The reverse direction
    /// (preview click → caret) is not possible: iced's editor has no
    /// programmatic scroll API.
    fn sync_preview_scroll(&mut self) -> Task<Message> {
        if !self.scroll_sync || self.pane != PaneMode::Split {
            return Task::none();
        }
        let Some(idx) = self.active else {
            return Task::none();
        };
        // The editor buffer is the whole file; its 0-based cursor line + 1
        // is the 1-based FILE line the spans use.
        let line = self.tabs[idx].content.cursor().position.line + 1;
        if self.tabs[idx].synced_line == Some(line) {
            return Task::none();
        }
        let block = self.tabs[idx].doc.block_at_line(line);
        self.tabs[idx].synced_line = Some(line);
        match block {
            Some(block) => crate::scroll::to_block::<Message>(block, 12.0),
            None => Task::none(),
        }
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
        self.dismiss_editor_overlays();
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
        // The removed tab may have been the one being live-edited.
        self.live = None;
        let title = self.sync_window_title();
        let focus = self.ensure_live_focus();
        Task::batch([title, focus])
    }

    /// Activate the nav-bar item for `panel` (after a programmatic switch).
    fn activate_nav(&mut self, panel: SidePanel) {
        activate_entity_with(&mut self.nav_model, &panel);
    }

    /// Perform a multi-action editor command (formatting, word delete) as
    /// one undoable step, then refresh derived state.
    fn run_editor_command(&mut self, idx: usize, actions: Vec<Action>) {
        self.run_editor_command_with(idx, actions, None);
    }

    /// As `run_editor_command`, but restoring a cursor afterwards (the
    /// auto-pair keeps a wrapped selection selected).
    fn run_editor_command_with(
        &mut self,
        idx: usize,
        actions: Vec<Action>,
        select: Option<Cursor>,
    ) {
        if actions.is_empty() {
            return;
        }
        {
            let tab = &mut self.tabs[idx];
            push_undo(tab);
            for action in actions {
                tab.content.perform(action);
            }
            if let Some(cursor) = select {
                tab.content.move_to(cursor);
            }
            tab.refresh_doc();
        }
        self.last_edit = Some(std::time::Instant::now());
        // A command can move the cursor into or out of an unclosed `[[`.
        self.update_wiki_complete();
    }

    /// Insert a footnote reference at the cursor and its definition at
    /// the end of the note (Ctrl+Alt+F), as one undoable step.
    ///
    /// A selection is replaced by the reference; the definitions are
    /// normalized (one blank line before each) on the way.
    fn insert_footnote_at_cursor(&mut self, idx: usize) {
        {
            let tab = &mut self.tabs[idx];
            push_undo(tab);
            // The reference replaces the selection, as it did when this
            // was a list of editor actions.
            if tab.content.selection().is_some() {
                tab.content.perform(Action::Edit(Edit::Delete));
            }
            let text = tab.content.text();
            let cursor = tab.content.cursor().position;
            let n = next_footnote_number(&text);
            let (new_text, cursor) = insert_footnote(&text, cursor, n);
            tab.content = Content::with_text(&new_text);
            tab.content.move_to(Cursor {
                position: cursor,
                selection: None,
            });
            tab.refresh_doc();
        }
        self.last_edit = Some(std::time::Instant::now());
        self.update_wiki_complete();
    }

    /// Select find match `index` in the active tab (cursor at its end, the
    /// editor's selection highlighting it).
    fn select_match(&mut self, index: usize) -> Task<Message> {
        let (query, case) = match self.find.as_ref() {
            Some(find) => (find.query.clone(), find.case_sensitive),
            None => return Task::none(),
        };
        let Some(idx) = self.active else {
            return Task::none();
        };
        let text = self.tabs[idx].content.text();
        let matches = find_matches(&text, &query, case);
        let Some(found) = matches.get(index).copied() else {
            return Task::none();
        };
        if self.pane == PaneMode::Live {
            // The caret and the selection live in the focused region's
            // buffer: open the region holding the match and select it
            // there (region-local coordinates).
            let slices = self.live_slices(idx);
            let region = live::block_containing(&slices, found.line);
            let start = slices[region].start_line;
            let relative_line = found.line - (start - 1);
            let task = self.live_jump_to_caret(
                region,
                relative_line,
                found.end,
                Some(Position {
                    line: relative_line,
                    column: found.start,
                }),
            );
            if let Some(find) = self.find.as_mut() {
                find.current = index;
            }
            return task;
        }
        self.tabs[idx].content.move_to(Cursor {
            position: Position {
                line: found.line,
                column: found.end,
            },
            selection: Some(Position {
                line: found.line,
                column: found.start,
            }),
        });
        if let Some(find) = self.find.as_mut() {
            find.current = index;
        }
        Task::none()
    }

    /// Step to the next (or previous) match, wrapping around.
    fn find_step(&mut self, forward: bool) -> Task<Message> {
        let (query, case, current) = match self.find.as_ref() {
            Some(find) => (find.query.clone(), find.case_sensitive, find.current),
            None => return Task::none(),
        };
        let Some(tab) = self.active_tab() else {
            return Task::none();
        };
        let text = tab.content.text();
        let len = find_matches(&text, &query, case).len();
        if len == 0 {
            return Task::none();
        }
        self.select_match(next_index(len, current, forward))
    }

    /// Jump the find selection to the first match at or after the editor's
    /// current selection anchor (or cursor). In live preview the anchor is
    /// the focused region's caret lifted to file coordinates.
    fn find_jump_to_cursor(&mut self) -> Task<Message> {
        let anchor = if self.pane == PaneMode::Live {
            self.live.as_ref().and_then(|live| {
                let idx = self.active?;
                let slices = self.live_slices(idx);
                let slice = slices.get(live.block)?;
                let cursor = live.editor.cursor();
                let pos = cursor.selection.unwrap_or(cursor.position);
                Some(Position {
                    line: slice.start_line - 1 + pos.line,
                    column: pos.column,
                })
            })
        } else {
            self.active_tab().map(|tab| {
                let cursor = tab.content.cursor();
                cursor.selection.unwrap_or(cursor.position)
            })
        };
        let Some(anchor) = anchor else {
            return Task::none();
        };
        let (query, case) = match self.find.as_ref() {
            Some(find) => (find.query.clone(), find.case_sensitive),
            None => return Task::none(),
        };
        let index = self
            .active_tab()
            .map(|tab| {
                let text = tab.content.text();
                first_at_or_after(
                    &find_matches(&text, &query, case),
                    anchor.line,
                    anchor.column,
                )
            })
            .unwrap_or(0);
        self.select_match(index)
    }

    /// Replace find match `index` in the active tab (one undoable step),
    /// then point the selection at the following match.
    fn find_replace_one(&mut self, index: usize) -> Task<Message> {
        let (query, case, replacement) = match self.find.as_ref() {
            Some(find) => (
                find.query.clone(),
                find.case_sensitive,
                find.replacement.clone(),
            ),
            None => return Task::none(),
        };
        let Some(idx) = self.active else {
            return Task::none();
        };
        let text = self.tabs[idx].content.text();
        let matches = find_matches(&text, &query, case);
        let Some(found) = matches.get(index).copied() else {
            return Task::none();
        };
        // Where the live caret is, so the buffer can re-aim after the
        // whole-text replacement (the focused region may have changed).
        let aim = self.live_aim_marker(idx);
        {
            let tab = &mut self.tabs[idx];
            push_undo(tab);
            let new_text = replace_spans(&text, &[found], &replacement);
            tab.content = Content::with_text(&new_text);
            if self.pane != PaneMode::Live {
                tab.content.move_to(Cursor {
                    position: Position {
                        line: found.line,
                        column: found.start + replacement.len(),
                    },
                    selection: None,
                });
            }
            tab.refresh_doc();
        }
        self.last_edit = Some(std::time::Instant::now());
        let aim_task = if let Some((line, column)) = aim {
            self.live_aim(line, column)
        } else {
            Task::none()
        };
        // The next match (if any) now sits at the same index.
        let text = self.tabs[idx].content.text();
        let len = find_matches(&text, &query, case).len();
        if let Some(find) = self.find.as_mut() {
            find.current = len.min(index);
        }
        let task = self.select_match(self.find.as_ref().map_or(0, |f| f.current));
        self.update_wiki_complete();
        Task::batch([aim_task, task])
    }

    /// Replace every match in the active tab (one undoable step).
    fn find_replace_all(&mut self) -> Task<Message> {
        let (query, case, replacement) = match self.find.as_ref() {
            Some(find) => (
                find.query.clone(),
                find.case_sensitive,
                find.replacement.clone(),
            ),
            None => return Task::none(),
        };
        if query.is_empty() {
            return Task::none();
        }
        let Some(idx) = self.active else {
            return Task::none();
        };
        let text = self.tabs[idx].content.text();
        let matches = find_matches(&text, &query, case);
        if matches.is_empty() {
            return Task::none();
        }
        // Same live-buffer rule as the single replace: re-aim after the
        // whole-text splice.
        let aim = self.live_aim_marker(idx);
        {
            let tab = &mut self.tabs[idx];
            push_undo(tab);
            let new_text = replace_spans(&text, &matches, &replacement);
            tab.content = Content::with_text(&new_text);
            tab.refresh_doc();
        }
        self.last_edit = Some(std::time::Instant::now());
        let aim_task = if let Some((line, column)) = aim {
            self.live_aim(line, column)
        } else {
            Task::none()
        };
        if let Some(find) = self.find.as_mut() {
            find.current = 0;
        }
        self.update_wiki_complete();
        aim_task
    }

    /// Re-evaluate the wikilink dropdown against the active tab's text and
    /// cursor: open while an unclosed `[[` is being typed, gone otherwise.
    /// The hovered drop target of an active tree drag, if any: a folder
    /// path, or "" for the vault root.
    pub fn tree_drag_hover(&self) -> Option<&str> {
        self.tree_drag
            .as_ref()
            .filter(|drag| drag.active)
            .and_then(|drag| drag.hover.as_deref())
    }

    /// The path of the row an active tree drag is carrying, if any.
    pub fn tree_drag_source(&self) -> Option<&str> {
        self.tree_drag
            .as_ref()
            .filter(|drag| drag.active)
            .and_then(|drag| drag.source.as_ref().map(|(path, _)| path.as_str()))
    }

    /// Advance the tree-drag state machine on cursor movement (drag.rs).
    fn advance_tree_drag(&mut self, pos: cosmic::iced::Point) {
        if self.tree_drag.is_none() {
            return;
        }
        if self.tree_drag.as_ref().is_some_and(|drag| drag.active) {
            let target = self.tree_bounds.hover_target(pos);
            if let Some(drag) = self.tree_drag.as_mut() {
                drag.hover = target;
            }
            return;
        }
        // Still arming: identify the source row from the press position.
        let source = match self.tree_drag.as_ref().and_then(|drag| drag.source.clone()) {
            Some(source) => Some(source),
            None => self
                .tree_drag
                .as_ref()
                .and_then(|drag| self.tree_bounds.row_at(drag.press_at)),
        };
        match source {
            // Press was not on a (draggable) tree row: not a tree drag.
            // Fresh bounds may still be in flight (e.g. the tree was just
            // scrolled), so wait for them before concluding the press
            // missed every row.
            None => {
                if !self.bounds_pending {
                    self.tree_drag = None;
                }
            }
            Some(source) => {
                let active = self
                    .tree_drag
                    .as_ref()
                    .is_some_and(|drag| drag.press_at.distance(pos) > TREE_DRAG_THRESHOLD);
                let hover = active.then(|| self.tree_bounds.hover_target(pos)).flatten();
                if let Some(drag) = self.tree_drag.as_mut() {
                    drag.source = Some(source);
                    drag.active = active;
                    drag.hover = hover;
                }
            }
        }
    }

    /// A drag was released over `drag.hover`: move the source there via
    /// the ordinary rename flow (which raises the link-update prompt).
    fn finish_tree_drag(&mut self, drag: TreeDrag) -> Task<Message> {
        let Some((source, is_folder)) = drag.source else {
            return Task::none();
        };
        // Releasing over the tree but not on a target (or outside the
        // tree entirely) cancels.
        let Some(target) = drag.hover else {
            return Task::none();
        };
        // Releasing over the source row itself is a click, not a move —
        // the row's button performs its normal action.
        let released_over_source = self.last_cursor.is_some_and(|pos| {
            self.tree_bounds
                .rows
                .iter()
                .any(|(path, _, bounds)| *path == source && bounds.contains(pos))
        });
        if released_over_source {
            return Task::none();
        }
        let Some(new_path) = crate::drag::drop_path(&source, is_folder, &target) else {
            return Task::none();
        };
        if is_folder {
            self.rename_tree_folder(source, new_path)
        } else {
            self.rename_tree_note(source, new_path)
        }
    }

    /// Scroll the file tree so the current note's row sits in view.
    ///
    /// The row only exists once the folders on its path are open, so
    /// this runs after the fold state changed: the measuring task waits
    /// for the next layout pass.
    fn scroll_tree_to_current_note(&self) -> Task<Message> {
        match self.active_tab_path() {
            Some(path) => crate::scroll::to_tree_note::<Message>(path, 4.0),
            None => Task::none(),
        }
    }

    /// Queue a bounds-measure task unless one is already in flight.
    fn measure_tree_bounds_if_needed(&mut self) -> Task<Message> {
        // Only measure while the tree is on screen: on other panels (or
        // with the sidebar collapsed) the tree is not built, so the
        // measurement would come back empty and wipe the bounds a later
        // press needs.
        if self.bounds_pending
            || self.vault.is_none()
            || self.panel != SidePanel::Files
            || !self.sidebar_open
        {
            return Task::none();
        }
        self.bounds_pending = true;
        crate::drag::measure_tree_bounds()
            .map(Message::TreeBounds)
            .map(cosmic::Action::App)
    }

    fn update_wiki_complete(&mut self) {
        if !self.wiki_autocomplete {
            self.dismiss_editor_overlays();
            return;
        }
        let Some(idx) = self.active else {
            self.dismiss_editor_overlays();
            return;
        };
        // Live preview: the caret and the text live in the focused
        // region's buffer, not the whole-file one.
        let (text, cursor) = if self.pane == PaneMode::Live {
            match (
                self.live.as_ref().map(|live| live.editor.text()),
                self.live.as_ref().map(|live| live.editor.cursor().position),
            ) {
                (Some(text), Some(cursor)) => (text, cursor),
                _ => {
                    self.dismiss_editor_overlays();
                    return;
                }
            }
        } else {
            let text = self.tabs[idx].content.text();
            let cursor = self.tabs[idx].content.cursor().position;
            (text, cursor)
        };
        let Some((_, query)) = wiki_query_at(&text, cursor) else {
            self.dismiss_editor_overlays();
            return;
        };
        let paths: Vec<String> = self
            .vault
            .as_ref()
            .map(|vault| {
                vault
                    .notes_sorted()
                    .iter()
                    .map(|note| note.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let items = wiki_paths(&query, &paths);
        if items.is_empty() {
            self.dismiss_editor_overlays();
        } else {
            let selected = self
                .wiki_complete
                .as_ref()
                .map_or(0, |wiki| wiki.selected.min(items.len() - 1));
            self.wiki_complete = Some(WikiComplete { items, selected });
        }
    }

    /// Accept the highlighted wikilink suggestion: replace the partial
    /// `[[query` up to the cursor with the target link (one undoable
    /// step). The link text follows the link-path-format preference. A
    /// custom label that follows the caret is kept. In live preview the
    /// edit lands in the focused region's buffer and lifts into the note
    /// in the same batch.
    fn wiki_accept(&mut self) -> Task<Message> {
        let Some(target) = self
            .wiki_complete
            .as_ref()
            .and_then(|wiki| wiki.items.get(wiki.selected).cloned())
        else {
            return Task::none();
        };
        let Some(idx) = self.active else {
            return Task::none();
        };
        // The link text follows the link-path-format preference.
        let link = {
            let from = self.tabs[idx].path.clone();
            let notes: Vec<String> = self
                .vault
                .as_ref()
                .map(|vault| {
                    vault
                        .notes_sorted()
                        .iter()
                        .map(|note| note.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            link_target(&from, &target, &notes, self.link_path_format)
        };
        if self.pane == PaneMode::Live {
            // Edit the region's buffer: select the partial `[[query`
            // (region-local coordinates) and paste the link, then lift
            // the buffer into the note in one batch.
            if self.live.is_none() {
                self.dismiss_editor_overlays();
                return Task::none();
            }
            let (text, cursor) = {
                let live = self.live.as_ref().expect("live block without state");
                (live.editor.text(), live.editor.cursor().position)
            };
            let Some(replacement) = wiki_replacement(&text, cursor, &link) else {
                self.dismiss_editor_overlays();
                return Task::none();
            };
            if let Some(live) = self.live.as_mut() {
                live.editor.move_to(Cursor {
                    position: Position {
                        line: cursor.line,
                        column: replacement.end,
                    },
                    selection: Some(Position {
                        line: cursor.line,
                        column: replacement.open,
                    }),
                });
            }
            // The caret lands right after the bar when a label was kept.
            let caret = replacement.caret.map(|column| Cursor {
                position: Position {
                    line: cursor.line,
                    column,
                },
                selection: None,
            });
            let task = self.live_run_with(
                vec![Action::Edit(Edit::Paste(Arc::new(replacement.text)))],
                caret,
            );
            self.dismiss_editor_overlays();
            return task;
        }
        let text = self.tabs[idx].content.text();
        let cursor = self.tabs[idx].content.cursor().position;
        let Some(replacement) = wiki_replacement(&text, cursor, &link) else {
            self.dismiss_editor_overlays();
            return Task::none();
        };
        {
            let tab = &mut self.tabs[idx];
            push_undo(tab);
            tab.content.move_to(Cursor {
                position: Position {
                    line: cursor.line,
                    column: replacement.end,
                },
                selection: Some(Position {
                    line: cursor.line,
                    column: replacement.open,
                }),
            });
            tab.content
                .perform(Action::Edit(Edit::Paste(Arc::new(replacement.text))));
            if let Some(column) = replacement.caret {
                tab.content.move_to(Cursor {
                    position: Position {
                        line: cursor.line,
                        column,
                    },
                    selection: None,
                });
            }
            tab.refresh_doc();
        }
        self.last_edit = Some(std::time::Instant::now());
        self.dismiss_editor_overlays();
        Task::none()
    }

    /// Move the dropdown highlight (wrapping).
    fn wiki_cycle(&mut self, delta: i32) {
        let Some(wiki) = self.wiki_complete.as_mut() else {
            return;
        };
        let len = wiki.items.len();
        if len == 0 {
            return;
        }
        let selected = wiki.selected as i64 + delta as i64;
        wiki.selected = (selected.rem_euclid(len as i64)) as usize;
    }
    /// A tree context-menu command, applied to the node the menu was opened
    /// on (`Message::TreeMenuTarget` recorded it when the menu opened).
    fn tree_menu_command(&mut self, action: TreeMenuAction) -> Task<Message> {
        let Some(target) = self.tree_menu_target.take() else {
            return Task::none();
        };
        match action {
            TreeMenuAction::NewNote | TreeMenuAction::NewFolder => {
                // New entries land beside the target: inside a folder, or
                // in a note's parent folder.
                let folder = if target.ends_with(".md") {
                    target
                        .rsplit_once('/')
                        .map(|(parent, _)| parent.to_string())
                        .unwrap_or_default()
                } else {
                    target
                };
                let kind = if action == TreeMenuAction::NewNote {
                    tree::TreeEditKind::NewNote {
                        folder: folder.clone(),
                    }
                } else {
                    tree::TreeEditKind::NewFolder {
                        folder: folder.clone(),
                    }
                };
                // Reveal the folder so the inline input is visible.
                self.collapsed.remove(&folder);
                self.tree_edit = Some(tree::TreeEdit {
                    kind,
                    value: String::new(),
                });
                cosmic::widget::text_input::focus::<Message>(Id::new(TREE_EDIT_INPUT))
                    .map(cosmic::Action::App)
            }
            TreeMenuAction::Rename => {
                let value = target.rsplit('/').next().unwrap_or(&target).to_string();
                self.tree_edit = Some(tree::TreeEdit {
                    kind: tree::TreeEditKind::Rename { old: target },
                    value,
                });
                cosmic::widget::text_input::focus::<Message>(Id::new(TREE_EDIT_INPUT))
                    .map(cosmic::Action::App)
            }
            TreeMenuAction::Delete => self.delete_tree_path(target),
        }
    }

    /// Apply the pending inline tree edit. All writes go through vault-core's
    /// guarded paths; failures surface in the error line.
    fn submit_tree_edit(&mut self) -> Task<Message> {
        let Some(edit) = self.tree_edit.take() else {
            return Task::none();
        };
        let name = edit.value.trim().trim_matches('/').to_string();
        if name.is_empty() || contains_hidden_component(&name) {
            self.error = Some(format!("invalid name: {:?}", edit.value));
            return Task::none();
        }
        // Computed before the match: the match moves out of `edit`.
        let new = edit.target_path();
        match edit.kind {
            tree::TreeEditKind::NewNote { folder } => self.create_tree_note(&folder, &name),
            tree::TreeEditKind::NewFolder { folder } => {
                let path = if folder.is_empty() {
                    name.clone()
                } else {
                    format!("{folder}/{name}")
                };
                let outcome = self.vault.as_mut().map(|vault| vault.create_folder(&path));
                match outcome {
                    Some(Ok(())) => self.rebuild_index(),
                    Some(Err(error)) => {
                        self.error = Some(format!("cannot create folder {path}: {error}"))
                    }
                    None => {}
                }
                Task::none()
            }
            tree::TreeEditKind::Rename { old } => {
                if old.ends_with(".md") {
                    let new = if new.ends_with(".md") {
                        new
                    } else {
                        format!("{new}.md")
                    };
                    self.rename_tree_note(old, new)
                } else {
                    self.rename_tree_folder(old, new)
                }
            }
        }
    }

    fn create_tree_note(&mut self, folder: &str, name: &str) -> Task<Message> {
        let path = if folder.is_empty() {
            format!("{name}.md")
        } else {
            format!("{folder}/{name}.md")
        };
        // A name that already resolves: open it instead of duplicating.
        if self
            .vault
            .as_ref()
            .is_some_and(|vault| vault.note_source(&path).is_some())
        {
            return self.open_tab(path);
        }
        // A name may itself contain folders ("a/b") — create them first.
        if let Some((parent, _)) = path.rsplit_once('/') {
            if parent != folder {
                let outcome = self.vault.as_mut().map(|vault| vault.create_folder(parent));
                if let Some(Err(error)) = outcome {
                    self.error = Some(format!("cannot create folder {parent}: {error}"));
                    return Task::none();
                }
            }
        }
        let outcome = self
            .vault
            .as_mut()
            .map(|vault| vault.save_note(&path, "", ""));
        match outcome {
            Some(Ok(_)) => {
                self.rebuild_index();
                self.open_tab(path)
            }
            Some(Err(error)) => {
                self.error = Some(format!("cannot create {path}: {error}"));
                Task::none()
            }
            None => Task::none(),
        }
    }

    fn rename_tree_note(&mut self, old: String, new: String) -> Task<Message> {
        // Snapshot BEFORE the rename: the backlink set and the resolution
        // index both refer to the old name.
        let pending = self.collect_link_updates(&[(old.clone(), new.clone())]);
        let outcome = self
            .vault
            .as_mut()
            .map(|vault| vault.rename_note(&old, &new));
        match outcome {
            Some(Ok(())) => {
                for tab in &mut self.tabs {
                    if tab.path == old {
                        tab.path = new.clone();
                    }
                }
                self.rebuild_index();
                self.pending_links = pending;
            }
            Some(Err(error)) => self.error = Some(format!("cannot rename {old}: {error}")),
            None => {}
        }
        Task::none()
    }

    fn rename_tree_folder(&mut self, old: String, new: String) -> Task<Message> {
        // Snapshot BEFORE the rename (see rename_tree_note).
        let pending = self.collect_link_updates(&[(old.clone(), new.clone())]);
        let outcome = self
            .vault
            .as_mut()
            .map(|vault| vault.rename_folder(&old, &new));
        match outcome {
            Some(Ok(())) => {
                let prefix = format!("{old}/");
                for tab in &mut self.tabs {
                    if let Some(rest) = tab.path.strip_prefix(&prefix) {
                        tab.path = format!("{new}/{rest}");
                    }
                }
                self.collapsed = self
                    .collapsed
                    .iter()
                    .map(|p| match p.strip_prefix(&prefix) {
                        Some(rest) => format!("{new}/{rest}"),
                        None => p.clone(),
                    })
                    .collect();
                self.rebuild_index();
                self.pending_links = pending;
            }
            Some(Err(error)) => self.error = Some(format!("cannot rename {old}: {error}")),
            None => {}
        }
        Task::none()
    }

    /// Notes containing wikilinks/embeds that resolve (pre-rename) into
    /// the scope moved by `renames` — a note-rename pair or a folder-rename
    /// pair. Referrers whose links would not change (markdown-only links,
    /// nothing resolving into scope) are filtered out, so a pointless
    /// prompt never appears. `None` when there is nothing to rewrite.
    fn collect_link_updates(&self, renames: &[(String, String)]) -> Option<PendingLinkUpdates> {
        let vault = self.vault.as_ref()?;
        let pre_index = vault.index();
        // In scope: a renamed path itself, or anything beneath a renamed
        // folder (the prefix case never matches for a `.md` note rename).
        let in_scope = |p: &str| {
            renames
                .iter()
                .any(|(old, _)| p == old || p.starts_with(&format!("{old}/")))
        };
        // Backlink keys cover notes AND attachments (embed targets).
        let mut targets: Vec<String> = vault
            .notes_sorted()
            .into_iter()
            .map(|note| note.path.clone())
            .collect();
        targets.extend(vault.attachments_sorted().iter().map(|s| (*s).to_string()));
        let mut candidates: Vec<String> = Vec::new();
        for target in &targets {
            if in_scope(target) {
                candidates.extend(vault.backlinks(target));
            }
        }
        candidates.sort();
        candidates.dedup();
        // Keep only referrers with at least one wikilink/embed that
        // resolves into the renamed scope: the rewriter only touches
        // those, so the prompt should only mention them.
        let mut referrers = Vec::new();
        for old_ref in candidates {
            let text = self.note_text(&old_ref);
            let hit = vault_md::link_spans(&text).into_iter().any(|span| {
                !span.raw.is_empty() && pre_index.resolve(&span.raw, &old_ref).is_some_and(in_scope)
            });
            if hit {
                referrers.push(old_ref);
            }
        }
        if referrers.is_empty() {
            return None;
        }
        Some(PendingLinkUpdates {
            renames: renames.to_vec(),
            referrers,
            pre_index,
        })
    }

    /// The current text of a note: the open tab's buffer if one exists
    /// (the live truth for an open note), else the file on disk.
    fn note_text(&self, path: &str) -> String {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) {
            return tab.content.text();
        }
        self.vault
            .as_ref()
            .and_then(|vault| std::fs::read_to_string(vault.root().join(path)).ok())
            .unwrap_or_default()
    }

    /// Rewrite the wikilink/embed targets that the rename/move
    /// invalidated (confirmed via the banner). Each referrer is re-scanned
    /// NOW: only a link whose resolution actually changed is rewritten,
    /// and the replacement is formatted per the link-path setting.
    fn apply_link_updates(&mut self) {
        let Some(pending) = self.pending_links.take() else {
            return;
        };
        let format = self.link_path_format;
        let (pre_index, post_index, post_notes) = {
            let Some(vault) = self.vault.as_ref() else {
                return;
            };
            (
                pending.pre_index,
                vault.index(),
                vault
                    .notes_sorted()
                    .into_iter()
                    .map(|note| note.path.clone())
                    .collect::<Vec<String>>(),
            )
        };
        let new_path_of = |p: &str| -> Option<String> {
            for (old, new) in &pending.renames {
                if p == old {
                    return Some(new.clone());
                }
                if let Some(rest) = p.strip_prefix(&format!("{old}/")) {
                    return Some(format!("{new}/{rest}"));
                }
            }
            None
        };
        // Phase 1 (immutable): compute the rewritten text for every
        // referrer, reading buffers/disk without holding the vault mutably.
        enum Target {
            Tab(usize),
            File(String, String), // path, original disk text
        }
        let mut rewrites: Vec<(Target, String)> = Vec::new();
        for old_ref in &pending.referrers {
            // The referrer itself may have moved with the rename.
            let from = new_path_of(old_ref).unwrap_or_else(|| old_ref.clone());
            let rewrite = |text: &str| -> Option<String> {
                vault_md::rewrite_link_targets(text, |span| {
                    if span.raw.is_empty() {
                        return None;
                    }
                    // Where the link pointed before the rename...
                    let pre = pre_index.resolve(&span.raw, &from)?;
                    // ...only links into the renamed scope are affected.
                    let target = new_path_of(pre)?;
                    // Still resolves to the same note? Leave it alone.
                    if post_index.resolve(&span.raw, &from) == Some(target.as_str()) {
                        return None;
                    }
                    Some(link_target(&from, &target, &post_notes, format))
                })
            };
            if let Some(idx) = self.tabs.iter().position(|tab| tab.path == from) {
                let text = self.tabs[idx].content.text();
                if let Some(new_text) = rewrite(&text) {
                    rewrites.push((Target::Tab(idx), new_text));
                }
            } else {
                let Some(text) = self
                    .vault
                    .as_ref()
                    .and_then(|vault| std::fs::read_to_string(vault.root().join(&from)).ok())
                else {
                    continue;
                };
                if let Some(new_text) = rewrite(&text) {
                    rewrites.push((Target::File(from.clone(), text), new_text));
                }
            }
        }
        // Phase 2 (mutable): apply.
        for (target, new_text) in rewrites {
            match target {
                Target::Tab(idx) => {
                    let tab = &mut self.tabs[idx];
                    push_undo(tab);
                    tab.content = Content::with_text(&new_text);
                    tab.refresh_doc();
                }
                Target::File(path, original) => {
                    if let Some(vault) = self.vault.as_mut() {
                        if let Err(error) = vault.save_note_full(&path, &original, &new_text) {
                            self.error = Some(format!("could not update links in {path}: {error}"));
                        }
                    }
                }
            }
        }
        self.rebuild_index();
        self.update_wiki_complete();
    }

    /// Delete a note or folder from the tree: files move into the vault's
    /// `.trash/` (recoverable, dot-ignored). Unsaved edits are saved first
    /// so nothing is silently lost; a refused save aborts the deletion.
    fn delete_tree_path(&mut self, path: String) -> Task<Message> {
        let is_note = path.ends_with(".md");
        let affected =
            |p: &str| (is_note && p == path) || (!is_note && p.starts_with(&format!("{path}/")));
        let dirty: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| affected(&tab.path) && tab.is_dirty())
            .map(|(idx, _)| idx)
            .collect();
        for idx in dirty {
            if !self.save_tab(idx) {
                // Conflict banner raised; the tab (and file) stay untouched.
                return Task::none();
            }
        }
        let outcome = {
            let Some(vault) = self.vault.as_mut() else {
                return Task::none();
            };
            if is_note {
                vault.delete_note(&path).map(|_| ())
            } else {
                vault.delete_folder(&path).map(|_| ())
            }
        };
        match outcome {
            Ok(()) => {
                let active_path = self
                    .active
                    .and_then(|idx| self.tabs.get(idx))
                    .map(|tab| tab.path.clone());
                self.tabs.retain(|tab| !affected(&tab.path));
                self.active = active_path
                    .and_then(|ap| self.tabs.iter().position(|tab| tab.path == ap))
                    .or(if self.tabs.is_empty() {
                        None
                    } else {
                        Some(self.tabs.len() - 1)
                    });
                self.rebuild_index();
            }
            Err(error) => self.error = Some(format!("cannot delete {path}: {error}")),
        }
        Task::none()
    }

    /// Poll the watcher: re-index externally changed paths, auto-refresh
    /// clean tabs, and flag dirty ones in the banner (M4 decision 7).
    /// Save the active tab if it was edited and has been idle for the
    /// configured delay. Goes through the guarded save path, so an external
    /// change refuses the write and raises the conflict banner instead of
    /// silently clobbering.
    fn autosave_tick(&mut self) -> Task<Message> {
        let idle = self.last_edit.map(|t| t.elapsed());
        if autosave_due(idle, self.autosave_secs)
            && self.active.is_some_and(|idx| self.tabs[idx].is_dirty())
        {
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
                        tab.undo.clear();
                        tab.redo.clear();
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
        // One settings read feeds every persisted field.
        let persisted = settings::load();
        let mut app = Self {
            core,
            vault: None,
            index: VaultIndex::new(Vec::<String>::new()),
            vault_name: String::new(),
            tree: Vec::new(),
            collapsed: HashSet::new(),
            tabs: Vec::new(),
            active: None,
            pane: persisted.default_pane,
            banner: None,
            pending_links: None,
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
            hide_md_extension: persisted.hide_md_extension,
            scroll_sync: persisted.scroll_sync,
            auto_pair: persisted.auto_pair,
            wiki_autocomplete: persisted.wiki_autocomplete,
            word_wrap: persisted.word_wrap,
            editor_font_mono: persisted.editor_font_mono,
            editor_font_family: persisted.editor_font_family.clone(),
            editor_font_size: persisted.editor_font_size,
            reading_font_family: persisted.reading_font_family.clone(),
            reading_font_size: persisted.reading_font_size,
            reading_margin: persisted.reading_margin,
            zoom_target: ZoomTarget::default(),
            modifiers: Modifiers::default(),
            restore_split: false,
            sidebar_open: persisted.sidebar_open,
            tree_drag: None,
            last_cursor: None,
            tree_bounds: crate::drag::TreeBounds::default(),
            bounds_pending: false,
            autosave_secs: persisted.autosave_secs,
            default_pane: persisted.default_pane,
            attachment_extensions: persisted.attachment_extensions,
            link_path_format: persisted.link_path_format,
            tree_sort: persisted.tree_sort,
            attachment_input: String::new(),
            unicode_input: None,
            tree_edit: None,
            tree_menu_target: None,
            find: None,
            wiki_complete: None,
            live: None,
            nav_model: build_nav_model(),
            pane_model: build_pane_model(persisted.default_pane),
        };
        app.core.set_header_title("Vault Notes".to_string());

        // Restore preferences (M4c): theme choice, then the glass request,
        // then the last-used vault (M4 decision 1) when it still exists.
        app.theme = persisted.theme;
        app.glass_style = crate::glass::plan(persisted.glass_enabled);

        let mut tasks = Vec::new();
        tasks.push(theme_task(persisted.theme));
        if app.glass_style == vault_glass::GlassStyle::Blur {
            // Ask the compositor for the frosted window; visibility still
            // depends on the theme's frosted-windows setting.
            tasks.push(Task::done(cosmic::Action::Cosmic(
                cosmic::app::Action::BlurEnabled,
            )));
        }
        tasks.push(
            persisted
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
            Message::VaultOpen(path) => {
                let opened = self.open_vault(path);
                Task::batch([opened, self.sync_window_title()])
            }
            Message::WatchTick => {
                let autosave = self.autosave_tick();
                let poll = self.poll_watcher();
                // Keep tree-row bounds fresh for drag hit-testing.
                let bounds = self.measure_tree_bounds_if_needed();
                Task::batch([autosave, poll, bounds])
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
                        // The buffer was replaced wholesale; undo history
                        // against the old text is meaningless now.
                        tab.undo.clear();
                        tab.redo.clear();
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
            Message::LinkUpdatesApply => {
                self.apply_link_updates();
                Task::none()
            }
            Message::LinkUpdatesDismiss => {
                self.pending_links = None;
                Task::none()
            }
            Message::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                persist_setting(|s| s.sidebar_open = self.sidebar_open);
                Task::none()
            }
            Message::MouseMoved(pos) => {
                self.last_cursor = Some(pos);
                self.advance_tree_drag(pos);
                self.measure_tree_bounds_if_needed()
            }
            Message::MouseWheel(delta) => {
                // Ctrl+scroll zooms the pane in focus, like a browser.
                // Without Ctrl the wheel belongs to the scrollable under
                // the pointer, which handles it on its own.
                if self.modifiers.control() && delta != 0.0 {
                    self.zoom_font(if delta > 0.0 { 1 } else { -1 });
                }
                Task::none()
            }
            Message::WindowResized(width) => {
                self.apply_window_width(width);
                Task::none()
            }
            Message::ReadingClicked => {
                // The reading view has the focus now: the zoom shortcuts
                // act on its font size.
                self.zoom_target = ZoomTarget::Reading;
                Task::none()
            }
            Message::MousePressed => {
                // Arm a possible tree drag at the cursor. Only when the
                // tree is actually on screen: other panels have no rows,
                // a collapsed sidebar hides the tree, and stale bounds
                // would misidentify sources.
                if self.vault.is_none() || self.panel != SidePanel::Files || !self.sidebar_open {
                    return Task::none();
                }
                let Some(pos) = self.last_cursor else {
                    return Task::none();
                };
                if !self.tree_bounds.area.is_some_and(|a| a.contains(pos)) {
                    return Task::none();
                }
                self.tree_drag = Some(TreeDrag {
                    press_at: pos,
                    source: None,
                    active: false,
                    hover: None,
                });
                self.measure_tree_bounds_if_needed()
            }
            Message::MouseReleased => {
                if let Some(drag) = self.tree_drag.take() {
                    if drag.active {
                        return self.finish_tree_drag(drag);
                    }
                }
                Task::none()
            }
            Message::TreeBounds(bounds) => {
                self.tree_bounds = bounds;
                self.bounds_pending = false;
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
            Message::TreeMenuTarget(target) => {
                self.tree_menu_target = target;
                Task::none()
            }
            Message::TreeMenu(action) => self.tree_menu_command(action),
            Message::TreeEditInput(value) => {
                if let Some(edit) = &mut self.tree_edit {
                    edit.value = value;
                }
                Task::none()
            }
            Message::TreeEditCancel => {
                self.tree_edit = None;
                Task::none()
            }
            Message::TreeEditSubmit => self.submit_tree_edit(),
            Message::Surface(action) => cosmic::task::message(cosmic::Action::Surface(action)),
            Message::NavSelect(id) => {
                // The widget only publishes; the model's active state is ours.
                self.nav_model.activate(id);
                if let Some(panel) = self.nav_model.data::<SidePanel>(id).copied() {
                    self.panel = panel;
                }
                self.dismiss_editor_overlays();
                // Switching back to Files re-measures the (possibly
                // re-laid-out) tree rows.
                self.measure_tree_bounds_if_needed()
            }
            Message::PaneActivated(id) => {
                self.pane_model.activate(id);
                if let Some(mode) = self.pane_model.data::<PaneMode>(id).copied() {
                    // The user picked a pane: the narrow-window fold
                    // must not undo that choice later.
                    self.restore_split = false;
                    self.set_pane(mode);
                }
                // Live-editing state belongs to the pane; leaving it (or
                // entering it fresh) drops the open region.
                self.live = None;
                self.dismiss_editor_overlays();
                self.ensure_live_focus()
            }
            Message::TabActivated(idx) => {
                let was = self.active;
                if idx < self.tabs.len() {
                    self.active = Some(idx);
                }
                // Live-editing state belongs to the previous tab's text.
                if was != self.active {
                    self.live = None;
                }
                self.dismiss_editor_overlays();
                let title = self.sync_window_title();
                let scroll = self.sync_preview_scroll();
                let focus = self.ensure_live_focus();
                Task::batch([title, scroll, focus])
            }
            Message::TabCloseRequested(idx) => self.close_tab(idx),
            Message::EditorAction(action) => {
                // The source editor has the focus: the zoom shortcuts act
                // on its font size.
                self.zoom_target = ZoomTarget::Source;
                // In live preview the action edits the focused region's
                // buffer and splices back into the note (live_action).
                if self.pane == PaneMode::Live {
                    return self.live_action(action);
                }
                if let Some(idx) = self.active {
                    let is_edit = action.is_edit();
                    let tab = &mut self.tabs[idx];
                    // Snapshot before the edit so undo can restore it.
                    if is_edit {
                        push_undo(tab);
                    }
                    tab.content.perform(action);
                    if is_edit {
                        tab.refresh_doc();
                        // Any editor activity (edits and cursor moves alike)
                        // resets the idle timer: the user is still working
                        // here.
                        self.last_edit = Some(std::time::Instant::now());
                    }
                }
                // Every editor interaction may open, move through, or close
                // an unclosed `[[`.
                self.update_wiki_complete();
                self.sync_preview_scroll()
            }
            Message::LiveFocus(idx) => self.live_focus(idx),
            Message::LiveUp => self.live_navigate(true),
            Message::LiveDown => self.live_navigate(false),
            Message::LiveBackspace => self.live_backspace(),
            Message::LiveDelete => self.live_delete(),
            Message::LiveDeleteWord(right) => self.live_delete_word(right),
            Message::LiveDismiss => {
                self.live = None;
                Task::none()
            }
            Message::LiveFormat(format) => {
                // The live editor has its own buffer: the selection and
                // the markers belong to the focused region, and the
                // splice back into the note is `live_run`'s job.
                let Some(live) = self.live.as_ref() else {
                    return Task::none();
                };
                let actions = format_actions(live.editor.selection(), format);
                self.live_run(actions)
            }
            Message::EditorUndo => {
                if let Some(idx) = self.active {
                    // Where the live caret is, so it can re-aim after the
                    // text is restored.
                    let aim = self.live_aim_marker(idx);
                    let tab = &mut self.tabs[idx];
                    if undo_tab(tab) {
                        tab.refresh_doc();
                        self.last_edit = Some(std::time::Instant::now());
                    }
                    let aim_task = if let Some((line, column)) = aim {
                        self.live_aim(line, column)
                    } else {
                        Task::none()
                    };
                    self.update_wiki_complete();
                    Task::batch([aim_task, self.sync_preview_scroll()])
                } else {
                    self.update_wiki_complete();
                    self.sync_preview_scroll()
                }
            }
            Message::EditorRedo => {
                if let Some(idx) = self.active {
                    let aim = self.live_aim_marker(idx);
                    let tab = &mut self.tabs[idx];
                    if redo_tab(tab) {
                        tab.refresh_doc();
                        self.last_edit = Some(std::time::Instant::now());
                    }
                    let aim_task = if let Some((line, column)) = aim {
                        self.live_aim(line, column)
                    } else {
                        Task::none()
                    };
                    self.update_wiki_complete();
                    Task::batch([aim_task, self.sync_preview_scroll()])
                } else {
                    self.update_wiki_complete();
                    self.sync_preview_scroll()
                }
            }
            Message::EditorFormat(format) => {
                if let Some(idx) = self.active {
                    let selection = self.tabs[idx].content.selection();
                    let actions = format_actions(selection, format);
                    self.run_editor_command(idx, actions);
                }
                self.sync_preview_scroll()
            }
            Message::EditorFootnote => {
                if let Some(idx) = self.active {
                    self.insert_footnote_at_cursor(idx);
                }
                self.sync_preview_scroll()
            }
            Message::EditorDeleteWord(right) => {
                if let Some(idx) = self.active {
                    let has_selection = self.tabs[idx].content.selection().is_some();
                    let actions = word_delete_actions(has_selection, right);
                    self.run_editor_command(idx, actions);
                }
                self.sync_preview_scroll()
            }
            Message::EditorAutoPair(typed) => {
                if !self.auto_pair {
                    // Pairing is off, but the keystroke must still land:
                    // the binding replaced the widget's own insert.
                    let actions = vec![Action::Edit(Edit::Insert(typed))];
                    if self.pane == PaneMode::Live {
                        return self.live_run(actions);
                    }
                    if let Some(idx) = self.active {
                        self.run_editor_command(idx, actions);
                    }
                    return Task::none();
                }
                if self.pane == PaneMode::Live {
                    // The buffer is the region source; the karat is its
                    // caret. The same decision table applies.
                    let (text, cursor) = {
                        let live = self.live.as_ref().expect("live-open auto pair");
                        (live.editor.text(), live.editor.cursor())
                    };
                    let pair = autopair_actions(&text, cursor, typed);
                    if pair.actions.is_empty() {
                        return self.live_run(vec![Action::Edit(Edit::Insert(typed))]);
                    }
                    return self.live_run_with(pair.actions, pair.select);
                }
                if let Some(idx) = self.active {
                    let text = self.tabs[idx].content.text();
                    let cursor = self.tabs[idx].content.cursor();
                    let pair = autopair_actions(&text, cursor, typed);
                    self.run_editor_command_with(idx, pair.actions, pair.select);
                }
                self.sync_preview_scroll()
            }
            Message::FindOpen => {
                let mut find = self.find.take().unwrap_or_default();
                // Prefill from a single-line selection.
                if let Some(sel) = self.active_tab().and_then(|tab| tab.content.selection()) {
                    if !sel.is_empty() && !sel.contains('\n') {
                        find.query = sel;
                    }
                }
                self.find = Some(find);
                let jump = self.find_jump_to_cursor();
                let focus = cosmic::widget::text_input::focus::<Message>(Id::new(FIND_INPUT))
                    .map(cosmic::Action::App);
                Task::batch([jump, focus])
            }
            Message::FindInput(value) => {
                if let Some(find) = self.find.as_mut() {
                    find.query = value;
                    find.current = 0;
                }
                let jump = self.find_jump_to_cursor();
                let sync = self.sync_preview_scroll();
                Task::batch([jump, sync])
            }
            Message::FindClose => {
                self.find = None;
                Task::none()
            }
            Message::FindNext => {
                let step = self.find_step(true);
                let sync = self.sync_preview_scroll();
                Task::batch([step, sync])
            }
            Message::FindPrev => {
                let step = self.find_step(false);
                let sync = self.sync_preview_scroll();
                Task::batch([step, sync])
            }
            Message::FindToggleCase => {
                if let Some(find) = self.find.as_mut() {
                    find.case_sensitive = !find.case_sensitive;
                    find.current = 0;
                }
                self.find_jump_to_cursor()
            }
            Message::FindReplaceInput(value) => {
                if let Some(find) = self.find.as_mut() {
                    find.replacement = value;
                }
                Task::none()
            }
            Message::FindReplace => {
                let current = self.find.as_ref().map_or(0, |f| f.current);
                let replace = self.find_replace_one(current);
                let sync = self.sync_preview_scroll();
                Task::batch([replace, sync])
            }
            Message::FindReplaceAll => {
                let replace = self.find_replace_all();
                let sync = self.sync_preview_scroll();
                Task::batch([replace, sync])
            }
            Message::WikiAccept => {
                let accept = self.wiki_accept();
                let sync = self.sync_preview_scroll();
                Task::batch([accept, sync])
            }
            Message::WikiSelect(index) => {
                if let Some(wiki) = self.wiki_complete.as_mut() {
                    wiki.selected = index;
                }
                let accept = self.wiki_accept();
                let sync = self.sync_preview_scroll();
                Task::batch([accept, sync])
            }
            Message::WikiCycle(delta) => {
                self.wiki_cycle(delta);
                Task::none()
            }
            Message::WikiDismiss => {
                self.dismiss_editor_overlays();
                Task::none()
            }
            Message::EditorCopy => {
                match self.active_tab().and_then(|tab| tab.content.selection()) {
                    Some(selection) => cosmic::iced::clipboard::write(selection),
                    None => Task::none(),
                }
            }
            Message::EditorCut => {
                let selection = self.active_tab().and_then(|tab| tab.content.selection());
                match (self.active, selection) {
                    (Some(idx), Some(selection)) => {
                        {
                            let tab = &mut self.tabs[idx];
                            push_undo(tab);
                            tab.content.perform(Action::Edit(Edit::Delete));
                            tab.refresh_doc();
                        }
                        self.last_edit = Some(std::time::Instant::now());
                        self.update_wiki_complete();
                        let copied = cosmic::iced::clipboard::write(selection);
                        let scroll = self.sync_preview_scroll();
                        Task::batch([copied, scroll])
                    }
                    _ => Task::none(),
                }
            }
            Message::EditorPasteAsk => cosmic::iced::clipboard::read()
                .map(Message::EditorPasteText)
                .map(cosmic::Action::App),
            Message::EditorPasteText(maybe_text) => {
                if let Some(text) = maybe_text {
                    if let Some(idx) = self.active {
                        {
                            let tab = &mut self.tabs[idx];
                            push_undo(tab);
                            tab.content
                                .perform(Action::Edit(Edit::Paste(Arc::new(text))));
                            tab.refresh_doc();
                        }
                        self.last_edit = Some(std::time::Instant::now());
                        self.update_wiki_complete();
                    }
                }
                self.sync_preview_scroll()
            }
            Message::EditorSelectAll => {
                if let Some(idx) = self.active {
                    self.tabs[idx].content.perform(Action::SelectAll);
                }
                self.sync_preview_scroll()
            }
            Message::SettingsToggle => {
                let open = self.core.window.show_context;
                self.core.set_show_context(!open);
                Task::none()
            }
            Message::SettingsAutoPair(on) => {
                self.auto_pair = on;
                persist_setting(|s| s.auto_pair = on);
                Task::none()
            }
            Message::SettingsWikiComplete(on) => {
                self.wiki_autocomplete = on;
                persist_setting(|s| s.wiki_autocomplete = on);
                if !on {
                    self.dismiss_editor_overlays();
                }
                Task::none()
            }
            Message::SettingsWordWrap(on) => {
                self.word_wrap = on;
                persist_setting(|s| s.word_wrap = on);
                Task::none()
            }
            Message::SettingsFontMono(on) => {
                self.editor_font_mono = on;
                persist_setting(|s| s.editor_font_mono = on);
                Task::none()
            }
            Message::SettingsEditorFontFamily(family) => {
                self.editor_font_family = family;
                persist_setting(|s| s.editor_font_family = self.editor_font_family.clone());
                Task::none()
            }
            Message::SettingsEditorFontSize(delta) => {
                self.set_editor_font_size(self.editor_font_size as i32 + delta);
                Task::none()
            }
            Message::SettingsReadingFontFamily(family) => {
                self.reading_font_family = family;
                persist_setting(|s| s.reading_font_family = self.reading_font_family.clone());
                Task::none()
            }
            Message::SettingsReadingFontSize(delta) => {
                self.set_reading_font_size(self.reading_font_size as i32 + delta);
                Task::none()
            }
            Message::SettingsReadingMargin(margin) => {
                self.reading_margin = margin;
                persist_setting(|s| s.reading_margin = margin);
                Task::none()
            }
            Message::ZoomIn => {
                self.zoom_font(1);
                Task::none()
            }
            Message::ZoomOut => {
                self.zoom_font(-1);
                Task::none()
            }
            Message::SettingsTheme(choice) => {
                self.theme = choice;
                persist_setting(|s| s.theme = choice);
                theme_task(choice)
            }
            Message::SettingsDefaultPane(mode) => {
                self.default_pane = mode;
                persist_setting(|s| s.default_pane = mode);
                Task::none()
            }
            Message::SettingsHideMdExt(on) => {
                self.hide_md_extension = on;
                persist_setting(|s| s.hide_md_extension = on);
                Task::none()
            }
            Message::SettingsAutosaveDelta(delta) => {
                let secs = (self.autosave_secs as i64 + delta as i64).clamp(1, 60) as u64;
                self.autosave_secs = secs;
                persist_setting(|s| s.autosave_secs = secs);
                Task::none()
            }
            Message::SettingsScrollSync(on) => {
                self.scroll_sync = on;
                persist_setting(|s| s.scroll_sync = on);
                Task::none()
            }
            Message::AttachmentInput(value) => {
                self.attachment_input = value;
                Task::none()
            }
            Message::AttachmentAdd => {
                if let Some(ext) = normalize_extension(&self.attachment_input) {
                    if !self.attachment_extensions.contains(&ext) {
                        self.attachment_extensions.push(ext);
                        self.attachment_extensions.sort();
                        let extensions = self.attachment_extensions.clone();
                        persist_setting(move |s| s.attachment_extensions = extensions);
                        self.rebuild_index();
                    }
                }
                self.attachment_input.clear();
                Task::none()
            }
            Message::AttachmentRemove(ext) => {
                self.attachment_extensions.retain(|e| *e != ext);
                let extensions = self.attachment_extensions.clone();
                persist_setting(move |s| s.attachment_extensions = extensions);
                self.rebuild_index();
                Task::none()
            }
            Message::SettingsLinkFormat(format) => {
                self.link_path_format = format;
                persist_setting(move |s| s.link_path_format = format);
                Task::none()
            }
            Message::UnicodeStart => {
                if self.active.is_some() {
                    self.dismiss_editor_overlays();
                    self.unicode_input = Some(String::new());
                }
                Task::none()
            }
            Message::UnicodeHex(typed) => {
                if let Some(hex) = self.unicode_input.as_mut() {
                    if typed.is_ascii_hexdigit() && hex.chars().count() < UNICODE_MAX_DIGITS {
                        hex.push(typed.to_ascii_lowercase());
                    }
                }
                Task::none()
            }
            Message::UnicodeBackspace => {
                let mut emptied = false;
                if let Some(hex) = self.unicode_input.as_mut() {
                    hex.pop();
                    emptied = hex.is_empty();
                }
                // Backspacing the empty "u" exits the mode, like GTK.
                if emptied {
                    self.unicode_input = None;
                }
                Task::none()
            }
            Message::UnicodeCommit => {
                let hex = self.unicode_input.take();
                if let (Some(idx), Some(hex)) = (self.active, hex) {
                    if let Some(ch) = unicode_char(&hex) {
                        self.run_editor_command(idx, vec![Action::Edit(Edit::Insert(ch))]);
                        return self.sync_preview_scroll();
                    }
                    // Invalid codepoint (surrogate, > U+10FFFF): the
                    // mode simply ends, leaving the buffer untouched.
                }
                Task::none()
            }
            Message::UnicodeIgnore => Task::none(),
            Message::UnicodeCancel => {
                self.unicode_input = None;
                Task::none()
            }
            Message::Save => self.save_active(),
            Message::ToggleFolder(path) => {
                if !self.collapsed.remove(&path) {
                    self.collapsed.insert(path);
                }
                // Row set changed: re-measure for drag hit-testing.
                self.measure_tree_bounds_if_needed()
            }
            Message::TreeSort(sort) => {
                self.tree_sort = sort;
                persist_setting(move |s| s.tree_sort = sort);
                self.rebuild_index();
                self.measure_tree_bounds_if_needed()
            }
            Message::TreeCollapseAll => {
                // Every folder folds away: the tree shows its roots.
                self.collapsed = tree::folder_paths(&self.tree).into_iter().collect();
                self.measure_tree_bounds_if_needed()
            }
            Message::TreeExpandAll => {
                self.collapsed.clear();
                self.measure_tree_bounds_if_needed()
            }
            Message::TreeCollapseOthers => {
                // The current note's own folders stay open; every other
                // folder folds away.
                let keep: HashSet<String> = self
                    .active_tab_path()
                    .map(tree::ancestor_paths)
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                self.collapsed = tree::folder_paths(&self.tree)
                    .into_iter()
                    .filter(|path| !keep.contains(path))
                    .collect();
                // The tree just changed shape: bring the note it is now
                // about into view.
                let scroll = self.scroll_tree_to_current_note();
                Task::batch([scroll, self.measure_tree_bounds_if_needed()])
            }
            Message::TreeRevealCurrent => {
                // Open the path to the current note, if it is folded. The
                // other folders keep whatever state they were in.
                let folders: Vec<String> = self
                    .active_tab_path()
                    .map(tree::ancestor_paths)
                    .unwrap_or_default();
                for folder in folders {
                    self.collapsed.remove(&folder);
                }
                let scroll = self.scroll_tree_to_current_note();
                Task::batch([scroll, self.measure_tree_bounds_if_needed()])
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
                self.activate_nav(panel);
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
                        vault_core::search::search(&root, &query, SEARCH_MAX_HITS)
                            .unwrap_or_default()
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
                    self.set_pane(PaneMode::Split);
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
            Message::KeyPressed(status, modifiers, key) => {
                // Remember the modifiers for the wheel: Ctrl+scroll zooms
                // the text, and the wheel event itself carries no
                // modifiers. Every key event reports the modifiers held
                // at that moment, including the Ctrl press itself.
                self.modifiers = modifiers;
                // Editor-scoped bindings (and any focused widget) capture
                // their events; a captured key must not also fire a global
                // shortcut. Our own text inputs (switcher, tree rename) are
                // the exception: their keys are exactly what we route here.
                if !global_key_allowed(status, self.overlay_input_open()) {
                    return Task::none();
                }
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
                } else if self.tree_edit.is_some() {
                    match key {
                        Key::Named(Named::Escape) => self.update(Message::TreeEditCancel),
                        _ => Task::none(),
                    }
                } else if self.find.is_some() {
                    match key {
                        Key::Named(Named::Enter) if modifiers.shift() => {
                            self.update(Message::FindPrev)
                        }
                        Key::Named(Named::Enter) => self.update(Message::FindNext),
                        Key::Named(Named::Escape) => self.update(Message::FindClose),
                        _ => Task::none(),
                    }
                } else if self.wiki_complete.is_some() {
                    // The editor's own key_binding already intercepts
                    // these (arrows walk the suggestions, Esc dismisses
                    // them), so events here are ones the editor ignored
                    // — e.g. the editor not focused.
                    match key {
                        Key::Named(Named::ArrowUp) if modifiers == Modifiers::default() => {
                            self.update(Message::WikiCycle(-1))
                        }
                        Key::Named(Named::ArrowDown) if modifiers == Modifiers::default() => {
                            self.update(Message::WikiCycle(1))
                        }
                        Key::Named(Named::Tab) if modifiers.control() => {
                            let step: i32 = if modifiers.shift() { -1 } else { 1 };
                            self.update(Message::WikiCycle(step))
                        }
                        Key::Named(Named::Tab) => self.update(Message::WikiAccept),
                        Key::Named(Named::Escape) => self.update(Message::WikiDismiss),
                        _ => Task::none(),
                    }
                } else if self.core.window.show_context {
                    // Nothing else claimed Esc, so it closes the settings
                    // drawer. (Keys captured by the drawer's own text
                    // inputs never reach this global handler.)
                    match key {
                        Key::Named(Named::Escape) => self.update(Message::SettingsToggle),
                        _ => Task::none(),
                    }
                } else if modifiers.control() {
                    match &key {
                        Key::Character(c) if c == "p" => self.update(Message::SwitcherToggle),
                        Key::Character(c) if c == "s" => self.save_active(),
                        Key::Character(c) if c == "d" => self.update(Message::DailyNote),
                        // Ctrl+F = find in the current note; Ctrl+Shift+F =
                        // the vault-wide search panel. Shifted keys arrive
                        // as either case depending on the layout, so both
                        // spellings route to the search panel.
                        Key::Character(c) if c == "f" && modifiers.shift() => {
                            self.update(Message::SidePanel(SidePanel::Search))
                        }
                        Key::Character(c) if c == "F" => {
                            self.update(Message::SidePanel(SidePanel::Search))
                        }
                        Key::Character(c) if c == "f" => self.update(Message::FindOpen),
                        Key::Character(c) if c == "w" => match self.active {
                            Some(idx) => self.update(Message::TabClose(idx)),
                            None => Task::none(),
                        },
                        // Ctrl+= / Ctrl+- zoom the pane in focus. The keys
                        // arrive unmodified ("=" is the base key, "+" is
                        // its shifted form), so both spellings of each
                        // step are matched.
                        Key::Character(c) if c == "=" || c == "+" => self.update(Message::ZoomIn),
                        Key::Character(c) if c == "-" => self.update(Message::ZoomOut),
                        _ => Task::none(),
                    }
                } else {
                    Task::none()
                }
            }
            Message::KeyReleased(key) => {
                // A modifier that goes up is no longer held: Ctrl+scroll
                // must stop zooming once Ctrl is let go.
                self.modifiers = released_modifier(self.modifiers, &key);
                Task::none()
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
            Message::OpenAttachment(path) => {
                // Vault-relative → absolute, then the platform handler.
                let absolute = self.attachment_path(&path);
                self.update(Message::OpenExternal(absolute.display().to_string()))
            }
        }
    }

    fn view(&self) -> cosmic::Element<'_, Self::Message> {
        crate::view::view(self)
    }

    /// The settings drawer (Phase G): shown while `show_context` is set,
    /// toggled by the header's settings button.
    ///
    /// This MUST gate on `show_context`: libcosmic's view renders the
    /// drawer inline whenever this returns `Some`, even with the drawer
    /// nominally closed, so an unconditional `Some` makes the drawer
    /// flicker between overlay and inline rendering instead of closing.
    fn context_drawer(&self) -> Option<cosmic::app::ContextDrawer<'_, Self::Message>> {
        self.core.window.show_context.then(|| {
            cosmic::app::context_drawer(crate::view::settings_view(self), Message::SettingsToggle)
        })
    }

    fn header_start(&self) -> Vec<cosmic::Element<'_, Self::Message>> {
        crate::view::header_start(self)
    }

    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        // Key events always flow (quick switcher + Ctrl+S work everywhere);
        // update filters by app state, so unused keys are cheap noise. The
        // capture status rides along so update can skip events a widget
        // (e.g. the text editor's own bindings) already consumed.
        let keys = cosmic::iced::event::listen_with(|event, status, _window| match event {
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                ..
            }) => Some(Message::KeyPressed(status, modifiers, key)),
            // A released modifier key ends its hold: the wheel needs to
            // know when Ctrl is let go.
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyReleased {
                key,
                ..
            }) => Some(Message::KeyReleased(key)),
            _ => None,
        });
        let mut subs = vec![keys];
        if self.vault.is_some() {
            subs.push(
                cosmic::iced::time::every(std::time::Duration::from_secs(2))
                    .map(|_| Message::WatchTick),
            );
            // Global mouse flow for tree drag-and-drop (and cursor
            // tracking): presses have no position, so the last
            // CursorMoved stands in for it. Captured events still arrive
            // (status is ignored) — buttons capture their clicks, and the
            // drag logic must see them anyway.
            subs.push(cosmic::iced::event::listen_with(
                |event, _status, _window| match event {
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::CursorMoved {
                        position,
                    }) => Some(Message::MouseMoved(position)),
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonPressed(
                        cosmic::iced::mouse::Button::Left,
                    )) => Some(Message::MousePressed),
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
                        cosmic::iced::mouse::Button::Left,
                    )) => Some(Message::MouseReleased),
                    // The wheel zooms the text when Ctrl is held; the
                    // event itself carries no modifiers, so update reads
                    // the ones from the last key event.
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::WheelScrolled {
                        delta,
                    }) => Some(Message::MouseWheel(wheel_delta(delta))),
                    _ => None,
                },
            ));
            // The window's width decides whether the split layout fits:
            // below the threshold it folds down to the source pane.
            subs.push(cosmic::iced::event::listen_with(
                |event, _status, _window| match event {
                    cosmic::iced::Event::Window(cosmic::iced::window::Event::Resized(size)) => {
                        Some(Message::WindowResized(size.width))
                    }
                    _ => None,
                },
            ));
        }
        cosmic::iced::Subscription::batch(subs)
    }
}

/// The vertical wheel movement in lines (up is positive).
///
/// A line-based wheel reports whole lines; a pixel-based one (touchpad
/// or high-resolution wheel) is scaled down to something line-like, so
/// one notch is one step either way.
fn wheel_delta(delta: cosmic::iced::mouse::ScrollDelta) -> f32 {
    match delta {
        cosmic::iced::mouse::ScrollDelta::Lines { x: _, y } => y,
        cosmic::iced::mouse::ScrollDelta::Pixels { x: _, y } => y / 40.0,
    }
}

/// The modifiers still held after `key` is released.
///
/// The wheel event carries no modifiers, so the app tracks them from the
/// key events: a modifier key that goes up is no longer held.
fn released_modifier(held: Modifiers, key: &Key) -> Modifiers {
    match key {
        Key::Named(Named::Control) => held - Modifiers::CTRL,
        Key::Named(Named::Shift) => held - Modifiers::SHIFT,
        Key::Named(Named::Alt) => held - Modifiers::ALT,
        Key::Named(Named::Super) => held - Modifiers::LOGO,
        _ => held,
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

/// A rename/move that invalidated wikilinks, with everything needed to
/// rewrite them later. The pre-rename index is kept because the old
/// targets no longer resolve against the current one; at apply time each
/// referrer is re-scanned fresh, so edits made since the prompt are
/// picked up and only links whose resolution actually changed are touched.
#[derive(Debug)]
pub struct PendingLinkUpdates {
    /// (old path, new path) for the rename just performed — a note, or a
    /// folder whose prefix moves every path beneath it.
    pub renames: Vec<(String, String)>,
    /// Notes (old paths) containing wikilinks/embeds that resolved into
    /// the renamed scope, from the pre-rename backlink index.
    pub referrers: Vec<String>,
    /// The resolution index as it was before the rename.
    pub pre_index: VaultIndex,
}

/// A tree drag in progress (see `drag.rs`): armed on left-press, the
/// source row is identified from the press position's measured bounds,
/// and the drag activates past a movement threshold.
#[derive(Debug, Clone)]
pub struct TreeDrag {
    /// Where the press happened.
    pub press_at: cosmic::iced::Point,
    /// The row under the press: (path, is_folder). Identified lazily so
    /// freshly measured bounds are used.
    pub source: Option<(String, bool)>,
    /// Past the movement threshold: really dragging.
    pub active: bool,
    /// The hovered drop target: a folder path, or "" for the vault root.
    pub hover: Option<String>,
}

/// Movement (px) from the press before a tree drag activates.
const TREE_DRAG_THRESHOLD: f32 = crate::drag::DRAG_THRESHOLD;

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
        assert!(!autosave_due(None, 2));
        // Recent edit: not yet.
        assert!(!autosave_due(Some(Duration::from_millis(500)), 2));
        // Idle past the window: save.
        assert!(autosave_due(Some(Duration::from_secs(2)), 2));
        assert!(autosave_due(Some(Duration::from_secs(10)), 2));
        // A longer configured delay pushes the window out.
        assert!(!autosave_due(Some(Duration::from_secs(2)), 5));
        assert!(autosave_due(Some(Duration::from_secs(5)), 5));
    }
}

#[cfg(test)]
mod chrome_model_tests {
    use super::*;

    #[test]
    fn nav_model_carries_the_five_panels_in_order() {
        let model = build_nav_model();
        assert_eq!(model.len(), SidePanel::ALL.len());
        let panels: Vec<SidePanel> = model
            .iter()
            .map(|id| model.data::<SidePanel>(id).copied().unwrap())
            .collect();
        assert_eq!(panels, SidePanel::ALL);
        // Labels and icons ride along.
        for id in model.iter() {
            let panel = model.data::<SidePanel>(id).copied().unwrap();
            assert_eq!(model.text(id), Some(panel.label()));
            assert!(model.icon(id).is_some(), "{} has an icon", panel.label());
        }
        // Files (the default panel) starts active.
        let active = model.active();
        assert_eq!(model.data::<SidePanel>(active), Some(&SidePanel::Files));
    }

    #[test]
    fn nav_activation_follows_programmatic_panel_switches() {
        let mut model = build_nav_model();
        assert!(activate_entity_with(&mut model, &SidePanel::Outline));
        assert_eq!(
            model.data::<SidePanel>(model.active()),
            Some(&SidePanel::Outline)
        );
        assert!(activate_entity_with(&mut model, &SidePanel::Tags));
        assert_eq!(
            model.data::<SidePanel>(model.active()),
            Some(&SidePanel::Tags)
        );
        // An unknown panel leaves the model untouched.
        // (All SidePanels are known; the false path is for robustness.)
    }

    #[test]
    fn pane_model_defaults_to_live() {
        let mut model = build_pane_model(PaneMode::default());
        assert_eq!(model.len(), 4);
        assert_eq!(
            model.data::<PaneMode>(model.active()),
            Some(&PaneMode::Live)
        );
        let modes: Vec<PaneMode> = model
            .iter()
            .map(|id| model.data::<PaneMode>(id).copied().unwrap())
            .collect();
        assert_eq!(
            modes,
            [
                PaneMode::Live,
                PaneMode::Split,
                PaneMode::Source,
                PaneMode::Reading
            ]
        );
        assert!(activate_entity_with(&mut model, &PaneMode::Source));
        assert_eq!(
            model.data::<PaneMode>(model.active()),
            Some(&PaneMode::Source)
        );
    }

    #[test]
    fn pane_model_highlights_the_persisted_layout_not_the_default() {
        // Startup renders the persisted layout: the highlighted button
        // must name that pane, or the control lies about what is on
        // screen.
        for mode in [
            PaneMode::Live,
            PaneMode::Split,
            PaneMode::Source,
            PaneMode::Reading,
        ] {
            let model = build_pane_model(mode);
            assert_eq!(
                model.data::<PaneMode>(model.active()),
                Some(&mode),
                "pane button does not follow {mode:?}"
            );
        }
    }

    #[test]
    fn set_pane_keeps_the_rendered_pane_and_the_button_in_step() {
        let mut app = find_tests::test_app_with_tab("hello");
        for mode in [
            PaneMode::Split,
            PaneMode::Source,
            PaneMode::Reading,
            PaneMode::Live,
        ] {
            app.set_pane(mode);
            assert_eq!(app.pane, mode);
            assert_eq!(
                app.pane_model.data::<PaneMode>(app.pane_model.active()),
                Some(&mode),
                "pane button does not follow {mode:?}"
            );
        }
    }

    #[test]
    fn tab_labels_show_the_title_and_the_folder_beside_it() {
        let tab = Tab::open("Journal/2026/D.md".to_string(), "written");
        // The title is the file stem; the folder rides along dimmed, so
        // two notes with the same name can be told apart.
        let (title, folder) = tab_label_parts(&tab);
        assert_eq!(title, "D");
        assert_eq!(folder, "Journal/2026");

        // A note at the vault root has no folder to show.
        let root = Tab::open("D.md".to_string(), "written");
        assert_eq!(tab_label_parts(&root), ("D".to_string(), String::new()));
    }

    #[test]
    fn tab_labels_mark_dirty_tabs() {
        let mut tab = Tab::open("D.md".to_string(), "written");
        assert_eq!(tab_label_parts(&tab).0, "D");
        // Same-module test: force dirtiness by diverging base from content.
        tab.base_text = String::new();
        assert_eq!(tab_label_parts(&tab).0, "• D");
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    /// A focused editor key press for shortcut tests.
    fn key_press(key: cosmic::iced::keyboard::Key, modifiers: Modifiers) -> text_editor::KeyPress {
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: cosmic::iced::keyboard::key::Physical::Code(
                cosmic::iced::keyboard::key::Code::KeyB,
            ),
            modifiers,
            text: None,
            status: text_editor::Status::Focused { is_hovered: true },
        }
    }

    #[test]
    fn live_format_shortcuts_map_to_the_formatting_markers() {
        let ctrl = Modifiers::CTRL;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("b".into()), ctrl)),
            Some(Format::Bold)
        );
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("i".into()), ctrl)),
            Some(Format::Italic)
        );
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("k".into()), ctrl)),
            Some(Format::Link)
        );
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("`".into()), ctrl)),
            Some(Format::Code)
        );
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("x".into()), ctrl_shift)),
            Some(Format::Strike)
        );
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("h".into()), ctrl_shift)),
            Some(Format::Highlight)
        );
        // Unmodified keys and non-formatting combos are not formats.
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("b".into()), Modifiers::default())),
            None
        );
        assert_eq!(
            live_format_shortcut(key_press(Key::Character("z".into()), ctrl)),
            None
        );
    }

    #[test]
    fn editor_shortcuts_map_to_messages() {
        let ctrl = Modifiers::CTRL;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        let ctrl_shift_alt = Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT;
        let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("z".into()), ctrl)),
            Some(Message::EditorUndo)
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("z".into()), ctrl_shift)),
            Some(Message::EditorRedo)
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("y".into()), ctrl)),
            Some(Message::EditorRedo)
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("b".into()), ctrl)),
            Some(Message::EditorFormat(Format::Bold))
        ));
        // A shifted layout reports the capital; both map the same.
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("B".into()), ctrl)),
            Some(Message::EditorFormat(Format::Bold))
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("i".into()), ctrl)),
            Some(Message::EditorFormat(Format::Italic))
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("k".into()), ctrl)),
            Some(Message::EditorFormat(Format::Link))
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("x".into()), ctrl_shift)),
            Some(Message::EditorFormat(Format::Strike))
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("h".into()), ctrl_shift)),
            Some(Message::EditorFormat(Format::Highlight))
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Character("f".into()), ctrl_alt)),
            Some(Message::EditorFootnote)
        ));
        // Ctrl+Shift+F is the global vault search — the editor must not
        // claim it.
        assert!(editor_shortcut(key_press(Key::Character("f".into()), ctrl_shift)).is_none());
        // Ctrl+Alt+B etc. are unclaimed (no accidental Alt variants).
        assert!(editor_shortcut(key_press(Key::Character("b".into()), ctrl_shift_alt)).is_none());
        assert!(matches!(
            editor_shortcut(key_press(Key::Named(Named::Backspace), ctrl)),
            Some(Message::EditorDeleteWord(false))
        ));
        assert!(matches!(
            editor_shortcut(key_press(Key::Named(Named::Delete), ctrl)),
            Some(Message::EditorDeleteWord(true))
        ));

        // Unbound combos and unmodified keys fall through to the editor's
        // default bindings (Ctrl+S is the app-wide save; Shift+B is typing).
        assert!(editor_shortcut(key_press(Key::Character("s".into()), ctrl)).is_none());
        assert!(editor_shortcut(key_press(Key::Character("b".into()), Modifiers::SHIFT)).is_none());

        // Not focused: no editor shortcuts at all.
        let mut unfocused = key_press(Key::Character("b".into()), ctrl);
        unfocused.status = text_editor::Status::Active;
        assert!(editor_shortcut(unfocused).is_none());
    }

    #[test]
    fn undo_redo_round_trips_edits() {
        let mut tab = Tab::open("t.md".to_string(), "hello\n");
        // An edit: snapshot first, then the change.
        push_undo(&mut tab);
        tab.content
            .perform(Action::Edit(Edit::Paste(Arc::new("X".to_string()))));
        tab.refresh_doc();
        assert_eq!(tab.content.text(), "Xhello\n");
        // Undo restores the pre-edit text and cursor.
        assert!(undo_tab(&mut tab));
        tab.refresh_doc();
        assert_eq!(tab.content.text(), "hello\n");
        // Redo replays it.
        assert!(redo_tab(&mut tab));
        tab.refresh_doc();
        assert_eq!(tab.content.text(), "Xhello\n");
        // Undo again; a fresh edit then discards the redo branch.
        assert!(undo_tab(&mut tab));
        push_undo(&mut tab);
        tab.content
            .perform(Action::Edit(Edit::Paste(Arc::new("Y".to_string()))));
        tab.refresh_doc();
        assert_eq!(tab.content.text(), "Yhello\n");
        assert!(!redo_tab(&mut tab));
        // Undo with an empty stack is a no-op.
        assert!(!undo_tab(&mut tab) == false || !tab.undo.is_empty());
        while undo_tab(&mut tab) {}
        assert!(!undo_tab(&mut tab));
    }

    #[test]
    fn undo_limit_bounded() {
        let mut tab = Tab::open("t.md".to_string(), "");
        for i in 0..(UNDO_LIMIT + 10) {
            push_undo(&mut tab);
            tab.content
                .perform(Action::Edit(Edit::Paste(Arc::new(i.to_string()))));
        }
        assert_eq!(tab.undo.len(), UNDO_LIMIT);
    }

    #[test]
    fn format_actions_wrap_or_insert_markers() {
        // With a selection: one paste wrapping it.
        assert_eq!(
            format_actions(Some("hi".to_string()), Format::Bold),
            vec![Action::Edit(Edit::Paste(Arc::new("**hi**".to_string())))]
        );
        // Without: empty markers with the cursor stepped between them.
        assert_eq!(
            format_actions(None, Format::Italic),
            vec![
                Action::Edit(Edit::Paste(Arc::new("**".to_string()))),
                Action::Move(Motion::Left),
            ]
        );
        // Link with a selection leaves the cursor inside the parens.
        assert_eq!(
            format_actions(Some("label".to_string()), Format::Link),
            vec![
                Action::Edit(Edit::Paste(Arc::new("[label]()".to_string()))),
                Action::Move(Motion::Left),
            ]
        );
        // An empty selection behaves like no selection.
        assert_eq!(
            format_actions(Some(String::new()), Format::Code),
            format_actions(None, Format::Code)
        );
    }

    #[test]
    fn insert_footnote_puts_the_reference_and_the_definition() {
        let (text, cursor) = insert_footnote("body\n", Position { line: 0, column: 4 }, 1);
        // The reference lands at the cursor; the definition at the end,
        // under a blank line.
        assert_eq!(text, "body[^1]\n\n[^1]: ");
        assert_eq!(cursor, Position { line: 0, column: 8 });
    }

    #[test]
    fn the_footnote_shortcut_edits_the_note_in_one_step() {
        let mut app = find_tests::test_app_with_tab("body\n");
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 4 },
            selection: None,
        });
        app.insert_footnote_at_cursor(0);
        assert_eq!(app.tabs[0].content.text(), "body[^1]\n\n[^1]: ");
        // The cursor sits right after the reference.
        assert_eq!(app.tabs[0].content.cursor().position.column, 8);
        // And it is one undo step.
        undo_tab(&mut app.tabs[0]);
        assert_eq!(app.tabs[0].content.text(), "body\n");
    }

    #[test]
    fn insert_footnote_normalizes_the_blank_lines_before_the_definitions() {
        // Two blank lines before a definition become one…
        let (text, _) =
            insert_footnote("body\n\n\n[^1]: note\n", Position { line: 0, column: 4 }, 2);
        assert_eq!(text, "body[^2]\n\n[^1]: note\n\n[^2]: ");
        // …and none becomes one.
        let (text, _) = insert_footnote("body\n[^1]: note\n", Position { line: 0, column: 4 }, 2);
        assert_eq!(text, "body[^2]\n\n[^1]: note\n\n[^2]: ");
    }

    #[test]
    fn a_definition_in_a_code_block_is_left_alone() {
        let (text, _) =
            insert_footnote("```\n[^1]: code\n```\n", Position { line: 0, column: 0 }, 2);
        assert_eq!(text, "[^2]```\n[^1]: code\n```\n\n[^2]: ");
    }

    #[test]
    fn the_cursor_follows_the_lines_the_normalization_moves() {
        let (text, cursor) = insert_footnote(
            "body\n\n\n[^1]: note\n\nafter\n",
            Position { line: 5, column: 0 },
            2,
        );
        // The blank lines above the definition collapsed, so the cursor
        // moved up with them.
        assert_eq!(cursor, Position { line: 4, column: 4 });
        assert_eq!(text, "body\n\n[^1]: note\n\n[^2]after\n\n[^2]: ");
    }

    #[test]
    fn next_footnote_number_skips_existing_definitions() {
        assert_eq!(next_footnote_number(""), 1);
        assert_eq!(next_footnote_number("no footnotes here"), 1);
        // Bare references do not reserve a number.
        assert_eq!(next_footnote_number("a[^1] ref[^1]\n"), 1);
        assert_eq!(next_footnote_number("[^1]: one\n"), 2);
        // Gaps are not filled: the next unused number wins.
        assert_eq!(next_footnote_number("[^1]: a\n[^3]: b\n"), 2);
        assert_eq!(next_footnote_number("[^1]: a\n[^2]: b\n"), 3);
    }

    #[test]
    fn word_delete_selects_a_word_only_without_a_selection() {
        assert_eq!(
            word_delete_actions(false, false),
            vec![
                Action::Select(Motion::WordLeft),
                Action::Edit(Edit::Backspace)
            ]
        );
        assert_eq!(
            word_delete_actions(false, true),
            vec![
                Action::Select(Motion::WordRight),
                Action::Edit(Edit::Delete)
            ]
        );
        // With a selection, the edit alone removes it.
        assert_eq!(
            word_delete_actions(true, false),
            vec![Action::Edit(Edit::Backspace)]
        );
    }

    #[test]
    fn word_delete_deletes_a_word_end_to_end() {
        let mut tab = Tab::open("t.md".to_string(), "hello world\n");
        // Park the cursor at the end of "world" (line 0, column 11).
        tab.content.perform(Action::Move(Motion::DocumentStart));
        for _ in 0..11 {
            tab.content.perform(Action::Move(Motion::Right));
        }
        for action in word_delete_actions(tab.content.selection().is_some(), false) {
            tab.content.perform(action);
        }
        tab.refresh_doc();
        assert_eq!(tab.content.text(), "hello \n");
    }

    #[test]
    fn word_count_counts_whitespace_runs() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("hello"), 1);
        assert_eq!(count_words("  two  words\nand more "), 4);
    }

    #[test]
    fn the_status_bar_counts_the_note_and_the_selection() {
        let tab = Tab::open("t.md".to_string(), "hello world\n");
        // Without a selection, the note's totals stand alone.
        assert_eq!(status_label(&tab, None), "2 words · 12 characters");
        // A selection adds its own counts.
        assert_eq!(
            status_label(&tab, Some("hello")),
            "2 words · 12 characters (1 words · 5 characters)"
        );
        // An empty selection counts as none.
        assert_eq!(status_label(&tab, Some("")), "2 words · 12 characters");
    }

    #[test]
    fn the_selection_comes_from_the_editor_in_front() {
        let mut app = find_tests::test_app_with_tab("hello world\n");
        // Select "hello" in the source editor: the caret sits at the end
        // of the selection, the anchor at its start.
        app.pane = PaneMode::Source;
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 5 },
            selection: Some(Position { line: 0, column: 0 }),
        });
        assert_eq!(app.selection_text().as_deref(), Some("hello"));

        // In the live pane the focused region's editor holds it instead.
        app.pane = PaneMode::Live;
        app.live = Some(LiveEdit {
            block: 0,
            editor: Content::with_text("one two three"),
        });
        app.live.as_mut().unwrap().editor.move_to(Cursor {
            position: Position { line: 0, column: 3 },
            selection: Some(Position { line: 0, column: 0 }),
        });
        assert_eq!(app.selection_text().as_deref(), Some("one"));

        // The reading pane shows no editor: nothing is selected there.
        app.pane = PaneMode::Reading;
        assert_eq!(app.selection_text(), None);
    }

    #[test]
    fn captured_keys_reach_global_handlers_only_for_our_own_inputs() {
        use cosmic::iced::event::Status;
        // An ignored event always reaches the global handler.
        assert!(global_key_allowed(Status::Ignored, false));
        // A captured event does not — unless one of the app's own text
        // inputs (switcher, tree rename) owns the keyboard.
        assert!(!global_key_allowed(Status::Captured, false));
        assert!(global_key_allowed(Status::Captured, true));
    }
}

#[cfg(test)]
mod autopair_tests {
    use super::*;

    /// A cursor at a line/byte-column position.
    fn cursor_at(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    /// A cursor whose selection runs from the anchor to the caret.
    fn cursor_selecting(line: usize, column: usize, anchor_line: usize, anchor: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: Some(Position {
                line: anchor_line,
                column: anchor,
            }),
        }
    }

    /// The actions of an auto-pair decision.
    fn actions(text: &str, cursor: Cursor, typed: char) -> Vec<Action> {
        autopair_actions(text, cursor, typed).actions
    }

    #[test]
    fn pair_table_covers_brackets_and_quotes() {
        assert_eq!(pair_for('('), Some(('(', ')')));
        assert_eq!(pair_for(')'), Some(('(', ')')));
        assert_eq!(pair_for('['), Some(('[', ']')));
        assert_eq!(pair_for(']'), Some(('[', ']')));
        assert_eq!(pair_for('{'), Some(('{', '}')));
        assert_eq!(pair_for('}'), Some(('{', '}')));
        assert_eq!(pair_for('"'), Some(('"', '"')));
        assert_eq!(pair_for('\''), Some(('\'', '\'')));
        assert_eq!(pair_for('`'), Some(('`', '`')));
        assert_eq!(pair_for('a'), None);
    }

    #[test]
    fn char_helpers_read_around_the_cursor() {
        let text = "ab\ncd\n";
        assert_eq!(char_at(text, Position { line: 0, column: 0 }), Some('a'));
        assert_eq!(char_at(text, Position { line: 0, column: 2 }), None);
        assert_eq!(char_at(text, Position { line: 1, column: 1 }), Some('d'));
        assert_eq!(char_before(text, Position { line: 1, column: 0 }), None);
        assert_eq!(
            char_before(text, Position { line: 1, column: 1 }),
            Some('c')
        );
        // Columns are byte offsets: the 'é' occupies two bytes.
        let text = "héllo";
        assert_eq!(
            char_before(text, Position { line: 0, column: 3 }),
            Some('é')
        );
        assert_eq!(char_at(text, Position { line: 0, column: 3 }), Some('l'));
    }

    #[test]
    fn a_selection_is_wrapped_in_the_pair() {
        assert_eq!(
            actions("hello world\n", cursor_selecting(0, 11, 0, 6), '('),
            vec![Action::Edit(Edit::Paste(Arc::new("(world)".to_string())))]
        );
        assert_eq!(
            actions("q\n", cursor_selecting(0, 1, 0, 0), '"'),
            vec![Action::Edit(Edit::Paste(Arc::new("\"q\"".to_string())))]
        );
    }

    #[test]
    fn a_wrapped_selection_stays_selected() {
        // "world" is wrapped in brackets: the text between the markers
        // stays selected, so another opener pairs again.
        let pair = autopair_actions("hello world\n", cursor_selecting(0, 11, 0, 6), '[');
        let cursor = pair.select.expect("the wrapped text stays selected");
        // "(world)" puts "world" at columns 7..12.
        assert_eq!(
            cursor.position,
            Position {
                line: 0,
                column: 12
            }
        );
        assert_eq!(cursor.selection, Some(Position { line: 0, column: 7 }));
        // The caret sits before the closing bracket.
        let mut text = "hello world\n".to_string();
        text.replace_range(6..11, "[world]");
        assert_eq!(&text[7..12], "world");
    }

    #[test]
    fn a_wrapped_selection_across_lines_keeps_its_bounds() {
        // The selection covers "one\ntwo": the wrapped text starts on
        // the first line and ends on the second.
        let text = "one\ntwo\n";
        let pair = autopair_actions(text, cursor_selecting(1, 3, 0, 0), '(');
        let cursor = pair.select.expect("the wrapped text stays selected");
        assert_eq!(cursor.position, Position { line: 1, column: 3 });
        assert_eq!(cursor.selection, Some(Position { line: 0, column: 1 }));
    }

    #[test]
    fn a_closer_over_a_selection_replaces_it() {
        // Typing the closing bracket does not wrap: it replaces the
        // selection, and nothing stays selected.
        let pair = autopair_actions("hello world\n", cursor_selecting(0, 11, 0, 6), ']');
        assert_eq!(
            pair.actions,
            vec![Action::Edit(Edit::Paste(Arc::new("]".to_string())))]
        );
        assert!(pair.select.is_none());
    }

    #[test]
    fn without_a_selection_the_pair_is_inserted() {
        assert_eq!(
            actions("", cursor_at(0, 0), '('),
            vec![
                Action::Edit(Edit::Paste(Arc::new("()".to_string()))),
                Action::Move(Motion::Left),
            ]
        );
    }

    #[test]
    fn typing_the_closer_steps_over_it() {
        // The cursor sits before the ")" of "()" — typing ")" moves past it.
        assert_eq!(
            actions("()", cursor_at(0, 1), ')'),
            vec![Action::Move(Motion::Right)]
        );
        // Not before a closer: the character is simply inserted.
        assert_eq!(
            actions("x", cursor_at(0, 1), ')'),
            vec![Action::Edit(Edit::Insert(')'))]
        );
    }

    #[test]
    fn quotes_do_not_pair_inside_a_word() {
        // An apostrophe inside a word stays a plain character.
        assert_eq!(
            actions("don", cursor_at(0, 3), '\''),
            vec![Action::Edit(Edit::Insert('\''))]
        );
        // After a space (or at the start) they pair.
        assert_eq!(actions("don ", cursor_at(0, 4), '\'').len(), 2);
        assert_eq!(actions("", cursor_at(0, 0), '"').len(), 2);
    }

    #[test]
    fn typing_a_pair_end_to_end_does_not_duplicate_closers() {
        let mut tab = Tab::open("t.md".to_string(), "");
        // Type "(" → "()" with the cursor in the middle.
        for action in actions("", tab.content.cursor(), '(') {
            tab.content.perform(action);
        }
        assert_eq!(tab.content.text(), "()");
        // Type ")" → the cursor steps over the existing closer.
        for action in actions(&tab.content.text(), tab.content.cursor(), ')') {
            tab.content.perform(action);
        }
        assert_eq!(tab.content.text(), "()");
        // The cursor now sits after the pair, so a new opener starts there.
        for action in actions(&tab.content.text(), tab.content.cursor(), '[') {
            tab.content.perform(action);
        }
        assert_eq!(tab.content.text(), "()[]");
        // Typing the closer of that pair steps over it too.
        for action in actions(&tab.content.text(), tab.content.cursor(), ']') {
            tab.content.perform(action);
        }
        assert_eq!(tab.content.text(), "()[]");
        tab.refresh_doc();
    }

    #[test]
    fn pairing_a_selection_twice_makes_a_wikilink() {
        // Select "My Link", pair it with "[", then pair it again: the
        // selection survives the first pair, so the second one wraps it
        // a second time.
        let mut tab = Tab::open("t.md".to_string(), "My Link\n");
        tab.content.move_to(cursor_selecting(0, 7, 0, 0));
        let pair = autopair_actions(&tab.content.text(), tab.content.cursor(), '[');
        for action in pair.actions {
            tab.content.perform(action);
        }
        if let Some(cursor) = pair.select {
            tab.content.move_to(cursor);
        }
        assert_eq!(tab.content.text(), "[My Link]\n");
        assert_eq!(tab.content.selection(), Some("My Link".to_string()));

        let pair = autopair_actions(&tab.content.text(), tab.content.cursor(), '[');
        for action in pair.actions {
            tab.content.perform(action);
        }
        if let Some(cursor) = pair.select {
            tab.content.move_to(cursor);
        }
        assert_eq!(tab.content.text(), "[[My Link]]\n");
        assert_eq!(tab.content.selection(), Some("My Link".to_string()));
    }
}

#[cfg(test)]
mod word_motion_tests {
    use super::*;

    /// The bytes a word delete would remove, the way the editor's own
    /// word motion finds them.
    fn before(text: &str, offset: usize) -> Option<String> {
        word_before(text, offset).map(|(start, end)| text[start..end].to_string())
    }

    fn after(text: &str, offset: usize) -> Option<String> {
        word_after(text, offset).map(|(start, end)| text[start..end].to_string())
    }

    #[test]
    fn the_word_before_the_caret_is_the_word_it_starts() {
        let text = "alpha beta";
        // The caret in the middle of "beta": the word part before it.
        assert_eq!(before(text, 8).as_deref(), Some("be"));
        // The caret before the word's last letter: the part before it.
        assert_eq!(before(text, 9).as_deref(), Some("bet"));
        // The caret at the end of the line: the whole word.
        assert_eq!(before(text, 10).as_deref(), Some("beta"));
        // The caret at the start of the line: nothing before it.
        assert_eq!(before(text, 0), None);
    }

    #[test]
    fn whitespace_before_the_caret_goes_with_the_word() {
        // The motion finds the start of the last word before the caret,
        // so the spaces between it and the caret go too.
        assert_eq!(before("alpha   beta", 8).as_deref(), Some("alpha   "));
        // A line with no word at all: only the spaces go.
        assert_eq!(before("   ", 3).as_deref(), Some("   "));
    }

    #[test]
    fn a_caret_at_a_line_start_reaches_the_line_break() {
        let text = "alpha beta\ngamma";
        // The caret at the start of the second line: the motion jumps to
        // the end of the line before it, so the break is what goes.
        assert_eq!(before(text, 11).as_deref(), Some("\n"));
        // The very start of the text: there is nothing to delete.
        assert_eq!(before(text, 0), None);
    }

    #[test]
    fn the_word_after_the_caret_is_the_word_it_ends() {
        let text = "alpha beta";
        // The caret at the start of the line: the whole word.
        assert_eq!(after(text, 0).as_deref(), Some("alpha"));
        // The caret inside the word: the rest of it.
        assert_eq!(after(text, 2).as_deref(), Some("pha"));
        // The caret before the word's last letter: that letter.
        assert_eq!(after(text, 9).as_deref(), Some("a"));
        // The caret at the end of the text: nothing after it.
        assert_eq!(after(text, 10), None);
    }

    #[test]
    fn a_caret_at_a_line_end_reaches_the_line_break() {
        let text = "alpha beta\ngamma";
        // The caret at the end of the first line: the motion steps over
        // the break.
        assert_eq!(after(text, 10).as_deref(), Some("\n"));
        // The end of the text: nothing follows.
        assert_eq!(after(text, text.len()), None);
    }

    #[test]
    fn offsets_and_positions_round_trip() {
        let text = "alpha beta\ngamma\n";
        assert_eq!(offset_of(text, 0, 6), 6);
        assert_eq!(offset_of(text, 1, 0), 11);
        assert_eq!(offset_of(text, 1, 3), 14);
        // A column past the line's end is clamped to it.
        assert_eq!(offset_of(text, 0, 99), 10);
        // A line past the text's end is clamped to the text's end.
        assert_eq!(offset_of(text, 9, 0), text.len());
        assert_eq!(position_of(text, 6), (0, 6));
        assert_eq!(position_of(text, 11), (1, 0));
        assert_eq!(position_of(text, 14), (1, 3));
        assert_eq!(position_of(text, text.len()), (2, 0));
    }

    #[test]
    fn multi_byte_text_keeps_the_columns_on_character_boundaries() {
        let text = "äöü x";
        // The caret cannot sit inside a character: the offset snaps back
        // to the character's start.
        assert_eq!(offset_of(text, 0, 1), 0);
        assert_eq!(offset_of(text, 0, 2), 2);
        assert_eq!(offset_of(text, 0, 5), 4);
        assert_eq!(position_of(text, 5), (0, 4));
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    #[test]
    fn find_matches_scans_lines_with_case_handling() {
        let text = "Alpha alpha\nbeta ALPHA\n";
        // Case-insensitive (default): every occurrence, in order.
        assert_eq!(
            find_matches(text, "alpha", false),
            vec![
                TextMatch {
                    line: 0,
                    start: 0,
                    end: 5
                },
                TextMatch {
                    line: 0,
                    start: 6,
                    end: 11
                },
                TextMatch {
                    line: 1,
                    start: 5,
                    end: 10
                },
            ]
        );
        // Case-sensitive: only the exact ones.
        assert_eq!(
            find_matches(text, "alpha", true),
            vec![TextMatch {
                line: 0,
                start: 6,
                end: 11
            }]
        );
    }

    #[test]
    fn find_matches_edge_cases() {
        // Empty and multi-line queries match nothing.
        assert!(find_matches("text", "", false).is_empty());
        assert!(find_matches("text", "a\nb", false).is_empty());
        // Columns are byte offsets: é is two bytes.
        let text = "café café";
        let hits = find_matches(text, "café", false);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].start, 6);
        // Adjacent, non-overlapping occurrences.
        assert_eq!(find_matches("aaaa", "aa", false).len(), 2);
    }

    #[test]
    fn navigation_indices_wrap() {
        assert_eq!(first_at_or_after(&[], 0, 0), 0);
        let matches = vec![
            TextMatch {
                line: 0,
                start: 0,
                end: 1,
            },
            TextMatch {
                line: 0,
                start: 4,
                end: 5,
            },
            TextMatch {
                line: 2,
                start: 0,
                end: 1,
            },
        ];
        assert_eq!(first_at_or_after(&matches, 0, 0), 0);
        assert_eq!(first_at_or_after(&matches, 0, 4), 1);
        assert_eq!(first_at_or_after(&matches, 1, 0), 2);
        assert_eq!(first_at_or_after(&matches, 5, 0), 0);
        assert_eq!(next_index(3, 0, true), 1);
        assert_eq!(next_index(3, 2, true), 0);
        assert_eq!(next_index(3, 0, false), 2);
        assert_eq!(next_index(0, 5, true), 0);
    }

    #[test]
    fn replace_splices_are_byte_exact() {
        let text = "one two one\nthree\n";
        let replaced = replace_spans(
            &text,
            &[TextMatch {
                line: 0,
                start: 4,
                end: 7,
            }],
            "2",
        );
        assert_eq!(replaced, "one 2 one\nthree\n");
        // Every occurrence at once, including the trailing newline.
        let all = replace_spans(
            &text,
            &[
                TextMatch {
                    line: 0,
                    start: 0,
                    end: 3,
                },
                TextMatch {
                    line: 0,
                    start: 8,
                    end: 11,
                },
            ],
            "1",
        );
        assert_eq!(all, "1 two 1\nthree\n");
    }

    #[test]
    fn find_replace_one_replaces_and_advances() {
        let mut app = find_tests::test_app_with_tab("dot dot dot\n");
        let find = Find {
            query: "dot".to_string(),
            replacement: "spot".to_string(),
            ..Find::default()
        };
        app.find = Some(find);
        // First match.
        app.select_match(0);
        app.find_replace_one(0);
        assert_eq!(app.tabs[0].content.text(), "spot dot dot\n");
        // The selection advanced to the following match.
        assert_eq!(app.find.as_ref().unwrap().current, 0);
        // Replace remaining.
        app.find_replace_one(0);
        app.find_replace_one(0);
        assert_eq!(app.tabs[0].content.text(), "spot spot spot\n");
        // Undo rolls the last replacement back.
        let tab = &mut app.tabs[0];
        assert!(undo_tab(tab));
        assert_eq!(tab.content.text(), "spot spot dot\n");
    }

    #[test]
    fn find_replace_all_replaces_everything_in_one_step() {
        let mut app = find_tests::test_app_with_tab("aXbXc\nX\n");
        app.find = Some(Find {
            query: "X".to_string(),
            replacement: "Y".to_string(),
            ..Find::default()
        });
        app.find_replace_all();
        assert_eq!(app.tabs[0].content.text(), "aYbYc\nY\n");
        // The whole batch is one undo.
        let tab = &mut app.tabs[0];
        assert!(undo_tab(tab));
        assert_eq!(tab.content.text(), "aXbXc\nX\n");
    }

    #[test]
    fn find_step_selects_matches_in_the_editor() {
        let mut app = find_tests::test_app_with_tab("one two one\n");
        // Source-editor cursor semantics (the live pane selects its own
        // region buffer — see live_tests).
        app.pane = PaneMode::Source;
        app.find = Some(Find {
            query: "one".to_string(),
            ..Find::default()
        });
        // Opening jumps to the first match at/after the cursor: match 0.
        app.find_jump_to_cursor();
        let cursor = app.tabs[0].content.cursor();
        assert_eq!(cursor.selection, Some(Position { line: 0, column: 0 }));
        assert_eq!(cursor.position, Position { line: 0, column: 3 });
        // Next wraps forward to the second match.
        app.find_step(true);
        let cursor = app.tabs[0].content.cursor();
        assert_eq!(cursor.selection, Some(Position { line: 0, column: 8 }));
        assert_eq!(
            cursor.position,
            Position {
                line: 0,
                column: 11
            }
        );
        // Previous wraps back around to the first.
        app.find_step(false);
        let cursor = app.tabs[0].content.cursor();
        assert_eq!(cursor.selection, Some(Position { line: 0, column: 0 }));
        assert_eq!(cursor.position, Position { line: 0, column: 3 });
    }

    /// A minimal app with one open tab, for find tests.
    pub(crate) fn test_app_with_tab(text: &str) -> VaultNotes {
        let mut app = VaultNotes {
            core: cosmic::app::Core::default(),
            vault: None,
            index: VaultIndex::new(Vec::<String>::new()),
            vault_name: String::new(),
            tree: Vec::new(),
            collapsed: HashSet::new(),
            tabs: Vec::new(),
            active: None,
            pane: PaneMode::default(),
            banner: None,
            pending_links: None,
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
            hide_md_extension: true,
            scroll_sync: true,
            auto_pair: true,
            wiki_autocomplete: true,
            word_wrap: true,
            editor_font_mono: true,
            editor_font_family: String::new(),
            editor_font_size: 14,
            reading_font_family: String::new(),
            reading_font_size: 16,
            reading_margin: settings::ReadingMargin::default(),
            zoom_target: ZoomTarget::default(),
            modifiers: Modifiers::default(),
            restore_split: false,
            sidebar_open: true,
            tree_drag: None,
            last_cursor: None,
            tree_bounds: crate::drag::TreeBounds::default(),
            bounds_pending: false,
            autosave_secs: 2,
            default_pane: PaneMode::default(),
            attachment_extensions: Vec::new(),
            link_path_format: LinkPathFormat::default(),
            tree_sort: TreeSort::default(),
            attachment_input: String::new(),
            unicode_input: None,
            tree_edit: None,
            tree_menu_target: None,
            find: None,
            wiki_complete: None,
            live: None,
            nav_model: build_nav_model(),
            pane_model: build_pane_model(PaneMode::default()),
        };
        app.tabs.push(Tab::open("t.md".to_string(), text));
        app.active = Some(0);
        app
    }
}

#[cfg(test)]
mod wiki_tests {
    use super::*;
    use cosmic::Application as _;

    #[test]
    fn wiki_query_detects_the_open_link_being_typed() {
        let text = "see [[An\ntail\n";
        // Cursor after "[[An": link start at byte 4, partial "An".
        assert_eq!(
            wiki_query_at(text, Position { line: 0, column: 8 }),
            Some((4, "An".to_string()))
        );
        // Empty query right after "[[".
        assert_eq!(
            wiki_query_at(text, Position { line: 0, column: 6 }),
            Some((4, String::new()))
        );
    }

    #[test]
    fn the_custom_label_after_the_caret_is_found() {
        // The caret sits in front of the bar; the label follows it.
        assert_eq!(
            wiki_alias_at("see [[|some label]]", Position { line: 0, column: 6 }),
            Some(("some label".to_string(), 17))
        );
        // An empty label is still a label.
        assert_eq!(
            wiki_alias_at("see [[|]]", Position { line: 0, column: 6 }),
            Some((String::new(), 7))
        );
        // No bar: no label.
        assert_eq!(
            wiki_alias_at("see [[Note]]", Position { line: 0, column: 6 }),
            None
        );
        // A link that is not closed on this line has no label to keep.
        assert_eq!(
            wiki_alias_at("see [[|some label", Position { line: 0, column: 6 }),
            None
        );
    }

    #[test]
    fn wiki_accept_keeps_the_custom_label() {
        // The user made the link shape first and put the caret in
        // front of the bar: accepting a suggestion fills in the target
        // and leaves the label alone.
        let mut app = find_tests::test_app_with_tab("see [[|]]\n");
        app.pane = PaneMode::Source;
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 6 },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["Note Title.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        assert_eq!(app.tabs[0].content.text(), "see [[Note Title|]]\n");
        // The caret sits right after the bar.
        assert_eq!(app.tabs[0].content.cursor().position.column, 17);
    }

    #[test]
    fn wiki_accept_keeps_a_written_label() {
        let mut app = find_tests::test_app_with_tab("see [[|some label]]\n");
        app.pane = PaneMode::Source;
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 6 },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["Note Title.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        assert_eq!(
            app.tabs[0].content.text(),
            "see [[Note Title|some label]]\n"
        );
        // The caret sits right after the bar, in front of the label.
        assert_eq!(app.tabs[0].content.cursor().position.column, 17);
    }

    #[test]
    fn wiki_query_stops_at_closers_and_earlier_lines() {
        // A closed link suggests nothing.
        assert_eq!(
            wiki_query_at("see [[An]]|", Position { line: 0, column: 9 }),
            None
        );
        // Typing an alias (`|`) suggests nothing.
        assert_eq!(
            wiki_query_at(
                "see [[An|ali",
                Position {
                    line: 0,
                    column: 12
                }
            ),
            None
        );
        // An unclosed `[[` on an earlier line does not reach across lines.
        assert_eq!(
            wiki_query_at("see [[An\ntail\n", Position { line: 1, column: 4 }),
            None
        );
        // No link at all.
        assert_eq!(
            wiki_query_at(
                "plain text",
                Position {
                    line: 0,
                    column: 10
                }
            ),
            None
        );
    }

    #[test]
    fn wiki_query_columns_are_byte_offsets() {
        // "é[[no" — é is two bytes, so `[[` starts at byte 2.
        let text = "é[[no\n";
        assert_eq!(
            wiki_query_at(text, Position { line: 0, column: 6 }),
            Some((2, "no".to_string()))
        );
    }

    #[test]
    fn wiki_paths_rank_by_title_but_carry_paths() {
        let paths: Vec<String> = ["Notes/Alpha.md", "Deep/Alphabet.md", "Beta.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Empty query passes everything through, in order.
        assert_eq!(wiki_paths("", &paths).len(), 3);
        let ranked = wiki_paths("alp", &paths);
        assert!(ranked.contains(&"Notes/Alpha.md".to_string()));
        assert!(ranked.contains(&"Deep/Alphabet.md".to_string()));
        assert!(!ranked.contains(&"Beta.md".to_string()));
        // No matches -> empty (which closes the dropdown).
        assert!(wiki_paths("zzz", &paths).is_empty());
    }

    #[test]
    fn wiki_accept_completes_the_link_in_one_undo_step() {
        let mut app = find_tests::test_app_with_tab("see [[no\n");
        // Source-editor path (the live-buffer path is covered in
        // live_tests::wiki_accept_in_live_mode_splices_the_link).
        app.pane = PaneMode::Source;
        // Cursor right after "[[no" (byte column 8).
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 8 },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["Note Title.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        // Shortest format with a unique title: the bare title.
        assert_eq!(app.tabs[0].content.text(), "see [[Note Title]]\n");
        // The dropdown closed.
        assert!(app.wiki_complete.is_none());
        // The whole completion is one undo step.
        let tab = &mut app.tabs[0];
        assert!(undo_tab(tab));
        assert_eq!(tab.content.text(), "see [[no\n");
    }

    #[test]
    fn wiki_close_run_counts_the_brackets_the_pairer_left() {
        // The caret sits in front of the "]]" the auto-pair inserted.
        let text = "see [[My Link]]";
        assert_eq!(
            wiki_close_run(
                text,
                Position {
                    line: 0,
                    column: 13
                }
            ),
            2
        );
        // Nothing behind the caret: nothing to consume.
        assert_eq!(
            wiki_close_run(
                "see [[My Link",
                Position {
                    line: 0,
                    column: 13
                }
            ),
            0
        );
        // A single bracket is a bracket.
        assert_eq!(
            wiki_close_run(
                "see [[My Link]",
                Position {
                    line: 0,
                    column: 13
                }
            ),
            1
        );
        // At most the link's own two brackets are consumed.
        assert_eq!(
            wiki_close_run(
                "see [[My Link]]]",
                Position {
                    line: 0,
                    column: 13
                }
            ),
            2
        );
    }

    #[test]
    fn wiki_accept_consumes_the_auto_paired_closing_brackets() {
        // With auto-pair on, typing "[[My Link" leaves the pairer's "]]"
        // behind the caret. Accepting the completion must consume them
        // with the rest, or the link ends up with four brackets.
        let mut app = find_tests::test_app_with_tab("see [[My Link]]\n");
        app.pane = PaneMode::Source;
        // The caret sits right after "My Link", in front of the "]]".
        app.tabs[0].content.move_to(Cursor {
            position: Position {
                line: 0,
                column: 13,
            },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["My Link.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        assert_eq!(app.tabs[0].content.text(), "see [[My Link]]\n");
    }

    #[test]
    fn wiki_messages_cycle_and_dismiss() {
        let mut app = find_tests::test_app_with_tab("see [[\n");
        app.wiki_complete = Some(WikiComplete {
            items: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            selected: 0,
        });
        app.update(Message::WikiCycle(1));
        assert_eq!(app.wiki_complete.as_ref().unwrap().selected, 1);
        // Wraps backwards: 0 - 1 -> 2.
        app.update(Message::WikiCycle(-1));
        app.update(Message::WikiCycle(-1));
        assert_eq!(app.wiki_complete.as_ref().unwrap().selected, 2);
        app.update(Message::WikiDismiss);
        assert!(app.wiki_complete.is_none());
    }

    #[test]
    fn wiki_dropdown_closes_without_an_open_link() {
        let mut app = find_tests::test_app_with_tab("see [[note]] done\n");
        app.tabs[0].content.move_to(Cursor {
            position: Position {
                line: 0,
                column: 18,
            },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["note".to_string()],
            selected: 0,
        });
        // The cursor sits past the closed link: no query, dropdown closes.
        app.update_wiki_complete();
        assert!(app.wiki_complete.is_none());
    }
}

#[cfg(test)]
mod editor_menu_tests {
    use super::*;
    use cosmic::Application as _;

    #[test]
    fn editor_paste_replaces_selection_and_is_undoable() {
        let mut app = find_tests::test_app_with_tab("hello world\n");
        // Select "hello".
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 5 },
            selection: Some(Position { line: 0, column: 0 }),
        });
        app.update(Message::EditorPasteText(Some("goodbye".to_string())));
        assert_eq!(app.tabs[0].content.text(), "goodbye world\n");
        // One undo step restores the selection era.
        let tab = &mut app.tabs[0];
        assert!(undo_tab(tab));
        assert_eq!(tab.content.text(), "hello world\n");
    }

    #[test]
    fn editor_paste_without_clipboard_text_is_a_no_op() {
        let mut app = find_tests::test_app_with_tab("intact\n");
        app.update(Message::EditorPasteText(None));
        assert_eq!(app.tabs[0].content.text(), "intact\n");
    }

    #[test]
    fn editor_cut_deletes_the_selection() {
        let mut app = find_tests::test_app_with_tab("hello world\n");
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 5 },
            selection: Some(Position { line: 0, column: 0 }),
        });
        app.update(Message::EditorCut);
        assert_eq!(app.tabs[0].content.text(), " world\n");
        // The deletion is undoable.
        let tab = &mut app.tabs[0];
        assert!(undo_tab(tab));
        assert_eq!(tab.content.text(), "hello world\n");
    }

    #[test]
    fn editor_copy_leaves_the_buffer_untouched() {
        let mut app = find_tests::test_app_with_tab("hello world\n");
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 5 },
            selection: Some(Position { line: 0, column: 0 }),
        });
        app.update(Message::EditorCopy);
        assert_eq!(app.tabs[0].content.text(), "hello world\n");
        // A copy is not an edit: no undo snapshot was pushed.
        assert!(!undo_tab(&mut app.tabs[0]));
    }

    #[test]
    fn editor_select_all_selects_everything() {
        let mut app = find_tests::test_app_with_tab("hello world");
        app.update(Message::EditorSelectAll);
        assert_eq!(
            app.tabs[0].content.selection(),
            Some("hello world".to_string())
        );
    }
}

#[cfg(test)]
mod scroll_sync_tests {
    use super::*;

    #[test]
    fn sync_records_the_caret_line_and_targets_its_block() {
        let mut app = find_tests::test_app_with_tab("# Head\n\nPara one.\ncontinues\n\n- item\n");
        // Spans: heading 1-2, paragraph 3-5, list 6-6.
        app.pane = PaneMode::Split;
        app.scroll_sync = true;
        // Cursor on "continues" (0-based line 3 = file line 4) → block 1.
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 3, column: 2 },
            selection: None,
        });
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, Some(4));
        // No cursor movement: no re-sync (the line is unchanged).
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, Some(4));
        // A line change re-arms sync.
        app.tabs[0].content.move_to(Cursor {
            position: Position { line: 0, column: 0 },
            selection: None,
        });
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, Some(1));
    }

    #[test]
    fn sync_is_gated_by_pane_mode_and_setting() {
        let mut app = find_tests::test_app_with_tab("# Head\n\ntext\n");
        // Source mode: no preview to follow.
        app.pane = PaneMode::Source;
        app.scroll_sync = true;
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, None);
        // Reading mode: the editor is not visible, nothing to follow.
        app.pane = PaneMode::Reading;
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, None);
        // Setting off: no sync even in Split.
        app.pane = PaneMode::Split;
        app.scroll_sync = false;
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, None);
        // Back on: it syncs.
        app.scroll_sync = true;
        app.sync_preview_scroll();
        assert_eq!(app.tabs[0].synced_line, Some(1));
    }
}

#[cfg(test)]
mod attachment_tree_tests {
    use super::*;

    #[test]
    fn attachments_appear_in_the_tree_and_open_vault_absolutely() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
        std::fs::write(tmp.path().join("n.md"), "note\n").unwrap();
        std::fs::write(tmp.path().join("assets/cat.png"), "png").unwrap();

        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();

        // The attachment is a tree leaf beside the notes.
        let leaves = tree::folder_paths(&app.tree); // sanity: helper runs
        let _ = leaves;
        fn collect(nodes: &[TreeNode], out: &mut Vec<(String, String)>) {
            for node in nodes {
                match node {
                    TreeNode::Folder { children, .. } => collect(children, out),
                    TreeNode::Note { name, path } => out.push((name.clone(), path.clone())),
                }
            }
        }
        let mut leaves = Vec::new();
        collect(&app.tree, &mut leaves);
        assert!(leaves.contains(&("n.md".to_string(), "n.md".to_string())));
        assert!(leaves.contains(&("cat.png".to_string(), "assets/cat.png".to_string())));

        // OpenAttachment resolves vault-relative to absolute for the handler.
        assert_eq!(
            app.attachment_path("assets/cat.png"),
            tmp.path().join("assets/cat.png")
        );
        // Without a vault, the raw path passes through.
        let bare = find_tests::test_app_with_tab("");
        assert_eq!(bare.attachment_path("x.png"), PathBuf::from("x.png"));
    }
}

#[cfg(test)]
mod context_drawer_tests {
    use super::*;
    use cosmic::app::Application as _;

    #[test]
    fn settings_drawer_exists_only_while_shown() {
        let mut app = find_tests::test_app_with_tab("");
        // Closed: no drawer element at all. libcosmic renders the drawer
        // inline whenever `context_drawer()` returns Some, even with the
        // drawer nominally closed — so this gate is what makes closing
        // actually work.
        assert!(app.context_drawer().is_none());
        app.core.set_show_context(true);
        assert!(app.context_drawer().is_some());
        app.core.set_show_context(false);
        assert!(app.context_drawer().is_none());
    }

    #[test]
    fn settings_toggle_flips_show_context() {
        let mut app = find_tests::test_app_with_tab("");
        let _ = app.update(Message::SettingsToggle);
        assert!(app.core.window.show_context);
        let _ = app.update(Message::SettingsToggle);
        assert!(!app.core.window.show_context);
    }

    #[test]
    fn escape_closes_the_settings_drawer_when_nothing_else_is_open() {
        let mut app = find_tests::test_app_with_tab("");
        app.core.set_show_context(true);
        // Plain Esc (uncaptured, no modifiers) closes the drawer.
        let _ = app.update(Message::KeyPressed(
            cosmic::iced::event::Status::Ignored,
            Modifiers::empty(),
            Key::Named(Named::Escape),
        ));
        assert!(!app.core.window.show_context);
        // With the drawer closed, Esc is a no-op (no re-toggle).
        let _ = app.update(Message::KeyPressed(
            cosmic::iced::event::Status::Ignored,
            Modifiers::empty(),
            Key::Named(Named::Escape),
        ));
        assert!(!app.core.window.show_context);
        // A captured Esc (e.g. a drawer text input holds focus) must NOT
        // leak to the global handler.
        app.core.set_show_context(true);
        let _ = app.update(Message::KeyPressed(
            cosmic::iced::event::Status::Captured,
            Modifiers::empty(),
            Key::Named(Named::Escape),
        ));
        assert!(app.core.window.show_context);
    }
}

#[cfg(test)]
mod attachment_filter_tests {
    use super::*;

    #[test]
    fn extension_input_normalizes_and_rejects_junk() {
        assert_eq!(normalize_extension(" PNG "), Some("png".to_string()));
        assert_eq!(normalize_extension(".Pdf"), Some("pdf".to_string()));
        assert_eq!(normalize_extension("..webm"), Some("webm".to_string()));
        assert_eq!(normalize_extension("   "), None);
        assert_eq!(normalize_extension("."), None);
    }

    #[test]
    fn attachment_filter_is_a_case_insensitive_whitelist() {
        let exts = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Empty list: everything shows (the default).
        assert!(attachment_shown("assets/cat.png", &exts(&[])));
        assert!(attachment_shown("file.without-ext", &exts(&[])));
        // Whitelist: matching extension (any case) shows.
        assert!(attachment_shown("assets/cat.PNG", &exts(&["png"])));
        assert!(attachment_shown("a/b.Pdf", &exts(&["png", "pdf"])));
        // Non-matching extension or extensionless files are hidden.
        assert!(!attachment_shown("assets/cat.png", &exts(&["pdf"])));
        assert!(!attachment_shown("file.without-ext", &exts(&["png"])));
    }

    #[test]
    fn tree_filter_hides_unlisted_attachments_but_keeps_notes() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
        std::fs::write(tmp.path().join("n.md"), "note\n").unwrap();
        std::fs::write(tmp.path().join("assets/cat.png"), "png").unwrap();
        std::fs::write(tmp.path().join("assets/scan.pdf"), "pdf").unwrap();

        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());

        // Whitelist of png only: the pdf disappears, the note stays.
        app.attachment_extensions = vec!["png".to_string()];
        app.rebuild_index();
        fn collect(nodes: &[TreeNode], out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    TreeNode::Folder { children, .. } => collect(children, out),
                    TreeNode::Note { path, .. } => out.push(path.clone()),
                }
            }
        }
        let mut leaves = Vec::new();
        collect(&app.tree, &mut leaves);
        assert!(leaves.contains(&"n.md".to_string()));
        assert!(leaves.contains(&"assets/cat.png".to_string()));
        assert!(!leaves.contains(&"assets/scan.pdf".to_string()));

        // Empty list: everything is back.
        app.attachment_extensions.clear();
        app.rebuild_index();
        let mut leaves = Vec::new();
        collect(&app.tree, &mut leaves);
        assert!(leaves.contains(&"assets/scan.pdf".to_string()));
    }
}

#[cfg(test)]
mod unicode_entry_tests {
    use super::*;
    use cosmic::app::Application as _;
    use cosmic::iced::keyboard::key::Named;
    use cosmic::iced::keyboard::{Key, Modifiers};
    use cosmic::iced::widget::text_editor::{self, Status};

    fn press(key: Key, modifiers: Modifiers) -> text_editor::KeyPress {
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: cosmic::iced::keyboard::key::Physical::Code(
                cosmic::iced::keyboard::key::Code::KeyU,
            ),
            modifiers,
            text: None,
            status: Status::Focused { is_hovered: false },
        }
    }

    #[test]
    fn codepoint_parsing_rejects_surrogates_and_overflow() {
        assert_eq!(unicode_char("1f600"), Some('😀'));
        assert_eq!(unicode_char("41"), Some('A'));
        // Surrogates are not valid characters.
        assert_eq!(unicode_char("d800"), None);
        // Beyond U+10FFFF.
        assert_eq!(unicode_char("110000"), None);
        assert_eq!(unicode_char("zz"), None);
    }

    #[test]
    fn start_chord_maps_to_unicode_entry() {
        let ctrl_shift_u = press(
            Key::Character("U".into()),
            Modifiers::CTRL | Modifiers::SHIFT,
        );
        assert!(matches!(
            editor_shortcut(ctrl_shift_u),
            Some(Message::UnicodeStart)
        ));
        // Without shift it stays unmapped (Ctrl+U is nothing today).
        let ctrl_u = press(Key::Character("u".into()), Modifiers::CTRL);
        assert!(matches!(editor_shortcut(ctrl_u), None));
    }

    #[test]
    fn entry_keys_feed_the_hex_buffer() {
        // Hex digits accumulate (lowercased), in any case.
        assert!(matches!(
            unicode_entry_binding(&press(Key::Character("F".into()), Modifiers::empty())),
            Some(text_editor::Binding::Custom(Message::UnicodeHex('F')))
        ));
        // Space commits (it arrives as a character key).
        assert!(matches!(
            unicode_entry_binding(&press(Key::Character(" ".into()), Modifiers::empty())),
            Some(text_editor::Binding::Custom(Message::UnicodeCommit))
        ));
        // Enter commits, Backspace unravels, Esc cancels, Tab is ignored.
        assert!(matches!(
            unicode_entry_binding(&press(Key::Named(Named::Enter), Modifiers::empty())),
            Some(text_editor::Binding::Custom(Message::UnicodeCommit))
        ));
        assert!(matches!(
            unicode_entry_binding(&press(Key::Named(Named::Backspace), Modifiers::empty())),
            Some(text_editor::Binding::Custom(Message::UnicodeBackspace))
        ));
        assert!(matches!(
            unicode_entry_binding(&press(Key::Named(Named::Escape), Modifiers::empty())),
            Some(text_editor::Binding::Custom(Message::UnicodeCancel))
        ));
        assert!(matches!(
            unicode_entry_binding(&press(Key::Named(Named::Tab), Modifiers::empty())),
            Some(text_editor::Binding::Custom(Message::UnicodeIgnore))
        ));
        // Chords with Ctrl or Alt fall through so shortcuts keep working.
        assert!(matches!(
            unicode_entry_binding(&press(Key::Character("s".into()), Modifiers::CTRL)),
            None
        ));
        // Arrow keys fall through: the caret may move, entry continues.
        assert!(matches!(
            unicode_entry_binding(&press(Key::Named(Named::ArrowLeft), Modifiers::empty())),
            None
        ));
    }

    #[test]
    fn entry_collects_commits_and_unravels() {
        let mut app = find_tests::test_app_with_tab("x\n");
        // Not started: hex input is ignored.
        let _ = app.update(Message::UnicodeHex('4'));
        assert_eq!(app.unicode_input, None);

        let _ = app.update(Message::UnicodeStart);
        assert_eq!(app.unicode_input, Some(String::new()));
        // Starting again dismisses any wikilink dropdown.
        app.wiki_complete = Some(crate::app::WikiComplete {
            items: vec!["n".into()],
            selected: 0,
        });
        let _ = app.update(Message::UnicodeStart);
        assert_eq!(app.wiki_complete, None);

        // Digits accumulate; non-hex is ignored; cap at 6 digits.
        for ch in ['2', 'g', '6', 'A'] {
            let _ = app.update(Message::UnicodeHex(ch));
        }
        assert_eq!(app.unicode_input.as_deref(), Some("26a"));
        let filler = "123";
        for ch in filler.chars() {
            let _ = app.update(Message::UnicodeHex(ch));
        }
        assert_eq!(app.unicode_input.as_deref(), Some("26a123"));
        let _ = app.update(Message::UnicodeHex('4'));
        assert_eq!(app.unicode_input.as_deref(), Some("26a123"));

        // Backspace unravels; emptying exits the mode.
        let _ = app.update(Message::UnicodeBackspace);
        assert_eq!(app.unicode_input.as_deref(), Some("26a12"));
        for _ in 0..6 {
            let _ = app.update(Message::UnicodeBackspace);
        }
        assert_eq!(app.unicode_input, None);
    }

    #[test]
    fn commit_inserts_the_codepoint_and_ends_the_mode() {
        let mut app = find_tests::test_app_with_tab("end\n");
        let _ = app.update(Message::UnicodeStart);
        for ch in "1F600".chars() {
            let _ = app.update(Message::UnicodeHex(ch));
        }
        let _ = app.update(Message::UnicodeCommit);
        assert_eq!(app.unicode_input, None);
        assert!(app.tabs[0].content.text().starts_with('\u{1F600}'));

        // An invalid codepoint ends the mode without touching the text.
        let before = app.tabs[0].content.text();
        let _ = app.update(Message::UnicodeStart);
        for ch in "D800".chars() {
            let _ = app.update(Message::UnicodeHex(ch));
        }
        let _ = app.update(Message::UnicodeCommit);
        assert_eq!(app.unicode_input, None);
        assert_eq!(app.tabs[0].content.text(), before);

        // Esc cancels.
        let _ = app.update(Message::UnicodeStart);
        let _ = app.update(Message::UnicodeCancel);
        assert_eq!(app.unicode_input, None);
    }

    #[test]
    fn context_changes_dismiss_the_entry() {
        let mut app = find_tests::test_app_with_tab("x\n");
        let _ = app.update(Message::UnicodeStart);
        // Opening another tab dismisses it (via the shared overlay dismiss).
        app.open_tab("other.md".to_string());
        assert_eq!(app.unicode_input, None);
    }
}

#[cfg(test)]
mod switcher_ranking_tests {
    use super::*;

    /// Write a note with an explicit modification time.
    fn write_note(dir: &std::path::Path, name: &str, modified: std::time::SystemTime) {
        let path = dir.join(name);
        std::fs::write(&path, "body\n").unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[test]
    fn empty_query_lists_the_most_recently_touched_notes_first() {
        let tmp = tempfile::TempDir::new().unwrap();
        let now = std::time::SystemTime::now();
        // Alphabetically "alpha" comes first, but "zulu" is newer: the
        // recent one leads.
        write_note(
            tmp.path(),
            "alpha.md",
            now - std::time::Duration::from_secs(3600),
        );
        write_note(tmp.path(), "zulu.md", now);

        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rerank_switcher();
        assert_eq!(app.switcher.items, vec!["zulu.md", "alpha.md"]);

        // A query switches back to match ranking: the title match
        // ("alpha.md") outranks the folder-only match
        // ("alpha/older.md"), which still shows up below it.
        std::fs::create_dir_all(tmp.path().join("alpha")).unwrap();
        write_note(tmp.path(), "alpha/older.md", now);
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.switcher.query = "alpha".to_string();
        app.rerank_switcher();
        assert_eq!(
            app.switcher.items.first().map(String::as_str),
            Some("alpha.md")
        );
        assert!(
            app.switcher.items.contains(&"alpha/older.md".to_string()),
            "{:?}",
            app.switcher.items
        );
    }
}

#[cfg(test)]
mod editor_focus_tests {
    use super::*;

    /// The editor pane must build with each popup state — and, critically,
    /// the popup layer is always mounted: if the pane switched between a
    /// bare container and a stack when a popup opened, iced would rebuild
    /// the subtree, drop the editor's focus state, and leave it unable to
    /// type until it was clicked again.
    #[test]
    fn view_builds_with_popups_open_and_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("n.md"), "[[no\n").unwrap();
        let mut app = find_tests::test_app_with_tab("[[no\n");
        app.vault = Some(Vault::open(tmp.path()).unwrap());

        // No popup.
        let _ = crate::view::view(&app);
        // Wikilink dropdown open.
        app.wiki_complete = Some(WikiComplete {
            items: vec!["n.md".into()],
            selected: 0,
        });
        let _ = crate::view::view(&app);
        // Unicode entry open (dismisses the dropdown).
        app.wiki_complete = None;
        app.unicode_input = Some("1f600".to_string());
        let _ = crate::view::view(&app);
        // Both closed again.
        app.unicode_input = None;
        let _ = crate::view::view(&app);

        // Pane modes all build too (each mode restructures the note area,
        // which is why the editor's focus rides on its custom id).
        for mode in [
            PaneMode::Live,
            PaneMode::Source,
            PaneMode::Reading,
            PaneMode::Split,
        ] {
            app.pane = mode;
            let _ = crate::view::view(&app);
        }
    }
}

#[cfg(test)]
mod link_target_tests {
    use super::*;
    use cosmic::Application as _;

    fn notes() -> Vec<String> {
        [
            "Glass.md",
            "Notes/Glass.md",
            "Projects/Obsidian/Glass.md",
            "Notes/Rust.md",
            "Journal/2026/11/05.md",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn shortest_is_the_bare_title_unless_titles_collide() {
        let notes = notes();
        // "05" is a unique title: bare stem even deep in folders.
        assert_eq!(
            link_target(
                "Notes/Rust.md",
                "Journal/2026/11/05.md",
                &notes,
                LinkPathFormat::Shortest
            ),
            "05"
        );
    }

    #[test]
    fn shortest_disambiguates_with_the_full_path() {
        let notes = notes();
        // "Glass" exists three times: the full path disambiguates.
        assert_eq!(
            link_target(
                "Notes/Rust.md",
                "Projects/Obsidian/Glass.md",
                &notes,
                LinkPathFormat::Shortest
            ),
            "Projects/Obsidian/Glass"
        );
        // A unique title stays short even when the note lives deep.
        assert_eq!(
            link_target(
                "Glass.md",
                "Notes/Rust.md",
                &notes,
                LinkPathFormat::Shortest
            ),
            "Rust"
        );
    }

    #[test]
    fn relative_paths_step_out_and_down() {
        let notes = notes();
        // From Notes/ to a sibling folder: one step out.
        assert_eq!(
            link_target(
                "Notes/Rust.md",
                "Projects/Obsidian/Glass.md",
                &notes,
                LinkPathFormat::Relative
            ),
            "../Projects/Obsidian/Glass"
        );
        // Same folder: just the stem.
        assert_eq!(
            link_target(
                "Notes/Rust.md",
                "Notes/Glass.md",
                &notes,
                LinkPathFormat::Relative
            ),
            "Glass"
        );
        // From a subfolder up to the root: two steps out.
        assert_eq!(
            link_target(
                "Journal/2026/11/05.md",
                "Glass.md",
                &notes,
                LinkPathFormat::Relative
            ),
            "../../../Glass"
        );
        // From the root down into folders: the plain relative path.
        assert_eq!(
            link_target(
                "Glass.md",
                "Projects/Obsidian/Glass.md",
                &notes,
                LinkPathFormat::Relative
            ),
            "Projects/Obsidian/Glass"
        );
    }

    #[test]
    fn absolute_is_the_full_vault_path() {
        let notes = notes();
        assert_eq!(
            link_target(
                "Notes/Rust.md",
                "Journal/2026/11/05.md",
                &notes,
                LinkPathFormat::Absolute
            ),
            "Journal/2026/11/05"
        );
    }

    #[test]
    fn relative_targets_resolve_back_through_the_vault_index() {
        // Every generated relative link must resolve to the target again.
        let index = VaultIndex::new(notes().iter().cloned());
        let cases = [
            ("Notes/Rust.md", "Projects/Obsidian/Glass.md"),
            ("Notes/Rust.md", "Notes/Glass.md"),
            ("Journal/2026/11/05.md", "Glass.md"),
            ("Glass.md", "Journal/2026/11/05.md"),
        ];
        for (from, target) in cases {
            let link = link_target(from, target, &notes(), LinkPathFormat::Relative);
            assert_eq!(
                index.resolve(&link, from),
                Some(target),
                "link {link:?} from {from:?}"
            );
        }
    }

    #[test]
    fn settings_choice_updates_the_format() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        let _ = app.update(Message::SettingsLinkFormat(LinkPathFormat::Relative));
        assert_eq!(app.link_path_format, LinkPathFormat::Relative);
        // Persisted through load-modify-store.
        assert_eq!(
            crate::settings::load().link_path_format,
            LinkPathFormat::Relative
        );
        let _ = app.update(Message::SettingsLinkFormat(LinkPathFormat::Absolute));
        assert_eq!(
            crate::settings::load().link_path_format,
            LinkPathFormat::Absolute
        );
    }
}

#[cfg(test)]
mod link_update_tests {
    use super::*;
    use cosmic::app::Application as _;

    /// A vault with the given (path, text) files and a rebuilt index.
    /// The TempDir must stay bound in the caller for the files to exist.
    fn app_with(files: &[(&str, &str)]) -> (tempfile::TempDir, VaultNotes) {
        let tmp = tempfile::TempDir::new().unwrap();
        for (path, text) in files {
            if let Some((dir, _)) = path.rsplit_once('/') {
                std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
            }
            std::fs::write(tmp.path().join(path), text).unwrap();
        }
        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        (tmp, app)
    }

    fn disk(tmp: &tempfile::TempDir, path: &str) -> String {
        std::fs::read_to_string(tmp.path().join(path)).unwrap()
    }

    #[test]
    fn renaming_a_note_prompts_then_rewrites_closed_referrers() {
        let (tmp, mut app) = app_with(&[("a.md", "see [[b]] here"), ("b.md", "b")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        // The prompt appears with the referrer named (old path).
        let pending = app.pending_links.as_ref().expect("prompt appears");
        assert_eq!(pending.referrers, vec!["a.md"]);
        assert_eq!(pending.renames, vec![("b.md".into(), "c.md".into())]);
        // Nothing written until confirmed.
        assert_eq!(disk(&tmp, "a.md"), "see [[b]] here");

        let _ = app.update(Message::LinkUpdatesApply);
        assert_eq!(disk(&tmp, "a.md"), "see [[c]] here");
        assert!(app.pending_links.is_none());
    }

    #[test]
    fn rewrites_preserve_headings_aliases_and_embeds() {
        let (tmp, mut app) = app_with(&[
            (
                "a.md",
                "[[b|alias]] and [[b#Head]] and ![[b]] and [[b#^blk]]",
            ),
            ("b.md", "b"),
        ]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        let _ = app.update(Message::LinkUpdatesApply);
        assert_eq!(
            disk(&tmp, "a.md"),
            "[[c|alias]] and [[c#Head]] and ![[c]] and [[c#^blk]]"
        );
    }

    #[test]
    fn dismissing_leaves_the_links_stale() {
        let (tmp, mut app) = app_with(&[("a.md", "see [[b]]"), ("b.md", "b")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        assert!(app.pending_links.is_some());
        let _ = app.update(Message::LinkUpdatesDismiss);
        assert!(app.pending_links.is_none());
        assert_eq!(disk(&tmp, "a.md"), "see [[b]]");
    }

    #[test]
    fn no_prompt_when_nothing_resolves_into_scope() {
        let (_, mut app) = app_with(&[("a.md", "unrelated"), ("b.md", "b")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        assert!(app.pending_links.is_none());
        // An unresolved link is not a referrer either.
        let (_, mut app) = app_with(&[("a.md", "[[missing]]"), ("b.md", "b")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        assert!(app.pending_links.is_none());
        // Markdown links are out of scope for the rewriter: no prompt.
        let (_, mut app) = app_with(&[("a.md", "[text](b.md)"), ("b.md", "b")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        assert!(app.pending_links.is_none());
    }

    #[test]
    fn open_tab_buffers_are_rewritten_not_the_disk_file() {
        let (tmp, mut app) = app_with(&[("a.md", "see [[b]]"), ("b.md", "b")]);
        app.tabs.push(Tab::open("a.md".to_string(), "see [[b]]"));
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        let _ = app.update(Message::LinkUpdatesApply);
        // The buffer is the live truth for an open note.
        assert_eq!(app.tabs[1].content.text(), "see [[c]]");
        // Saving it is the user's normal flow.
        assert_eq!(disk(&tmp, "a.md"), "see [[b]]");
    }

    #[test]
    fn folder_renames_rewrite_links_from_outside_but_not_inside() {
        let (tmp, mut app) = app_with(&[
            ("a.md", "[[old/n]]"),
            ("old/n.md", ""),
            ("old/m.md", "see [[n]]"),
        ]);
        let _ = app.rename_tree_folder("old".into(), "new".into());
        let _ = app.update(Message::LinkUpdatesApply);
        // The outside link's target moved: rewritten (Shortest format:
        // the bare title, unambiguous after the rename).
        assert_eq!(disk(&tmp, "a.md"), "[[n]]");
        // The inside bare link still resolves within the folder: untouched.
        assert_eq!(disk(&tmp, "new/m.md"), "see [[n]]");
    }

    #[test]
    fn self_links_in_the_renamed_note_are_rewritten() {
        let (tmp, mut app) = app_with(&[("b.md", "intro [[b#Head]]")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        let _ = app.update(Message::LinkUpdatesApply);
        assert_eq!(disk(&tmp, "c.md"), "intro [[c#Head]]");
    }

    #[test]
    fn rewrites_follow_the_link_path_format() {
        let (tmp, mut app) = app_with(&[("notes/a.md", "[[b]]"), ("b.md", "b")]);
        app.link_path_format = LinkPathFormat::Relative;
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        let _ = app.update(Message::LinkUpdatesApply);
        assert_eq!(disk(&tmp, "notes/a.md"), "[[../c]]");
    }

    #[test]
    fn edits_between_prompt_and_apply_are_rescanned() {
        let (tmp, mut app) = app_with(&[("a.md", "see [[b]]"), ("b.md", "b")]);
        let _ = app.rename_tree_note("b.md".into(), "c.md".into());
        assert!(app.pending_links.is_some());
        // The referrer changes on disk before the user confirms.
        std::fs::write(tmp.path().join("a.md"), "nothing relevant now").unwrap();
        let _ = app.update(Message::LinkUpdatesApply);
        assert_eq!(disk(&tmp, "a.md"), "nothing relevant now");
    }
}

#[cfg(test)]
mod sidebar_collapse_tests {
    use super::*;
    use cosmic::app::Application as _;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> cosmic::iced::Rectangle {
        cosmic::iced::Rectangle {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn toggling_the_sidebar_flips_state_and_persists() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        assert!(app.sidebar_open, "starts open");
        let _ = app.update(Message::ToggleSidebar);
        assert!(!app.sidebar_open);
        assert!(!crate::settings::load().sidebar_open, "persisted closed");
        let _ = app.update(Message::ToggleSidebar);
        assert!(app.sidebar_open);
        assert!(crate::settings::load().sidebar_open, "persisted open");
        // Leave the developer's settings file as it was.
        persist_setting(|s| s.sidebar_open = true);
    }

    #[test]
    fn the_view_builds_with_the_sidebar_open_and_closed() {
        let mut app = find_tests::test_app_with_tab("");
        let _ = crate::view::view(&app);
        app.sidebar_open = false;
        let _ = crate::view::view(&app);
    }

    #[test]
    fn a_press_does_not_arm_a_drag_while_the_sidebar_is_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "").unwrap();
        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        // Bounds left over from before the sidebar was collapsed: a
        // click inside the old tree area must arm nothing, or a release
        // over a stale target could move files invisibly.
        app.tree_bounds = crate::drag::TreeBounds {
            rows: vec![("a.md".into(), false, rect(0.0, 0.0, 200.0, 24.0))],
            area: Some(rect(0.0, 0.0, 200.0, 300.0)),
        };
        app.sidebar_open = false;
        app.last_cursor = Some(cosmic::iced::Point::new(10.0, 10.0));
        let _ = app.update(Message::MousePressed);
        let _ = app.update(Message::MouseMoved(cosmic::iced::Point::new(10.0, 300.0)));
        let _ = app.update(Message::MouseReleased);
        assert!(tmp.path().join("a.md").exists(), "nothing moved");
        assert!(app.tree_drag.is_none(), "never armed");
    }

    #[test]
    fn bounds_are_not_measured_while_the_sidebar_is_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "").unwrap();
        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        app.panel = SidePanel::Files;
        app.sidebar_open = false;
        let _ = app.measure_tree_bounds_if_needed();
        assert!(!app.bounds_pending, "no measure while the tree is hidden");
    }
}

#[cfg(test)]
mod editor_font_tests {
    use super::*;
    use cosmic::app::Application as _;

    #[test]
    fn the_custom_font_family_persists_from_the_settings_input() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        let _ = app.update(Message::SettingsEditorFontFamily("Fira Code".into()));
        assert_eq!(app.editor_font_family, "Fira Code");
        assert_eq!(crate::settings::load().editor_font_family, "Fira Code");
        // Clearing it returns to the monospace toggle's behavior.
        let _ = app.update(Message::SettingsEditorFontFamily(String::new()));
        assert!(app.editor_font_family.is_empty());
        assert!(crate::settings::load().editor_font_family.is_empty());
    }

    #[test]
    fn the_reading_view_has_its_own_family_and_size() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        let before = crate::settings::load();
        let (family, size) = (before.reading_font_family, before.reading_font_size);
        let reading = app.reading_font_size;

        let _ = app.update(Message::SettingsReadingFontFamily("Iowan Old Style".into()));
        assert_eq!(app.reading_font_family, "Iowan Old Style");
        assert_eq!(
            crate::settings::load().reading_font_family,
            "Iowan Old Style"
        );
        // The source editor's own font is untouched by the reading one.
        assert_eq!(app.editor_font_family, "");

        let _ = app.update(Message::SettingsReadingFontSize(3));
        assert_eq!(
            app.reading_font_size,
            crate::settings::clamp_font_size(reading as i32 + 3)
        );
        assert_eq!(app.editor_font_size, 14, "the source size is its own");

        persist_setting(move |s| {
            s.reading_font_family = family;
            s.reading_font_size = size;
        });
    }

    #[test]
    fn font_sizes_take_any_value_inside_the_range() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        let before = crate::settings::load();
        let (editor, reading) = (before.editor_font_size, before.reading_font_size);
        let (editor_size, reading_size) = (app.editor_font_size, app.reading_font_size);

        // One pixel at a time: the sizes are arbitrary, not stepped.
        let _ = app.update(Message::SettingsEditorFontSize(1));
        assert_eq!(
            app.editor_font_size,
            crate::settings::clamp_font_size(editor_size as i32 + 1)
        );
        let _ = app.update(Message::SettingsReadingFontSize(-1));
        assert_eq!(
            app.reading_font_size,
            crate::settings::clamp_font_size(reading_size as i32 - 1)
        );

        // The ends are held, so a runaway shortcut cannot make the text
        // vanish.
        let _ = app.update(Message::SettingsEditorFontSize(1000));
        assert_eq!(app.editor_font_size, crate::settings::MAX_FONT_SIZE);
        let _ = app.update(Message::SettingsReadingFontSize(-1000));
        assert_eq!(app.reading_font_size, crate::settings::MIN_FONT_SIZE);

        persist_setting(move |s| {
            s.editor_font_size = editor;
            s.reading_font_size = reading;
        });
    }

    #[test]
    fn the_reading_margin_persists_and_widens_the_gutter() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        let before = crate::settings::load().reading_margin;

        let _ = app.update(Message::SettingsReadingMargin(
            crate::settings::ReadingMargin::Spacious,
        ));
        assert_eq!(app.reading_margin, crate::settings::ReadingMargin::Spacious);
        assert_eq!(
            crate::settings::load().reading_margin,
            crate::settings::ReadingMargin::Spacious
        );
        // A wider margin leaves a wider gutter.
        assert!(
            crate::settings::ReadingMargin::Spacious.padding()
                > crate::settings::ReadingMargin::Compact.padding()
        );

        persist_setting(move |s| s.reading_margin = before);
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;
    use cosmic::app::Application as _;

    /// A key event that only records the modifiers held (the global
    /// handler ignores the key itself).
    fn press_with(modifiers: Modifiers) -> Message {
        Message::KeyPressed(
            cosmic::iced::event::Status::Ignored,
            modifiers,
            Key::Character("z".into()),
        )
    }

    /// The zoom shortcuts persist the size they change, so the tests
    /// take the settings lock and put the stored sizes back.
    fn restore_sizes() {
        let before = crate::settings::load();
        let (editor, reading) = (before.editor_font_size, before.reading_font_size);
        persist_setting(move |s| {
            s.editor_font_size = editor;
            s.reading_font_size = reading;
        });
    }

    #[test]
    fn the_zoom_shortcut_follows_the_pane_in_focus() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        // In the source pane the editor's size moves.
        app.pane = PaneMode::Source;
        let editor = app.editor_font_size;
        let reading = app.reading_font_size;
        let _ = app.update(Message::ZoomIn);
        assert_eq!(app.editor_font_size, editor + 1);
        assert_eq!(
            app.reading_font_size, reading,
            "the reading size is its own"
        );

        // In the reading pane the reading size moves instead.
        app.pane = PaneMode::Reading;
        let _ = app.update(Message::ZoomIn);
        assert_eq!(app.reading_font_size, reading + 1);
        assert_eq!(app.editor_font_size, editor + 1, "the editor size stays");

        restore_sizes();
    }

    #[test]
    fn the_split_pane_zooms_the_pane_that_was_touched_last() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        app.pane = PaneMode::Split;
        // The editor is the target until the reading view is touched.
        let _ = app.update(Message::ZoomIn);
        assert_eq!(app.editor_font_size, 15);
        assert_eq!(app.reading_font_size, 16);

        // Clicking the reading view moves the target there.
        let _ = app.update(Message::ReadingClicked);
        let _ = app.update(Message::ZoomIn);
        assert_eq!(app.reading_font_size, 17);
        assert_eq!(app.editor_font_size, 15);

        restore_sizes();
    }

    #[test]
    fn ctrl_scroll_zooms_and_a_plain_wheel_does_not() {
        let _guard = settings_guard();
        let mut app = find_tests::test_app_with_tab("");
        app.pane = PaneMode::Reading;

        // A wheel without Ctrl belongs to the scrollable under the
        // pointer: nothing is zoomed.
        let _ = app.update(Message::MouseWheel(1.0));
        assert_eq!(app.reading_font_size, 16);

        // With Ctrl held, scrolling up zooms in and down zooms out.
        let _ = app.update(press_with(Modifiers::CTRL));
        let _ = app.update(Message::MouseWheel(1.0));
        assert_eq!(app.reading_font_size, 17);
        let _ = app.update(Message::MouseWheel(-1.0));
        assert_eq!(app.reading_font_size, 16);

        // Letting go of Ctrl ends the zoom: the wheel belongs to the
        // scrollable again.
        let _ = app.update(Message::KeyReleased(Key::Named(Named::Control)));
        let _ = app.update(Message::MouseWheel(1.0));
        assert_eq!(app.reading_font_size, 16);

        restore_sizes();
    }

    #[test]
    fn a_narrow_window_folds_the_split_pane_away() {
        let mut app = find_tests::test_app_with_tab("");
        app.pane = PaneMode::Split;

        // Below the threshold two panes do not fit: the split view
        // collapses to the source pane.
        let _ = app.update(Message::WindowResized(NARROW_WINDOW_WIDTH - 1.0));
        assert_eq!(app.pane, PaneMode::Source);

        // Wide enough again: the split layout comes back.
        let _ = app.update(Message::WindowResized(NARROW_WINDOW_WIDTH + 1.0));
        assert_eq!(app.pane, PaneMode::Split);
    }

    #[test]
    fn a_pane_the_user_picks_is_not_overridden_by_the_window_width() {
        let mut app = find_tests::test_app_with_tab("");
        app.pane = PaneMode::Split;
        let _ = app.update(Message::WindowResized(NARROW_WINDOW_WIDTH - 1.0));
        assert_eq!(app.pane, PaneMode::Source);

        // The user asks for the reading view while the window is narrow.
        let reading = app
            .pane_model
            .iter()
            .find(|id| app.pane_model.data::<PaneMode>(*id) == Some(&PaneMode::Reading))
            .expect("the reading pane button exists");
        let _ = app.update(Message::PaneActivated(reading));
        assert_eq!(app.pane, PaneMode::Reading);

        // Widening the window must not undo that choice.
        let _ = app.update(Message::WindowResized(NARROW_WINDOW_WIDTH + 1.0));
        assert_eq!(app.pane, PaneMode::Reading);
    }
}

#[cfg(test)]
mod tree_drag_tests {
    use super::*;
    use cosmic::app::Application as _;

    fn app_with(files: &[(&str, &str)]) -> (tempfile::TempDir, VaultNotes) {
        let tmp = tempfile::TempDir::new().unwrap();
        for (path, text) in files {
            if let Some((dir, _)) = path.rsplit_once('/') {
                std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
            }
            std::fs::write(tmp.path().join(path), text).unwrap();
        }
        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        (tmp, app)
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> cosmic::iced::Rectangle {
        cosmic::iced::Rectangle {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Bounds matching a three-row tree: folder "old", then notes "a.md"
    /// and "old/n.md", inside a 200x300 tree area.
    fn bounds() -> crate::drag::TreeBounds {
        crate::drag::TreeBounds {
            rows: vec![
                ("old".into(), true, rect(0.0, 0.0, 200.0, 24.0)),
                ("a.md".into(), false, rect(0.0, 24.0, 200.0, 24.0)),
                ("old/n.md".into(), false, rect(0.0, 48.0, 200.0, 24.0)),
            ],
            area: Some(rect(0.0, 0.0, 200.0, 300.0)),
        }
    }

    /// Simulate a press-move-release drag between window positions.
    fn drag(app: &mut VaultNotes, from: (f32, f32), to: (f32, f32)) {
        app.last_cursor = Some(cosmic::iced::Point::new(from.0, from.1));
        let _ = app.update(Message::MousePressed);
        let _ = app.update(Message::MouseMoved(cosmic::iced::Point::new(to.0, to.1)));
        let _ = app.update(Message::MouseReleased);
    }

    #[test]
    fn dragging_a_note_into_a_folder_moves_the_file() {
        let (tmp, mut app) = app_with(&[("a.md", ""), ("old/n.md", "")]);
        app.tree_bounds = bounds();
        // a.md (row at y=24..48) onto the "old" folder row (y=0..24).
        drag(&mut app, (10.0, 30.0), (10.0, 8.0));
        assert!(tmp.path().join("old/a.md").exists(), "note moved");
        assert!(!tmp.path().join("a.md").exists(), "old path gone");
        // The drag state is fully cleared.
        assert!(app.tree_drag.is_none());
    }

    #[test]
    fn dragging_a_note_to_the_tree_area_moves_it_to_the_root() {
        let (tmp, mut app) = app_with(&[("old/n.md", "")]);
        app.tree_bounds = bounds();
        // old/n.md (row at y=48..72) onto empty tree space below (root).
        drag(&mut app, (10.0, 55.0), (10.0, 200.0));
        assert!(tmp.path().join("n.md").exists(), "moved to root");
        assert!(!tmp.path().join("old/n.md").exists());
    }

    #[test]
    fn dragging_outside_the_tree_cancels() {
        let (tmp, mut app) = app_with(&[("a.md", "")]);
        app.tree_bounds = bounds();
        // Below the tree area: no hover target, no move.
        drag(&mut app, (10.0, 30.0), (10.0, 400.0));
        assert!(tmp.path().join("a.md").exists(), "unchanged");
        assert!(app.tree_drag.is_none());
    }

    #[test]
    fn releasing_over_the_source_row_is_a_click_not_a_move() {
        let (tmp, mut app) = app_with(&[("a.md", ""), ("old/n.md", "")]);
        app.tree_bounds = bounds();
        // Past the threshold but still inside the source row's bounds.
        drag(&mut app, (10.0, 30.0), (60.0, 36.0));
        assert!(tmp.path().join("a.md").exists(), "stayed put");
    }

    #[test]
    fn a_folder_cannot_be_dropped_into_itself() {
        let (tmp, mut app) = app_with(&[("old/n.md", "")]);
        app.tree_bounds = bounds();
        // The "old" folder row onto itself: refused.
        drag(&mut app, (10.0, 8.0), (30.0, 12.0));
        assert!(tmp.path().join("old/n.md").exists(), "unchanged");
        assert!(!tmp.path().join("old/old").exists());
    }

    #[test]
    fn a_plain_click_arms_nothing_and_moves_nothing() {
        let (tmp, mut app) = app_with(&[("a.md", "")]);
        app.tree_bounds = bounds();
        // Press and release at the same spot: no movement.
        app.last_cursor = Some(cosmic::iced::Point::new(10.0, 30.0));
        let _ = app.update(Message::MousePressed);
        let _ = app.update(Message::MouseReleased);
        assert!(tmp.path().join("a.md").exists());
        assert!(app.tree_drag.is_none());
    }

    #[test]
    fn dragging_prompts_for_link_updates_like_a_rename() {
        let (_tmp, mut app) = app_with(&[("a.md", "see [[n]]"), ("old/n.md", "n")]);
        app.tree_bounds = bounds();
        // old/n.md (row at y=48..72) onto the "old" folder row is a
        // no-op; drop it on a.md's row's folder (root) instead.
        app.tree_bounds.rows[2] = ("old/n.md".into(), false, rect(0.0, 48.0, 200.0, 24.0));
        drag(&mut app, (10.0, 55.0), (10.0, 200.0));
        let pending = app.pending_links.as_ref().expect("link prompt appears");
        assert_eq!(pending.referrers, vec!["a.md"]);
    }

    #[test]
    fn arming_waits_for_fresh_bounds_before_giving_up() {
        let (tmp, mut app) = app_with(&[("a.md", "")]);
        app.tree_bounds = bounds();
        // Press on empty tree space below every row: with the tree just
        // scrolled, the bounds on hand may predate the scroll and miss
        // rows the press actually landed on — so a miss is not yet
        // conclusive while fresh bounds are in flight.
        app.last_cursor = Some(cosmic::iced::Point::new(10.0, 200.0));
        let _ = app.update(Message::MousePressed);
        assert!(app.tree_drag.is_some(), "armed");
        assert!(app.bounds_pending, "fresh bounds queued");

        // Movement before the fresh bounds arrive: still armed, still
        // not a drag (the source row is not yet identified).
        let _ = app.update(Message::MouseMoved(cosmic::iced::Point::new(20.0, 210.0)));
        assert!(
            app.tree_drag.as_ref().is_some_and(|drag| !drag.active),
            "stays armed while bounds are pending"
        );

        // Fresh bounds arrive and confirm the press hit no row: the
        // next movement disarms the candidate.
        let _ = app.update(Message::TreeBounds(bounds()));
        let _ = app.update(Message::MouseMoved(cosmic::iced::Point::new(25.0, 215.0)));
        assert!(
            app.tree_drag.is_none(),
            "disarmed once the miss is confirmed"
        );
        assert!(tmp.path().join("a.md").exists(), "nothing moved");
    }

    #[test]
    fn bounds_are_measured_only_while_the_files_panel_is_open() {
        let (_tmp, mut app) = app_with(&[("a.md", "")]);
        // Another panel: the tree is not built there, so a measurement
        // would come back empty and wipe the bounds a later press needs.
        app.panel = SidePanel::Search;
        let _ = app.measure_tree_bounds_if_needed();
        assert!(!app.bounds_pending, "no measure queued off the Files panel");

        // On the Files panel a measure is queued and marked pending.
        app.panel = SidePanel::Files;
        let _ = app.measure_tree_bounds_if_needed();
        assert!(app.bounds_pending, "measure queued");

        // One in flight is enough: no second queue.
        let _ = app.measure_tree_bounds_if_needed();
        assert!(app.bounds_pending);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use cosmic::iced::core::SmolStr;
    use cosmic::iced::keyboard::{Key, Modifiers};
    use cosmic::Application as _;

    /// Move the live caret (no selection) inside the focused region.
    fn aim(app: &mut VaultNotes, line: usize, column: usize) {
        let live = app.live.as_mut().expect("live region open");
        live.editor.move_to(Cursor {
            position: Position { line, column },
            selection: None,
        });
    }

    fn caret(app: &VaultNotes) -> (usize, usize) {
        let editor = &app.live.as_ref().expect("live region open").editor;
        let cursor = editor.cursor().position;
        (cursor.line, cursor.column)
    }

    #[test]
    fn typing_in_live_mode_splices_into_the_note() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(0);
        // Caret to the region's start; the buffer is the region's source,
        // trailing blank line included.
        assert_eq!(
            app.live.as_ref().unwrap().editor.text(),
            "alpha\n",
            "the buffer is the block source"
        );
        aim(&mut app, 0, 0);
        app.update(Message::EditorAction(Action::Edit(Edit::Insert('X'))));
        // The keystroke spliced back into the note and the buffer kept the
        // caret: the region source is re-tiled identically.
        assert_eq!(app.tabs[0].content.text(), "Xalpha\n\nbeta");
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "Xalpha\n");
        assert_eq!(app.live.as_ref().unwrap().block, 0);
        assert_eq!(caret(&app), (0, 1));
        // The edit is real: dirty, undoable, autosave-timed.
        assert!(app.tabs[0].is_dirty());
        assert!(app.last_edit.is_some());
    }

    #[test]
    fn live_format_wraps_the_regions_selection() {
        let mut app = find_tests::test_app_with_tab("alpha here\n\nbeta");
        app.live_focus(0);
        // Select "here" inside the focused region.
        let live = app.live.as_mut().unwrap();
        live.editor.move_to(Cursor {
            position: Position { line: 0, column: 6 },
            selection: Some(Position {
                line: 0,
                column: 10,
            }),
        });
        app.update(Message::LiveFormat(Format::Bold));
        // The markers went into the region buffer and were spliced back
        // into the note.
        assert_eq!(app.tabs[0].content.text(), "alpha **here**\n\nbeta");
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "alpha **here**\n");
    }

    #[test]
    fn live_format_without_a_selection_inserts_empty_markers() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(0);
        aim(&mut app, 0, 5);
        app.update(Message::LiveFormat(Format::Bold));
        // The same convention as the source editor: an empty bold is the
        // marker pair `****` with the caret in the middle.
        assert_eq!(app.tabs[0].content.text(), "alpha****\n\nbeta");
        assert_eq!(caret(&app), (0, 7));
    }

    #[test]
    fn live_backspace_at_region_start_joins_line_by_line() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta\n\ngamma");
        app.live_focus(1);
        aim(&mut app, 0, 0);
        app.update(Message::LiveBackspace);
        // One newline is deleted at file level: "alpha" merges onto the
        // "beta" line's region, keeping the blank after it; the blocks
        // collapse into one soft-broken paragraph region.
        assert_eq!(app.tabs[0].content.text(), "alpha\nbeta\n\ngamma");
        let live = app.live.as_ref().unwrap();
        assert_eq!(live.editor.text(), "alpha\nbeta\n");
        // The caret stays where "beta" began (line 1, column 0).
        assert_eq!(caret(&app), (1, 0));
    }

    #[test]
    fn live_ctrl_backspace_deletes_the_word_before_the_caret() {
        let mut app = find_tests::test_app_with_tab("alpha beta\n\ngamma");
        app.live_focus(0);
        // The caret at the end of "alpha beta".
        aim(&mut app, 0, 10);
        app.update(Message::LiveDeleteWord(false));
        // The word goes, the rest of the line stays.
        assert_eq!(app.tabs[0].content.text(), "alpha \n\ngamma");
        // The caret lands where the word began.
        assert_eq!(caret(&app), (0, 6));
    }

    #[test]
    fn live_ctrl_backspace_at_a_region_start_reaches_the_line_before() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta\n\ngamma");
        app.live_focus(1);
        // The caret at the region's start: the word before it lives in the
        // region above, so the motion runs over the whole note.
        aim(&mut app, 0, 0);
        app.update(Message::LiveDeleteWord(false));
        // The line break before the region goes, exactly like a plain
        // backspace there: the blank line above "beta" is gone and the
        // two paragraphs become one.
        assert_eq!(app.tabs[0].content.text(), "alpha\nbeta\n\ngamma");
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "alpha\nbeta\n");
        // The caret stays where "beta" began.
        assert_eq!(caret(&app), (1, 0));
    }

    #[test]
    fn live_ctrl_delete_deletes_the_word_after_the_caret() {
        let mut app = find_tests::test_app_with_tab("alpha beta\n\ngamma");
        app.live_focus(0);
        // The caret right after "alpha".
        aim(&mut app, 0, 5);
        app.update(Message::LiveDeleteWord(true));
        // The word and the space before it go.
        assert_eq!(app.tabs[0].content.text(), "alpha\n\ngamma");
        assert_eq!(caret(&app), (0, 5));
    }

    #[test]
    fn live_ctrl_delete_at_a_region_end_reaches_the_next_region() {
        let mut app = find_tests::test_app_with_tab("alpha beta\n\ngamma");
        app.live_focus(0);
        // The caret at the very end of the region's buffer: what follows
        // lives in the next region.
        aim(&mut app, 1, 0);
        app.update(Message::LiveDeleteWord(true));
        // The line break after the region goes: the blank line between
        // the two blocks is gone.
        assert_eq!(app.tabs[0].content.text(), "alpha beta\ngamma");
    }

    #[test]
    fn live_backspace_inside_a_region_is_a_plain_backspace() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(1);
        aim(&mut app, 0, 1);
        app.update(Message::LiveBackspace);
        assert_eq!(app.tabs[0].content.text(), "alpha\n\neta");
        assert_eq!(caret(&app), (0, 0));
    }

    #[test]
    fn live_delete_at_region_end_joins_with_the_region_below() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta\n\ngamma");
        app.live_focus(0);
        // live_focus leaves the caret at the region's content end — the
        // join point after "alpha".
        assert_eq!(caret(&app), (0, 5));
        app.update(Message::LiveDelete);
        assert_eq!(app.tabs[0].content.text(), "alpha\nbeta\n\ngamma");
        // The caret is unmoved: still right after "alpha".
        assert_eq!(caret(&app), (0, 5));
    }

    #[test]
    fn live_undo_restores_the_note_and_reaims_the_caret() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(0);
        aim(&mut app, 0, 0);
        app.update(Message::EditorAction(Action::Edit(Edit::Insert('X'))));
        assert_eq!(app.tabs[0].content.text(), "Xalpha\n\nbeta");
        // Undo: the mode-agnostic task snapshots the live caret (0,1),
        // restores the pre-edit text, then re-aims the re-tiled region.
        let before = caret(&app);
        app.update(Message::EditorUndo);
        assert_eq!(app.tabs[0].content.text(), "alpha\n\nbeta");
        assert_eq!(
            caret(&app),
            (before.0, before.1),
            "the caret lands back where the edit happened"
        );
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "alpha\n");
    }

    #[test]
    fn live_escape_dismisses_and_blocks_reappear() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(0);
        assert!(app.live.is_some());
        app.update(Message::LiveDismiss);
        assert!(app.live.is_none());
    }

    #[test]
    fn an_empty_note_opens_its_body_region_for_typing() {
        let mut app = find_tests::test_app_with_tab("");
        // No blocks at all: the note is one body region, auto-focused on
        // open (ensure_live_focus) so the user can just start writing.
        assert!(app.tabs[0].doc.blocks.is_empty());
        let _ = app.ensure_live_focus();
        let live = app.live.as_ref().expect("the body region is open");
        assert_eq!(live.block, 0);
        assert_eq!(live.editor.text(), "");
        assert_eq!(caret(&app), (0, 0));
    }

    #[test]
    fn live_boundaries_cross_into_neighboring_regions() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(1);
        // Up from the region's top enters the region above at its
        // content end (the caret lands after "alpha").
        app.update(Message::LiveUp);
        assert_eq!(app.live.as_ref().unwrap().block, 0);
        assert_eq!(
            app.tabs[0].content.text(),
            "alpha\n\nbeta",
            "navigation does not edit"
        );
        // Down from region 0's caret at its content end: first Down
        // moves onto the region's trailing blank line, the second one
        // crosses into region 1 (the column is preserved).
        let live = app.live.as_mut().unwrap();
        live.editor.move_to(Cursor {
            position: Position { line: 0, column: 5 },
            selection: None,
        });
        app.update(Message::LiveDown);
        app.update(Message::LiveDown);
        assert_eq!(app.live.as_ref().unwrap().block, 1);
        assert_eq!(caret(&app), (0, 0), "the column is preserved");
    }

    #[test]
    fn pane_switch_drops_the_open_region() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(0);
        assert!(app.live.is_some());
        // The Source segment of the pane model, by entity id.
        let source_entity = app
            .pane_model
            .iter()
            .find(|id| app.pane_model.data::<PaneMode>(*id) == Some(&PaneMode::Source))
            .unwrap();
        app.update(Message::PaneActivated(source_entity));
        assert_eq!(app.pane, PaneMode::Source);
        assert!(app.live.is_none());
    }

    fn press(key: Key, modifiers: Modifiers) -> text_editor::KeyPress {
        press_text(key, modifiers, None)
    }

    fn press_text(key: Key, modifiers: Modifiers, text: Option<&str>) -> text_editor::KeyPress {
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: cosmic::iced::keyboard::key::Physical::Unidentified(
                cosmic::iced::keyboard::key::NativeCode::Unidentified,
            ),
            modifiers,
            text: text.map(SmolStr::new),
            status: text_editor::Status::Focused { is_hovered: false },
        }
    }

    #[test]
    fn key_bindings_route_the_live_edits() {
        let plain = Modifiers::default();
        // Boundary keys route to the app, not the widget.
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::ArrowUp), plain), false),
            Some(text_editor::Binding::Custom(Message::LiveUp))
        ));
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::ArrowDown), plain), false),
            Some(text_editor::Binding::Custom(Message::LiveDown))
        ));
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::Backspace), plain), false),
            Some(text_editor::Binding::Custom(Message::LiveBackspace))
        ));
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::Delete), plain), false),
            Some(text_editor::Binding::Custom(Message::LiveDelete))
        ));
        // Ctrl+Backspace / Ctrl+Delete delete a word, not one character.
        let ctrl = Modifiers::CTRL;
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::Backspace), ctrl), false),
            Some(text_editor::Binding::Custom(Message::LiveDeleteWord(false)))
        ));
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::Delete), ctrl), false),
            Some(text_editor::Binding::Custom(Message::LiveDeleteWord(true)))
        ));
        // Escape commits the region instead of unfocusing.
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::Escape), plain), false),
            Some(text_editor::Binding::Custom(Message::LiveDismiss))
        ));
        // With the wiki dropdown open, Escape dismisses the dropdown.
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::Escape), plain), true),
            Some(text_editor::Binding::Custom(Message::WikiDismiss))
        ));
        // Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y stay app-scoped undo/redo.
        let ctrl = Modifiers::CTRL;
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Character(SmolStr::new("z")), ctrl), false),
            Some(text_editor::Binding::Custom(Message::EditorUndo))
        ));
        assert!(matches!(
            crate::view::live_key_binding(
                press(Key::Character(SmolStr::new("z")), ctrl | Modifiers::SHIFT),
                false
            ),
            Some(text_editor::Binding::Custom(Message::EditorRedo))
        ));
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Character(SmolStr::new("y")), ctrl), false),
            Some(text_editor::Binding::Custom(Message::EditorRedo))
        ));
        // Plain typing keeps the editor's own Insert path: never intercepted.
        assert!(matches!(
            crate::view::live_key_binding(
                press_text(Key::Character(SmolStr::new("q")), plain, Some("q")),
                false
            ),
            Some(text_editor::Binding::Insert('q'))
        ));
        // Typed brackets/quotes route through the auto-pair table.
        assert!(matches!(
            crate::view::live_key_binding(
                press_text(Key::Character(SmolStr::new("(")), plain, Some("(")),
                false
            ),
            Some(text_editor::Binding::Custom(Message::EditorAutoPair('(')))
        ));
        assert!(matches!(
            crate::view::live_key_binding(
                press_text(Key::Character(SmolStr::new("\"")), plain, Some("\"")),
                false
            ),
            Some(text_editor::Binding::Custom(Message::EditorAutoPair('"')))
        ));
        // Shifted arrows fall through to the widget's select-motion path.
        assert!(matches!(
            crate::view::live_key_binding(
                press(Key::Named(Named::ArrowDown), Modifiers::SHIFT),
                false
            ),
            Some(text_editor::Binding::Select(_))
        ));
        // While unfocused, nothing is bound (mirrors editor_shortcut).
        assert!(matches!(
            crate::view::live_key_binding(
                text_editor::KeyPress {
                    key: Key::Named(Named::Escape),
                    modified_key: Key::Named(Named::Escape),
                    physical_key: cosmic::iced::keyboard::key::Physical::Unidentified(
                        cosmic::iced::keyboard::key::NativeCode::Unidentified
                    ),
                    modifiers: Modifiers::default(),
                    text: None,
                    status: text_editor::Status::Active,
                },
                false
            ),
            None
        ));
    }

    #[test]
    fn the_arrows_walk_the_region_before_crossing_into_the_next_one() {
        // A paragraph of two lines: the caret walks the region's own
        // lines first. (The editor's motion is visual-line aware, so a
        // wrapped line counts as the several rows it is drawn as.)
        let mut app = find_tests::test_app_with_tab("alpha\nbeta\n\ngamma");
        app.live_focus(0);
        aim(&mut app, 0, 0);
        app.update(Message::LiveDown);
        assert_eq!(
            app.live.as_ref().unwrap().block,
            0,
            "the caret stays inside the region while it can move"
        );
        assert_eq!(caret(&app), (1, 0));
        // The region's trailing blank line is still inside the region.
        app.update(Message::LiveDown);
        assert_eq!(app.live.as_ref().unwrap().block, 0);
        assert_eq!(caret(&app), (2, 0));
        // The next Down has nowhere left to go inside the region: it
        // crosses into the next one.
        app.update(Message::LiveDown);
        assert_eq!(app.live.as_ref().unwrap().block, 1);
    }

    #[test]
    fn the_wiki_dropdown_owns_the_arrows_while_it_is_open() {
        let plain = Modifiers::default();
        // The live editor: the arrows walk the suggestions instead of
        // moving the caret between blocks.
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::ArrowUp), plain), true),
            Some(text_editor::Binding::Custom(Message::WikiCycle(-1)))
        ));
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::ArrowDown), plain), true),
            Some(text_editor::Binding::Custom(Message::WikiCycle(1)))
        ));
        // Without the dropdown, the arrows keep navigating the regions.
        assert!(matches!(
            crate::view::live_key_binding(press(Key::Named(Named::ArrowUp), plain), false),
            Some(text_editor::Binding::Custom(Message::LiveUp))
        ));
        // The source editor: same, and the arrows leave the caret alone.
        assert!(matches!(
            crate::view::editor_key_binding(press(Key::Named(Named::ArrowUp), plain), true, false),
            Some(text_editor::Binding::Custom(Message::WikiCycle(-1)))
        ));
        assert!(matches!(
            crate::view::editor_key_binding(
                press(Key::Named(Named::ArrowDown), plain),
                true,
                false
            ),
            Some(text_editor::Binding::Custom(Message::WikiCycle(1)))
        ));
        // Escape still dismisses the dropdown in both editors.
        assert!(matches!(
            crate::view::editor_key_binding(press(Key::Named(Named::Escape), plain), true, false),
            Some(text_editor::Binding::Custom(Message::WikiDismiss))
        ));
        // Shift+Arrow still extends the selection instead.
        assert!(matches!(
            crate::view::live_key_binding(
                press(Key::Named(Named::ArrowDown), Modifiers::SHIFT),
                true
            ),
            Some(text_editor::Binding::Select(_))
        ));
    }

    #[test]
    fn the_arrows_walk_the_suggestions() {
        let mut app = find_tests::test_app_with_tab("see [[no\n");
        app.wiki_complete = Some(WikiComplete {
            items: vec!["Alpha.md".to_string(), "Beta.md".to_string()],
            selected: 0,
        });
        app.update(Message::WikiCycle(1));
        assert_eq!(app.wiki_complete.as_ref().unwrap().selected, 1);
        app.update(Message::WikiCycle(-1));
        assert_eq!(app.wiki_complete.as_ref().unwrap().selected, 0);
        // The wrap-around keeps the highlight in range.
        app.update(Message::WikiCycle(-1));
        assert_eq!(app.wiki_complete.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn typing_a_pair_in_live_mode_splices_the_pair() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.live_focus(0);
        aim(&mut app, 0, 0);
        app.update(Message::EditorAutoPair('('));
        // The pair went into the region buffer and lifted into the note.
        assert_eq!(app.tabs[0].content.text(), "()alpha\n\nbeta");
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "()alpha\n");
        // The caret sits between the markers.
        assert_eq!(caret(&app), (0, 1));
    }

    #[test]
    fn typing_the_closer_steps_over_instead_of_duplicating() {
        let mut app = find_tests::test_app_with_tab("ab\n");
        app.live_focus(0);
        aim(&mut app, 0, 0);
        app.update(Message::EditorAutoPair('['));
        // "[" paired: "[]ab" with the caret between the markers.
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "[]ab\n");
        assert_eq!(caret(&app), (0, 1));
        // Typing "]" now steps over the closer.
        app.update(Message::EditorAutoPair(']'));
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "[]ab\n");
        assert_eq!(caret(&app), (0, 2));
    }

    #[test]
    fn auto_pair_off_still_types_the_bracket() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta");
        app.auto_pair = false;
        app.live_focus(0);
        aim(&mut app, 0, 0);
        app.update(Message::EditorAutoPair('('));
        // A single plain character lands — no pair, no swallowed key.
        assert_eq!(app.tabs[0].content.text(), "(alpha\n\nbeta");
        assert_eq!(caret(&app), (0, 1));
    }

    #[test]
    fn wiki_completion_opens_from_the_live_region() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Alpha.md"), "alpha note\n").unwrap();
        let mut app = find_tests::test_app_with_tab("see [[al\n");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        app.live_focus(0);
        // live_focus leaves the caret after "[[al".
        assert_eq!(caret(&app), (0, 8));
        app.update_wiki_complete();
        let wiki = app.wiki_complete.as_ref().expect("the dropdown is open");
        assert_eq!(wiki.items, vec!["Alpha.md".to_string()]);
    }

    #[test]
    fn wiki_accept_in_live_mode_splices_the_link() {
        let mut app = find_tests::test_app_with_tab("see [[al\n");
        app.live_focus(0);
        app.wiki_complete = Some(WikiComplete {
            items: vec!["Alpha.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        // Shortest format, unique title: the bare title.
        assert_eq!(app.tabs[0].content.text(), "see [[Alpha]]\n");
        // The buffer is the region source: trailing blank line included.
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "see [[Alpha]]\n");
        // The link closed the dropdown.
        assert!(app.wiki_complete.is_none());
        // One undo step restores the partial link.
        assert!(undo_tab(&mut app.tabs[0]));
        assert_eq!(app.tabs[0].content.text(), "see [[al\n");
    }

    #[test]
    fn wiki_accept_in_live_mode_consumes_the_auto_paired_brackets() {
        // Same as the source path, in the region's buffer: the "]]" the
        // auto-pair left behind the caret goes with the partial link.
        let mut app = find_tests::test_app_with_tab("see [[My Link]]\n");
        app.live_focus(0);
        // The caret sits right after "My Link", in front of the "]]".
        let live = app.live.as_mut().unwrap();
        live.editor.move_to(Cursor {
            position: Position {
                line: 0,
                column: 13,
            },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["My Link.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        assert_eq!(app.tabs[0].content.text(), "see [[My Link]]\n");
    }

    #[test]
    fn wiki_accept_in_live_mode_keeps_the_custom_label() {
        let mut app = find_tests::test_app_with_tab("see [[|]]\n");
        app.live_focus(0);
        // The caret sits in front of the bar.
        let live = app.live.as_mut().unwrap();
        live.editor.move_to(Cursor {
            position: Position { line: 0, column: 6 },
            selection: None,
        });
        app.wiki_complete = Some(WikiComplete {
            items: vec!["Alpha.md".to_string()],
            selected: 0,
        });
        app.wiki_accept();
        assert_eq!(app.tabs[0].content.text(), "see [[Alpha|]]\n");
        // The caret sits right after the bar.
        assert_eq!(caret(&app), (0, 12));
    }

    #[test]
    fn auto_pair_keeps_a_wrapped_selection_in_the_live_region() {
        let mut app = find_tests::test_app_with_tab("My Link\n");
        app.live_focus(0);
        // Select "My Link" in the region.
        let live = app.live.as_mut().unwrap();
        live.editor.move_to(Cursor {
            position: Position { line: 0, column: 7 },
            selection: Some(Position { line: 0, column: 0 }),
        });
        app.update(Message::EditorAutoPair('['));
        assert_eq!(app.tabs[0].content.text(), "[My Link]\n");
        // The wrapped text is still selected: pairing again nests it.
        assert_eq!(
            app.live.as_ref().unwrap().editor.selection(),
            Some("My Link".to_string())
        );
        app.update(Message::EditorAutoPair('['));
        assert_eq!(app.tabs[0].content.text(), "[[My Link]]\n");
        assert_eq!(
            app.live.as_ref().unwrap().editor.selection(),
            Some("My Link".to_string())
        );
    }

    #[test]
    fn jump_to_block_focuses_the_holding_region_and_scrolls_it_in() {
        // "alpha\n\nbeta\n\ngamma" spans (1,2),(3,4),(5,5): block 2 is
        // region 2. An outline on block 2 opens its region's editor.
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta\n\ngamma\n");
        app.live = None;
        app.jump_to_block(2);
        assert_eq!(app.live.as_ref().unwrap().block, 2);
        // The batch carries the region anchor measure op plus the editor
        // focus op: both must exist even though their effects are
        // runtime-side (view tree + widget state).
        app.jump_to_block(0);
        assert_eq!(app.live.as_ref().unwrap().block, 0);
    }

    #[test]
    fn find_selects_matches_in_the_live_region() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nbeta alpha\n");
        app.find = Some(Find {
            query: "alpha".to_string(),
            replacement: String::new(),
            case_sensitive: false,
            current: 0,
        });
        app.live_focus(0);
        app.select_match(0);
        // Match 0 opens region 0 with the hit selected.
        let live = app.live.as_ref().unwrap();
        assert_eq!(live.block, 0);
        assert_eq!(caret(&app), (0, 5));
        assert_eq!(
            live.editor.cursor().selection,
            Some(Position { line: 0, column: 0 })
        );
        // The next match lives in region 1 (absolute line 2 -> relative 0).
        app.find_step(true);
        let live = app.live.as_ref().unwrap();
        assert_eq!(live.block, 1);
        assert_eq!(caret(&app), (0, 10));
        assert_eq!(
            live.editor.cursor().selection,
            Some(Position { line: 0, column: 5 })
        );
    }

    #[test]
    fn find_replace_all_keeps_the_live_buffer_fresh() {
        let mut app = find_tests::test_app_with_tab("alpha\n\nx alpha y\n");
        app.find = Some(Find {
            query: "alpha".to_string(),
            replacement: "z".to_string(),
            case_sensitive: false,
            current: 0,
        });
        app.live_focus(0);
        app.find_replace_all();
        assert_eq!(app.tabs[0].content.text(), "z\n\nx z y\n");
        // The focused region's buffer was re-lifted with the replacement.
        assert_eq!(app.live.as_ref().unwrap().editor.text(), "z\n");
        // One undo step restores both regions at once.
        assert!(undo_tab(&mut app.tabs[0]));
        assert_eq!(app.tabs[0].content.text(), "alpha\n\nx alpha y\n");
    }

    #[test]
    fn folds_survive_edits_inside_the_foldable() {
        // Block 0 is the callout (fold id 1 — frontmatter owns 0), block 1
        // the paragraph. The ids come from traversal order, so they stay
        // valid as long as the callout count is unchanged.
        let mut app = find_tests::test_app_with_tab("> [!note] T\n> body\n\nafter\n");
        app.live_focus(0);
        app.update(Message::ToggleFold(1));
        assert!(app.tabs[0].folds.contains(&1));
        // Typing inside the callout body: count unchanged, fold stays.
        aim(&mut app, 1, 6);
        app.live_run(vec![Action::Edit(Edit::Insert('x'))]);
        assert_eq!(
            app.tabs[0].content.text(),
            "> [!note] T\n> bodyx\n\nafter\n"
        );
        assert!(app.tabs[0].folds.contains(&1));
    }

    #[test]
    fn folds_reset_when_a_foldable_is_added_or_removed() {
        let mut app = find_tests::test_app_with_tab("> [!note] T\n> body\n\nafter\n");
        app.live_focus(0);
        app.update(Message::ToggleFold(1));
        // Adding a callout below shifts the positional ids: folds reset.
        app.live_focus(1);
        aim(&mut app, 0, 5);
        app.live_run(vec![Action::Edit(Edit::Paste(Arc::new(
            "\n> [!warn] W\n> w".to_string(),
        )))]);
        // Two callouts now (the count changed), and the stale fold went
        // with the shifted ids.
        assert_eq!(crate::reading::foldable_count(&app.tabs[0].doc), 2);
        assert!(app.tabs[0].folds.is_empty());
        // Removing one must reset again too: two callouts folded, then
        // the first demoted back to a paragraph by replacing its region.
        app.live_focus(0);
        app.update(Message::ToggleFold(1));
        app.update(Message::ToggleFold(2));
        assert_eq!(app.tabs[0].folds.len(), 2);
        if let Some(live) = app.live.as_mut() {
            live.editor.move_to(Cursor {
                position: Position { line: 2, column: 0 },
                selection: Some(Position { line: 0, column: 0 }),
            });
        }
        app.live_run(vec![Action::Edit(Edit::Insert('x'))]);
        assert!(app.tabs[0].folds.is_empty());
    }

    #[test]
    fn live_edits_refresh_the_status_counts() {
        let mut app = find_tests::test_app_with_tab("one\n");
        app.live_focus(0);
        aim(&mut app, 0, 3);
        app.live_run(vec![
            Action::Edit(Edit::Insert(' ')),
            Action::Edit(Edit::Paste(Arc::new("two three".to_string()))),
        ]);
        assert_eq!(app.tabs[0].content.text(), "one two three\n");
        assert_eq!(app.tabs[0].words, 3);
        assert_eq!(app.tabs[0].chars, "one two three\n".chars().count());
    }

    #[test]
    fn live_edits_restart_the_autosave_idle_and_save() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("n.md"), "hello\n").unwrap();
        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        app.tabs.clear();
        app.tabs.push(Tab::open("n.md".to_string(), "hello\n"));
        app.active = Some(0);
        app.live_focus(0);
        assert!(app.last_edit.is_none());
        // Editing from the live pane restarts the idle timer.
        aim(&mut app, 0, 5);
        app.live_run(vec![Action::Edit(Edit::Paste(Arc::new(
            " there".to_string(),
        )))]);
        assert!(app.last_edit.is_some());
        // Idling past the window fires the guarded save to disk.
        app.last_edit = Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
        app.autosave_tick();
        assert_eq!(
            app.vault.as_ref().unwrap().note_source("n.md").as_deref(),
            Some("hello there\n")
        );
        assert!(!app.tabs[0].is_dirty());
    }
}

#[cfg(test)]
mod tree_sort_tests {
    use super::*;
    use cosmic::app::Application as _;

    fn app_with(files: &[(&str, &str)]) -> (tempfile::TempDir, VaultNotes) {
        let tmp = tempfile::TempDir::new().unwrap();
        for (path, text) in files {
            if let Some((dir, _)) = path.rsplit_once('/') {
                std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
            }
            std::fs::write(tmp.path().join(path), text).unwrap();
        }
        let mut app = find_tests::test_app_with_tab("");
        app.vault = Some(Vault::open(tmp.path()).unwrap());
        app.rebuild_index();
        (tmp, app)
    }

    /// The names of the tree's rows, depth-first.
    fn labels(app: &VaultNotes) -> Vec<String> {
        fn walk(nodes: &[TreeNode], out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    TreeNode::Folder { name, children, .. } => {
                        out.push(name.clone());
                        walk(children, out);
                    }
                    TreeNode::Note { name, .. } => out.push(name.clone()),
                }
            }
        }
        let mut out = Vec::new();
        walk(&app.tree, &mut out);
        out
    }

    #[test]
    fn the_sort_order_moves_the_notes() {
        let _guard = settings_guard();
        let (_tmp, mut app) = app_with(&[("b.md", ""), ("a.md", ""), ("Folder/c.md", "")]);
        // The name order is the default: folders first, notes A to Z.
        assert_eq!(labels(&app), ["Folder", "c.md", "a.md", "b.md"]);

        // Keep the developer's own setting out of the test.
        let before = crate::settings::load().tree_sort;
        let _ = app.update(Message::TreeSort(TreeSort::NameDesc));
        assert_eq!(app.tree_sort, TreeSort::NameDesc);
        // The folder keeps its place; the notes turn around.
        assert_eq!(labels(&app), ["Folder", "c.md", "b.md", "a.md"]);
        persist_setting(move |s| s.tree_sort = before);
    }

    #[test]
    fn every_folder_can_fold_away_and_come_back() {
        let (_tmp, mut app) = app_with(&[("Folder/Deep/n.md", ""), ("a.md", "")]);
        // Nothing is folded at first.
        assert!(app.collapsed.is_empty());

        let _ = app.update(Message::TreeCollapseAll);
        assert_eq!(
            app.collapsed,
            ["Folder".to_string(), "Folder/Deep".to_string()]
                .into_iter()
                .collect()
        );

        let _ = app.update(Message::TreeExpandAll);
        assert!(app.collapsed.is_empty());
    }

    /// A vault with a current note in "Folder/Deep".
    fn app_with_current_note() -> (tempfile::TempDir, VaultNotes) {
        let (tmp, mut app) = app_with(&[
            ("a.md", ""),
            ("Folder/n.md", ""),
            ("Folder/Deep/deep.md", ""),
            ("Other/o.md", ""),
        ]);
        app.tabs[0].path = "Folder/Deep/deep.md".to_string();
        (tmp, app)
    }

    #[test]
    fn collapsing_the_others_keeps_the_current_notes_path_open() {
        let (_tmp, mut app) = app_with_current_note();

        let _ = app.update(Message::TreeCollapseOthers);
        // "Folder" and "Folder/Deep" stay open, so the note stays
        // visible; every other folder folds away.
        assert_eq!(app.collapsed, ["Other".to_string()].into_iter().collect());
    }

    #[test]
    fn revealing_the_current_note_opens_its_path_and_leaves_the_rest() {
        let (_tmp, mut app) = app_with_current_note();

        // Everything folds away first.
        let _ = app.update(Message::TreeCollapseAll);
        assert_eq!(app.collapsed.len(), 3);

        // Revealing opens the path to the current note — and only that.
        let _ = app.update(Message::TreeRevealCurrent);
        assert_eq!(app.collapsed, ["Other".to_string()].into_iter().collect());
    }

    #[test]
    fn without_an_open_note_there_is_no_path_to_work_from() {
        let (_tmp, mut app) = app_with_current_note();
        app.active = None;

        // No path to keep open, so everything folds away.
        let _ = app.update(Message::TreeCollapseOthers);
        assert_eq!(app.collapsed.len(), 3);

        // Nothing to reveal: the tree stays as it is.
        let _ = app.update(Message::TreeRevealCurrent);
        assert_eq!(app.collapsed.len(), 3);
    }
}
