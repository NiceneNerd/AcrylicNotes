//! The persistence boundary: read and guarded-write notes.
//!
//! vault-core owns the real vault machinery (safe atomic writes, watcher
//! suppression, indexing). The shared workflow talks to it only through
//! this trait so tests — and future frontends — can inject deterministic
//! fakes instead of a filesystem (restructure plan §"Phase 3: Extract
//! effects", step 3).
//!
//! Contract rules:
//!
//! * Paths are vault-relative, '/'-separated, exactly as vault-core takes
//!   them; path-safety rules stay inside the adapter.
//! * A save is *guarded*: `original` is the text the caller last saw, and
//!   the adapter refuses the write with
//!   [`RepositoryError::ExternallyModified`] when the stored note no
//!   longer matches it (the sentinel that turns an external change into a
//!   conflict instead of a silent clobber).
//! * [`Self::read_note`] mirrors [`vault_core::NoteSource::note_source`]'s
//!   refusal of dot-paths, expressed as an error instead of `None`.

use vault_core::vault::VaultError;
use vault_core::{NoteSource, Vault, WriteError};

/// Why a repository operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryError {
    /// The note changed on disk since the caller last saw it; the save
    /// was refused before anything was written.
    ExternallyModified,
    /// The note does not exist (or lives under a refused dot-path).
    NotFound { path: String },
    /// Any other failure, with an actionable message.
    Failed { message: String },
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExternallyModified => write!(f, "changed on disk while you edited"),
            Self::NotFound { path } => write!(f, "note not found: {path}"),
            Self::Failed { message } => write!(f, "{message}"),
        }
    }
}

/// Reads and guarded-writes the notes of one open vault.
pub trait NoteRepository {
    /// The full raw text of the note at `path`.
    fn read_note(&self, path: &str) -> Result<String, RepositoryError>;

    /// Write a body-only edit: the adapter reuses the stored
    /// frontmatter bytes verbatim (the caller passes `original` as the
    /// sentinel and `body` as the replacement body).
    fn save_body(&mut self, path: &str, original: &str, body: &str) -> Result<(), RepositoryError>;

    /// Write a full-text edit (frontmatter edited): `content` replaces
    /// the entire file, still guarded by `original`.
    fn save_full(
        &mut self,
        path: &str,
        original: &str,
        content: &str,
    ) -> Result<(), RepositoryError>;

    /// Every note path in the vault, sorted vault-relative. Used by the
    /// Windows file-tree sidebar to enumerate the vault; the shared
    /// builder (`vault_app::tree::build_with_folders`) re-sorts and adds
    /// folder chains, so callers need only list the leaf notes.
    fn note_paths(&self) -> Vec<String>;
}

/// Map a vault-core failure onto the repository error family: the
/// external-change sentinel gets its own variant so the workflow can act
/// on it, everything else degrades to an actionable message.
fn map_error(error: VaultError) -> RepositoryError {
    match error {
        VaultError::Write(WriteError::ExternallyModified) => RepositoryError::ExternallyModified,
        other => RepositoryError::Failed {
            message: other.to_string(),
        },
    }
}

/// The real adapter: a live [`Vault`].
impl NoteRepository for Vault {
    fn read_note(&self, path: &str) -> Result<String, RepositoryError> {
        NoteSource::note_source(self, path).ok_or_else(|| RepositoryError::NotFound {
            path: path.to_string(),
        })
    }

    fn save_body(&mut self, path: &str, original: &str, body: &str) -> Result<(), RepositoryError> {
        self.save_note(path, original, body)
            .map(|_| ())
            .map_err(map_error)
    }

    fn save_full(
        &mut self,
        path: &str,
        original: &str,
        content: &str,
    ) -> Result<(), RepositoryError> {
        self.save_note_full(path, original, content)
            .map(|_| ())
            .map_err(map_error)
    }

    fn note_paths(&self) -> Vec<String> {
        self.notes_sorted()
            .iter()
            .map(|note| note.path.clone())
            .collect()
    }
}

/// A deterministic [`NoteRepository`] fake for tests.
///
/// Backed by an in-memory map, it mirrors the real adapter's guarded
/// semantics: a save is refused with
/// [`RepositoryError::ExternallyModified`] when the stored text no longer
/// matches the `original` sentinel. Saves are recorded so tests can
/// assert exactly what was written.
pub mod testing {
    use std::collections::HashMap;

    use super::{NoteRepository, RepositoryError};

    /// One recorded save attempt that got through the sentinel guard.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SaveRecord {
        /// Vault-relative path of the note.
        pub path: String,
        /// The sentinel the write was guarded with.
        pub original: String,
        /// For a body write, the replacement body; for a full write, the
        /// full content (check `full_text` to tell the two apart).
        pub body: String,
        /// Whether this was a full-text write (`save_full`).
        pub full_text: bool,
    }

    /// An in-memory vault: notes keyed by vault-relative path.
    #[derive(Debug, Default, Clone)]
    pub struct MemoryRepository {
        notes: HashMap<String, String>,
        fail_saves_with: Option<String>,
        saved: Vec<SaveRecord>,
    }

    impl MemoryRepository {
        /// An empty vault.
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// Seed (or replace) the stored text of `path`.
        pub fn insert(&mut self, path: &str, text: &str) {
            self.notes.insert(path.to_string(), text.to_string());
        }

        /// Script every subsequent save to fail with this message
        /// (simulating I/O errors). Passing `None` restores success.
        pub fn fail_saves_with(&mut self, message: &str) {
            self.fail_saves_with = Some(message.to_string());
        }

        /// The stored text of `path`.
        #[must_use]
        pub fn note_text(&self, path: &str) -> Option<String> {
            self.notes.get(path).cloned()
        }

        /// The saves that got through the sentinel guard, in order.
        #[must_use]
        pub fn save_records(&self) -> Vec<SaveRecord> {
            self.saved.clone()
        }

        fn write(
            &mut self,
            path: &str,
            original: &str,
            content: &str,
            full_text: bool,
        ) -> Result<(), RepositoryError> {
            if let Some(message) = &self.fail_saves_with {
                return Err(RepositoryError::Failed {
                    message: message.clone(),
                });
            }
            match self.notes.get(path) {
                // Guarded write: refuse when the disk text is not what
                // the caller last saw (the same sentinel rule the real
                // vault enforces).
                Some(current) if current != original => Err(RepositoryError::ExternallyModified),
                Some(_) => {
                    // A body write reassembles the sentinel's frontmatter
                    // (BOM and raw block byte-for-byte) with the new
                    // body — the same thing the real save_note does.
                    let stored = if full_text {
                        content.to_string()
                    } else {
                        let split = vault_core::split_frontmatter(original);
                        format!("{}{}{}", split.bom, split.raw.unwrap_or_default(), content)
                    };
                    self.notes.insert(path.to_string(), stored);
                    self.saved.push(SaveRecord {
                        path: path.to_string(),
                        original: original.to_string(),
                        body: content.to_string(),
                        full_text,
                    });
                    Ok(())
                }
                None => Err(RepositoryError::NotFound {
                    path: path.to_string(),
                }),
            }
        }
    }

    impl NoteRepository for MemoryRepository {
        fn read_note(&self, path: &str) -> Result<String, RepositoryError> {
            self.notes
                .get(path)
                .cloned()
                .ok_or_else(|| RepositoryError::NotFound {
                    path: path.to_string(),
                })
        }

        fn save_body(
            &mut self,
            path: &str,
            original: &str,
            body: &str,
        ) -> Result<(), RepositoryError> {
            self.write(path, original, body, false)
        }

        fn save_full(
            &mut self,
            path: &str,
            original: &str,
            content: &str,
        ) -> Result<(), RepositoryError> {
            self.write(path, original, content, true)
        }

        fn note_paths(&self) -> Vec<String> {
            let mut paths: Vec<String> = self.notes.keys().cloned().collect();
            paths.sort();
            paths
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_fake_refuses_a_stale_sentinel_like_the_real_vault() {
            let mut repo = MemoryRepository::new();
            repo.insert("a.md", "disk\n");
            assert!(repo.save_body("a.md", "stale\n", "x\n").is_err());
            // Nothing was written and nothing was recorded.
            assert_eq!(repo.note_text("a.md"), Some("disk\n".to_string()));
            assert!(repo.save_records().is_empty());
        }

        #[test]
        fn the_fake_records_what_gets_written() {
            let mut repo = MemoryRepository::new();
            repo.insert("a.md", "old\n");
            repo.save_full("a.md", "old\n", "new full\n").unwrap();
            repo.save_body("a.md", "new full\n", "new body\n").unwrap();
            let records = repo.save_records();
            assert_eq!(records.len(), 2);
            assert!(records[0].full_text);
            assert_eq!(records[0].body, "new full\n");
            assert!(!records[1].full_text);
        }

        #[test]
        fn a_missing_note_reads_and_writes_as_not_found() {
            let mut repo = MemoryRepository::new();
            assert_eq!(
                repo.read_note("gone.md"),
                Err(RepositoryError::NotFound {
                    path: "gone.md".into()
                })
            );
            assert!(matches!(
                repo.save_body("gone.md", "", "x"),
                Err(RepositoryError::NotFound { .. })
            ));
        }
    }
}
