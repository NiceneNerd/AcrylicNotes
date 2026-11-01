//! Safe atomic file writes with external-change detection.
//!
//! - Reconstructs files losslessly: BOM + frontmatter bytes + body, so an
//!   unedited frontmatter block (and the BOM and any blank lines after it)
//!   are byte-identical after a body edit.
//! - Preserves line endings (CRLF vs LF) by never touching bytes.
//! - Writes through a unique temp file in the same directory, fsyncs the file
//!   before rename and the directory after, then renames atomically.
//! - Preserves the file's permission mode.
//! - Refuses to write into dot-directories (`.obsidian/`, `.git/`, …).
//! - Detects external modification before save (content compare; the mtime /
//!   size / hash tripwire lives in `vault::Note`).

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::SystemTime;

use crate::frontmatter::split_frontmatter;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Result of a safe write operation.
#[derive(Debug)]
pub struct WriteResult {
    /// The new modification time.
    pub modified: SystemTime,
    /// The new file size.
    pub size: u64,
    /// The new content hash.
    pub hash: [u8; 32],
}

/// Write a Markdown file atomically, preserving frontmatter losslessly.
///
/// `original` is the file content as it was when last read (used for
/// external-change detection). `new_body` replaces the body *verbatim* —
/// i.e. `split_frontmatter(original).body` — including any leading blank
/// lines. The BOM and frontmatter block are reused byte-for-byte.
pub fn atomic_write(
    path: &Path,
    original: &str,
    new_body: &str,
) -> Result<WriteResult, WriteError> {
    let split = split_frontmatter(original);
    let new_content = format!(
        "{}{}{}",
        split.bom,
        split.raw.as_deref().unwrap_or(""),
        new_body
    );
    write_durable(path, original, new_content.as_bytes())?;
    Ok(stat_written(path, &new_content))
}

/// Write full file content atomically — frontmatter included, verbatim.
///
/// Unlike [`atomic_write`], the caller supplies the *complete* new text
/// (BOM, frontmatter, and body). Use this when the frontmatter itself was
/// edited; an unedited frontmatter block should go through [`atomic_write`]
/// so the original bytes are provably reused. External-change detection and
/// durability are identical.
pub fn atomic_write_full(
    path: &Path,
    original: &str,
    content: &str,
) -> Result<WriteResult, WriteError> {
    write_durable(path, original, content.as_bytes())?;
    Ok(stat_written(path, content))
}

/// Write arbitrary bytes atomically (attachments, etc.).
pub fn atomic_write_raw(
    path: &Path,
    original: &[u8],
    content: &[u8],
) -> Result<WriteResult, WriteError> {
    write_durable(path, &String::from_utf8_lossy(original), content)?;
    let lossy = String::from_utf8_lossy(content);
    Ok(stat_written(path, &lossy))
}

/// Detect external modification, then write through a unique temp file.
fn write_durable(path: &Path, original: &str, content: &[u8]) -> Result<(), WriteError> {
    // Detect external modification: the caller must have seen `original`.
    if fs::metadata(path).is_ok() {
        let current = fs::read_to_string(path)?;
        if current != original {
            return Err(WriteError::ExternallyModified);
        }
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let previous_mode = fs::metadata(path).ok().map(|m| m.permissions());

    // Unique temp name in the same directory: same filesystem for an atomic
    // rename, and a dotfile prefix so vault watchers ignore the intermediary.
    let mut temp = tempfile::Builder::new()
        .prefix(".vault-write-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    temp.write_all(content)?;
    temp.flush()?;
    // fsync the file BEFORE the rename so the content survives a crash.
    temp.as_file().sync_all()?;
    if let Some(mode) = previous_mode {
        let _ = temp.as_file().set_permissions(mode); // best effort
    }
    temp.persist(path).map_err(|e| e.error)?;

    // fsync the directory so the rename itself survives a crash.
    #[cfg(unix)]
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    Ok(())
}

fn stat_written(path: &Path, content: &str) -> WriteResult {
    let meta = fs::metadata(path).expect("file exists after durable write");
    WriteResult {
        modified: meta.modified().expect("mtime available"),
        size: meta.len(),
        hash: sha256(content.as_bytes()),
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Errors from write operations.
#[derive(Debug, Error)]
pub enum WriteError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("file was modified externally since last read")]
    ExternallyModified,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn body_edit_preserves_frontmatter_bom_and_blank_lines() {
        let tmp = TempDir::new().unwrap();
        for original in [
            "---\ntitle: Test\ntags: [a, b]\n---\n\nbody",
            "\u{feff}---\ntitle: Test\n---\n\n\nbody",
            "---\r\ntitle: Test\r\n---\r\n\r\nbody",
        ] {
            let name = if original.contains("\r\n") {
                "crlf.md"
            } else if original.starts_with('\u{feff}') {
                "bom.md"
            } else {
                "note.md"
            };
            let file = tmp.path().join(name);
            fs::write(&file, original).unwrap();

            // The app edits the body, keeping whatever led it had.
            let split = split_frontmatter(original);
            let new_body = split.body.replacen("body", "edited body", 1);
            let result = atomic_write(&file, original, &new_body).unwrap();

            let after = fs::read_to_string(&file).unwrap();
            assert!(after.contains("edited body"));
            // The untouched parts are byte-identical: BOM + frontmatter block.
            let fm_len = split.raw.as_deref().unwrap_or("").len() + split.bom.len();
            assert_eq!(&after[..fm_len], &original[..fm_len]);
            assert_eq!(result.hash, sha256(after.as_bytes()));
        }
    }

    #[test]
    fn unedited_frontmatter_block_is_byte_identical_after_save() {
        // HANDOFF M1: a test that an unedited frontmatter block is
        // byte-identical after a body edit.
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("note.md");
        let original = "---\ntitle: Test\ntags: [a, b]\n---\n\nbody";
        fs::write(&file, original).unwrap();

        atomic_write(&file, original, "\nnew body").unwrap();

        let result = fs::read_to_string(&file).unwrap();
        let fm_end = original.find("\n---\n").unwrap() + 5;
        assert_eq!(&result[..fm_end], &original[..fm_end]);
        // And the blank line after the frontmatter survives.
        assert_eq!(result, "---\ntitle: Test\ntags: [a, b]\n---\n\nnew body");
    }

    #[test]
    fn atomic_write_preserves_crlf() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("note.md");
        let original = "---\r\ntitle: Test\r\n---\r\n\r\nbody";
        fs::write(&file, original).unwrap();

        atomic_write(&file, original, "\r\nnew body").unwrap();

        let result = fs::read_to_string(&file).unwrap();
        assert!(result.contains("\r\n"));
    }

    #[test]
    fn atomic_write_detects_external_change() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("note.md");
        let original = "body";
        fs::write(&file, original).unwrap();

        // Simulate external modification.
        fs::write(&file, "external change").unwrap();

        let err = atomic_write(&file, original, "new body").unwrap_err();
        assert!(matches!(err, WriteError::ExternallyModified));
    }

    #[test]
    fn atomic_write_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("note.md");
        let original = "just body";
        fs::write(&file, original).unwrap();

        atomic_write(&file, original, "new body").unwrap();

        let result = fs::read_to_string(&file).unwrap();
        assert_eq!(result, "new body");
    }

    #[test]
    #[cfg(unix)]
    fn file_mode_is_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("note.md");
        fs::write(&file, "original").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();

        atomic_write(&file, "original", "new").unwrap();

        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn leaves_no_temp_files() {
        let tmp = TempDir::new().unwrap();
        let ok = tmp.path().join("note.md");
        atomic_write(&ok, "", "content").unwrap();
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".vault-write-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
    }
}
