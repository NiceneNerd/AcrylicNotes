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
//! * whether tags inside headings count,
//! * same-document links (`[[#Heading]]`) are skipped here (they are not
//!   vault-level backlink edges).

/// A wikilink or embed target as written (note part only, before `#`/`|`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLink {
    pub embed: bool,
    /// Note target as written; empty for same-document links (`[[#Heading]]`).
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

/// Scan a note body for inline `#tags` (frontmatter tags live in
/// [`crate::FrontmatterMeta`]). Skips fenced code and inline code.
///
/// Tag rules from Obsidian's docs: letters, digits, `_ - /`, at least one
/// non-digit; must start at a word boundary; trailing `/` trimmed.
/// Boundary behavior (`foo#bar`) is ASSUMED (HANDOFF 4.3).
pub fn scan_tags(body: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for line in non_code_lines(body) {
        let chars: Vec<char> = line.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if c != '#' || !tag_boundary(chars.get(i.wrapping_sub(1)).filter(|_| i > 0)) {
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

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

fn tag_boundary(prev: Option<&char>) -> bool {
    prev.is_none_or(|p| !(p.is_alphanumeric() || matches!(p, '_' | '-' | '/' | '#')))
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

    #[test]
    fn links_basic_and_embeds() {
        let links = scan_links("see [[Note]] and [[Other|alias]] and ![[img.png|300]]\n");
        assert_eq!(
            links,
            [
                RawLink {
                    embed: false,
                    target: "Note".into()
                },
                RawLink {
                    embed: false,
                    target: "Other".into()
                },
                RawLink {
                    embed: true,
                    target: "img.png".into()
                },
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
