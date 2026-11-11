//! YAML frontmatter handling.
//!
//! The frontmatter block is kept verbatim as raw bytes. A *copy* is parsed
//! for metadata (tags, aliases, cssclasses); the original is never
//! re-serialized on save.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use serde_saphyr::from_str;
use thiserror::Error;

/// Result of splitting a Markdown document into frontmatter and body.
///
/// The split is LOSSLESS: `bom + raw + body == original` byte-for-byte, so a
/// save that reuses `raw` can never alter bytes the user did not edit
/// (HANDOFF section 2: frontmatter bytes verbatim, line endings preserved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitFrontmatter {
    /// UTF-8 BOM exactly as it appeared ("" if the file had none).
    pub bom: String,
    /// The full frontmatter block including delimiters and the closing
    /// delimiter's trailing newline, byte-for-byte. `None` if the file has
    /// no frontmatter. Never contains the BOM (that lives in `bom`).
    pub raw: Option<String>,
    /// The YAML between the delimiters (for parsing a copy). `None` if no
    /// frontmatter.
    pub content: Option<String>,
    /// Everything after the frontmatter block, verbatim — never trimmed.
    pub body: String,
}

/// Metadata extracted from frontmatter YAML.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrontmatterMeta {
    /// Note title (`title:` field), if present.
    #[serde(default)]
    pub title: Option<String>,
    /// Tags as a list of strings (Obsidian `tags: [a, b]` or `tags: a`).
    #[serde(default, deserialize_with = "deserialize_tags")]
    pub tags: Vec<String>,
    /// Aliases for this note (Obsidian `aliases: [x, y]` or `aliases: x`).
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub aliases: Vec<String>,
    /// CSS classes (Obsidian `cssclasses: [x, y]` or `cssclasses: x`).
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub cssclasses: Vec<String>,
}

/// Split a Markdown document at the frontmatter boundary.
///
/// The opener must be `---` alone on the file's first line (trailing spaces or
/// tabs tolerated); `----` or `--- text` do NOT open frontmatter. The closer
/// is `---` or `...` alone on its line. BOM and CRLF are handled, and the
/// split is lossless: `bom + raw + body` reproduces the input byte-for-byte.
pub fn split_frontmatter(src: &str) -> SplitFrontmatter {
    let (bom, rest) = if src.as_bytes().starts_with(b"\xef\xbb\xbf") {
        ("\u{feff}", &src[3..])
    } else {
        ("", src)
    };
    let bytes = rest.as_bytes();

    let no_frontmatter = || SplitFrontmatter {
        bom: bom.to_string(),
        raw: None,
        content: None,
        body: rest.to_string(),
    };

    // Opener: `---` alone on the first line.
    let Some(opener_end) = delimiter_line_end(bytes, 0, b"---") else {
        return no_frontmatter();
    };

    let yaml_start = opener_end;
    let mut i = yaml_start;
    while i < bytes.len() {
        if let Some(closer_end) =
            delimiter_line_end(bytes, i, b"---").or_else(|| delimiter_line_end(bytes, i, b"..."))
        {
            return SplitFrontmatter {
                bom: bom.to_string(),
                raw: Some(rest[..closer_end].to_string()),
                content: Some(rest[yaml_start..i].to_string()),
                body: rest[closer_end..].to_string(),
            };
        }
        i = next_line_start(bytes, i);
    }

    // No closing delimiter: not frontmatter.
    no_frontmatter()
}

/// If a `delim` delimiter line starts at `pos` — the delimiter optionally
/// followed by spaces/tabs, then end of line or end of file — return the
/// offset just past the line terminator (or past the delimiter at EOF).
fn delimiter_line_end(bytes: &[u8], pos: usize, delim: &[u8]) -> Option<usize> {
    if bytes.get(pos..pos + delim.len()) != Some(delim) {
        return None;
    }
    let mut i = pos + delim.len();
    while matches!(bytes.get(i), Some(b' ') | Some(b'\t')) {
        i += 1;
    }
    match bytes.get(i) {
        None => Some(i), // EOF right after the delimiter
        Some(b'\n') => Some(i + 1),
        Some(b'\r') => Some(if bytes.get(i + 1) == Some(&b'\n') {
            i + 2
        } else {
            i + 1
        }),
        Some(_) => None, // more content on the line: not a delimiter
    }
}

fn next_line_start(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos] != b'\n' {
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'\n' {
        pos += 1;
    }
    pos
}

/// Parse the raw frontmatter content (without delimiters) for metadata.
///
/// Returns `None` if the YAML is invalid or not a mapping.
pub fn parse_frontmatter_meta(content: &str) -> Result<FrontmatterMeta, FrontmatterError> {
    let value: Value = from_str(content)?;
    let mapping = value.as_object().ok_or(FrontmatterError::NotMapping)?;
    let mut meta = FrontmatterMeta {
        title: mapping
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        ..Default::default()
    };
    // tags
    if let Some(val) = mapping.get("tags") {
        meta.tags = deserialize_tags_value(val)?;
    }
    // aliases
    if let Some(val) = mapping.get("aliases") {
        meta.aliases = deserialize_string_or_vec_value(val)?;
    }
    // cssclasses
    if let Some(val) = mapping.get("cssclasses") {
        meta.cssclasses = deserialize_string_or_vec_value(val)?;
    }

    Ok(meta)
}

/// A helper type that can deserialize either a string or a sequence of strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    String(String),
    Vec(Vec<String>),
}

/// Deserialize a field that can be either a string or a sequence of strings.
fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = StringOrVec::deserialize(deserializer)?;
    Ok(match val {
        StringOrVec::String(s) => vec![s],
        StringOrVec::Vec(v) => v,
    })
}

/// Deserialize tags field (same as string_or_vec, but with different name for clarity).
fn deserialize_tags<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_string_or_vec(deserializer)
}

fn deserialize_tags_value(val: &Value) -> Result<Vec<String>, FrontmatterError> {
    match val {
        Value::Array(seq) => {
            let mut tags = Vec::with_capacity(seq.len());
            for v in seq {
                if let Some(s) = v.as_str() {
                    tags.push(s.to_string());
                } else {
                    return Err(FrontmatterError::ValueError(
                        "tag must be string".to_string(),
                    ));
                }
            }
            Ok(tags)
        }
        Value::String(s) => Ok(vec![s.clone()]),
        _ => Ok(Vec::new()),
    }
}

fn deserialize_string_or_vec_value(val: &Value) -> Result<Vec<String>, FrontmatterError> {
    match val {
        Value::Array(seq) => {
            let mut items = Vec::with_capacity(seq.len());
            for v in seq {
                if let Some(s) = v.as_str() {
                    items.push(s.to_string());
                } else {
                    return Err(FrontmatterError::ValueError(
                        "value must be string".to_string(),
                    ));
                }
            }
            Ok(items)
        }
        Value::String(s) => Ok(vec![s.clone()]),
        _ => Ok(Vec::new()),
    }
}

/// Errors from frontmatter operations.
#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_saphyr::Error),
    #[error("Frontmatter is not a YAML mapping")]
    NotMapping,
    #[error("Frontmatter value error: {0}")]
    ValueError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Corpus of documents the split must handle losslessly.
    fn corpus() -> Vec<&'static str> {
        vec![
            "just body",
            "",
            "\u{feff}plain with BOM\nsecond line\n",
            "---\ntitle: Test\n---\nbody",
            "---\ntitle: Test\n---\n\nbody", // blank line after frontmatter
            "---\ntitle: Test\n---\n\n\n\nbody", // several blank lines
            "---\ntitle: Test\n---\n",       // nothing after frontmatter
            "---\n---\n",                    // empty frontmatter
            "---\r\ntitle: Test\r\n---\r\n\r\nbody",
            "\u{feff}---\ntitle: Test\n---\nbody", // BOM + frontmatter
            "----\nnot frontmatter\n",             // 4 dashes: no opener
            "--- text\nnot frontmatter\n",         // content on opener line
            "--- \ntrailing space tolerated\n",    // hmm: this IS an opener with no closer
            "---\ntitle: Test\nbody",              // no closer
            "---\ntitle: Test\n...\nbody",         // dots closer
            "---\ntitle: Test\n---  \nbody",       // closer with trailing space
            "---\nrelated: \"[[Note]]\"\n---\nbody",
            "text\n---\nrule later\n---\nmore\n", // --- not at byte 0
        ]
    }

    #[test]
    fn split_is_lossless() {
        for src in corpus() {
            let split = split_frontmatter(src);
            let rebuilt = format!(
                "{}{}{}",
                split.bom,
                split.raw.as_deref().unwrap_or(""),
                split.body
            );
            assert_eq!(rebuilt, src, "lossless round-trip failed for {src:?}");
        }
    }

    #[test]
    fn no_frontmatter() {
        let s = "just body";
        let split = split_frontmatter(s);
        assert_eq!(split.raw, None);
        assert_eq!(split.content, None);
        assert_eq!(split.body, "just body");
    }

    #[test]
    fn blank_line_after_frontmatter_is_preserved() {
        let split = split_frontmatter("---\ntitle: Test\n---\n\nbody");
        assert_eq!(split.body, "\nbody");
        let split2 = split_frontmatter("---\ntitle: Test\n---\n\n\n\nbody");
        assert_eq!(split2.body, "\n\n\nbody");
    }

    #[test]
    fn four_dashes_are_not_an_opener() {
        let split = split_frontmatter("----\nnot frontmatter\n---\nlater rule\n");
        assert_eq!(split.raw, None);
        assert_eq!(split.body, "----\nnot frontmatter\n---\nlater rule\n");
    }

    #[test]
    fn content_on_opener_line_is_not_frontmatter() {
        let split = split_frontmatter("--- text\nnot frontmatter\n");
        assert_eq!(split.raw, None);
    }

    #[test]
    fn closer_with_trailing_whitespace() {
        let split = split_frontmatter("---\ntitle: Test\n---  \nbody");
        assert_eq!(split.content.as_deref(), Some("title: Test\n"));
        assert_eq!(split.body, "body");
    }

    #[test]
    fn bom_is_preserved_separately() {
        let split = split_frontmatter("\u{feff}---\ntitle: Test\n---\nbody");
        assert_eq!(split.bom, "\u{feff}");
        assert_eq!(split.raw.as_deref(), Some("---\ntitle: Test\n---\n"));
        assert_eq!(split.body, "body");
        // and a BOM file without frontmatter
        let split2 = split_frontmatter("\u{feff}plain");
        assert!(split2.raw.is_none());
        assert_eq!(split2.bom, "\u{feff}");
        assert_eq!(split2.body, "plain");
    }

    #[test]
    fn frontmatter_with_dashes() {
        let s = "---\ntitle: Test\n---\nbody";
        let split = split_frontmatter(s);
        assert_eq!(split.content.as_deref(), Some("title: Test\n"));
        assert_eq!(split.raw.as_deref(), Some("---\ntitle: Test\n---\n"));
        assert_eq!(split.body, "body");
        let meta = parse_frontmatter_meta(split.content.as_ref().unwrap()).unwrap();
        assert_eq!(meta.tags, Vec::<String>::new());
    }

    #[test]
    fn frontmatter_with_dots() {
        let s = "---\ntitle: Test\n...\nbody";
        let split = split_frontmatter(s);
        assert_eq!(split.content.as_deref(), Some("title: Test\n"));
        assert_eq!(split.raw.as_deref(), Some("---\ntitle: Test\n...\n"));
        assert_eq!(split.body, "body");
    }

    #[test]
    fn frontmatter_crlf() {
        let s = "---\r\ntitle: Test\r\n---\r\nbody";
        let split = split_frontmatter(s);
        assert_eq!(split.content.as_deref(), Some("title: Test\r\n"));
        assert_eq!(split.raw.as_deref(), Some("---\r\ntitle: Test\r\n---\r\n"));
        assert_eq!(split.body, "body");
    }

    #[test]
    fn no_closing_delimiter_is_no_frontmatter() {
        let s = "---\ntitle: Test\nbody";
        let split = split_frontmatter(s);
        assert_eq!(split.raw, None);
        assert_eq!(split.content, None);
        assert_eq!(split.body, s);
    }

    #[test]
    fn tags_list_and_string() {
        let s = "---\ntags: [a, b]\n---\nbody";
        let split = split_frontmatter(s);
        let meta = parse_frontmatter_meta(split.content.as_ref().unwrap()).unwrap();
        assert_eq!(meta.tags, vec!["a", "b"]);

        let s2 = "---\ntags: single\n---\nbody";
        let split2 = split_frontmatter(s2);
        let meta2 = parse_frontmatter_meta(split2.content.as_ref().unwrap()).unwrap();
        assert_eq!(meta2.tags, vec!["single"]);
    }

    #[test]
    fn title_and_aliases_and_cssclasses() {
        let s = "---\ntitle: My Title\naliases: [a, b]\ncssclasses: wide\n---\nbody";
        let split = split_frontmatter(s);
        let meta = parse_frontmatter_meta(split.content.as_ref().unwrap()).unwrap();
        assert_eq!(meta.title.as_deref(), Some("My Title"));
        assert_eq!(meta.aliases, vec!["a", "b"]);
        assert_eq!(meta.cssclasses, vec!["wide"]);
    }
}
