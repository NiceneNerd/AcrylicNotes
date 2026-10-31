//! Link resolution over vault-relative, '/'-separated paths.
//! Sketch of Obsidian's shortest-unique-name behavior.
//! ASSUMPTIONS to confirm against real Obsidian with a test vault:
//!   * ambiguous basename tie-break (proximity to the linking note)
//!   * aliases do NOT resolve links (they feed autocomplete / unlinked mentions)

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;

/// In-memory index of vault files for link resolution.
#[derive(Debug, Clone)]
pub struct VaultIndex {
    files: Vec<String>,
    // Maps lowercased stem -> list of indices into `files`
    by_stem: HashMap<String, Vec<usize>>,
}

impl VaultIndex {
    /// Create an index from an iterator of vault-relative paths (using '/' separators).
    pub fn new(files: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let files: Vec<String> = files.into_iter().map(Into::into).collect();
        let mut by_stem: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, f) in files.iter().enumerate() {
            let stem = Self::stem(f);
            by_stem.entry(stem).or_default().push(idx);
        }
        Self { files, by_stem }
    }

    fn stem(p: &str) -> String {
        p.strip_suffix(".md").unwrap_or(p).to_lowercase()
    }

    /// All files a target could mean, best first.
    pub fn candidates(&self, target: &str, from: &str) -> Vec<&str> {
        let t = target.trim().trim_start_matches('/');
        let t = Self::stem(t);
        if t.is_empty() {
            return self
                .files
                .iter()
                .filter(|f| *f == from)
                .map(|s| s.as_str())
                .collect();
        }
        let suffix = format!("/{t}");
        let mut hits: Vec<&str> = self
            .files
            .iter()
            .filter(|f| {
                let s = Self::stem(f);
                s == t || s.ends_with(&suffix)
            })
            .map(|s| s.as_str())
            .collect();
        let from_dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        hits.sort_by_key(|f| {
            let exact = Self::stem(f) != t;
            let shared = common_dirs(f, from_dir);
            (
                exact,
                Reverse(shared),
                f.matches('/').count(),
                f.to_string(),
            )
        });
        hits
    }

    pub fn resolve(&self, target: &str, from: &str) -> Option<&str> {
        self.candidates(target, from).into_iter().next()
    }

    /// Get all indexed files.
    pub fn all_files(&self) -> &[String] {
        &self.files
    }
}

fn common_dirs(file: &str, dir: &str) -> usize {
    let fd = file.rsplit_once('/').map_or("", |(d, _)| d);
    fd.split('/')
        .zip(dir.split('/'))
        .take_while(|(a, b)| a == b && !a.is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> VaultIndex {
        VaultIndex::new([
            "Journal/2026-10-31.md",
            "Notes/Rust.md",
            "Notes/Glass.md",
            "Archive/Glass.md",
            "Projects/Obsidian/Glass.md",
            "assets/cat.png",
        ])
    }

    #[test]
    fn unique_basename_any_case_optional_ext() {
        let i = idx();
        assert_eq!(
            i.resolve("rust", "Journal/2026-10-31.md"),
            Some("Notes/Rust.md")
        );
        assert_eq!(i.resolve("Rust.md", "x.md"), Some("Notes/Rust.md"));
        assert_eq!(i.resolve("cat.png", "x.md"), Some("assets/cat.png"));
    }

    #[test]
    fn path_qualified_disambiguates() {
        let i = idx();
        assert_eq!(
            i.resolve("Archive/Glass", "Notes/Rust.md"),
            Some("Archive/Glass.md")
        );
        assert_eq!(
            i.resolve("Obsidian/Glass", "x.md"),
            Some("Projects/Obsidian/Glass.md")
        );
    }

    #[test]
    fn ambiguous_prefers_nearest_then_shallowest() {
        let i = idx();
        assert_eq!(
            i.resolve("Glass", "Archive/old.md"),
            Some("Archive/Glass.md")
        );
        assert_eq!(i.resolve("Glass", "Notes/Rust.md"), Some("Notes/Glass.md"));
        assert_eq!(i.candidates("Glass", "Journal/x.md").len(), 3);
    }

    #[test]
    fn missing_and_same_note() {
        let i = idx();
        assert_eq!(i.resolve("Nope", "x.md"), None);
        assert_eq!(i.resolve("", "Notes/Rust.md"), Some("Notes/Rust.md"));
    }
}
