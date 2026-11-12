//! Quick-switcher fuzzy matching over note paths, powered by nucleo (the
//! engine behind helix's picker). Small vaults need no streaming machinery:
//! score every path, sort by score, cut to a limit.
//!
//! Two rankers live here: [`rank`] for plain lists of titles (the
//! wikilink dropdown) and [`rank_paths`] for the quick switcher, which
//! matches against whole vault paths and therefore needs path-aware
//! rules.

use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32String};

/// A scored candidate: index into the original needles slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ranked {
    pub index: usize,
    pub score: u32,
    /// Match quality tier: 2 = every query word matched the file
    /// title, 1 = some word matched only the folder path, 0 = plain
    /// (no tiering).
    pub tier: u8,
}

/// Rank `needles` against `query` (smart case, smart normalization). An empty
/// query passes everything through in original order.
pub fn rank(query: &str, needles: &[String], limit: usize) -> Vec<Ranked> {
    if query.is_empty() {
        return needles
            .iter()
            .take(limit)
            .enumerate()
            .map(|(index, _)| Ranked {
                index,
                score: 0,
                tier: 0,
            })
            .collect();
    }

    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut ranked: Vec<Ranked> = needles
        .iter()
        .enumerate()
        .filter_map(|(index, needle)| {
            let haystack = Utf32String::from(needle.as_str());
            let score = pattern.score(haystack.slice(..), &mut matcher)?;
            Some(Ranked {
                index,
                score,
                tier: 0,
            })
        })
        .collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    ranked.truncate(limit);
    ranked
}

/// Rank vault paths for the quick switcher.
///
/// The generic [`rank`] has two quirks that make a file picker feel
/// wrong, so paths get their own ranking:
///
/// 1. The query is split into words, and *every* word must match
///    somewhere in the path, in any order. A single conjunctive
///    pattern can only match one haystack in order, so "projects
///    note" found nothing even when one note matched "projects" in
///    its folder and "note" in its title.
/// 2. Words matched in the file title outrank words matched only in
///    the folder path, so typing a note's name doesn't surface
///    unrelated files whose folder happens to contain those letters.
///
/// Ties keep the caller's order (the vault's alphabetical paths).
/// The file title of a vault-relative path: the last component without
/// its `.md` ("Notes/Journal/Daily.md" → "Daily"). The path-aware ranker
/// scores the title separately from the whole path.
pub fn title_of(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string()
}

/// Rank `paths` against `query` with path-aware rules: `/` counts as a
/// delimiter, so matches right after a folder separator get the
/// word-boundary bonus, and word hits on the file title outrank hits on
/// the folder path (tiered).
pub fn rank_paths(query: &str, paths: &[String], limit: usize) -> Vec<Ranked> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return paths
            .iter()
            .take(limit)
            .enumerate()
            .map(|(index, _)| Ranked {
                index,
                score: 0,
                tier: 0,
            })
            .collect();
    }

    // Path-tuned matcher: `/` counts as a delimiter, so matches right
    // after a folder separator get the word-boundary bonus.
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut ranked: Vec<Candidate> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let title = title_of(path);
        let mut score_total = 0u32;
        // Tier 2: every word hit the title. Tier 1: at least one word
        // only hit the folder path.
        let mut tier = 2u8;
        let mut matched = true;
        for word in &words {
            let pattern = Pattern::new(
                word,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );
            let in_title = pattern.score(Utf32String::from(title.as_str()).slice(..), &mut matcher);
            let in_path = pattern.score(Utf32String::from(path.as_str()).slice(..), &mut matcher);
            match (in_title, in_path) {
                (Some(word_score), _) => score_total += word_score,
                (None, Some(word_score)) => {
                    score_total += word_score;
                    tier = 1;
                }
                (None, None) => {
                    matched = false;
                    break;
                }
            }
        }
        if !matched {
            continue;
        }
        ranked.push(Candidate {
            ranked: Ranked {
                index,
                score: score_total,
                tier,
            },
            // A shorter title is the tighter match ("glass" over
            // "Glass redesign") when the scores tie.
            title_len: title.chars().count(),
            // Case-insensitive path for tie-breaks: byte order would
            // rank "Projects/..." before "glass.md" ('P' < 'g').
            path_key: path.to_lowercase(),
        });
    }
    ranked.sort_by(|a, b| {
        b.ranked
            .tier
            .cmp(&a.ranked.tier)
            .then(b.ranked.score.cmp(&a.ranked.score))
            .then(a.title_len.cmp(&b.title_len))
            .then(a.path_key.cmp(&b.path_key))
            .then(a.ranked.index.cmp(&b.ranked.index))
    });
    ranked.truncate(limit);
    ranked.into_iter().map(|c| c.ranked).collect()
}

/// A path candidate plus the tie-break keys the ranking needs.
struct Candidate {
    ranked: Ranked,
    title_len: usize,
    path_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        [
            "Journal/2026/04/14.md",
            "Glass.md",
            "Notes/Glass.md",
            "Archive/inner.md",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn empty_query_passes_everything_through_in_order() {
        let ranked = rank("", &paths(), 10);
        assert_eq!(ranked.len(), 4);
        assert!(ranked.iter().enumerate().all(|(i, r)| r.index == i));
    }

    #[test]
    fn fuzzy_query_ranks_and_filters() {
        let paths = paths();
        let ranked = rank("glss", &paths, 10);
        let picked: Vec<&str> = ranked.iter().map(|r| paths[r.index].as_str()).collect();
        assert_eq!(picked.len(), 2, "{picked:?}");
        assert!(picked.iter().all(|p| p.contains("Glass")), "{picked:?}");
        // Basename matches outrank deep paths on a tie-ish score.
        assert!(ranked[0].score >= ranked[1].score);
    }

    #[test]
    fn limit_caps_results() {
        let ranked = rank("", &paths(), 2);
        assert_eq!(ranked.len(), 2);
    }
}

#[cfg(test)]
mod path_ranking_tests {
    use super::*;

    fn vault_paths() -> Vec<String> {
        [
            "Archive/2025/meeting-notes.md",
            "Daily/2026/11/Note ideas.md",
            "Inbox/quick note.md",
            "Notebook.md",
            "Projects/Glass redesign.md",
            "Projects/notes.md",
            "Reading List.md",
            "glass.md",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    fn picked(query: &str, paths: &[String]) -> Vec<String> {
        rank_paths(query, paths, 20)
            .into_iter()
            .map(|r| paths[r.index].clone())
            .collect()
    }

    #[test]
    fn title_matches_outrank_folder_only_matches() {
        let paths = vault_paths();
        // "daily" is only in a folder path, so it lands in the lower tier.
        let ranked = rank_paths("daily", &paths, 20);
        assert!(ranked.iter().any(|r| r.tier == 1), "folder match found");
        // With a title match in the mix, titles win.
        let mut with_title = paths.clone();
        with_title.push("Daily review.md".to_string());
        let order = picked("daily", &with_title);
        assert_eq!(
            order.first().map(String::as_str),
            Some("Daily review.md"),
            "{order:?}"
        );
    }

    #[test]
    fn words_can_match_across_the_folder_and_the_title() {
        let paths = vault_paths();
        // The generic ranker found nothing here: "projects" only matches
        // the folder and "note" only the title.
        let order = picked("projects note", &paths);
        assert_eq!(
            order.first().map(String::as_str),
            Some("Projects/notes.md"),
            "{order:?}"
        );
        // A word that matches nothing at all still excludes the file.
        let order = picked("projects zzzz", &paths);
        assert!(order.is_empty(), "{order:?}");
    }

    #[test]
    fn words_may_appear_in_any_order() {
        let paths = vault_paths();
        let order = picked("note quick", &paths);
        assert_eq!(
            order.first().map(String::as_str),
            Some("Inbox/quick note.md"),
            "{order:?}"
        );
    }

    #[test]
    fn equal_scores_favor_the_tighter_title() {
        let paths = vault_paths();
        let order = picked("glass", &paths);
        // "glass.md" (title exactly the query) and "Projects/Glass
        // redesign.md" score the same; the shorter title leads, and the
        // case-insensitive tie-break keeps "glass.md" ahead.
        assert_eq!(
            order.first().map(String::as_str),
            Some("glass.md"),
            "{order:?}"
        );
        assert!(
            order.contains(&"Projects/Glass redesign.md".to_string()),
            "{order:?}"
        );
    }

    #[test]
    fn empty_query_keeps_the_input_order() {
        let paths = vault_paths();
        let ranked = rank_paths("   ", &paths, 3);
        let got: Vec<&str> = ranked.iter().map(|r| paths[r.index].as_str()).collect();
        assert_eq!(
            got,
            [
                "Archive/2025/meeting-notes.md",
                "Daily/2026/11/Note ideas.md",
                "Inbox/quick note.md"
            ]
        );
    }
}
