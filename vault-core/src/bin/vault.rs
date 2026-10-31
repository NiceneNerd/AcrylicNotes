//! `vault` CLI — M1 acceptance tool over vault-core.
//!
//! usage:
//!   vault index <dir>                    list notes/attachments/tags
//!   vault resolve <dir> <from> <target>  resolve a link from a note
//!   vault backlinks <dir> <note>         list notes linking to a note
//!
//! Read-only: it never writes to the vault.

use std::path::Path;

use vault_core::Vault;

const USAGE: &str = "usage:\n  vault index <dir>\n  vault resolve <dir> <from-note> <target>\n  vault backlinks <dir> <note>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

fn run(args: &[String]) -> i32 {
    match args {
        [cmd, dir] if cmd == "index" => cmd_index(dir),
        [cmd, dir, from, target] if cmd == "resolve" => cmd_resolve(dir, from, target),
        [cmd, dir, note] if cmd == "backlinks" => cmd_backlinks(dir, note),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

fn open(dir: &str) -> Result<Vault, i32> {
    Vault::open(Path::new(dir)).map_err(|e| {
        eprintln!("error: {e}");
        1
    })
}

fn cmd_index(dir: &str) -> i32 {
    let Ok(vault) = open(dir) else {
        return 1;
    };
    let notes = vault.notes_sorted();
    let attachments = vault.attachments_sorted();
    println!(
        "{} notes, {} attachments, {} tags",
        notes.len(),
        attachments.len(),
        vault.all_tags().len()
    );
    for note in &notes {
        let tags = if note.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", note.tags.join(", "))
        };
        println!("  {} — {}{}", note.path, note.title, tags);
    }
    for a in &attachments {
        println!("  {a} (attachment)");
    }
    0
}

fn cmd_resolve(dir: &str, from: &str, target: &str) -> i32 {
    let Ok(vault) = open(dir) else {
        return 1;
    };
    match vault.index().resolve(target, from) {
        Some(path) => {
            println!("{path}");
            0
        }
        None => {
            println!("unresolved: {target}");
            1
        }
    }
}

fn cmd_backlinks(dir: &str, note: &str) -> i32 {
    let Ok(vault) = open(dir) else {
        return 1;
    };
    let backlinks = vault.backlinks(note);
    if backlinks.is_empty() {
        println!("(none)");
        return 1;
    }
    for source in &backlinks {
        println!("{source}");
    }
    0
}
