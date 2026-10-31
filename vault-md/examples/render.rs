//! Render a real note to HTML on stdout.
//!
//! usage: cargo run -p vault-md --example render -- <vault-dir> <note-path-relative>
use std::path::Path;

use vault_core::{NoteSource, ObsidianSettings, Vault};
use vault_md::{parse_note, render_html, RenderContext, RenderOptions};

struct VaultNotes(vault_core::Vault);

impl NoteSource for VaultNotes {
    fn note_source(&self, path: &str) -> Option<String> {
        self.0.note_source(path)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let vault_dir = args.next().expect("vault dir");
    let note = args.next().expect("note path (vault-relative)");
    let vault = vault_core::Vault::open(Path::new(&vault_dir)).expect("open vault");
    let source = vault.note_source(&note).expect("note exists");
    let doc = parse_note(&source);

    let settings = ObsidianSettings::load(&vault_dir);
    let index = vault.index();
    let notes = VaultNotes(vault);
    let options = RenderOptions {
        strict_line_breaks: settings.strict_line_breaks,
        ..RenderOptions::default()
    };
    let ctx = RenderContext::new(&index, &note, &notes, options);
    print!("{}", render_html(&doc, &ctx));
}
