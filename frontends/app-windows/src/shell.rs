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

use std::cell::RefCell;
use std::rc::Rc;

use windows_reactor::{
    Button, ChildrenControl, Component, ComponentContext, ContentControl, Grid, GridChildExt,
    GridLength, HyperlinkButton, InfoBar, InfoBarSeverity, KeyedView, Orientation,
    ScrollBarVisibility, ScrollViewer, SlotView, SlotsControl, SplitView, SplitViewDisplayMode,
    SplitViewSlot, StackPanel, TextBlock, TextBox, TitleBar, TitleBarSlot, TreeNode, TreeView,
    TreeViewExt, TreeViewSelectionMode, View, ViewContext, WindowBackdrop, WindowTheme,
    WindowTitleBarHeight, WindowVisuals,
};

use vault_app::commands::UiCommand;
use vault_app::navigation::{outline, NavSection};
use vault_app::settings::{PaneMode, ThemeChoice};
use vault_app::tree as kernel_tree;
use vault_md::parse_note;
use vault_platform::OpenTarget;

use crate::app::WindowsApp;
use crate::file_tree::FileTree;
use crate::reading::WindowsMarkdownRenderer;
use crate::reading_view::{doc_view, LinkNav};
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
    /// The vault's file tree: kernel `TreeNode`s plus `TreeState`
    /// (expansion/selection). Built once from the vault's note paths;
    /// expansion state persists across renders here.
    file_tree: FileTree,
    /// The tag panel's drill-down selection (shell-local view state, the
    /// same split as the file tree's expansion state): `None` shows the
    /// tag list, `Some(tag)` shows that tag's notes. Clicking the
    /// selected tag again clears it.
    selected_tag: Option<String>,
}

impl Component for Shell {
    type Input = ShellHandle;
    type Message = WindowsMessage;

    fn create(input: &Self::Input, _context: &ComponentContext<Self>) -> Self {
        let file_tree = FileTree::from_paths(&input.0.borrow().note_paths());
        Shell {
            handle: input.clone(),
            rendered_editor: RefCell::new(String::new()),
            file_tree,
            selected_tag: None,
        }
    }

    fn input_changed(&mut self, input: &Self::Input, _context: &ComponentContext<Self>) {
        self.handle = input.clone();
    }

    fn update(&mut self, message: Self::Message, _context: &ComponentContext<Self>) {
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

        // File-tree sidebar: one TreeView node per vault folder/note,
        // keyed by vault path. Invoking a note row asks the kernel to
        // open it (`UiCommand::OpenNote`); expansion state lives in
        // `FileTree`'s `TreeState` and persists across renders.
        let tree_view = TreeView::new()
            .selection_mode(TreeViewSelectionMode::Single)
            .on_item_invoked(
                context
                    .callback(|path: String| WindowsMessage::Command(UiCommand::OpenNote { path })),
            )
            .nodes(build_tree_nodes(&self.file_tree));

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

        // The active panel. Files is the default.
        let sidebar_panel = match state.panel {
            NavSection::Files => tree_view,
            NavSection::Search => search_panel,
            NavSection::Tags => tags_panel,
            NavSection::Links => links_panel,
            NavSection::Outline => outline_panel,
        };
        let sidebar = StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children([
                KeyedView::new("nav-row", nav_row),
                KeyedView::new("sidebar-panel", sidebar_panel),
            ]);

        // Tab bar: one activate button and one close button per tab,
        // keyed by tab id so reconciliation preserves identity as tabs
        // open and close. The active tab's label is bracketed.
        let tabs: Vec<KeyedView> = state
            .tabs
            .order()
            .iter()
            .map(|&tab| {
                let path = state.tabs.path(tab).unwrap_or("?").to_string();
                let is_active = state.tabs.active() == Some(tab);
                let label = if is_active {
                    format!("[{path}]")
                } else {
                    path.clone()
                };
                let position = state.tabs.position(tab).unwrap_or(0);
                let activate = Button::new()
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::ActivateTab { position })
                    }))
                    .content(TextBlock::new().text(label));
                let close = Button::new()
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::CloseTab { position })
                    }))
                    .content(TextBlock::new().text(format!("close {path}")));
                // The tab path doubles as the key: the kernel never
                // holds two tabs for one path (open_or_focus dedupes).
                KeyedView::new(
                    path.clone(),
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .children([activate, close]),
                )
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
        if show_source {
            let editor_text = active_buffer.map(|b| b.text.clone()).unwrap_or_default();
            *self.rendered_editor.borrow_mut() = editor_text.clone();
        }
        let source_editor = TextBox::new()
            .on_text_changed(context.callback(|text: String| {
                WindowsMessage::Native(NativeEvent::EditorEdited { text })
            }))
            .text(active_buffer.map(|b| b.text.clone()).unwrap_or_default());
        let reading_area = active_buffer
            .map(|buffer| {
                let doc = parse_note(&buffer.text);
                // Link clicks become kernel commands (plan §10.3): URLs go
                // through `OpenExternal` (the ExternalOpener service owns
                // the hand-off), wiki links open or focus their tab.
                let on_link = |nav: LinkNav| {
                    let handle = self.handle.clone();
                    context.callback(move |()| match nav.clone() {
                        LinkNav::Url(url) => WindowsMessage::Command(UiCommand::OpenExternal {
                            target: OpenTarget::Url(url),
                        }),
                        LinkNav::Note(path) => {
                            WindowsMessage::Command(UiCommand::OpenNote { path })
                        }
                        // Relative Markdown link: resolve against the
                        // active note's folder through the vault index
                        // (plan §10.3's `resolve_relative` flow). A
                        // resolved note opens a tab; anything else
                        // (attachments, unresolvable targets) is handed
                        // to the platform's default file handler.
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
                            if let Some(path) =
                                handle.0.borrow().runner.resolve_relative(&target, &from)
                            {
                                WindowsMessage::Command(UiCommand::OpenNote { path })
                            } else {
                                WindowsMessage::Command(UiCommand::OpenExternal {
                                    target: OpenTarget::File(std::path::PathBuf::from(&target)),
                                })
                            }
                        }
                        // A reading-view tag routes to the tags panel
                        // with the tag selected (shell-local filter).
                        LinkNav::Tag(tag) => {
                            WindowsMessage::Native(NativeEvent::TagToggled { tag })
                        }
                        // A footnote reference jumps to its definition:
                        // the block index was resolved at render time,
                        // so just anchor it (the Border marker).
                        LinkNav::Footnote(block) => {
                            WindowsMessage::Command(UiCommand::JumpOutline { block })
                        }
                    })
                };
                doc_view(
                    &doc,
                    &WindowsMarkdownRenderer::default(),
                    state.jump_anchor,
                    &on_link,
                )
            })
            .unwrap_or_else(|| View::from(TextBlock::new().text("(no note open)")));

        // Note area: everything that renders inside the SplitView's content
        // pane — the tab strip, the editor/reading view, and (in a later
        // slice) the status bar. The save button, the pane-mode switcher,
        // and the appearance row have moved into the title bar's toolbar.
        let mut note_area: Vec<KeyedView> = Vec::new();
        note_area.push(KeyedView::new(
            "tab-bar",
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .keyed_children(tabs),
        ));
        if show_source {
            note_area.push(KeyedView::new(
                "editor",
                ScrollViewer::new()
                    .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                    .content(source_editor),
            ));
        }
        if show_reading {
            note_area.push(KeyedView::new(
                "reading-view",
                ScrollViewer::new()
                    .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                    .content(reading_area),
            ));
        }
        let note_area = StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children(note_area);

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

/// Convert the kernel tree (folders + notes, expansion state from
/// `TreeState`) into the hierarchy the native TreeView renders. Folder
/// and note keys are their vault paths; invoking a note row reports its
/// path to `on_item_invoked`.
fn build_tree_nodes(tree: &FileTree) -> Vec<TreeNode> {
    fn convert(node: &kernel_tree::TreeNode, tree: &FileTree) -> TreeNode {
        match node {
            kernel_tree::TreeNode::Folder {
                name,
                path,
                children,
            } => TreeNode::new(path.as_str(), name.as_str())
                .expanded(tree.state.is_expanded(path))
                .children(children.iter().map(|child| convert(child, tree))),
            kernel_tree::TreeNode::Note { name, path } => {
                TreeNode::new(path.as_str(), name.as_str())
            }
        }
    }
    tree.nodes.iter().map(|node| convert(node, tree)).collect()
}
