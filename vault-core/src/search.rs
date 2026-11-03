//! Vault-wide content search via ripgrep's own crates (the `grep` family),
//! used as a library — M4 decision 4 is *literal ripgrep*, not a re-rolled
//! regex walk, and no content index. Shared by the GUI and the M5 web
//! server.
//!
//! Scope: `.md` notes only, hidden entries skipped (`.obsidian/`),
//! gitignore files ignored so the search sees exactly what vault-core
//! indexes. Smart-case like ripgrep: a lowercase query matches
//! case-insensitively, any uppercase makes it case-sensitive. A query that is
//! not a valid regex falls back to a literal (escaped) match.

use std::path::Path;

use grep::regex::RegexMatcherBuilder;
use grep::searcher::sinks::UTF8;
use grep::searcher::{BinaryDetection, SearcherBuilder};

/// One match: the note (vault-relative path), its 1-based line number and the
/// matched line, trimmed of trailing whitespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub line_no: u64,
    pub line: String,
}

/// Search every note under `root` for `query`. Returns at most `max_hits`
/// matches, in file walk order (stable, cheap, deterministic).
pub fn search(root: &Path, query: &str, max_hits: usize) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let matcher = match RegexMatcherBuilder::new().case_smart(true).build(query) {
        Ok(matcher) => matcher,
        // Not a valid regex: fall back to a literal (escaped) match.
        Err(_) => RegexMatcherBuilder::new()
            .case_smart(true)
            .build(&regex::escape(query))
            .map_err(|e| format!("bad query: {e}"))?,
    };

    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        // Match vault-core's scan: ignore files don't hide notes from us.
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .build();

    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .build();

    let mut hits = Vec::new();
    for entry in walker {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        let is_note = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        if !is_note {
            continue;
        }
        if hits.len() >= max_hits {
            break;
        }

        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let mut sink = UTF8(|line_no, line| {
            if hits.len() < max_hits {
                hits.push(SearchHit {
                    path: rel.clone(),
                    line_no,
                    line: line.trim_end().to_string(),
                });
                Ok(true)
            } else {
                Ok(false)
            }
        });
        let _ = searcher.search_path(&matcher, path, &mut sink);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, rel: &str, content: &str) {
        let full = path.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }

    #[test]
    fn finds_notes_not_attachments_or_hidden_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "Glass.md", "# Glass\n\nsome glassware notes\n");
        write(root, "Notes/glass blower.md", "glass in a subfolder\n");
        write(root, "attachments/glass.txt", "plain glass text file\n");
        write(root, ".obsidian/app.json", "{ \"glass\": true }");

        let hits = search(root, "glass", 100).unwrap();
        let paths: Vec<&str> = hits.iter().map(|hit| hit.path.as_str()).collect();
        assert!(paths.contains(&"Glass.md"), "{paths:?}");
        assert!(paths.contains(&"Notes/glass blower.md"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.ends_with(".txt")), "{paths:?}");
        assert!(
            !paths.iter().any(|p| p.starts_with(".obsidian")),
            "{paths:?}"
        );
    }

    #[test]
    fn smart_case_and_line_numbers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "a.md", "one\nGLASS here\nglass too\n");

        // Lowercase query: case-insensitive.
        let hits = search(root, "glass", 100).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line_no, 2);
        assert_eq!(hits[0].line, "GLASS here");
        assert_eq!(hits[1].line_no, 3);

        // Uppercase in the query: case-sensitive.
        let hits = search(root, "GLASS", 100).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_no, 2);
    }

    #[test]
    fn max_hits_caps_and_invalid_regex_falls_back_to_literal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        for i in 0..5 {
            write(root, &format!("n{i}.md"), &format!("needle {i}\n"));
        }

        let hits = search(root, "needle", 3).unwrap();
        assert_eq!(hits.len(), 3);

        // `[` alone is not a valid regex; must match literally, not error.
        write(root, "literal.md", "has a [ bracket\n");
        let hits = search(root, "[ bracket", 100).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "literal.md");
    }
}
