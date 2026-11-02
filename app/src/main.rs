//! Vault Notes — libcosmic GUI (M4a shell).
//!
//! One window: sidebar file tree, tabs, a syntax-highlighted source editor
//! (iced `text_editor` + syntect Markdown grammar) and a reading view
//! rendered from `vault-md`'s IR. The M0 glass spike is gone; glass returns
//! in M4c through `vault-glass`.

mod app;
mod fuzzy;
mod glass;
mod reading;
mod scroll;
mod search;
mod settings;
mod tree;
mod view;

use app::VaultNotes;

fn main() -> cosmic::iced::Result {
    let settings = cosmic::app::Settings::default().size(cosmic::iced::Size::new(1180.0, 760.0));

    cosmic::app::run::<VaultNotes>(settings, ())
}
