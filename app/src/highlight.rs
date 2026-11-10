//! Editor syntax highlighting: markdown plus Obsidian's wikilinks.
//!
//! iced's highlighter is syntect's Markdown grammar, which knows nothing
//! about `[[wikilinks]]`: they come out as plain text. This module wraps
//! that highlighter and paints the wikilinks itself — the target half in
//! the theme's accent, the custom-label half in its green — so both the
//! source editor and the live preview show them.
//!
//! The wrapper keeps the base highlighter's line-by-line contract: the
//! widget feeds lines in order and says which line changed when the text
//! changes. Fenced code blocks are therefore tracked here: a wikilink
//! inside a code block is code, not a link. The state is checkpointed
//! every [`LINES_PER_CHECKPOINT`] lines (the base highlighter's
//! granularity), so a jump back to an edited line can restore the state
//! there and rebuild it as the lines are fed again.

use std::ops::Range;

use cosmic::iced::core::font::Font;
use cosmic::iced::core::text::highlighter::{Format, Highlighter};

/// How many lines between fence-state checkpoints.
const LINES_PER_CHECKPOINT: usize = 50;

/// Which half of a wikilink a highlighted range belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WikiPart {
    /// `[[target` — the link's target, up to the bar.
    Target,
    /// `|alias` — the custom display text, plus the closing brackets.
    Alias,
}

/// One highlighted range: the syntax theme's format for it, plus which
/// half of a wikilink it is (when it is part of one).
#[derive(Debug, Clone, Copy)]
pub struct Highlight {
    /// The syntax theme's format for the range.
    pub base: Format<Font>,
    /// The wikilink half, when the range belongs to a wikilink.
    pub wiki: Option<WikiPart>,
}

/// Markdown highlighting with Obsidian's wikilinks painted on top.
pub struct MarkdownHighlighter {
    /// syntect's Markdown grammar (the base highlighter).
    base: cosmic::iced::highlighter::Highlighter,
    /// Whether the lines being fed are inside a fenced code block.
    fences: FenceTracker,
}

impl Highlighter for MarkdownHighlighter {
    type Settings = cosmic::iced::highlighter::Settings;
    type Highlight = Highlight;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Self::Highlight)>;

    fn new(settings: &Self::Settings) -> Self {
        Self {
            base: cosmic::iced::highlighter::Highlighter::new(settings),
            fences: FenceTracker::default(),
        }
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        self.base.update(new_settings);
        // The theme changed: the parse state restarts, so does ours.
        self.fences = FenceTracker::default();
    }

    fn change_line(&mut self, line: usize) {
        self.base.change_line(line);
        // The base restarts at its own snapshot boundary; restore the
        // fence state for the line it will feed next.
        let at = self.base.current_line();
        self.fences.jump(at);
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let base: Vec<(Range<usize>, cosmic::iced::highlighter::Highlight)> =
            self.base.highlight_line(line).collect();
        // The base counts the lines it has been fed; the line it just
        // took is one before its counter.
        let index = self.base.current_line().saturating_sub(1);
        let wiki = if self.fences.is_code(index, line) {
            Vec::new()
        } else {
            wikilink_spans(line)
        };
        merge(base, wiki).into_iter()
    }

    fn current_line(&self) -> usize {
        self.base.current_line()
    }
}

/// Paint the wikilink ranges over the syntax theme's ranges: the
/// wikilink's own colour wins over the markdown colour inside it, and
/// the range is split at the bar so the two halves can differ.
fn merge(
    base: Vec<(Range<usize>, cosmic::iced::highlighter::Highlight)>,
    wiki: Vec<(Range<usize>, WikiPart)>,
) -> Vec<(Range<usize>, Highlight)> {
    let mut out = Vec::with_capacity(base.len());
    let mut next = 0usize;
    for (range, style) in base {
        let format = style.to_format();
        let mut start = range.start;
        // Skip the wikilinks that ended before this range.
        while next < wiki.len() && wiki[next].0.end <= start {
            next += 1;
        }
        let mut i = next;
        while i < wiki.len() && wiki[i].0.start < range.end {
            let (span, part) = &wiki[i];
            if span.start > start {
                out.push((
                    start..span.start,
                    Highlight {
                        base: format,
                        wiki: None,
                    },
                ));
            }
            let from = span.start.max(start);
            let to = span.end.min(range.end);
            if from < to {
                out.push((
                    from..to,
                    Highlight {
                        base: format,
                        wiki: Some(*part),
                    },
                ));
            }
            start = to;
            i += 1;
        }
        if start < range.end {
            out.push((
                start..range.end,
                Highlight {
                    base: format,
                    wiki: None,
                },
            ));
        }
    }
    out
}

/// The wikilinks on one line, as byte ranges and the half each belongs
/// to. Inline code spans are skipped: a `[[link]]` inside backticks is
/// code, not a link.
fn wikilink_spans(line: &str) -> Vec<(Range<usize>, WikiPart)> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                // An inline code span runs to the next run of the same
                // number of backticks.
                let run = run_len(bytes, i, b'`');
                match find_run(bytes, i + run, b'`', run) {
                    Some(end) => i = end + run,
                    None => i += run,
                }
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                let Some(close) = find_sub(bytes, i + 2, b"]]") else {
                    i += 1;
                    continue;
                };
                let inner = &line[i + 2..close];
                // An empty link, or one that opens another link, is not
                // a link (the parser rejects both).
                if inner.is_empty() || inner.contains("[[") {
                    i += 1;
                    continue;
                }
                // `![[...]]` is an embed: the bang belongs to the link.
                let start =
                    if i > 0 && bytes[i - 1] == b'!' && bytes.get(i.wrapping_sub(2)) != Some(&b'!')
                    {
                        i - 1
                    } else {
                        i
                    };
                // The bar splits the target from the custom label (the
                // same split the parser makes).
                match inner.find('|') {
                    Some(bar) => {
                        let bar = i + 2 + bar;
                        spans.push((start..bar, WikiPart::Target));
                        spans.push((bar..close + 2, WikiPart::Alias));
                    }
                    None => spans.push((start..close + 2, WikiPart::Target)),
                }
                i = close + 2;
            }
            _ => i += 1,
        }
    }
    spans
}

/// The length of the run of `byte` starting at `at`.
fn run_len(bytes: &[u8], at: usize, byte: u8) -> usize {
    bytes[at..].iter().take_while(|&&b| b == byte).count()
}

/// The end of the next run of exactly `len` `byte`s at or after `from`.
fn find_run(bytes: &[u8], from: usize, byte: u8, len: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == byte {
            let run = run_len(bytes, i, byte);
            if run == len {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// The position of the next `needle` at or after `from`.
fn find_sub(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    (from..bytes.len().saturating_sub(needle.len() - 1))
        .find(|&i| &bytes[i..i + needle.len()] == needle)
}

/// A fenced code block's fence: the character, the run length, and
/// whether the fence line carries an info string.
struct Fence {
    character: char,
    length: usize,
    has_info: bool,
}

/// The fence line opening or closing a code block, if the line is one.
/// CommonMark allows up to three spaces of indentation.
fn fence_line(line: &str) -> Option<Fence> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let character = rest.chars().next()?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = run_len(rest.as_bytes(), 0, character as u8);
    if length < 3 {
        return None;
    }
    let info = rest[length..].trim();
    Some(Fence {
        character,
        length,
        // A backtick fence's info string may not contain a backtick.
        has_info: !info.is_empty() && !(character == '`' && info.contains('`')),
    })
}

/// Whether the highlighter is inside a fenced code block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FenceState {
    /// The open fence's character and run length.
    open: Option<(char, usize)>,
    /// Whether the state is known. After a jump the state at the new
    /// line can be unknown; a bare fence line then reads as the closer
    /// of a block whose opening was missed.
    unknown: bool,
}

impl FenceState {
    /// Feed one line and return the state after it.
    fn update(mut self, line: &str) -> Self {
        let Some(fence) = fence_line(line) else {
            return self;
        };
        match self.open {
            // A fence of the same character and at least the same length
            // closes the block.
            Some((character, length)) if character == fence.character && fence.length >= length => {
                self.open = None;
                self.unknown = false;
            }
            // Any other fence line is content of the open block.
            Some(_) => {}
            None => {
                // A fence line opens a block — unless the state is
                // unknown and the fence carries no info string: then it
                // is most likely the closer of a block we never saw
                // open.
                if fence.has_info || !self.unknown {
                    self.open = Some((fence.character, fence.length));
                    self.unknown = false;
                }
            }
        }
        self
    }
}

/// Tracks the fenced-code state across the lines the highlighter is fed.
#[derive(Debug, Default)]
struct FenceTracker {
    /// The state before the line being processed.
    state: FenceState,
    /// Checkpoints: the state before line `at`.
    checkpoints: Vec<(usize, FenceState)>,
}

impl FenceTracker {
    /// Feed the line at `index`; returns whether it is inside a fenced
    /// code block.
    fn is_code(&mut self, index: usize, line: &str) -> bool {
        // Checkpoint the state at every block boundary, before the line
        // changes it.
        let boundary = index - index % LINES_PER_CHECKPOINT;
        if index == boundary {
            self.checkpoints.retain(|(at, _)| *at < boundary);
            self.checkpoints.push((boundary, self.state));
        }
        self.state = self.state.update(line);
        self.state.open.is_some()
    }

    /// The highlighter jumps to `at`: restore the state there.
    fn jump(&mut self, at: usize) {
        self.state = match self.checkpoints.iter().find(|(line, _)| *line == at) {
            Some((_, state)) => *state,
            None => FenceState {
                open: None,
                unknown: true,
            },
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(line: &str) -> Vec<(Range<usize>, WikiPart)> {
        wikilink_spans(line)
    }

    fn text_of<'a>(line: &'a str, spans: &[(Range<usize>, WikiPart)]) -> Vec<(&'a str, WikiPart)> {
        spans
            .iter()
            .map(|(span, part)| (&line[span.clone()], *part))
            .collect()
    }

    #[test]
    fn a_plain_wikilink_is_one_target_range() {
        let line = "see [[Note]] here";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![("[[Note]]", WikiPart::Target)]
        );
    }

    #[test]
    fn the_bar_splits_the_target_from_the_label() {
        let line = "[[Some/Path|this text]]";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![
                ("[[Some/Path", WikiPart::Target),
                ("|this text]]", WikiPart::Alias),
            ]
        );
    }

    #[test]
    fn an_embed_keeps_its_bang() {
        let line = "![[image.png]]";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![("![[image.png]]", WikiPart::Target)]
        );
        // A doubled bang is not an embed marker.
        let line = "!![[Note]]";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![("[[Note]]", WikiPart::Target)]
        );
    }

    #[test]
    fn links_inside_inline_code_are_code() {
        let line = "`[[Note]]` and [[Other]]";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![("[[Other]]", WikiPart::Target)]
        );
        // A code span with two backticks holds a single one.
        let line = "``[[Note]]`` and [[Other]]";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![("[[Other]]", WikiPart::Target)]
        );
    }

    #[test]
    fn an_unclosed_link_is_not_a_link() {
        let line = "[[Note and [[Other]]";
        assert_eq!(
            text_of(line, &parts(line)),
            vec![("[[Other]]", WikiPart::Target)]
        );
    }

    #[test]
    fn fence_lines_open_and_close_code_blocks() {
        let fence = fence_line("```rust").expect("a fence line");
        assert_eq!(fence.character, '`');
        assert_eq!(fence.length, 3);
        assert!(fence.has_info);
        let fence = fence_line("```").expect("a bare fence line");
        assert!(!fence.has_info);
        assert!(
            fence_line("    ```").is_none(),
            "indented code is not a fence"
        );
        assert!(fence_line("``").is_none(), "two ticks are not a fence");
    }

    #[test]
    fn the_tracker_follows_fenced_code() {
        let mut tracker = FenceTracker::default();
        assert!(!tracker.is_code(0, "before"));
        // The opening fence and the block's content are code.
        assert!(tracker.is_code(1, "```rust"));
        assert!(tracker.is_code(2, "let x = 1;"));
        // The closing fence ends the block.
        assert!(!tracker.is_code(3, "```"));
        assert!(!tracker.is_code(4, "after"));
    }

    #[test]
    fn a_tilde_fence_is_its_own_block() {
        let mut tracker = FenceTracker::default();
        assert!(tracker.is_code(0, "~~~"));
        assert!(tracker.is_code(1, "content"));
        // A backtick fence does not close a tilde fence.
        assert!(tracker.is_code(2, "```"));
        assert!(tracker.is_code(3, "still code"));
        assert!(!tracker.is_code(4, "~~~"));
        assert!(!tracker.is_code(5, "after"));
    }

    #[test]
    fn a_jump_restores_the_state_at_the_line() {
        let mut tracker = FenceTracker::default();
        assert!(tracker.is_code(0, "```"));
        assert!(tracker.is_code(1, "code"));
        // The editor jumps back to line 0 (the changed line) and feeds
        // it again: the fence opens again.
        tracker.jump(0);
        assert!(tracker.is_code(0, "```"));
        assert!(tracker.is_code(1, "code"));
        assert!(!tracker.is_code(2, "```"));
    }

    #[test]
    fn the_wiki_ranges_win_over_the_markdown_ranges() {
        let line = "see [[Note]] here";
        let mut base =
            cosmic::iced::highlighter::Highlighter::new(&cosmic::iced::highlighter::Settings {
                theme: cosmic::iced::highlighter::Theme::Base16Ocean,
                token: "md".to_string(),
            });
        let spans: Vec<_> = base.highlight_line(line).collect();
        let merged = merge(spans, vec![(4..11, WikiPart::Target)]);
        // The ranges tile the line without gaps.
        let mut at = 0;
        for (range, _) in &merged {
            assert_eq!(range.start, at);
            at = range.end;
        }
        assert_eq!(at, line.len());
        // Exactly one range carries the wikilink colour.
        let marked: Vec<_> = merged
            .iter()
            .filter(|(_, highlight)| highlight.wiki.is_some())
            .collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].0, 4..11);
        assert_eq!(marked[0].1.wiki, Some(WikiPart::Target));
    }
}
