//! Pure editor text operations shared by every frontend.
//!
//! These algorithms answer "what does this edit do to plain text" —
//! footnote insertion, find/replace, word ranges, auto-pair decisions —
//! with no toolkit types: positions are `(line, byte-column)` pairs,
//! exactly how the editor's own cursors measure columns. The frontends
//! build their native editor actions from these results (restructure
//! plan commit 09: pure operations with Unicode/boundary tests).
//!
//! The shared kernel never performs an edit itself; it computes the new
//! text (and caret) and hands it back.

use std::collections::HashSet;

use unicode_segmentation::UnicodeSegmentation;

/// A caret position: 0-based line and byte column within that line
/// (the same measure the editor widget's cursor columns use).
pub type Caret = (usize, usize);

/// One find hit: line number plus the byte range within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

// --- find / replace ---------------------------------------------------------

/// Every occurrence of `query` in `text`, in order. Queries are matched
/// within single lines (a newline in the query matches nothing); columns
/// are byte offsets, like the editor's cursor columns.
#[must_use]
pub fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<TextMatch> {
    if query.is_empty() || query.contains('\n') {
        return Vec::new();
    }
    let query_chars: Vec<char> = query.chars().collect();
    let mut out = Vec::new();
    for (line_no, line) in text.split('\n').enumerate() {
        let mut at = 0usize;
        while at <= line.len() {
            if !line.is_char_boundary(at) {
                at += 1;
                continue;
            }
            let mut probe = at;
            let mut hit = true;
            for &expected in &query_chars {
                match line[probe..].chars().next() {
                    Some(c) if char_matches(c, expected, case_sensitive) => {
                        probe += c.len_utf8();
                    }
                    _ => {
                        hit = false;
                        break;
                    }
                }
            }
            if hit {
                out.push(TextMatch {
                    line: line_no,
                    start: at,
                    end: probe,
                });
                at = probe;
            } else {
                at += line[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    out
}

/// Character equality under the find bar's case sensitivity. Only ASCII
/// case folding is applied, which keeps byte offsets stable.
fn char_matches(a: char, b: char, case_sensitive: bool) -> bool {
    a == b || (!case_sensitive && a.eq_ignore_ascii_case(&b))
}

/// Index of the first match at or after a caret position (0 when the
/// caret is past every match).
#[must_use]
pub fn first_at_or_after(matches: &[TextMatch], line: usize, column: usize) -> usize {
    matches
        .iter()
        .position(|m| (m.line, m.start) >= (line, column))
        .unwrap_or(0)
}

/// The next (or previous) match index, wrapping around.
#[must_use]
pub fn next_index(len: usize, current: usize, forward: bool) -> usize {
    if len == 0 {
        0
    } else if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

/// Splice `replacement` over every span (all within single lines,
/// non-overlapping, in order).
#[must_use]
pub fn replace_spans(text: &str, spans: &[TextMatch], replacement: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        let mut copied = 0usize;
        let mut touched = false;
        for span in spans.iter().filter(|m| m.line == i) {
            out.push_str(&line[copied..span.start]);
            out.push_str(replacement);
            copied = span.end;
            touched = true;
        }
        if touched {
            out.push_str(&line[copied..]);
        } else {
            out.push_str(line);
        }
        if i + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

// --- footnotes --------------------------------------------------------------

/// Insert a `[^N]` reference at the caret and a `[^N]: ` definition at
/// the end of the document; N is the first number without a definition.
///
/// The definitions are normalized on the way: every top-level
/// `[^label]:` line gets exactly one blank line before it. Markdown
/// needs that blank line — without it the definition is a lazy
/// continuation of the paragraph above and never becomes a footnote.
///
/// Returns the new text and the caret's place in it (right after the
/// inserted reference).
#[must_use]
pub fn insert_footnote(text: &str, caret: Caret, n: u32) -> (String, Caret) {
    let lines: Vec<&str> = text.split('\n').collect();
    let (mut lines, cursor_line) = normalise_footnote_definitions(lines, caret.0);
    // The reference goes in at the caret.
    let line = cursor_line.min(lines.len().saturating_sub(1));
    let reference = format!("[^{n}]");
    let column = char_boundary(&lines[line], caret.1);
    lines[line].insert_str(column, &reference);
    let caret = (line, column + reference.len());
    // The definition goes at the end, under one blank line.
    if lines.last().is_some_and(|last| !last.is_empty()) {
        lines.push(String::new());
    }
    lines.push(format!("[^{n}]: "));
    (lines.join("\n"), caret)
}

/// Give every top-level footnote definition exactly one blank line
/// before it.
///
/// The caret's line is tracked: a blank line removed above it moves it
/// up, one added moves it down.
fn normalise_footnote_definitions(lines: Vec<&str>, cursor_line: usize) -> (Vec<String>, usize) {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut cursor = cursor_line;
    let mut fence: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        // A definition inside a code fence is code, not a definition.
        let in_code = match fence_of(line) {
            Some((character, length)) => match fence {
                Some((open, open_len)) if open == character && length >= open_len => {
                    // The fence closes here.
                    fence = None;
                    false
                }
                Some(_) => true,
                None => {
                    fence = Some((character, length));
                    true
                }
            },
            None => fence.is_some(),
        };
        if !in_code && is_footnote_def(line) {
            // Drop the blank lines that are there…
            let mut removed = 0;
            while out.last().is_some_and(|last| last.trim().is_empty()) {
                out.pop();
                removed += 1;
            }
            // …and put exactly one back.
            let added = usize::from(!out.is_empty());
            if added == 1 {
                out.push(String::new());
            }
            if i < cursor_line {
                cursor = cursor + added - removed;
            }
        }
        out.push((*line).to_string());
    }
    (out, cursor)
}

/// Whether a line starts a footnote definition: `[^label]:`.
fn is_footnote_def(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("[^") else {
        return false;
    };
    let Some(end) = rest.find(']') else {
        return false;
    };
    end > 0 && rest[end + 1..].starts_with(':')
}

/// The code fence a line opens or closes, if it is one: the character
/// and the run length.
fn fence_of(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let character = rest.chars().next()?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = rest.chars().take_while(|&c| c == character).count();
    (length >= 3).then_some((character, length))
}

/// The largest character boundary at or before `column`.
#[must_use]
pub fn char_boundary(line: &str, column: usize) -> usize {
    let mut at = column.min(line.len());
    while at > 0 && !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The first footnote number that has no `[^N]:` definition in `text`.
#[must_use]
pub fn next_footnote_number(text: &str) -> u32 {
    let mut used = HashSet::new();
    let mut rest = text;
    while let Some(pos) = rest.find("[^") {
        let after = &rest[pos + 2..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            if let Ok(n) = digits.parse::<u32>() {
                // Only definitions reserve a number; bare references do not.
                if after[digits.len()..].starts_with("]:") {
                    used.insert(n);
                }
            }
        }
        rest = after;
    }
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

// --- word motion ------------------------------------------------------------

/// The byte range of the word before the caret, the way the editor's own
/// word motion finds it: from the start of the last word before the
/// caret up to the caret, or — when the caret sits at the start of a
/// line — the line break.
///
/// The live editor needs this at file level: its buffer holds one
/// region, so the word can live in the neighbouring one.
#[must_use]
pub fn word_before(text: &str, offset: usize) -> Option<(usize, usize)> {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let prefix = &before[line_start..];
    if !prefix.is_empty() {
        // The last word that starts before the caret; without one, the
        // whole prefix goes (the whitespace in front of the caret).
        let start = prefix
            .unicode_word_indices()
            .rev()
            .map(|(i, _)| i)
            .find(|&i| i < prefix.len())
            .unwrap_or(0);
        return Some((line_start + start, offset));
    }
    // The caret is at the start of a line: the motion reaches back to
    // the end of the line before it.
    let previous_end = line_start.checked_sub(1)?;
    Some((previous_end, offset))
}

/// The byte range of the word after the caret: the word's characters
/// (with the whitespace in front of them), or — at the end of a line —
/// the line break.
#[must_use]
pub fn word_after(text: &str, offset: usize) -> Option<(usize, usize)> {
    let rest = &text[offset..];
    let line_end = rest.find('\n').map_or(text.len(), |i| offset + i);
    let suffix = &text[offset..line_end];
    if !suffix.is_empty() {
        // The first word that ends after the caret; without one, the
        // rest of the line goes.
        let end = suffix
            .unicode_word_indices()
            .map(|(i, word)| i + word.len())
            .find(|&i| i > 0)
            .unwrap_or(suffix.len());
        return Some((offset, offset + end));
    }
    // The caret is at the end of a line: the motion steps over the line
    // break.
    (line_end < text.len()).then_some((offset, line_end + 1))
}

/// The byte offset of a caret position (0-based line, byte column) in
/// the note text.
#[must_use]
pub fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let mut offset = 0;
    for (i, current) in text.split('\n').enumerate() {
        if i == line {
            let mut at = offset + column.min(current.len());
            while at > offset && !text.is_char_boundary(at) {
                at -= 1;
            }
            return at;
        }
        offset += current.len() + 1;
    }
    text.len()
}

/// The caret position (0-based line, byte column) of a byte offset in
/// the note text.
#[must_use]
pub fn position_of(text: &str, offset: usize) -> Caret {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let column = offset - before.rfind('\n').map_or(0, |i| i + 1);
    (line, column)
}

/// Word count for the status bar: whitespace-separated runs.
#[must_use]
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

// --- auto pairs -------------------------------------------------------------

/// The wikilink being typed at a caret position, if any: the byte column
/// of the opening `[[` in the caret's line and the partial link text.
/// Suggesting stops when the link is closed (`]`), aliases are being typed
/// (`|`), or the `[[` sits on an earlier line.
#[must_use]
pub fn wiki_query_at(text: &str, caret: Caret) -> Option<(usize, String)> {
    let line = text.split('\n').nth(caret.0)?;
    let end = line.len().min(caret.1);
    if !line.is_char_boundary(end) {
        return None;
    }
    let before = &line[..end];
    let open = before.rfind("[[")?;
    let query = &before[open + 2..];
    if query.contains(']') || query.contains('|') {
        return None;
    }
    Some((open, query.to_string()))
}

/// The matching pair for `typed` when it is a bracket or quote.
#[must_use]
pub fn pair_for(typed: char) -> Option<(char, char)> {
    match typed {
        '(' | ')' => Some(('(', ')')),
        '[' | ']' => Some(('[', ']')),
        '{' | '}' => Some(('{', '}')),
        '"' => Some(('"', '"')),
        '\'' => Some(('\'', '\'')),
        '`' => Some(('`', '`')),
        _ => None,
    }
}

#[cfg(test)]
mod footnote_tests {
    use super::*;

    #[test]
    fn insert_footnote_puts_the_reference_and_the_definition() {
        let (text, cursor) = insert_footnote("body\n", (0, 4), 1);
        // The reference lands at the cursor; the definition at the end,
        // under a blank line.
        assert_eq!(text, "body[^1]\n\n[^1]: ");
        assert_eq!(cursor, (0, 8));
    }

    #[test]
    fn insert_footnote_normalizes_the_blank_lines_before_the_definitions() {
        // Two blank lines before a definition become one…
        let (text, _) = insert_footnote("body\n\n\n[^1]: note\n", (0, 4), 2);
        assert_eq!(text, "body[^2]\n\n[^1]: note\n\n[^2]: ");
        // …and none becomes one.
        let (text, _) = insert_footnote("body\n[^1]: note\n", (0, 4), 2);
        assert_eq!(text, "body[^2]\n\n[^1]: note\n\n[^2]: ");
    }

    #[test]
    fn a_definition_in_a_code_block_is_left_alone() {
        let (text, _) = insert_footnote("```\n[^1]: code\n```\n", (0, 0), 2);
        assert_eq!(text, "[^2]```\n[^1]: code\n```\n\n[^2]: ");
    }

    #[test]
    fn the_cursor_follows_the_lines_the_normalization_moves() {
        let (text, cursor) = insert_footnote("body\n\n\n[^1]: note\n\nafter\n", (5, 0), 2);
        // The blank lines above the definition collapsed, so the cursor
        // moved up with them.
        assert_eq!(cursor, (4, 4));
        assert_eq!(text, "body\n\n[^1]: note\n\n[^2]after\n\n[^2]: ");
    }

    #[test]
    fn next_footnote_number_skips_existing_definitions() {
        assert_eq!(next_footnote_number(""), 1);
        assert_eq!(next_footnote_number("no footnotes here"), 1);
        // Bare references do not reserve a number.
        assert_eq!(next_footnote_number("a[^1] ref[^1]\n"), 1);
        assert_eq!(next_footnote_number("[^1]: one\n"), 2);
        // Gaps are not filled: the next unused number wins.
        assert_eq!(next_footnote_number("[^1]: a\n[^3]: b\n"), 2);
        assert_eq!(next_footnote_number("[^1]: a\n[^2]: b\n"), 3);
    }
}

#[cfg(test)]
mod misc_tests {
    use super::*;

    #[test]
    fn word_count_counts_whitespace_runs() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("hello"), 1);
        assert_eq!(count_words("  two  words\nand more "), 4);
    }

    #[test]
    fn pair_table_covers_brackets_and_quotes() {
        assert_eq!(pair_for('('), Some(('(', ')')));
        assert_eq!(pair_for(')'), Some(('(', ')')));
        assert_eq!(pair_for('['), Some(('[', ']')));
        assert_eq!(pair_for(']'), Some(('[', ']')));
        assert_eq!(pair_for('{'), Some(('{', '}')));
        assert_eq!(pair_for('}'), Some(('{', '}')));
        assert_eq!(pair_for('"'), Some(('"', '"')));
        assert_eq!(pair_for('\''), Some(('\'', '\'')));
        assert_eq!(pair_for('`'), Some(('`', '`')));
        assert_eq!(pair_for('a'), None);
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    #[test]
    fn find_matches_scans_lines_with_case_handling() {
        let text = "Alpha alpha\nbeta ALPHA\n";
        // Case-insensitive (default): every occurrence, in order.
        assert_eq!(
            find_matches(text, "alpha", false),
            vec![
                TextMatch {
                    line: 0,
                    start: 0,
                    end: 5
                },
                TextMatch {
                    line: 0,
                    start: 6,
                    end: 11
                },
                TextMatch {
                    line: 1,
                    start: 5,
                    end: 10
                },
            ]
        );
        // Case-sensitive: only the exact ones.
        assert_eq!(
            find_matches(text, "alpha", true),
            vec![TextMatch {
                line: 0,
                start: 6,
                end: 11
            }]
        );
    }

    #[test]
    fn find_matches_edge_cases() {
        // Empty and multi-line queries match nothing.
        assert!(find_matches("text", "", false).is_empty());
        assert!(find_matches("text", "a\nb", false).is_empty());
        // Columns are byte offsets: é is two bytes.
        let text = "café café";
        let hits = find_matches(text, "café", false);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].start, 6);
        // Adjacent, non-overlapping occurrences.
        assert_eq!(find_matches("aaaa", "aa", false).len(), 2);
    }

    #[test]
    fn navigation_indices_wrap() {
        assert_eq!(first_at_or_after(&[], 0, 0), 0);
        let matches = vec![
            TextMatch {
                line: 0,
                start: 0,
                end: 1,
            },
            TextMatch {
                line: 0,
                start: 4,
                end: 5,
            },
            TextMatch {
                line: 2,
                start: 0,
                end: 1,
            },
        ];
        assert_eq!(first_at_or_after(&matches, 0, 0), 0);
        assert_eq!(first_at_or_after(&matches, 0, 4), 1);
        assert_eq!(first_at_or_after(&matches, 1, 0), 2);
        assert_eq!(first_at_or_after(&matches, 5, 0), 0);
        assert_eq!(next_index(3, 0, true), 1);
        assert_eq!(next_index(3, 2, true), 0);
        assert_eq!(next_index(3, 0, false), 2);
        assert_eq!(next_index(0, 5, true), 0);
    }

    #[test]
    fn replace_splices_are_byte_exact() {
        let text = "one two one\nthree\n";
        let replaced = replace_spans(
            &text,
            &[TextMatch {
                line: 0,
                start: 4,
                end: 7,
            }],
            "2",
        );
        assert_eq!(replaced, "one 2 one\nthree\n");
        // Every occurrence at once, including the trailing newline.
        let all = replace_spans(
            &text,
            &[
                TextMatch {
                    line: 0,
                    start: 0,
                    end: 3,
                },
                TextMatch {
                    line: 0,
                    start: 8,
                    end: 11,
                },
            ],
            "1",
        );
        assert_eq!(all, "1 two 1\nthree\n");
    }
}

#[cfg(test)]
mod wiki_tests {
    use super::*;

    #[test]
    fn wiki_query_detects_the_open_link_being_typed() {
        let text = "see [[An\ntail\n";
        // Cursor after "[[An": link start at byte 4, partial "An".
        assert_eq!(wiki_query_at(text, (0, 8)), Some((4, "An".to_string())));
        // Empty query right after "[[".
        assert_eq!(wiki_query_at(text, (0, 6)), Some((4, String::new())));
    }

    #[test]
    fn wiki_query_stops_at_closers_and_earlier_lines() {
        // A closed link suggests nothing.
        assert_eq!(wiki_query_at("see [[An]]|", (0, 9)), None);
        // Typing an alias (`|`) suggests nothing.
        assert_eq!(wiki_query_at("see [[An|ali", (0, 12)), None);
        // An unclosed `[[` on an earlier line does not reach across lines.
        assert_eq!(wiki_query_at("see [[An\ntail\n", (1, 4)), None);
        // No link at all.
        assert_eq!(wiki_query_at("plain text", (0, 10)), None);
    }

    #[test]
    fn wiki_query_columns_are_byte_offsets() {
        // "é[[no" — é is two bytes, so `[[` starts at byte 2.
        let text = "é[[no\n";
        assert_eq!(wiki_query_at(text, (0, 6)), Some((2, "no".to_string())));
    }
}

#[cfg(test)]
mod word_motion_tests {
    use super::*;

    /// The bytes a word delete would remove, the way the editor's own
    /// word motion finds them.
    fn before(text: &str, offset: usize) -> Option<String> {
        word_before(text, offset).map(|(start, end)| text[start..end].to_string())
    }

    fn after(text: &str, offset: usize) -> Option<String> {
        word_after(text, offset).map(|(start, end)| text[start..end].to_string())
    }

    #[test]
    fn the_word_before_the_caret_is_the_word_it_starts() {
        let text = "alpha beta";
        // The caret in the middle of "beta": the word part before it.
        assert_eq!(before(text, 8).as_deref(), Some("be"));
        // The caret before the word's last letter: the part before it.
        assert_eq!(before(text, 9).as_deref(), Some("bet"));
        // The caret at the end of the line: the whole word.
        assert_eq!(before(text, 10).as_deref(), Some("beta"));
        // The caret at the start of the line: nothing before it.
        assert_eq!(before(text, 0), None);
    }

    #[test]
    fn whitespace_before_the_caret_goes_with_the_word() {
        // The motion finds the start of the last word before the caret,
        // so the spaces between it and the caret go too.
        assert_eq!(before("alpha   beta", 8).as_deref(), Some("alpha   "));
        // A line with no word at all: only the spaces go.
        assert_eq!(before("   ", 3).as_deref(), Some("   "));
    }

    #[test]
    fn a_caret_at_a_line_start_reaches_the_line_break() {
        let text = "alpha beta\ngamma";
        // The caret at the start of the second line: the motion jumps to
        // the end of the line before it, so the break is what goes.
        assert_eq!(before(text, 11).as_deref(), Some("\n"));
        // The very start of the text: there is nothing to delete.
        assert_eq!(before(text, 0), None);
    }

    #[test]
    fn the_word_after_the_caret_is_the_word_it_ends() {
        let text = "alpha beta";
        // The caret at the start of the line: the whole word.
        assert_eq!(after(text, 0).as_deref(), Some("alpha"));
        // The caret inside the word: the rest of it.
        assert_eq!(after(text, 2).as_deref(), Some("pha"));
        // The caret before the word's last letter: that letter.
        assert_eq!(after(text, 9).as_deref(), Some("a"));
        // The caret at the end of the text: nothing after it.
        assert_eq!(after(text, 10), None);
    }

    #[test]
    fn a_caret_at_a_line_end_reaches_the_line_break() {
        let text = "alpha beta\ngamma";
        // The caret at the end of the first line: the motion steps over
        // the break.
        assert_eq!(after(text, 10).as_deref(), Some("\n"));
        // The end of the text: nothing follows.
        assert_eq!(after(text, text.len()), None);
    }

    #[test]
    fn offsets_and_positions_round_trip() {
        let text = "alpha beta\ngamma\n";
        assert_eq!(offset_of(text, 0, 6), 6);
        assert_eq!(offset_of(text, 1, 0), 11);
        assert_eq!(offset_of(text, 1, 3), 14);
        // A column past the line's end is clamped to it.
        assert_eq!(offset_of(text, 0, 99), 10);
        // A line past the text's end is clamped to the text's end.
        assert_eq!(offset_of(text, 9, 0), text.len());
        assert_eq!(position_of(text, 6), (0, 6));
        assert_eq!(position_of(text, 11), (1, 0));
        assert_eq!(position_of(text, 14), (1, 3));
        assert_eq!(position_of(text, text.len()), (2, 0));
    }

    #[test]
    fn multi_byte_text_keeps_the_columns_on_character_boundaries() {
        let text = "äöü x";
        // The caret cannot sit inside a character: the offset snaps back
        // to the character's start.
        assert_eq!(offset_of(text, 0, 1), 0);
        assert_eq!(offset_of(text, 0, 2), 2);
        assert_eq!(offset_of(text, 0, 5), 4);
        assert_eq!(position_of(text, 5), (0, 4));
    }
}
