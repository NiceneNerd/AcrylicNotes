//! The save/autosave/conflict workflow, shared by every frontend.
//!
//! Persistence decisions live here, not in duplicated native click
//! handlers (restructure plan §21). Everything is a pure function over a
//! [`NoteRepository`](crate::repository::NoteRepository) plus plain data,
//! so reducer-level tests can run the whole guarded-save story against a
//! deterministic fake before any UI is touched.
//!
//! The three flows:
//!
//! * [`guarded_save`] — the safe write: body edits reuse the stored
//!   frontmatter bytes verbatim, frontmatter edits replace the whole
//!   file, and an external change refuses the write instead of
//!   clobbering it.
//! * [`resolve_conflict`] — the conflict banner's three choices
//!   ([`ConflictDecision`]), applied against the repository.
//! * [`autosave_due`] — the idle-based autosave policy.

use std::time::Duration;

use vault_core::split_frontmatter;

use crate::persistence::ConflictDecision;
use crate::repository::{NoteRepository, RepositoryError};

/// Which write path a save takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavePlan {
    /// Frontmatter untouched: write the body only, reusing the stored
    /// frontmatter bytes verbatim (BOM and formatting survive).
    Body(String),
    /// Frontmatter edited (or added/removed): the full editor text
    /// replaces the file.
    Full(String),
}

/// Decide the write path by comparing the editor text's frontmatter with
/// the disk text's (moved here from the cosmic frontend; the rule is
/// shared behavior, not shell detail).
#[must_use]
pub fn save_plan(editor: &str, disk: &str) -> SavePlan {
    let editor_split = split_frontmatter(editor);
    let disk_split = split_frontmatter(disk);
    if editor_split.bom == disk_split.bom && editor_split.raw == disk_split.raw {
        SavePlan::Body(editor_split.body.to_string())
    } else {
        SavePlan::Full(editor.to_string())
    }
}

/// The outcome of one guarded save attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The write succeeded; the buffer text is now the disk state.
    Saved,
    /// Refused: the note changed on disk since the caller last saw it.
    /// The caller raises the conflict banner; the buffer stays dirty.
    ExternallyModified,
    /// Any other failure, with user-facing text.
    Failed(String),
}

/// Save `editor_text` through the guarded write path, with `base` as the
/// sentinel the edits were made against.
pub fn guarded_save(
    repo: &mut dyn NoteRepository,
    path: &str,
    editor_text: &str,
    base: &str,
) -> SaveOutcome {
    let outcome = match save_plan(editor_text, base) {
        SavePlan::Body(body) => repo.save_body(path, base, &body),
        SavePlan::Full(full) => repo.save_full(path, base, &full),
    };
    match outcome {
        Ok(()) => SaveOutcome::Saved,
        Err(RepositoryError::ExternallyModified) => SaveOutcome::ExternallyModified,
        Err(RepositoryError::NotFound { path }) => {
            SaveOutcome::Failed(format!("save failed: note not found: {path}"))
        }
        Err(RepositoryError::Failed { message }) => {
            SaveOutcome::Failed(format!("save failed: {message}"))
        }
    }
}

/// The result of applying one conflict-banner decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Reload from disk: `text` is the new buffer content (and the new
    /// base). Local edits and undo history are dropped by the caller.
    Reloaded { text: String },
    /// Keep mine: the buffer stays as the user edited it, unsaved.
    Kept,
    /// Save mine anyway: the overwrite succeeded (the fresh disk text
    /// became the sentinel), so the buffer is now clean.
    Saved,
    /// The overwrite was refused again (changed a second time): the
    /// banner should be raised again.
    ExternallyModified,
    /// A failure with user-facing text.
    Failed(String),
}

/// Apply a conflict-banner decision for `path` through the repository.
///
/// `editor_text` is the local (possibly unsaved) buffer content.
pub fn resolve_conflict(
    repo: &mut dyn NoteRepository,
    path: &str,
    editor_text: &str,
    decision: ConflictDecision,
) -> ConflictResolution {
    match decision {
        ConflictDecision::ReloadFromDisk => match repo.read_note(path) {
            Ok(text) => ConflictResolution::Reloaded { text },
            Err(RepositoryError::NotFound { path }) => {
                ConflictResolution::Failed(format!("reload failed: note not found: {path}"))
            }
            Err(RepositoryError::Failed { message }) => {
                ConflictResolution::Failed(format!("reload failed: {message}"))
            }
            Err(RepositoryError::ExternallyModified) => ConflictResolution::Failed(
                "reload failed: changed on disk while you edited".to_string(),
            ),
        },
        ConflictDecision::KeepMine => ConflictResolution::Kept,
        ConflictDecision::SaveMineAnyway => {
            // Deliberate overwrite: a fresh disk snapshot becomes the
            // sentinel, so the user's explicit choice wins over the
            // external change they were shown.
            let disk = match repo.read_note(path) {
                Ok(disk) => disk,
                Err(RepositoryError::NotFound { path }) => {
                    return ConflictResolution::Failed(format!(
                        "overwrite failed: note not found: {path}"
                    ));
                }
                Err(RepositoryError::Failed { message }) => {
                    return ConflictResolution::Failed(format!("overwrite failed: {message}"));
                }
                Err(RepositoryError::ExternallyModified) => {
                    return ConflictResolution::Failed(
                        "overwrite failed: could not read the current note".to_string(),
                    );
                }
            };
            match guarded_save(repo, path, editor_text, &disk) {
                SaveOutcome::Saved => ConflictResolution::Saved,
                SaveOutcome::ExternallyModified => ConflictResolution::ExternallyModified,
                SaveOutcome::Failed(message) => ConflictResolution::Failed(format!(
                    "overwrite failed: {}",
                    message.trim_start_matches("save failed: ")
                )),
            }
        }
    }
}

/// Pure autosave policy: an idle period measured since the last edit is
/// due for a save when it is at least the configured delay. Unit-testable
/// without an `Instant` (moved here from the cosmic frontend).
#[must_use]
pub fn autosave_due(idle_since_last_edit: Option<Duration>, delay_secs: u64) -> bool {
    idle_since_last_edit.is_some_and(|idle| idle >= Duration::from_secs(delay_secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::testing::MemoryRepository;

    fn repo_with(disk: &str) -> MemoryRepository {
        let mut repo = MemoryRepository::new();
        repo.insert("a.md", disk);
        repo
    }

    // --- save_plan ---

    #[test]
    fn body_edits_keep_the_body_path() {
        let disk = "---\ntitle: T\n---\n\nold body\n";
        let editor = "---\ntitle: T\n---\n\nnew body\n";
        match save_plan(editor, disk) {
            SavePlan::Body(body) => assert_eq!(body, "\nnew body\n"),
            SavePlan::Full(_) => panic!("frontmatter untouched; body path expected"),
        }
    }

    #[test]
    fn frontmatter_edits_take_the_full_text_path() {
        let disk = "---\ntitle: T\n---\n\nbody\n";
        let editor = "---\ntitle: Edited\ntags: [x]\n---\n\nbody\n";
        match save_plan(editor, disk) {
            SavePlan::Body(_) => panic!("frontmatter edited; full-text path expected"),
            SavePlan::Full(full) => assert_eq!(full, editor),
        }
    }

    #[test]
    fn adding_or_removing_frontmatter_takes_the_full_text_path() {
        assert!(matches!(
            save_plan("---\na: 1\n---\n\nb\n", "b\n"),
            SavePlan::Full(_)
        ));
        assert!(matches!(
            save_plan("b\n", "---\na: 1\n---\n\nb\n"),
            SavePlan::Full(_)
        ));
    }

    #[test]
    fn bom_differences_take_the_full_text_path() {
        let disk = "---\na: 1\n---\n\nb\n";
        let editor = "\u{feff}---\na: 1\n---\n\nb\n";
        assert!(matches!(save_plan(editor, disk), SavePlan::Full(_)));
    }

    // --- guarded_save ---

    #[test]
    fn a_clean_save_writes_through_the_adapter_and_updates_the_disk() {
        let mut repo = repo_with("---\ntitle: T\n---\n\nold\n");
        let outcome = guarded_save(
            &mut repo,
            "a.md",
            "---\ntitle: T\n---\n\nnew\n",
            "---\ntitle: T\n---\n\nold\n",
        );
        assert_eq!(outcome, SaveOutcome::Saved);
        assert_eq!(
            repo.note_text("a.md"),
            Some("---\ntitle: T\n---\n\nnew\n".to_string())
        );
        // The body path was taken: the record shows a body-only write.
        let record = repo.save_records().pop().expect("one save recorded");
        assert!(!record.full_text, "body edit should take the body path");
        assert_eq!(record.body, "\nnew\n");
        assert_eq!(record.original, "---\ntitle: T\n---\n\nold\n");
    }

    #[test]
    fn an_external_change_refuses_the_save_without_touching_the_disk() {
        // The disk moved on since the caller last saw it.
        let mut repo = repo_with("---\ntitle: T\n---\n\nexternally changed\n");
        let outcome = guarded_save(
            &mut repo,
            "a.md",
            "---\ntitle: T\n---\n\nmine\n",
            "---\ntitle: T\n---\n\nolder\n",
        );
        assert_eq!(outcome, SaveOutcome::ExternallyModified);
        assert_eq!(
            repo.note_text("a.md"),
            Some("---\ntitle: T\n---\n\nexternally changed\n".to_string()),
            "the external text must not be clobbered"
        );
        assert!(repo.save_records().is_empty());
    }

    #[test]
    fn a_scripted_failure_surfaces_its_message() {
        let mut repo = MemoryRepository::new();
        repo.insert("a.md", "b\n");
        repo.fail_saves_with("disk full");
        let outcome = guarded_save(&mut repo, "a.md", "new\n", "b\n");
        assert_eq!(
            outcome,
            SaveOutcome::Failed("save failed: disk full".into())
        );
    }

    // --- resolve_conflict ---

    #[test]
    fn reload_from_disk_replaces_the_buffer_with_the_disk_text() {
        let mut repo = repo_with("disk truth\n");
        let resolution = resolve_conflict(
            &mut repo,
            "a.md",
            "my unsaved edits\n",
            ConflictDecision::ReloadFromDisk,
        );
        assert_eq!(
            resolution,
            ConflictResolution::Reloaded {
                text: "disk truth\n".to_string()
            }
        );
        assert!(repo.save_records().is_empty(), "a reload never writes");
    }

    #[test]
    fn keep_mine_touches_nothing() {
        let mut repo = repo_with("disk truth\n");
        let resolution = resolve_conflict(
            &mut repo,
            "a.md",
            "my unsaved edits\n",
            ConflictDecision::KeepMine,
        );
        assert_eq!(resolution, ConflictResolution::Kept);
        assert!(repo.save_records().is_empty());
        assert_eq!(repo.note_text("a.md"), Some("disk truth\n".to_string()));
    }

    #[test]
    fn save_mine_anyway_overwrites_with_a_fresh_sentinel() {
        // Disk moved on since the base; the user's explicit choice wins.
        let mut repo = repo_with("external change\n");
        let resolution = resolve_conflict(
            &mut repo,
            "a.md",
            "my edits\n",
            ConflictDecision::SaveMineAnyway,
        );
        assert_eq!(resolution, ConflictResolution::Saved);
        assert_eq!(repo.note_text("a.md"), Some("my edits\n".to_string()));
        // The write was guarded by the *fresh* disk text, not the stale base.
        let record = repo.save_records().pop().expect("one save recorded");
        assert_eq!(record.original, "external change\n");
    }

    #[test]
    fn save_mine_anyway_reports_a_second_refusal() {
        // Simulate the note changing again between the read and the
        // write: the fake fails the save itself.
        let mut repo = MemoryRepository::new();
        repo.insert("a.md", "external\n");
        repo.fail_saves_with("changed on disk while you edited");
        let resolution = resolve_conflict(
            &mut repo,
            "a.md",
            "my edits\n",
            ConflictDecision::SaveMineAnyway,
        );
        assert!(matches!(
            resolution,
            ConflictResolution::Failed(ref message)
                if message.contains("changed on disk")
        ));
    }

    #[test]
    fn a_missing_note_fails_each_decision_with_an_actionable_message() {
        let mut repo = MemoryRepository::new();
        for decision in ConflictDecision::ALL {
            let resolution = resolve_conflict(&mut repo, "gone.md", "x\n", decision);
            match decision {
                ConflictDecision::ReloadFromDisk => {
                    assert!(matches!(
                        resolution,
                        ConflictResolution::Failed(ref message) if message.contains("gone.md")
                    ));
                }
                ConflictDecision::KeepMine => assert_eq!(resolution, ConflictResolution::Kept),
                ConflictDecision::SaveMineAnyway => {
                    assert!(matches!(
                        resolution,
                        ConflictResolution::Failed(ref message) if message.contains("gone.md")
                    ));
                }
            }
        }
    }

    // --- close gate (workflow-level orchestration) ---

    #[test]
    fn a_close_saves_first_and_a_refusal_keeps_the_tab() {
        // The close workflow: save dirty edits; only a clean outcome may
        // close. Simulated with the workflow's own outcome values.
        let mut repo = repo_with("disk truth\n");

        // Dirty tab, clean save → the close proceeds.
        let outcome = guarded_save(&mut repo, "a.md", "my edits\n", "disk truth\n");
        assert_eq!(outcome, SaveOutcome::Saved);

        // Dirty tab against a moved-on disk → the close is blocked.
        let outcome = guarded_save(&mut repo, "a.md", "second edit\n", "stale base\n");
        assert_eq!(outcome, SaveOutcome::ExternallyModified);
    }

    // --- autosave policy ---

    #[test]
    fn autosave_fires_after_the_configured_idle_delay() {
        let delay = 3;
        assert!(!autosave_due(None, delay), "no edit yet: never fire");
        assert!(autosave_due(Some(Duration::from_secs(3)), delay));
        assert!(autosave_due(Some(Duration::from_secs(10)), delay));
        assert!(!autosave_due(Some(Duration::from_secs(2)), delay));
    }
}
