//! Shared vault-wide search session with stale-result protection.
//!
//! A search runs asynchronously: the user may submit again (or switch
//! notes) before the worker answers. Every request carries a stable
//! [`SearchRequestId`]; results keyed to an older request are dropped
//! instead of overwriting fresh ones (restructure plan §5.3). The
//! frontend executes the actual vault scan; this model owns the query,
//! the displayed hits, and which request is current.

use serde::{Deserialize, Serialize};

use crate::ids::{IdAllocator, SearchRequestId};

/// One search result row as both frontends render it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHitView {
    /// Vault-relative note path.
    pub path: String,
    /// 1-based line number of the match.
    pub line_no: u64,
    /// The matching line's text.
    pub line: String,
}

/// The vault-wide search panel's state.
#[derive(Debug, Default)]
pub struct SearchSession {
    query: String,
    hits: Vec<SearchHitView>,
    /// The most recent request; only its completion may update `hits`.
    current: Option<SearchRequestId>,
    allocator: IdAllocator,
}

impl SearchSession {
    /// Record the query being typed. Hits stay until the next submit.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// The query currently in the input.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The hits currently displayed.
    #[must_use]
    pub fn hits(&self) -> &[SearchHitView] {
        &self.hits
    }

    /// Start a new request: it becomes the only one whose completion is
    /// applied. Returns its id for the effect the frontend will run.
    pub fn begin_request(&mut self) -> SearchRequestId {
        let id = self.allocator.allocate();
        self.current = Some(id);
        id
    }

    /// Whether a completion for `id` would still be applied.
    #[must_use]
    pub fn is_current(&self, id: SearchRequestId) -> bool {
        self.current == Some(id)
    }

    /// Apply a completed request. Returns `false` (leaving the current
    /// hits untouched) when the completion belongs to an older request.
    pub fn complete(&mut self, id: SearchRequestId, hits: Vec<SearchHitView>) -> bool {
        if !self.is_current(id) {
            return false;
        }
        self.hits = hits;
        true
    }

    /// Reset for a newly opened vault.
    pub fn clear(&mut self) {
        self.query.clear();
        self.hits.clear();
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, line_no: u64) -> SearchHitView {
        SearchHitView {
            path: path.to_string(),
            line_no,
            line: "needle".to_string(),
        }
    }

    #[test]
    fn the_query_is_kept_and_hits_arrive_only_for_the_current_request() {
        let mut session = SearchSession::default();
        session.set_query("needle");
        assert_eq!(session.query(), "needle");
        assert!(session.hits().is_empty());

        let first = session.begin_request();
        assert!(session.is_current(first));
        assert!(session.complete(first, vec![hit("a.md", 3)]));
        assert_eq!(session.hits().len(), 1);

        // A second request makes the first stale.
        let second = session.begin_request();
        assert!(!session.complete(first, vec![hit("old.md", 1)]));
        assert_eq!(session.hits().len(), 1, "stale results are dropped");
        assert!(session.complete(second, vec![hit("b.md", 7), hit("c.md", 9)]));
        assert_eq!(session.hits().len(), 2);
    }

    #[test]
    fn a_never_issued_request_is_stale() {
        let mut session = SearchSession::default();
        let issued = session.begin_request();
        let never = SearchRequestId::new(999);
        assert!(!session.complete(never, vec![hit("x.md", 1)]));
        assert!(session.complete(issued, vec![]));
    }

    #[test]
    fn clear_resets_the_query_hits_and_requests() {
        let mut session = SearchSession::default();
        session.set_query("q");
        let id = session.begin_request();
        assert!(session.complete(id, vec![hit("a.md", 1)]));
        session.clear();
        assert_eq!(session.query(), "");
        assert!(session.hits().is_empty());
        assert!(
            !session.is_current(id),
            "old requests are stale after clear"
        );
    }
}
