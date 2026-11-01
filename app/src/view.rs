//! View composition: chrome, sidebar tree, tab bar, editor/reading panes.
//!
//! Pure functions of the model; every interactive element routes through
//! `Message`. Colors come from the active cosmic theme via `Palette`.

use cosmic::iced::widget::text_editor;
use cosmic::iced::{alignment, Background, Length, Padding};
use cosmic::widget::{button, column, container, row, scrollable, text, text_input};
use cosmic::Element;

use crate::app::{Message, Palette, PaneMode, VaultNotes};
use crate::reading;
use crate::tree::TreeNode;

pub fn view(app: &VaultNotes) -> Element<'_, Message> {
    if app.vault.is_none() {
        empty_state(app)
    } else {
        let palette = Palette::from(&cosmic::theme::active());
        main_view(app, palette)
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
    let sidebar = container(scrollable::scrollable(tree_view(app, palette)).width(Length::Fill))
        .width(Length::Fixed(250.0))
        .height(Length::Fill);
    let note = container(note_area(app, palette))
        .width(Length::Fill)
        .height(Length::Fill);
    col = col.push(row::Row::new().push(sidebar).push(note));
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
        .map(|path| crate::app::title_of(path))
        .collect::<Vec<_>>()
        .join(", ")
}

// --- sidebar ---------------------------------------------------------------

fn tree_view<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    if app.tree.is_empty() {
        return container(text("No notes.").class(palette.muted))
            .padding(12)
            .into();
    }
    nodes_view(&app.tree, 0, app)
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
                col = col.push(
                    container(
                        button::text(format!("{icon} {name}"))
                            .on_press(Message::ToggleFolder(path.clone()))
                            .width(Length::Fill),
                    )
                    .padding(Padding {
                        left: indent,
                        ..Padding::ZERO
                    })
                    .width(Length::Fill),
                );
                if !app.collapsed.contains(path) {
                    col = col.push(nodes_view(children, depth + 1, app));
                }
            }
            TreeNode::Note { name, path } => {
                let mut label = name.clone();
                if app.active_tab_path() == Some(path.as_str()) {
                    label = format!("● {label}");
                } else if app.is_tab_dirty(path) {
                    label = format!("• {label}");
                }
                col = col.push(
                    container(
                        button::text(label)
                            .on_press(Message::TabOpen(path.clone()))
                            .width(Length::Fill),
                    )
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
            ),
    )
    .padding(8)
    .width(Length::Fill)
    .height(Length::Fill)
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
    scrollable::scrollable(reading::render_doc(&tab.doc, &ctx))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn with_alpha(color: cosmic::iced::Color, alpha: f32) -> cosmic::iced::Color {
    cosmic::iced::Color { a: alpha, ..color }
}
