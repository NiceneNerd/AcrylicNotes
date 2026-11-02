//! View composition: chrome, sidebar panels (files/search/tags/links/outline),
//! tab bar, editor/reading panes, and the quick-switcher overlay.
//!
//! Pure functions of the model; every interactive element routes through
//! `Message`. Colors come from the active cosmic theme via `Palette`.

use cosmic::iced::core::widget::Id;
use cosmic::iced::widget::{opaque, stack, text_editor};
use cosmic::iced::{alignment, border, Background, Length, Padding};
use cosmic::widget::{
    button, column, container, context_menu, flex_row, menu, row, scrollable, text, text_input,
};
use cosmic::Element;

use crate::app::{
    title_of, Message, Palette, PaneMode, SidePanel, TreeMenuAction, VaultNotes, SWITCHER_INPUT,
    TREE_EDIT_INPUT,
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
    let pane_button = |label: &'static str, mode: PaneMode| {
        let btn = if app.pane == mode {
            button::suggested(label)
        } else {
            button::text(label)
        };
        btn.on_press(Message::PaneMode(mode)).into()
    };
    vec![
        button::text("Save").on_press(Message::Save).into(),
        button::text("Daily").on_press(Message::DailyNote).into(),
        // Label shows the theme a click switches to.
        button::text(if cosmic::theme::is_dark() {
            "Light"
        } else {
            "Dark"
        })
        .on_press(Message::ToggleTheme)
        .into(),
        pane_button("Source", PaneMode::Source),
        pane_button("Reading", PaneMode::Reading),
        pane_button("Split", PaneMode::Split),
    ]
}

// --- empty state -----------------------------------------------------------

fn empty_state(app: &VaultNotes) -> Element<'_, Message> {
    let palette = Palette::from(&cosmic::theme::active());
    let mut col = column::Column::new().spacing(16).max_width(460);
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

/// Sidebar: a row of panel switcher tabs over the active panel's body.
fn sidebar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(4).width(Length::Fill);
    // flex_row wraps the five panel buttons within the 250px sidebar; a plain
    // row overflows and clips the last button (Outline) off the edge.
    let switcher: Vec<Element<'a, Message>> = SidePanel::ALL
        .iter()
        .map(|panel| {
            let btn = if app.panel == *panel {
                button::suggested(panel.label())
            } else {
                button::text(panel.label())
            };
            btn.height(Length::Shrink)
                .on_press(Message::SidePanel(*panel))
                .into()
        })
        .collect();
    col = col.push(flex_row(switcher));
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
                let icon = if app.collapsed.contains(path) {
                    "▸"
                } else {
                    "▾"
                };
                let row = if app.tree_rename_target() == Some(path.as_str()) {
                    tree_edit_row(app, indent)
                } else {
                    button::text(format!("{icon} {name}"))
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
                let row = if app.tree_rename_target() == Some(path.as_str()) {
                    tree_edit_row(app, indent + 10.0)
                } else {
                    let mut label = name.clone();
                    if app.active_tab_path() == Some(path.as_str()) {
                        label = format!("● {label}");
                    } else if app.is_tab_dirty(path) {
                        label = format!("• {label}");
                    }
                    button::text(label)
                        .on_press(Message::TabOpen(path.clone()))
                        .height(Length::Shrink)
                        .width(Length::Fill)
                        .into()
                };
                col = col.push(
                    container(menu_row(row, path))
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
                button::text(hit.path.clone())
                    .on_press(Message::SearchOpen(i))
                    .height(Length::Shrink)
                    .width(Length::Fill),
            );
            col = col.push(
                text(format!("{}  {}", hit.line_no, snippet(&hit.line)))
                    .size(12)
                    .class(palette.muted),
            );
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
        };
        col = col.push(
            btn.on_press(Message::SwitcherSelect(i))
                .height(Length::Shrink)
                .width(Length::Fill),
        );
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
    let content: Element<'_, Message> = match app.pane {
        PaneMode::Source => editor_pane(tab, palette),
        PaneMode::Reading => reading_pane(app, tab, palette),
        PaneMode::Split => row::Row::new()
            .push(container(editor_pane(tab, palette)).width(Length::FillPortion(1)))
            .push(container(reading_pane(app, tab, palette)).width(Length::FillPortion(1)))
            .into(),
    };
    col = col.push(container(content).width(Length::Fill).height(Length::Fill));
    col.into()
}

fn tab_bar<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut bar = row::Row::new().spacing(4);
    for (i, tab) in app.tabs.iter().enumerate() {
        let mut label = tab.title();
        if tab.is_dirty() {
            label = format!("• {label}");
        }
        let open_btn = if app.active == Some(i) {
            button::suggested(label).on_press(Message::TabSelect(i))
        } else {
            button::text(label).on_press(Message::TabSelect(i))
        };
        bar = bar
            .push(open_btn)
            .push(button::text("×").on_press(Message::TabClose(i)));
    }
    if app.tabs.is_empty() {
        bar = bar.push(text("No open tabs").class(palette.muted));
    }
    container(bar).padding(4).width(Length::Fill).into()
}

// --- panes -----------------------------------------------------------------

fn editor_pane<'a>(tab: &'a crate::app::Tab, palette: Palette) -> Element<'a, Message> {
    // Syntect Markdown grammar via iced's highlighter (M4 decision 3);
    // theme follows the system light/dark preference.
    let highlighter_theme = if palette.is_dark {
        cosmic::iced::highlighter::Theme::Base16Ocean
    } else {
        cosmic::iced::highlighter::Theme::InspiredGitHub
    };
    container(
        text_editor(&tab.content)
            .height(Length::Fill)
            .on_action(Message::EditorAction)
            .font(cosmic::font::mono())
            .highlight_with::<cosmic::iced::highlighter::Highlighter>(
                cosmic::iced::highlighter::Settings {
                    theme: highlighter_theme,
                    token: "md".to_string(),
                },
                |highlight, _theme| highlight.to_format(),
            )
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
                        width: f32::from(cosmic.space_xxxs()),
                        color: border_color.into(),
                    },
                    placeholder: with_alpha(cosmic.palette.neutral_9.into(), 0.7),
                    value: cosmic.palette.neutral_9.into(),
                    selection: cosmic.accent.base.into(),
                }
            }),
    )
    .padding(8)
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
    // The id anchors outline/search/footnote jumps (see scroll.rs).
    scrollable::scrollable(reading::render_doc(&tab.doc, &ctx))
        .id(Id::new(READING_SCROLL))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn with_alpha(color: cosmic::iced::Color, alpha: f32) -> cosmic::iced::Color {
    cosmic::iced::Color { a: alpha, ..color }
}
