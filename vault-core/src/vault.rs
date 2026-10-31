//! Vault scanning and in-memory indexing.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};
use walkdir::WalkDir;

use crate::frontmatter::{parse_frontmatter_meta, split_frontmatter, FrontmatterMeta};
use crate::resolve::VaultIndex;
use thiserror::Error;

/// A note file in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Vault-relative path with '/' separators.
    pub path: String,
    /// File modification time (for external change detection).
    pub modified: SystemTime,
    /// File size in bytes.
    pub size: u64,
    /// SHA-256 hash of file contents (for exact change detection).
    pub hash: [u8; 32],
    /// Parsed frontmatter metadata.
    pub meta: FrontmatterMeta,
    /// Raw frontmatter block (verbatim, including delimiters).
    pub frontmatter_raw: Option<String>,
    /// Note title: frontmatter `title` or file stem.
    pub title: String,
    /// All tags (frontmatter + inline #tags from body).
    pub tags: Vec<String>,
    /// Headings extracted from the note.
    pub headings: Vec<Heading>,
    /// Block IDs (`^id` at end of paragraphs).
    pub block_ids: Vec<String>,
    /// Outgoing wikilinks and embeds (target strings as written).
    pub outgoing_links: Vec<String>,
}

/// A heading in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// Optional explicit ID from `{#custom-id}` attribute.
    pub id: Option<String>,
}

/// Complete vault state: index, notes, backlinks, tag index.
#[derive(Debug, Default)]
pub struct Vault {
    root: PathBuf,
    notes: HashMap<String, Note>,
    /// Reverse index: target path -> set of source paths that link to it.
    backlinks: HashMap<String, HashSet<String>>,
    /// Tag -> set of note paths.
    tag_index: HashMap<String, HashSet<String>>,
    /// File watcher (kept alive while vault is open).
    watcher: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
    /// Our own pending writes, to ignore self-generated events.
    pending_writes: Arc<RwLock<HashSet<String>>>,
}

impl Vault {
    /// Open a vault at `root`, build the initial index.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, VaultError> {
        let root = root.as_ref().canonicalize()?;
        let mut vault = Vault {
            root: root.clone(),
            notes: HashMap::new(),
            backlinks: HashMap::new(),
            tag_index: HashMap::new(),
            watcher: None,
            pending_writes: Arc::new(RwLock::new(HashSet::new())),
        };
        vault.scan()?;
        vault.start_watcher()?;
        Ok(vault)
    }

    fn start_watcher(&mut self) -> Result<(), VaultError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(std::time::Duration::from_millis(300), None, tx)?;
        debouncer.watch(&self.root, RecursiveMode::Recursive)?;
        self.watcher = Some(debouncer);

        // Spawn handler thread.
        let notes = Arc::new(RwLock::new(HashMap::new()));
        let backlinks = Arc::new(RwLock::new(HashMap::new()));
        let tag_index = Arc::new(RwLock::new(HashMap::new()));
        let pending = self.pending_writes.clone();
        let root = self.root.clone();

        std::thread::spawn(move || {
            for result in rx {
                match result {
                    Ok(events) => {
                        for event in events {
                            Self::handle_event(
                                &root, &event, &notes, &backlinks, &tag_index, &pending,
                            );
                        }
                    }
                    Err(errors) => {
                        for e in errors {
                            eprintln!("vault watcher error: {e}");
                        }
                    }
                }
            }
        });

        Ok(())
    }

    fn handle_event(
        root: &Path,
        event: &DebouncedEvent,
        notes: &RwLock<HashMap<String, Note>>,
        backlinks: &RwLock<HashMap<String, HashSet<String>>>,
        tag_index: &RwLock<HashMap<String, HashSet<String>>>,
        pending: &Arc<RwLock<HashSet<String>>>,
    ) {
        // DebouncedEvent derefs to Event, which has `paths` and `kind`.
        let paths = &event.paths;
        for path in paths {
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };

            // Skip dot-directories.
            if rel.split('/').any(|c| c.starts_with('.') && c != ".") {
                continue;
            }

            // Skip our own writes.
            if pending.read().unwrap().contains(&rel) {
                continue;
            }

            match &event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    if rel.ends_with(".md") {
                        if let Ok(note) = Note::read(&root.join(&rel)) {
                            let mut n = notes.write().unwrap();
                            let _old = n.insert(rel.clone(), note.clone());
                            // Update backlinks and tag index...
                        }
                    }
                }
                EventKind::Remove(_) => {
                    notes.write().unwrap().remove(&rel);
                    backlinks.write().unwrap().remove(&rel);
                }
                _ => {}
            }
        }
    }

    /// Full vault scan (rebuilds all indexes).
    fn scan(&mut self) -> Result<(), VaultError> {
        self.notes.clear();
        self.backlinks.clear();
        self.tag_index.clear();

        for entry in WalkDir::new(&self.root) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let path = entry.path();
                let rel = path
                    .strip_prefix(&self.root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                // Skip dot-directories and non-md files.
                if rel.split('/').any(|c| c.starts_with('.') && c != ".") {
                    continue;
                }
                if !rel.ends_with(".md") {
                    continue;
                }
                if let Ok(mut note) = Note::read(path) {
                    note.path = rel.clone();
                    let tags = note.tags.clone();
                    let path_str = note.path.clone();
                    for tag in tags {
                        self.tag_index
                            .entry(tag)
                            .or_default()
                            .insert(path_str.clone());
                    }
                    self.notes.insert(rel, note);
                }
            }
        }
        self.rebuild_backlinks();
        Ok(())
    }

    fn rebuild_backlinks(&mut self) {
        self.backlinks.clear();
        for (path, note) in &self.notes {
            for target in &note.outgoing_links {
                self.backlinks
                    .entry(target.clone())
                    .or_default()
                    .insert(path.clone());
            }
        }
    }

    /// Get a note by vault-relative path.
    pub fn get_note(&self, path: &str) -> Option<&Note> {
        self.notes.get(path)
    }

    /// Get all notes.
    pub fn all_notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
    }

    /// Get backlinks for a note.
    pub fn backlinks(&self, path: &str) -> Vec<String> {
        self.backlinks
            .get(path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all tags.
    pub fn all_tags(&self) -> Vec<String> {
        self.tag_index.keys().cloned().collect()
    }

    /// Get notes for a tag.
    pub fn notes_for_tag(&self, tag: &str) -> Vec<String> {
        self.tag_index
            .get(tag)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the resolution index.
    pub fn index(&self) -> VaultIndex {
        VaultIndex::new(self.notes.keys().cloned())
    }

    /// Register a path as being written by us (to suppress watcher events).
    pub fn mark_pending_write(&self, path: &str) {
        self.pending_writes
            .write()
            .unwrap()
            .insert(path.to_string());
    }

    /// Unregister a pending write.
    pub fn unmark_pending_write(&self, path: &str) {
        self.pending_writes.write().unwrap().remove(path);
    }

    /// Get the vault root.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Note {
    fn read(path: &Path) -> Result<Self, VaultError> {
        let meta = fs::metadata(path)?;
        let modified = meta.modified()?;
        let size = meta.len();
        let content = fs::read_to_string(path)?;
        let hash = sha256(&content);

        let split = split_frontmatter(&content);
        let meta = split
            .content
            .as_ref()
            .map(|r| parse_frontmatter_meta(r))
            .transpose()?
            .unwrap_or_default();

        // Extract title from frontmatter or file stem.
        let title = meta.aliases.first().cloned().unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        });

        // TODO: parse headings, block IDs, outgoing links from body using vault-md
        let note = Note {
            path: String::new(), // Will be set by caller
            modified,
            size,
            hash,
            meta,
            frontmatter_raw: split.raw,
            title,
            tags: Vec::new(),
            headings: Vec::new(),
            block_ids: Vec::new(),
            outgoing_links: Vec::new(),
        };
        Ok(note)
    }
}

fn sha256(data: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hasher.finalize().into()
}

/// Errors from vault operations.
#[derive(Debug, Error)]
pub enum VaultError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Walkdir error: {0}")]
    Walkdir(#[from] walkdir::Error),
    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),
    #[error("Frontmatter error: {0}")]
    Frontmatter(#[from] crate::frontmatter::FrontmatterError),
    #[error("Path strip prefix error: {0}")]
    StripPrefix(#[from] std::path::StripPrefixError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scan_reads_notes() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.md"), "---\ntitle: A\n---\nbody").unwrap();
        fs::write(tmp.path().join("b.md"), "body b").unwrap();
        fs::create_dir(tmp.path().join(".obsidian")).unwrap();
        fs::write(tmp.path().join(".obsidian/config"), "x").unwrap();

        let vault = Vault::open(tmp.path()).unwrap();
        assert_eq!(vault.notes.len(), 2);
        assert!(vault.get_note("a.md").is_some());
        assert!(vault.get_note("b.md").is_some());
    }

    #[test]
    fn ignores_dot_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.md"), "a").unwrap();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        fs::write(tmp.path().join(".git/ignored.md"), "x").unwrap();

        let vault = Vault::open(tmp.path()).unwrap();
        assert_eq!(vault.notes.len(), 1);
    }
}
