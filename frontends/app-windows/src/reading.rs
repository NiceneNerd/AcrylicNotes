//! Reading-mode support for the Windows shell.
//!
//! Native element construction from the `vault_md` IR (paragraph, list,
//! callout, table, image, ...) is p11 Windows work — there is nothing to
//! render into until the WinUI controls exist, so this module makes no
//! pretense of it. What exists here is real and Linux-tested: the
//! toolkit-neutral plain-text projection of a document (the same job the
//! Linux frontend uses for previews and counts) and the renderer
//! configuration the shell will consume.

use vault_md::{Block, Doc, Inline};

/// Reading-renderer configuration for the Windows shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsMarkdownRenderer {
    /// Whether images render inline or as placeholders.
    pub show_images: bool,
    /// Whether the theme is dark (accent/contrast decisions).
    pub dark_mode: bool,
}

impl Default for WindowsMarkdownRenderer {
    fn default() -> Self {
        Self {
            show_images: true,
            dark_mode: false,
        }
    }
}

impl WindowsMarkdownRenderer {
    /// The plain text of `inlines` (labels, find snippets, switcher
    /// entries). Shared kernel projection: same output as the Linux
    /// frontend.
    #[must_use]
    pub fn plain_text(inlines: &[Inline]) -> String {
        vault_app::markdown::plain_inlines(inlines)
    }

    /// The plain text of a whole document, recursively over blocks in
    /// document order (previews, word/character counts). Footnote
    /// definitions and embed targets contribute no inline text: they are
    /// render-time constructs (placeholder or transclusion), not prose.
    #[must_use]
    pub fn doc_plain_text(doc: &Doc) -> String {
        let mut out = String::new();
        push_blocks(&mut out, &doc.blocks);
        out
    }
}

fn push_blocks(out: &mut String, blocks: &[Block]) {
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph { content, .. } => {
                push_inlines(out, content);
            }
            Block::List { items, .. } => {
                for item in items {
                    push_blocks(out, &item.body);
                }
            }
            Block::Quote(inner) => push_blocks(out, inner),
            Block::Callout { title, body, .. } => {
                push_inlines(out, title);
                push_blocks(out, body);
            }
            Block::Code { text, .. } | Block::Math(text) | Block::Html(text) => {
                push_text(out, text);
            }
            Block::Table { header, rows, .. } => {
                for cell in header {
                    push_inlines(out, cell);
                }
                for row in rows {
                    for cell in row {
                        push_inlines(out, cell);
                    }
                }
            }
            Block::Rule | Block::EmbedBlock(_) | Block::FootnoteDef { .. } => {}
        }
    }
}

fn push_inlines(out: &mut String, inlines: &[Inline]) {
    push_text(out, &vault_app::markdown::plain_inlines(inlines));
}

fn push_text(out: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_md::Inline;

    fn doc_with(blocks: Vec<Block>) -> Doc {
        Doc {
            frontmatter_raw: None,
            blocks,
            spans: Vec::new(),
        }
    }

    #[test]
    fn paragraphs_and_headings_project_in_order() {
        let doc = doc_with(vec![
            Block::Heading {
                level: 1,
                content: vec![Inline::Text("Title".into())],
                id: None,
            },
            Block::Paragraph {
                content: vec![
                    Inline::Text("first ".into()),
                    Inline::Strong(vec![Inline::Text("second".into())]),
                ],
                block_id: None,
            },
        ]);
        assert_eq!(
            WindowsMarkdownRenderer::doc_plain_text(&doc),
            "Title first second"
        );
    }

    #[test]
    fn lists_quotes_and_callouts_recurse() {
        let doc = doc_with(vec![Block::List {
            ordered: false,
            start: None,
            tight: true,
            items: vec![vault_md::ListItem {
                task: None,
                block_id: None,
                body: vec![Block::Paragraph {
                    content: vec![Inline::Text("item".into())],
                    block_id: None,
                }],
            }],
        }]);
        assert_eq!(WindowsMarkdownRenderer::doc_plain_text(&doc), "item");

        let doc = doc_with(vec![Block::Quote(vec![Block::Paragraph {
            content: vec![Inline::Text("quoted".into())],
            block_id: None,
        }])]);
        assert_eq!(WindowsMarkdownRenderer::doc_plain_text(&doc), "quoted");

        let doc = doc_with(vec![Block::Callout {
            kind: "note".into(),
            fold: vault_md::Fold::None,
            metadata: None,
            title: vec![Inline::Text("Heads up".into())],
            body: vec![Block::Paragraph {
                content: vec![Inline::Text("body".into())],
                block_id: None,
            }],
        }]);
        assert_eq!(
            WindowsMarkdownRenderer::doc_plain_text(&doc),
            "Heads up body"
        );
    }

    #[test]
    fn code_math_and_tables_project_their_text() {
        let doc = doc_with(vec![
            Block::Code {
                lang: Some("rust".into()),
                text: "let x = 1;".into(),
            },
            Block::Math("e=mc^2".into()),
            Block::Table {
                alignments: vec![],
                header: vec![vec![Inline::Text("A".into())]],
                rows: vec![vec![vec![Inline::Text("b".into())]]],
            },
        ]);
        assert_eq!(
            WindowsMarkdownRenderer::doc_plain_text(&doc),
            "let x = 1; e=mc^2 A b"
        );
    }

    #[test]
    fn rules_embeds_and_empty_blocks_contribute_nothing() {
        let doc = doc_with(vec![
            Block::Rule,
            Block::Paragraph {
                content: vec![],
                block_id: None,
            },
        ]);
        assert_eq!(WindowsMarkdownRenderer::doc_plain_text(&doc), "");
    }

    #[test]
    fn plain_text_of_inlines_matches_the_kernel_projection() {
        let inlines = vec![
            Inline::Text("see ".into()),
            Inline::Link {
                url: "https://example.com".into(),
                title: None,
                content: vec![Inline::Text("the docs".into())],
            },
        ];
        assert_eq!(
            WindowsMarkdownRenderer::plain_text(&inlines),
            "see the docs"
        );
    }
}
