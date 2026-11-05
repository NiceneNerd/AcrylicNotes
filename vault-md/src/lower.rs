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
//!   id, and a paragraph that is ONLY `^id` is invisible: it attaches to the
//!   previous block (confirmed against real Obsidian in M3),
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
    // The body passed to comrak starts after the frontmatter: shift
    // comrak's 1-based body lines down onto FILE lines. `raw` ends with the
    // closing delimiter's own newline (or EOF), so its line count is exactly
    // the number of file lines before the body.
    let offset = split.raw.as_ref().map_or(0, |raw| raw.lines().count());
    let (blocks, body_spans) = lower_source_spanned(&split.body);
    let spans = body_spans
        .into_iter()
        .map(|span| crate::ir::SourceSpan {
            start: span.start + offset,
            end: span.end + offset,
        })
        .collect();
    Doc {
        frontmatter_raw: split.raw,
        blocks,
        spans,
    }
}

/// Lower a source fragment (no frontmatter expected) into blocks.
pub fn lower_source(src: &str) -> Vec<Block> {
    lower_source_spanned(src).0
}

/// Lower a source fragment into blocks plus each top-level block's source
/// line span (1-based, inclusive, relative to `src`).
pub fn lower_source_spanned(src: &str) -> (Vec<Block>, Vec<crate::ir::SourceSpan>) {
    let extracted = crate::inline::extract(src);
    lower_comrak_spanned(&extracted)
}

fn lower_comrak_spanned(extracted: &Extracted) -> (Vec<Block>, Vec<crate::ir::SourceSpan>) {
    let arena = Arena::new();
    let root = parse_document(&arena, &extracted.text, &comrak_options());
    // Each AST node lowers to 0..=1 top-level blocks; carry the node's
    // start line with each emitted block so spans survive the lone-id
    // attach pass (which removes blocks).
    let mut paired: Vec<(Block, usize)> = Vec::new();
    for child in root.children() {
        let start = child.data().sourcepos.start.line;
        let mut out = Vec::new();
        lower_block(child, &extracted.tokens, &mut out);
        for block in out {
            paired.push((block, start));
        }
    }
    attach_top_lone_block_ids(&mut paired);
    // A block spans from its first line to the line before the next
    // block: blank lines and interleaved comments attach to the block
    // above, which is what editor→preview scrolling wants. The last block
    // owns through the end of the text.
    let body_lines = extracted.text.lines().count().max(1);
    let mut blocks = Vec::with_capacity(paired.len());
    let mut spans = Vec::with_capacity(paired.len());
    for (i, (block, start)) in paired.iter().enumerate() {
        let end = match paired.get(i + 1) {
            Some(&(_, next)) => next.saturating_sub(1).max(*start),
            None => body_lines.max(*start),
        };
        blocks.push(block.clone());
        spans.push(crate::ir::SourceSpan { start: *start, end });
    }
    (blocks, spans)
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

/// Confirmed against real Obsidian (M3, 2026-10-31): a paragraph that holds
/// nothing but a `^block-id` is INVISIBLE in reading view — it renders no
/// text and attaches its id to the preceding block.
///
/// ```text
/// para text
/// ^abc          <- invisible; `para text` becomes the block `^abc`
/// ```
///
/// The id goes to the previous paragraph or, after a list, to the list's last
/// item. A block that already carries an id keeps it (first id wins).
fn attach_lone_block_ids(blocks: &mut Vec<Block>) {
    let mut i = 0;
    while i < blocks.len() {
        if let Some(id) = lone_block_id(&blocks[i]) {
            blocks.remove(i);
            if i > 0 {
                attach_block_id(&mut blocks[i - 1], &id);
            }
            // The block that shifted into place is not re-examined as a lone
            // id (a second `^id` line simply has nowhere to attach).
            continue;
        }
        match &mut blocks[i] {
            Block::List { items, .. } => {
                for item in items {
                    attach_lone_block_ids(&mut item.body);
                }
            }
            Block::Quote(body) => attach_lone_block_ids(body),
            Block::Callout { body, .. } => attach_lone_block_ids(body),
            Block::FootnoteDef { body, .. } => attach_lone_block_ids(body),
            _ => {}
        }
        i += 1;
    }
}

/// The top-level lone-id pass, keeping each block's body start line in
/// lockstep: a removed `^id` paragraph drops its line entry too, and the
/// id attaches to the block above (whose span now covers those lines).
fn attach_top_lone_block_ids(paired: &mut Vec<(Block, usize)>) {
    let mut i = 0;
    while i < paired.len() {
        if let Some(id) = lone_block_id(&paired[i].0) {
            paired.remove(i);
            if i > 0 {
                attach_block_id(&mut paired[i - 1].0, &id);
            }
            continue;
        }
        match &mut paired[i].0 {
            Block::List { items, .. } => {
                for item in items {
                    attach_lone_block_ids(&mut item.body);
                }
            }
            Block::Quote(body) => attach_lone_block_ids(body),
            Block::Callout { body, .. } => attach_lone_block_ids(body),
            Block::FootnoteDef { body, .. } => attach_lone_block_ids(body),
            _ => {}
        }
        i += 1;
    }
}

/// The id of a paragraph whose entire content is one `^id` token.
fn lone_block_id(block: &Block) -> Option<String> {
    let Block::Paragraph { content, .. } = block else {
        return None;
    };
    let [Inline::Text(text)] = content.as_slice() else {
        return None;
    };
    let trimmed = text.trim();
    let id = trimmed.strip_prefix('^')?;
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(id.to_string())
}

/// Attach `id` to a block that can carry one.
fn attach_block_id(block: &mut Block, id: &str) {
    match block {
        Block::Paragraph { block_id, .. } => {
            if block_id.is_none() {
                *block_id = Some(id.to_string());
            }
        }
        Block::List { items, .. } => {
            if let Some(item) = items.last_mut() {
                if item.block_id.is_none() {
                    item.block_id = Some(id.to_string());
                }
            }
        }
        // Headings, tables and rules have no block-id slot in our IR; the
        // id stays invisible (ASSUMPTION, see COMPAT.md).
        _ => {}
    }
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

    /// Confirmed against real Obsidian (M3): a lone `^id` line is invisible
    /// and attaches to the previous block.
    #[test]
    fn lone_block_id_attaches_to_previous_paragraph() {
        let doc = parsed("para text\n\n^just-an-id\n");
        assert_eq!(doc.blocks.len(), 1, "the lone id paragraph disappears");
        match &doc.blocks[0] {
            Block::Paragraph { content, block_id } => {
                assert_eq!(block_id.as_deref(), Some("just-an-id"));
                assert!(content
                    .iter()
                    .any(|i| matches!(i, Inline::Text(t) if t == "para text")));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn lone_block_id_after_list_attaches_to_last_item() {
        let doc = parsed("- item one\n- item two\n\n^list-id\n");
        match &doc.blocks[0] {
            Block::List { items, .. } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[1].block_id.as_deref(), Some("list-id"));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn lone_block_id_without_previous_block_is_invisible() {
        let doc = parsed("^just-an-id\n");
        assert!(
            doc.blocks.is_empty(),
            "a leading id has nothing to attach to"
        );
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

    #[test]
    fn spans_are_body_lines_without_frontmatter() {
        let src = "# Title\n\nFirst paragraph.\n\n- one\n- two\n";
        let doc = parsed(src);
        // Head: line 1. Paragraph: line 3. List: lines 5-6.
        let spans: Vec<(usize, usize)> = doc.spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, vec![(1, 2), (3, 4), (5, 6)]);
    }

    #[test]
    fn spans_shift_to_file_lines_with_frontmatter() {
        let src = "---\ntags: [a]\n---\n\n# Head\n\nBody text.\n";
        let doc = parsed(src);
        // Frontmatter is 3 file lines; the body starts on file line 4.
        assert_eq!(doc.frontmatter_line_count(), 3);
        let spans: Vec<(usize, usize)> = doc.spans.iter().map(|s| (s.start, s.end)).collect();
        // The heading owns the blank file line 6 too.
        assert_eq!(spans, vec![(5, 6), (7, 7)]);
    }

    #[test]
    fn blank_lines_and_comments_attach_to_the_block_above() {
        let src = "one\n\n%% secret %%\n\ntwo\n";
        let doc = parsed(src);
        // The comment-only paragraph is dropped; its line joins "one"'s span.
        let spans: Vec<(usize, usize)> = doc.spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, vec![(1, 4), (5, 5)]);
        assert_eq!(doc.blocks.len(), 2);
    }

    #[test]
    fn lone_block_id_lines_join_the_previous_block_span() {
        let src = "text\n\n^my-id\n\nnext\n";
        let doc = parsed(src);
        // The `^my-id` paragraph is removed; the first block keeps lines
        // 1-4 (blank line 2, the id line 3, blank line 4 all attach above).
        let spans: Vec<(usize, usize)> = doc.spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, vec![(1, 4), (5, 5)]);
        assert_eq!(doc.blocks.len(), 2);
        assert!(
            matches!(&doc.blocks[0], Block::Paragraph { block_id: Some(id), .. } if id == "my-id")
        );
    }

    #[test]
    fn block_at_line_maps_editor_lines_to_blocks() {
        let src = "# Head\n\nPara one.\ncontinues\n\n- item\n";
        let doc = parsed(src);
        // Spans: heading 1-2, paragraph 3-5, list 6-6.
        assert_eq!(doc.block_at_line(1), Some(0));
        assert_eq!(doc.block_at_line(2), Some(0)); // blank line under the head
        assert_eq!(doc.block_at_line(4), Some(1));
        assert_eq!(doc.block_at_line(5), Some(1)); // blank line
        assert_eq!(doc.block_at_line(6), Some(2));
        // Beyond the document: no block.
        assert_eq!(doc.block_at_line(9), None);
        // 0 is before every span: no block claims it.
        assert_eq!(doc.block_at_line(0), None);
    }

    #[test]
    fn container_blocks_carry_one_span() {
        let src = "> quote line\n> more\n\n```rust\nlet x = 1;\n```\n";
        let doc = parsed(src);
        let spans: Vec<(usize, usize)> = doc.spans.iter().map(|s| (s.start, s.end)).collect();
        // The quote owns the blank line 3 (blocks stretch to the next one).
        assert_eq!(spans, vec![(1, 3), (4, 6)]);
    }
}
