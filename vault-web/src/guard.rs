//! Path confinement: the single choke point every user-controlled path
//! passes through before the server touches the filesystem (M5 spec).
//!
//! Three independent layers, so no single mistake exposes files:
//! 1. component check — `..` and dotfile components are rejected *before*
//!    any filesystem call (catches `..`, percent-encoded `..`, `..%2F`);
//! 2. canonicalize — resolves symlinks;
//! 3. prefix check — the canonical path must stay inside the canonical
//!    vault root (what actually stops symlink escapes).

use std::path::{Path, PathBuf};

/// Resolve a vault-relative request path to a real file inside `root`.
///
/// `None` for anything that is not a plain file inside the vault: `..`
/// components, dotfile components (`.obsidian/`, `.git`, …), missing files,
/// directories, and symlinks that escape the vault.
pub fn safe_resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return None;
    }
    if rel
        .split('/')
        .any(|component| component.starts_with('.') || component == "..")
    {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let canonical = root.join(rel).canonicalize().ok()?;
    if canonical.starts_with(&root) && canonical.is_file() {
        Some(canonical)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("vault");
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(root.join("Note.md"), "visible\n").unwrap();
        fs::write(root.join("Notes/Nested.md"), "nested\n").unwrap();
        fs::write(root.join(".obsidian/app.json"), "{}").unwrap();
        fs::write(root.join(".hidden.md"), "hidden\n").unwrap();
        // A sibling directory OUTSIDE the vault, with a secret in it.
        fs::write(tmp.path().join("secret.md"), "secret\n").unwrap();
        (tmp, root)
    }

    #[test]
    fn resolves_notes_and_attachments_inside_the_vault() {
        let (_tmp, root) = setup();
        assert_eq!(
            safe_resolve(&root, "Note.md"),
            Some(root.join("Note.md").canonicalize().unwrap())
        );
        assert_eq!(
            safe_resolve(&root, "/Notes/Nested.md"),
            Some(root.join("Notes/Nested.md").canonicalize().unwrap())
        );
        assert!(safe_resolve(&root, "missing.md").is_none());
        // A directory is not a file.
        assert!(safe_resolve(&root, "Notes").is_none());
        assert!(safe_resolve(&root, "").is_none());
    }

    #[test]
    fn rejects_dotdot_in_every_shape() {
        let (tmp, root) = setup();
        let secret = tmp.path().join("secret.md");
        assert!(secret.exists());

        // Plain.
        assert!(safe_resolve(&root, "../secret.md").is_none());
        // Nested traversal that would land back inside is still refused:
        // `..` is denied as a component, not merely checked by destination.
        assert!(safe_resolve(&root, "Notes/../Note.md").is_none());
        // Deep escape.
        assert!(safe_resolve(&root, "a/b/../../../../secret.md").is_none());
    }

    #[test]
    fn rejects_dotfiles_including_obsidian() {
        let (_tmp, root) = setup();
        assert!(safe_resolve(&root, ".obsidian/app.json").is_none());
        assert!(safe_resolve(&root, "Notes/.obsidian/config").is_none());
        assert!(safe_resolve(&root, ".hidden.md").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn rejects_symlinks_out_of_the_vault() {
        let (tmp, root) = setup();
        // A symlinked note pointing at the outside secret.
        std::os::unix::fs::symlink(tmp.path().join("secret.md"), root.join("leak.md")).unwrap();
        assert!(safe_resolve(&root, "leak.md").is_none());

        // A symlinked directory containing a note.
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("note.md"), "outside\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("mirror")).unwrap();
        assert!(safe_resolve(&root, "mirror/note.md").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn allows_symlinks_that_stay_inside() {
        let (_tmp, root) = setup();
        std::os::unix::fs::symlink(root.join("Note.md"), root.join("alias.md")).unwrap();
        assert!(safe_resolve(&root, "alias.md").is_some());
    }
}
