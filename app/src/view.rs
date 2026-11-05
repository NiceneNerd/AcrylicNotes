//! View composition: chrome, sidebar panels (files/search/tags/links/outline),
//! tab bar, editor/reading panes, and the quick-switcher overlay.
//!
//! Pure functions of the model; every interactive element routes through
//! `Message`. Colors come from the active cosmic theme via `Palette`.

use cosmic::iced::core::widget::Id;
use cosmic::iced::widget::{opaque, stack, text_editor};
use cosmic::iced::{alignment, border, Background, Length, Padding};
use cosmic::widget::{
    button, column, container, context_menu, icon, menu, nav_bar, row, scrollable,
    segmented_button, settings, tab_bar, text, text_input, toggler,
};
use cosmic::Element;

use crate::app::{
    editor_shortcut, find_matches, parent_of, title_of, tree_note_label, unicode_char,
    unicode_entry_binding, Message, Palette, PaneMode, SidePanel, TreeMenuAction, VaultNotes,
    ATTACHMENT_INPUT, EDITOR_INPUT, FIND_INPUT, SWITCHER_INPUT, TREE_EDIT_INPUT,
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
                .on_activate(Message::PaneActivated),
        )
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
    col = col.push(row::Row::new().push(sidebar(app, palette)).push(note));
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

// --- sidebar ---------------------------------------------------------------

/// Sidebar: a COSMIC nav bar (icon + label per panel) over the active
/// panel's body — the same pattern COSMIC Files uses for its places list.
/// The nav bar's own solid background is cleared so the whole sidebar reads
/// as one surface (the nav bar's default style paints `primary.base`, which
/// split the sidebar into a solid strip over a translucent body).
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
            .width(Length::Fill)
            .height(Length::Fill),
    );
    container(col)
        .width(Length::Fixed(250.0))
        .height(Length::Fill)
        .style(move |_| {
            // Default: translucent, so the compositor's frosted-window blur
            // shows through (COSMIC transparency setting). "Solid sidebar"
            // paints the theme surface instead.
            let background = if app.sidebar_solid {
                Some(Background::Color(cosmic::iced::Color::from(
                    cosmic::theme::active()
                        .cosmic()
                        .background(palette.is_dark)
                        .base,
                )))
            } else {
                None
            };
            cosmic::widget::container::Style {
                background,
                ..Default::default()
            }
        })
        .into()
}

fn tree_view<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    if app.tree.is_empty() && app.tree_folder_edit_target().is_none() {
        return container(text("No notes.").class(palette.muted))
            .padding(12)
            .into();
    }
    let mut col = column::Column::new().spacing(0).width(Length::Fill);
    // A root-level new-note/new-folder input renders above the tree.
    if app.tree_folder_edit_target() == Some("") {
        col = col.push(tree_edit_row(app, 4.0));
    }
    col = col.push(nodes_view(&app.tree, 0, app));
    col.into()
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
                col = col.push(
                    container(menu_row(row, path))
                        .padding(Padding {
                            left: indent,
                            ..Padding::ZERO
                        })
                        .width(Length::Fill),
                );
                if !app.collapsed.contains(path) {
                    // New-note/new-folder inputs render as the folder's first row.
                    if app.tree_folder_edit_target() == Some(path.as_str()) {
                        col = col.push(tree_edit_row(app, indent + 14.0));
                    }
                    col = col.push(nodes_view(children, depth + 1, app));
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
                col = col.push(
                    container(if is_note { menu_row(row, path) } else { row })
                        .padding(Padding {
                            left: indent + 10.0,
                            ..Padding::ZERO
                        })
                        .width(Length::Fill),
                );
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
    col = col.push(status_bar(tab, palette));
    col.into()
}

fn tab_bar<'a>(app: &'a VaultNotes, _palette: Palette) -> Element<'a, Message> {
    if app.tabs.is_empty() {
        return container(text("No open tabs").class(_palette.muted))
            .padding(4)
            .width(Length::Fill)
            .into();
    }
    // The COSMIC tab strip: model-driven tabs with close buttons; entities
    // carry the tab index (kept in sync by `sync_tab_model`). The default
    // `button_spacing` is 0, so the label sits against the close icon.
    tab_bar::horizontal(&app.tab_model)
        .button_spacing(cosmic::theme::spacing().space_xxs)
        .on_activate(Message::TabActivated)
        .on_close(Message::TabCloseRequested)
        .into()
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

fn editor_pane<'a>(
    app: &'a VaultNotes,
    tab: &'a crate::app::Tab,
    palette: Palette,
    wiki: Option<&'a crate::app::WikiComplete>,
) -> Element<'a, Message> {
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
        .font(if app.editor_font_mono {
            cosmic::font::mono()
        } else {
            cosmic::font::default()
        })
        .size(f32::from(app.editor_font_size))
        .wrapping(if app.word_wrap {
            cosmic::iced::core::text::Wrapping::Word
        } else {
            cosmic::iced::core::text::Wrapping::None
        })
        .highlight_with::<cosmic::iced::highlighter::Highlighter>(
            cosmic::iced::highlighter::Settings {
                theme: highlighter_theme,
                token: "md".to_string(),
            },
            |highlight, _theme| highlight.to_format(),
        )
        // Editor-scoped shortcuts: undo/redo, formatting, footnotes,
        // word delete. Anything else falls back to the editor's own
        // bindings (copy/cut/paste/select-all, arrows, …).
        .key_binding(move |press: text_editor::KeyPress| {
            // While Unicode codepoint entry (Ctrl+Shift+U) is open, the
            // editor swallows typing and feeds the hex buffer instead.
            if app.unicode_input.is_some() {
                if let Some(binding) = unicode_entry_binding(&press) {
                    return Some(binding);
                }
            }
            // While the wikilink dropdown is open, Esc dismisses it
            // instead of the editor's default unfocus. (Tab and
            // Ctrl+Tab are not editor-bound; they reach the global
            // handler, which accepts/cycles the dropdown.)
            if wiki.is_some()
                && press.key
                    == cosmic::iced::keyboard::Key::Named(
                        cosmic::iced::keyboard::key::Named::Escape,
                    )
            {
                return Some(text_editor::Binding::Custom(Message::WikiDismiss));
            }
            match editor_shortcut(press.clone()) {
                Some(message) => Some(text_editor::Binding::Custom(message)),
                None => text_editor::Binding::from_key_press(press),
            }
        })
        .style(|theme: &cosmic::Theme, status: text_editor::Status| {
            // Cosmic's default text_editor style paints an opaque
            // bg_color, which hides the pane's translucent overlay (and
            // any compositor frosted-window blur behind it). Keep only
            // the border and text colors; the pane container's dark
            // overlay is the visible editor surface.
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
                placeholder: with_alpha(cosmic.palette.neutral_9.into(), 0.7),
                value: cosmic.palette.neutral_9.into(),
                selection: cosmic.accent.base.into(),
            }
        });
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
    let mut col = column::Column::new().spacing(2).padding(6);
    for (i, title) in wiki.items.iter().enumerate() {
        let btn = if i == wiki.selected {
            button::suggested(title.clone())
        } else {
            button::text(title.clone())
        };
        col = col.push(
            btn.on_press(Message::WikiSelect(i))
                .width(Length::Fill)
                .height(Length::Shrink),
        );
    }
    // The outer container fills the editor area so the popup can anchor to
    // its bottom edge; the inner one is the popup itself.
    container(
        container(col)
            .width(Length::Shrink)
            .max_width(280)
            .style(popup_style(palette)),
    )
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
        folds: &tab.folds,
        images: &app.images,
    };
    // The id anchors outline/search/footnote jumps (see scroll.rs). The
    // document column is centered at a readable measure (matching the web
    // view's ~46rem) instead of stretching edge to edge. Padding goes on
    // the wrapping container: a Scrollable's own `padding` only insets its
    // scrollbar, not its content.
    container(
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
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(16)
    .into()
}

/// Status bar under the note area: word/character counts for the active
/// tab (cached on the tab, refreshed with the doc — not per frame).
fn status_bar<'a>(tab: &'a crate::app::Tab, palette: Palette) -> Element<'a, Message> {
    container(
        text(format!("{} words · {} characters", tab.words, tab.chars))
            .size(12)
            .class(palette.muted),
    )
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
        .add(stepper(
            "Font size",
            format!("{} px", app.editor_font_size),
            Message::SettingsFontSize(-2),
            Message::SettingsFontSize(2),
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
        .add(toggle(
            "Solid sidebar",
            app.sidebar_solid,
            Message::SettingsSidebarSolid(!app.sidebar_solid),
        ))
        .add(settings::item(
            "Default layout",
            choice(
                match app.default_pane {
                    PaneMode::Split => "Split",
                    PaneMode::Source => "Source",
                    PaneMode::Reading => "Reading",
                },
                &[
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
