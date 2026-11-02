//! Quick-switcher fuzzy matching over note paths, powered by nucleo (the
//! engine behind helix's picker). Small vaults need no streaming machinery:
//! score every path, sort by score, cut to a limit.

use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32String};

/// A scored candidate: index into the original needles slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ranked {
    pub index: usize,
    pub score: u32,
}

/// Rank `needles` against `query` (smart case, smart normalization). An empty
/// query passes everything through in original order.
pub fn rank(query: &str, needles: &[String], limit: usize) -> Vec<Ranked> {
    if query.is_empty() {
        return needles
            .iter()
            .take(limit)
            .enumerate()
            .map(|(index, _)| Ranked { index, score: 0 })
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
            Some(Ranked { index, score })
        })
        .collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    ranked.truncate(limit);
    ranked
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
