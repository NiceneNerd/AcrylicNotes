//! Safe atomic file writes with external-change detection.
//!
//! - Preserves frontmatter bytes verbatim.
//! - Preserves line endings (CRLF vs LF).
//! - Writes to temp file in same directory, then renames atomically.
//! - Detects external modification before save (mtime/size/hash).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::frontmatter::{split_frontmatter, SplitFrontmatter};
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

/// Write a Markdown file atomically, preserving frontmatter.
///
/// `original` is the file content as it was when last read (used for
/// external-change detection). `new_body` is the new body content (without
/// frontmatter). The original frontmatter block is kept verbatim.
pub fn atomic_write(
    path: &Path,
    original: &str,
    new_body: &str,
) -> Result<WriteResult, WriteError> {
    let split = split_frontmatter(original);

    // Detect external modification.
    if let Ok(meta) = fs::metadata(path) {
        let current = fs::read_to_string(path)?;
        if current != original {
            return Err(WriteError::ExternallyModified);
        }
    }

    // Reconstruct content: original frontmatter (if any) + new body.
    let new_content = match split.raw {
        Some(fm) => format!("{fm}{new_body}"),
        None => new_body.to_string(),
    };

    // Write to temp file in same directory.
    let temp_path = path.with_extension("md.tmp");
    {
        let mut f = fs::File::create(&temp_path)?;
        f.write_all(new_content.as_bytes())?;
        f.flush()?;
    }

    // Atomic rename.
    fs::rename(&temp_path, path)?;

    let meta = fs::metadata(path)?;
    Ok(WriteResult {
        modified: meta.modified()?,
        size: meta.len(),
        hash: sha256(&new_content),
    })
}

/// Write a file atomically without frontmatter handling (for attachments, etc.).
pub fn atomic_write_raw(path: &Path, content: &[u8]) -> Result<WriteResult, WriteError> {
    let temp_path = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&temp_path)?;
        f.write_all(content)?;
        f.flush()?;
    }
    fs::rename(&temp_path, path)?;
    let meta = fs::metadata(path)?;
    Ok(WriteResult {
        modified: meta.modified()?,
        size: meta.len(),
        hash: sha256_str(&String::from_utf8_lossy(content)),
    })
}

fn sha256(s: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher.finalize().into()
}

fn sha256_str(s: &str) -> [u8; 32] {
    sha256(s)
}

/// Errors from write operations.
#[derive(Debug, Error)]
pub enum WriteError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("File was modified externally since last read")]
    ExternallyModified,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_preserves_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("note.md");
        let original = "---\ntitle: Test\ntags: [a, b]\n---\n\nbody";
        fs::write(&file, original).unwrap();

        atomic_write(&file, original, "\nnew body").unwrap();

        let result = fs::read_to_string(&file).unwrap();
        assert!(result.starts_with("---\ntitle: Test\ntags: [a, b]\n---\n"));
        assert!(result.ends_with("new body"));
        // Frontmatter bytes are identical (up to the closing delimiter).
        let fm_end = original.find("\n---\n").unwrap() + 5;
        assert_eq!(&result[..fm_end], &original[..fm_end]);
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
}
