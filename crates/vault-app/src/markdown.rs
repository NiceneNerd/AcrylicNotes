//! Pure markdown text helpers shared by view models.
//!
//! The reading renderer owns full rendering; these helpers answer the
//! small "what does this look like as plain text" questions the
//! navigation view models (outline) and later editor operations ask.

use vault_md::{Inline, WikiLink};

/// The plain text of a run of inlines, descending into nested structures
/// (emphasis, links, footnotes). Embeds do not descend — they pull other
/// notes in at render time; their target name still counts as text so
/// `![[Note]]` is searchable in a coarse way.
pub fn plain_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(s) | Inline::Code(s) | Inline::Math(s) | Inline::Html(s) => {
                out.push_str(s)
            }
            Inline::Emph(inner)
            | Inline::Strong(inner)
            | Inline::Strike(inner)
            | Inline::Highlight(inner)
            | Inline::InlineFootnote(inner) => out.push_str(&plain_inlines(inner)),
            Inline::Link { content, .. } => out.push_str(&plain_inlines(content)),
            Inline::WikiLink(link) => out.push_str(&plain_wiki_label(link)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Embed(embed) => out.push_str(&embed.target.note),
            Inline::Tag(t) => {
                out.push('#');
                out.push_str(t);
            }
            Inline::FootnoteRef(l) => out.push_str(&format!("[{l}]")),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
        }
    }
    out
}

/// The displayed label of a wikilink: the alias when present, else the
/// target note's name.
pub fn plain_wiki_label(link: &WikiLink) -> String {
    link.alias
        .as_ref()
        .map(|alias| plain_inlines(alias))
        .unwrap_or_else(|| link.target.note.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_md::WikiTarget;

    fn link(target: &str, alias: Option<Vec<Inline>>) -> WikiLink {
        WikiLink {
            target: WikiTarget {
                note: target.to_string(),
                heading: None,
                block: None,
            },
            alias,
        }
    }

    #[test]
    fn a_wikilink_prefers_its_alias() {
        let aliased = link("Note.md", Some(vec![Inline::Text("Display".into())]));
        assert_eq!(plain_wiki_label(&aliased), "Display");
        let bare = link("Note.md", None);
        assert_eq!(plain_wiki_label(&bare), "Note.md");
    }
}
