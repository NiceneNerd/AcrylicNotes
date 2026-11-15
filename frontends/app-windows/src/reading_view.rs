//! Reading-mode native element construction.
//!
//! Converts a [`vault_md::Doc`] into WinUI elements ([`View`]) for the
//! reading renderer. Block-level structure (paragraphs, headings, lists,
//! quotes, callouts, code, ...) becomes nested [`StackPanel`]s of
//! [`TextBlock`]s; inline formatting (strong, code, links, tags) becomes
//! per-inline [`TextBlock`]s with semantic font weight or
//! [`HyperlinkButton`]s.
//!
//! This is pure rendering: no shell state, no commands. Link click
//! handling is wired by the shell when reading mode is mounted; this
//! module only builds the element tree. All functions are tested
//! headlessly under Wine via the `RecordingRuntime`.

use std::collections::HashMap;
use vault_app::markdown::plain_inlines;
use vault_md::{Block, Doc, Inline};
use windows_reactor::{
    Border, Callback, ChildrenControl, ContentControl, FontWeight, HyperlinkButton, Image,
    KeyedView, LayoutControl, Orientation, StackPanel, Stretch, TextBlock, View,
};

use super::reading::WindowsMarkdownRenderer;

/// Where a clicked link navigates (plan §10.3's shared link commands,
/// materialized): the renderer emits the decision, the shell maps it to
/// kernel commands (`OpenExternal` for URLs, `OpenNote` for wiki links,
/// resolution and jumps decided by the shell at click time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkNav {
    /// An external URL — the kernel hands it to the platform's
    /// `ExternalOpener` (failures surface on the error line).
    Url(String),
    /// A wiki-link target note — the kernel opens or focuses its tab.
    Note(String),
    /// A relative Markdown link's raw target (`notes/b.md`,
    /// `../media/x.png`) — the shell resolves it through the vault index
    /// at click time: a note opens a tab, an attachment goes to the
    /// platform opener, an unresolved target to the OS default.
    Relative(String),
    /// A tag clicked in the reading view (`#work`) — the shell selects
    /// it (shell-local filter) and switches the sidebar to the tags
    /// panel, the same route the Linux app's `tag:` links take.
    Tag(String),
    /// A footnote reference clicked in the reading view — the shell
    /// resolved the definition's block index at render time and the
    /// click asks the kernel to anchor it (the Border marker). A
    /// reference without a definition renders as plain text.
    Footnote(usize),
}

/// Render a whole document into reading-mode WinUI elements.
///
/// Each top-level block becomes a keyed child of a vertical stack, keyed
/// by its block index so reconciliation preserves identity across edits.
/// Obsidian's order (and the Linux app's, `reading.rs::reading_order`):
/// prose first, then the footnote definitions gathered under a rule —
/// each definition keeps its own block index as key, so a footnote
/// reference still jumps to it. Blocks matching `anchor` are wrapped in
/// a `Border` (the kernel scroll anchor's visible marker; no toolkit
/// scroll API exists yet). `on_link` turns a clicked link's navigation
/// decision into the click callback the shell wires to kernel commands.
pub fn doc_view(
    doc: &Doc,
    config: &WindowsMarkdownRenderer,
    anchor: Option<usize>,
    on_link: &dyn Fn(LinkNav) -> Callback<()>,
) -> View {
    // Footnote label → definition block index, resolved once per render
    // so references never dangle: a ref without a definition renders as
    // plain text instead of a dead link.
    let footnote_blocks: std::collections::HashMap<String, usize> = doc
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| match block {
            Block::FootnoteDef { label, .. } => Some((label.clone(), i)),
            _ => None,
        })
        .collect();
    let prose: Vec<KeyedView> = doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| !matches!(block, Block::FootnoteDef { .. }))
        .map(|(i, block)| {
            KeyedView::new(
                i.to_string(),
                anchored(
                    block_view(block, config, &footnote_blocks, on_link),
                    anchor == Some(i),
                ),
            )
        })
        .collect();
    let footnotes: Vec<KeyedView> = doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| matches!(block, Block::FootnoteDef { .. }))
        .map(|(i, block)| {
            KeyedView::new(
                i.to_string(),
                anchored(
                    block_view(block, config, &footnote_blocks, on_link),
                    anchor == Some(i),
                ),
            )
        })
        .collect();

    let mut children = prose;
    if !footnotes.is_empty() {
        // The hairline between prose and the gathered footnotes.
        children.push(KeyedView::new(
            "footnote-rule",
            View::from(TextBlock::new().text("─")),
        ));
        children.extend(footnotes);
    }
    StackPanel::new()
        .orientation(Orientation::Vertical)
        .keyed_children(children)
}

/// Wrap a block in a marked-up Border when it is the jump anchor (the
/// kernel scroll anchor's visible marker); a pass-through otherwise.
fn anchored(view: View, is_anchor: bool) -> View {
    if is_anchor {
        Border::new().border_thickness(1.0).content(view)
    } else {
        view
    }
}

/// Render a single block into WinUI elements.
fn block_view(
    block: &Block,
    config: &WindowsMarkdownRenderer,
    footnote_blocks: &HashMap<String, usize>,
    on_link: &dyn Fn(LinkNav) -> Callback<()>,
) -> View {
    match block {
        Block::Paragraph { content, .. } => StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children(content.iter().enumerate().map(|(i, inline)| {
                KeyedView::new(
                    i.to_string(),
                    inline_view(inline, config, footnote_blocks, on_link),
                )
            })),
        Block::Heading { level, content, .. } => {
            let size = match *level {
                1 => 28.0,
                2 => 24.0,
                3 => 20.0,
                _ => 16.0,
            };
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .keyed_children(content.iter().enumerate().map(|(i, inline)| {
                    KeyedView::new(
                        i.to_string(),
                        inline_view_with_style(
                            inline,
                            config,
                            size,
                            None,
                            footnote_blocks,
                            on_link,
                        ),
                    )
                }))
        }
        Block::Code { text, lang, .. } => {
            let label = match lang {
                Some(lang) => format!("```{lang}\n{text}\n```"),
                None => format!("```\n{text}\n```"),
            };
            TextBlock::new().text(label).into()
        }
        Block::Math(text) => TextBlock::new().text(text).into(),
        Block::Rule => TextBlock::new().text("─").into(),
        Block::List { items, .. } => StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children(items.iter().enumerate().map(|(i, item)| {
                KeyedView::new(
                    i.to_string(),
                    list_item_view(item, config, footnote_blocks, on_link),
                )
            })),
        Block::Quote(inner) => StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children(inner.iter().enumerate().map(|(i, b)| {
                KeyedView::new(
                    i.to_string(),
                    block_view(b, config, footnote_blocks, on_link),
                )
            })),
        Block::Callout { title, body, .. } => {
            let mut children: Vec<KeyedView> = title
                .iter()
                .enumerate()
                .map(|(i, inline)| {
                    KeyedView::new(
                        format!("title-{i}"),
                        inline_view_with_style(
                            inline,
                            config,
                            14.0,
                            Some(FontWeight::SEMI_BOLD),
                            footnote_blocks,
                            on_link,
                        ),
                    )
                })
                .collect();
            children.extend(body.iter().enumerate().map(|(i, b)| {
                KeyedView::new(
                    format!("body-{i}"),
                    block_view(b, config, footnote_blocks, on_link),
                )
            }));
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .keyed_children(children)
        }
        Block::Table { header, rows, .. } => StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children(
                std::iter::once(KeyedView::new(
                    "header",
                    table_row_view(header, config, footnote_blocks),
                ))
                .chain(rows.iter().enumerate().map(|(i, row)| {
                    KeyedView::new(
                        format!("row-{i}"),
                        table_row_view(row, config, footnote_blocks),
                    )
                })),
            ),
        Block::EmbedBlock(_) => TextBlock::new().text("(embedded note)").into(),
        // Defensive: doc_view filters footnote definitions out, but the
        // match must be total so block_view stays reusable.
        Block::FootnoteDef { label, body, .. } => {
            let mut children: Vec<KeyedView> = vec![KeyedView::new(
                "label",
                View::from(TextBlock::new().text(format!("[^{label}]"))),
            )];
            children.extend(body.iter().enumerate().map(|(i, b)| {
                KeyedView::new(
                    i.to_string(),
                    block_view(b, config, footnote_blocks, on_link),
                )
            }));
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .keyed_children(children)
        }
        Block::Html(text) => TextBlock::new().text(text).into(),
    }
}

/// Render a list item (task checkbox marker + body blocks).
fn list_item_view(
    item: &vault_md::ListItem,
    config: &WindowsMarkdownRenderer,
    footnote_blocks: &HashMap<String, usize>,
    on_link: &dyn Fn(LinkNav) -> Callback<()>,
) -> View {
    let prefix = match item.task {
        Some(true) => "[x] ",
        Some(false) => "[ ] ",
        None => "- ",
    };
    let prefix_view: View = View::from(TextBlock::new().text(prefix));
    let mut children: Vec<KeyedView> = vec![KeyedView::new("prefix", prefix_view)];
    children.extend(item.body.iter().enumerate().map(|(i, b)| {
        KeyedView::new(
            i.to_string(),
            block_view(b, config, footnote_blocks, on_link),
        )
    }));
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .keyed_children(children)
}

/// Render a table row (each cell inline-projected).
fn table_row_view(
    cells: &[Vec<Inline>],
    config: &WindowsMarkdownRenderer,
    footnote_blocks: &HashMap<String, usize>,
) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .keyed_children(cells.iter().enumerate().map(|(i, cell)| {
            KeyedView::new(
                i.to_string(),
                inline_cells_view(cell, config, footnote_blocks),
            )
        }))
}

/// Render a cell's inlines as plain text.
fn inline_cells_view(
    inlines: &[Inline],
    _config: &WindowsMarkdownRenderer,
    _footnote_blocks: &HashMap<String, usize>,
) -> View {
    let text = plain_inlines(inlines);
    TextBlock::new().text(text).into()
}

/// Render a single inline into WinUI elements (body text size).
fn inline_view(
    inline: &Inline,
    config: &WindowsMarkdownRenderer,
    footnote_blocks: &HashMap<String, usize>,
    on_link: &dyn Fn(LinkNav) -> Callback<()>,
) -> View {
    inline_view_with_style(inline, config, 14.0, None, footnote_blocks, on_link)
}

/// A text block with the size and any explicit weight applied.
fn styled_text(text: String, size: f64, font_weight: Option<FontWeight>) -> TextBlock {
    let mut block = TextBlock::new().text(text).font_size(size);
    if let Some(weight) = font_weight {
        block = block.font_weight(weight);
    }
    block
}

/// Render a single inline with a given font size and an explicit font
/// weight (headings scale the size; callout titles set a weight).
/// `Strong` always forces bold regardless of the explicit weight.
fn inline_view_with_style(
    inline: &Inline,
    config: &WindowsMarkdownRenderer,
    size: f64,
    font_weight: Option<FontWeight>,
    footnote_blocks: &HashMap<String, usize>,
    on_link: &dyn Fn(LinkNav) -> Callback<()>,
) -> View {
    match inline {
        Inline::Text(t) => View::from(styled_text(t.clone(), size, font_weight)),
        Inline::Strong(children) => {
            let text = plain_inlines(children);
            View::from(
                TextBlock::new()
                    .text(text)
                    .font_size(size)
                    .font_weight(FontWeight::BOLD),
            )
        }
        Inline::Emph(children) => {
            View::from(styled_text(plain_inlines(children), size, font_weight))
        }
        Inline::Code(t) => View::from(styled_text(t.clone(), size, font_weight)),
        Inline::Math(t) => View::from(styled_text(t.clone(), size, font_weight)),
        Inline::Link { url, content, .. } => {
            let label = plain_inlines(content);
            if url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("mailto:")
            {
                // The kernel owns the open (plan §10.3): the click emits
                // `OpenExternal`, the effect runs against the injected
                // ExternalOpener, and failures surface on the error line.
                // No navigate_uri: the control must not bypass the kernel.
                let click = on_link(LinkNav::Url(url.clone()));
                HyperlinkButton::new().on_click(click).content(styled_text(
                    label,
                    size,
                    font_weight,
                ))
            } else {
                // Relative link: the shell resolves it through the vault
                // index at click time (note tab, attachment open, or OS
                // default — the Linux app's three-way split).
                let click = on_link(LinkNav::Relative(url.clone()));
                HyperlinkButton::new().on_click(click).content(styled_text(
                    label,
                    size,
                    font_weight,
                ))
            }
        }
        Inline::WikiLink(wl) => {
            let label = match &wl.alias {
                Some(alias) => plain_inlines(alias),
                None => wl.target.note.clone(),
            };
            // Invoking a wiki link asks the kernel to open (or focus) the
            // target note's tab — the same command the sidebar tree sends.
            let click = on_link(LinkNav::Note(wl.target.note.clone()));
            HyperlinkButton::new()
                .on_click(click)
                .content(styled_text(label, size, font_weight))
        }
        Inline::Tag(t) => {
            // A reading-view tag routes to the tags panel with the tag
            // selected (the Linux app's `tag:` link scheme): a
            // Hyperlink-equivalent callback, never a raw open.
            let click = on_link(LinkNav::Tag(t.clone()));
            HyperlinkButton::new().on_click(click).content(styled_text(
                format!("#{t}"),
                size,
                font_weight,
            ))
        }
        Inline::FootnoteRef(r) => {
            // A footnote reference jumps to its definition: the shell
            // resolved the block index at render time. A ref without a
            // definition renders as plain text instead of a dead link.
            match footnote_blocks.get(r) {
                Some(&block) => {
                    let click = on_link(LinkNav::Footnote(block));
                    HyperlinkButton::new().on_click(click).content(styled_text(
                        format!("^{r}"),
                        size,
                        font_weight,
                    ))
                }
                None => View::from(styled_text(format!("^{r}"), size, font_weight)),
            }
        }
        Inline::Image { url, alt } => {
            // A local vault raster renders inline (the shell installs the
            // resolver); remote URLs and unreadable targets keep the text
            // placeholder, the same contract the Linux shell follows.
            match config
                .show_images
                .then_some(config.image_resolver.as_ref())
                .flatten()
            {
                Some(resolve) => match resolve(url) {
                    Some(image) => View::from(
                        Image::new()
                            .source_data(image)
                            .stretch(Stretch::Uniform)
                            .max_width(600.0),
                    ),
                    None => View::from(styled_text(format!("[img: {alt}]"), size, font_weight)),
                },
                None => {
                    let label = if config.show_images {
                        format!("[img: {alt}]")
                    } else {
                        format!("[img hidden: {alt}]")
                    };
                    View::from(styled_text(label, size, font_weight))
                }
            }
        }
        Inline::Embed(embed) => {
            // `![[image.png]]` transcludes a local image when it resolves;
            // everything else (note embeds, unresolvable targets) stays the
            // placeholder until transclusion renders real note bodies.
            if config.show_images {
                if let Some(resolve) = config.image_resolver.as_ref() {
                    if let Some(image) = resolve(&embed.target.note) {
                        return View::from(
                            Image::new()
                                .source_data(image)
                                .stretch(Stretch::Uniform)
                                .max_width(600.0),
                        );
                    }
                }
            }
            View::from(styled_text(String::from("(embed)"), size, font_weight))
        }
        Inline::InlineFootnote(children) => {
            View::from(styled_text(plain_inlines(children), size, font_weight))
        }
        Inline::Html(t) => View::from(styled_text(t.clone(), size, font_weight)),
        Inline::SoftBreak | Inline::HardBreak => {
            View::from(styled_text(String::from(" "), size, font_weight))
        }
        Inline::Strike(children) => {
            View::from(styled_text(plain_inlines(children), size, font_weight))
        }
        Inline::Highlight(children) => {
            View::from(styled_text(plain_inlines(children), size, font_weight))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_md::{ListItem, WikiLink, WikiTarget};
    use windows_reactor::test::{
        EventId, NodeId, PropertyId, PropertyValue, Pump, RecordingRuntime,
    };

    fn doc_with(blocks: Vec<Block>) -> Doc {
        Doc {
            frontmatter_raw: None,
            blocks,
            spans: Vec::new(),
        }
    }

    fn render(doc: &Doc) -> (Pump<RecordingRuntime>, NodeId) {
        render_anchored(doc, None)
    }

    /// Render with an explicit jump anchor (the kernel scroll anchor).
    fn render_anchored(doc: &Doc, anchor: Option<usize>) -> (Pump<RecordingRuntime>, NodeId) {
        let mut pump = Pump::new(RecordingRuntime::default());
        // Render tests only assert the tree, not click routing: the stub
        // factory hands back a do-nothing callback.
        let on_link = |_nav: LinkNav| Callback::new(|()| ());
        pump.mount_view(doc_view(
            doc,
            &WindowsMarkdownRenderer::default(),
            anchor,
            &on_link,
        ))
        .expect("mount");
        let window = pump.window().expect("window node");
        (pump, window)
    }

    /// Find a TextBlock whose text exactly matches `expected`.
    fn find_text(rt: &RecordingRuntime, from: NodeId, expected: &str) -> Option<NodeId> {
        let node = rt.node(from)?;
        if let Some(PropertyValue::Str(t)) = node.property(PropertyId::TextBlockText) {
            if t == expected {
                return Some(from);
            }
        }
        for &child in node.children() {
            if let Some(found) = find_text(rt, child, expected) {
                return Some(found);
            }
        }
        None
    }

    /// Collect all TextBlock texts in document order.
    fn collect_texts(rt: &RecordingRuntime, from: NodeId, out: &mut Vec<String>) {
        if let Some(node) = rt.node(from) {
            if let Some(PropertyValue::Str(t)) = node.property(PropertyId::TextBlockText) {
                out.push(t.clone());
            }
            for &child in node.children() {
                collect_texts(rt, child, out);
            }
        }
    }

    fn paragraph(content: Vec<Inline>) -> Block {
        Block::Paragraph {
            content,
            block_id: None,
        }
    }

    #[test]
    fn paragraph_renders_as_text_block() {
        let doc = doc_with(vec![paragraph(vec![Inline::Text("hello world".into())])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "hello world").is_some());
    }

    #[test]
    fn heading_renders_with_larger_font() {
        let doc = doc_with(vec![Block::Heading {
            level: 1,
            content: vec![Inline::Text("Title".into())],
            id: None,
        }]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "Title").is_some());
        let title_node = find_text(rt, window, "Title").expect("title text block");
        assert_eq!(
            rt.node(title_node)
                .unwrap()
                .property(PropertyId::TextBlockFontSize),
            Some(&PropertyValue::F64(28.0)),
            "h1 uses 28pt font"
        );
    }

    #[test]
    fn strong_renders_bold() {
        let doc = doc_with(vec![paragraph(vec![Inline::Strong(vec![Inline::Text(
            "bold".into(),
        )])])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "bold").is_some());
        let bold_node = find_text(rt, window, "bold").expect("bold text block");
        assert_eq!(
            rt.node(bold_node)
                .unwrap()
                .property(PropertyId::TextBlockFontWeight),
            Some(&PropertyValue::FontWeight(FontWeight::BOLD)),
            "strong text is bold"
        );
    }

    #[test]
    fn code_block_renders_with_fences() {
        let doc = doc_with(vec![Block::Code {
            lang: Some("rust".into()),
            text: "let x = 1;".into(),
        }]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(
            find_text(rt, window, "```rust\nlet x = 1;\n```").is_some(),
            "code block has language fences"
        );
    }

    /// Find the node owning the TextBlock child whose text is exactly
    /// `label` (a button's content label makes its owner the button).
    fn find_owner_of_label(rt: &RecordingRuntime, from: NodeId, label: &str) -> Option<NodeId> {
        let node = rt.node(from)?;
        for &child in node.children() {
            if let Some(child_node) = rt.node(child) {
                if child_node.property(PropertyId::TextBlockText)
                    == Some(&PropertyValue::Str(label.to_string()))
                {
                    return Some(from);
                }
                if let Some(found) = find_owner_of_label(rt, child, label) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn external_link_renders_as_a_clickable_hyperlink() {
        let doc = doc_with(vec![paragraph(vec![Inline::Link {
            url: "https://example.com".into(),
            title: None,
            content: vec![Inline::Text("the docs".into())],
        }])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        // The label renders (the button's content is a child TextBlock),
        // and the click event is active on its owner.
        let button = find_owner_of_label(rt, window, "the docs").expect("link button recorded");
        assert!(
            pump.event_revision(button, EventId::HyperlinkButtonClick)
                .is_some(),
            "the external link's click wiring is active"
        );
    }

    #[test]
    fn wiki_link_renders_clickable() {
        let doc = doc_with(vec![paragraph(vec![Inline::WikiLink(WikiLink {
            target: WikiTarget {
                note: "Meeting Notes".into(),
                heading: None,
                block: None,
            },
            alias: None,
        })])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "Meeting Notes").is_some());
        let button =
            find_owner_of_label(rt, window, "Meeting Notes").expect("wiki link button recorded");
        assert!(
            pump.event_revision(button, EventId::HyperlinkButtonClick)
                .is_some(),
            "the wiki link's click wiring is active"
        );
    }

    #[test]
    fn list_renders_items_with_dash_prefix() {
        let doc = doc_with(vec![Block::List {
            ordered: false,
            start: None,
            tight: true,
            items: vec![
                ListItem {
                    task: None,
                    block_id: None,
                    body: vec![paragraph(vec![Inline::Text("first".into())])],
                },
                ListItem {
                    task: None,
                    block_id: None,
                    body: vec![paragraph(vec![Inline::Text("second".into())])],
                },
            ],
        }]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        // Each item is a marker TextBlock followed by its body blocks.
        let first = texts
            .iter()
            .position(|t| t == "first")
            .expect("first item text recorded");
        let second = texts
            .iter()
            .position(|t| t == "second")
            .expect("second item text recorded");
        assert!(first < second, "items render in order");
        // Both items carry a dash marker before their text.
        let markers = texts.iter().filter(|t| t.as_str() == "- ").count();
        assert_eq!(markers, 2, "each item has a dash marker");
    }

    #[test]
    fn task_list_renders_checkboxes() {
        let doc = doc_with(vec![Block::List {
            ordered: false,
            start: None,
            tight: true,
            items: vec![ListItem {
                task: Some(true),
                block_id: None,
                body: vec![paragraph(vec![Inline::Text("done".into())])],
            }],
        }]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        assert!(
            texts.iter().any(|t| t == "[x] "),
            "checked task has [x] marker"
        );
        assert!(texts.iter().any(|t| t == "done"), "task text recorded");
    }

    #[test]
    fn callout_renders_title_and_body() {
        let doc = doc_with(vec![Block::Callout {
            kind: "note".into(),
            fold: vault_md::Fold::None,
            metadata: None,
            title: vec![Inline::Text("Heads up".into())],
            body: vec![paragraph(vec![Inline::Text("body text".into())])],
        }]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "Heads up").is_some());
        assert!(find_text(rt, window, "body text").is_some());
    }

    #[test]
    fn tag_renders_with_hash_prefix() {
        let doc = doc_with(vec![paragraph(vec![Inline::Tag("rust".into())])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "#rust").is_some());
        // Tags are Hyperlink-equivalent callbacks (plan §10.3), routed to
        // the tags panel by the shell.
        let button = find_owner_of_label(rt, window, "#rust").expect("tag button recorded");
        assert!(
            pump.event_revision(button, EventId::HyperlinkButtonClick)
                .is_some(),
            "the tag's click wiring is active"
        );
    }

    #[test]
    fn footnote_ref_renders_with_caret() {
        // A ref without a definition renders as plain text (not clickable).
        let doc = doc_with(vec![paragraph(vec![Inline::FootnoteRef("1".into())])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "^1").is_some());
        let owner = find_owner_of_label(rt, window, "^1").expect("footnote ref text recorded");
        assert!(
            pump.event_revision(owner, EventId::HyperlinkButtonClick)
                .is_none(),
            "a dangling footnote ref is not a button"
        );
    }

    #[test]
    fn footnote_ref_with_definition_renders_clickable() {
        // A ref with a matching definition renders as a jump button.
        let doc = doc_with(vec![
            paragraph(vec![Inline::FootnoteRef("1".into())]),
            Block::FootnoteDef {
                label: "1".into(),
                body: vec![paragraph(vec![Inline::Text("definition".into())])],
            },
        ]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        assert!(find_text(rt, window, "^1").is_some());
        let button = find_owner_of_label(rt, window, "^1").expect("footnote ref button recorded");
        assert!(
            pump.event_revision(button, EventId::HyperlinkButtonClick)
                .is_some(),
            "the footnote ref's click wiring is active"
        );
    }

    #[test]
    fn relative_link_renders_clickable() {
        let doc = doc_with(vec![paragraph(vec![Inline::Link {
            url: "notes/b.md".into(),
            title: None,
            content: vec![Inline::Text("a sibling note".into())],
        }])]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        let button =
            find_owner_of_label(rt, window, "a sibling note").expect("relative link button");
        assert!(
            pump.event_revision(button, EventId::HyperlinkButtonClick)
                .is_some(),
            "the relative link's click wiring is active"
        );
    }

    #[test]
    fn footnote_defs_render_at_the_end_under_a_rule() {
        // Obsidian's order (and the Linux app's): prose first, then the
        // footnote definitions gathered under a hairline.
        let doc = doc_with(vec![
            paragraph(vec![Inline::Text("prose".into())]),
            Block::FootnoteDef {
                label: "1".into(),
                body: vec![paragraph(vec![Inline::Text("definition".into())])],
            },
        ]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        let prose = texts
            .iter()
            .position(|t| t == "prose")
            .expect("prose recorded");
        let rule = texts
            .iter()
            .position(|t| t == "─")
            .expect("the footnote rule is rendered");
        let marker = texts
            .iter()
            .position(|t| t == "[^1]")
            .expect("the definition's marker is rendered");
        let def = texts
            .iter()
            .position(|t| t == "definition")
            .expect("definition recorded");
        assert!(prose < rule, "the rule separates prose from footnotes");
        assert!(rule < marker && marker < def, "definitions follow the rule");
    }

    #[test]
    fn the_jump_anchor_wraps_its_block_in_a_border() {
        let doc = doc_with(vec![
            paragraph(vec![Inline::Text("first".into())]),
            paragraph(vec![Inline::Text("second".into())]),
        ]);
        let (pump, window) = render_anchored(&doc, Some(1));
        let rt = pump.runtime();
        assert!(
            find_border_wrapping(rt, window, "second"),
            "the anchored block sits in a Border marker"
        );
        assert!(
            !find_border_wrapping(rt, window, "first"),
            "non-anchored blocks are not wrapped"
        );
    }

    /// Whether any node in the tree under `from` is a `Border` (detected
    /// via its `BorderThickness` property — `MountedKind` is private in
    /// the toolkit) whose subtree contains a TextBlock with exactly
    /// `text`, i.e. the jump anchor's visible marker.
    fn find_border_wrapping(rt: &RecordingRuntime, from: NodeId, text: &str) -> bool {
        let node = match rt.node(from) {
            Some(node) => node,
            None => return false,
        };
        let is_anchor_marker = node.property(PropertyId::BorderBorderThickness).is_some()
            && find_text(rt, from, text).is_some();
        if is_anchor_marker {
            return true;
        }
        node.children()
            .iter()
            .any(|&child| find_border_wrapping(rt, child, text))
    }

    #[test]
    fn table_renders_rows_in_order() {
        let doc = doc_with(vec![Block::Table {
            alignments: vec![],
            header: vec![vec![Inline::Text("A".into())]],
            rows: vec![vec![vec![Inline::Text("b".into())]]],
        }]);
        let (pump, window) = render(&doc);
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        let a = texts.iter().position(|t| t == "A").expect("header cell");
        let b = texts.iter().position(|t| t == "b").expect("body cell");
        assert!(a < b, "header renders before body rows");
    }
}
