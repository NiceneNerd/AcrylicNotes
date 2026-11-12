//! Sidebar and overlay components.
//!
//! Each component is a pure function of the shared view models plus native
//! handles: `search_panel` reads `vault_app::search::SearchSession`
//! (shared state), `tags_panel`/`links_panel` read
//! `vault_app::navigation::{tag_counts, tag_notes, sorted_backlinks}`,
//! `outline_panel` reads `tab.outline` (`vault_app::navigation::OutlineEntry`),
//! and `switcher_overlay` reads `vault_app::navigation::QuickSwitcher`.
//! Every interactive element routes through [`crate::app::Message`]; no
//! component mutates state directly (restructure plan p7:
//! "thin each view module until it reads view models and emits commands").

use cosmic::iced::{alignment, border, Background, Length, Padding};
use cosmic::widget::{button, column, container, row, text, text_input};
use cosmic::{widget, Element};

use crate::app::{
    parent_of as app_parent_of, title_of as app_title_of, Message, Palette, VaultNotes,
    SWITCHER_INPUT,
};
use vault_app::navigation::{sorted_backlinks, tag_counts, tag_notes};

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

/// Blend a color toward a surface alpha for overlays.
pub(crate) fn with_alpha(color: cosmic::iced::Color, alpha: f32) -> cosmic::iced::Color {
    cosmic::iced::Color {
        a: color.a * alpha,
        ..color
    }
}

fn panel_hint(message: &str, palette: Palette) -> Element<'static, Message> {
    container(text(message.to_string()).class(palette.muted))
        .padding(12)
        .into()
}

/// Vault-wide text search panel: input + button, hit list with title/path and
/// a 64-char snippet. The query/hits live in shared state; the panel only
/// renders and routes to `Message`.
pub fn search_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new()
        .spacing(6)
        .padding(8)
        .width(Length::Fill);
    col = col.push(
        text_input("Search notes…", app.shared.state.search.query())
            .on_input(Message::SearchInput)
            .on_submit(|_| Message::SearchSubmitted)
            .width(Length::Fill),
    );
    col = col.push(button::suggested("Search").on_press(Message::SearchSubmitted));
    if app.shared.state.search.hits().is_empty() {
        if !app.shared.state.search.query().trim().is_empty() {
            col = col.push(text("No matches.").class(palette.muted));
        }
    } else {
        col = col.push(
            text(format!("{} matches", app.shared.state.search.hits().len())).class(palette.muted),
        );
        for (i, hit) in app.shared.state.search.hits().iter().enumerate() {
            col = col.push(
                button::text(app_title_of(&hit.path))
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
pub fn tags_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(vault) = app.vault.as_ref() else {
        return panel_hint("No vault open.", palette);
    };
    let mut col = column::Column::new()
        .spacing(4)
        .padding(8)
        .width(Length::Fill);
    match &app.selected_tag {
        None => {
            let tags = tag_counts(vault);
            if tags.is_empty() {
                col = col.push(text("No tags in this vault.").class(palette.muted));
            }
            for (tag, count) in tags {
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
            for path in tag_notes(vault, tag) {
                col = col.push(
                    button::text(app_title_of(&path))
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
pub fn links_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
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
    col = col.push(text(format!("Links to “{}”", app_title_of(path))).class(palette.muted));
    let backlinks = sorted_backlinks(vault, path);
    if backlinks.is_empty() {
        col = col.push(text("No backlinks.").class(palette.muted));
    }
    for source in backlinks {
        col = col.push(
            button::text(app_title_of(&source))
                .on_press(Message::TabOpen(source))
                .height(Length::Shrink)
                .width(Length::Fill),
        );
    }
    col.into()
}

/// Headings of the active note; a click scrolls the reading view to it.
pub fn outline_panel<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let Some(idx) = app.active_idx() else {
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

/// Modal fuzzy-file-finder overlay (Ctrl+P). Keys are routed in `update` via
/// the global keyboard subscription; Enter activates, arrows move, Esc closes.
pub fn switcher_overlay<'a>(app: &'a VaultNotes, palette: Palette) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(6).width(Length::Fill);
    col = col.push(
        text_input("Jump to note…", &app.switcher.query)
            .id(cosmic::iced::widget::Id::new(SWITCHER_INPUT))
            .on_input(Message::SwitcherInput)
            .on_submit(|_| Message::SwitcherActivate)
            .width(Length::Fill),
    );
    if app.switcher.items.is_empty() {
        col = col.push(text("No matching notes.").class(palette.muted));
    }
    for (i, path) in app.switcher.items.iter().enumerate() {
        let selected = app.switcher.selected == i;
        let mut label = app_title_of(path);
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
        let parent = app_parent_of(path);
        let mut row = row::Row::new().push(btn.width(Length::Shrink).height(Length::Shrink));
        if !parent.is_empty() {
            row = row.push(text(parent).size(12).class(palette.muted));
        }
        col = col.push(row.width(Length::Fill));
    }
    widget::container(
        widget::container(col)
            .padding(12)
            .max_width(560)
            .style(move |_| {
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
            }),
    )
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
