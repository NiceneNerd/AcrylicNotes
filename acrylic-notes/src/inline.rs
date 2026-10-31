//! Obsidian-specific syntax that comrak doesn't know.
//!
//! Stage 1 (`extract`): runs on raw source BEFORE comrak. Pulls out `[[..]]`,
//! `![[..]]` and drops `%%comments%%`, skipping code. Each wikilink/embed is
//! replaced by an opaque sentinel so comrak can't split it (e.g. `[[a *b* c]]`).
//! Stage 2 (`lower_text`): runs on comrak Text nodes. Swaps sentinels back to
//! IR inlines and recognizes `#tags`.
use crate::ir::*;

const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

#[derive(Debug, Clone, PartialEq)]
pub enum Token { Wiki(WikiLink), Embed(Embed) }

#[derive(Debug)]
pub struct Extracted { pub text: String, pub tokens: Vec<Token> }

pub fn extract(src: &str) -> Extracted {
    let chars: Vec<char> = src.chars().filter(|c| *c != OPEN && *c != CLOSE).collect();
    let (mut out, mut tokens) = (String::new(), Vec::new());
    let (mut i, mut at_line_start) = (0, true);
    while i < chars.len() {
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
                    Some(j) => { out.extend(&chars[i..j + n]); i = j + n; }
                    None => { out.extend(&chars[i..i + n]); i += n; }
                }
                at_line_start = false;
                continue;
            }
            '%' if starts(&chars, i, "%%") => {
                if let Some(j) = find_str(&chars, i + 2, "%%") { i = j + 2; continue; }
            }
            '!' if starts(&chars, i, "![[") => {
                if let Some((inner, end)) = wiki_inner(&chars, i + 3) {
                    tokens.push(Token::Embed(parse_embed(&inner)));
                    out.push_str(&format!("{OPEN}{}{CLOSE}", tokens.len() - 1));
                    i = end; at_line_start = false; continue;
                }
            }
            '[' if starts(&chars, i, "[[") => {
                if let Some((inner, end)) = wiki_inner(&chars, i + 2) {
                    tokens.push(Token::Wiki(parse_wiki(&inner)));
                    out.push_str(&format!("{OPEN}{}{CLOSE}", tokens.len() - 1));
                    i = end; at_line_start = false; continue;
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
        if !buf.is_empty() { out.push(Inline::Text(std::mem::take(buf))); }
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
            let body: String = chars[i + 1..].iter().take_while(|&&x| is_tag_char(x)).collect();
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

fn is_tag_char(c: char) -> bool { c.is_alphanumeric() || matches!(c, '_' | '-' | '/') }
fn tag_boundary(prev: Option<&char>) -> bool {
    prev.map_or(true, |p| !(p.is_alphanumeric() || matches!(p, '_' | '-' | '/' | '#')))
}

pub fn parse_target(s: &str) -> WikiTarget {
    match s.split_once('#') {
        None => WikiTarget { note: s.trim().into(), ..Default::default() },
        Some((note, rest)) => {
            let note = note.trim().to_string();
            match rest.strip_prefix('^') {
                Some(b) => WikiTarget { note, block: Some(b.into()), heading: None },
                None => WikiTarget { note, heading: Some(rest.into()), block: None },
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
    WikiLink { target: parse_target(&t), alias }
}

pub fn parse_embed(inner: &str) -> Embed {
    let (t, alias) = split_alias(inner);
    let size = alias.as_deref().and_then(parse_size);
    let alias = if size.is_some() { None } else { alias };
    Embed { target: parse_target(&t), size, alias }
}

fn parse_size(s: &str) -> Option<(u32, Option<u32>)> {
    let (w, h) = match s.split_once('x') { Some((w, h)) => (w, Some(h)), None => (s, None) };
    let w = w.parse().ok()?;
    let h = match h { Some(h) => Some(h.parse().ok()?), None => None };
    Some((w, h))
}

// ---- tiny scanners over Vec<char> ----
fn starts(c: &[char], i: usize, p: &str) -> bool {
    p.chars().enumerate().all(|(k, pc)| c.get(i + k) == Some(&pc))
}
fn find_str(c: &[char], from: usize, p: &str) -> Option<usize> {
    (from..c.len()).find(|&i| starts(c, i, p))
}
fn run_len(c: &[char], i: usize, ch: char) -> usize { c[i..].iter().take_while(|&&x| x == ch).count() }
fn find_run(c: &[char], mut i: usize, ch: char, n: usize) -> Option<usize> {
    while i < c.len() {
        if c[i] == ch { let r = run_len(c, i, ch); if r == n { return Some(i); } i += r; } else { i += 1; }
    }
    None
}
fn wiki_inner(c: &[char], from: usize) -> Option<(String, usize)> {
    let end = find_str(c, from, "]]")?;
    let inner: String = c[from..end].iter().collect();
    if inner.is_empty() || inner.contains('\n') || inner.contains("[[") { return None; }
    Some((inner, end + 2))
}
fn fence_at(c: &[char], i: usize) -> Option<(char, usize)> {
    let mut k = i;
    while k < c.len() && k - i < 3 && c[k] == ' ' { k += 1; }
    let ch = *c.get(k)?;
    if ch != '`' && ch != '~' { return None; }
    let n = run_len(c, k, ch);
    (n >= 3).then_some((ch, n))
}
fn fence_end(c: &[char], start: usize, fc: char, flen: usize) -> usize {
    let mut i = start;
    while i < c.len() && c[i] != '\n' { i += 1; } // skip opening line
    while i < c.len() {
        i += 1; // line start
        if let Some((ch, n)) = fence_at(c, i) { if ch == fc && n >= flen {
            while i < c.len() && c[i] != '\n' { i += 1; }
            return i;
        } }
        while i < c.len() && c[i] != '\n' { i += 1; }
    }
    c.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lower(s: &str) -> Vec<Inline> { let e = extract(s); lower_text(&e.text, &e.tokens) }
    fn tags(v: &[Inline]) -> Vec<&str> {
        v.iter().filter_map(|i| if let Inline::Tag(t) = i { Some(t.as_str()) } else { None }).collect()
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
        assert!(!e.text.contains("[["));          // both hidden from comrak
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
    fn tag_rules() {
        assert_eq!(tags(&lower("a #tag and #nested/tag, #y1984.")), ["tag", "nested/tag", "y1984"]);
        assert!(tags(&lower("year #1984 here")).is_empty());   // all-numeric
        assert!(tags(&lower("foo#bar")).is_empty());           // no boundary
        assert!(tags(&lower("# heading-ish")).is_empty());     // '#' + space
        assert_eq!(tags(&lower("#a-b_c")), ["a-b_c"]);
        assert_eq!(tags(&lower("#tag/")), ["tag"]);
    }
}
