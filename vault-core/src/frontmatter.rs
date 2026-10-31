//! YAML frontmatter handling.
//!
//! The frontmatter block is kept verbatim as raw bytes. A *copy* is parsed
//! for metadata (tags, aliases, cssclasses); the original is never
//! re-serialized on save.

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_norway::{Mapping, Value};
use thiserror::Error;

/// Result of splitting a Markdown document into frontmatter and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitFrontmatter {
    /// The raw frontmatter content (between the delimiters, without delimiters),
    /// suitable for YAML parsing. `None` if no frontmatter was present.
    pub content: Option<String>,
    /// The full frontmatter block including delimiters, verbatim with original
    /// line endings. `None` if no frontmatter was present.
    pub raw: Option<String>,
    /// The document body (everything after the closing delimiter).
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
/// Recognizes `---` or `...` delimiters on their own line at the start of
/// the file. Handles BOM and CRLF line endings. The raw frontmatter
/// (including delimiters and trailing newline) is returned verbatim; the body
/// excludes the closing delimiter and its trailing newline.
pub fn split_frontmatter(src: &str) -> SplitFrontmatter {
    let bytes = src.as_bytes();
    let mut i = 0;
    let bom_skipped = if bytes.starts_with(b"\xef\xbb\xbf") {
        i = 3;
        true
    } else {
        false
    };

    // Must start with `---` on its own line.
    if !matches_at(bytes, i, b"---") {
        return SplitFrontmatter {
            content: None,
            raw: None,
            body: src.to_string(),
        };
    }
    i += 3;
    // Consume the rest of the opening delimiter line (optional whitespace + newline).
    i = skip_line(bytes, i);

    // Find the closing delimiter: `---` or `...` on its own line.
    let content_start = i;
    while i < bytes.len() {
        let line_start = i;
        // Check for closing delimiter.
        if matches_at(bytes, i, b"---") || matches_at(bytes, i, b"...") {
            let after = i + 3;
            // Must be followed by newline or end of file.
            if after >= bytes.len() || bytes[after] == b'\n' || bytes[after] == b'\r' {
                let content = src[content_start..line_start].to_string(); // content between delimiters
                                                                          // Include the closing delimiter and its trailing newline in raw.
                                                                          // Handle both LF and CRLF.
                let raw_end = if after < bytes.len() {
                    if bytes[after] == b'\r' && after + 1 < bytes.len() && bytes[after + 1] == b'\n'
                    {
                        after + 2 // CRLF
                    } else if bytes[after] == b'\n' || bytes[after] == b'\r' {
                        after + 1 // LF or standalone CR
                    } else {
                        after
                    }
                } else {
                    after
                };
                let raw_start = if bom_skipped { 3 } else { 0 };
                let full_raw = src[raw_start..raw_end].to_string(); // including delimiters and trailing newline
                let body = src[raw_end..].trim_start_matches(['\n', '\r']).to_string();
                return SplitFrontmatter {
                    content: Some(content),
                    raw: Some(full_raw),
                    body,
                };
            }
        }
        // Skip to next line.
        i = skip_line(bytes, i);
    }

    // No closing delimiter: treat as no frontmatter.
    SplitFrontmatter {
        content: None,
        raw: None,
        body: src.to_string(),
    }
}

fn matches_at(bytes: &[u8], pos: usize, pattern: &[u8]) -> bool {
    bytes.get(pos..pos + pattern.len()) == Some(pattern)
}

fn skip_line(bytes: &[u8], mut pos: usize) -> usize {
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
    let mapping: Mapping = serde_norway::from_str(content)?;
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

fn deserialize_tags_value(val: &Value) -> Result<Vec<String>, serde_norway::Error> {
    match val {
        Value::Sequence(seq) => seq
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .ok_or_else(|| DeError::custom("tag must be string"))
            })
            .collect(),
        Value::String(s) => Ok(vec![s.clone()]),
        _ => Ok(Vec::new()),
    }
}

fn deserialize_string_or_vec_value(val: &Value) -> Result<Vec<String>, serde_norway::Error> {
    match val {
        Value::Sequence(seq) => seq
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .ok_or_else(|| DeError::custom("value must be string"))
            })
            .collect(),
        Value::String(s) => Ok(vec![s.clone()]),
        _ => Ok(Vec::new()),
    }
}

/// Errors from frontmatter operations.
#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_norway::Error),
    #[error("Frontmatter is not a YAML mapping")]
    NotMapping,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let s = "just body";
        let split = split_frontmatter(s);
        assert_eq!(split.raw, None);
        assert_eq!(split.content, None);
        assert_eq!(split.body, "just body");
    }

    #[test]
    fn frontmatter_with_dashes() {
        let s = "---\ntitle: Test\n---\nbody";
        let split = split_frontmatter(s);
        assert!(split.raw.is_some());
        assert_eq!(split.content.as_ref().unwrap(), "title: Test\n");
        assert_eq!(split.raw.as_ref().unwrap(), "---\ntitle: Test\n---\n");
        assert_eq!(split.body, "body");
        let meta = parse_frontmatter_meta(split.content.as_ref().unwrap()).unwrap();
        assert_eq!(meta.tags, Vec::<String>::new());
    }

    #[test]
    fn frontmatter_with_dots() {
        let s = "---\ntitle: Test\n...\nbody";
        let split = split_frontmatter(s);
        assert!(split.raw.is_some());
        assert_eq!(split.content.as_ref().unwrap(), "title: Test\n");
        assert_eq!(split.raw.as_ref().unwrap(), "---\ntitle: Test\n...\n");
        assert_eq!(split.body, "body");
    }

    #[test]
    fn frontmatter_crlf() {
        let s = "---\r\ntitle: Test\r\n---\r\nbody";
        let split = split_frontmatter(s);
        assert!(split.raw.is_some());
        assert_eq!(split.content.as_ref().unwrap(), "title: Test\r\n");
        assert_eq!(split.raw.as_ref().unwrap(), "---\r\ntitle: Test\r\n---\r\n");
        assert_eq!(split.body, "body");
    }

    #[test]
    fn frontmatter_bom() {
        let s = "\u{feff}---\ntitle: Test\n---\nbody";
        let split = split_frontmatter(s);
        assert!(split.raw.is_some());
        assert_eq!(split.content.as_ref().unwrap(), "title: Test\n");
        assert_eq!(split.raw.as_ref().unwrap(), "---\ntitle: Test\n---\n");
        assert_eq!(split.body, "body");
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
    fn aliases_and_cssclasses() {
        let s = "---\naliases: [a, b]\ncssclasses: wide\n---\nbody";
        let split = split_frontmatter(s);
        let meta = parse_frontmatter_meta(split.content.as_ref().unwrap()).unwrap();
        assert_eq!(meta.aliases, vec!["a", "b"]);
        assert_eq!(meta.cssclasses, vec!["wide"]);
    }

    #[test]
    fn no_closing_delimiter_is_no_frontmatter() {
        let s = "---\ntitle: Test\nbody";
        let split = split_frontmatter(s);
        assert_eq!(split.raw, None);
        assert_eq!(split.content, None);
        assert_eq!(split.body, s);
    }
}
