//! Obsidian-specific syntax that comrak doesn't know.
//!
//! Stage 1 (`extract`): runs on raw source BEFORE comrak. Pulls out `[[..]]`,
//! `![[..]]` and drops `%%comments%%`, skipping code. Each wikilink/embed is
//! replaced by an opaque sentinel so comrak can't split it (e.g. `[[a *b* c]]`).
//! Stage 2 (`lower_text`): runs on comrak Text nodes. Swaps sentinels back to
//! IR inlines and recognizes `#tags`.
//!
//! For whole documents prefer [`extract_document`], which splits YAML
//! frontmatter off FIRST so properties like `related: "[[Note]]"` are never
//! scanned for links (HANDOFF 4.3: definite gap).
use vault_core::split_frontmatter;

use crate::ir::*;

const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Wiki(WikiLink),
    Embed(Embed),
    /// Characters that must survive as literal text: an escaped `\#` is not
    /// a tag, and comrak would consume the backslash before `lower_text` ran.
    Literal(String),
}

#[derive(Debug)]
pub struct Extracted {
    pub text: String,
    pub tokens: Vec<Token>,
}

/// Result of [`extract_document`]: frontmatter bytes verbatim plus the
/// extraction of the body only.
#[derive(Debug)]
pub struct ExtractedDoc {
    /// Full frontmatter block including delimiters, byte-for-byte, or `None`.
    pub frontmatter_raw: Option<String>,
    pub extracted: Extracted,
}

/// Extract links/embeds from a full document, frontmatter excluded.
pub fn extract_document(src: &str) -> ExtractedDoc {
    let split = split_frontmatter(src);
    let extracted = extract(&split.body);
    ExtractedDoc {
        frontmatter_raw: split.raw,
        extracted,
    }
}

pub fn extract(src: &str) -> Extracted {
    let chars: Vec<char> = src.chars().filter(|c| *c != OPEN && *c != CLOSE).collect();
    let code_mask = indented_code_mask(&chars);
    let (mut out, mut tokens) = (String::new(), Vec::new());
    let (mut i, mut at_line_start) = (0, true);
    while i < chars.len() {
        // top-level indented code block: copy verbatim (HANDOFF 4.3: definite
        // gap — wikilinks inside indented code must not become tokens)
        if code_mask[i] {
            let c = chars[i];
            out.push(c);
            at_line_start = c == '\n';
            i += 1;
            continue;
        }
        // Confirmed against Obsidian (M3): wikilinks inside raw HTML blocks
        // are literal. A line starting with '<' + letter, '/', '!' or '?'
        // opens a CommonMark HTML block; copy lines until a blank line.
        if at_line_start && chars[i] == '<' {
            let opens_html = chars
                .get(i + 1)
                .is_some_and(|&n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?'));
            if opens_html {
                let mut j = i;
                while j < chars.len() {
                    let line_end = chars[j..]
                        .iter()
                        .position(|&x| x == '\n')
                        .map_or(chars.len(), |p| j + p);
                    let blank = chars[j..line_end]
                        .iter()
                        .all(|&x| matches!(x, ' ' | '\t' | '\r'));
                    if blank {
                        break;
                    }
                    j = if line_end < chars.len() {
                        line_end + 1
                    } else {
                        line_end
                    };
                }
                out.extend(&chars[i..j]);
                i = j;
                at_line_start = true;
                continue;
            }
        }
        // fenced code block: copy verbatim through the closing fence
        if at_line_start {
            if let Some((fc, flen)) = fence_at(&chars, i) {
                let end = fence_end(&chars, i, fc, flen);
                out.extend(&chars[i..end]);
                i = end;
                at_line_start = true;
                continue;
            }
        }
        let c = chars[i];
        match c {
            '`' => {
                let n = run_len(&chars, i, '`');
                match find_run(&chars, i + n, '`', n) {
                    Some(j) => {
                        out.extend(&chars[i..j + n]);
                        i = j + n;
                    }
                    None => {
                        out.extend(&chars[i..i + n]);
                        i += n;
                    }
                }
                at_line_start = false;
                continue;
            }
            '\\' if starts(&chars, i, "\\#") => {
                tokens.push(Token::Literal("#".into()));
                out.push_str(&format!("{OPEN}l{}{CLOSE}", tokens.len() - 1));
                i += 2;
                at_line_start = false;
                continue;
            }
            // Confirmed against Obsidian (M3): \\[[x]] does NOT render as a
            // link — pass the escape through verbatim so comrak unescapes it.
            '\\' => {
                out.push(c);
                if let Some(&next) = chars.get(i + 1) {
                    out.push(next);
                }
                i += 2;
                at_line_start = false;
                continue;
            }
            // Confirmed against Obsidian (M3): wikilinks inside $math$ and
            // $$display$$ are NOT links — copy math spans verbatim (comrak
            // still parses them as math).
            '$' if starts(&chars, i, "$$") => {
                match find_str(&chars, i + 2, "$$") {
                    Some(end) => {
                        out.extend(&chars[i..end + 2]);
                        i = end + 2;
                    }
                    // Unterminated `$$`: literal dollars.
                    None => {
                        out.push('$');
                        i += 1;
                    }
                }
                at_line_start = false;
                continue;
            }
            '$' => {
                // The closing `$` must not follow whitespace, which keeps
                // "$5 and $10" literal (currency) and mirrors comrak's
                // math_dollars rules.
                let mut j = i + 1;
                let mut closer = None;
                while j < chars.len() {
                    if chars[j] == '$' && j > i + 1 && !matches!(chars[j - 1], ' ' | '\t' | '\n') {
                        closer = Some(j);
                        break;
                    }
                    // A blank line ends the hunt: unterminated math.
                    if chars[j] == '\n' && matches!(chars.get(j + 1), Some('\n') | Some('\r')) {
                        break;
                    }
                    j += 1;
                }
                match closer {
                    Some(end) => {
                        out.extend(&chars[i..=end]);
                        i = end + 1;
                    }
                    None => {
                        out.push('$');
                        i += 1;
                    }
                }
                at_line_start = false;
                continue;
            }
            '%' => {
                // `%%comments%%` are comrak's job (ofm_comments → CommentInline
                // nodes, dropped at lowering). The pre-pass must NOT delete
                // them: deleting text can collapse block structure, and a
                // comment node keeps the literal verbatim so wikilinks inside
                // comments are never interpreted.
            }
            '!' if starts(&chars, i, "![[") => {
                if let Some((inner, end)) = wiki_inner(&chars, i + 3) {
                    tokens.push(Token::Embed(parse_embed(&inner)));
                    out.push_str(&format!("{OPEN}{}{CLOSE}", tokens.len() - 1));
                    i = end;
                    at_line_start = false;
                    continue;
                }
            }
            '[' if starts(&chars, i, "[[") => {
                if let Some((inner, end)) = wiki_inner(&chars, i + 2) {
                    tokens.push(Token::Wiki(parse_wiki(&inner)));
                    out.push_str(&format!("{OPEN}{}{CLOSE}", tokens.len() - 1));
                    i = end;
                    at_line_start = false;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        at_line_start = c == '\n';
        i += 1;
    }
    Extracted { text: out, tokens }
}

pub fn lower_text(text: &str, tokens: &[Token]) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut buf) = (Vec::new(), String::new());
    let mut i = 0;
    let flush = |buf: &mut String, out: &mut Vec<Inline>| {
        if !buf.is_empty() {
            out.push(Inline::Text(std::mem::take(buf)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == OPEN {
            if let Some(rel) = chars[i..].iter().position(|&x| x == CLOSE) {
                let idx: String = chars[i + 1..i + rel].iter().collect();
                // `l<idx>` marks a literal token (escaped characters).
                let token = if let Some(num) = idx.strip_prefix('l') {
                    num.parse::<usize>().ok().and_then(|n| tokens.get(n))
                } else {
                    idx.parse::<usize>().ok().and_then(|n| tokens.get(n))
                };
                if let Some(t) = token {
                    flush(&mut buf, &mut out);
                    out.push(match t {
                        Token::Wiki(w) => Inline::WikiLink(w.clone()),
                        Token::Embed(e) => Inline::Embed(e.clone()),
                        Token::Literal(s) => Inline::Text(s.clone()),
                    });
                    i += rel + 1;
                    continue;
                }
            }
        }
        if c == '#'
            && chars.get(i.wrapping_sub(1)) != Some(&'\\')
            && tag_boundary(chars.get(i.wrapping_sub(1)).filter(|_| i > 0))
        {
            let body: String = chars[i + 1..]
                .iter()
                .take_while(|&&x| is_tag_char(x))
                .collect();
            let trimmed = body.trim_end_matches('/');
            if !trimmed.is_empty() && trimmed.chars().any(|x| !x.is_ascii_digit()) {
                flush(&mut buf, &mut out);
                out.push(Inline::Tag(trimmed.to_string()));
                i += 1 + trimmed.chars().count();
                continue;
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut out);
    out
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}
fn tag_boundary(prev: Option<&char>) -> bool {
    prev.is_none_or(|p| !(p.is_alphanumeric() || matches!(p, '_' | '-' | '/' | '#' | '(' | ')')))
}

pub fn parse_target(s: &str) -> WikiTarget {
    match s.split_once('#') {
        None => WikiTarget {
            note: s.trim().into(),
            ..Default::default()
        },
        Some((note, rest)) => {
            let note = note.trim().to_string();
            match rest.strip_prefix('^') {
                Some(b) => WikiTarget {
                    note,
                    block: Some(b.into()),
                    heading: None,
                },
                None => WikiTarget {
                    note,
                    heading: Some(rest.into()),
                    block: None,
                },
            }
        }
    }
}

fn split_alias(inner: &str) -> (String, Option<String>) {
    // first unescaped '|' ; `\|` is the table-safe spelling
    let b: Vec<char> = inner.chars().collect();
    for i in 0..b.len() {
        if b[i] == '|' {
            let t: String = b[..i].iter().collect();
            let a: String = b[i + 1..].iter().collect();
            return (t.trim_end_matches('\\').to_string(), Some(a));
        }
    }
    (inner.to_string(), None)
}

pub fn parse_wiki(inner: &str) -> WikiLink {
    let (t, alias) = split_alias(inner);
    WikiLink {
        target: parse_target(&t),
        alias,
    }
}

pub fn parse_embed(inner: &str) -> Embed {
    let (t, alias) = split_alias(inner);
    let size = alias.as_deref().and_then(parse_size);
    let alias = if size.is_some() { None } else { alias };
    Embed {
        target: parse_target(&t),
        size,
        alias,
    }
}

fn parse_size(s: &str) -> Option<(u32, Option<u32>)> {
    let (w, h) = match s.split_once('x') {
        Some((w, h)) => (w, Some(h)),
        None => (s, None),
    };
    let w = w.parse().ok()?;
    let h = match h {
        Some(h) => Some(h.parse().ok()?),
        None => None,
    };
    Some((w, h))
}

// ---- tiny scanners over Vec<char> ----
fn starts(c: &[char], i: usize, p: &str) -> bool {
    p.chars()
        .enumerate()
        .all(|(k, pc)| c.get(i + k) == Some(&pc))
}
fn find_str(c: &[char], from: usize, p: &str) -> Option<usize> {
    (from..c.len()).find(|&i| starts(c, i, p))
}
fn run_len(c: &[char], i: usize, ch: char) -> usize {
    c[i..].iter().take_while(|&&x| x == ch).count()
}
fn find_run(c: &[char], mut i: usize, ch: char, n: usize) -> Option<usize> {
    while i < c.len() {
        if c[i] == ch {
            let r = run_len(c, i, ch);
            if r == n {
                return Some(i);
            }
            i += r;
        } else {
            i += 1;
        }
    }
    None
}
fn wiki_inner(c: &[char], from: usize) -> Option<(String, usize)> {
    let end = find_str(c, from, "]]")?;
    let inner: String = c[from..end].iter().collect();
    if inner.is_empty() || inner.contains('\n') || inner.contains("[[") {
        return None;
    }
    Some((inner, end + 2))
}
fn fence_at(c: &[char], i: usize) -> Option<(char, usize)> {
    let mut k = i;
    while k < c.len() && k - i < 3 && c[k] == ' ' {
        k += 1;
    }
    let ch = *c.get(k)?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = run_len(c, k, ch);
    (n >= 3).then_some((ch, n))
}
fn fence_end(c: &[char], start: usize, fc: char, flen: usize) -> usize {
    let mut i = start;
    while i < c.len() && c[i] != '\n' {
        i += 1;
    } // skip opening line
    while i < c.len() {
        i += 1; // line start
        if let Some((ch, n)) = fence_at(c, i) {
            if ch == fc && n >= flen {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
                return i;
            }
        }
        while i < c.len() && c[i] != '\n' {
            i += 1;
        }
    }
    c.len()
}

/// Per-char mask of top-level indented code blocks (CommonMark): a block can
/// only start after a blank line (indent >= 4 spaces or a tab cannot interrupt
/// a paragraph) and continues while lines are blank or indented >= 4.
///
/// KNOWN LIMITATION (HANDOFF 4.3): list-item continuation is not modeled, so
/// a 4-space-indented line inside a list item after a blank line is treated
/// as top-level indented code. Whether Obsidian agrees must be confirmed in
/// M3 against a real vault before this is relied on.
fn indented_code_mask(chars: &[char]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let (mut prev_blank, mut in_code) = (true, false);
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && chars[i] != '\n' {
            i += 1;
        }
        let has_nl = i < chars.len();
        let line = &chars[start..i];
        let indent = line.iter().take_while(|&&c| c == ' ').count();
        let blank = line.iter().all(|&c| c == ' ' || c == '\t' || c == '\r');
        let indented = indent >= 4 || line.first() == Some(&'\t');
        let is_code_line = if in_code {
            blank || indented
        } else {
            prev_blank && !blank && indented
        };
        if is_code_line {
            in_code = true;
            for slot in mask.iter_mut().take(i).skip(start) {
                *slot = true;
            }
            if has_nl {
                mask[i] = true;
            }
        } else {
            in_code = false;
        }
        prev_blank = blank;
        if has_nl {
            i += 1;
        }
    }
    mask
}

// --- link-target spans (rename/link rewriting) ------------------------------

/// A wikilink or embed target found in a document body, as byte ranges into
/// the FULL source passed to [`link_spans`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// Byte range of the note part of the target — between the `[[` /
    /// `![[` opener and the first `#` (heading/block) or `|` (alias).
    /// Rewriting exactly this span preserves any `#...`/`|...` suffix.
    pub target: (usize, usize),
    /// The raw target text, trimmed the way resolution would see it.
    pub raw: String,
    /// `true` for `![[...]]` embeds, `false` for `[[...]]` links.
    pub embed: bool,
}

/// Scan a full document for wikilink/embed note targets, recording byte
/// spans. Frontmatter, code (fenced, indented, inline), math and raw HTML
/// blocks are skipped exactly like [`extract`] skips them, so a link that
/// does not render is never rewritten either. `[[#Heading]]` (empty note
/// part) is not reported — there is nothing to rewrite.
pub fn link_spans(src: &str) -> Vec<LinkSpan> {
    // Frontmatter is split off first (like `extract_document`), so
    // `related: "[[Note]]"` properties are never rewritten. All spans are
    // byte offsets into the FULL source, hence the body offset.
    let split = split_frontmatter(src);
    let offset = split.bom.len() + split.raw.as_deref().map_or(0, str::len);
    // Unlike `extract`, no sentinel chars are stripped: the offsets must
    // be exact for byte-range rewriting of the original text.
    let src_body = &src[offset.min(src.len())..];
    let chars: Vec<char> = src_body.chars().collect();
    // Byte offset of each char index; the final entry is `src.len()`.
    let mut byte_of = Vec::with_capacity(chars.len() + 1);
    let mut b = 0usize;
    for &c in &chars {
        byte_of.push(b);
        b += c.len_utf8();
    }
    byte_of.push(b);

    let code_mask = indented_code_mask(&chars);
    let mut spans = Vec::new();
    let mut i = 0usize;
    let mut at_line_start = true;
    while i < chars.len() {
        if code_mask[i] {
            at_line_start = chars[i] == '\n';
            i += 1;
            continue;
        }
        if at_line_start && chars[i] == '<' {
            let opens_html = chars
                .get(i + 1)
                .is_some_and(|&n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?'));
            if opens_html {
                let mut j = i;
                while j < chars.len() {
                    let line_end = chars[j..]
                        .iter()
                        .position(|&x| x == '\n')
                        .map_or(chars.len(), |p| j + p);
                    let blank = chars[j..line_end]
                        .iter()
                        .all(|&x| matches!(x, ' ' | '\t' | '\r'));
                    if blank {
                        break;
                    }
                    j = if line_end < chars.len() {
                        line_end + 1
                    } else {
                        line_end
                    };
                }
                i = j;
                at_line_start = true;
                continue;
            }
        }
        if at_line_start {
            if let Some((fc, flen)) = fence_at(&chars, i) {
                i = fence_end(&chars, i, fc, flen);
                at_line_start = true;
                continue;
            }
        }
        let c = chars[i];
        match c {
            '`' => {
                let n = run_len(&chars, i, '`');
                i = match find_run(&chars, i + n, '`', n) {
                    Some(j) => j + n,
                    None => i + n,
                };
                at_line_start = false;
                continue;
            }
            // `\[[x]]` is not a link (confirmed M3); escapes never open one.
            '\\' => {
                i = (i + 2).min(chars.len());
                at_line_start = false;
                continue;
            }
            '$' if starts(&chars, i, "$$") => {
                i = match find_str(&chars, i + 2, "$$") {
                    Some(end) => end + 2,
                    None => i + 1,
                };
                at_line_start = false;
                continue;
            }
            '$' => {
                let mut j = i + 1;
                let mut closer = None;
                while j < chars.len() {
                    if chars[j] == '$' && j > i + 1 && !matches!(chars[j - 1], ' ' | '\t' | '\n') {
                        closer = Some(j);
                        break;
                    }
                    if chars[j] == '\n' && matches!(chars.get(j + 1), Some('\n') | Some('\r')) {
                        break;
                    }
                    j += 1;
                }
                i = match closer {
                    Some(end) => end + 1,
                    None => i + 1,
                };
                at_line_start = false;
                continue;
            }
            '!' if starts(&chars, i, "![[") => {
                if let Some((inner, end)) = wiki_inner(&chars, i + 3) {
                    push_link_span(&mut spans, &byte_of, offset, i, &inner, true);
                    i = end;
                    at_line_start = false;
                    continue;
                }
                at_line_start = chars[i] == '\n';
                i += 1;
            }
            '[' if starts(&chars, i, "[[") => {
                if let Some((inner, end)) = wiki_inner(&chars, i + 2) {
                    push_link_span(&mut spans, &byte_of, offset, i, &inner, false);
                    i = end;
                    at_line_start = false;
                    continue;
                }
                at_line_start = chars[i] == '\n';
                i += 1;
            }
            _ => {
                at_line_start = c == '\n';
                i += 1;
            }
        }
    }
    spans
}

/// Record the note part of one `[[...]]`/`![[...]]`: from just after the
/// opener to the first `#` (heading/block) or `|` (alias) — the same
/// target/alias split `parse_wiki` performs. `offset` maps body-relative
/// char indices back to full-source byte offsets.
fn push_link_span(
    spans: &mut Vec<LinkSpan>,
    byte_of: &[usize],
    offset: usize,
    link_start: usize,
    inner: &str,
    embed: bool,
) {
    let inner_start = link_start + if embed { 3 } else { 2 };
    let inner_chars: Vec<char> = inner.chars().collect();
    let note_len = inner_chars
        .iter()
        .position(|&c| c == '#' || c == '|')
        .unwrap_or(inner_chars.len());
    if note_len == 0 {
        return; // `[[#Heading]]`: same-document, nothing to rewrite.
    }
    let raw: String = inner_chars[..note_len].iter().collect();
    spans.push(LinkSpan {
        target: (
            offset + byte_of[inner_start],
            offset + byte_of[inner_start + note_len],
        ),
        raw: raw.trim().to_string(),
        embed,
    });
}

/// Rewrite the note part of every wikilink/embed target for which
/// `resolver` returns `Some(replacement)`. Returns `None` when nothing
/// changed, so callers can skip the write entirely.
pub fn rewrite_link_targets(
    src: &str,
    resolver: impl Fn(&LinkSpan) -> Option<String>,
) -> Option<String> {
    let spans = link_spans(src);
    let mut out = String::with_capacity(src.len());
    let mut last = 0usize;
    let mut changed = false;
    for span in &spans {
        let Some(replacement) = resolver(span) else {
            continue;
        };
        let (start, end) = span.target;
        out.push_str(&src[last..start]);
        out.push_str(&replacement);
        last = end;
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(&src[last..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lower(s: &str) -> Vec<Inline> {
        let e = extract(s);
        lower_text(&e.text, &e.tokens)
    }
    fn tags(v: &[Inline]) -> Vec<&str> {
        v.iter()
            .filter_map(|i| {
                if let Inline::Tag(t) = i {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn wikilink_forms() {
        let w = parse_wiki("Note#Heading|shown");
        assert_eq!(w.target.note, "Note");
        assert_eq!(w.target.heading.as_deref(), Some("Heading"));
        assert_eq!(w.alias.as_deref(), Some("shown"));
        let b = parse_wiki("Note#^abc123");
        assert_eq!(b.target.block.as_deref(), Some("abc123"));
        assert_eq!(parse_wiki("#Local").target.note, "");
        assert_eq!(parse_wiki("a\\|b").alias.as_deref(), Some("b")); // table-escaped pipe
    }

    #[test]
    fn embed_size_vs_alt() {
        let e = parse_embed("img.png|300");
        assert_eq!(e.size, Some((300, None)));
        assert_eq!(e.alias, None);
        assert_eq!(parse_embed("img.png|300x200").size, Some((300, Some(200))));
        assert_eq!(parse_embed("img.png|a cat").alias.as_deref(), Some("a cat"));
    }

    #[test]
    fn emphasis_inside_wikilink_cannot_split_it() {
        let e = extract("see [[a *b* c]] and [[Note|*alias*]]");
        assert!(!e.text.contains("[[")); // both hidden from comrak
        assert_eq!(e.tokens.len(), 2);
        let out = lower_text(&e.text, &e.tokens);
        assert!(matches!(out[1], Inline::WikiLink(_)));
    }

    #[test]
    fn code_is_untouched() {
        let src = "`[[no]]` and\n```\n[[no]] #no %%no%%\n```\nbut [[yes]]";
        let e = extract(src);
        assert_eq!(e.tokens.len(), 1);
        assert!(e.text.contains("`[[no]]`") && e.text.contains("%%no%%"));
        let tilde = extract("~~~\n[[no]]\n~~~\n[[yes]]");
        assert_eq!(tilde.tokens.len(), 1);
    }

    /// Confirmed against real Obsidian (M3, 2026-10-31): an escaped
    /// wikilink is NOT a link — it renders as literal text.
    #[test]
    fn escaped_wikilink_is_not_a_link() {
        let e = extract("literal \\[[not a link]] here");
        assert_eq!(e.tokens.len(), 0, "escaped link must not become a token");
        assert!(e.text.contains("\\[[not a link]]"));
    }

    /// Confirmed against real Obsidian (M3): no links inside math.
    #[test]
    fn no_links_inside_math() {
        let e = extract("inline $a[[b]]c$ math");
        assert_eq!(e.tokens.len(), 0);
        assert!(e.text.contains("$a[[b]]c$"));
        let d = extract("$$\na[[b]]c\n$$");
        assert_eq!(d.tokens.len(), 0);
        assert!(d.text.contains("a[[b]]c"));
    }

    /// Currency stays text: the closing `$` follows whitespace.
    #[test]
    fn currency_dollars_are_literal() {
        let e = extract("Costs $5 and $10 each.");
        assert_eq!(e.tokens.len(), 0);
        assert_eq!(e.text, "Costs $5 and $10 each.");
    }

    /// Confirmed against real Obsidian (M3): raw HTML blocks are literal.
    #[test]
    fn html_blocks_are_literal() {
        let e = extract("<div>[[x]]</div>\n\nbut [[yes]]");
        assert_eq!(e.tokens.len(), 1, "only the wikilink outside the block");
        assert!(e.text.contains("<div>[[x]]</div>"));
    }

    /// Confirmed against real Obsidian (M3): a `#` directly after an open
    /// or close paren does NOT start a tag.
    #[test]
    fn tag_after_paren_is_not_a_tag() {
        assert!(tags(&lower("(#tag) text")).is_empty());
        assert!(tags(&lower("word)#tag")).is_empty());
        // a tag after whitespace or other punctuation is still a tag
        assert_eq!(tags(&lower("word #tag")), ["tag"]);
        assert_eq!(tags(&lower("word, #tag")), ["tag"]);
        // a hyphen IS a tag character, so a hash after it is mid-word
        assert!(tags(&lower("word-#tag")).is_empty());
    }

    #[test]
    fn comments_pass_through_to_comrak() {
        // Comments are comrak's job now (ofm_comments); the pre-pass leaves
        // them in place so block structure survives, and lowering drops the
        // comment nodes (see lower tests). An unterminated `%%` is literal.
        let e = extract("a %%hidden [[x]]%% b");
        assert_eq!(e.text, "a %%hidden \u{e000}0\u{e001}%% b"); // wikilink sentinelized, comment kept
        assert_eq!(extract("a %%hidden b").text, "a %%hidden b");
    }

    #[test]
    fn indented_code_is_untouched() {
        // HANDOFF 4.3 definite gap: 4-space indented block after a blank line.
        let e = extract("para\n\n    [[in-code]]\n");
        assert_eq!(e.tokens.len(), 0);
        assert!(e.text.contains("[[in-code]]"));
        // tab-indented too
        assert_eq!(extract("\t[[in-code]]\n").tokens.len(), 0);
        // 4-space indent cannot interrupt a paragraph: still code-skipped? No —
        // it is a lazy continuation, so the link is real text there.
        let e2 = extract("para\n    [[lazy]]\n");
        assert_eq!(e2.tokens.len(), 1);
        // blank line inside the block keeps it code
        let e3 = extract("a\n\n    [[x]]\n\n    [[y]]\n\n[[z]]");
        assert_eq!(e3.tokens.len(), 1);
    }

    #[test]
    fn frontmatter_is_never_scanned() {
        // HANDOFF 4.3 definite gap: Obsidian properties often hold quoted
        // "[[Note]]"; extraction must run on the body only.
        let src = "---\nrelated: \"[[Note]]\"\ntags: a\n---\nbody [[Real]]\n";
        let doc = extract_document(src);
        assert_eq!(doc.extracted.tokens.len(), 1);
        assert!(matches!(doc.extracted.tokens[0], Token::Wiki(_)));
        assert_eq!(
            doc.frontmatter_raw.as_deref(),
            Some("---\nrelated: \"[[Note]]\"\ntags: a\n---\n")
        );
        // no frontmatter: raw is None, extraction still works
        let doc2 = extract_document("[[X]]");
        assert!(doc2.frontmatter_raw.is_none());
        assert_eq!(doc2.extracted.tokens.len(), 1);
    }

    #[test]
    fn tag_rules() {
        assert_eq!(
            tags(&lower("a #tag and #nested/tag, #y1984.")),
            ["tag", "nested/tag", "y1984"]
        );
        assert!(tags(&lower("year #1984 here")).is_empty()); // all-numeric
        assert!(tags(&lower("foo#bar")).is_empty()); // no boundary
        assert!(tags(&lower("# heading-ish")).is_empty()); // '#' + space
        assert_eq!(tags(&lower("#a-b_c")), ["a-b_c"]);
        assert_eq!(tags(&lower("#tag/")), ["tag"]);
    }

    #[test]
    fn extract_preserves_line_structure() {
        // comrak's sourcepos (block spans) is only meaningful if extraction
        // never adds or removes lines. Sentinels replace same-line spans;
        // multi-line `[[...]]` is rejected by wiki_inner and copied
        // verbatim, so every path keeps the line count and boundaries.
        let cases = [
            "plain text\n",
            "[[Note]] and ![[Embed.png|300]]\n",
            "[[Note\nspanning lines]] stays literal\n",
            "\\# not a tag\n",
            "%% a comment %%\n\nsecond para\n",
            "```rust\n[[in fence]]\n```\n",
            "\t[[indented code]]\n",
            "<div>\n[[in html]]\n</div>\n",
            "$x [[in math]] y$\n\n$$\n[[display]]\n$$\n",
            "- list with [[link]]\n- [ ] task\n",
        ];
        for src in cases {
            let extracted = extract(src);
            assert_eq!(
                extracted.text.lines().count(),
                src.lines().count(),
                "line count changed for {src:?}"
            );
        }
    }

    /// The raw target text of every span, for readable assertions.
    fn raws(src: &str) -> Vec<String> {
        link_spans(src).into_iter().map(|s| s.raw).collect()
    }

    #[test]
    fn spans_cover_wikilinks_embeds_and_suffixes() {
        // Plain link and embed.
        assert_eq!(raws("see [[Note]] here"), vec!["Note"]);
        assert_eq!(raws("![[img.png|300]]"), vec!["img.png"]);
        // The note part ends at the first `#` or `|`; suffixes stay out.
        assert_eq!(raws("[[Note#Head]]"), vec!["Note"]);
        assert_eq!(raws("[[Note|alias]]"), vec!["Note"]);
        assert_eq!(raws("[[Note#Head|alias]]"), vec!["Note"]);
        assert_eq!(raws("[[Note#^bid]]"), vec!["Note"]);
        // Same-document links have no note part to rewrite.
        assert!(raws("[[#Head]] and [[#^bid]]").is_empty());
        // Several on one line.
        assert_eq!(raws("[[A]] then [[B]]"), vec!["A", "B"]);
        // Folder targets and surrounding spaces are trimmed for resolution.
        assert_eq!(raws("[[ folder/Note ]]"), vec!["folder/Note"]);
    }

    #[test]
    fn spans_skip_code_math_html_and_frontmatter() {
        assert!(raws("```rust\n[[in fence]]\n```").is_empty());
        assert!(raws("\t[[indented code]]").is_empty());
        assert!(raws("`[[inline code]]`").is_empty());
        assert!(raws("$x [[in math]] y$").is_empty());
        assert!(raws("<div>\n[[in html]]\n</div>").is_empty());
        assert!(raws("\\[[escaped]]").is_empty());
        assert!(raws("---\nrelated: \"[[FM]]\"\n---\n[[body]]").len() == 1);
        assert_eq!(raws("---\nrelated: \"[[FM]]\"\n---\n[[body]]")[0], "body");
    }

    #[test]
    fn spans_are_byte_offsets_into_the_full_source() {
        // Multibyte characters before the link must not shift the span.
        let src = "héllo wörld — see [[Note]] now";
        let spans = link_spans(src);
        assert_eq!(spans.len(), 1);
        let (start, end) = spans[0].target;
        assert_eq!(&src[start..end], "Note");
    }

    #[test]
    fn rewrite_touches_only_resolved_targets() {
        let src = "a [[Old]] b [[Old#Head|alias]] c [[Stay]] d ![[Old]]";
        let out = rewrite_link_targets(src, |span| (span.raw == "Old").then(|| "New".to_string()))
            .expect("something changed");
        assert_eq!(out, "a [[New]] b [[New#Head|alias]] c [[Stay]] d ![[New]]");
        // Nothing to do → None (no write).
        assert!(rewrite_link_targets("no links", |_| None).is_none());
        assert!(rewrite_link_targets("[[Other]]", |_| None).is_none());
    }

    #[test]
    fn rewrite_keeps_byte_accuracy_across_multibyte_text() {
        let src = "ünïcodé [[A]] and [[B]]";
        let out =
            rewrite_link_targets(src, |span| (span.raw == "B").then(|| "x/y".to_string())).unwrap();
        assert_eq!(out, "ünïcodé [[A]] and [[x/y]]");
    }
}
