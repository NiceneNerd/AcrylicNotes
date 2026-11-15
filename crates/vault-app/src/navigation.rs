//! Navigation view-model types shared by both frontends.

use serde::{Deserialize, Serialize};

use vault_md::{Block, Doc};

use crate::markdown::plain_inlines;

/// The vault index adapter: tag lookups, backlinks, and link-path
/// resolution.
///
/// The [`NoteRepository`](crate::repository::NoteRepository) trait
/// deliberately hides its backing store, so tag/backlink data and link
/// resolution need their own contract. vault-core's `Vault` provides the
/// real one (nearest-first resolution is its index's job); tests inject
/// the deterministic fake in `repository::testing`. Plan §12's
/// navigation row: "counts and navigation come from shared models".
pub trait NoteIndex {
    /// Every tag with its note count, sorted by tag name (the order the
    /// tag panel shows them).
    fn tag_counts(&self) -> Vec<(String, usize)>;

    /// The notes tagged `tag`, sorted by path (the order the panel's
    /// drill-down lists them).
    fn tag_notes(&self, tag: &str) -> Vec<String>;

    /// The backlinks of `path`, sorted by source path.
    fn backlinks(&self, path: &str) -> Vec<String>;

    /// Resolve a link target (`[[Meeting Notes]]`, `Meeting Notes.md`,
    /// `notes/b.md`) to a vault-relative note path, from `from`
    /// (vault-core's index: exact, then extension, then
    /// shortest-unique-name).
    fn resolve_note(&self, target: &str, from: &str) -> Option<String>;

    /// Resolve a folder-relative target (`Note.md`,
    /// `../media/x.png`) against the source note's folder — standard
    /// Markdown links, never shortest-unique-name. Attachment files
    /// resolve too.
    fn resolve_relative(&self, target: &str, from: &str) -> Option<String>;
}

impl NoteIndex for vault_core::Vault {
    fn tag_counts(&self) -> Vec<(String, usize)> {
        tag_counts(self)
    }

    fn tag_notes(&self, tag: &str) -> Vec<String> {
        tag_notes(self, tag)
    }

    fn backlinks(&self, path: &str) -> Vec<String> {
        sorted_backlinks(self, path)
    }

    fn resolve_note(&self, target: &str, from: &str) -> Option<String> {
        self.index().resolve(target, from).map(str::to_string)
    }

    fn resolve_relative(&self, target: &str, from: &str) -> Option<String> {
        self.index()
            .resolve_relative(target, from)
            .map(str::to_string)
    }
}

/// One heading for the outline panel: its top-level block index, the
/// heading level and its plain text (moved from the cosmic frontend's
/// reading module; the data is pure `Doc` → rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub block: usize,
    pub level: u8,
    pub text: String,
}

/// Top-level headings of a document, in document order.
#[must_use]
pub fn outline(doc: &Doc) -> Vec<OutlineEntry> {
    doc.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| match block {
            Block::Heading { level, content, .. } => Some(OutlineEntry {
                block: i,
                level: *level,
                text: plain_inlines(content),
            }),
            _ => None,
        })
        .collect()
}

/// The tag panel's list view: every tag with its note count, sorted by
/// tag name (the order the panel shows them).
#[must_use]
pub fn tag_counts(vault: &vault_core::Vault) -> Vec<(String, usize)> {
    let mut tags = vault.all_tags();
    tags.sort();
    tags.into_iter()
        .map(|tag| {
            let count = vault.notes_for_tag(&tag).len();
            (tag, count)
        })
        .collect()
}

/// The tag panel's drill-down view: the notes tagged `tag`, sorted by
/// path (the order the panel lists them).
#[must_use]
pub fn tag_notes(vault: &vault_core::Vault, tag: &str) -> Vec<String> {
    vault.notes_for_tag(tag)
}

/// The links panel's view: the backlinks of `path`, sorted by source
/// path (nearest-first resolution is vault-core's index's job).
#[must_use]
pub fn sorted_backlinks(vault: &vault_core::Vault, path: &str) -> Vec<String> {
    let mut backlinks = vault.backlinks(path);
    backlinks.sort();
    backlinks
}

/// How many notes the quick switcher shows at most (moved from the
/// cosmic shell with the view model).
pub const SWITCHER_MAX_ITEMS: usize = 20;

/// The quick switcher's state: an open flag, the query, and the ranked
/// note paths (M4b). The frontend feeds the vault's note paths; the
/// view model ranks them, clamps the selection, and answers which path
/// is under the cursor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QuickSwitcher {
    pub open: bool,
    pub query: String,
    /// Ranked note paths (already materialized from the index).
    pub items: Vec<String>,
    pub selected: usize,
}

impl QuickSwitcher {
    /// Toggle the overlay. Opening clears the query and the selection
    /// (exactly the reset the shell did before extracting).
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.selected = 0;
        }
    }

    /// Accept typed input and re-rank the items from the vault's notes.
    ///
    /// The policy is the shell's original one: an empty (or blank)
    /// query shows the most recently touched notes first — the ones you
    /// are working on — with alphabetical tie-breaks; a real query
    /// fuzzy-ranks all paths instead. The selection resets to the top
    /// on typing.
    pub fn set_query(
        &mut self,
        query: impl Into<String>,
        notes: &[vault_core::Note],
        max_items: usize,
    ) {
        self.query = query.into();
        self.selected = 0;
        if self.query.trim().is_empty() {
            let mut notes: Vec<&vault_core::Note> = notes.iter().collect();
            notes.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
            self.items = notes
                .into_iter()
                .take(max_items)
                .map(|note| note.path.clone())
                .collect();
        } else {
            let paths: Vec<String> = notes.iter().map(|note| note.path.clone()).collect();
            self.items = crate::fuzzy::rank_paths(&self.query, &paths, max_items)
                .into_iter()
                .map(|ranked| paths[ranked.index].clone())
                .collect();
        }
        self.clamp_selection();
    }

    /// The items when no vault is open: nothing to switch between.
    pub fn clear_items(&mut self) {
        self.items.clear();
        self.selected = 0;
    }

    /// Move the selection by `delta` entries, wrapping around the list.
    pub fn move_selection(&mut self, delta: i32) {
        if !self.items.is_empty() {
            let len = self.items.len() as i32;
            self.selected = (self.selected as i32 + delta).rem_euclid(len) as usize;
        }
    }

    /// The path under the cursor, if any.
    #[must_use]
    pub fn selected_path(&self) -> Option<&String> {
        self.items.get(self.selected)
    }

    /// Dismiss the overlay (activation or Escape); items stay until the
    /// next open resets them.
    pub fn close(&mut self) {
        self.open = false;
    }

    fn clamp_selection(&mut self) {
        if self.selected >= self.items.len() {
            self.selected = 0;
        }
    }
}

/// Which sidebar panel is active. Rendering and panel-specific widgets are
/// native; the active section itself is shared state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavSection {
    /// The file tree.
    #[default]
    #[serde(rename = "files")]
    Files,
    /// Vault-wide text search.
    #[serde(rename = "search")]
    Search,
    /// Tag list with counts and per-tag note lists.
    #[serde(rename = "tags")]
    Tags,
    /// Backlinks of the active note.
    #[serde(rename = "links")]
    Links,
    /// Headings of the active note.
    #[serde(rename = "outline")]
    Outline,
}

impl NavSection {
    /// Every section, in the order the sidebar lists them.
    pub const ALL: [NavSection; 5] = [
        NavSection::Files,
        NavSection::Search,
        NavSection::Tags,
        NavSection::Links,
        NavSection::Outline,
    ];

    /// The user-facing name of this section.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Search => "Search",
            Self::Tags => "Tags",
            Self::Links => "Links",
            Self::Outline => "Outline",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// The crate's test data directory.
    ///
    /// Normally the compile-time manifest dir. When the test binary runs
    /// on another machine (the native Windows lane), xtask stages the
    /// fixtures next to the executable instead, so the compile-time path
    /// does not exist there.
    fn tests_dir() -> std::path::PathBuf {
        // Candidates, in order: the workspace-relative fixture tree (the
        // normal case on the developer machine), then the tree xtask
        // stages next to the test binary on another machine.
        let candidates: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"),
            std::env::current_exe()
                .ok()
                .and_then(|exe| {
                    exe.parent()
                        .map(|p| p.join("vaultnotes-fixtures").join("fixtures"))
                })
                .unwrap_or_default(),
        ];
        for cand in &candidates {
            if cand.is_dir() {
                return cand.clone();
            }
        }
        candidates.into_iter().next().unwrap_or_default()
    }
    use std::time::{Duration, SystemTime};

    fn note(path: &str, age_secs_ago: u64) -> vault_core::Note {
        note_at(path, SystemTime::now() - Duration::from_secs(age_secs_ago))
    }

    fn note_at(path: &str, modified: SystemTime) -> vault_core::Note {
        vault_core::Note {
            path: path.to_string(),
            modified,
            size: 1,
            hash: [0; 32],
            meta: vault_core::frontmatter::FrontmatterMeta::default(),
            frontmatter_raw: None,
            title: path.to_string(),
            tags: Vec::new(),
            headings: Vec::new(),
            block_ids: Vec::new(),
            outgoing_links: Vec::new(),
        }
    }

    #[test]
    fn an_empty_query_ranks_recently_touched_notes_first() {
        // b and c share one timestamp to exercise the alphabetical
        // tie-break deterministically (SystemTime resolution is coarse).
        let now = SystemTime::now();
        let tie = now;
        let notes = [
            note_at("a.md", now - Duration::from_secs(100)),
            note_at("b.md", tie),
            note_at("c.md", tie),
            note_at("d.md", now - Duration::from_secs(1)),
        ];
        let mut switcher = QuickSwitcher::default();
        switcher.set_query("", &notes, SWITCHER_MAX_ITEMS);
        // Newest first; the shared-timestamp pair breaks its tie
        // alphabetically (b before c by path), then d (1 s ago), then a.
        assert_eq!(switcher.items, ["b.md", "c.md", "d.md", "a.md"]);
    }

    #[test]
    fn a_query_fuzzy_ranks_all_paths_and_the_selection_wraps() {
        let notes = [note("Journal/2026/daily.md", 1), note("Notes/alpha.md", 2)];
        let mut switcher = QuickSwitcher::default();
        switcher.set_query("alpha", &notes, SWITCHER_MAX_ITEMS);
        assert_eq!(switcher.items, vec!["Notes/alpha.md".to_string()]);
        assert_eq!(
            switcher.selected_path(),
            Some(&"Notes/alpha.md".to_string())
        );

        // Selection wraps instead of running off the ends.
        switcher.move_selection(1);
        assert_eq!(switcher.selected, 0, "wraps at the bottom");
        switcher.move_selection(-1);
        assert_eq!(
            switcher.selected,
            switcher.items.len() - 1,
            "wraps at the top"
        );
    }

    #[test]
    fn toggling_resets_and_clamps_and_closes() {
        let notes = [note("a.md", 1), note("b.md", 2), note("c.md", 3)];
        let mut switcher = QuickSwitcher::default();
        switcher.toggle();
        assert!(switcher.open);
        assert_eq!(switcher.query, "");
        switcher.set_query("", &notes, SWITCHER_MAX_ITEMS);
        switcher.move_selection(2);
        assert_eq!(switcher.selected, 2);

        // Closing keeps items; reopening resets query + selection.
        switcher.close();
        assert!(!switcher.open);
        switcher.toggle();
        assert!(switcher.open);
        assert_eq!(switcher.selected, 0);

        // A limit clamps the item list.
        switcher.set_query("", &notes, 2);
        assert_eq!(switcher.items.len(), 2);
        assert_eq!(switcher.selected, 0, "clamped to a valid selection");
    }

    #[test]
    fn outline_lists_headings_in_document_order() {
        let doc = vault_md::parse_note("# Top\n\npara\n\n## Sub\n\ntext\n");
        let entries = outline(&doc);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "Top");
        assert_eq!(entries[0].level, 1);
        assert_eq!(entries[1].text, "Sub");
        assert_eq!(entries[1].level, 2);
        assert!(entries[1].block > entries[0].block);
    }

    #[test]
    fn backlink_and_tag_view_models_read_a_real_vault() {
        // The repo's basic fixture: Welcome <-> Deep Note link both ways.
        let root = tests_dir().join("basic");
        let vault = vault_core::Vault::open(&root).expect("fixture vault opens");

        let backlinks = sorted_backlinks(&vault, "notes/sub/Deep Note.md");
        assert_eq!(backlinks, vec!["notes/Welcome.md".to_string()]);

        // A scratch vault exercises the tag drill-down end to end.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("a.md"),
            "---\ntags: [alpha, beta]\n---\n\nbody\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("b.md"), "---\ntags: [alpha]\n---\n\nbody\n").unwrap();
        let scratch = vault_core::Vault::open(tmp.path()).unwrap();
        assert_eq!(
            tag_counts(&scratch),
            vec![("alpha".to_string(), 2usize), ("beta".to_string(), 1usize)]
        );
        assert_eq!(
            tag_notes(&scratch, "alpha"),
            vec!["a.md".to_string(), "b.md".to_string()]
        );
    }

    #[test]
    fn the_vault_index_adapter_serves_views_and_resolution() {
        // The same fixture data arrives through the adapter the Windows
        // shell holds (the free view models delegate to it).
        let root = tests_dir().join("basic");
        let vault = vault_core::Vault::open(&root).expect("fixture vault opens");
        let adapter: &dyn NoteIndex = &vault;

        assert_eq!(
            adapter.backlinks("notes/sub/Deep Note.md"),
            vec!["notes/Welcome.md".to_string()]
        );

        // Wikilink targets resolve to the fixture's note paths
        // (nearest-first is vault-core's index policy).
        assert_eq!(
            adapter.resolve_note("Deep Note", "notes/Welcome.md"),
            Some("notes/sub/Deep Note.md".to_string())
        );
        assert_eq!(
            adapter.resolve_note("notes/Getting Started.md", "notes/Welcome.md"),
            Some("notes/Getting Started.md".to_string())
        );
        assert_eq!(
            adapter.resolve_note("no such note", "notes/Welcome.md"),
            None
        );

        // Tag counts arrive through the same adapter.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "---\ntags: [alpha]\n---\n\nbody\n").unwrap();
        let scratch = vault_core::Vault::open(tmp.path()).unwrap();
        let scratch_index: &dyn NoteIndex = &scratch;
        assert_eq!(
            scratch_index.tag_counts(),
            vec![("alpha".to_string(), 1usize)]
        );
    }
}
