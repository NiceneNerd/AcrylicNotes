//! Indexing-grade body scans: links, tags, headings, block IDs.
//!
//! This module is deliberately simpler than `vault-md`'s sentinel extractor:
//! it only answers "what does this note point at / contain" for the index and
//! the backlink graph, not produce renderable spans. The two scanners must
//! agree on what counts as a link; vault-md's golden tests (M2) cross-check
//! the overlap. Do not extend one without checking the other.
//!
//! ASSUMPTIONS to confirm against real Obsidian in M3 (HANDOFF 4.3):
//! * block-ID character set and placement rules,
//! * whether tags immediately after `(` count (we say yes, matching the
//!   not-mid-word rule; link DESTINATIONS are excluded, which the user
//!   confirmed matches their vault's tag pane),
//! * same-document links (`[[#Heading]]`, `(#anchor)`) are skipped (they are
//!   not vault-level backlink edges).

/// How a link was written; decides how it resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `[[Note]]` — resolves by Obsidian's shortest-unique-name rules.
    Wiki,
    /// `[text](path)` — resolves relative to the note's folder.
    Markdown,
}

/// A wikilink/embed target as written (note part only, before `#`/`|`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLink {
    pub embed: bool,
    pub kind: LinkKind,
    /// Wiki: note target as written; empty for same-document (`[[#Heading]]`).
    /// Markdown: percent-decoded vault-relative path. External URLs and pure
    /// `#anchors` are not reported at all.
    pub target: String,
}

/// Scan a note body for wikilinks and embeds, skipping fenced code blocks
/// and inline code spans.
pub fn scan_links(body: &str) -> Vec<RawLink> {
    let mut links = Vec::new();
    for line in non_code_lines(body) {
        let mut search = 0;
        while let Some(pos) = line[search..].find("[[") {
            let abs = search + pos;
            let embed = line[..abs].ends_with('!');
            let content_start = abs + 2;
            let Some(end) = line[content_start..].find("]]") else {
                break;
            };
            let inner = &line[content_start..content_start + end];
            if !inner.contains('[') && !inner.trim().is_empty() {
                // target = before alias `|` and before heading/block `#`
                let target = inner
                    .split('|')
                    .next()
                    .unwrap_or("")
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !target.is_empty() {
                    links.push(RawLink {
                        embed,
                        kind: LinkKind::Wiki,
                        target: target.to_string(),
                    });
                }
                search = content_start + end + 2;
            } else {
                search = abs + 2;
            }
        }
    }
    links
}

/// Scan a note body for standard Markdown links and images, skipping code.
/// Only vault-internal targets are returned: external schemes (`http(s):`,
/// `mailto:`, `obsidian://`, …) and pure `#anchors` are skipped, and
/// percent-escapes are decoded so `[x](My%20Note.md)` resolves.
pub fn scan_md_links(body: &str) -> Vec<RawLink> {
    let mut links = Vec::new();
    for line in non_code_lines(body) {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '[' {
                i += 1;
                continue;
            }
            let embed = i > 0 && chars[i - 1] == '!';
            // link text: to the matching `]`, counting nested brackets
            let Some(text_end) = matching_bracket(&chars, i) else {
                i += 1;
                continue;
            };
            // inline links only: `](` — reference links `[x][y]` have no target here
            if chars.get(text_end + 1) != Some(&'(') {
                i = text_end + 1;
                continue;
            }
            let Some(close) = balanced_parens(&chars, text_end + 1) else {
                i = text_end + 1;
                continue;
            };
            let raw_dest: String = chars[text_end + 2..close].iter().collect();
            let dest = raw_dest.trim();
            if !is_external(dest) {
                links.push(RawLink {
                    embed,
                    kind: LinkKind::Markdown,
                    target: percent_decode(dest),
                });
            }
            i = close + 1;
        }
    }
    links
}

/// The index of the `]` matching the `[` at `start` (nested brackets allowed).
fn matching_bracket(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &c) in chars.iter().enumerate().skip(start) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Given the index of `(`, the index of its matching `)` (balanced).
fn balanced_parens(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Schemes that are not vault paths. A leading `#` is a same-document anchor.
fn is_external(dest: &str) -> bool {
    let lower = dest.to_ascii_lowercase();
    lower.starts_with('#')
        || [
            "http://",
            "https://",
            "mailto:",
            "obsidian://",
            "file://",
            "www.",
        ]
        .iter()
        .any(|s| lower.starts_with(s))
}

/// Minimal percent-decoding for link destinations (`%20` → space).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Scan a note body for inline `#tags` (frontmatter tags live in
/// [`crate::FrontmatterMeta`]). Skips fenced code, inline code, and Markdown
/// link destinations — `[fn](#footnote-3)` is a URL fragment, not a tag
/// (user-confirmed against the real vault's tag pane).
///
/// Tag rules from Obsidian's docs: letters, digits, `_ - /`, at least one
/// non-digit; must start at a word boundary; trailing `/` trimmed.
pub fn scan_tags(body: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for line in non_code_lines(body) {
        let line = blank_html_attr_values(&blank_md_destinations(&line));
        let chars: Vec<char> = line.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if c != '#' {
                continue;
            }
            // `\#tag` is an escaped hash, not a tag.
            if i > 0 && chars[i - 1] == '\\' {
                continue;
            }
            if !tag_boundary(chars.get(i.wrapping_sub(1)).filter(|_| i > 0)) {
                continue;
            }
            let body: String = chars[i + 1..]
                .iter()
                .take_while(|&&x| is_tag_char(x))
                .collect();
            let trimmed = body.trim_end_matches('/');
            if !trimmed.is_empty() && trimmed.chars().any(|x| !x.is_ascii_digit()) {
                tags.push(trimmed.to_string());
            }
        }
    }
    tags
}

/// ATX headings (`#`..`######` followed by space/end) outside code.
/// Returns `(level, text)` with text trimmed; inline markup is kept raw
/// (the AST-based index in M2 replaces this).
pub fn scan_headings(body: &str) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    for line in non_code_lines(body) {
        let t = line.trim_start();
        let level = t.chars().take_while(|&c| c == '#').count();
        if level == 0 || level > 6 {
            continue;
        }
        let rest = &t[level..];
        if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
            continue;
        }
        out.push((level as u8, rest.trim().to_string()));
    }
    out
}

/// Block IDs: a final `^id` token on a non-code line.
/// ASSUMED rules (confirm in M3): id chars are alphanumeric plus `-`/`_`,
/// the token must be whitespace-separated, headings excluded.
pub fn scan_block_ids(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in non_code_lines(body) {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let Some(last) = line.split_whitespace().next_back() else {
            continue;
        };
        let Some(id) = last.strip_prefix('^') else {
            continue;
        };
        if !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            out.push(id.to_string());
        }
    }
    out
}

/// Blank quoted attribute values inside HTML tags (`<a href="#fn1">` →
/// `<a href="     ">`) so URL fragments in raw HTML are never scanned as
/// tags. Comments are skipped wholesale.
fn blank_html_attr_values(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = chars.clone();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' {
            i += 1;
            continue;
        }
        // skip comments
        if chars[i..].starts_with(&['<', '!', '-', '-']) {
            match find_seq(&chars, i + 4, &['-', '-', '>']) {
                Some(end) => {
                    i = end + 3;
                    continue;
                }
                None => break,
            }
        }
        // an HTML tag: scan to '>', blanking quoted values
        if chars
            .get(i + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '/')
        {
            let (mut j, mut quote, mut quote_start) = (i + 1, None, 0);
            while j < chars.len() {
                let c = chars[j];
                match quote {
                    Some(q) => {
                        if c == q {
                            for slot in out.iter_mut().take(j).skip(quote_start) {
                                *slot = ' ';
                            }
                            quote = None;
                        }
                    }
                    None => match c {
                        '>' => break,
                        '"' | '\'' => {
                            quote = Some(c);
                            quote_start = j + 1;
                        }
                        _ => {}
                    },
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out.into_iter().collect()
}

fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(seq.len() - 1)).find(|&i| chars[i..i + seq.len()] == *seq)
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

fn tag_boundary(prev: Option<&char>) -> bool {
    prev.is_none_or(|p| !(p.is_alphanumeric() || matches!(p, '_' | '-' | '/' | '#' | '(' | ')')))
}

/// Lines that are not inside a fenced code block, with inline code spans
/// blanked out (replaced by spaces) so scanners never see their content.
fn non_code_lines(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in body.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match fence {
            Some((fc, flen)) => {
                let s = line.trim_start_matches(' ');
                let n = s.chars().take_while(|&c| c == fc).count();
                if n >= flen && s[n..].trim().is_empty() {
                    fence = None;
                }
            }
            None => {
                if fence_run(line, '`') || fence_run(line, '~') {
                    let s = line.trim_start_matches(' ');
                    let ch = s.chars().next().unwrap();
                    fence = Some((ch, s.chars().take_while(|&c| c == ch).count()));
                    continue;
                }
                out.push(blank_inline_code(line));
            }
        }
    }
    out
}

/// Blank the destinations of inline Markdown links/images (`[text](dest)` →
/// `[text]      `), preserving character positions. Wikilinks are untouched.
fn blank_md_destinations(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = chars.clone();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '[' {
            i += 1;
            continue;
        }
        let Some(text_end) = matching_bracket(&chars, i) else {
            i += 1;
            continue;
        };
        if chars.get(text_end + 1) != Some(&'(') {
            i = text_end + 1;
            continue;
        }
        let Some(close) = balanced_parens(&chars, text_end + 1) else {
            i = text_end + 1;
            continue;
        };
        for slot in out.iter_mut().take(close).skip(text_end + 2) {
            *slot = ' ';
        }
        i = close + 1;
    }
    out.into_iter().collect()
}

/// Whether the line (after <=3 leading spaces... simplified: any leading
/// spaces) opens with a run of >= 3 of `ch`. Fence indentation beyond 3
/// spaces would be indented code, not a fence; not modeled here.
fn fence_run(line: &str, ch: char) -> bool {
    let s = line.trim_start_matches(' ');
    let n = s.chars().take_while(|&c| c == ch).count();
    n >= 3
}

/// Replace inline code spans (backtick runs) with spaces, preserving length.
fn blank_inline_code(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = vec![' '; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let n = chars[i..].iter().take_while(|&&c| c == '`').count();
            // find a matching run of the same length
            let mut j = i + n;
            let mut matched = None;
            while j < chars.len() {
                if chars[j] == '`' {
                    let m = chars[j..].iter().take_while(|&&c| c == '`').count();
                    if m == n {
                        matched = Some(j);
                        break;
                    }
                    j += m;
                } else {
                    j += 1;
                }
            }
            if let Some(end) = matched {
                for slot in out.iter_mut().take(end + n).skip(i) {
                    *slot = ' ';
                }
                i = end + n;
                continue;
            }
            // unmatched backtick: literal
            for slot in out.iter_mut().skip(i).take(n) {
                *slot = '`';
            }
            i += n;
        } else {
            out[i] = chars[i];
            i += 1;
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(links: &[RawLink]) -> Vec<(&str, LinkKind, bool)> {
        links
            .iter()
            .map(|l| (l.target.as_str(), l.kind, l.embed))
            .collect()
    }

    #[test]
    fn links_basic_and_embeds() {
        let links = scan_links("see [[Note]] and [[Other|alias]] and ![[img.png|300]]\n");
        assert_eq!(
            targets(&links),
            [
                ("Note", LinkKind::Wiki, false),
                ("Other", LinkKind::Wiki, false),
                ("img.png", LinkKind::Wiki, true),
            ]
        );
    }

    #[test]
    fn links_heading_block_targets_reduce_to_note() {
        let links = scan_links("[[Note#Heading]] [[Note#^abc]] [[#Self]]\n");
        let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["Note", "Note"]); // [[#Self]] skipped
    }

    #[test]
    fn links_skip_code() {
        let src = "`[[no]]` and\n```\n[[no]]\n```\nbut [[yes]]";
        let targets: Vec<String> = scan_links(src).into_iter().map(|l| l.target).collect();
        assert_eq!(targets, ["yes"]);
    }

    #[test]
    fn markdown_links_internal_only_and_decoded() {
        let links = scan_md_links(
            "[text](Note.md) ![img](../media/my%20pic.png)\n[web](https://x.co/a#f) [anchor](#s) [ref][1]",
        );
        assert_eq!(
            targets(&links),
            [
                ("Note.md", LinkKind::Markdown, false),
                ("../media/my pic.png", LinkKind::Markdown, true),
            ]
        );
    }

    #[test]
    fn markdown_link_with_parens_and_nested_brackets() {
        let links = scan_md_links("[[wiki]] then [deep [x] link](a(b)c.md)\n");
        assert_eq!(targets(&links), [("a(b)c.md", LinkKind::Markdown, false)]);
    }

    #[test]
    fn tags_exclude_link_destinations() {
        // The real-vault false positive: 114 `](#anchor)` destinations.
        assert!(scan_tags("[fn](#footnote-3) and [[*]](#fm1)\n").is_empty());
        // HTML href attributes too (`<a href="#fn1">`)
        assert!(scan_tags("<a href=\"#fn1\">[*]</a> prose\n").is_empty());
        // but visible text keeps its tags
        assert_eq!(scan_tags("[see #tag here](x.md)\n"), ["tag"]);
        assert_eq!(scan_tags("<a href=\"#x\">[see #tag]</a>\n"), ["tag"]);
    }

    /// Confirmed against real Obsidian (M3, 2026-10-31): a `#` directly
    /// after an open or close paren does NOT register as a tag.
    #[test]
    fn tags_after_parens_are_not_tags() {
        assert!(scan_tags("(#tag) text\n").is_empty());
        assert!(scan_tags("word)#tag\n").is_empty());
        // whitespace and ordinary punctuation still start a tag
        assert_eq!(scan_tags("word #tag\n"), ["tag"]);
        assert_eq!(scan_tags("word, #tag\n"), ["tag"]);
    }

    #[test]
    fn escaped_hash_is_not_a_tag() {
        // The real vault: `\#ActuallyAutistic` is escaped prose.
        assert!(scan_tags("many \\#ActuallyAutistic activists\n").is_empty());
        assert_eq!(scan_tags("escaped \\#no but real #yes\n"), ["yes"]);
    }

    #[test]
    fn tags_rules() {
        assert_eq!(
            scan_tags("a #tag and #nested/tag, #y1984.\n"),
            ["tag", "nested/tag", "y1984"]
        );
        assert!(scan_tags("year #1984 here\n").is_empty()); // all-numeric
        assert!(scan_tags("foo#bar\n").is_empty()); // no boundary
        assert!(scan_tags("# heading-ish\n").is_empty()); // '#' + space
        assert_eq!(scan_tags("#a-b_c\n"), ["a-b_c"]);
        assert_eq!(scan_tags("#tag/\n"), ["tag"]);
        // inline code is skipped
        assert!(scan_tags("`#notag`\n").is_empty());
    }

    #[test]
    fn headings_levels() {
        assert_eq!(
            scan_headings("# One\n\n### Three\n####no-space\n```\n# in code\n```\n###### Six"),
            [(1, "One".into()), (3, "Three".into()), (6, "Six".into()),]
        );
    }

    #[test]
    fn block_ids() {
        assert_eq!(
            scan_block_ids(
                "para text ^abc-1\n\n- item ^li_id\n# Head ^nope\n```\n^incode\n```\nword^glued"
            ),
            ["abc-1", "li_id"]
        );
    }
}
