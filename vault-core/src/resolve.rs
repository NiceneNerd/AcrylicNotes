//! Link resolution over vault-relative, '/'-separated paths.
//! Sketch of Obsidian's shortest-unique-name behavior.
//! ASSUMPTIONS to confirm against real Obsidian with a test vault:
//!   * ambiguous basename tie-break (proximity to the linking note)
//!   * aliases do NOT resolve links (they feed autocomplete / unlinked mentions)
//!   * non-`.md` targets require their extension (`[[cat]]` does not find
//!     `cat.png` — unconfirmed)
//!   * relative targets do NOT fall back to basename matching (user-observed
//!     in real Obsidian: stale `../` links show as unresolved)

use std::cmp::Reverse;
use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

/// In-memory index of vault files for link resolution.
///
/// Normalized keys (NFC + case-folded, `.md` stripped) are computed once per
/// file set, so lookups are map hits instead of a full rescan with per-file
/// normalization on every call.
#[derive(Debug, Clone)]
pub struct VaultIndex {
    files: Vec<String>,
    /// Normalized full-path key for each file (same order as `files`).
    keys: Vec<String>,
    /// key -> indices of files with that exact full-path key.
    by_key: HashMap<String, Vec<usize>>,
    /// normalized basename -> indices of files with that basename.
    by_stem: HashMap<String, Vec<usize>>,
}

impl VaultIndex {
    /// Create an index from an iterator of vault-relative paths (using '/' separators).
    pub fn new(files: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let files: Vec<String> = files.into_iter().map(Into::into).collect();
        let mut keys = Vec::with_capacity(files.len());
        let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_stem: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            let key = key(f);
            by_key.entry(key.clone()).or_default().push(i);
            let base = key.rsplit_once('/').map_or(key.as_str(), |(_, b)| b);
            by_stem.entry(base.to_string()).or_default().push(i);
            keys.push(key);
        }
        Self {
            files,
            keys,
            by_key,
            by_stem,
        }
    }

    /// All files a wikilink target could mean, best first.
    pub fn candidates(&self, target: &str, from: &str) -> Vec<&str> {
        let t = target.trim().trim_start_matches('/');

        // Relative targets (`../x`, `./x`) resolve against the source note's
        // folder and require an exact match (no basename fallback).
        if t.split('/').any(|c| c == "." || c == "..") {
            return self.relative_candidates(t, from);
        }

        let t = key(t);
        if t.is_empty() {
            // `[[#Heading]]`: the note itself.
            return self
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| *f == from)
                .map(|(i, _)| i)
                .filter_map(|i| self.files.get(i).map(|s| s.as_str()))
                .collect();
        }

        // Exact full-path hits ("Notes/Rust", "rust" if a root file).
        let mut hit_idx: Vec<usize> = self.by_key.get(&t).cloned().unwrap_or_default();
        // Suffix hits: basename matches, filtered to paths ending with the
        // target's directory part (if any).
        let (dir_part, base) = t.rsplit_once('/').unwrap_or(("", &t));
        for &i in self.by_stem.get(base).unwrap_or(&vec![]) {
            if dir_part.is_empty() || ends_with_path(&self.keys[i], &t) {
                hit_idx.push(i);
            }
        }
        hit_idx.sort_unstable();
        hit_idx.dedup();

        let from_dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let mut hits: Vec<&str> = hit_idx.iter().map(|&i| self.files[i].as_str()).collect();
        hits.sort_by_key(|f| {
            let exact = key(f) != t;
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

    /// Resolve a path-relative target (`Note.md`, `../media/x.png`) against
    /// the source note's folder. Used for standard Markdown links, which are
    /// always folder-relative — never shortest-unique-name.
    pub fn resolve_relative(&self, target: &str, from: &str) -> Option<&str> {
        self.relative_candidates(target.trim(), from)
            .into_iter()
            .next()
    }

    fn relative_candidates(&self, target: &str, from: &str) -> Vec<&str> {
        let from_dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let joined = normalize_rel(&format!("{from_dir}/{target}"));
        // Wikilinks to notes omit `.md`; Markdown links usually carry one.
        for cand in [joined.clone(), format!("{joined}.md")] {
            if let Some(idxs) = self.by_key.get(&key(&cand)) {
                return idxs.iter().map(|&i| self.files[i].as_str()).collect();
            }
        }
        Vec::new()
    }

    /// Get all indexed files.
    pub fn all_files(&self) -> &[String] {
        &self.files
    }
}

/// Matching key: strip `.md`, Unicode NFC-normalize, case-fold.
/// NFC handles macOS filenames stored NFD; lowercase handles
/// case-insensitive matching.
fn key(p: &str) -> String {
    p.strip_suffix(".md")
        .unwrap_or(p)
        .nfc()
        .collect::<String>()
        .to_lowercase()
}

/// Component-wise path suffix test over normalized keys:
/// "projects/obsidian/glass" ends with "obsidian/glass".
fn ends_with_path(path_key: &str, suffix: &str) -> bool {
    let p: Vec<&str> = path_key.split('/').collect();
    let s: Vec<&str> = suffix.split('/').collect();
    s.len() <= p.len() && p[p.len() - s.len()..] == s[..]
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
        // a suffix match must respect component boundaries
        assert_eq!(i.resolve("sidian/Glass", "x.md"), None);
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
        // a relative path that matches nothing stays unresolved (user-observed
        // in real Obsidian: no basename fallback for relative links)
        assert_eq!(
            i.resolve("../missing", "Creans/Teretz/posts/nations.md"),
            None
        );
    }

    #[test]
    fn markdown_links_are_folder_relative_not_name_matched() {
        let i = VaultIndex::new(["Sub/Note.md", "Other/Note.md", "Top.md"]);
        let from = "Sub/src.md";
        // plain relative target resolves inside the note's own folder
        assert_eq!(i.resolve_relative("Note.md", from), Some("Sub/Note.md"));
        assert_eq!(i.resolve_relative("./Note.md", from), Some("Sub/Note.md"));
        assert_eq!(
            i.resolve_relative("../Other/Note.md", from),
            Some("Other/Note.md")
        );
        assert_eq!(i.resolve_relative("../Top.md", from), Some("Top.md"));
        // NOT a vault-wide name match: from a sibling folder it must miss
        assert_eq!(
            i.resolve_relative("Note.md", "Other/src.md"),
            Some("Other/Note.md")
        );
        // and a path that does not exist relative to the note is unresolved,
        // even though the name exists elsewhere in the vault
        assert_eq!(i.resolve_relative("Top.md", from), None);
        assert_eq!(i.resolve_relative("Missing.md", from), None);
    }
}
