//! Source-mode editor adapter for the Windows shell.
//!
//! Holds the editor's caret in the shared kernel convention (0-based
//! line + byte column) and does all offset math through
//! [`vault_app::editor`] — the same functions the Linux frontend calls —
//! so caret behavior is identical on both platforms and covered by the
//! kernel tests. The native cursor conversion (whatever type the WinUI
//! control exposes) reduces to these offsets.

use vault_app::editor::{self, Caret, TextMatch};

/// The Source editor's adapter state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceEditor {
    /// The caret in the shared (line, byte-column) convention.
    pub caret: Caret,
    /// Whether the editor currently shows a selection.
    pub has_selection: bool,
}

impl SourceEditor {
    /// The byte offset of the caret in `text` (shared kernel math).
    #[must_use]
    pub fn offset(&self, text: &str) -> usize {
        editor::offset_of(text, self.caret.0, self.caret.1)
    }

    /// Move the caret to the note's byte `offset` in `text` (shared
    /// kernel math; multi-byte boundaries snap like the Linux editor).
    pub fn set_offset(&mut self, text: &str, offset: usize) {
        self.caret = editor::position_of(text, offset);
        self.has_selection = false;
    }

    /// Every occurrence of `query` in `text` (shared kernel algorithm:
    /// single-line matches, byte columns).
    #[must_use]
    pub fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<TextMatch> {
        editor::find_matches(text, query, case_sensitive)
    }

    /// Move the caret to `matches[index]` and mark the selection. An
    /// out-of-range index leaves the caret alone (the find bar clamps
    /// before calling, mirroring the Linux wrap-around navigation).
    pub fn select_match(&mut self, index: usize, matches: &[TextMatch]) {
        if let Some(m) = matches.get(index) {
            self.caret = (m.line, m.start);
            self.has_selection = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "alpha beta\ngamma delta\n";

    #[test]
    fn caret_and_offset_round_trip_through_the_kernel() {
        let mut editor = SourceEditor::default();
        // The caret starts at the document start.
        assert_eq!(editor.caret, (0, 0));
        assert_eq!(editor.offset(TEXT), 0);

        // Offset 17 is line 1, column 6 ("delta" start).
        editor.set_offset(TEXT, 17);
        assert_eq!(editor.caret, (1, 6));
        assert!(!editor.has_selection);
        assert_eq!(editor.offset(TEXT), 17);

        // The end of the text snaps to the last boundary.
        editor.set_offset(TEXT, TEXT.len() + 5);
        assert_eq!(editor.offset(TEXT), TEXT.len());
    }

    #[test]
    fn find_and_select_use_the_shared_match_shape() {
        // "a" occurs six times across the two lines.
        let matches = SourceEditor::find_matches(TEXT, "a", false);
        assert_eq!(matches.len(), 6);
        assert_eq!(
            matches[0],
            TextMatch {
                line: 0,
                start: 0,
                end: 1
            }
        );
        // Match columns are byte offsets within their line.
        assert_eq!(
            matches[5],
            TextMatch {
                line: 1,
                start: 10,
                end: 11
            }
        );

        let mut editor = SourceEditor::default();
        editor.select_match(1, &matches);
        assert_eq!(editor.caret, (0, 4));
        assert!(editor.has_selection);

        // The caret offset lands on the match start.
        assert_eq!(editor.offset(TEXT), 4);

        // An out-of-range index is a no-op.
        let before = editor.caret;
        editor.select_match(99, &matches);
        assert_eq!(editor.caret, before);
    }

    #[test]
    fn case_sensitive_matching_matches_the_kernel() {
        let text = "Needle\nneedle\n";
        assert_eq!(SourceEditor::find_matches(text, "needle", true).len(), 1);
        assert_eq!(SourceEditor::find_matches(text, "needle", false).len(), 2);
    }
}
