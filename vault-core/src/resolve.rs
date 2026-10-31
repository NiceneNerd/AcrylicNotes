//! Link resolution over vault-relative, '/'-separated paths.
//! Sketch of Obsidian's shortest-unique-name behavior.
//! ASSUMPTIONS to confirm against real Obsidian with a test vault:
//!   * ambiguous basename tie-break (proximity to the linking note)
//!   * aliases do NOT resolve links (they feed autocomplete / unlinked mentions)
//!   * non-`.md` targets require their extension (`[[cat]]` does not find
//!     `cat.png` — unconfirmed)

use std::cmp::Reverse;

use unicode_normalization::UnicodeNormalization;

/// In-memory index of vault files for link resolution.
#[derive(Debug, Clone)]
pub struct VaultIndex {
    files: Vec<String>,
}

impl VaultIndex {
    /// Create an index from an iterator of vault-relative paths (using '/' separators).
    pub fn new(files: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            files: files.into_iter().map(Into::into).collect(),
        }
    }

    /// Matching key: strip `.md`, Unicode NFC-normalize, case-fold.
    /// NFC handles macOS filenames stored NFD; lowercase handles
    /// case-insensitive matching.
    fn stem(p: &str) -> String {
        p.strip_suffix(".md")
            .unwrap_or(p)
            .nfc()
            .collect::<String>()
            .to_lowercase()
    }

    /// All files a target could mean, best first.
    pub fn candidates(&self, target: &str, from: &str) -> Vec<&str> {
        let t = target.trim().trim_start_matches('/');

        // Relative targets (`../x`, `./x`): resolve against the source note's
        // folder, then require an exact (NFC, case-folded) match. Wikilinks to
        // notes omit `.md`, so try both with and without the extension.
        // ASSUMPTION (confirm in M3): no basename fallback when the relative
        // path does not resolve; `..` beyond the root clamps at the root.
        if t.split('/').any(|c| c == "." || c == "..") {
            let from_dir = from.rsplit_once('/').map_or("", |(d, _)| d);
            let joined = normalize_rel(&format!("{from_dir}/{t}"));
            for cand in [joined.clone(), format!("{joined}.md")] {
                let key = Self::stem(&cand);
                let hits: Vec<&str> = self
                    .files
                    .iter()
                    .filter(|f| Self::stem(f) == key)
                    .map(|s| s.as_str())
                    .collect();
                if !hits.is_empty() {
                    return hits;
                }
            }
            return Vec::new();
        }

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

/// Lexically normalize a '/'-separated path: drop empty and `.` components,
/// resolve `..` against the previous component (clamped at the root).
fn normalize_rel(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
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

    #[test]
    fn unicode_nfc_and_casefold_matching() {
        // A filename stored NFD (as macOS may present it) matches an NFC query.
        let nfd_name = "Cafe\u{0301}.md"; // Café decomposed
        let i = VaultIndex::new([nfd_name]);
        assert_eq!(i.resolve("Caf\u{e9}", "x.md"), Some(nfd_name)); // composed query
        assert_eq!(i.resolve("caf\u{e9}", "x.md"), Some(nfd_name)); // case-folded
    }

    #[test]
    fn relative_targets_resolve_against_source_folder() {
        // Forms observed in the user's real vault (192 `../` wikilinks).
        let i = VaultIndex::new([
            "Creans/Teretz/posts/nations.md",
            "Creans/Teretz/media/meim.png",
            "Creans/Substack/Like Father, Like Son.md",
            "Journal/2026/10/d.md",
        ]);
        // attachment with extension, one level up
        assert_eq!(
            i.resolve("../media/meim.png", "Creans/Teretz/posts/nations.md"),
            Some("Creans/Teretz/media/meim.png")
        );
        // note without extension, several levels up
        assert_eq!(
            i.resolve(
                "../../../Creans/Substack/Like Father, Like Son",
                "Journal/2026/10/d.md"
            ),
            Some("Creans/Substack/Like Father, Like Son.md")
        );
        // `..` beyond the root clamps at the root (assumption)
        assert_eq!(
            i.resolve("../../nope/../../Journal/2026/10/d", "x.md"),
            Some("Journal/2026/10/d.md")
        );
        // a relative path that matches nothing stays unresolved
        assert_eq!(
            i.resolve("../missing", "Creans/Teretz/posts/nations.md"),
            None
        );
    }
}
