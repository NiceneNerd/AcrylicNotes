//! The native WinUI shell component (p11).
//!
//! One Reactor component over the shared [`WindowsApp`] kernel: native
//! events become [`WindowsMessage`]s (the same envelope the runner
//! consumes), `update` hands them to `WindowsApp::handle_native`, and
//! `view` renders kernel state into WinUI controls. This file holds no
//! feature logic — decisions stay in the kernel reducer; the shell only
//! maps controls to commands.
//!
//! Headless verification lives in `tests/ui_headless.rs`: the shell runs
//! under the `RecordingRuntime` (built for windows-gnu, executed under
//! Wine via `cargo xtask test-windows-ui`), where clicks are injected as
//! recorded events and the reconciled tree is asserted.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use windows_reactor::{
    AutomationExt, Border, Brush, Button, ButtonStyle, ChildrenControl, Component,
    ComponentContext, ContentControl, CornerRadius, FontWeight, Grid, GridChildExt, GridLength,
    HorizontalAlignment, HyperlinkButton, InfoBar, InfoBarSeverity, KeyedView, LayoutControl,
    Orientation, RichEditBox, ScrollBarVisibility, ScrollViewer, SlotView, SlotsControl, SplitView,
    SplitViewDisplayMode, SplitViewSlot, StackPanel, TextBlock, TextBox, TextWrapping, ThemeBrush,
    Thickness, TitleBar, TitleBarSlot, TreeNode, TreeView, TreeViewExt, TreeViewSelectionMode,
    VerticalAlignment, View, ViewContext, WindowBackdrop, WindowSize, WindowTheme,
    WindowTitleBarHeight, WindowVisuals,
};

use vault_app::commands::UiCommand;
use vault_app::navigation::{outline, NavSection};
use vault_app::settings::{PaneMode, ThemeChoice};
use vault_app::tree as kernel_tree;
use vault_md::{parse_note, Block, Doc};
use vault_platform::OpenTarget;

use crate::app::WindowsApp;
use crate::file_tree::FileTree;
use crate::reading::{ImageResolver, WindowsMarkdownRenderer};
use crate::reading_view::{doc_view, live_block_view, LinkNav};
use crate::windows_message::{NativeEvent, WindowsMessage};

/// The shell component's input: a shared handle to the kernel host.
///
/// Pointer equality (the component never re-creates when the same app
/// handle is re-passed).
#[derive(Clone)]
pub struct ShellHandle(Rc<RefCell<WindowsApp>>);

impl PartialEq for ShellHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl ShellHandle {
    /// Wrap the kernel host for shell construction.
    #[must_use]
    pub fn new(app: Rc<RefCell<WindowsApp>>) -> Self {
        Self(app)
    }
}

/// Window width (DIPs) below which the split layout folds to the source
/// pane: two panes side by side need more room than one.
const NARROW_WINDOW_WIDTH: f64 = 480.0;

/// Horizontal padding (DIPs) the reading view leaves around its text.
/// Mirrors the kernel's default `ReadingMargin::Comfortable` (32 px); the
/// settings drawer will drive it live once that lands on Windows.
const READING_MARGIN: f64 = 32.0;

/// The pane a window resize should switch to, if any. Mirrors the Linux
/// shell: below [`NARROW_WINDOW_WIDTH`] Split folds to Source and the fold
/// is remembered, and widening restores Split unless the user picked a
/// different pane while narrow (the caller's `restore_split` is only
/// cleared when the fold is actually undone).
fn narrow_fold(current: PaneMode, width: f64, restore_split: &mut bool) -> Option<PaneMode> {
    let narrow = width < NARROW_WINDOW_WIDTH;
    if narrow && current == PaneMode::Split {
        *restore_split = true;
        Some(PaneMode::Source)
    } else if !narrow && *restore_split && current == PaneMode::Source {
        *restore_split = false;
        Some(PaneMode::Split)
    } else {
        None
    }
}

/// Raster extensions the reading view will decode. Anything else (SVG,
/// PDF, remote URLs) keeps the text placeholder.
fn is_raster_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}

/// Resolve an image URL as written in a note to a canonical file path for
/// inline rendering. The shell builds a `file:///` URI from this path; the
/// WinUI backend handles `ImageValue::Uri` (not `EncodedImage`). Remote URLs
/// and anything that escapes the vault root return `None` (the text
/// placeholder). The note-relative path is tried first, then the vault root,
/// matching Obsidian's attachment lookup.
fn resolve_local_image(
    root: &std::path::Path,
    note_folder: &str,
    cache: &Rc<RefCell<HashMap<String, Option<String>>>>,
    url: &str,
) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:") {
        return None;
    }
    // Obsidian anchors (`image.png#center`) are not part of the filename.
    let url = url.split('#').next().unwrap_or(url).trim();
    if url.is_empty() {
        return None;
    }
    let canonical_root = root.canonicalize().ok()?;
    let from_note = if note_folder.is_empty() {
        canonical_root.join(url)
    } else {
        canonical_root.join(note_folder).join(url)
    };
    for candidate in [from_note, canonical_root.join(url)] {
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&canonical_root) || !is_raster_image(&canonical) {
            continue;
        }
        let key = canonical.to_string_lossy().into_owned();
        if let Some(cached) = cache.borrow().get(&key) {
            return cached.clone();
        }
        let resolved = Some(canonical.to_string_lossy().into_owned());
        cache.borrow_mut().insert(key, resolved.clone());
        return resolved;
    }
    None
}

/// The native shell over the shared kernel: a banner zone above a
/// SplitView whose pane holds the sidebar and whose content holds the
/// note area (tab strip, editor, reading view).
pub struct Shell {
    handle: ShellHandle,
    /// The editor text as last rendered programmatically. A real WinUI
    /// control raises `TextChanged` for property sets as well as user
    /// typing; an incoming change equal to this is that echo of the
    /// shell's own sync and is suppressed instead of being fed back to
    /// the buffer (plan §11.1). `RefCell` because `view` (which takes
    /// `&self`) records it while `update` compares against it. Only the
    /// pane modes that render the source TextBox record it (Source,
    /// Split): while only the reading view is shown no TextBox exists,
    /// so no echoes can be generated.
    rendered_editor: RefCell<String>,
    /// Whether the source TextBox has had its full text rendered at least
    /// once. windows-reactor applies `TextBoxText` before `TextBoxAcceptsReturn`
    /// (fixed enum order, not builder-call order), so on the very first
    /// render multiline text is truncated to one line while `AcceptsReturn`
    /// is still `false`. We render empty text on the first pass, let
    /// `AcceptsReturn`/`Wrap` apply, then re-render with the full text.
    source_text_seeded: Cell<bool>,
    /// The pane mode last rendered with a source TextBox. Switching into a
    /// source-bearing mode (Split/Source) from a different mode recreates the
    /// native control, so the seed flag must reset and re-seed; otherwise a
    /// fresh TextBox receives full text while `AcceptsReturn` is still false
    /// and WinUI truncates it to the first line.
    last_pane: Cell<Option<PaneMode>>,
    /// The vault's file tree: kernel `TreeNode`s plus `TreeState`
    /// (expansion/selection). Built once from the vault's note paths;
    /// expansion state persists across renders here.
    file_tree: FileTree,
    /// The tag panel's drill-down selection (shell-local view state, the
    /// same split as the file tree's expansion state): `None` shows the
    /// tag list, `Some(tag)` shows that tag's notes. Clicking the
    /// selected tag again clears it.
    selected_tag: Option<String>,
    /// Whether the narrow-window fold hid a user-requested Split layout
    /// and should restore it once the window is wide again. Shell-local
    /// (view) state, mirroring the Linux shell's `restore_split`.
    restore_split: bool,
    /// Cache of locally resolved images, keyed by the canonical file path.
    /// An entry of `None` records a failed lookup so a missing file is not
    /// re-read on every render. Shared with the renderer's resolver closure
    /// (hence `Rc`).
    image_cache: Rc<RefCell<HashMap<String, Option<String>>>>,
    /// Which region index is being edited in the Live pane (RichEditBox).
    /// Shell-local view state; `None` means all blocks render as rich text.
    live_focused: Cell<Option<usize>>,
    /// The note path the focused region belongs to. A tab switch invalidates
    /// the focus (the region index belongs to the previous note's slices),
    /// so typing can never splice into the wrong note.
    live_path: RefCell<Option<String>>,
    /// The note's editable regions, cached per render. `BlockSlice` tiling
    /// comes from `vault_app::live::slices`.
    live_slices: RefCell<Vec<vault_app::live::BlockSlice>>,
    /// Last programmatic text rendered into the Live-pane RichEditBox
    /// (echo suppression, same pattern as `rendered_editor`).
    live_editor_text: RefCell<String>,
}

impl Component for Shell {
    type Input = ShellHandle;
    type Message = WindowsMessage;

    fn create(input: &Self::Input, _context: &ComponentContext<Self>) -> Self {
        let file_tree = FileTree::from_paths(&input.0.borrow().note_paths());
        Shell {
            handle: input.clone(),
            rendered_editor: RefCell::new(String::new()),
            source_text_seeded: Cell::new(false),
            file_tree,
            selected_tag: None,
            restore_split: false,
            image_cache: Rc::new(RefCell::new(HashMap::new())),
            last_pane: Cell::new(None),
            live_focused: Cell::new(None),
            live_path: RefCell::new(None),
            live_slices: RefCell::new(Vec::new()),
            live_editor_text: RefCell::new(String::new()),
        }
    }

    fn input_changed(&mut self, input: &Self::Input, _context: &ComponentContext<Self>) {
        self.handle = input.clone();
    }

    fn update(&mut self, message: Self::Message, _context: &ComponentContext<Self>) {
        // Folder rows toggle expansion in the shell's own tree state (the
        // native TreeView raises no expansion events, so the FileTree stays
        // the authority — the same shell-local split the tag drill-down
        // uses). Clicking a folder row must never open a note.
        if let WindowsMessage::Native(NativeEvent::FolderToggled { path }) = &message {
            self.file_tree.toggle_folder(path);
            return;
        }
        // Tag drill-down is shell-local view state (never touches the
        // vault): selecting a tag shows its notes, clicking the selected
        // tag again clears the filter — the filter stays out of the
        // kernel (the same split the file tree's expansion state uses).
        // Tag clicks also navigate the sidebar to the tags panel, the
        // same route the Linux app's `tag:` links take, whether they
        // come from the sidebar rows or from a reading-view tag link.
        if let WindowsMessage::Native(NativeEvent::TagToggled { tag }) = message {
            self.selected_tag = if self.selected_tag.as_deref() == Some(tag.as_str()) {
                None
            } else {
                Some(tag)
            };
            self.handle
                .0
                .borrow_mut()
                .handle_native(WindowsMessage::Command(UiCommand::SetNavSection {
                    section: NavSection::Tags,
                }));
            return;
        }
        // Narrow-window fold: below `NARROW_WINDOW_WIDTH` the split layout
        // folds to the source pane (two panes do not fit), and widening
        // restores it unless the user picked a pane in the meantime. The
        // fold is shell presentation, so it drives the kernel's pane mode
        // through the normal `SetPaneMode` command.
        if let WindowsMessage::Native(NativeEvent::WindowResized { width }) = &message {
            let current = self.handle.0.borrow().shared.state.pane;
            if let Some(mode) = narrow_fold(current, *width, &mut self.restore_split) {
                self.handle
                    .0
                    .borrow_mut()
                    .handle_native(WindowsMessage::Command(UiCommand::SetPaneMode { mode }));
            }
            return;
        }
        // Live pane: clicking a rendered block opens it for editing.
        if let WindowsMessage::Native(NativeEvent::LiveFocus { region }) = message {
            self.live_focused.set(Some(region));
            *self.live_path.borrow_mut() = self
                .handle
                .0
                .borrow()
                .shared
                .state
                .tabs
                .active_path()
                .map(String::from);
            return;
        }
        // Live pane: the focused block's editor text changed. The payload is
        // the BLOCK's source, not the whole note: splice it into the region's
        // line range and hand the full note to the buffer (libcosmic's
        // `typing_in_live_mode_splices_into_the_note` — the buffer stays live,
        // so focusing another block needs no commit step). An incoming text
        // equal to the last rendered editor text is the echo of our own
        // programmatic sync (the mount's SetText) and is suppressed.
        if let WindowsMessage::Native(NativeEvent::LiveBlockEdited { text }) = &message {
            if *text == *self.live_editor_text.borrow() {
                return;
            }
            let region = match self.live_focused.get() {
                Some(region) => region,
                None => return,
            };
            let slice = match self.live_slices.borrow().get(region) {
                Some(&slice) => slice,
                None => return,
            };
            let spliced = {
                let app = self.handle.0.borrow();
                let Some(tab) = app.shared.state.tabs.active() else {
                    return;
                };
                let Some(buffer) = app.buffers.get(&tab) else {
                    return;
                };
                vault_app::live::replace_lines(&buffer.text, slice.start_line, slice.end_line, text)
            };
            *self.live_editor_text.borrow_mut() = text.clone();
            self.handle.0.borrow_mut().set_active_buffer_text(spliced);
            return;
        }
        // Live pane: Escape commits the edited block's source back into
        // the buffer via a line-range splice (round-trips byte-for-byte).
        if let WindowsMessage::Native(NativeEvent::LiveCommit) = message {
            if let Some(region) = self.live_focused.get() {
                let slices = self.live_slices.borrow();
                if let Some(slice) = slices.get(region) {
                    let app = self.handle.0.borrow();
                    let state = &app.shared.state;
                    if let Some(tab) = state.tabs.active() {
                        if let Some(buffer) = app.buffers.get(&tab) {
                            let editor_text = self.live_editor_text.borrow().clone();
                            let new_text = vault_app::live::replace_lines(
                                &buffer.text,
                                slice.start_line,
                                slice.end_line,
                                &editor_text,
                            );
                            drop(app);
                            self.live_focused.set(None);
                            self.handle
                                .0
                                .borrow_mut()
                                .handle_native(WindowsMessage::Native(NativeEvent::EditorEdited {
                                    text: new_text,
                                }));
                            return;
                        }
                    }
                }
            }
        }
        if let WindowsMessage::Native(NativeEvent::EditorEdited { text }) = &message {
            // Echo suppression: equal to the last rendered text means
            // the echo of our own programmatic sync (initial text set,
            // tab switch, reload), not a user edit.
            if *text == *self.rendered_editor.borrow() {
                return;
            }
        }
        if let WindowsMessage::Command(UiCommand::SetSearchQuery { query }) = &message {
            // The search box is controlled by the kernel session; a real
            // TextBox raises `TextChanged` for property sets too, so the
            // echo of our own programmatic sync must not re-enter the
            // kernel (it would bounce on every keystroke).
            if *query == self.handle.0.borrow().shared.state.search.query() {
                return;
            }
        }
        self.handle.0.borrow_mut().handle_native(message);
    }

    fn view(&self, _input: &Self::Input, context: &mut ViewContext<Self>) -> View {
        let app = self.handle.0.borrow();
        let state = &app.shared.state;
        let active_buffer = state.tabs.active().and_then(|tab| app.buffers.get(&tab));

        // The window environment comes from the kernel's appearance
        // state: the persisted theme choice maps to the requested
        // theme (`System` follows the desktop), and the glass toggle
        // requests the frosted Mica backdrop. Declared every
        // publication, so a theme click re-themes the live window.
        let window_theme = match state.theme {
            ThemeChoice::System => WindowTheme::System,
            ThemeChoice::Light => WindowTheme::Light,
            ThemeChoice::Dark => WindowTheme::Dark,
        };
        let backdrop = if state.glass_enabled {
            WindowBackdrop::Mica
        } else {
            WindowBackdrop::None
        };
        context.window_visuals(
            WindowVisuals::new()
                .theme(window_theme)
                .backdrop(backdrop)
                .client_size(1180.0, 760.0),
        );
        context.window_title("VaultNotes");
        // Window-size observation: the shell folds the split layout below
        // `NARROW_WINDOW_WIDTH`. Declared every publication like the other
        // window observations; the runtime only calls it on a real resize.
        context.on_window_size(context.callback(|size: WindowSize| {
            WindowsMessage::Native(NativeEvent::WindowResized { width: size.width })
        }));

        // File-tree sidebar: one TreeView node per vault folder/note,
        // keyed by vault path. The native control reports the invoked
        // row's label (its content string), not its key, so
        // `build_tree_nodes` also returns a label→row map: a note label
        // routes to `OpenNote` with its exact path, a folder label toggles
        // the row's expansion (shell-local state), and expansion flags
        // persist in `FileTree`'s `TreeState` across renders.
        let (tree_nodes, tree_targets) = build_tree_nodes(&self.file_tree);
        let tree_view = TreeView::new()
            .selection_mode(TreeViewSelectionMode::Single)
            .on_item_invoked(context.callback(
                move |label: String| match tree_targets.get(&label) {
                    Some(TreeRowTarget::Note(path)) => {
                        WindowsMessage::Command(UiCommand::OpenNote { path: path.clone() })
                    }
                    Some(TreeRowTarget::Folder(path)) => {
                        WindowsMessage::Native(NativeEvent::FolderToggled { path: path.clone() })
                    }
                    None => WindowsMessage::Native(NativeEvent::Noop),
                },
            ))
            .grid_row(1)
            .nodes(tree_nodes);

        // Sidebar navigation (kernel-owned via `state.panel`): one
        // button per section, the active one bracketed. Tags and links
        // read the vault through the shared `NoteIndex` adapter (the
        // runner's `tag_counts`/`tag_notes`/`backlinks`).
        let nav_buttons: Vec<KeyedView> = NavSection::ALL
            .iter()
            .map(|&section| {
                let label = section.label().to_lowercase();
                let text = if state.panel == section {
                    format!("[{label}]")
                } else {
                    label.clone()
                };
                KeyedView::new(
                    format!("nav-{label}"),
                    Button::new()
                        .on_click(context.callback(move |()| {
                            WindowsMessage::Command(UiCommand::SetNavSection { section })
                        }))
                        .content(TextBlock::new().text(text)),
                )
            })
            .collect();
        let nav_row = StackPanel::new()
            .grid_row(0)
            .orientation(Orientation::Horizontal)
            .keyed_children(nav_buttons);

        // Search panel: the query box mirrors the kernel session (text
        // changes are `SetSearchQuery`), the button runs the vault scan
        // (`SearchVault` -> the runner's shared engine -> the session's
        // hits), and each hit opens its note. Stage A runs the scan
        // synchronously per submit (the Linux shell debounces off-thread);
        // a result row opens the note but does not scroll to the hit yet.
        let search_panel = {
            let query = state.search.query().to_string();
            let search_box = TextBox::new()
                .on_text_changed(context.callback(|query: String| {
                    WindowsMessage::Command(UiCommand::SetSearchQuery { query })
                }))
                .text(query.clone());
            let run_search = Button::new()
                .on_click(context.callback(move |()| {
                    WindowsMessage::Command(UiCommand::SearchVault {
                        query: query.clone(),
                    })
                }))
                .content(TextBlock::new().text("search"));
            let hits: Vec<KeyedView> = state
                .search
                .hits()
                .iter()
                .enumerate()
                .map(|(i, hit)| {
                    let path = hit.path.clone();
                    let label = format!("{}:{} {}", hit.path, hit.line_no, hit.line);
                    KeyedView::new(
                        i.to_string(),
                        Button::new()
                            .on_click(context.callback(move |()| {
                                WindowsMessage::Command(UiCommand::OpenNote { path: path.clone() })
                            }))
                            .content(TextBlock::new().text(label)),
                    )
                })
                .collect();
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .keyed_children([
                    KeyedView::new("search-box", View::from(search_box)),
                    KeyedView::new("search-run", run_search),
                    KeyedView::new(
                        "search-hits",
                        StackPanel::new()
                            .orientation(Orientation::Vertical)
                            .keyed_children(hits),
                    ),
                ])
        };

        // Tags panel (shell-local drill-down via `selected_tag`): the
        // tag list with counts (`#tag (n)` rows), or the selected tag's
        // notes with a back row. Clicking a tag toggles the filter
        // (shell-local; never touches the vault); clicking a note opens
        // it through the kernel.
        let tags_panel = {
            let mut rows: Vec<KeyedView> = Vec::new();
            match &self.selected_tag {
                None => {
                    let counts = app.runner.tag_counts();
                    if counts.is_empty() {
                        rows.push(KeyedView::new(
                            "no-tags",
                            View::from(TextBlock::new().text("(no tags in this vault)")),
                        ));
                    }
                    rows.extend(counts.into_iter().map(|(tag, count)| {
                        let key = tag.clone();
                        let click_tag = tag.clone();
                        KeyedView::new(
                            key,
                            Button::new()
                                .on_click(context.callback(move |()| {
                                    WindowsMessage::Native(NativeEvent::TagToggled {
                                        tag: click_tag.clone(),
                                    })
                                }))
                                .content(TextBlock::new().text(format!("#{tag} ({count})"))),
                        )
                    }));
                }
                Some(tag) => {
                    let selected = tag.clone();
                    rows.push(KeyedView::new(
                        "all-tags",
                        Button::new()
                            .on_click(context.callback(move |()| {
                                WindowsMessage::Native(NativeEvent::TagToggled {
                                    tag: selected.clone(),
                                })
                            }))
                            .content(TextBlock::new().text(format!("← #{tag}"))),
                    ));
                    rows.extend(app.runner.tag_notes(tag).into_iter().map(|path| {
                        let click_path = path.clone();
                        KeyedView::new(
                            path.clone(),
                            Button::new()
                                .on_click(context.callback(move |()| {
                                    WindowsMessage::Command(UiCommand::OpenNote {
                                        path: click_path.clone(),
                                    })
                                }))
                                .content(TextBlock::new().text(path)),
                        )
                    }));
                }
            }
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .keyed_children(rows)
        };

        // Links panel: the active note's backlinks (the shared
        // `NoteIndex` adapter's data, sorted by source path). Each row
        // opens its source note.
        let links_panel = match state.tabs.active_path() {
            None => View::from(TextBlock::new().text("(open a note to see its backlinks)")),
            Some(path) => {
                let sources = app.runner.backlinks(path);
                let rows: Vec<KeyedView> = if sources.is_empty() {
                    vec![KeyedView::new(
                        "no-backlinks",
                        View::from(TextBlock::new().text("(no backlinks)")),
                    )]
                } else {
                    sources
                        .into_iter()
                        .map(|source| {
                            let click_source = source.clone();
                            KeyedView::new(
                                source.clone(),
                                Button::new()
                                    .on_click(context.callback(move |()| {
                                        WindowsMessage::Command(UiCommand::OpenNote {
                                            path: click_source.clone(),
                                        })
                                    }))
                                    .content(TextBlock::new().text(source)),
                            )
                        })
                        .collect()
                };
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .keyed_children(rows)
            }
        };

        // Outline panel: the active buffer's headings (the shared
        // `vault_app::navigation::outline` view model), indented by
        // level. Clicking an entry anchors that block in the reading
        // view via the kernel `JumpOutline` command (rendered as a
        // Border marker; real scrolling deferred to a toolkit scroll
        // API).
        let outline_panel = active_buffer
            .map(|buffer| {
                let doc = parse_note(&buffer.text);
                let rows: Vec<KeyedView> = outline(&doc)
                    .iter()
                    .map(|entry| {
                        let indent = "  ".repeat(usize::from(entry.level.saturating_sub(1)));
                        let text = format!("{indent}{}", entry.text);
                        let block = entry.block;
                        KeyedView::new(
                            entry.block.to_string(),
                            HyperlinkButton::new()
                                .on_click(context.callback(move |()| {
                                    WindowsMessage::Command(UiCommand::JumpOutline { block })
                                }))
                                .content(TextBlock::new().text(text)),
                        )
                    })
                    .collect();
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .keyed_children(rows)
            })
            .unwrap_or_else(|| View::from(TextBlock::new().text("(no note open)")));

        // The active panel. Files is the default. Only the tree scrolls
        // itself (the native TreeView owns its viewport); the other panels
        // need an explicit ScrollViewer. Every panel sits in a Star grid
        // row so it gets a bounded height — a StackPanel would hand its
        // children infinite height and nothing would ever scroll.
        let sidebar_panel: View = match state.panel {
            NavSection::Files => tree_view,
            NavSection::Search => ScrollViewer::new().grid_row(1).content(search_panel),
            NavSection::Tags => ScrollViewer::new().grid_row(1).content(tags_panel),
            NavSection::Links => ScrollViewer::new().grid_row(1).content(links_panel),
            NavSection::Outline => ScrollViewer::new().grid_row(1).content(outline_panel),
        };
        // Sidebar as a rounded card: the card surface picks up the theme
        // (and Mica translucency when glass is on) and the stroke keeps the
        // edge legible against the note area.
        let sidebar = Border::new()
            .corner_radius(CornerRadius::uniform(8.0))
            .border_thickness(Thickness::uniform(1.0))
            .background(Brush::Theme(ThemeBrush::CardBackground))
            .border_brush(Brush::Theme(ThemeBrush::CardStroke))
            .padding(Thickness::uniform(8.0))
            .content(
                Grid::new()
                    .rows([GridLength::Auto, GridLength::STAR])
                    .columns([GridLength::STAR])
                    .keyed_children([
                        KeyedView::new("nav-row", nav_row),
                        KeyedView::new("sidebar-panel", sidebar_panel),
                    ]),
            );

        // Tab strip: a horizontal, scrollable row of tabs mirroring the
        // Linux app's tab bar. Each tab shows the note's title and, dimmed
        // beside it, the folder the note lives in (so two notes with the
        // same name are told apart at a glance); a leading dot marks
        // unsaved edits and a per-tab close button sits at the right. The
        // active tab is tinted and its title is bold. Keyed by path so
        // reconciliation keeps identity as tabs open and close (the kernel
        // never holds two tabs for one path). Automation names give each
        // control a stable, accessible handle for screen readers and the
        // headless tests.
        let tabs: Vec<KeyedView> = state
            .tabs
            .order()
            .iter()
            .map(|&tab| {
                let path = state.tabs.path(tab).unwrap_or("?").to_string();
                let is_active = state.tabs.active() == Some(tab);
                let dirty = app
                    .buffers
                    .get(&tab)
                    .is_some_and(crate::app::NoteBuffer::is_dirty);
                let mut title = vault_app::fuzzy::title_of(&path);
                if dirty {
                    title = format!("• {title}");
                }
                let folder = path
                    .rsplit_once('/')
                    .map(|(folder, _)| folder.to_string())
                    .unwrap_or_default();
                let position = state.tabs.position(tab).unwrap_or(0);

                // The clickable label: the title, plus the folder dimmed
                // beside it when the note is not at the vault root.
                let mut label_cells: Vec<KeyedView> = vec![KeyedView::new(
                    "title",
                    TextBlock::new().text(title).font_weight(if is_active {
                        FontWeight::SEMI_BOLD
                    } else {
                        FontWeight::NORMAL
                    }),
                )];
                if !folder.is_empty() {
                    label_cells.push(KeyedView::new(
                        "folder",
                        TextBlock::new().text(folder).font_size(12.0).opacity(0.6),
                    ));
                }
                let label = StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(6.0)
                    .keyed_children(label_cells);
                let activate = Button::new()
                    .automation_name(format!("open {path}"))
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::ActivateTab { position })
                    }))
                    .content(label);
                let close = Button::new()
                    .automation_name(format!("close {path}"))
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::CloseTab { position })
                    }))
                    .content(TextBlock::new().text("✕"));
                let row = StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(4.0)
                    .keyed_children([
                        KeyedView::new("open", activate),
                        KeyedView::new("close", close),
                    ]);
                let mut tab_body = Border::new().padding(4.0).corner_radius(4.0);
                if is_active {
                    // A card fill plus a thin accent border reads as the
                    // selected tab against the note-area surface in both
                    // light and dark themes (the surface itself is themed
                    // below, so a plain fill could blend into it).
                    tab_body = tab_body
                        .background(Brush::Theme(ThemeBrush::CardBackground))
                        .border_brush(Brush::Theme(ThemeBrush::Accent))
                        .border_thickness(1.0);
                }
                KeyedView::new(path, tab_body.content(row))
            })
            .collect();

        // Error banner: the kernel's last actionable error, full-width and
        // above the sidebar (Linux parity). It opens when the kernel records
        // an error and closes again once a later action clears `last_error`.
        let error_banner = InfoBar::new()
            .severity(InfoBarSeverity::Error)
            .is_open(state.last_error.is_some())
            .message(state.last_error.clone().unwrap_or_default())
            .is_closable(false);

        // Pane layout (kernel-owned via `state.pane`): one button per
        // mode, the active one bracketed. A click is a kernel command.
        let pane_buttons: Vec<KeyedView> = [
            ("live", PaneMode::Live),
            ("split", PaneMode::Split),
            ("source", PaneMode::Source),
            ("reading", PaneMode::Reading),
        ]
        .iter()
        .map(|&(label, mode)| {
            let text = if state.pane == mode {
                format!("[{label}]")
            } else {
                label.to_string()
            };
            KeyedView::new(
                label,
                Button::new()
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::SetPaneMode { mode })
                    }))
                    .content(TextBlock::new().text(text)),
            )
        })
        .collect();

        // Appearance controls: one theme-choice button each (the active
        // choice bracketed, the same convention as the pane row) plus a
        // glass toggle. Each click is a kernel command that persists the
        // snapshot and re-themes the window through `window_visuals`.
        let theme_buttons: Vec<KeyedView> =
            [
                ("system", ThemeChoice::System),
                ("light", ThemeChoice::Light),
                ("dark", ThemeChoice::Dark),
            ]
            .iter()
            .map(|&(label, theme)| {
                let text = if state.theme == theme {
                    format!("[{label}]")
                } else {
                    label.to_string()
                };
                KeyedView::new(
                    format!("theme-{label}"),
                    Button::new()
                        .on_click(context.callback(move |()| {
                            WindowsMessage::Command(UiCommand::SetTheme { theme })
                        }))
                        .content(TextBlock::new().text(text)),
                )
            })
            .collect();
        let glass_label = if state.glass_enabled {
            "[glass]"
        } else {
            "glass"
        };
        // The toggle reads the kernel's live state at click time (the
        // handle is 'static; the render-time borrow is not).
        let glass_handle = self.handle.clone();
        let glass_button = KeyedView::new(
            "glass-toggle",
            Button::new()
                .on_click(context.callback(move |()| {
                    let enabled = !glass_handle.0.borrow().shared.state.glass_enabled;
                    WindowsMessage::Command(UiCommand::SetGlassEnabled { enabled })
                }))
                .content(TextBlock::new().text(glass_label)),
        );
        let mut appearance_buttons = theme_buttons;
        appearance_buttons.push(glass_button);
        let appearance_row = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .keyed_children(appearance_buttons);

        // The note area per the kernel's pane mode. Source and Split
        // render the source TextBox (user edits arrive as
        // `TextBoxTextChanged` callbacks and become `EditorEdited`
        // native events; buffer update and echo suppression live in
        // `update` — Stage A sync, plan §11.1). Split and Reading also
        // render the parsed current buffer as native reading elements;
        // the doc is parsed per render (the Linux app caches per edit;
        // caching is a later slice). Live renders the reading view too —
        // block-level editing inside Live is plan §11.4 and lands last.
        // With no active buffer the source editor renders empty.
        let show_source = !matches!(state.pane, PaneMode::Live | PaneMode::Reading);
        let show_reading = !matches!(state.pane, PaneMode::Source);
        // No source TextBox in the note area means the next source-mode
        // render recreates the native control, so it needs a fresh seed pass.
        if !show_source {
            self.source_text_seeded.set(false);
        }
        // Entering a source-bearing mode from a different pane recreates the
        // native TextBox, so it needs a fresh seed pass: full text on a fresh
        // control would be truncated while `AcceptsReturn` is still false.
        if show_source && self.last_pane.get() != Some(state.pane) {
            self.source_text_seeded.set(false);
        }
        self.last_pane.set(Some(state.pane));
        // Live focus is per-pane and per-note: leaving Live (or switching to
        // a different note) drops the editor. The region index belongs to the
        // note whose slices were current when it was focused; typing after a
        // tab switch must never splice into the wrong note. Edits are already
        // spliced live, so there is nothing to commit — the block just
        // renders as rich text again.
        if state.pane != PaneMode::Live {
            self.live_focused.set(None);
        } else {
            let active_path = state.tabs.active_path().map(String::from);
            if self.live_path.borrow().as_deref() != active_path.as_deref() {
                self.live_focused.set(None);
            }
        }
        // windows-reactor applies `TextBoxText` before `TextBoxAcceptsReturn`
        // (fixed enum order, not builder-call order), so on the very first
        // render multiline text is truncated to one line while
        // `AcceptsReturn` is still `false`. Render empty text on the first
        // source-mode pass, let `AcceptsReturn`/`Wrap` apply, then re-render
        // with the full text — now `AcceptsReturn` is already true so
        // multiline content survives `SetText`.
        let is_first_source_render = show_source && !self.source_text_seeded.get();
        let editor_text = active_buffer.map(|b| b.text.clone()).unwrap_or_default();
        let source_display_text = if is_first_source_render {
            String::new()
        } else {
            editor_text.clone()
        };
        if show_source {
            *self.rendered_editor.borrow_mut() = source_display_text.clone();
        }

        let source_editor = TextBox::new()
            .accepts_return(true)
            .text_wrapping(TextWrapping::Wrap)
            .vertical_alignment(VerticalAlignment::Stretch)
            .min_height(300.0)
            .on_text_changed(context.callback(|text: String| {
                WindowsMessage::Native(NativeEvent::EditorEdited { text })
            }))
            .text(source_display_text);

        if is_first_source_render {
            self.source_text_seeded.set(true);
            let sender = context.sender();
            context.use_effect("source-text-seed", (), move || {
                sender.send(WindowsMessage::Native(NativeEvent::Noop));
                None
            });
        }
        // Local image resolver: reads vault rasters relative to the active
        // note's folder (then the vault root) and caches them by canonical
        // path. Remote URLs and paths escaping the root keep the text
        // placeholder. The cache is shared across renders.
        let vault_root = self.handle.0.borrow().runner.root().to_path_buf();
        // The active note's folder, splitting on either path separator so
        // Windows backslash paths resolve images relative to the note too.
        let note_folder = state
            .tabs
            .active_path()
            .and_then(|path| {
                path.rsplit_once(['/', '\\'])
                    .map(|(folder, _)| folder.to_string())
            })
            .unwrap_or_default();
        let image_resolver: ImageResolver = {
            let cache = self.image_cache.clone();
            Rc::new(move |url: &str| resolve_local_image(&vault_root, &note_folder, &cache, url))
        };
        let renderer = WindowsMarkdownRenderer {
            image_resolver: Some(image_resolver),
            ..Default::default()
        };
        // Parse the doc once; Reading/Split use doc_view, Live uses
        // live_block_view per region.
        let doc: Option<Doc> = active_buffer
            .as_ref()
            .map(|buffer| parse_note(&buffer.text));

        // Link-click callback: shared logic, created at each call site
        // (context.callback takes &self, so multiple can coexist).
        let link_callback = |handle: &ShellHandle, nav: &LinkNav| -> WindowsMessage {
            match nav {
                LinkNav::Url(url) => WindowsMessage::Command(UiCommand::OpenExternal {
                    target: OpenTarget::Url(url.clone()),
                }),
                LinkNav::Note(path) => {
                    WindowsMessage::Command(UiCommand::OpenNote { path: path.clone() })
                }
                // Relative Markdown link: resolve against the active note's
                // folder through the vault index (plan §10.3's
                // `resolve_relative` flow).
                LinkNav::Relative(target) => {
                    let from = handle
                        .0
                        .borrow()
                        .shared
                        .state
                        .tabs
                        .active_path()
                        .unwrap_or("")
                        .to_string();
                    if let Some(path) = handle.0.borrow().runner.resolve_relative(target, &from) {
                        WindowsMessage::Command(UiCommand::OpenNote { path })
                    } else {
                        WindowsMessage::Command(UiCommand::OpenExternal {
                            target: OpenTarget::File(std::path::PathBuf::from(target.as_str())),
                        })
                    }
                }
                LinkNav::Tag(tag) => {
                    WindowsMessage::Native(NativeEvent::TagToggled { tag: tag.clone() })
                }
                LinkNav::Footnote(block) => {
                    WindowsMessage::Command(UiCommand::JumpOutline { block: *block })
                }
            }
        };

        let footnote_blocks: std::collections::HashMap<String, usize> = doc
            .as_ref()
            .map(|doc| {
                doc.blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, block)| match block {
                        Block::FootnoteDef { label, .. } => Some((label.clone(), i)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let reading_area = doc
            .as_ref()
            .map(|doc| {
                let handle = self.handle.clone();
                let on_link = |nav: LinkNav| {
                    let handle = handle.clone();
                    context.callback(move |()| link_callback(&handle, &nav))
                };
                doc_view(doc, &renderer, state.jump_anchor, &on_link)
            })
            .unwrap_or_else(|| View::from(TextBlock::new().text("(no note open)")));

        // Reading margins: the text column keeps the configurable gutter
        // around it (horizontal only, matching the Linux reading view).
        let reading_area = Border::new()
            .padding(Thickness::new(READING_MARGIN, 0.0, READING_MARGIN, 0.0))
            .content(reading_area);

        // Live pane: each region renders as rich text, except the focused
        // region which is a raw-source RichEditBox. Click a block to edit
        // it; Escape commits the block's source back via line-range splice.
        let live_pane_body = if state.pane == PaneMode::Live {
            if let Some(buffer) = active_buffer {
                let doc = doc.as_ref().unwrap();
                let slices = vault_app::live::slices(&buffer.text, doc);
                self.live_slices.replace(slices.clone());
                let children: Vec<KeyedView> = slices
                    .iter()
                    .enumerate()
                    .map(|(region_idx, slice)| {
                        let is_focused = self.live_focused.get() == Some(region_idx);
                        let key = format!("live-region-{region_idx}");

                        if is_focused {
                            // Raw-source editor for the focused block. Its
                            // text changes splice into the region (see the
                            // LiveBlockEdited handler), so the buffer stays
                            // live and no commit step is needed.
                            let source = slice.source(&buffer.text);
                            *self.live_editor_text.borrow_mut() = source.clone();

                            KeyedView::new(
                                key,
                                RichEditBox::new()
                                    .text(source)
                                    .on_text_changed(context.callback(|text: String| {
                                        WindowsMessage::Native(NativeEvent::LiveBlockEdited {
                                            text,
                                        })
                                    }))
                                    .vertical_alignment(VerticalAlignment::Stretch)
                                    .min_height(60.0),
                            )
                        } else {
                            match slice.block {
                                Some(block_idx) => {
                                    let region = region_idx;
                                    let on_link = |nav: LinkNav| {
                                        let handle = self.handle.clone();
                                        context.callback(move |()| link_callback(&handle, &nav))
                                    };
                                    KeyedView::new(
                                        key,
                                        Button::new()
                                            .style(ButtonStyle::Subtle)
                                            .horizontal_content_alignment(HorizontalAlignment::Left)
                                            .vertical_content_alignment(VerticalAlignment::Top)
                                            .automation_name(format!("edit region {region}"))
                                            .on_click(context.callback(move |()| {
                                                WindowsMessage::Native(NativeEvent::LiveFocus {
                                                    region,
                                                })
                                            }))
                                            .content(live_block_view(
                                                &doc.blocks[block_idx],
                                                &renderer,
                                                &footnote_blocks,
                                                &on_link,
                                            )),
                                    )
                                }
                                None => {
                                    // Frontmatter or empty body region: raw
                                    // text, muted, still clickable to edit.
                                    let text = slice.source(&buffer.text);
                                    KeyedView::new(
                                        key,
                                        Button::new()
                                            .style(ButtonStyle::Subtle)
                                            .horizontal_content_alignment(HorizontalAlignment::Left)
                                            .vertical_content_alignment(VerticalAlignment::Top)
                                            .automation_name(format!("edit region {region_idx}"))
                                            .on_click(context.callback(move |()| {
                                                WindowsMessage::Native(NativeEvent::LiveFocus {
                                                    region: region_idx,
                                                })
                                            }))
                                            .content(
                                                TextBlock::new()
                                                    .text(text)
                                                    .font_size(12.0)
                                                    .opacity(0.6)
                                                    .text_wrapping(TextWrapping::Wrap),
                                            ),
                                    )
                                }
                            }
                        }
                    })
                    .collect();

                ScrollViewer::new()
                    .grid_row(1)
                    .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                    .content(
                        StackPanel::new()
                            .orientation(Orientation::Vertical)
                            .spacing(10.0)
                            .keyed_children(children),
                    )
            } else {
                TextBlock::new()
                    .text("Open a note from the sidebar, or click a link in reading view.")
                    .opacity(0.6)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .vertical_alignment(VerticalAlignment::Center)
                    .grid_row(1)
                    .into()
            }
        } else {
            View::from(TextBlock::new().text(""))
        };

        // Note area: the tab strip on top, the pane body filling the rest.
        // The body sits in a Star grid row so the ScrollViewers get a
        // bounded viewport — a StackPanel would hand them infinite height
        // and nothing would ever scroll. Split shows both panes side by
        // side (Linux's fixed 50/50, no divider).
        // The strip scrolls sideways when more tabs than fit are open, the
        // same behavior as the Linux app's horizontal tab bar.
        let tab_bar = ScrollViewer::new()
            .grid_row(0)
            .horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Disabled)
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(2.0)
                    .keyed_children(tabs),
            );
        // The source editor is a multiline TextBox that scrolls internally,
        // so it goes straight into the bounded STAR grid cell. Wrapping it
        // in a ScrollViewer was wrong: a vertical ScrollViewer measures its
        // content with infinite height, so the TextBox kept its one-line
        // desired height on the first pass (and `VerticalAlignment::Stretch`
        // could not apply), clipping multi-line notes until a tab switch
        // forced a remeasure. As a direct grid child the row gives it a
        // bounded height and Stretch fills it immediately. The reading view
        // keeps its ScrollViewer — its element tree has no internal scroller.
        let pane_body: View = if state.tabs.active().is_none() {
            // Empty state: no note open, so the note column shows a hint
            // instead of an empty editor. Mirrors the Linux shell's
            // centered muted message.
            TextBlock::new()
                .text("Open a note from the sidebar, or click a link in reading view.")
                .opacity(0.6)
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center)
                .grid_row(1)
                .into()
        } else if state.pane == PaneMode::Live {
            // Live pane: rich-text blocks with a per-block raw-source
            // editor; click a block to edit it, Escape commits.
            live_pane_body
        } else if show_source && show_reading {
            let editor = View::from(source_editor.grid_column(0));
            let reading = ScrollViewer::new()
                .grid_column(1)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .content(reading_area);
            Grid::new()
                .grid_row(1)
                .columns([GridLength::STAR, GridLength::STAR])
                .keyed_children([
                    KeyedView::new("editor", editor),
                    KeyedView::new("reading-view", reading),
                ])
        } else if show_source {
            View::from(source_editor.grid_row(1))
        } else {
            ScrollViewer::new()
                .grid_row(1)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .content(reading_area)
        };
        // Surface follows the app theme (Dark/Light/System), not the OS
        // theme — `RequestedTheme` only reaches text colors in this
        // toolkit version, so the chrome could look right while the note
        // area stayed white-on-light-OS or black-on-dark-OS.
        let note_area_bg = if state.glass_enabled {
            // Glass covers the backdrop; keep the surface translucent.
            Brush::Theme(ThemeBrush::CardBackground)
        } else {
            Brush::Theme(ThemeBrush::SolidBackground)
        };
        // Status bar under the note column only (not the sidebar): word and
        // character counts for the active buffer, mirroring the Linux
        // shell's label. `count_words` is shared kernel logic; the counts
        // come from the live buffer so unsaved edits update immediately.
        let status_text = active_buffer
            .map(|buffer| {
                let words = vault_app::editor::count_words(&buffer.text);
                let chars = buffer.text.chars().count();
                format!("{words} words • {chars} characters")
            })
            .unwrap_or_default();
        let status_bar = TextBlock::new()
            .text(status_text)
            .font_size(12.0)
            .opacity(0.6)
            .margin(Thickness::xy(12.0, 4.0))
            .grid_row(2);
        let note_area = Grid::new()
            .background(note_area_bg)
            .rows([GridLength::Auto, GridLength::STAR, GridLength::Auto])
            .columns([GridLength::STAR])
            .keyed_children([
                KeyedView::new("tab-bar", tab_bar),
                KeyedView::new("pane-body", pane_body),
                KeyedView::new("status-bar", status_bar),
            ]);

        // Toolbar row hosted in the title bar's Content slot: save, the
        // pane-mode switcher, and the appearance controls. Icon-only in the
        // real WinUI build; text labels here keep headless labels stable.
        let mut toolbar_buttons: Vec<KeyedView> = Vec::new();
        toolbar_buttons.push(KeyedView::new(
            "save",
            Button::new()
                .on_click(context.callback(|()| WindowsMessage::Command(UiCommand::SaveActiveTab)))
                .content(TextBlock::new().text("save")),
        ));
        toolbar_buttons.extend(pane_buttons);
        toolbar_buttons.push(KeyedView::new("appearance-row", appearance_row));
        let toolbar = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .keyed_children(toolbar_buttons);

        // Banner zone (row 1): the error InfoBar spans the full width above
        // the sidebar, matching the Linux app's banner/error stack.
        let banner_zone = StackPanel::new()
            .grid_row(1)
            .grid_column(0)
            .orientation(Orientation::Vertical)
            .keyed_children([KeyedView::new("error-line", error_banner)]);

        // SplitView (row 2): sidebar in the pane, note area in the content.
        // `is_pane_open` is controlled by kernel `sidebar_open`; the title
        // bar's pane toggle and the pane's own close both write back
        // through `SetSidebarVisible`.
        let pane_handle = self.handle.clone();
        let split_view = SplitView::new()
            .grid_row(2)
            .grid_column(0)
            .display_mode(SplitViewDisplayMode::Inline)
            .open_pane_length(250.0)
            .is_pane_open(Some(state.sidebar_open))
            .on_pane_closed(context.callback(move |open: bool| {
                WindowsMessage::Command(UiCommand::SetSidebarVisible { open })
            }))
            .slots([
                SlotView::new(SplitViewSlot::Pane, sidebar),
                SlotView::new(SplitViewSlot::Content, note_area),
            ]);

        // Title bar (row 0): content extends into the title bar; the
        // built-in pane toggle button writes back to kernel `sidebar_open`
        // through `SetSidebarVisible`. The callback returns `()` so the
        // message is sent imperatively through the context sender.
        let pane_sender = context.sender();
        let title_bar = TitleBar::new()
            .grid_row(0)
            .grid_column(0)
            .grid_column_span(1)
            .title("Vault Notes")
            .preferred_height(WindowTitleBarHeight::Tall)
            .is_pane_toggle_button_visible(true)
            .on_pane_toggle_requested(move || {
                let open = !pane_handle.0.borrow().shared.state.sidebar_open;
                pane_sender.send(WindowsMessage::Command(UiCommand::SetSidebarVisible {
                    open,
                }));
            })
            .slots([SlotView::new(TitleBarSlot::Content, toolbar)]);

        Grid::new()
            .rows([GridLength::Auto, GridLength::Auto, GridLength::STAR])
            .columns([GridLength::STAR])
            .keyed_children([
                KeyedView::new("title-bar", title_bar),
                KeyedView::new("banner-zone", banner_zone),
                KeyedView::new("split-view", split_view),
            ])
    }
}

/// What invoking a file-tree row means. The native TreeView reports the
/// invoked row's label (its content string), not its key, so the shell
/// maps each label back to the row it rendered. Labels are made unique
/// up front, which keeps the mapping exact.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeRowTarget {
    /// Open this note (its exact vault-relative path).
    Note(String),
    /// Toggle the folder's expansion (shell-local tree state).
    Folder(String),
}

/// Convert the kernel tree (folders + notes, expansion state from
/// `TreeState`) into the hierarchy the native TreeView renders, plus the
/// label→row map `on_item_invoked` routes through.
///
/// A row's label must identify it exactly: a name that appears once in
/// the vault stays bare, while a shared name is shown as its full
/// vault-relative path. Paths cannot collide with single-segment names
/// (a filename never contains a separator) and are unique among
/// themselves, so the resulting labels are all distinct.
fn build_tree_nodes(tree: &FileTree) -> (Vec<TreeNode>, Rc<HashMap<String, TreeRowTarget>>) {
    /// Collect every row as (path, name, is-folder), depth-first.
    fn flatten(node: &kernel_tree::TreeNode, rows: &mut Vec<(String, String, bool)>) {
        match node {
            kernel_tree::TreeNode::Folder {
                name,
                path,
                children,
            } => {
                rows.push((path.clone(), name.clone(), true));
                for child in children {
                    flatten(child, rows);
                }
            }
            kernel_tree::TreeNode::Note { name, path } => {
                rows.push((path.clone(), name.clone(), false));
            }
        }
    }

    let mut rows: Vec<(String, String, bool)> = Vec::new();
    for node in &tree.nodes {
        flatten(node, &mut rows);
    }
    // A name shared by two or more rows is ambiguous on its own.
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for (_, name, _) in &rows {
        *name_counts.entry(name.as_str()).or_default() += 1;
    }
    // path → label: the bare name when unique, the full path when shared.
    let labels: HashMap<String, String> = rows
        .iter()
        .map(|(path, name, _)| {
            let label = if name_counts[name.as_str()] == 1 {
                name.clone()
            } else {
                path.clone()
            };
            (path.clone(), label)
        })
        .collect();
    let targets: HashMap<String, TreeRowTarget> = rows
        .iter()
        .map(|(path, _, is_folder)| {
            let target = if *is_folder {
                TreeRowTarget::Folder(path.clone())
            } else {
                TreeRowTarget::Note(path.clone())
            };
            (labels[path].clone(), target)
        })
        .collect();

    fn convert(
        node: &kernel_tree::TreeNode,
        tree: &FileTree,
        labels: &HashMap<String, String>,
    ) -> TreeNode {
        match node {
            kernel_tree::TreeNode::Folder { path, children, .. } => {
                TreeNode::new(path.as_str(), labels[path].as_str())
                    .expanded(tree.state.is_expanded(path))
                    .children(children.iter().map(|child| convert(child, tree, labels)))
            }
            kernel_tree::TreeNode::Note { path, .. } => {
                TreeNode::new(path.as_str(), labels[path].as_str())
            }
        }
    }

    let nodes = tree
        .nodes
        .iter()
        .map(|node| convert(node, tree, &labels))
        .collect();
    (nodes, Rc::new(targets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn narrow_fold_collapses_split_and_restores_it_when_wide() {
        let mut restore = false;
        // Split folds to Source below the threshold, remembering the fold.
        assert_eq!(
            narrow_fold(PaneMode::Split, NARROW_WINDOW_WIDTH - 1.0, &mut restore),
            Some(PaneMode::Source)
        );
        assert!(restore);
        // Widening over the same pane restores Split and clears the flag.
        assert_eq!(
            narrow_fold(PaneMode::Source, NARROW_WINDOW_WIDTH + 1.0, &mut restore),
            Some(PaneMode::Split)
        );
        assert!(!restore);
    }

    #[test]
    fn narrow_fold_leaves_a_pane_the_user_picked() {
        // The user picked Reading while narrow: widening must not override
        // that choice back to Split.
        let mut restore = true;
        assert_eq!(
            narrow_fold(PaneMode::Reading, NARROW_WINDOW_WIDTH - 1.0, &mut restore),
            None
        );
        assert_eq!(
            narrow_fold(PaneMode::Reading, NARROW_WINDOW_WIDTH + 1.0, &mut restore),
            None
        );
        // A pane that was never folded is untouched either way.
        assert_eq!(
            narrow_fold(PaneMode::Live, NARROW_WINDOW_WIDTH + 1.0, &mut restore),
            None
        );
    }

    #[test]
    fn resolve_local_image_finds_image_at_vault_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("photo.png"), b"PNG").expect("write image");
        let cache = Rc::new(RefCell::new(HashMap::new()));
        assert!(resolve_local_image(dir.path(), "", &cache, "photo.png").is_some());
    }

    #[test]
    fn resolve_local_image_finds_image_in_note_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("notes")).expect("create notes");
        std::fs::write(dir.path().join("notes").join("img.png"), b"PNG").expect("write image");
        let cache = Rc::new(RefCell::new(HashMap::new()));
        assert!(
            resolve_local_image(dir.path(), "notes", &cache, "img.png").is_some(),
            "image next to the active note resolves"
        );
    }

    #[test]
    fn resolve_local_image_handles_backslash_note_folder() {
        // Windows-style paths use backslashes; the folder lookup splits on
        // either separator so images beside a note still resolve.
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("notes")).expect("create notes");
        std::fs::write(dir.path().join("notes").join("img.png"), b"PNG").expect("write image");
        let cache = Rc::new(RefCell::new(HashMap::new()));
        assert!(
            resolve_local_image(dir.path(), "notes\\", &cache, "img.png").is_some(),
            "backslash note folder still resolves the image"
        );
    }

    #[test]
    fn resolve_local_image_rejects_remote_nonraster_and_escape() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("diagram.svg"), b"svg").expect("write svg");
        let cache = Rc::new(RefCell::new(HashMap::new()));
        assert!(
            resolve_local_image(dir.path(), "", &cache, "https://example.com/x.png").is_none(),
            "remote url is a placeholder"
        );
        assert!(
            resolve_local_image(dir.path(), "", &cache, "diagram.svg").is_none(),
            "non-raster is a placeholder"
        );
        assert!(
            resolve_local_image(dir.path(), "", &cache, "../escape.png").is_none(),
            "path escaping the vault root is a placeholder"
        );
        assert!(
            resolve_local_image(dir.path(), "", &cache, "missing.png").is_none(),
            "missing file is a placeholder"
        );
    }
}
