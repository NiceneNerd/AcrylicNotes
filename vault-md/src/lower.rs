//! Lowering: comrak AST → [`crate::ir::Doc`].
//!
//! comrak 0.59 natively parses most Obsidian syntax; this module maps its AST
//! onto our IR. Our pre-pass (`inline::extract`) owns wikilinks/embeds and
//! tags (comrak's wikilinks extension is deliberately OFF, per HANDOFF 4.1);
//! comrak owns callouts, comments, highlight, math, tasks, tables and
//! footnotes.
//!
//! Deliberate behaviors:
//! * empty paragraphs are dropped (a comment-only paragraph leaves nothing),
//! * a trailing `^block-id` on a paragraph or list item becomes that block's
//!   id (a lone `^id` paragraph stays literal text — assumption, see COMPAT),
//! * display math alone in a paragraph becomes `Block::Math`.

use comrak::nodes::{AstNode, ListType, NodeValue};
use comrak::{parse_document, Arena, Options};

use crate::inline::{lower_text, Extracted, Token};
use crate::ir::{Alignment, Block, Doc, Fold, Inline, ListItem};

/// Parse a whole note (frontmatter included) into a [`Doc`].
///
/// The frontmatter block is split off first with vault-core's lossless split
/// and kept verbatim; it is never parsed as markdown.
pub fn parse_note(src: &str) -> Doc {
    let split = vault_core::split_frontmatter(src);
    Doc {
        frontmatter_raw: split.raw,
        blocks: lower_source(&split.body),
    }
}

/// Lower a source fragment (no frontmatter expected) into blocks.
pub fn lower_source(src: &str) -> Vec<Block> {
    let extracted = crate::inline::extract(src);
    lower_comrak(&extracted)
}

fn lower_comrak(extracted: &Extracted) -> Vec<Block> {
    let arena = Arena::new();
    let root = parse_document(&arena, &extracted.text, &comrak_options());
    let mut blocks = Vec::new();
    for child in root.children() {
        lower_block(child, &extracted.tokens, &mut blocks);
    }
    blocks
}

/// The extension set: everything Obsidian's reading view does that comrak
/// implements natively. Wikilinks stay OFF — `inline::extract` owns them.
fn comrak_options() -> Options<'static> {
    let mut options = Options::default();
    let e = &mut options.extension;
    e.table = true;
    e.tasklist = true;
    e.strikethrough = true;
    e.footnotes = true;
    e.inline_footnotes = true;
    e.ofm_callouts = true;
    e.ofm_comments = true;
    e.highlight = true;
    e.math_dollars = true;
    e.autolink = true;
    e.header_attributes = true;
    options
}

fn lower_block<'a>(node: &'a AstNode<'a>, tokens: &[Token], out: &mut Vec<Block>) {
    match &node.data().value {
        NodeValue::Paragraph => {
            // Display math alone in a paragraph is a math block.
            let children: Vec<&AstNode> = node.children().collect();
            if let [single] = children.as_slice() {
                if let NodeValue::Math(math) = &single.data().value {
                    if math.display_math {
                        out.push(Block::Math(math.literal.trim().to_string()));
                        return;
                    }
                }
            }
            let mut content = lower_inlines(node, tokens);
            let block_id = take_trailing_block_id(&mut content);
            // A paragraph that is exactly one embed is a block-level embed.
            if content.len() == 1 {
                if let Inline::Embed(embed) = &content[0] {
                    out.push(Block::EmbedBlock(embed.clone()));
                    return;
                }
            }
            // Drop paragraphs left empty (e.g. comment-only paragraphs).
            if !is_empty_inlines(&content) {
                out.push(Block::Paragraph { content, block_id });
            }
        }
        NodeValue::Heading(heading) => {
            let content = lower_inlines(node, tokens);
            let id = node
                .data()
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.id.clone());
            out.push(Block::Heading {
                level: heading.level,
                content,
                id,
            });
        }
        NodeValue::List(list) => {
            let ordered = list.list_type == ListType::Ordered;
            let start = if ordered {
                Some(list.start as u64)
            } else {
                None
            };
            let mut items = Vec::new();
            for child in node.children() {
                match &child.data().value {
                    // A task list replaces the Item node with TaskItem.
                    NodeValue::TaskItem(task) => {
                        let mut body = Vec::new();
                        for c in child.children() {
                            lower_block(c, tokens, &mut body);
                        }
                        let block_id = move_paragraph_block_id(&mut body);
                        items.push(ListItem {
                            task: Some(task.symbol.is_some()),
                            block_id,
                            body,
                        });
                    }
                    _ => items.push(lower_list_item(child, tokens)),
                }
            }
            out.push(Block::List {
                ordered,
                start: start.filter(|&n| n != 1),
                tight: list.tight,
                items,
            });
        }
        NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) => {
            let mut body = Vec::new();
            for child in node.children() {
                lower_block(child, tokens, &mut body);
            }
            out.push(Block::Quote(body));
        }
        NodeValue::Callout(callout) => {
            let mut title = Vec::new();
            let mut body = Vec::new();
            for child in node.children() {
                match &child.data().value {
                    NodeValue::CalloutTitle => title = lower_inlines(child, tokens),
                    _ => lower_block(child, tokens, &mut body),
                }
            }
            // Obsidian shows the capitalized type name when no title text.
            let title = if title.is_empty() {
                vec![Inline::Text(callout.default_title())]
            } else {
                title
            };
            out.push(Block::Callout {
                kind: callout.callout_type.clone(),
                fold: match callout.fold {
                    comrak::nodes::CalloutFold::None => Fold::None,
                    comrak::nodes::CalloutFold::Expanded => Fold::Open,
                    comrak::nodes::CalloutFold::Collapsed => Fold::Closed,
                },
                metadata: callout.metadata.clone(),
                title,
                body,
            });
        }
        NodeValue::CodeBlock(code) => {
            let lang = if code.fenced && !code.info.is_empty() {
                Some(code.info.split(' ').next().unwrap_or("").to_string())
                    .filter(|l| !l.is_empty())
            } else {
                None
            };
            out.push(Block::Code {
                lang,
                text: code.literal.clone(),
            });
        }
        NodeValue::Table(table) => {
            let mut header = Vec::new();
            let mut rows = Vec::new();
            for row in node.children() {
                if let NodeValue::TableRow(is_header) = &row.data().value {
                    let cells: Vec<Vec<Inline>> = row
                        .children()
                        .map(|cell| lower_inlines(cell, tokens))
                        .collect();
                    if *is_header {
                        header = cells;
                    } else {
                        rows.push(cells);
                    }
                }
            }
            out.push(Block::Table {
                alignments: table
                    .alignments
                    .iter()
                    .map(|a| match a {
                        comrak::nodes::TableAlignment::None => Alignment::None,
                        comrak::nodes::TableAlignment::Left => Alignment::Left,
                        comrak::nodes::TableAlignment::Center => Alignment::Center,
                        comrak::nodes::TableAlignment::Right => Alignment::Right,
                    })
                    .collect(),
                header,
                rows,
            });
        }
        NodeValue::ThematicBreak => out.push(Block::Rule),
        NodeValue::FootnoteDefinition(def) => {
            let mut body = Vec::new();
            for child in node.children() {
                lower_block(child, tokens, &mut body);
            }
            out.push(Block::FootnoteDef {
                label: def.name.clone(),
                body,
            });
        }
        NodeValue::HtmlBlock(html) => out.push(Block::Html(html.literal.clone())),
        // The body was already frontmatter-split; a `---` opener line inside
        // would be a thematic rule, not metadata.
        NodeValue::FrontMatter(_) => {}
        other => {
            // Anything else at block level we do not model (e.g. math alone
            // in a paragraph is handled by the Paragraph branch above).
            let _ = other;
        }
    }
}

fn lower_list_item<'a>(item: &'a AstNode<'a>, tokens: &[Token]) -> ListItem {
    let mut body = Vec::new();
    for child in item.children() {
        lower_block(child, tokens, &mut body);
    }
    let block_id = move_paragraph_block_id(&mut body);
    ListItem {
        task: None,
        block_id,
        body,
    }
}

/// A trailing `^id` on the item's last paragraph belongs to the item.
fn move_paragraph_block_id(body: &mut [Block]) -> Option<String> {
    match body.last_mut() {
        Some(Block::Paragraph { block_id, .. }) => block_id.take(),
        _ => None,
    }
}

/// Extract `^block-id` from the end of a paragraph's inlines.
/// A paragraph that would become empty keeps its literal text (assumption).
fn take_trailing_block_id(inlines: &mut [Inline]) -> Option<String> {
    let last = inlines.last_mut()?;
    let Inline::Text(text) = last else {
        return None;
    };
    // Compute everything from owned data first, so the borrow of `text`
    // ends before we write back.
    let owned = text.clone();
    let trimmed = owned.trim_end();
    let (before, token) = trimmed.rsplit_once(char::is_whitespace)?;
    let id = token.strip_prefix('^')?;
    let id_ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !id_ok || before.trim().is_empty() {
        // Either not a valid id, or the whole paragraph is the id — in the
        // latter case leave it as literal text (assumption, see COMPAT.md).
        return None;
    }
    *text = before.to_string();
    Some(id.to_string())
}

fn is_empty_inlines(inlines: &[Inline]) -> bool {
    inlines.iter().all(|inline| match inline {
        Inline::Text(text) => text.trim().is_empty(),
        Inline::SoftBreak => true,
        _ => false,
    })
}

fn lower_inlines<'a>(node: &'a AstNode<'a>, tokens: &[Token]) -> Vec<Inline> {
    match &node.data().value {
        NodeValue::Text(text) => lower_text(text, tokens),
        NodeValue::Code(code) => vec![Inline::Code(code.literal.clone())],
        NodeValue::Emph => container(node, tokens, Inline::Emph),
        NodeValue::Strong => container(node, tokens, Inline::Strong),
        NodeValue::Strikethrough => container(node, tokens, Inline::Strike),
        NodeValue::Highlight => container(node, tokens, Inline::Highlight),
        NodeValue::Link(link) => vec![Inline::Link {
            url: link.url.clone(),
            title: (!link.title.is_empty()).then(|| link.title.clone()),
            content: flatten(node, tokens),
        }],
        NodeValue::Image(image) => vec![Inline::Image {
            url: image.url.clone(),
            alt: plain_text(node, tokens),
        }],
        NodeValue::FootnoteReference(footnote) => {
            vec![Inline::FootnoteRef(footnote.name.clone())]
        }
        NodeValue::Math(math) => vec![Inline::Math(math.literal.clone())],
        NodeValue::HtmlInline(html) => vec![Inline::Html(html.clone())],
        NodeValue::SoftBreak => vec![Inline::SoftBreak],
        NodeValue::LineBreak => vec![Inline::HardBreak],
        // comrak owns comments (ofm_comments); reading view drops them.
        NodeValue::CommentInline(_) | NodeValue::MultilineComment(_) => Vec::new(),
        // Escaped characters and unknown inline containers: descend.
        _ => flatten(node, tokens),
    }
}

fn container<'a>(
    node: &'a AstNode<'a>,
    tokens: &[Token],
    wrap: fn(Vec<Inline>) -> Inline,
) -> Vec<Inline> {
    vec![wrap(flatten(node, tokens))]
}

fn flatten<'a>(node: &'a AstNode<'a>, tokens: &[Token]) -> Vec<Inline> {
    let mut out = Vec::new();
    for child in node.children() {
        out.extend(lower_inlines(child, tokens));
    }
    out
}

fn plain_text<'a>(node: &'a AstNode<'a>, tokens: &[Token]) -> String {
    let mut text = String::new();
    for inline in flatten(node, tokens) {
        match inline {
            Inline::Text(t) => text.push_str(&t),
            Inline::SoftBreak => text.push(' '),
            Inline::HardBreak => text.push(' '),
            _ => {}
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> Doc {
        parse_note(src)
    }

    fn paragraph_text(block: &Block) -> String {
        match block {
            Block::Paragraph { content, .. } => {
                let mut text = String::new();
                for inline in content {
                    if let Inline::Text(t) = inline {
                        text.push_str(t);
                    }
                }
                text
            }
            _ => String::new(),
        }
    }

    #[test]
    fn callouts_map_from_comrak() {
        let doc = parsed("> [!note] Custom title\n> body\n");
        assert_eq!(doc.blocks.len(), 1);
        match &doc.blocks[0] {
            Block::Callout {
                kind,
                fold,
                metadata,
                title,
                body,
            } => {
                assert_eq!(kind, "note");
                assert_eq!(fold, &Fold::None);
                assert_eq!(*metadata, None);
                assert!(matches!(title.as_slice(), [Inline::Text(t)] if t == "Custom title"));
                assert!(paragraph_text(&body[0]).contains("body"));
            }
            other => panic!("expected callout, got {other:?}"),
        }
    }

    #[test]
    fn callout_folding_and_metadata() {
        let doc = parsed("> [!tip|wide]+\n> body\n");
        match &doc.blocks[0] {
            Block::Callout {
                kind,
                fold,
                metadata,
                ..
            } => {
                assert_eq!(kind, "tip");
                assert_eq!(fold, &Fold::Open);
                assert_eq!(metadata.as_deref(), Some("wide"));
            }
            other => panic!("expected callout, got {other:?}"),
        }
        let doc = parsed("> [!warning]-\n> collapsed\n");
        match &doc.blocks[0] {
            Block::Callout { fold, .. } => assert_eq!(fold, &Fold::Closed),
            other => panic!("expected callout, got {other:?}"),
        }
    }

    #[test]
    fn comments_are_dropped_even_spanning_blocks() {
        let doc =
            parsed("before %%hidden [[link]]%% after\n\n%%multi\nline\ncomment%%\n\nafter2\n");
        let text: Vec<String> = doc.blocks.iter().map(paragraph_text).collect();
        assert!(text[0].contains("before") && text[0].contains("after"));
        assert!(!text[0].contains("hidden"));
        // The comment-only paragraph is dropped entirely.
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(text[1], "after2");
    }

    #[test]
    fn block_ids_paragraph_and_list_item() {
        let doc = parsed("para text ^abc-1\n\n- item ^li_id\n- plain\n");
        match &doc.blocks[0] {
            Block::Paragraph { content, block_id } => {
                assert_eq!(block_id.as_deref(), Some("abc-1"));
                assert!(content
                    .iter()
                    .any(|i| matches!(i, Inline::Text(t) if t.starts_with("para text"))));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
        match &doc.blocks[1] {
            Block::List { items, .. } => {
                assert_eq!(items[0].block_id.as_deref(), Some("li_id"));
                assert!(items[0].task.is_none());
                assert_eq!(items[1].block_id, None);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn lone_block_id_stays_literal() {
        // Assumption: Obsidian shows a lone ^id as text.
        let doc = parsed("^just-an-id\n");
        assert!(doc
            .blocks
            .iter()
            .any(|b| paragraph_text(b).contains("^just-an-id")));
    }

    #[test]
    fn tasks_map_checked_and_unchecked() {
        let doc = parsed("- [ ] open\n- [x] done\n");
        match &doc.blocks[0] {
            Block::List { items, .. } => {
                assert_eq!(items[0].task, Some(false));
                assert_eq!(items[1].task, Some(true));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn math_inline_display_and_currency() {
        let doc = parsed(
            "Euler: $e^{i\\pi}$ wrote.\n\n$$\na^2 + b^2 = c^2\n$$\n\nCosts $5 and $10 each.\n",
        );
        // currency stays text
        assert!(paragraph_text(&doc.blocks[2]).contains("Costs $5 and $10"));
        // inline math
        match &doc.blocks[0] {
            Block::Paragraph { content, .. } => assert!(content
                .iter()
                .any(|i| matches!(i, Inline::Math(t) if t.contains("e^{i\\pi}")))),
            other => panic!("expected paragraph, got {other:?}"),
        }
        // display math alone in a paragraph becomes a block
        assert!(matches!(&doc.blocks[1], Block::Math(t) if t.contains("a^2")));
    }

    #[test]
    fn heading_id_from_attributes() {
        let doc = parsed("# Intro {#custom-id}\n");
        match &doc.blocks[0] {
            Block::Heading { id, .. } => assert_eq!(id.as_deref(), Some("custom-id")),
            other => panic!("expected heading, got {other:?}"),
        }
    }

    #[test]
    fn wikilinks_survive_emphasis_splitting() {
        let doc = parsed("see [[a *b* c]]\n");
        match &doc.blocks[0] {
            Block::Paragraph { content, .. } => assert!(content
                .iter()
                .any(|i| matches!(i, Inline::WikiLink(w) if w.target.note == "a *b* c"))),
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn block_embed_is_recognized() {
        let doc = parsed("![[Note]]\n");
        assert!(matches!(&doc.blocks[0], Block::EmbedBlock(e) if e.target.note == "Note"));
        let doc = parsed("![[img.png|300x200]]\n");
        match &doc.blocks[0] {
            Block::EmbedBlock(e) => assert_eq!(e.size, Some((300, Some(200)))),
            other => panic!("expected embed, got {other:?}"),
        }
    }

    #[test]
    fn tables_carry_alignment_and_embed_cells() {
        let doc = parsed("| L | C | R |\n|---|:-:|--:|\n| a | ![[pic.png]] | b |\n");
        match &doc.blocks[0] {
            Block::Table {
                alignments, rows, ..
            } => {
                assert_eq!(
                    alignments,
                    &[Alignment::None, Alignment::Center, Alignment::Right]
                );
                assert!(matches!(
                    &rows[0][1][0],
                    Inline::Embed(e) if e.target.note == "pic.png"
                ));
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn escaped_hash_is_not_a_tag_in_ir() {
        let doc = parsed("many \\#ActuallyAutistic people #real\n");
        match &doc.blocks[0] {
            Block::Paragraph { content, .. } => {
                assert!(!content
                    .iter()
                    .any(|i| matches!(i, Inline::Tag(t) if t == "ActuallyAutistic")));
                assert!(content
                    .iter()
                    .any(|i| matches!(i, Inline::Tag(t) if t == "real")));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn inline_footnotes_become_references_and_definitions() {
        let doc = parsed("A note^[the note body] here.\n");
        match &doc.blocks[0] {
            Block::Paragraph { content, .. } => {
                assert!(content.iter().any(|i| matches!(i, Inline::FootnoteRef(_))));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
        assert!(matches!(&doc.blocks[1], Block::FootnoteDef { .. }));
    }

    #[test]
    fn parse_keeps_frontmatter_verbatim() {
        let src = "---\ntitle: T\n---\n\nbody [[Note]]\n";
        let doc = parsed(src);
        assert_eq!(doc.frontmatter_raw.as_deref(), Some("---\ntitle: T\n---\n"));
        // body only is parsed: frontmatter content never leaks into blocks
        assert!(doc
            .blocks
            .iter()
            .all(|b| !paragraph_text(b).contains("title: T")));
    }
}
