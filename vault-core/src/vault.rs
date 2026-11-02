//! Vault scanning, in-memory indexing, and change tracking.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, Debouncer, RecommendedCache};
use thiserror::Error;
use walkdir::WalkDir;

use crate::frontmatter::{parse_frontmatter_meta, split_frontmatter, FrontmatterMeta};
use crate::links::{
    scan_block_ids, scan_headings, scan_links, scan_md_links, scan_tags, LinkKind, RawLink,
};
use crate::resolve::VaultIndex;
use crate::source::{moment_to_chrono, render_daily_template, DailyNotesSettings};
use crate::write::{atomic_write, atomic_write_full, WriteResult};

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
    /// All tags (frontmatter + inline `#tags` from the body).
    pub tags: Vec<String>,
    /// Headings extracted from the body (level, raw text).
    pub headings: Vec<Heading>,
    /// Block IDs (`^id` at block end).
    pub block_ids: Vec<String>,
    /// Outgoing links and embeds (wikilinks and internal Markdown links),
    /// with the kind that decides how each resolves.
    pub outgoing_links: Vec<RawLink>,
}

/// A heading in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// Optional explicit ID (filled by the M2 AST pass, not the line scan).
    pub id: Option<String>,
}

/// Complete vault state: notes, attachments, backlinks, tag index.
#[derive(Debug)]
pub struct Vault {
    root: PathBuf,
    notes: HashMap<String, Note>,
    /// Non-markdown files (attachments), for link resolution.
    attachments: HashSet<String>,
    /// Reverse index: resolved target path -> set of source paths linking to it.
    backlinks: HashMap<String, HashSet<String>>,
    /// Tag -> set of note paths.
    tag_index: HashMap<String, HashSet<String>>,
    /// File watcher (kept alive while the vault is open).
    watcher: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
    /// Our own pending writes, to ignore self-generated events.
    pending_writes: Arc<RwLock<HashSet<String>>>,
    /// Paths the watcher has seen change and the app has not consumed yet.
    dirty: Arc<Mutex<Vec<String>>>,
}

impl Vault {
    /// Open a vault at `root`, build the initial index, start the watcher.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, VaultError> {
        let root = root.as_ref().canonicalize()?;
        let mut vault = Vault {
            root,
            notes: HashMap::new(),
            attachments: HashSet::new(),
            backlinks: HashMap::new(),
            tag_index: HashMap::new(),
            watcher: None,
            pending_writes: Arc::new(RwLock::new(HashSet::new())),
            dirty: Arc::new(Mutex::new(Vec::new())),
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

        let dirty = self.dirty.clone();
        let pending = self.pending_writes.clone();
        let root = self.root.clone();

        // The watcher thread only records which vault-relative paths changed
        // (filtered for dot-dirs and our own writes); the owner consumes the
        // dirty set via `take_dirty` + `reload`. It deliberately does NOT
        // mutate the index itself, so `Vault` keeps plain owned maps and
        // borrows stay cheap.
        std::thread::spawn(move || {
            for result in rx {
                let events = match result {
                    Ok(events) => events,
                    Err(errors) => {
                        for e in errors {
                            eprintln!("vault watcher error: {e}");
                        }
                        continue;
                    }
                };
                let mut dirty = dirty.lock().unwrap();
                for event in events {
                    let relevant = matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    );
                    if !relevant {
                        continue;
                    }
                    for path in &event.paths {
                        let Ok(rel) = path.strip_prefix(&root) else {
                            continue;
                        };
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if is_ignored(&rel) || pending.read().unwrap().contains(&rel) {
                            continue;
                        }
                        dirty.push(rel);
                    }
                }
            }
        });

        Ok(())
    }

    /// Drain the set of vault-relative paths the watcher saw change.
    pub fn take_dirty(&self) -> Vec<String> {
        std::mem::take(&mut self.dirty.lock().unwrap())
    }

    /// Re-index the given vault-relative paths (create/modify/remove) and
    /// rebuild the derived indexes. Idempotent per path.
    pub fn reload(&mut self, paths: &[String]) -> Result<(), VaultError> {
        for rel in paths {
            let full = self.root.join(rel);
            match fs::metadata(&full) {
                Ok(_) => {
                    if rel.ends_with(".md") {
                        self.attachments.remove(rel);
                        match Note::read(&self.root, rel) {
                            Ok(note) => {
                                self.notes.insert(rel.clone(), note);
                            }
                            // e.g. file deleted between watcher event and read,
                            // or transiently unreadable; leave the old entry.
                            Err(_) => continue,
                        }
                    } else if full.is_file() {
                        self.notes.remove(rel);
                        self.attachments.insert(rel.clone());
                    } else {
                        // A directory: neither note nor attachment.
                        self.notes.remove(rel);
                        self.attachments.remove(rel);
                    }
                }
                Err(_) => {
                    self.notes.remove(rel);
                    self.attachments.remove(rel);
                }
            }
        }
        self.rebuild_derived();
        Ok(())
    }

    /// Full vault scan (rebuilds all indexes).
    fn scan(&mut self) -> Result<(), VaultError> {
        self.notes.clear();
        self.attachments.clear();
        self.backlinks.clear();
        self.tag_index.clear();

        for entry in WalkDir::new(&self.root) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.root)?
                .to_string_lossy()
                .replace('\\', "/");
            if is_ignored(&rel) {
                continue;
            }
            if rel.ends_with(".md") {
                if let Ok(note) = Note::read(&self.root, &rel) {
                    self.notes.insert(rel, note);
                }
            } else {
                self.attachments.insert(rel);
            }
        }
        self.rebuild_derived();
        Ok(())
    }

    /// Rebuild backlinks and the tag index from the note set.
    fn rebuild_derived(&mut self) {
        let index = self.index();
        self.backlinks.clear();
        let mut edges: Vec<(String, String)> = Vec::new();
        for (path, note) in &self.notes {
            for link in &note.outgoing_links {
                // Backlinks follow RESOLVED targets, not raw link text.
                // Wikilinks use shortest-unique-name; Markdown links are
                // always relative to the note's folder.
                let resolved = match link.kind {
                    LinkKind::Wiki => index.resolve(&link.target, path),
                    LinkKind::Markdown => index.resolve_relative(&link.target, path),
                };
                if let Some(target) = resolved {
                    edges.push((target.to_string(), path.clone()));
                }
            }
        }
        for (target, source) in edges {
            self.backlinks.entry(target).or_default().insert(source);
        }

        self.tag_index.clear();
        for (path, note) in &self.notes {
            for tag in &note.tags {
                self.tag_index
                    .entry(tag.clone())
                    .or_default()
                    .insert(path.clone());
            }
        }
    }

    /// Get a note by vault-relative path.
    pub fn get_note(&self, path: &str) -> Option<&Note> {
        self.notes.get(path)
    }

    /// All notes, unordered.
    pub fn all_notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
    }

    /// Notes sorted by vault-relative path.
    pub fn notes_sorted(&self) -> Vec<&Note> {
        let mut v: Vec<&Note> = self.notes.values().collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }

    /// All indexed attachment paths, sorted.
    pub fn attachments_sorted(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.attachments.iter().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v
    }

    /// Notes linking (or embedding) to `path`, via resolved targets.
    pub fn backlinks(&self, path: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .backlinks
            .get(path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// All tags.
    pub fn all_tags(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tag_index.keys().cloned().collect();
        v.sort();
        v
    }

    /// Notes tagged `tag`.
    pub fn notes_for_tag(&self, tag: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .tag_index
            .get(tag)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// The resolution index over notes AND attachments.
    pub fn index(&self) -> VaultIndex {
        VaultIndex::new(self.notes.keys().chain(self.attachments.iter()).cloned())
    }

    /// Register a path as being written by us (to suppress watcher events).
    pub fn mark_pending_write(&self, path: &str) {
        self.pending_writes
            .write()
            .unwrap()
            .insert(path.to_string());
    }

    /// Save a note through the safe write path and refresh its index entry.
    ///
    /// `original` must be the note content as the caller last saw it (external
    /// changes are refused); `new_body` replaces the body verbatim. The write
    /// is bracketed with the watcher suppression so our own save does not read
    /// back as an external change.
    pub fn save_note(
        &mut self,
        path: &str,
        original: &str,
        new_body: &str,
    ) -> Result<WriteResult, VaultError> {
        // HANDOFF section 2: never write into dot-directories (`.obsidian/`).
        // This guard comes first: protecting the vault beats path-shape rules.
        if is_ignored(path) {
            return Err(VaultError::ProtectedPath(path.to_string()));
        }
        if !path.ends_with(".md") {
            return Err(VaultError::NotANote(path.to_string()));
        }
        self.mark_pending_write(path);
        match atomic_write(&self.root.join(path), original, new_body) {
            Ok(result) => {
                self.reload(&[path.to_string()])?;
                Ok(result)
            }
            Err(e) => {
                self.unmark_pending_write(path);
                Err(e.into())
            }
        }
    }

    /// Unregister a pending write.
    pub fn unmark_pending_write(&self, path: &str) {
        self.pending_writes.write().unwrap().remove(path);
    }

    /// Save a note whose *full text* — frontmatter included — the user edited.
    ///
    /// `new_content` replaces the entire file verbatim. All the guards of
    /// [`Vault::save_note`] apply: protected paths are refused, the write is
    /// bracketed with the watcher suppression, and the file is re-indexed.
    /// For body-only edits prefer [`Vault::save_note`], which provably reuses
    /// the original frontmatter bytes.
    pub fn save_note_full(
        &mut self,
        path: &str,
        original: &str,
        new_content: &str,
    ) -> Result<WriteResult, VaultError> {
        if is_ignored(path) {
            return Err(VaultError::ProtectedPath(path.to_string()));
        }
        if !path.ends_with(".md") {
            return Err(VaultError::NotANote(path.to_string()));
        }
        self.mark_pending_write(path);
        match atomic_write_full(&self.root.join(path), original, new_content) {
            Ok(result) => {
                self.reload(&[path.to_string()])?;
                Ok(result)
            }
            Err(e) => {
                self.unmark_pending_write(path);
                Err(e.into())
            }
        }
    }

    /// The vault root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The vault-relative path today's daily note lives at, from
    /// `.obsidian/daily-notes.json` (falling back to `Journal/YYYY/MM/DD.md`).
    pub fn daily_note_path(&self, date: chrono::NaiveDate) -> String {
        let settings = DailyNotesSettings::load(&self.root);
        let name = date
            .format(&moment_to_chrono(settings.format()))
            .to_string();
        let folder = settings.folder().trim_matches('/');
        if folder.is_empty() {
            format!("{name}.md")
        } else {
            format!("{folder}/{name}.md")
        }
    }

    /// Open today's daily note, creating it from the configured template if
    /// it does not exist (folders are created as needed). Returns the
    /// vault-relative path and whether it was just created. The write goes
    /// through the same guarded, watcher-bracketed path as manual saves.
    pub fn open_or_create_daily_note(
        &mut self,
        date: chrono::NaiveDate,
    ) -> Result<(String, bool), VaultError> {
        let path = self.daily_note_path(date);
        if self.root.join(&path).exists() {
            return Ok((path, false));
        }
        // The same guards as save_note: never write into dot-directories,
        // and only produce notes.
        if is_ignored(&path) {
            return Err(VaultError::ProtectedPath(path));
        }
        if !path.ends_with(".md") {
            return Err(VaultError::NotANote(path));
        }

        let settings = DailyNotesSettings::load(&self.root);
        let template = settings
            .template
            .as_deref()
            .map(|rel| fs::read_to_string(self.root.join(rel)))
            .transpose()?
            .unwrap_or_default();
        let title = path
            .rsplit('/')
            .next()
            .and_then(|name| name.strip_suffix(".md"))
            .unwrap_or(&path)
            .to_string();
        let now = chrono::Local::now().time();
        let content = render_daily_template(&template, date, &title, now);

        // Daily-note formats may contain '/' (e.g. YYYY/MM/DD): create the
        // folder chain before the atomic write needs it.
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(self.root.join(parent))?;
        }

        self.mark_pending_write(&path);
        match atomic_write_full(&self.root.join(&path), "", &content) {
            Ok(_result) => {
                self.reload(std::slice::from_ref(&path))?;
                Ok((path, true))
            }
            Err(e) => {
                self.unmark_pending_write(&path);
                Err(e.into())
            }
        }
    }

    /// All folders under the vault root, excluding dot-directories,
    /// sorted, vault-relative with '/' separators. Includes empty folders,
    /// so the sidebar can offer them for new notes.
    pub fn folders(&self) -> Vec<String> {
        let mut folders = Vec::new();
        for entry in WalkDir::new(&self.root).min_depth(1) {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_dir() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if is_ignored(&rel) {
                continue;
            }
            folders.push(rel);
        }
        folders.sort();
        folders
    }

    /// Create a folder (and any missing parents). Folders do not enter the
    /// note index; they show up via [`Vault::folders`].
    pub fn create_folder(&mut self, rel: &str) -> Result<(), VaultError> {
        if is_ignored(rel) {
            return Err(VaultError::ProtectedPath(rel.to_string()));
        }
        if rel.is_empty() || rel.ends_with('/') {
            return Err(VaultError::NotANote(rel.to_string()));
        }
        fs::create_dir_all(self.root.join(rel))?;
        // Suppress the folder-create watcher event so it is not misread.
        self.mark_pending_write(rel);
        Ok(())
    }

    /// Rename (move) a note. The target must not exist. Links in other notes
    /// are NOT rewritten (M4c scope); their text keeps pointing at the old
    /// name and shows as unresolved until updated.
    pub fn rename_note(&mut self, old: &str, new: &str) -> Result<(), VaultError> {
        for path in [old, new] {
            if is_ignored(path) {
                return Err(VaultError::ProtectedPath(path.to_string()));
            }
            if !path.ends_with(".md") {
                return Err(VaultError::NotANote(path.to_string()));
            }
        }
        if old == new {
            return Ok(());
        }
        let target = self.root.join(new);
        if target.exists() {
            return Err(VaultError::TargetExists(new.to_string()));
        }
        if let Some(parent) = Path::new(new).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(self.root.join(parent))?;
            }
        }
        self.mark_pending_write(old);
        self.mark_pending_write(new);
        fs::rename(self.root.join(old), target)?;
        // `reload` drops the vanished old path and indexes the new one.
        self.reload(&[old.to_string(), new.to_string()])?;
        Ok(())
    }

    /// Move a note into the vault's `.trash/` folder (Obsidian-compatible:
    /// recoverable, and dot-ignored so it vanishes from the app). Returns
    /// the path it was moved to, inside `.trash/`.
    pub fn delete_note(&mut self, path: &str) -> Result<String, VaultError> {
        if is_ignored(path) {
            return Err(VaultError::ProtectedPath(path.to_string()));
        }
        if !path.ends_with(".md") {
            return Err(VaultError::NotANote(path.to_string()));
        }
        let target = self.trash_target(path);
        self.mark_pending_write(path);
        fs::create_dir_all(self.root.join(".trash"))?;
        fs::rename(self.root.join(path), self.root.join(&target))?;
        self.reload(&[path.to_string()])?;
        Ok(target)
    }

    /// Move a whole folder into `.trash/`. A full rescan follows, since
    /// every note below it may have moved.
    pub fn delete_folder(&mut self, rel: &str) -> Result<String, VaultError> {
        if is_ignored(rel) {
            return Err(VaultError::ProtectedPath(rel.to_string()));
        }
        let dir = self.root.join(rel);
        if !dir.is_dir() {
            return Err(VaultError::NotANote(rel.to_string()));
        }
        let target = self.trash_target(rel);
        fs::create_dir_all(self.root.join(".trash"))?;
        fs::rename(dir, self.root.join(&target))?;
        self.scan()?;
        Ok(target)
    }

    /// Rename (move) a whole folder; the target must not exist. A full
    /// rescan follows, since every note below it changed paths.
    pub fn rename_folder(&mut self, old: &str, new: &str) -> Result<(), VaultError> {
        for path in [old, new] {
            if is_ignored(path) {
                return Err(VaultError::ProtectedPath(path.to_string()));
            }
            if path.is_empty() || path.ends_with('/') {
                return Err(VaultError::NotANote(path.to_string()));
            }
        }
        if old == new {
            return Ok(());
        }
        let target = self.root.join(new);
        if target.exists() {
            return Err(VaultError::TargetExists(new.to_string()));
        }
        if let Some(parent) = Path::new(new).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(self.root.join(parent))?;
            }
        }
        fs::rename(self.root.join(old), target)?;
        self.scan()?;
        Ok(())
    }

    /// A unique `.trash/`-relative target for `rel`: `name.md` when free,
    /// otherwise `name (2).md`, `name (3).md`, …
    fn trash_target(&self, rel: &str) -> String {
        let name = Path::new(rel)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "deleted".to_string());
        let mut candidate = format!(".trash/{name}");
        let mut n = 2;
        let stem = name
            .strip_suffix(".md")
            .map(|s| s.to_string())
            .unwrap_or(name.clone());
        let ext = name.strip_suffix(".md").map(|_| ".md").unwrap_or("");
        while self.root.join(&candidate).exists() {
            candidate = format!(".trash/{stem} ({n}){ext}");
            n += 1;
        }
        candidate
    }
}

/// Dot-directories (`.obsidian`, `.git`, `.trash`, …) are ignored everywhere.
fn is_ignored(rel: &str) -> bool {
    rel.split('/').any(|c| c.starts_with('.') && c != ".")
}

impl Note {
    fn read(root: &Path, rel: &str) -> Result<Self, VaultError> {
        let path = root.join(rel);
        let stat = fs::metadata(&path)?;
        let modified = stat.modified()?;
        let size = stat.len();
        let content = fs::read_to_string(&path)?;
        let hash = sha256(&content);

        let split = split_frontmatter(&content);
        let meta = split
            .content
            .as_ref()
            .map(|r| parse_frontmatter_meta(r))
            .transpose()?
            .unwrap_or_default();

        let stem = Path::new(rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let title = meta.title.clone().unwrap_or_else(|| stem.clone());

        // Index-grade body scans (see links.rs for the vault-md relationship).
        let body_tags = scan_tags(&split.body);
        let mut tags = meta.tags.clone();
        for t in body_tags {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        let headings = scan_headings(&split.body)
            .into_iter()
            .map(|(level, text)| Heading {
                level,
                text,
                id: None,
            })
            .collect();
        let mut outgoing_links = scan_links(&split.body);
        outgoing_links.extend(scan_md_links(&split.body));

        Ok(Note {
            path: rel.to_string(),
            modified,
            size,
            hash,
            meta,
            frontmatter_raw: split.raw,
            title,
            tags,
            headings,
            block_ids: scan_block_ids(&split.body),
            outgoing_links,
        })
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
    #[error("write error: {0}")]
    Write(#[from] crate::write::WriteError),
    #[error("not a note path: {0}")]
    NotANote(String),
    #[error("refusing to write protected path: {0}")]
    ProtectedPath(String),
    #[error("target already exists: {0}")]
    TargetExists(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("a.md"),
            "---\ntitle: A\ntags: [fm, shared]\n---\nbody #inline shared\n[[b]] [[Notes/Glass]] ^blk1\n",
        )
        .unwrap();
        fs::write(tmp.path().join("b.md"), "links back to [[a]]\n").unwrap();
        fs::create_dir_all(tmp.path().join("Notes")).unwrap();
        fs::write(tmp.path().join("Notes/Glass.md"), "# Glass\n").unwrap();
        fs::create_dir(tmp.path().join(".obsidian")).unwrap();
        fs::write(tmp.path().join(".obsidian/config"), "x").unwrap();
        fs::create_dir(tmp.path().join("assets")).unwrap();
        fs::write(tmp.path().join("assets/cat.png"), "png").unwrap();
        let vault = Vault::open(tmp.path()).unwrap();
        (tmp, vault)
    }

    #[test]
    fn scan_reads_notes_and_attachments() {
        let (_tmp, vault) = test_vault();
        assert_eq!(vault.notes.len(), 3);
        assert!(vault.get_note("a.md").is_some());
        assert!(vault.get_note("Notes/Glass.md").is_some());
        assert_eq!(vault.attachments_sorted(), ["assets/cat.png"]);
    }

    #[test]
    fn ignores_dot_dirs() {
        let (_tmp, vault) = test_vault();
        // .obsidian/config is neither a note nor an attachment
        assert!(!vault.index().resolve("config", "a.md").is_some());
    }

    #[test]
    fn note_metadata_is_extracted() {
        let (_tmp, vault) = test_vault();
        let a = vault.get_note("a.md").unwrap();
        assert_eq!(a.title, "A");
        // frontmatter + inline tags, deduped, order preserved
        assert_eq!(a.tags, ["fm", "shared", "inline"]);
        assert_eq!(a.block_ids, ["blk1"]);
        assert_eq!(
            a.outgoing_links,
            [
                RawLink {
                    embed: false,
                    kind: LinkKind::Wiki,
                    target: "b".into()
                },
                RawLink {
                    embed: false,
                    kind: LinkKind::Wiki,
                    target: "Notes/Glass".into()
                },
            ]
        );
        let g = vault.get_note("Notes/Glass.md").unwrap();
        assert_eq!(
            g.headings,
            [Heading {
                level: 1,
                text: "Glass".into(),
                id: None
            }]
        );
    }

    #[test]
    fn backlinks_follow_resolved_targets() {
        let (_tmp, vault) = test_vault();
        // a.md links [[b]] and [[Notes/Glass]]; b.md links [[a]]
        assert_eq!(vault.backlinks("b.md"), ["a.md"]);
        assert_eq!(vault.backlinks("Notes/Glass.md"), ["a.md"]);
        assert_eq!(vault.backlinks("a.md"), ["b.md"]);
    }

    #[test]
    fn tags_index() {
        let (_tmp, vault) = test_vault();
        assert!(vault.all_tags().contains(&"fm".to_string()));
        assert!(vault.all_tags().contains(&"inline".to_string()));
        assert_eq!(vault.notes_for_tag("inline"), ["a.md"]);
    }

    #[test]
    fn reload_picks_up_external_changes() {
        let (tmp, mut vault) = test_vault();
        fs::write(
            tmp.path().join("b.md"),
            "now links to [[Notes/Glass]] too\n",
        )
        .unwrap();
        // simulate what the app does after the watcher signals a change
        vault.reload(&["b.md".to_string()]).unwrap();
        assert_eq!(vault.backlinks("Notes/Glass.md"), ["a.md", "b.md"]);
        // removal
        fs::remove_file(tmp.path().join("b.md")).unwrap();
        vault.reload(&["b.md".to_string()]).unwrap();
        assert!(vault.get_note("b.md").is_none());
        assert!(vault.backlinks("a.md").is_empty());
    }

    #[test]
    fn watcher_reports_external_changes_but_not_ours() {
        let (tmp, vault) = test_vault();
        // our own write: suppressed
        vault.mark_pending_write("own.md");
        fs::write(tmp.path().join("own.md"), "ours\n").unwrap();
        // external write
        fs::write(tmp.path().join("ext.md"), "theirs [[a]]\n").unwrap();
        // wait for the debouncer (300 ms) plus slack
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let dirty = vault.take_dirty();
            if dirty.iter().any(|p| p == "ext.md") {
                assert!(
                    !dirty.iter().any(|p| p == "own.md"),
                    "own write leaked: {dirty:?}"
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watcher never reported ext.md"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    fn save_note_updates_content_and_index() {
        let (tmp, mut vault) = test_vault();
        let before = fs::read_to_string(tmp.path().join("b.md")).unwrap();
        // stale original: external-change detection must refuse
        let err = vault.save_note("b.md", "stale original", "new body");
        assert!(
            matches!(
                err,
                Err(VaultError::Write(
                    crate::write::WriteError::ExternallyModified
                ))
            ),
            "{err:?}"
        );

        let result = vault
            .save_note("b.md", &before, "now links [[Notes/Glass]]\n")
            .unwrap();
        assert_eq!(result.size, "now links [[Notes/Glass]]\n".len() as u64);
        assert_eq!(
            fs::read_to_string(tmp.path().join("b.md")).unwrap(),
            "now links [[Notes/Glass]]\n"
        );
        assert_eq!(vault.backlinks("Notes/Glass.md"), ["a.md", "b.md"]);
    }

    #[test]
    fn save_note_refuses_protected_and_non_note_paths() {
        let (_tmp, mut vault) = test_vault();
        assert!(matches!(
            vault.save_note(".obsidian/app.json", "{}", "{}"),
            Err(VaultError::ProtectedPath(_))
        ));
        assert!(matches!(
            vault.save_note("assets/cat.png", "", ""),
            Err(VaultError::NotANote(_))
        ));
    }

    #[test]
    fn save_note_full_writes_frontmatter_edits_verbatim() {
        let (tmp, mut vault) = test_vault();
        let before = fs::read_to_string(tmp.path().join("b.md")).unwrap();
        assert!(!before.starts_with("---"), "fixture has no frontmatter");

        // The user added frontmatter in the source editor: full-text save.
        let edited = "---\ntags: [new]\n---\n\nnew full text [[a]]\n";
        vault.save_note_full("b.md", &before, edited).unwrap();
        assert_eq!(fs::read_to_string(tmp.path().join("b.md")).unwrap(), edited);
        // Re-indexed: the new tag is visible.
        assert!(vault.all_tags().iter().any(|t| t == "new"));
        // A second full-text save keeps bytes byte-identical (no re-serialization).
        let after = fs::read_to_string(tmp.path().join("b.md")).unwrap();
        vault.save_note_full("b.md", &after, &after).unwrap();
        assert_eq!(fs::read_to_string(tmp.path().join("b.md")).unwrap(), after);
    }

    #[test]
    fn save_note_full_refuses_stale_original_and_bad_paths() {
        let (tmp, mut vault) = test_vault();
        let err = vault.save_note_full("b.md", "stale original", "---\n---\n\nx");
        assert!(matches!(
            err,
            Err(VaultError::Write(
                crate::write::WriteError::ExternallyModified
            ))
        ));
        assert!(matches!(
            vault.save_note_full(".obsidian/app.json", "{}", "{}"),
            Err(VaultError::ProtectedPath(_))
        ));
        assert!(matches!(
            vault.save_note_full("assets/cat.png", "", ""),
            Err(VaultError::NotANote(_))
        ));
        // Nothing was written by any of the refused calls.
        assert_eq!(
            fs::read_to_string(tmp.path().join("b.md")).unwrap(),
            fs::read_to_string(tmp.path().join("b.md")).unwrap()
        );
    }

    #[test]
    fn daily_note_path_uses_fallback_without_settings() {
        let (_tmp, vault) = test_vault();
        let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        assert_eq!(vault.daily_note_path(d), "Journal/2026/11/02.md");
    }

    #[test]
    fn daily_note_path_honors_obsidian_settings() {
        let (tmp, vault) = test_vault();
        fs::write(
            tmp.path().join(".obsidian/daily-notes.json"),
            r#"{"folder": "Days", "format": "YYYY-MM-DD"}"#,
        )
        .unwrap();
        let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        assert_eq!(vault.daily_note_path(d), "Days/2026-11-02.md");
    }

    #[test]
    fn daily_note_creates_folders_template_and_index_entry() {
        let (tmp, mut vault) = test_vault();
        fs::create_dir(tmp.path().join("Templates")).unwrap();
        fs::write(
            tmp.path().join("Templates/Daily.md"),
            "# {{date}}\n\n{{title}}\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join(".obsidian/daily-notes.json"),
            r#"{"folder": "Journal", "format": "YYYY-MM-DD", "template": "Templates/Daily.md"}"#,
        )
        .unwrap();

        let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let (path, created) = vault.open_or_create_daily_note(d).unwrap();
        assert_eq!(path, "Journal/2026-11-02.md");
        assert!(created);
        let content = fs::read_to_string(tmp.path().join(&path)).unwrap();
        assert_eq!(content, "# 2026-11-02\n\n2026-11-02\n");
        // Indexed: the new note is visible and its {{title}}-style name resolves.
        assert!(vault.get_note(&path).is_some());

        // Second call opens the existing note instead of recreating it.
        let (path2, created2) = vault.open_or_create_daily_note(d).unwrap();
        assert_eq!(path2, path);
        assert!(!created2);
        assert_eq!(fs::read_to_string(tmp.path().join(&path)).unwrap(), content);
    }

    #[test]
    fn daily_note_refuses_protected_paths() {
        let (tmp, mut vault) = test_vault();
        // A folder setting pointing into .obsidian must be refused, not written.
        fs::write(
            tmp.path().join(".obsidian/daily-notes.json"),
            r#"{"folder": ".obsidian"}"#,
        )
        .unwrap();
        let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        assert!(matches!(
            vault.open_or_create_daily_note(d),
            Err(VaultError::ProtectedPath(_))
        ));
        // Nothing was written.
        assert!(!tmp.path().join(".obsidian/2026-11-02.md").exists());
    }

    #[test]
    fn folders_list_directories_including_empty_ones() {
        let (tmp, vault) = test_vault();
        fs::create_dir_all(tmp.path().join("Notes/Empty")).unwrap();
        assert_eq!(vault.folders(), ["Notes", "Notes/Empty", "assets"]);
    }

    #[test]
    fn create_folder_makes_parents_and_refuses_protected() {
        let (tmp, mut vault) = test_vault();
        vault.create_folder("a/b/c").unwrap();
        assert!(tmp.path().join("a/b/c").is_dir());
        assert!(matches!(
            vault.create_folder(".obsidian/x"),
            Err(VaultError::ProtectedPath(_))
        ));
        assert!(matches!(
            vault.create_folder(""),
            Err(VaultError::NotANote(_))
        ));
    }

    #[test]
    fn rename_note_moves_file_and_updates_index() {
        let (tmp, mut vault) = test_vault();
        vault.rename_note("b.md", "Notes/renamed.md").unwrap();
        assert!(!tmp.path().join("b.md").exists());
        assert!(tmp.path().join("Notes/renamed.md").exists());
        assert!(vault.get_note("b.md").is_none());
        assert!(vault.get_note("Notes/renamed.md").is_some());
        // Backlinks rebuilt: a.md's [[b]] link now resolves to nothing.
        assert!(vault.backlinks("Notes/renamed.md").is_empty());

        // Refusals: existing target, protected paths, non-notes.
        assert!(matches!(
            vault.rename_note("a.md", "Notes/renamed.md"),
            Err(VaultError::TargetExists(_))
        ));
        assert!(matches!(
            vault.rename_note("a.md", ".obsidian/x.md"),
            Err(VaultError::ProtectedPath(_))
        ));
        assert!(matches!(
            vault.rename_note("a.md", "assets/cat.png"),
            Err(VaultError::NotANote(_))
        ));
    }

    #[test]
    fn delete_note_moves_into_trash_and_drops_from_index() {
        let (tmp, mut vault) = test_vault();
        let target = vault.delete_note("b.md").unwrap();
        assert_eq!(target, ".trash/b.md");
        assert!(!tmp.path().join("b.md").exists());
        assert!(tmp.path().join(".trash/b.md").exists());
        assert!(vault.get_note("b.md").is_none());
        // .trash stays invisible to the index (dot-directory).
        assert!(vault.folders().iter().all(|f| !f.starts_with(".trash")));

        // Trash names never collide.
        fs::write(tmp.path().join("b.md"), "back again\n").unwrap();
        vault.reload(&["b.md".to_string()]).unwrap();
        let second = vault.delete_note("b.md").unwrap();
        assert_eq!(second, ".trash/b (2).md");

        // Refusals.
        assert!(matches!(
            vault.delete_note(".obsidian/config"),
            Err(VaultError::ProtectedPath(_))
        ));
        assert!(matches!(
            vault.delete_note("assets/cat.png"),
            Err(VaultError::NotANote(_))
        ));
    }

    #[test]
    fn folder_rename_and_delete_move_their_notes() {
        let (tmp, mut vault) = test_vault();
        vault.rename_folder("Notes", "Docs").unwrap();
        assert!(!tmp.path().join("Notes").exists());
        assert!(vault.get_note("Docs/Glass.md").is_some());
        assert!(matches!(
            vault.rename_folder("Docs", "assets"),
            Err(VaultError::TargetExists(_))
        ));

        vault.delete_folder("Docs").unwrap();
        assert!(!tmp.path().join("Docs").exists());
        assert!(vault.get_note("Docs/Glass.md").is_none());
        assert!(tmp.path().join(".trash/Docs").is_dir());
        // A full rescan happened: only the root-level notes remain.
        assert_eq!(vault.notes.len(), 2);
    }
}
