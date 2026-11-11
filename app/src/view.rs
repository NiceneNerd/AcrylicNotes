//! View composition: chrome, sidebar panels (files/search/tags/links/outline),
//! tab bar, editor/reading panes, and the quick-switcher overlay.
//!
//! Pure functions of the model; every interactive element routes through
//! `Message`. Colors come from the active cosmic theme via `Palette`.

use cosmic::iced::core::widget::Id;
use cosmic::iced::keyboard::{key::Named, Key, Modifiers};
use cosmic::iced::widget::{opaque, stack, text_editor};
use cosmic::iced::{alignment, border, Background, Length, Padding};
use cosmic::widget::{
    button, column, container, context_menu, icon, menu, nav_bar, row, scrollable,
    segmented_button, settings, text, text_input, toggler,
};
use cosmic::Element;

use crate::app::{
    editor_shortcut, find_matches, parent_of, tab_label_parts, title_of, tree_note_label,
    unicode_char, unicode_entry_binding, LinkPathFormat, Message, Palette, PaneMode, SidePanel,
    SortMenuAction, TreeMenuAction, TreeSort, VaultNotes, ATTACHMENT_INPUT, EDITOR_FONT_INPUT,
    EDITOR_INPUT, FIND_INPUT, READING_FONT_INPUT, SIDEBAR_OPACITY, SWITCHER_INPUT, TREE_EDIT_INPUT,
};
use crate::editor_text::{
    editor_font, editor_highlight_format, editor_placeholder_color, editor_value_color,
    reading_font, set_editor_base_font,
};
use crate::reading;
use crate::scroll::READING_SCROLL;
use crate::tree::TreeNode;

pub fn view(app: &VaultNotes) -> Element<'_, Message> {
    if app.vault.is_none() {
        empty_state(app)
    } else {
        let palette = Palette::from(&cosmic::theme::active());
        let base = main_view(app, palette);
        if app.switcher.open {
            // The quick switcher floats above everything (Obsidian-style);
            // `opaque` keeps mouse clicks from reaching the panes below.
            stack([base, opaque(switcher_overlay(app, palette))]).into()
        } else {
            base
        }
    }
}

/// Header controls: save + pane mode. Empty until a vault is open.
pub fn header_start(app: &VaultNotes) -> Vec<Element<'_, Message>> {
    if app.vault.is_none() {
        return Vec::new();
    }
    vec![
        // Sidebar visibility toggle, far left like COSMIC's own apps.
        // The icon mirrors libcosmic's nav_bar_toggle: it shows the
        // current state (sidebar open -> open-navbar icon), and the
        // button reads selected while the sidebar is visible.
        button::icon(icon::from_name(if app.sidebar_open {
            "navbar-open-symbolic"
        } else {
            "navbar-closed-symbolic"
        }))
        .selected(app.sidebar_open)
        .on_press(Message::ToggleSidebar)
        .into(),
        button::icon(icon::from_name("document-save-symbolic"))
            .on_press(Message::Save)
            .into(),
        button::icon(icon::from_name("date-symbolic"))
            .on_press(Message::DailyNote)
            .into(),
        // Icon shows the theme a click switches to (both names verified in
        // the installed Pop and Cosmic icon themes).
        button::icon(icon::from_name(if cosmic::theme::is_dark() {
            "weather-clear-symbolic"
        } else {
            "weather-clear-night-symbolic"
        }))
        .on_press(Message::ToggleTheme)
        .into(),
        // The segmented control's default button padding is zero, which
        // cramps the labels.
        container(
            segmented_button::horizontal(&app.pane_model)
                .button_padding([
                    cosmic::theme::spacing().space_s,
                    cosmic::theme::spacing().space_xxs,
                    cosmic::theme::spacing().space_s,
                    cosmic::theme::spacing().space_xxs,
                ])
                .width(Length::Shrink)
                .on_activate(Message::PaneActivated),
        )
        .width(Length::Shrink) // leave room for the window title in the center
        .padding([2, 4])
        .into(),
        // Settings live in the right-side context drawer (Phase G).
        button::icon(icon::from_name("emblem-system-symbolic"))
            .on_press(Message::SettingsToggle)
            .into(),
    ]
}

// --- empty state -----------------------------------------------------------

fn empty_state(app: &VaultNotes) -> Element<'_, Message> {
    let palette = Palette::from(&cosmic::theme::active());
    let mut col = column::Column::new().spacing(16).max_width(460);
    col = col.push(icon::from_name("folder-symbolic").size(64));
    col = col.push(text::title2("Vault Notes"));
    col = col.push(text(
        "Open an Obsidian vault folder to begin. Files stay on disk; the \
         app only ever writes through its safe save path.",
    ));
    col = col.push(
        text_input("Vault folder path", &app.vault_input)
            .on_input(Message::VaultInput)
            .on_submit(|_| Message::VaultOpenSubmitted)
            .width(Length::Fill),
    );
    col = col.push(button::suggested("Open vault").on_press(Message::VaultOpenSubmitted));
    // The backdrop actually offered by this platform (M4c glass probe).
    col = col.push(text::caption(format!("Backdrop: {}", app.glass_style)));
    if let Some(error) = &app.error {
        col = col.push(text(error).class(palette.danger));
    }
    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into()
}

// --- main layout -----------------------------------------------------------

fn main_view<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new()
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(banner) = &app.banner {
        col = col.push(banner_view(banner, palette));
    }
    if let Some(pending) = &app.pending_links {
        col = col.push(link_updates_view(pending, palette));
    }
    if let Some(error) = &app.error {
        col = col.push(
            row::Row::new()
                .spacing(8)
                .push(text(error).class(palette.danger))
                .push(button::text("×").on_press(Message::ErrorDismiss)),
        );
    }
    let note = container(note_area(app, palette))
        .width(Length::Fill)
        .height(Length::Fill);
    col = col.push(
        row::Row::new()
            .push_maybe(app.sidebar_open.then(|| sidebar(app, palette)))
            .push(note),
    );
    col.into()
}

/// Non-blocking external-change banner (M4 decision 7). Two sections:
/// reloaded tabs are informational; conflicting tabs (unsaved edits vs.
/// disk) offer reload-discard, keep-local, or a deliberate overwrite.
fn banner_view<'a>(banner: &crate::app::Banner, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(4).width(Length::Fill);
    if !banner.conflicts.is_empty() {
        col = col.push(
            row::Row::new()
                .spacing(8)
                .push(
                    text(format!(
                        "Changed on disk while you edited: {}",
                        titles(&banner.conflicts)
                    ))
                    .class(palette.text),
                )
                .push(button::text("Reload from disk").on_press(Message::BannerReload))
                .push(button::text("Keep mine").on_press(Message::BannerDismiss))
                .push(button::suggested("Save mine anyway").on_press(Message::BannerOverwrite)),
        );
    }
    if !banner.reloaded.is_empty() {
        col = col.push(
            row::Row::new()
                .spacing(8)
                .push(
                    text(format!(
                        "Reloaded from disk (changed externally): {}",
                        titles(&banner.reloaded)
                    ))
                    .class(palette.muted),
                )
                .push(button::text("Dismiss").on_press(Message::BannerDismiss)),
        );
    }
    let background = if banner.conflicts.is_empty() {
        with_alpha(palette.muted, 0.15)
    } else {
        with_alpha(palette.warning, 0.18)
    };
    container(col)
        .padding(6)
        .width(Length::Fill)
        .style(move |_| cosmic::widget::container::Style {
            background: Some(Background::Color(background)),
            ..Default::default()
        })
        .into()
}

fn titles(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| title_of(path))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Non-blocking prompt after a rename/move invalidated links: the user
/// chooses whether to rewrite them (Obsidian asks first too). Dismissing
/// leaves the links as they are.
fn link_updates_view<'a>(
    pending: &crate::app::PendingLinkUpdates,
    palette: Palette,
) -> Element<'a, Message> {
    let (old, _) = pending.renames.first().cloned().unwrap_or_default();
    let noun = if old.ends_with(".md") {
        "note"
    } else {
        "folder"
    };
    let count = pending.referrers.len();
    let notes = if count == 1 { "note" } else { "notes" };
    let background = with_alpha(palette.warning, 0.18);
    container(
        row::Row::new()
            .spacing(8)
            .push(
                text(format!(
                    "Renamed {noun} \u{201C}{}\u{201D} \u{2014} {count} {notes} link to it.",
                    title_of(&old)
                ))
                .class(palette.text),
            )
            .push(button::suggested("Update links").on_press(Message::LinkUpdatesApply))
            .push(button::text("Leave as is").on_press(Message::LinkUpdatesDismiss)),
    )
    .padding(6)
    .width(Length::Fill)
    .style(move |_| cosmic::widget::container::Style {
        background: Some(Background::Color(background)),
        ..Default::default()
    })
    .into()
}

// --- sidebar ---------------------------------------------------------------

/// Sidebar: a COSMIC nav bar (icon + label per panel) over the active
/// panel's body — the same pattern COSMIC Files uses for its places list.
/// The nav bar's own solid background is cleared so the whole sidebar reads
/// as one surface (the nav bar's default style paints `primary.base`, which
/// split the sidebar into a solid strip over a translucent body).
/// The sidebar as a floating COSMIC panel: padded away from the window
/// edges and the note area, nearly opaque (a hint of the frosted backdrop
/// still reads through), with corners that follow the user's COSMIC
/// rounding setting (`radius_s`), so the panel goes square automatically
/// under the Sharp preference. Width is fixed; the panel is shown or
/// hidden whole via the titlebar toggle.
fn sidebar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(0).width(Length::Fill);
    col = col.push(
        nav_bar(&app.nav_model, Message::NavSelect)
            .into_container()
            // The default nav-bar container fills its parent, leaving a
            // blank solid area under the items; shrink to the items instead.
            .height(Length::Shrink)
            .class(cosmic::theme::Container::custom(|_theme| {
                cosmic::widget::container::Style {
                    background: None,
                    ..Default::default()
                }
            })),
    );
    let body: Element<'a, Message> = match app.panel {
        SidePanel::Files => tree_view(app, palette),
        SidePanel::Search => search_panel(app, palette),
        SidePanel::Tags => tags_panel(app, palette),
        SidePanel::Links => links_panel(app, palette),
        SidePanel::Outline => outline_panel(app, palette),
    };
    col = col.push(
        scrollable::scrollable(body)
            .id(Id::new(crate::drag::SCROLL_ID))
            .width(Length::Fill)
            .height(Length::Fill),
    );
    // The styled panel: the background paints over the container's own
    // padding, so the floating margins must live on an outer transparent
    // container. The shell already insets the whole content row by its
    // border padding, hence no left margin here.
    let panel = container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| {
            // `active()` clones the theme, so bind it before borrowing
            // the cosmic palette out of it.
            let theme = cosmic::theme::active();
            let cosmic = theme.cosmic();
            cosmic::widget::container::Style {
                // Almost opaque: the theme's opaque background base drawn
                // back a touch, so the frosted backdrop still whispers.
                background: Some(Background::Color(with_alpha(
                    cosmic::iced::Color::from(cosmic.background(false).base),
                    SIDEBAR_OPACITY,
                ))),
                border: border::Border {
                    radius: cosmic.radius_s().into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });
    let gap = f32::from(cosmic::theme::spacing().space_xxs);
    container(panel)
        .width(Length::Fixed(crate::app::SIDEBAR_WIDTH))
        .height(Length::Fill)
        .padding([gap, gap, gap, 0.0])
        .into()
}

fn tree_view<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(0).width(Length::Fill);
    // The toolbar: the sort order and the fold controls.
    col = col.push(tree_toolbar(app, palette));
    if app.tree.is_empty() && app.tree_folder_edit_target().is_none() {
        return container(text("No notes.").class(palette.muted))
            .padding(12)
            .into();
    }
    // A root-level new-note/new-folder input renders above the tree.
    if app.tree_folder_edit_target() == Some("") {
        col = col.push(tree_edit_row(app, 4.0));
    }
    col = col.push(nodes_view(&app.tree, 0, app, palette));
    // The tree area is the root drop region: highlighted while a drag
    // hovers it (drop = move to the vault root).
    let root_hover = app.tree_drag_hover() == Some("");
    container(col)
        .width(Length::Fill)
        .id(Id::new(crate::drag::AREA_ID))
        .style(move |_| {
            if root_hover {
                cosmic::widget::container::Style {
                    background: Some(Background::Color(with_alpha(palette.accent, 0.25))),
                    ..Default::default()
                }
            } else {
                cosmic::widget::container::Style::default()
            }
        })
        .into()
}

/// The Files panel's toolbar: the sort order and the fold controls.
///
/// The sort button shows the order in force and cycles through the four
/// on a click; a right click opens the menu, where the order in force is
/// checked. The two chevron buttons fold every folder away or open them
/// all again.
///
/// The row stays quiet on purpose: flat buttons, small icons, a muted
/// label, and the fold controls pushed to the far right, away from the
/// order they act on.
fn tree_toolbar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    // The path-aware controls need a note to work from: with none open,
    // they have no path to keep open.
    let has_note = app.active_tab_path().is_some();
    let sort_icon = if app.tree_sort.descending() {
        "view-sort-descending-symbolic"
    } else {
        "view-sort-ascending-symbolic"
    };
    let sort_button = button::custom(
        row::Row::new()
            .push(icon::from_name(sort_icon).size(14))
            .push(text(app.tree_sort.label()).size(12).class(palette.muted))
            .spacing(4),
    )
    .class(cosmic::theme::Button::Text)
    .on_press(Message::TreeSort(app.tree_sort.next()))
    .height(Length::Shrink)
    .width(Length::Shrink)
    .padding(Padding {
        left: 4.0,
        right: 4.0,
        top: 2.0,
        bottom: 2.0,
    });

    let sort: cosmic::Element<'a, Message> =
        context_menu(sort_button, Some(sort_menu(app.tree_sort)))
            .on_surface_action(Message::Surface)
            .into();

    row::Row::new()
        .push(sort)
        .push(cosmic::iced::widget::space::horizontal())
        .push(
            button::icon(icon::from_name("pan-end-symbolic"))
                .icon_size(14)
                .class(cosmic::theme::Button::Icon)
                .tooltip("Collapse all folders")
                .on_press(Message::TreeCollapseAll)
                .height(Length::Shrink),
        )
        .push(
            button::icon(icon::from_name("object-select-symbolic"))
                .icon_size(14)
                .class(cosmic::theme::Button::Icon)
                .tooltip("Collapse all but the current note's path")
                .on_press_maybe(has_note.then_some(Message::TreeCollapseOthers))
                .height(Length::Shrink),
        )
        .push(
            button::icon(icon::from_name("pan-down-symbolic"))
                .icon_size(14)
                .class(cosmic::theme::Button::Icon)
                .tooltip("Expand all folders")
                .on_press(Message::TreeExpandAll)
                .height(Length::Shrink),
        )
        .push(
            button::icon(icon::from_name("go-jump-symbolic"))
                .icon_size(14)
                .class(cosmic::theme::Button::Icon)
                .tooltip("Open the current note's path")
                .on_press_maybe(has_note.then_some(Message::TreeRevealCurrent))
                .height(Length::Shrink),
        )
        .spacing(2)
        .align_y(alignment::Vertical::Center)
        .padding(Padding {
            left: 4.0,
            right: 4.0,
            top: 2.0,
            bottom: 2.0,
        })
        .width(Length::Fill)
        .into()
}

/// The sort menu: the four orders, the one in force checked.
fn sort_menu(current: TreeSort) -> Vec<menu::Tree<Message>> {
    menu::items(
        &std::collections::HashMap::new(),
        TreeSort::ALL
            .into_iter()
            .map(|order| {
                menu::Item::CheckBox(order.label(), None, order == current, SortMenuAction(order))
            })
            .collect(),
    )
}

/// The style of a tree row while a drag is in flight: the hovered drop
/// target is accented, the dragged row dimmed, everything else plain.
fn row_drag_style(hover: bool, source: bool, palette: Palette) -> cosmic::widget::container::Style {
    if hover {
        cosmic::widget::container::Style {
            background: Some(Background::Color(with_alpha(palette.accent, 0.3))),
            ..Default::default()
        }
    } else if source {
        cosmic::widget::container::Style {
            background: Some(Background::Color(with_alpha(palette.muted, 0.35))),
            ..Default::default()
        }
    } else {
        cosmic::widget::container::Style::default()
    }
}

/// The shared right-click menu for a tree row: new note/folder beside the
/// target, then rename and delete (delete moves files to `.trash/`).
fn node_menu() -> Vec<menu::Tree<Message>> {
    menu::items(
        &std::collections::HashMap::new(),
        vec![
            menu::Item::Button("New note", None, TreeMenuAction::NewNote),
            menu::Item::Button("New folder", None, TreeMenuAction::NewFolder),
            menu::Item::Divider,
            menu::Item::Button("Rename", None, TreeMenuAction::Rename),
            menu::Item::Button("Delete", None, TreeMenuAction::Delete),
        ],
    )
}

/// The editor context menu's items as a `MenuAction` enum (cosmic menus
/// need a Copy action type; `Message` itself is neither Copy nor Eq).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorMenuAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl menu::action::MenuAction for EditorMenuAction {
    type Message = Message;

    fn message(&self) -> Message {
        match self {
            Self::Undo => Message::EditorUndo,
            Self::Redo => Message::EditorRedo,
            Self::Cut => Message::EditorCut,
            Self::Copy => Message::EditorCopy,
            Self::Paste => Message::EditorPasteAsk,
            Self::SelectAll => Message::EditorSelectAll,
        }
    }
}

/// The right-click menu for the editor pane: undo/redo, then clipboard
/// operations and select-all. Copy/Cut/Paste go through app messages
/// (`iced::clipboard` read is async; write is an effect task), so they
/// route through the same undo/dirty bookkeeping as typed edits.
fn editor_menu() -> Vec<menu::Tree<Message>> {
    menu::items(
        &std::collections::HashMap::new(),
        vec![
            menu::Item::Button("Undo", None, EditorMenuAction::Undo),
            menu::Item::Button("Redo", None, EditorMenuAction::Redo),
            menu::Item::Divider,
            menu::Item::Button("Cut", None, EditorMenuAction::Cut),
            menu::Item::Button("Copy", None, EditorMenuAction::Copy),
            menu::Item::Button("Paste", None, EditorMenuAction::Paste),
            menu::Item::Divider,
            menu::Item::Button("Select All", None, EditorMenuAction::SelectAll),
        ],
    )
}

/// Wrap a tree row in the right-click menu; the opened menu records its
/// target so a later `TreeMenu` command knows what to act on.
fn menu_row<'a>(
    row: impl Into<cosmic::Element<'a, Message>>,
    path: &str,
) -> cosmic::Element<'a, Message> {
    context_menu(row, Some(node_menu()))
        .on_open(Message::TreeMenuTarget(Some(path.to_string())))
        .on_close(Message::TreeMenuTarget(None))
        .on_surface_action(Message::Surface)
        .into()
}

/// The inline name input for new-note / new-folder / rename edits.
fn tree_edit_row<'a>(app: &'a VaultNotes, indent: f32) -> cosmic::Element<'a, Message> {
    let value = app
        .tree_edit
        .as_ref()
        .map(|edit| edit.value.as_str())
        .unwrap_or("");
    container(
        text_input("Name", value)
            .id(Id::new(TREE_EDIT_INPUT))
            .on_input(Message::TreeEditInput)
            .on_submit(|_| Message::TreeEditSubmit)
            .width(Length::Fill),
    )
    .padding(Padding {
        left: indent + 10.0,
        right: 8.0,
        ..Padding::ZERO
    })
    .width(Length::Fill)
    .into()
}

fn nodes_view<'a>(
    nodes: &'a [TreeNode],
    depth: usize,
    app: &'a VaultNotes,
    palette: Palette,
) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(0).width(Length::Fill);
    for node in nodes {
        let indent = 4.0 + 14.0 * depth as f32;
        match node {
            TreeNode::Folder {
                name,
                path,
                children,
            } => {
                let chevron = if app.collapsed.contains(path) {
                    "pan-end-symbolic"
                } else {
                    "pan-down-symbolic"
                };
                let row = if app.tree_rename_target() == Some(path.as_str()) {
                    tree_edit_row(app, indent)
                } else {
                    button::custom(
                        row::Row::new()
                            .push(icon::from_name(chevron).size(14))
                            .push(icon::from_name("folder-symbolic").size(16))
                            .push(text(name))
                            .spacing(6),
                    )
                    .class(cosmic::theme::Button::Text)
                    .on_press(Message::ToggleFolder(path.clone()))
                    .height(Length::Shrink)
                    .width(Length::Fill)
                    .into()
                };
                let hover = app.tree_drag_hover() == Some(path.as_str());
                let source = app.tree_drag_source() == Some(path.as_str());
                col = col.push(
                    container(menu_row(row, path))
                        .id(crate::drag::row_id(true, path))
                        .padding(Padding {
                            left: indent,
                            ..Padding::ZERO
                        })
                        .width(Length::Fill)
                        .style(move |_| row_drag_style(hover, source, palette)),
                );
                if !app.collapsed.contains(path) {
                    // New-note/new-folder inputs render as the folder's first row.
                    if app.tree_folder_edit_target() == Some(path.as_str()) {
                        col = col.push(tree_edit_row(app, indent + 14.0));
                    }
                    col = col.push(nodes_view(children, depth + 1, app, palette));
                }
            }
            TreeNode::Note { name, path } => {
                // Non-markdown files (attachments) open with the platform
                // handler: no rename row, dirty dots, or context menu —
                // renaming wouldn't rewrite links to them anyway.
                let is_note = path.ends_with(".md");
                let row = if is_note && app.tree_rename_target() == Some(path.as_str()) {
                    tree_edit_row(app, indent + 10.0)
                } else if !is_note {
                    button::custom(
                        row::Row::new()
                            .push(icon::from_name("mail-attachment-symbolic").size(16))
                            .push(text(name.to_owned()))
                            .spacing(6),
                    )
                    .class(cosmic::theme::Button::Text)
                    .on_press(Message::OpenAttachment(path.clone()))
                    .height(Length::Shrink)
                    .width(Length::Fill)
                    .into()
                } else {
                    let mut label = tree_note_label(name, app.hide_md_extension);
                    if app.active_tab_path() == Some(path.as_str()) {
                        label = format!("● {label}");
                    } else if app.is_tab_dirty(path) {
                        label = format!("• {label}");
                    }
                    button::custom(
                        row::Row::new()
                            .push(icon::from_name("text-x-generic-symbolic").size(16))
                            .push(text(label))
                            .spacing(6),
                    )
                    .class(cosmic::theme::Button::Text)
                    .on_press(Message::TabOpen(path.clone()))
                    .height(Length::Shrink)
                    .width(Length::Fill)
                    .into()
                };
                let mut row_container = container(if is_note { menu_row(row, path) } else { row })
                    .padding(Padding {
                        left: indent + 10.0,
                        ..Padding::ZERO
                    })
                    .width(Length::Fill);
                if is_note {
                    // Notes are draggable; attachments are not (they carry
                    // no row id, so a press on them never arms a drag).
                    let hover = app.tree_drag_hover() == Some(path.as_str());
                    let source = app.tree_drag_source() == Some(path.as_str());
                    row_container = row_container
                        .id(crate::drag::row_id(false, path))
                        .style(move |_| row_drag_style(hover, source, palette));
                }
                col = col.push(row_container);
            }
        }
    }
    col.into()
}

/// Vault-wide literal-ripgrep search (M4 decision 4): query box + flat hit
/// list; a hit opens its note and scrolls the reading view to the match.
fn search_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new()
        .spacing(6)
        .padding(8)
        .width(Length::Fill);
    col = col.push(
        text_input("Search notes…", &app.search_query)
            .on_input(Message::SearchInput)
            .on_submit(|_| Message::SearchSubmitted)
            .width(Length::Fill),
    );
    col = col.push(button::suggested("Search").on_press(Message::SearchSubmitted));
    if app.search_hits.is_empty() {
        if !app.search_query.trim().is_empty() {
            col = col.push(text("No matches.").class(palette.muted));
        }
    } else {
        col = col.push(text(format!("{} matches", app.search_hits.len())).class(palette.muted));
        for (i, hit) in app.search_hits.iter().enumerate() {
            col = col.push(
                button::text(title_of(&hit.path))
                    .on_press(Message::SearchOpen(i))
                    .height(Length::Shrink)
                    .width(Length::Fill),
            );
            col = col.push(
                text(format!("{}:{}", hit.path, hit.line_no))
                    .size(11)
                    .class(palette.muted),
            );
            col = col.push(text(snippet(&hit.line)).size(12).class(palette.muted));
        }
    }
    col.into()
}

/// Tag index: all tags with note counts, or the notes of the selected tag.
fn tags_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(vault) = app.vault.as_ref() else {
        return panel_hint("No vault open.", palette);
    };
    let mut col = column::Column::new()
        .spacing(4)
        .padding(8)
        .width(Length::Fill);
    match &app.selected_tag {
        None => {
            let mut tags = vault.all_tags();
            tags.sort();
            if tags.is_empty() {
                col = col.push(text("No tags in this vault.").class(palette.muted));
            }
            for tag in tags {
                let count = vault.notes_for_tag(&tag).len();
                col = col.push(
                    button::text(format!("#{tag} ({count})"))
                        .on_press(Message::TagSelected(tag))
                        .height(Length::Shrink)
                        .width(Length::Fill),
                );
            }
        }
        Some(tag) => {
            // Clicking the selected tag again clears the filter (update).
            col = col.push(
                button::text(format!("← #{tag}"))
                    .on_press(Message::TagSelected(tag.clone()))
                    .height(Length::Shrink)
                    .width(Length::Fill),
            );
            let mut notes = vault.notes_for_tag(tag);
            notes.sort();
            for path in notes {
                col = col.push(
                    button::text(title_of(&path))
                        .on_press(Message::TabOpen(path))
                        .height(Length::Shrink)
                        .width(Length::Fill),
                );
            }
        }
    }
    col.into()
}

/// Backlinks of the active note (nearest-first resolution already done by
/// vault-core's index).
fn links_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(vault) = app.vault.as_ref() else {
        return panel_hint("No vault open.", palette);
    };
    let Some(path) = app.active_tab_path() else {
        return panel_hint("Open a note to see what links to it.", palette);
    };
    let mut col = column::Column::new()
        .spacing(4)
        .padding(8)
        .width(Length::Fill);
    col = col.push(text(format!("Links to “{}”", title_of(path))).class(palette.muted));
    let mut backlinks = vault.backlinks(path);
    backlinks.sort();
    if backlinks.is_empty() {
        col = col.push(text("No backlinks.").class(palette.muted));
    }
    for source in backlinks {
        col = col.push(
            button::text(title_of(&source))
                .on_press(Message::TabOpen(source))
                .height(Length::Shrink)
                .width(Length::Fill),
        );
    }
    col.into()
}

/// Headings of the active note; a click scrolls the reading view to it.
fn outline_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(idx) = app.active else {
        return panel_hint("Open a note to see its outline.", palette);
    };
    let tab = &app.tabs[idx];
    let mut col = column::Column::new()
        .spacing(4)
        .padding(8)
        .width(Length::Fill);
    if tab.outline.is_empty() {
        col = col.push(text("No headings.").class(palette.muted));
    }
    for entry in &tab.outline {
        let depth = (entry.level as usize).saturating_sub(1).min(4);
        col = col.push(
            container(
                button::text(entry.text.clone())
                    .on_press(Message::JumpToBlock(entry.block))
                    .height(Length::Shrink)
                    .width(Length::Fill),
            )
            .padding(Padding {
                left: 14.0 * depth as f32,
                ..Padding::ZERO
            })
            .width(Length::Fill),
        );
    }
    col.into()
}

fn panel_hint(message: &str, palette: Palette) -> Element<'static, Message> {
    container(text(message.to_string()).class(palette.muted))
        .padding(12)
        .into()
}

/// Trim a matched line for the hit list.
fn snippet(line: &str) -> String {
    const MAX: usize = 64;
    let line = line.trim();
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        let cut: String = line.chars().take(MAX).collect();
        format!("{cut}…")
    }
}

// --- quick switcher --------------------------------------------------------

/// Modal fuzzy-file-finder overlay (Ctrl+P). Keys are routed in `update` via
/// the global keyboard subscription; Enter activates, arrows move, Esc closes.
fn switcher_overlay<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(6).width(Length::Fill);
    col = col.push(
        text_input("Jump to note…", &app.switcher.query)
            .id(Id::new(SWITCHER_INPUT))
            .on_input(Message::SwitcherInput)
            .on_submit(|_| Message::SwitcherActivate)
            .width(Length::Fill),
    );
    if app.switcher.items.is_empty() {
        col = col.push(text("No matching notes.").class(palette.muted));
    }
    for (i, path) in app.switcher.items.iter().enumerate() {
        let selected = app.switcher.selected == i;
        let mut label = title_of(path);
        if selected {
            label = format!("▸ {label}");
        }
        let btn = if selected {
            button::suggested(label)
        } else {
            button::text(label)
        }
        .on_press(Message::SwitcherSelect(i));
        // The note's parentage rides along, dimmed, so same-named notes in
        // different folders are distinguishable at a glance.
        let parent = parent_of(path);
        let mut row = row::Row::new().push(btn.width(Length::Shrink).height(Length::Shrink));
        if !parent.is_empty() {
            row = row.push(text(parent).size(12).class(palette.muted));
        }
        col = col.push(row.width(Length::Fill));
    }
    container(container(col).padding(12).max_width(560).style(move |_| {
        cosmic::widget::container::Style {
            // Slightly translucent so the frosted-window blur (when the
            // compositor provides it) shows through the dialog.
            background: Some(Background::Color(with_alpha(palette.surface, 0.88))),
            border: border::Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }))
    .padding(60)
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Top)
    .style(move |_| cosmic::widget::container::Style {
        background: Some(Background::Color(with_alpha(
            cosmic::iced::Color::BLACK,
            0.25,
        ))),
        ..Default::default()
    })
    .into()
}

// --- note area -------------------------------------------------------------

fn note_area<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(idx) = app.active else {
        return container(
            text("Open a note from the sidebar, or click a link in reading view.")
                .class(Palette::from(&cosmic::theme::active()).muted),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into();
    };
    let tab = &app.tabs[idx];

    let mut col = column::Column::new()
        .width(Length::Fill)
        .height(Length::Fill);
    col = col.push(tab_bar(app, palette));
    if app.find.is_some() {
        col = col.push(find_bar(app, palette));
    }
    let content: Element<'_, Message> = match app.pane {
        PaneMode::Live => live_pane(app, tab, palette),
        PaneMode::Source => editor_pane(app, tab, palette, app.wiki_complete.as_ref()),
        PaneMode::Reading => reading_pane(app, tab, palette),
        PaneMode::Split => row::Row::new()
            .push(
                container(editor_pane(app, tab, palette, app.wiki_complete.as_ref()))
                    .width(Length::FillPortion(1)),
            )
            .push(container(reading_pane(app, tab, palette)).width(Length::FillPortion(1)))
            .into(),
    };
    col = col.push(container(content).width(Length::Fill).height(Length::Fill));
    col = col.push(status_bar(app, palette));
    col.into()
}

/// The open-note tab strip.
///
/// Each tab shows the note's title and, dimmed beside it, the folder the
/// note lives in — two notes with the same name are told apart at a
/// glance. The folder is cut off with an ellipsis when it does not fit.
fn tab_bar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    if app.tabs.is_empty() {
        return container(text("No open tabs").class(palette.muted))
            .padding(4)
            .width(Length::Fill)
            .into();
    }
    let mut strip = row::Row::new()
        .spacing(2)
        .align_y(alignment::Vertical::Center);
    for (idx, tab) in app.tabs.iter().enumerate() {
        strip = strip.push(tab_button(tab, idx, app.active == Some(idx), palette));
    }
    // More tabs than fit: the strip scrolls sideways.
    scrollable::horizontal(strip)
        .width(Length::Fill)
        .height(Length::Shrink)
        .into()
}

/// One tab: the note's title, its folder dimmed beside it, and a close
/// button. The active tab gets a tinted background.
fn tab_button<'a>(
    tab: &'a crate::app::Tab,
    idx: usize,
    active: bool,
    palette: Palette,
) -> Element<'a, Message> {
    let (title, folder) = tab_label_parts(tab);
    let mut label = row::Row::new()
        .spacing(6)
        .align_y(alignment::Vertical::Center);
    label = label.push(text(title));
    if !folder.is_empty() {
        label = label.push(
            text(folder.clone())
                .size(12)
                .class(palette.muted)
                .width(folder_width(&folder))
                // A long path is cut off instead of pushing the tab off
                // the strip.
                .ellipsize(cosmic::iced::core::text::Ellipsize::End(
                    cosmic::iced::core::text::EllipsizeHeightLimit::Lines(1),
                )),
        );
    }
    let label = button::custom(label)
        .class(cosmic::theme::Button::Text)
        .on_press(Message::TabActivated(idx))
        .padding([4, 8])
        .height(Length::Shrink);
    let close = button::icon(icon::from_name("window-close-symbolic"))
        .icon_size(12)
        .class(cosmic::theme::Button::Icon)
        .on_press(Message::TabCloseRequested(idx))
        .height(Length::Shrink);
    container(
        row::Row::new()
            .spacing(0)
            .align_y(alignment::Vertical::Center)
            .push(label)
            .push(close),
    )
    .style(move |_| cosmic::widget::container::Style {
        background: active.then(|| Background::Color(with_alpha(palette.accent, 0.14))),
        ..Default::default()
    })
    .into()
}

/// The width the folder beside a tab's title may take.
///
/// A short folder takes its natural width, so a tab stays as narrow as
/// its content. A long one is given a fixed column and cut off with an
/// ellipsis, so a deep vault does not push the tab off the strip.
fn folder_width(folder: &str) -> Length {
    const LONG_FOLDER_CHARS: usize = 16;
    const FOLDER_COLUMN: f32 = 120.0;
    if folder.chars().count() > LONG_FOLDER_CHARS {
        Length::Fixed(FOLDER_COLUMN)
    } else {
        Length::Shrink
    }
}

/// The in-note find bar (Ctrl+F): query, live count, prev/next, case
/// toggle, close — plus a replace row. Keys are routed globally while it
/// is open (Enter next, Shift+Enter previous, Esc close).
fn find_bar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let find = app.find.as_ref().expect("find bar rendered without state");
    let buffer = app
        .active_tab()
        .map(|tab| tab.content.text())
        .unwrap_or_default();
    let matches = find_matches(&buffer, &find.query, find.case_sensitive);
    let count = if find.query.is_empty() {
        "—".to_string()
    } else if matches.is_empty() {
        "0 of 0".to_string()
    } else {
        format!(
            "{} of {}",
            find.current.min(matches.len() - 1) + 1,
            matches.len()
        )
    };

    let case_btn = if find.case_sensitive {
        button::suggested("Aa")
    } else {
        button::text("Aa")
    };

    let mut col = column::Column::new().spacing(4);
    col = col.push(
        row::Row::new()
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .push(
                text_input("Find", &find.query)
                    .id(Id::new(FIND_INPUT))
                    .on_input(Message::FindInput)
                    .width(Length::Fill),
            )
            .push(text(count).size(12).class(palette.muted))
            .push(button::icon(icon::from_name("go-up-symbolic")).on_press(Message::FindPrev))
            .push(button::icon(icon::from_name("go-down-symbolic")).on_press(Message::FindNext))
            .push(
                case_btn
                    .on_press(Message::FindToggleCase)
                    .height(Length::Shrink),
            )
            .push(
                button::icon(icon::from_name("window-close-symbolic")).on_press(Message::FindClose),
            ),
    );
    col = col.push(
        row::Row::new()
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .push(
                text_input("Replace", &find.replacement)
                    .on_input(Message::FindReplaceInput)
                    .width(Length::Fill),
            )
            .push(button::text("Replace").on_press(Message::FindReplace))
            .push(button::text("All").on_press(Message::FindReplaceAll)),
    );
    container(col).padding([6, 12]).width(Length::Fill).into()
}

// --- panes -----------------------------------------------------------------

/// The text_editor surface style shared by the source editor and the
/// live preview's focused block. Cosmic's default text_editor style
/// paints an opaque bg_color, which hides the pane's translucent
/// overlay (and any compositor frosted-window blur behind it). Keep
/// only the border and text colors; the pane container's dark overlay
/// is the visible editor surface. Text colors come from contrast.rs so
/// they stay readable on any theme — including custom palettes whose
/// `neutral_9` is a dim mid-gray (measured at 2.15:1 before the fix).
fn editor_surface_style(theme: &cosmic::Theme, status: text_editor::Status) -> text_editor::Style {
    let cosmic = theme.cosmic();
    let border_color = match status {
        text_editor::Status::Focused { .. } => cosmic.accent.base,
        _ => cosmic.bg_divider(),
    };
    text_editor::Style {
        background: Background::Color(cosmic::iced::Color::TRANSPARENT),
        border: border::Border {
            radius: cosmic.corner_radii.radius_0.into(),
            width: 1.0,
            color: border_color.into(),
        },
        placeholder: editor_placeholder_color(theme),
        value: editor_value_color(theme),
        selection: cosmic.accent.base.into(),
    }
}

fn editor_pane<'a>(
    app: &'a VaultNotes,
    tab: &'a crate::app::Tab,
    palette: Palette,
    wiki: Option<&'a crate::app::WikiComplete>,
) -> Element<'a, Message> {
    // The editor's font: custom family if set, else the monospace
    // toggle's choice. It is parked for the highlighter so highlighted
    // spans (italics, bold) share it instead of iced's hardcoded
    // Monospace base.
    let base_font = editor_font(app.editor_font_mono, &app.editor_font_family);
    set_editor_base_font(base_font);
    // Syntect Markdown grammar via iced's highlighter (M4 decision 3);
    // theme follows the system light/dark preference.
    let highlighter_theme = if palette.is_dark {
        cosmic::iced::highlighter::Theme::Base16Ocean
    } else {
        cosmic::iced::highlighter::Theme::InspiredGitHub
    };
    let editor = text_editor(&tab.content)
        .id(Id::new(EDITOR_INPUT))
        .height(Length::Fill)
        .on_action(Message::EditorAction)
        // Inset the text from the editor's border.
        .padding(8)
        // Font family, size, and wrapping are user preferences (Phase G).
        .font(base_font)
        .size(f32::from(app.editor_font_size))
        .wrapping(if app.word_wrap {
            cosmic::iced::core::text::Wrapping::Word
        } else {
            cosmic::iced::core::text::Wrapping::None
        })
        .highlight_with::<crate::highlight::MarkdownHighlighter>(
            cosmic::iced::highlighter::Settings {
                theme: highlighter_theme,
                token: "md".to_string(),
            },
            // Token colors are contrast-checked against the editor
            // surface (see contrast.rs): dim syntax colors on custom
            // themes are lifted to the AA bar, strong ones unchanged.
            // Wikilinks are painted on top (see highlight.rs).
            editor_highlight_format,
        )
        // Editor-scoped shortcuts: undo/redo, formatting, footnotes,
        // word delete. Anything else falls back to the editor's own
        // bindings (copy/cut/paste/select-all, arrows, …).
        .key_binding(move |press: text_editor::KeyPress| {
            editor_key_binding(press, wiki.is_some(), app.unicode_input.is_some())
        })
        .style(editor_surface_style);
    let surface: Element<'a, Message> = container(editor)
        .padding(12)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| cosmic::widget::container::Style {
            // Darker editor overlay (M4 decision 6): darkens the surface now,
            // and its translucency is what shows the compositor's frosted-window
            // blur through once that is active (COSMIC transparency setting).
            background: Some(Background::Color(with_alpha(
                cosmic::iced::Color::BLACK,
                0.18,
            ))),
            ..Default::default()
        })
        .into();
    // The popup layer is ALWAYS the stack's second child, even when no
    // popup is open (an empty placeholder takes its slot). If the pane
    // instead switched between a bare container and a stack, iced would
    // see a widget-type change, rebuild the subtree, and drop the
    // editor's focus state — leaving it deaf to the keyboard until it
    // is clicked again (the "can't type while the dropdown is open"
    // bug). The EDITOR_INPUT id additionally protects focus across the
    // structural changes outside this pane.
    let overlay: cosmic::Element<'a, Message> = match (&app.unicode_input, wiki) {
        (Some(_), _) => unicode_entry_overlay(app, palette),
        (None, Some(wiki)) => wiki_dropdown(wiki, palette),
        (None, None) => container(column::Column::new()).into(),
    };
    let pane: cosmic::Element<'a, Message> = stack([surface, overlay]).into();
    // Right-click anywhere on the pane opens the editor menu.
    context_menu(pane, Some(editor_menu()))
        .on_surface_action(Message::Surface)
        .into()
}

/// The wikilink suggestion dropdown: a small popup anchored to the editor's
/// bottom edge (iced exposes no caret pixel position). Rows accept with a
/// click; keyboard handling (Tab / Ctrl+Tab / Esc) is routed globally.
fn wiki_dropdown<'a>(wiki: &'a crate::app::WikiComplete, palette: Palette) -> Element<'a, Message> {
    // The column is Fill-width so the popup container inherits a fluid
    // (uncompressed) width: a Shrink popup container puts the flex
    // resolver into cross-compression, which lays every Fill-width row
    // out at accumulated width 0 — a thin, tall, unreadable sliver.
    // (The quick switcher avoids this the same way.)
    let mut col = column::Column::new()
        .spacing(2)
        .padding(6)
        .width(Length::Fill);
    for (i, path) in wiki.items.iter().enumerate() {
        // Items are note paths; the row shows the title, with the note's
        // folder dimmed so duplicate titles are distinguishable.
        let label = title_of(path);
        let btn = if i == wiki.selected {
            button::suggested(label)
        } else {
            button::text(label)
        }
        .on_press(Message::WikiSelect(i));
        let mut row = row::Row::new()
            .push(btn.width(Length::Shrink).height(Length::Shrink))
            .align_y(alignment::Vertical::Center);
        let parent = parent_of(path);
        if !parent.is_empty() {
            row = row.push(text(parent).size(12).class(palette.muted));
        }
        col = col.push(row.width(Length::Fill).height(Length::Shrink));
    }
    // The outer container fills the editor area so the popup can anchor to
    // its bottom edge; the inner one is the popup itself (capped wide).
    container(container(col).max_width(280).style(popup_style(palette)))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Bottom)
        .padding([0, 12, 4, 12])
        .into()
}

fn reading_pane<'a>(
    app: &'a VaultNotes,
    tab: &'a crate::app::Tab,
    palette: Palette,
) -> Element<'a, Message> {
    let Some(vault) = app.vault.as_ref() else {
        return container(text("No vault open.")).into();
    };
    let ctx = reading::Ctx {
        vault,
        index: &app.index,
        from: tab.path.clone(),
        strict_breaks: app.strict_breaks,
        palette,
        font: reading_font(&app.reading_font_family),
        size: f32::from(app.reading_font_size),
        folds: &tab.folds,
        images: &app.images,
    };
    // The id anchors outline/search/footnote jumps (see scroll.rs). The
    // document column is centered at a readable measure (matching the web
    // view's ~46rem) instead of stretching edge to edge. Padding goes on
    // the wrapping container: a Scrollable's own `padding` only insets its
    // scrollbar, not its content. The margin around the column is the
    // user's choice (None / Compact / Comfortable / Spacious).
    //
    // A click in the reading view marks it as the pane in focus, so the
    // zoom shortcuts act on its font size.
    container(
        cosmic::iced::widget::mouse_area(
            scrollable::scrollable(
                container(reading::render_doc(&tab.doc, &ctx))
                    .max_width(736)
                    .width(Length::Fill)
                    .align_x(alignment::Horizontal::Center),
            )
            .id(Id::new(READING_SCROLL))
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .on_press(Message::ReadingClicked),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    // Horizontal margin only: left and right. Vertical padding would
    // push the document away from the pane's top and bottom edges for
    // no reading benefit.
    .padding([
        0.0,
        app.reading_margin.padding(),
        0.0,
        app.reading_margin.padding(),
    ])
    .into()
}

/// Live preview (`Live` pane mode): the note renders like the reading
/// pane — the same renderer, the same callouts, folds and embeds —
/// except the block being edited, which is a raw-source text editor
/// over that block's source. Click a block to edit it; Escape commits
/// it back to the render.
fn live_pane<'a>(
    app: &'a VaultNotes,
    tab: &'a crate::app::Tab,
    palette: Palette,
) -> Element<'a, Message> {
    let Some(vault) = app.vault.as_ref() else {
        return container(text("No vault open.")).into();
    };
    let ctx = reading::Ctx {
        vault,
        index: &app.index,
        from: tab.path.clone(),
        strict_breaks: app.strict_breaks,
        palette,
        font: reading_font(&app.reading_font_family),
        size: f32::from(app.reading_font_size),
        folds: &tab.folds,
        images: &app.images,
    };
    let slices = crate::live::slices(&tab.content.text(), &tab.doc);
    // Callout fold ids are assigned sequentially across the pass, matching
    // render_doc's traversal, so they line up with folds taken on this pane
    // (and stay unique between renders — blocks, callout bodies, embeds).
    let mut fold_id = 0u64;
    let mut col = column::Column::new().spacing(10).width(Length::Fill);
    for (region, slice) in slices.iter().enumerate() {
        col = col.push(live_region(
            app,
            tab,
            &ctx,
            palette,
            slice,
            region,
            &mut fold_id,
        ));
    } // The id anchors the scroll-into-view work (R3).
    let scroll = scrollable::scrollable(col)
        .id(Id::new(crate::scroll::LIVE_SCROLL))
        .width(Length::Fill)
        .height(Length::Fill);
    // The wiki dropdown is ALWAYS the stack's second child, even closed
    // (an empty placeholder takes its slot) — a structural change would
    // rebuild the subtree and drop the focused editor's key state, the
    // same trap the source editor's popup avoids.
    let wiki = app.wiki_complete.as_ref();
    let overlay: cosmic::Element<'a, Message> = match wiki {
        Some(wiki) => wiki_dropdown(wiki, palette),
        None => container(column::Column::new()).into(),
    };
    let pane: cosmic::Element<'a, Message> = stack([scroll.into(), overlay]).into();
    container(pane)
        .width(Length::Fill)
        .height(Length::Fill)
        // Horizontal margin only: left and right (see reading_pane).
        .padding([
            0.0,
            app.reading_margin.padding(),
            0.0,
            app.reading_margin.padding(),
        ])
        .into()
}

/// One region of the live preview: either the focused block's raw
/// editor, or the block rendered for clicking (the frontmatter and a
/// blank note's body get their own standing regions).
fn live_region<'a>(
    app: &'a VaultNotes,
    tab: &'a crate::app::Tab,
    ctx: &reading::Ctx<'a>,
    palette: Palette,
    slice: &crate::live::BlockSlice,
    region: usize,
    fold_id: &mut u64,
) -> cosmic::Element<'a, Message> {
    let inner: cosmic::Element<'a, Message> =
        if app.live.as_ref().is_some_and(|state| state.block == region) {
            focused_block_editor(app, palette, region)
        } else if let Some(block) = slice.block.and_then(|b| tab.doc.blocks.get(b)) {
            // The block list guards embed cycles (`stack`); the region's own
            // source line mapping is `slice`, not the block's, because the
            // buffer edits re-slice after every keystroke.
            let rendered = reading::render_block(block, ctx, 0, &stack_from(ctx), fold_id);
            cosmic::iced::widget::mouse_area(rendered)
                .on_press(Message::LiveFocus(region))
                .into()
        } else if tab.doc.frontmatter_raw.is_some() {
            // The frontmatter region (first region whenever it exists).
            let rendered =
                reading::render_frontmatter(&tab.doc, ctx).expect("frontmatter exists per the doc");
            cosmic::iced::widget::mouse_area(rendered)
                .on_press(Message::LiveFocus(region))
                .into()
        } else {
            // A note with no markdown blocks: one body region, standing open
            // (ensure_live_focus focuses it) or inviting a click.
            let placeholder: cosmic::Element<'a, Message> =
                if crate::live::content_end(&slice.source(&tab.content.text())) == (0, 0) {
                    container(text("Start writing…").class(palette.muted))
                        .width(Length::Fill)
                        .padding([12, 0])
                        .into()
                } else {
                    container(text("").size(0))
                        .width(Length::Fill)
                        .height(Length::Shrink)
                        .into()
                };
            cosmic::iced::widget::mouse_area(placeholder)
                .on_press(Message::LiveFocus(region))
                .into()
        };
    // The transparent wrapper carries the region anchor used by the
    // scroll-into-view op (`to_live_region`). Inner ids (the focused
    // editor's per-region `live_editor_id`, heading anchors) piggyback
    // inside it.
    container(inner)
        .id(crate::scroll::live_region_id(region))
        .width(Length::Fill)
        .into()
}

/// The embed-cycle guard stack render_doc seeds: the note itself.
fn stack_from<'a>(ctx: &reading::Ctx<'a>) -> Vec<String> {
    vec![ctx.from.clone()]
}

/// The focused region's raw-source editor: one buffer, one keystroke
/// from the note text (see `live_action`). `Shrink` height lets the
/// buffer grow with its content while the surrounding blocks re-render.
fn focused_block_editor<'a>(
    app: &'a VaultNotes,
    palette: Palette,
    region: usize,
) -> Element<'a, Message> {
    let live = app
        .live
        .as_ref()
        .expect("focused editor without live state");
    let wiki_open = app.wiki_complete.is_some();
    let base_font = editor_font(app.editor_font_mono, &app.editor_font_family);
    set_editor_base_font(base_font);
    // Same highlighter and surface as the source editor (M4 decision 3);
    // theme follows the system light/dark preference.
    let highlighter_theme = if palette.is_dark {
        cosmic::iced::highlighter::Theme::Base16Ocean
    } else {
        cosmic::iced::highlighter::Theme::InspiredGitHub
    };
    text_editor(&live.editor)
        // Per-region id (see `scroll::live_editor_id`): a constant id
        // here panics when focus hops between regions, because iced's
        // by-id state restore would resurrect the old node at the old
        // index while the editor renders at the new one.
        .id(crate::scroll::live_editor_id(region))
        // The widget auto-fits: `min_bounds()` tracks the buffer.
        .height(Length::Shrink)
        .on_action(Message::EditorAction)
        .padding(8)
        .font(base_font)
        .size(f32::from(app.editor_font_size))
        .wrapping(if app.word_wrap {
            cosmic::iced::core::text::Wrapping::Word
        } else {
            cosmic::iced::core::text::Wrapping::None
        })
        .highlight_with::<crate::highlight::MarkdownHighlighter>(
            cosmic::iced::highlighter::Settings {
                theme: highlighter_theme,
                token: "md".to_string(),
            },
            editor_highlight_format,
        )
        .key_binding(move |press| live_key_binding(press, wiki_open))
        .style(editor_surface_style)
        .into()
}

/// The source editor's key bindings.
///
/// While Unicode codepoint entry (Ctrl+Shift+U) is open, the editor
/// swallows typing and feeds the hex buffer instead. While the wikilink
/// dropdown is open, Esc dismisses it instead of the editor's default
/// unfocus, and the arrows walk the suggestions instead of moving the
/// caret. (Tab and Ctrl+Tab are not editor-bound; they reach the
/// global handler, which accepts/cycles the dropdown.)
///
/// Everything else is an editor-scoped shortcut (undo/redo, formatting,
/// footnotes, word delete) or falls back to the editor's own bindings
/// (copy/cut/paste/select-all, arrows, …).
pub(crate) fn editor_key_binding(
    press: text_editor::KeyPress,
    wiki_open: bool,
    unicode_open: bool,
) -> Option<text_editor::Binding<Message>> {
    if unicode_open {
        if let Some(binding) = unicode_entry_binding(&press) {
            return Some(binding);
        }
    }
    if wiki_open {
        match &press.key {
            Key::Named(Named::Escape) => {
                return Some(text_editor::Binding::Custom(Message::WikiDismiss));
            }
            Key::Named(Named::ArrowUp) if press.modifiers == Modifiers::default() => {
                return Some(text_editor::Binding::Custom(Message::WikiCycle(-1)));
            }
            Key::Named(Named::ArrowDown) if press.modifiers == Modifiers::default() => {
                return Some(text_editor::Binding::Custom(Message::WikiCycle(1)));
            }
            _ => {}
        }
    }
    match editor_shortcut(press.clone()) {
        Some(message) => Some(text_editor::Binding::Custom(message)),
        None => text_editor::Binding::from_key_press(press),
    }
}

/// The live editor's key bindings: Escape commits the region (or, with
/// the wiki dropdown open, dismisses it — the dropdown wins), the arrows
/// walk the dropdown's suggestions while it is open (and otherwise move
/// the caret), undo/redo stay app-scoped since the whole note is the
/// undo unit, the markdown formatting shortcuts (Ctrl+B/I/K/`,
/// Ctrl+Shift+X/H) become `LiveFormat` messages applied to the live
/// buffer, the four boundary keys — arrows, backspace, delete — are
/// intercepted so `app.rs` can decide between a plain motion and
/// crossing into a neighboring region, Ctrl+Backspace / Ctrl+Delete
/// delete a word like they do in the source editor, and typed
/// brackets/quotes route through the auto-pair table.
/// Everything else (typing, Enter, copy/paste, selection) falls back to
/// the editor's own bindings — including Ctrl combos the formatting map
/// does not claim, so copy/cut/paste/select-all keep working.
pub(crate) fn live_key_binding(
    press: text_editor::KeyPress,
    wiki_open: bool,
) -> Option<text_editor::Binding<Message>> {
    if !matches!(press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    let ctrl = press.modifiers.command();
    let plain = press.modifiers == Modifiers::default();
    // A plainly typed character (byte-column semantics apply later in
    // the app): pairs route to the auto-pair decision table, everything
    // else falls back to the widget's own insert.
    let typed = press
        .text
        .as_deref()
        .and_then(|t| t.chars().find(|c| !c.is_control()));
    match &press.key {
        // While the wikilink dropdown is open, Esc dismisses it instead
        // of committing the region (same priority as the source editor).
        Key::Named(Named::Escape) => Some(text_editor::Binding::Custom(if wiki_open {
            Message::WikiDismiss
        } else {
            Message::LiveDismiss
        })),
        // The dropdown owns the arrows while it is open: they walk the
        // suggestions instead of moving the caret between blocks.
        Key::Named(Named::ArrowUp) if wiki_open && plain => {
            Some(text_editor::Binding::Custom(Message::WikiCycle(-1)))
        }
        Key::Named(Named::ArrowDown) if wiki_open && plain => {
            Some(text_editor::Binding::Custom(Message::WikiCycle(1)))
        }
        Key::Character(c) if ctrl => {
            let lower = c.to_lowercase();
            match (lower.as_str(), press.modifiers.shift()) {
                ("z", false) => Some(text_editor::Binding::Custom(Message::EditorUndo)),
                ("z", true) => Some(text_editor::Binding::Custom(Message::EditorRedo)),
                ("y", false) => Some(text_editor::Binding::Custom(Message::EditorRedo)),
                _ => match crate::app::live_format_shortcut(press.clone()) {
                    Some(format) => Some(text_editor::Binding::Custom(Message::LiveFormat(format))),
                    // Unhandled ctrl combos keep the editor's own
                    // bindings (copy, cut, paste, select all) — the
                    // source editor falls back the same way.
                    None => text_editor::Binding::from_key_press(press),
                },
            }
        }
        Key::Character(_) if plain && typed.is_some_and(|c| crate::app::pair_for(c).is_some()) => {
            Some(text_editor::Binding::Custom(Message::EditorAutoPair(
                typed.expect("checked above"),
            )))
        }
        Key::Named(Named::Backspace) if plain => {
            Some(text_editor::Binding::Custom(Message::LiveBackspace))
        }
        Key::Named(Named::Delete) if plain => {
            Some(text_editor::Binding::Custom(Message::LiveDelete))
        }
        // Ctrl+Backspace / Ctrl+Delete delete a word, like in the source
        // editor. Without this the widget's own binding matches the
        // modified key and drops one character instead.
        Key::Named(Named::Backspace) if ctrl => {
            Some(text_editor::Binding::Custom(Message::LiveDeleteWord(false)))
        }
        Key::Named(Named::Delete) if ctrl => {
            Some(text_editor::Binding::Custom(Message::LiveDeleteWord(true)))
        }
        Key::Named(Named::ArrowUp) if plain => Some(text_editor::Binding::Custom(Message::LiveUp)),
        Key::Named(Named::ArrowDown) if plain => {
            Some(text_editor::Binding::Custom(Message::LiveDown))
        }
        _ => text_editor::Binding::from_key_press(press),
    }
}

/// Status bar under the note area: word/character counts for the active
/// tab (cached on the tab, refreshed with the doc — not per frame).
///
/// A selection counts on its own too: the marked words and characters
/// are shown next to the note's totals.
fn status_bar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(tab) = app.active_tab() else {
        return container(text("")).width(Length::Fill).into();
    };
    let selection = app.selection_text();
    let label = crate::app::status_label(tab, selection.as_deref());
    container(text(label).size(12).class(palette.muted))
        .padding([4, 12])
        .width(Length::Fill)
        .into()
}

fn with_alpha(color: cosmic::iced::Color, alpha: f32) -> cosmic::iced::Color {
    cosmic::iced::Color { a: alpha, ..color }
}

/// The settings context drawer (Phase G): toggles and steppers for every
/// persisted preference. Each change updates the app field immediately and
/// persists through load-modify-store.
pub fn settings_view<'a>(app: &'a VaultNotes) -> Element<'a, Message> {
    let toggle = |label: &'a str, on: bool, msg: Message| {
        settings::item(
            label,
            toggler(on).on_toggle(move |_| msg.clone()).spacing(8),
        )
    };
    let stepper = |label: &'a str, value: String, minus: Message, plus: Message| {
        settings::item(
            label,
            row::Row::new()
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .push(button::text("−").on_press(minus))
                .push(text(value).size(14))
                .push(button::text("+").on_press(plus)),
        )
    };
    // A three-way choice rendered as small buttons; the active one is
    // suggested-classed.
    let choice = |current: &str, options: &[(&'static str, Message)]| {
        let mut row = row::Row::new().spacing(4);
        for (label, msg) in options {
            let btn = if *label == current {
                button::suggested(*label)
            } else {
                button::text(*label)
            };
            row = row.push(btn.on_press(msg.clone()));
        }
        row
    };

    let editor = settings::section().title("Editor");
    let editor = editor
        .add(toggle(
            "Auto-pairs",
            app.auto_pair,
            Message::SettingsAutoPair(!app.auto_pair),
        ))
        .add(toggle(
            "Wikilink suggestions",
            app.wiki_autocomplete,
            Message::SettingsWikiComplete(!app.wiki_autocomplete),
        ))
        .add(toggle(
            "Word wrap",
            app.word_wrap,
            Message::SettingsWordWrap(!app.word_wrap),
        ))
        .add(toggle(
            "Monospace font",
            app.editor_font_mono,
            Message::SettingsFontMono(!app.editor_font_mono),
        ))
        .add(settings::item(
            "Custom font",
            text_input("e.g. Fira Code", &app.editor_font_family)
                .id(Id::new(EDITOR_FONT_INPUT))
                .on_input(Message::SettingsEditorFontFamily)
                .width(Length::Fixed(160.0)),
        ))
        .add(
            text(
                "Overrides the monospace toggle while set; an \
                    uninstalled name falls back to the default font.",
            )
            .size(12),
        )
        .add(stepper(
            "Font size",
            format!("{} px", app.editor_font_size),
            Message::SettingsEditorFontSize(-1),
            Message::SettingsEditorFontSize(1),
        ));

    // The reading view has its own font: a custom family (the system
    // font when the field is empty), its own size, and a margin around
    // the text column.
    let margins: Vec<(&'static str, Message)> = crate::settings::ReadingMargin::ALL
        .iter()
        .map(|margin| (margin.label(), Message::SettingsReadingMargin(*margin)))
        .collect();
    let reading = settings::section().title("Reading");
    let reading = reading
        .add(settings::item(
            "Custom font",
            text_input("e.g. Iowan Old Style", &app.reading_font_family)
                .id(Id::new(READING_FONT_INPUT))
                .on_input(Message::SettingsReadingFontFamily)
                .width(Length::Fixed(160.0)),
        ))
        .add(
            text(
                "The font the rendered note uses; empty follows the \
                    system font. An uninstalled name falls back to it.",
            )
            .size(12),
        )
        .add(stepper(
            "Font size",
            format!("{} px", app.reading_font_size),
            Message::SettingsReadingFontSize(-1),
            Message::SettingsReadingFontSize(1),
        ))
        .add(settings::item(
            "Margin",
            choice(app.reading_margin.label(), &margins),
        ));

    let view = settings::section().title("View");
    let view = view
        .add(settings::item(
            "Theme",
            choice(
                match app.theme {
                    crate::settings::ThemeChoice::System => "System",
                    crate::settings::ThemeChoice::Dark => "Dark",
                    crate::settings::ThemeChoice::Light => "Light",
                },
                &[
                    (
                        "System",
                        Message::SettingsTheme(crate::settings::ThemeChoice::System),
                    ),
                    (
                        "Dark",
                        Message::SettingsTheme(crate::settings::ThemeChoice::Dark),
                    ),
                    (
                        "Light",
                        Message::SettingsTheme(crate::settings::ThemeChoice::Light),
                    ),
                ],
            ),
        ))
        .add(settings::item(
            "Default layout",
            choice(
                match app.default_pane {
                    PaneMode::Live => "Live",
                    PaneMode::Split => "Split",
                    PaneMode::Source => "Source",
                    PaneMode::Reading => "Reading",
                },
                &[
                    ("Live", Message::SettingsDefaultPane(PaneMode::Live)),
                    ("Split", Message::SettingsDefaultPane(PaneMode::Split)),
                    ("Source", Message::SettingsDefaultPane(PaneMode::Source)),
                    ("Reading", Message::SettingsDefaultPane(PaneMode::Reading)),
                ],
            ),
        ))
        .add(toggle(
            "Hide .md in file tree",
            app.hide_md_extension,
            Message::SettingsHideMdExt(!app.hide_md_extension),
        ));

    let files = settings::section().title("Files & saving");
    let mut files = files
        .add(stepper(
            "Autosave delay",
            format!("{} s", app.autosave_secs),
            Message::SettingsAutosaveDelta(-1),
            Message::SettingsAutosaveDelta(1),
        ))
        .add(toggle(
            "Follow caret in preview",
            app.scroll_sync,
            Message::SettingsScrollSync(!app.scroll_sync),
        ))
        .add(settings::item(
            "File types in tree",
            row::Row::new()
                .spacing(4)
                .push(
                    text_input("e.g. png", &app.attachment_input)
                        .id(Id::new(ATTACHMENT_INPUT))
                        .on_input(Message::AttachmentInput)
                        .on_submit(|_| Message::AttachmentAdd)
                        .width(Length::Fixed(120.0)),
                )
                .push(button::text("Add").on_press(Message::AttachmentAdd)),
        ))
        .add(settings::item(
            "Link path",
            choice(
                match app.link_path_format {
                    LinkPathFormat::Shortest => "Shortest",
                    LinkPathFormat::Relative => "Relative",
                    LinkPathFormat::Absolute => "Absolute",
                },
                &[
                    (
                        "Shortest",
                        Message::SettingsLinkFormat(LinkPathFormat::Shortest),
                    ),
                    (
                        "Relative",
                        Message::SettingsLinkFormat(LinkPathFormat::Relative),
                    ),
                    (
                        "Absolute",
                        Message::SettingsLinkFormat(LinkPathFormat::Absolute),
                    ),
                ],
            ),
        ));
    // The filter list itself: empty shows every attachment; each entry is
    // a removable row.
    if app.attachment_extensions.is_empty() {
        files = files.add(text("All file types are shown.").size(12));
    } else {
        for ext in app.attachment_extensions.clone() {
            files = files.add(settings::item(
                format!(".{ext}"),
                button::text("Remove").on_press(Message::AttachmentRemove(ext)),
            ));
        }
    }

    scrollable::scrollable(
        container(
            column::Column::new()
                .spacing(16)
                .push(editor)
                .push(reading)
                .push(view)
                .push(files),
        )
        .padding(12)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Shared style for the editor's floating popups (wikilink dropdown,
/// Unicode entry indicator).
fn popup_style(palette: Palette) -> impl Fn(&cosmic::Theme) -> cosmic::widget::container::Style {
    move |_| cosmic::widget::container::Style {
        background: Some(Background::Color(with_alpha(palette.surface, 0.94))),
        border: border::Border {
            radius: 8.0.into(),
            width: 1.0,
            color: with_alpha(palette.muted, 0.5),
        },
        ..Default::default()
    }
}

/// The Unicode codepoint entry indicator (Ctrl+Shift+U): GTK shows the
/// underlined hex at the caret, but iced exposes no caret pixel
/// position, so this anchors to the editor's bottom edge like the
/// wikilink dropdown. Shows `u` plus the digits so far, the resolved
/// character once the codepoint is valid, and the key hints.
fn unicode_entry_overlay<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let hex = app.unicode_input.as_deref().unwrap_or("");
    let mut col = column::Column::new()
        .spacing(2)
        .push(text(format!("u{hex}")).size(16));
    if let Some(ch) = unicode_char(hex) {
        col = col.push(text(format!("= {ch}")).size(16));
    }
    col = col.push(
        text("Enter insert · Backspace · Esc cancel")
            .size(11)
            .class(palette.muted),
    );
    container(container(col).padding(6).style(popup_style(palette)))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Bottom)
        .padding([0, 12, 4, 12])
        .into()
}

#[cfg(test)]
mod tab_strip_tests {
    //! The tab strip shows the note's folder beside its title. A short
    //! folder keeps the tab narrow; a long one is given a fixed column
    //! so the ellipsis can cut it instead of pushing the tab off the
    //! strip.

    use super::*;

    #[test]
    fn a_short_folder_takes_its_natural_width() {
        assert_eq!(folder_width("Notes"), Length::Shrink);
        assert_eq!(folder_width("Journal/2026"), Length::Shrink);
        // A long path gets a fixed column, so it can be cut off.
        assert_eq!(
            folder_width("Journal/2026/04/14/notes"),
            Length::Fixed(120.0)
        );
    }
}

#[cfg(test)]
mod popup_layout_tests {
    //! Headless layout proofs for the editor popup structure, run through
    //! iced's real layout engine with the null renderer.
    //!
    //! The wikilink dropdown once rendered as a thin, tall, almost
    //! invisible sliver: its popup container was explicitly
    //! `Shrink`-width, which turns on iced's flex *cross-compression* —
    //! and cross-fluid (Fill-width) children are then laid out at the
    //! accumulated cross width, which is 0 for a Shrink parent. Text
    //! wrapped at zero width: a tall, unreadable strip. The quick
    //! switcher never had the problem because its popup container
    //! inherits a fluid (Fill) width from its Fill-width column.
    //! `popup_layout` pins the working structure;
    //! `shrink_popup_container_crushes` pins the trap itself, so a future
    //! "tidy up" back to a Shrink popup fails loudly instead of silently
    //! regressing.

    use cosmic::iced::core::layout::{Limits, Node};
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::widget::{container, Column, Row};
    use cosmic::iced::{Element, Length, Size, Theme};

    /// A fixed-size leaf standing in for a dropdown row's button.
    fn leaf<'a>() -> Element<'a, (), Theme, ()> {
        Column::new()
            .width(Length::Fixed(90.0))
            .height(Length::Fixed(24.0))
            .into()
    }

    /// A dropdown column: Fill-width, padded, filled with Fill-width rows
    /// of one fixed-size child each.
    fn dropdown_col() -> Column<'static, (), Theme, ()> {
        let mut col = Column::new().spacing(2).padding(6).width(Length::Fill);
        for _ in 0..3 {
            col = col.push(
                Row::new()
                    .push(leaf())
                    .width(Length::Fill)
                    .height(Length::Shrink),
            );
        }
        col
    }

    /// The editor popup's structure: an anchoring Fill×Fill container
    /// holding the popup container (fluid width, capped) over the column.
    fn popup() -> Element<'static, (), Theme, ()> {
        container(container(dropdown_col()).max_width(280.0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Lay out like the runtime does: the widget tree must be built
    /// recursively (`Tree::new`), since flex zips items with tree
    /// children — an empty tree lays out nothing.
    fn layout(elem: &mut Element<'static, (), Theme, ()>) -> Node {
        let mut tree = Tree::new(elem.as_widget());
        elem.as_widget_mut().layout(
            &mut tree,
            &(),
            &Limits::new(Size::ZERO, Size::new(800.0, 600.0)),
        )
    }

    #[test]
    fn popup_rows_keep_their_width_inside_the_popup() {
        let mut elem = popup();
        let node = layout(&mut elem);
        // The outer anchoring container fills the pane.
        assert_eq!(node.size(), Size::new(800.0, 600.0));
        // The popup itself: fluid width capped at 280.
        let popup = &node.children()[0];
        assert_eq!(popup.size().width, 280.0);
        // Rows fill the column's padded width (280 - 2 * 6 padding)...
        let col = &popup.children()[0];
        for row in col.children() {
            assert_eq!(row.size().width, 268.0);
            // ...and their contents are NOT crushed to zero width (the
            // "thin, tall, invisible" regression).
            assert_eq!(row.children()[0].size().width, 90.0);
            assert_eq!(row.children()[0].size().height, 24.0);
        }
    }

    #[test]
    fn shrink_popup_container_crushes_fill_children() {
        // The old structure, kept as a canary: an explicitly Shrink popup
        // container collapses to its padding (12px wide) and lays every
        // Fill-width row out at width 0.
        let mut elem: Element<'static, (), Theme, ()> = container(
            container(dropdown_col())
                .width(Length::Shrink)
                .max_width(280.0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        let node = layout(&mut elem);
        let popup = &node.children()[0];
        assert_eq!(popup.size().width, 12.0);
        for row in popup.children()[0].children() {
            assert_eq!(row.size().width, 0.0);
        }
    }
}
