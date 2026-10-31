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
            '%' if starts(&chars, i, "%%") => {
                if let Some(j) = find_str(&chars, i + 2, "%%") {
                    i = j + 2;
                    continue;
                }
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
                if let Some(t) = idx.parse::<usize>().ok().and_then(|n| tokens.get(n)) {
                    flush(&mut buf, &mut out);
                    out.push(match t {
                        Token::Wiki(w) => Inline::WikiLink(w.clone()),
                        Token::Embed(e) => Inline::Embed(e.clone()),
                    });
                    i += rel + 1;
                    continue;
                }
            }
        }
        if c == '#' && tag_boundary(chars.get(i.wrapping_sub(1)).filter(|_| i > 0)) {
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
    prev.is_none_or(|p| !(p.is_alphanumeric() || matches!(p, '_' | '-' | '/' | '#')))
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

    #[test]
    fn comments_dropped_unterminated_kept() {
        assert_eq!(extract("a %%hidden [[x]]%% b").tokens.len(), 0);
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
}
