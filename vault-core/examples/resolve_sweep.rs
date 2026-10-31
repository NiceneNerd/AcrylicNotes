//! Diagnostic: resolve every outgoing link in a vault in-process and report
//! unresolved ones. Unresolved is NOT necessarily a bug — Obsidian also has
//! dangling links — but samples show whether the resolver misses real files.
//!
//! usage: cargo run -p vault-core --example resolve_sweep -- <vault-dir>
use std::collections::BTreeMap;

use vault_core::Vault;

fn main() {
    let dir = std::env::args().nth(1).expect("vault dir required");
    let vault = Vault::open(&dir).expect("open vault");
    let index = vault.index();

    let mut total = 0usize;
    let mut resolved = 0usize;
    let mut unresolved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for note in vault.all_notes() {
        for link in &note.outgoing_links {
            total += 1;
            // resolve each link the way its kind demands
            let hit = match link.kind {
                vault_core::LinkKind::Wiki => index.resolve(&link.target, &note.path),
                vault_core::LinkKind::Markdown => index.resolve_relative(&link.target, &note.path),
            };
            if let Some(_path) = hit {
                resolved += 1;
            } else {
                unresolved
                    .entry(link.target.clone())
                    .or_default()
                    .push(note.path.clone());
            }
        }
    }

    println!(
        "{total} outgoing links, {resolved} resolved, {} unresolved",
        unresolved.len()
    );
    let mut by_count: Vec<(&String, &Vec<String>)> = unresolved.iter().collect();
    by_count.sort_by_key(|(_, srcs)| std::cmp::Reverse(srcs.len()));
    println!("most-linked unresolved targets:");
    for (target, sources) in by_count.iter().take(15) {
        println!(
            "  {:?} (from {} notes, e.g. {})",
            target,
            sources.len(),
            sources[0]
        );
    }
}
