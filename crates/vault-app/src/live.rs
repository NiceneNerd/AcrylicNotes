//! Live preview: block slicing and splicing behind the `Live` pane mode.
//!
//! The live preview renders every top-level block of a note like the
//! reading pane does — except the block being edited, which renders as
//! a raw-source text editor. The block-to-text mapping comes from
//! vault-md's per-block source spans (`Doc::spans`): a block spans
//! from its first line to the line before the next block, blank lines
//! attaching to the block above. Those spans therefore tile the text
//! with no gaps, and pushing an edited block's source back into the
//! note is a single contiguous line-range replace.

use vault_md::Doc;

/// One editable region of a note: an inclusive 1-based file-line range.
///
/// The regions tile the note: editing one never disturbs another, and
/// a block region's source plus the frontmatter's, spliced back in
/// place, reproduces the text byte for byte (the round-trip test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSlice {
    /// First file line of the region (1-based, inclusive).
    pub start_line: usize,
    /// Last file line of the region (1-based, inclusive).
    pub end_line: usize,
    /// The top-level block rendered for this region, if it is a
    /// markdown block (`None` for the frontmatter and for a body with
    /// no blocks).
    pub block: Option<usize>,
}

impl BlockSlice {
    /// The region's source: file lines `start..=end`, joined with `\n`.
    pub fn source(&self, text: &str) -> String {
        let mut out = String::new();
        for (i, line) in text.split('\n').enumerate().skip(self.start_line - 1) {
            if i + 1 > self.end_line {
                break;
            }
            if i > self.start_line - 1 {
                out.push('\n');
            }
            out.push_str(line);
        }
        out
    }
}

/// The note's editable regions, from the parsed document's block spans.
///
/// The frontmatter is its own region and absorbs the blank lines
/// between it and the first block; a body with no blocks at all is one
/// region covering the body.
pub fn slices(text: &str, doc: &Doc) -> Vec<BlockSlice> {
    let line_count = text.split('\n').count().max(1);
    let fm = doc.frontmatter_line_count();
    let mut out = Vec::new();
    if fm > 0 {
        // The frontmatter region exists whenever the frontmatter does; it
        // also owns the blank lines between it and the first block (the
        // parser attaches no markdown block to them).
        let end = doc
            .spans
            .first()
            .map_or(fm, |span| (span.start - 1).max(fm));
        out.push(BlockSlice {
            start_line: 1,
            end_line: end,
            block: None,
        });
    }
    for (i, span) in doc.spans.iter().enumerate() {
        // Blank lines before the first block ride with it (there is no
        // frontmatter region to own them); the parser already owns the
        // blank line *after* every block but the last.
        let start = if i == 0 && fm == 0 {
            span.start.min(fm + 1)
        } else {
            span.start
        };
        out.push(BlockSlice {
            start_line: start,
            end_line: span.end,
            block: Some(i),
        });
    }
    match out.last_mut() {
        // Trailing blank lines at EOF belong to the last block.
        Some(last) if last.block.is_some() => last.end_line = last.end_line.max(line_count),
        _ => {}
    }
    if doc.blocks.is_empty() {
        // A note without markdown blocks is one body region (possibly
        // empty), so there is always something to click into.
        out.push(BlockSlice {
            start_line: fm + 1,
            end_line: line_count.max(fm + 1),
            block: None,
        });
    }
    out
}

/// The note with the newline just before file line `line` (1-based,
/// `line >= 2`) deleted: the previous line merges into it. This is the
/// whole-note text operation behind a backspace at a region's start and
/// a delete at a region's end — one newline, spliced everywhere at once.
pub fn delete_newline_before(text: &str, line: usize) -> Option<String> {
    if line < 2 || line > text.split('\n').count() {
        return None;
    }
    let mut offset = 0usize;
    for (i, cur) in text.split('\n').enumerate() {
        if i == line - 1 {
            break;
        }
        offset += cur.len() + 1;
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..offset - 1]);
    out.push_str(&text[offset..]);
    Some(out)
}

/// Replace file lines `start..=end` (1-based, inclusive) with `source`.
pub fn replace_lines(text: &str, start_line: usize, end_line: usize, source: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let n = i + 1;
        if n == start_line {
            out.extend(source.split('\n'));
        }
        if n < start_line || n > end_line {
            out.push(line);
        }
    }
    out.join("\n")
}

/// The slice containing `line` (0-based file line), clamped to the
/// last slice. The slices tile, so every line lands in one.
pub fn block_containing(slices: &[BlockSlice], line: usize) -> usize {
    slices
        .iter()
        .position(|s| s.start_line <= line + 1 && line < s.end_line)
        .unwrap_or_else(|| slices.len().saturating_sub(1))
}

/// A caret position clamped inside `source`: the line count and the
/// target line's column count are the walls.
pub fn clamp_position(source: &str, line: usize, column: usize) -> (usize, usize) {
    let lines: Vec<&str> = source.split('\n').collect();
    let line = line.min(lines.len() - 1);
    (line, column.min(lines[line].chars().count()))
}

/// The caret position at the end of `source`'s content: the last line
/// that is not all blank, since a region's trailing blank lines are
/// the gap to the next block, not text. An all-blank region: (0, 0).
pub fn content_end(source: &str) -> (usize, usize) {
    let lines: Vec<&str> = source.split('\n').collect();
    for (i, line) in lines.iter().enumerate().rev() {
        if !line.trim().is_empty() {
            return (i, line.chars().count());
        }
    }
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_md::parse_note;

    fn round_trips(text: &str) {
        let doc = parse_note(text);
        let slices = slices(text, &doc);
        assert!(!slices.is_empty(), "every note has at least one region");
        for slice in &slices {
            assert_eq!(
                replace_lines(text, slice.start_line, slice.end_line, &slice.source(text)),
                text,
                "splicing a region's own source back must be a no-op"
            );
        }
    }

    #[test]
    fn frontmatter_without_blocks_is_two_regions() {
        let text = "---\ntitle: x\n---\n\n\n";
        round_trips(text);
        let doc = parse_note(text);
        let slices = slices(text, &doc);
        assert_eq!(slices.len(), 2);
        assert_eq!(
            slices[0],
            BlockSlice {
                start_line: 1,
                end_line: 3,
                block: None
            }
        );
        assert_eq!(
            slices[1],
            BlockSlice {
                start_line: 4,
                end_line: 6,
                block: None
            }
        );
    }

    #[test]
    fn regions_tile_simple_paragraphs() {
        let text = "alpha\n\nbeta\n\ngamma";
        round_trips(text);
        let slices = slices(text, &parse_note(text));
        assert_eq!(slices.len(), 3);
        assert_eq!(
            slices[0],
            BlockSlice {
                start_line: 1,
                end_line: 2,
                block: Some(0)
            }
        );
        assert_eq!(
            slices[1],
            BlockSlice {
                start_line: 3,
                end_line: 4,
                block: Some(1)
            }
        );
        assert_eq!(
            slices[2],
            BlockSlice {
                start_line: 5,
                end_line: 5,
                block: Some(2)
            }
        );
    }

    #[test]
    fn frontmatter_and_its_following_blanks_are_one_region() {
        let text = "---\ntitle: x\n---\n\n\n# Head\n\nbody";
        round_trips(text);
        let slices = slices(text, &parse_note(text));
        assert_eq!(
            slices[0],
            BlockSlice {
                start_line: 1,
                end_line: 5,
                block: None
            }
        );
        assert_eq!(
            slices[1],
            BlockSlice {
                start_line: 6,
                end_line: 7,
                block: Some(0)
            }
        );
    }

    #[test]
    fn a_body_without_blocks_is_one_region() {
        for text in ["", "\n", "  \n  "] {
            round_trips(text);
            let slices = slices(text, &parse_note(text));
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].block, None);
        }
    }

    #[test]
    fn leading_blanks_ride_with_the_first_block() {
        let text = "\n\n\npara";
        round_trips(text);
        let slices = slices(text, &parse_note(text));
        assert_eq!(slices.len(), 1);
        assert_eq!(
            slices[0],
            BlockSlice {
                start_line: 1,
                end_line: 4,
                block: Some(0)
            }
        );
    }

    #[test]
    fn a_list_is_one_region_despite_not_passing_blank_lines() {
        let text = "- one\n- two\n\npara";
        round_trips(text);
        let slices = slices(text, &parse_note(text));
        assert_eq!(slices.len(), 2);
        // The blank line after the list is part of the list block's span.
        assert_eq!(
            slices[0],
            BlockSlice {
                start_line: 1,
                end_line: 3,
                block: Some(0)
            }
        );
        assert_eq!(
            slices[1],
            BlockSlice {
                start_line: 4,
                end_line: 4,
                block: Some(1)
            }
        );
    }

    #[test]
    fn replace_lines_replaces_exactly_the_range() {
        let text = "a\nb\nc";
        assert_eq!(replace_lines(text, 2, 2, "B\nB"), "a\nB\nB\nc");
        assert_eq!(replace_lines(text, 1, 3, "one"), "one");
        assert_eq!(replace_lines(text, 1, 1, ""), "\nb\nc");
    }

    #[test]
    fn block_containing_clamps_to_the_last_region() {
        let text = "alpha\n\nbeta";
        let slices = slices(text, &parse_note(text));
        assert_eq!(block_containing(&slices, 0), 0);
        assert_eq!(block_containing(&slices, 1), 0);
        assert_eq!(block_containing(&slices, 2), 1);
        assert_eq!(block_containing(&slices, 99), 1);
    }

    #[test]
    fn clamp_position_and_content_end_reflect_the_source() {
        assert_eq!(clamp_position("ab\nc", 5, 9), (1, 1));
        assert_eq!(clamp_position("ab\nc", 0, 1), (0, 1));
        assert_eq!(content_end("ab\n\n"), (0, 2));
        assert_eq!(content_end("\n\n"), (0, 0));
    }

    #[test]
    fn delete_newline_before_splices_one_newline() {
        assert_eq!(
            delete_newline_before("alpha\n\nbeta\n\ngamma", 3).as_deref(),
            Some("alpha\nbeta\n\ngamma")
        );
        // The first file line has no newline before it.
        assert_eq!(delete_newline_before("alpha\nbeta", 1), None);
        // A past-the-end line has none either.
        assert_eq!(delete_newline_before("alpha\nbeta", 3), None);
    }
}
