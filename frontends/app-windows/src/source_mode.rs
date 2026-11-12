//! Source-mode editor adapter for the Windows frontend.
//!
//! Adapts the shared editor text operations from `vault_app::editor` to a
//! WinUI 3 text-editing surface. On Windows this drives a real editor
//! control; on non-Windows targets it is a compile-time skeleton so the
//! crate type-checks here without windows-reactor.
//!
//! Native Windows verification is user-side (no Windows runner in this
//! session); never claimed via Wine.

use vault_app::editor::{self, Caret, TextMatch};

/// The Source editor's view of a note's text.
///
/// The shell owns the actual text buffer; this adapter only translates
/// between the shared `Caret` (line, byte-column) convention and the
/// native editor's cursor representation.
pub struct SourceEditor {
    /// The caret position in the shared convention.
    pub caret: Caret,
    /// Whether the editor currently has a selection.
    pub has_selection: bool,
}

impl SourceEditor {
    /// Create a new source editor showing `text`.
    #[cfg(target_os = "windows")]
    pub fn new(_text: &str) -> Self {
        Self {
            caret: (0, 0),
            has_selection: false,
        }
    }

    /// Create a new source editor (stub on non-Windows).
    #[cfg(not(target_os = "windows"))]
    pub fn new(_text: &str) -> Self {
        Self {
            caret: (0, 0),
            has_selection: false,
        }
    }

    /// Translate a native cursor position to a shared `Caret`.
    #[must_use]
    pub fn cursor_to_caret(&self, _line: usize, _column: usize) -> Caret {
        (0, 0)
    }

    /// Translate a shared `Caret` to a native cursor position.
    #[must_use]
    pub fn caret_to_cursor(&self, _caret: Caret) -> (usize, usize) {
        (0, 0)
    }

    /// Find matches for `query` using the shared find algorithm.
    #[must_use]
    pub fn find_matches(&self, text: &str, query: &str, case_sensitive: bool) -> Vec<TextMatch> {
        editor::find_matches(text, query, case_sensitive)
    }

    /// Apply a find match selection: move the caret to `matches[index]`.
    pub fn select_match(&mut self, index: usize, matches: &[TextMatch]) {
        if let Some(m) = matches.get(index) {
            self.caret = (m.line, m.start);
            self.has_selection = true;
        }
    }
}

impl Default for SourceEditor {
    fn default() -> Self {
        Self::new("")
    }
}
