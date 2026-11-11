//! Reading view: `vault_md` IR → libcosmic widgets (M4 decision 2 — a custom
//! renderer, not HTML). Paragraphs wrap through iced's `rich_text`; every
//! interactive span carries a URI (`note:`, `wiki:`, `create:`, `tag:`,
//! `fnref:`, `scroll:`, `open:`) that `update` parses — the URI string is the
//! only channel from the renderer back to the app, which keeps the renderer
//! free of app state.
//!
//! Fidelity choices for M4a (documented in the milestone report):
//! - images render from disk (cached `Handle` per path); remote images are
//!   placeholders — no network.
//! - raw HTML blocks/spans render as plain text; a GUI cannot host arbitrary
//!   HTML, and the sanitized-HTML path is the web renderer's job.
//! - same-document `[[#Heading]]` links emit `scroll:` and are inert until
//!   M4b wires scrolling.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use cosmic::iced::core::text::Span;
use cosmic::iced::core::widget::Id;
use cosmic::iced::widget::{self, image::Handle as ImageHandle, rich_text, text};
use cosmic::iced::{border, font, Background, Color, Font, Length, Padding};
use cosmic::widget::{button, column, container, row, scrollable, Space};
use cosmic::Element;

use vault_core::{NoteSource, Vault, VaultIndex};
use vault_md::{parse_note, Block, Doc, Embed, Fold, Inline, ListItem, WikiLink};

use crate::app::{Message, Palette};

const EMBED_DEPTH_LIMIT: usize = 4;

/// Fold id of the frontmatter block. Block fold ids count up from 0 during
/// each render, so this fixed sentinel never collides with them.
pub const FRONTMATTER_FOLD: u64 = u64::MAX;

pub struct Ctx<'a> {
    pub vault: &'a Vault,
    pub index: &'a VaultIndex,
    /// The note currently being rendered; link resolution is relative to it.
    pub from: String,
    pub strict_breaks: bool,
    pub palette: Palette,
    /// The font the rendered prose uses: the user's custom family, or
    /// the system font when none is set.
    pub font: Font,
    /// The base text size in pixels. Headings scale from it, so the
    /// whole reading view grows and shrinks together.
    pub size: f32,
    /// Callouts toggled by hand (tab-scoped, index-based).
    pub folds: &'a HashSet<u64>,
    /// Decoded images, keyed by vault-relative path, so re-renders do not
    /// re-decode. `RefCell` because `view` takes `&self`.
    pub images: &'a RefCell<HashMap<String, ImageHandle>>,
}

/// The size of a heading at `level`, scaled from the reading view's base
/// size.
///
/// The factors keep the hierarchy the fixed sizes had (26/22/19/18/16 at
/// a 16-pixel base) while following the user's chosen size.
pub fn heading_size(level: u8, base: f32) -> f32 {
    let factor = match level {
        1 => 1.6,
        2 => 1.35,
        3 => 1.2,
        4 => 1.1,
        _ => 1.0,
    };
    (base * factor).round()
}

/// One heading for the outline panel (M4b): its top-level block index, the
/// heading level and its plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub block: usize,
    pub level: u8,
    pub text: String,
}

/// Top-level headings of a document, in document order.
pub fn outline(doc: &Doc) -> Vec<OutlineEntry> {
    doc.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| match block {
            Block::Heading { level, content, .. } => Some(OutlineEntry {
                block: i,
                level: *level,
                text: plain_inlines(content),
            }),
            _ => None,
        })
        .collect()
}

/// The plain text of a block, descending into nested structures (lists,
/// quotes, callouts, footnote definitions, tables). Embeds do not descend —
/// they pull other notes in at render time; their target name still counts
/// as text so `![[Note]]` is searchable in a coarse way.
pub fn block_text(block: &Block) -> String {
    match block {
        Block::Heading { content, .. } | Block::Paragraph { content, .. } => plain_inlines(content),
        Block::List { items, .. } => items
            .iter()
            .map(|item| {
                item.body
                    .iter()
                    .map(block_text)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" "),
        Block::Quote(body) | Block::FootnoteDef { body, .. } => {
            body.iter().map(block_text).collect::<Vec<_>>().join(" ")
        }
        Block::Callout { title, body, .. } => {
            let mut parts = vec![plain_inlines(title)];
            parts.extend(body.iter().map(block_text));
            parts.join(" ")
        }
        Block::Code { text, .. } | Block::Math(text) | Block::Html(text) => text.clone(),
        Block::Table { header, rows, .. } => header
            .iter()
            .chain(rows.iter().flatten())
            .map(|cell| cell.iter().map(inline_text).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join(" "),
        Block::Rule => String::new(),
        Block::EmbedBlock(embed) => embed.target.note.clone(),
    }
}

fn inline_text(inline: &Inline) -> String {
    match inline {
        Inline::Text(s) | Inline::Code(s) | Inline::Math(s) | Inline::Html(s) => s.clone(),
        Inline::Emph(inner)
        | Inline::Strong(inner)
        | Inline::Strike(inner)
        | Inline::Highlight(inner)
        | Inline::InlineFootnote(inner) => inline_texts(inner),
        Inline::Link { content, .. } => inline_texts(content),
        Inline::WikiLink(link) => link
            .alias
            .as_ref()
            .map(|alias| inline_texts(alias))
            .unwrap_or_else(|| link.target.note.clone()),
        Inline::Image { alt, .. } => alt.clone(),
        Inline::Embed(embed) => embed.target.note.clone(),
        Inline::Tag(t) => format!("#{t}"),
        Inline::FootnoteRef(l) => format!("[{l}]"),
        Inline::SoftBreak | Inline::HardBreak => " ".to_string(),
    }
}

/// The superscript digits of a footnote number: `12` becomes `¹²`.
///
/// iced's text layout has no baseline offset, so a raised marker is
/// spelled with the Unicode superscript characters. `None` for a label
/// that is not a number — a named footnote keeps its label.
fn superscript_digits(label: &str) -> Option<String> {
    if label.is_empty() || !label.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    const DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    Some(
        label
            .chars()
            .map(|c| DIGITS[c.to_digit(10).expect("an ASCII digit") as usize])
            .collect(),
    )
}

/// The plain text of a run of inlines.
fn inline_texts(inlines: &[Inline]) -> String {
    inlines.iter().map(inline_text).collect()
}

/// First top-level block whose text contains `query` (case-insensitive) —
/// used to scroll a search hit's reading view to the match.
pub fn find_block_with_text(doc: &Doc, query: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    doc.blocks
        .iter()
        .position(|block| block_text(block).to_lowercase().contains(&query))
}

/// Render a whole document (frontmatter preview + blocks).
pub fn render_doc<'a>(doc: &Doc, ctx: &Ctx<'a>) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(10).width(Length::Fill);
    if let Some(frontmatter) = render_frontmatter(doc, ctx) {
        col = col.push(frontmatter);
    }
    // Callout fold ids are assigned sequentially during this render. The
    // traversal is deterministic, so ids are stable between renders; a single
    // counter threaded through every nesting level (documents, callout bodies,
    // embeds) keeps them globally unique per tab.
    let mut fold_id = 0u64;
    let stack = vec![ctx.from.clone()];
    // The blocks are RENDERED in source order — the fold ids are positional
    // and the live preview counts them the same way — but PLACED in reading
    // order: the prose first, the footnote definitions last, under a rule.
    let (prose, footnotes) = reading_order(&doc.blocks);
    let mut rendered: Vec<Option<Element<'a, Message>>> = Vec::with_capacity(doc.blocks.len());
    for block in &doc.blocks {
        rendered.push(Some(render_block(block, ctx, 0, &stack, &mut fold_id)));
    }
    for i in prose {
        let element = rendered[i].take().expect("every block renders once");
        // Anchor every top-level block so outline/search jumps can measure
        // and scroll to it (see scroll.rs). The container is pass-through.
        col = col.push(container(element).id(Id::from(format!("block-{i}"))));
    }
    if !footnotes.is_empty() {
        // The rule: a hairline between the prose and the footnotes.
        col = col.push(cosmic::iced::widget::rule::horizontal(1));
        for i in footnotes {
            let element = rendered[i].take().expect("every block renders once");
            // The anchor keeps the block's own index, so a footnote
            // reference still jumps to its definition.
            col = col.push(container(element).id(Id::from(format!("block-{i}"))));
        }
    }
    col.into()
}

/// A document's blocks in reading order: the prose first, the footnote
/// definitions last.
///
/// Obsidian's reading view gathers the footnote definitions at the end of
/// the document, under a rule; the web renderer does the same (see
/// vault-md's `<section class="footnotes">`). The GUI follows.
fn reading_order(blocks: &[Block]) -> (Vec<usize>, Vec<usize>) {
    let mut prose = Vec::with_capacity(blocks.len());
    let mut footnotes = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        if matches!(block, Block::FootnoteDef { .. }) {
            footnotes.push(i);
        } else {
            prose.push(i);
        }
    }
    (prose, footnotes)
}

/// How many stop positions the fold-id cursor advances for this doc: one
/// per callout block, in the same traversal order `render_doc` assigns
/// ids (`Quote`, `List` items, `Callout` bodies and `FootnoteDef` bodies
/// recurse; embeds resolve at render time and are not counted — a callout
/// count change *inside* an embedded note isn't visible here, matching
/// the renderer's trick that the traversal cost must stay cheap).
///
/// Fold ids are positional, so two docs with the same count hand out the
/// same ids: fold state can be retained across an edit whenever the count
/// is unchanged, and must reset when a callout is added (new ids shift
/// the traversal) or removed.
pub(crate) fn foldable_count(doc: &Doc) -> usize {
    foldable_blocks(&doc.blocks)
}

fn foldable_blocks(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Callout { body, .. } => 1 + foldable_blocks(body),
            Block::Quote(body) => foldable_blocks(body),
            Block::List { items, .. } => items.iter().map(|item| foldable_blocks(&item.body)).sum(),
            Block::FootnoteDef { body, .. } => foldable_blocks(body),
            _ => 0,
        })
        .sum()
}

/// The frontmatter block under `render_doc`: foldable, rendered as
/// structured YAML instead of raw text. Nested maps and sequences are
/// indented so the metadata is readable at a glance. `None` when the note
/// has no frontmatter (no empty code box is rendered). The live preview's
/// frontmatter region reuses this.
pub(crate) fn render_frontmatter<'a>(doc: &Doc, ctx: &Ctx<'a>) -> Option<Element<'a, Message>> {
    let frontmatter = doc.frontmatter_raw.as_ref()?;
    let yaml = frontmatter_yaml(frontmatter)?;
    let palette = ctx.palette;
    let mut fm_col = column::Column::new().spacing(6);
    {
        // Frontmatter is metadata, not prose: fold it like a callout so it
        // can be tucked away while reading. It owns a fixed fold id (block
        // fold ids count up from 0, so this never collides).
        let open = !ctx.folds.contains(&FRONTMATTER_FOLD);
        let chevron = if open { "▾ " } else { "▸ " };
        fm_col = fm_col.push(
            button::custom(
                text(format!("{chevron}Frontmatter"))
                    .font(ctx.font)
                    .size(ctx.size)
                    .class(palette.accent),
            )
            .on_press(Message::ToggleFold(FRONTMATTER_FOLD)),
        );
        if open {
            match serde_norway::from_str::<serde_norway::Value>(yaml) {
                Ok(value) => {
                    fm_col = fm_col.push(render_yaml_value(&value, 0, ctx));
                }
                Err(_) => {
                    // Invalid YAML: fall back to showing the raw text.
                    fm_col = fm_col.push(
                        text(yaml.trim_end().to_string())
                            .font(cosmic::font::mono())
                            .size(12.0)
                            .class(palette.muted),
                    );
                }
            }
        }
    }
    Some(
        container(fm_col)
            .style(move |_| code_block_style(&palette))
            .into(),
    )
}

/// Extract the YAML content between the `---` delimiters from a raw
/// frontmatter block like `"---\ntitle: Test\n---\n"`.
fn frontmatter_yaml(raw: &str) -> Option<&str> {
    // Skip the opening `---` line.
    let start = raw.find('\n')? + 1;
    // Find the closing delimiter (`---` or `...` on its own line).
    let end = raw.rfind("\n---")?;
    if end <= start {
        return None;
    }
    Some(&raw[start..end])
}

/// Render one YAML value at the given nesting depth.
///
/// Indentation is done through layout (container padding), not through
/// space-prefixed text — proportional fonts make space-based indentation
/// misalign, so padding on the wrapping container keeps everything lined
/// up regardless of the font.
fn render_yaml_value<'a>(
    value: &serde_norway::Value,
    depth: u16,
    ctx: &Ctx<'a>,
) -> Element<'a, Message> {
    match value {
        serde_norway::Value::Null => text("null")
            .font(cosmic::font::mono())
            .size(12.0)
            .class(ctx.palette.muted)
            .into(),
        serde_norway::Value::Bool(b) => text(format!("{b}"))
            .font(ctx.font)
            .size(12.0)
            .class(ctx.palette.text)
            .into(),
        serde_norway::Value::Number(n) => {
            let s = if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                n.as_f64().map(|f| f.to_string()).unwrap_or_default()
            };
            text(s)
                .font(cosmic::font::mono())
                .size(12.0)
                .class(ctx.palette.text)
                .into()
        }
        serde_norway::Value::String(s) => {
            // Quote strings that look like they could be confused with
            // other YAML types, but keep plain strings unquoted.
            let displayed = if s.is_empty() || s.starts_with(' ') || s.ends_with(' ') {
                format!("{:?}", s)
            } else {
                s.clone()
            };
            text(displayed)
                .font(ctx.font)
                .size(12.0)
                .class(ctx.palette.text)
                .into()
        }
        serde_norway::Value::Sequence(seq) => {
            if seq.is_empty() {
                return text("[]")
                    .font(cosmic::font::mono())
                    .size(12.0)
                    .class(ctx.palette.muted)
                    .into();
            }
            let mut col = column::Column::new().spacing(2);
            for item in seq.iter() {
                col = col.push(
                    container(
                        row::Row::new()
                            .spacing(4)
                            .push(
                                text("• ")
                                    .font(ctx.font)
                                    .size(12.0)
                                    .class(ctx.palette.muted),
                            )
                            .push(render_yaml_value(item, depth + 1, ctx)),
                    )
                    .padding([0.0, 0.0, 0.0, depth as f32 * INDENT_WIDTH]),
                );
            }
            col.into()
        }
        serde_norway::Value::Mapping(map) => {
            if map.is_empty() {
                return text("{}")
                    .font(cosmic::font::mono())
                    .size(12.0)
                    .class(ctx.palette.muted)
                    .into();
            }
            let mut col = column::Column::new().spacing(2);
            for (key, val) in map.iter() {
                // Frontmatter keys are almost always strings; fall back to
                // the Debug representation for anything else.
                let key_str = match key {
                    serde_norway::Value::String(s) => s.clone(),
                    _ => format!("{key:?}"),
                };
                let child = render_yaml_value(val, depth + 1, ctx);
                col = col.push(
                    container(
                        row::Row::new()
                            .spacing(4)
                            .push(
                                text(format!("{key_str}: "))
                                    .font(ctx.font)
                                    .size(12.0)
                                    .class(ctx.palette.accent),
                            )
                            .push(child),
                    )
                    .padding([0.0, 0.0, 0.0, depth as f32 * INDENT_WIDTH]),
                );
            }
            col.into()
        }
        serde_norway::Value::Tagged(tagged) => {
            // Tag implements Display: "!tagname"
            text(format!("!{}", tagged.tag))
                .font(cosmic::font::mono())
                .size(12.0)
                .class(ctx.palette.muted)
                .into()
        }
    }
}

/// Horizontal indentation width per nesting level for structured
/// frontmatter rendering.
const INDENT_WIDTH: f32 = 20.0;

fn render_blocks<'a>(
    blocks: &[Block],
    ctx: &Ctx<'a>,
    depth: usize,
    stack: &[String],
    fold_id: &mut u64,
) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(8).width(Length::Fill);
    for block in blocks {
        col = col.push(render_block(block, ctx, depth, stack, fold_id));
    }
    col.into()
}

/// Render one markdown block, any nesting level. The live preview calls
/// this for each unfocused top-level block; `stack` guards embed cycles
/// and `fold_id` assigns callout fold ids in traversal order.
pub(crate) fn render_block<'a>(
    block: &Block,
    ctx: &Ctx<'a>,
    depth: usize,
    stack: &[String],
    fold_id: &mut u64,
) -> Element<'a, Message> {
    let palette = ctx.palette;
    match block {
        Block::Heading { level, content, .. } => {
            let (spans, _) = collect_inline(content, ctx, &InlineStyle::default());
            rich_text(spans)
                .size(heading_size(*level, ctx.size))
                .font(ctx.font)
                .on_link_click(Message::OpenUri)
                .into()
        }
        Block::Paragraph { content, .. } => render_paragraph(content, ctx),
        Block::List {
            ordered,
            start,
            items,
            ..
        } => render_list(ordered, *start, items, ctx, depth, stack, fold_id),
        Block::Quote(body) => {
            let inner = render_blocks(body, ctx, depth, stack, fold_id);
            row::Row::new()
                .push(
                    container(Space::new())
                        .width(Length::Fixed(3.0))
                        .height(Length::Fill)
                        .style(move |_| quote_bar_style(&palette)),
                )
                .push(
                    container(inner)
                        .padding(Padding {
                            left: 4.0,
                            ..Padding::ZERO
                        })
                        .width(Length::Fill),
                )
                .into()
        }
        Block::Callout {
            kind,
            fold,
            title,
            body,
            ..
        } => {
            *fold_id += 1;
            let id = *fold_id;
            render_callout(kind, *fold, title, body, id, ctx, depth, stack, fold_id)
        }
        Block::Code { text: code, .. } => container(
            scrollable::scrollable(
                text(code.trim_end().to_string())
                    .font(cosmic::font::mono())
                    .size(ctx.size)
                    .class(ctx.palette.text),
            )
            .width(Length::Fill),
        )
        .padding(8)
        .style(move |_| code_block_style(&palette))
        .into(),
        Block::Math(expr) => container(
            text(expr.trim().to_string())
                .font(cosmic::font::mono())
                .size(ctx.size)
                .class(ctx.palette.text),
        )
        .padding(6)
        .style(move |_| code_block_style(&palette))
        .into(),
        Block::Table { header, rows, .. } => render_table(header, rows, ctx),
        Block::Rule => cosmic::iced::widget::rule::horizontal(1).into(),
        Block::EmbedBlock(embed) => render_embed(embed, ctx, depth, stack, fold_id),
        Block::FootnoteDef { label, body } => {
            let inner = render_blocks(body, ctx, depth, stack, fold_id);
            // The marker matches the one in the text: a raised number for
            // a numbered footnote, the label itself for a named one.
            let marker = match superscript_digits(label) {
                Some(digits) => text(digits).size(14.0).class(ctx.palette.accent),
                None => text(format!("[{label}]"))
                    .size(12.0)
                    .class(ctx.palette.muted),
            };
            row::Row::new()
                .push(container(marker).width(Length::Fixed(52.0)))
                .push(container(inner).width(Length::Fill))
                .into()
        }
        Block::Html(raw) => container(
            text(raw.trim_end().to_string())
                .font(cosmic::font::mono())
                .size(12.0)
                .class(ctx.palette.muted),
        )
        .padding(6)
        .style(move |_| code_block_style(&palette))
        .into(),
    }
}

fn render_list<'a>(
    ordered: &bool,
    start: Option<u64>,
    items: &[ListItem],
    ctx: &Ctx<'a>,
    depth: usize,
    stack: &[String],
    fold_id: &mut u64,
) -> Element<'a, Message> {
    let mut col = column::Column::new().spacing(2);
    for (i, item) in items.iter().enumerate() {
        let (marker, marker_color) = match item.task {
            Some(true) => ("☑".to_string(), ctx.palette.success),
            Some(false) => ("☐".to_string(), ctx.palette.muted),
            None if *ordered => (
                format!("{}.", start.unwrap_or(1) as usize + i),
                ctx.palette.muted,
            ),
            None => ("•".to_string(), ctx.palette.muted),
        };
        let body = render_blocks(&item.body, ctx, depth + 1, stack, fold_id);
        col = col.push(
            row::Row::new()
                .push(
                    container(
                        text(marker)
                            .font(ctx.font)
                            .size(ctx.size)
                            .class(marker_color),
                    )
                    .width(Length::Fixed(26.0)),
                )
                .push(container(body).width(Length::Fill)),
        );
    }
    container(col)
        .padding(Padding {
            left: 16.0 * depth as f32,
            ..Padding::ZERO
        })
        .width(Length::Fill)
        .into()
}

#[allow(clippy::too_many_arguments)]
fn render_callout<'a>(
    kind: &str,
    fold: Fold,
    title: &[Inline],
    body: &[Block],
    id: u64,
    ctx: &Ctx<'a>,
    depth: usize,
    stack: &[String],
    fold_id: &mut u64,
) -> Element<'a, Message> {
    let palette = ctx.palette;
    let color = callout_color(kind, &palette);
    let custom_title = plain_inlines(title);
    let label = if custom_title.is_empty() {
        default_callout_title(kind).to_string()
    } else {
        custom_title
    };

    // Fold semantics (confirmed, M3): `+` starts open, `-` starts closed,
    // plain callouts do not fold. The set holds *toggled* ids.
    let default_open = !matches!(fold, Fold::Closed);
    let open = if default_open {
        !ctx.folds.contains(&id)
    } else {
        ctx.folds.contains(&id)
    };

    let header: Element<'static, Message> = if fold == Fold::None {
        text(label)
            .font(ctx.font)
            .size(ctx.size)
            .class(color)
            .into()
    } else {
        // A foldable callout's title is prose: it reads in the reading
        // view's font, not the UI font the plain button would use.
        let chevron = if open { "▾ " } else { "▸ " };
        button::custom(
            text(format!("{chevron}{label}"))
                .font(ctx.font)
                .size(ctx.size)
                .class(color),
        )
        .on_press(Message::ToggleFold(id))
        .into()
    };

    let mut col = column::Column::new().spacing(6);
    col = col.push(header);
    if open {
        col = col.push(render_blocks(body, ctx, depth, stack, fold_id));
    }

    container(col)
        .padding(8)
        .width(Length::Fill)
        .style(move |_| callout_style(color, &palette))
        .into()
}

fn render_table<'a>(
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    ctx: &Ctx<'a>,
) -> Element<'a, Message> {
    let mut grid = column::Column::new().spacing(0).width(Length::Fill);

    let mut header_row = row::Row::new().spacing(8);
    for cell in header {
        let (spans, _) = collect_inline(cell, ctx, &InlineStyle::bold(ctx.font));
        header_row = header_row
            .push(container(rich_text(spans).size(ctx.size).font(ctx.font)).width(Length::Fill));
    }
    grid = grid.push(header_row);
    grid = grid.push(cosmic::iced::widget::rule::horizontal(1));

    for row_cells in rows {
        let mut r = row::Row::new().spacing(8);
        for cell in row_cells {
            let (spans, _) = collect_inline(cell, ctx, &InlineStyle::default());
            r = r.push(
                container(rich_text(spans).size(ctx.size).font(ctx.font)).width(Length::Fill),
            );
        }
        grid = grid.push(r);
    }
    grid.into()
}

/// One paragraph: inline spans (wrapping) plus any block-ish extras
/// (images/embeds) appended below.
fn render_paragraph<'a>(content: &[Inline], ctx: &Ctx<'a>) -> Element<'a, Message> {
    let mut spans = Vec::new();
    let mut extras: Vec<Element<'a, Message>> = Vec::new();
    collect_inline_into(
        content,
        ctx,
        &InlineStyle::default(),
        &mut spans,
        &mut extras,
    );

    let mut col = column::Column::new().spacing(6).width(Length::Fill);
    if !spans.is_empty() {
        col = col.push(
            rich_text(spans)
                .size(ctx.size)
                .font(ctx.font)
                .on_link_click(Message::OpenUri),
        );
    }
    for extra in extras {
        col = col.push(extra);
    }
    col.into()
}

/// Inline style context threaded through nested emphasis.
#[derive(Default, Clone)]
struct InlineStyle {
    font: Option<Font>,
    color: Option<Color>,
    link: Option<String>,
    strike: bool,
}

impl InlineStyle {
    /// Bold text in the reading view's own family: the weight changes,
    /// the family does not.
    fn bold(base: Font) -> Self {
        Self {
            font: Some(Font {
                weight: font::Weight::Bold,
                ..base
            }),
            ..Default::default()
        }
    }
}

fn collect_inline<'a>(
    inlines: &[Inline],
    ctx: &Ctx<'a>,
    style: &InlineStyle,
) -> (Vec<Span<'static, String, Font>>, Vec<Element<'a, Message>>) {
    let mut spans = Vec::new();
    let mut extras = Vec::new();
    collect_inline_into(inlines, ctx, style, &mut spans, &mut extras);
    (spans, extras)
}

fn collect_inline_into<'a>(
    inlines: &[Inline],
    ctx: &Ctx<'a>,
    style: &InlineStyle,
    spans: &mut Vec<Span<'static, String, Font>>,
    extras: &mut Vec<Element<'a, Message>>,
) {
    for inline in inlines {
        match inline {
            Inline::Text(s) => spans.push(base_span(s.clone(), style, ctx)),
            Inline::Emph(inner) => {
                // Italic keeps the reading view's family: only the
                // slant changes.
                let font = style.font.unwrap_or(ctx.font);
                let mut inner_style = style.clone();
                inner_style.font = Some(Font {
                    style: font::Style::Italic,
                    ..font
                });
                collect_inline_into(inner, ctx, &inner_style, spans, extras);
            }
            Inline::Strong(inner) => {
                // Bold keeps the reading view's family too.
                let font = style.font.unwrap_or(ctx.font);
                let mut inner_style = style.clone();
                inner_style.font = Some(Font {
                    weight: font::Weight::Bold,
                    ..font
                });
                collect_inline_into(inner, ctx, &inner_style, spans, extras);
            }
            Inline::Strike(inner) => {
                let mut inner_style = style.clone();
                inner_style.strike = true;
                collect_inline_into(inner, ctx, &inner_style, spans, extras);
            }
            Inline::Highlight(inner) => {
                let mut inner_style = style.clone();
                inner_style.color = Some(ctx.palette.warning);
                collect_inline_into(inner, ctx, &inner_style, spans, extras);
            }
            Inline::Code(s) => {
                let mut span = Span::new(s.clone()).font(cosmic::font::mono());
                if let Some(link) = &style.link {
                    span = span.link(link.clone());
                }
                span = span.background(Background::Color(ctx.palette.surface));
                spans.push(span);
            }
            Inline::Math(s) => {
                spans.push(base_span(format!("${s}$"), style, ctx).font(cosmic::font::mono()))
            }
            Inline::Link { url, content, .. } => {
                let mut inner = style.clone();
                if url.starts_with("http://")
                    || url.starts_with("https://")
                    || url.starts_with("mailto:")
                {
                    inner.link = Some(url.clone());
                    inner.color = Some(ctx.palette.accent);
                } else if let Some(path) = ctx.index.resolve_relative(url, &ctx.from) {
                    inner.link = Some(format!("note:{path}"));
                    inner.color = Some(ctx.palette.accent);
                } else {
                    // Unresolvable relative link: hand it to the OS opener.
                    inner.link = Some(format!("open:{url}"));
                    inner.color = Some(ctx.palette.muted);
                }
                collect_inline_into(content, ctx, &inner, spans, extras);
            }
            Inline::Image { url, alt } => {
                extras.push(inline_image(url, alt, ctx));
            }
            Inline::WikiLink(link) => {
                let (uri, color) = wikilink_target(link, ctx);
                match wikilink_label(link) {
                    // The custom label is inline markdown: it keeps its
                    // own formatting inside the link.
                    WikiLabel::Custom(inlines) => {
                        let mut inner = style.clone();
                        inner.link = Some(uri);
                        inner.color = Some(color);
                        collect_inline_into(inlines, ctx, &inner, spans, extras);
                    }
                    WikiLabel::Target(text) => {
                        let mut span = Span::new(text).color(color).link(uri);
                        span = span.font_maybe(style.font);
                        if style.strike {
                            span = span.strikethrough(true);
                        }
                        spans.push(span);
                    }
                }
            }
            Inline::Embed(embed) => {
                extras.push(render_embed(
                    embed,
                    ctx,
                    0,
                    std::slice::from_ref(&ctx.from),
                    &mut 0,
                ));
            }
            Inline::Tag(t) => {
                spans.push(
                    Span::new(format!("#{t}"))
                        .color(ctx.palette.success)
                        .link(format!("tag:{t}")),
                );
            }
            Inline::FootnoteRef(label) => {
                let uri = format!("fnref:{label}");
                match superscript_digits(label) {
                    // A numbered footnote reads as a raised marker. Make it
                    // slightly larger than body text so it's easy to spot.
                    Some(marker) => spans.push(
                        Span::new(marker)
                            .size(ctx.size + 2.0)
                            .color(ctx.palette.accent)
                            .link(uri),
                    ),
                    // A named one keeps its bracketed label, smaller but
                    // still easy to read.
                    None => spans.push(
                        Span::new(format!("[{label}]"))
                            .size(13.0)
                            .color(ctx.palette.text)
                            .link(uri),
                    ),
                }
            }
            Inline::InlineFootnote(inner) => {
                spans.push(base_span("(".to_string(), style, ctx).color(ctx.palette.muted));
                let mut muted = style.clone();
                muted.color = Some(ctx.palette.muted);
                collect_inline_into(inner, ctx, &muted, spans, extras);
                spans.push(base_span(")".to_string(), style, ctx).color(ctx.palette.muted));
            }
            Inline::Html(h) => spans.push(base_span(h.clone(), style, ctx)),
            Inline::SoftBreak => {
                let sep = if ctx.strict_breaks { " " } else { "\n" };
                spans.push(base_span(sep.to_string(), style, ctx));
            }
            Inline::HardBreak => spans.push(base_span("\n".to_string(), style, ctx)),
        }
    }
}

fn base_span(s: String, style: &InlineStyle, ctx: &Ctx) -> Span<'static, String, Font> {
    let mut span = Span::new(s);
    if let Some(font) = style.font {
        span = span.font(font);
    }
    span = span.color(style.color.unwrap_or(ctx.palette.text));
    if let Some(link) = &style.link {
        span = span.link(link.clone());
    }
    if style.strike {
        span = span.strikethrough(true);
    }
    span
}

/// The text of a wikilink: its custom label (inline markdown) or, when
/// it has none, the target's own display text.
enum WikiLabel<'a> {
    /// The custom label after the bar, parsed as inline content.
    Custom(&'a [Inline]),
    /// The target's display text.
    Target(String),
}

/// The label of a wikilink, mirroring the HTML renderer
/// (`[[Note#Heading]]` reads "Note > Heading"; confirmed, M3).
fn wikilink_label(link: &WikiLink) -> WikiLabel<'_> {
    let target = &link.target;
    match &link.alias {
        Some(alias) => WikiLabel::Custom(alias),
        None => WikiLabel::Target(match (&target.heading, &target.block) {
            (Some(h), _) if !target.note.is_empty() => format!("{} > {h}", target.note),
            (Some(h), _) => h.clone(),
            (None, Some(_)) if !target.note.is_empty() => target.note.clone(),
            (None, Some(b)) => format!("^{b}"),
            (None, None) => target.note.clone(),
        }),
    }
}

/// Where a wikilink points, and the color it wears.
fn wikilink_target(link: &WikiLink, ctx: &Ctx) -> (String, Color) {
    let target = &link.target;
    if target.note.is_empty() {
        // Same-document anchor: inert until M4b wires scrolling.
        let anchor = match (&target.heading, &target.block) {
            (Some(h), _) => h.clone(),
            (None, Some(b)) => format!("^{b}"),
            (None, None) => String::new(),
        };
        return (format!("scroll:{anchor}"), ctx.palette.accent);
    }

    match ctx.index.resolve(&target.note, &ctx.from) {
        Some(path) => (format!("note:{path}"), ctx.palette.accent),
        None => (
            format!("create:{}|{}", target.note, ctx.from),
            ctx.palette.muted,
        ),
    }
}

/// An inline image or a placeholder for it.
fn inline_image<'a>(url: &str, alt: &str, ctx: &Ctx<'a>) -> Element<'a, Message> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return placeholder(format!("{alt} {url}"), ctx);
    }
    match ctx.index.resolve_relative(url, &ctx.from) {
        Some(path) => media_view(Some(path), url, None, ctx),
        None => placeholder(format!("[unresolved image: {alt}]"), ctx),
    }
}

/// An embed (`![[target]]`): media renders as an image, notes/sections/blocks
/// transclude with a depth limit and cycle detection (mirrors html.rs).
fn render_embed<'a>(
    embed: &Embed,
    ctx: &Ctx<'a>,
    depth: usize,
    stack: &[String],
    fold_id: &mut u64,
) -> Element<'a, Message> {
    let target = &embed.target;
    if target.note.is_empty() {
        return placeholder("[empty embed target]".into(), ctx);
    }
    if is_image(&target.note) || is_svg(&target.note) {
        let resolved = ctx.index.resolve(&target.note, &ctx.from);
        return media_view(resolved, &target.note, embed.size, ctx);
    }
    if is_other_media(&target.note) {
        return placeholder(format!("[attachment: {}]", target.note), ctx);
    }

    let Some(path) = ctx
        .index
        .resolve(&target.note, &ctx.from)
        .map(str::to_string)
    else {
        return placeholder(format!("[unresolved embed: {}]", target.note), ctx);
    };
    if depth >= EMBED_DEPTH_LIMIT {
        return placeholder("[embed depth limit]".into(), ctx);
    }
    if stack.contains(&path) {
        return placeholder("[embed cycle]".into(), ctx);
    }
    let Some(source) = ctx.vault.note_source(&path) else {
        return placeholder(format!("[unreadable embed: {path}]"), ctx);
    };

    let doc = parse_note(&source);
    let blocks: &[Block] = match (&target.heading, &target.block) {
        (Some(heading), _) => section(&doc.blocks, heading),
        (None, Some(block_id)) => slice_block(&doc.blocks, block_id),
        _ => &doc.blocks,
    };

    let palette = ctx.palette;
    let mut inner_stack = stack.to_vec();
    inner_stack.push(path.clone());
    let inner = render_blocks(blocks, ctx, depth + 1, &inner_stack, fold_id);

    container(inner)
        .padding(8)
        .width(Length::Fill)
        .style(move |_| embed_style(&palette))
        .into()
}

/// Media by extension: images render, everything else gets a placeholder.
fn media_view<'a>(
    resolved: Option<&str>,
    target: &str,
    size: Option<(u32, Option<u32>)>,
    ctx: &Ctx<'a>,
) -> Element<'a, Message> {
    let Some(path) = resolved else {
        return placeholder(format!("[unresolved: {target}]"), ctx);
    };
    if is_svg(path) {
        let handle = widget::svg::Handle::from_path(ctx.vault.root().join(path));
        let mut svg = widget::svg(handle);
        if let Some((w, _)) = size {
            svg = svg.width(Length::Fixed(w as f32));
        }
        return svg.into();
    }
    if !is_image(path) {
        return placeholder(format!("[attachment: {target}]"), ctx);
    }

    // Cache decoded handles by path so re-renders do not re-decode.
    let handle = {
        let mut cache = ctx.images.borrow_mut();
        if let Some(handle) = cache.get(path) {
            handle.clone()
        } else {
            match std::fs::read(ctx.vault.root().join(path)) {
                Ok(bytes) => {
                    let handle = ImageHandle::from_bytes(bytes);
                    cache.insert(path.to_string(), handle.clone());
                    handle
                }
                Err(_) => return placeholder(format!("[unreadable: {target}]"), ctx),
            }
        }
    };
    let mut img = widget::image(handle);
    if let Some((w, h)) = size {
        img = img.width(Length::Fixed(w as f32));
        if let Some(h) = h {
            img = img.height(Length::Fixed(h as f32));
        }
    }
    img.into()
}

fn placeholder<'a>(label: String, ctx: &Ctx<'a>) -> Element<'a, Message> {
    text(label).size(12.0).class(ctx.palette.muted).into()
}

fn extension_lower(path: &str) -> String {
    // Mirrors html.rs: extension checks are case-insensitive.
    path.rsplit_once('.')
        .map_or(String::new(), |(_, ext)| ext.to_lowercase())
}

fn is_image(path: &str) -> bool {
    matches!(
        extension_lower(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "avif"
    )
}

fn is_svg(path: &str) -> bool {
    extension_lower(path) == "svg"
}

fn is_other_media(path: &str) -> bool {
    matches!(
        extension_lower(path).as_str(),
        "mp4" | "webm" | "mov" | "mkv" | "mp3" | "wav" | "ogg" | "flac" | "pdf"
    )
}

/// `#Heading` section slice: from the matching heading to the next heading of
/// the same or higher level (mirrors html.rs).
fn section<'a>(blocks: &'a [Block], heading: &str) -> &'a [Block] {
    let wanted = heading.trim().to_lowercase();
    let mut started_at: Option<(usize, u8)> = None;
    let mut end = blocks.len();
    for (i, block) in blocks.iter().enumerate() {
        if let Block::Heading { level, content, .. } = block {
            let heading_text = plain_inlines(content).to_lowercase();
            match started_at {
                None => {
                    if heading_text == wanted {
                        started_at = Some((i, *level));
                    }
                }
                Some((_, open_level)) => {
                    if *level <= open_level {
                        end = i;
                        break;
                    }
                }
            }
        }
    }
    match started_at {
        Some((s, _)) => &blocks[s..end],
        None => &[],
    }
}

/// The single block carrying `#^block-id` (mirrors html.rs).
fn slice_block<'a>(blocks: &'a [Block], block_id: &str) -> &'a [Block] {
    for (i, block) in blocks.iter().enumerate() {
        let found = match block {
            Block::Paragraph { block_id: id, .. } => id.as_deref() == Some(block_id),
            Block::List { items, .. } => items
                .iter()
                .any(|it| it.block_id.as_deref() == Some(block_id)),
            _ => false,
        };
        if found {
            return &blocks[i..i + 1];
        }
    }
    &[]
}

fn plain_inlines(inlines: &[Inline]) -> String {
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

fn plain_wiki_label(link: &WikiLink) -> String {
    link.alias
        .as_ref()
        .map(|alias| plain_inlines(alias))
        .unwrap_or_else(|| link.target.note.clone())
}

fn default_callout_title(kind: &str) -> &'static str {
    match kind.to_lowercase().as_str() {
        "note" | "info" | "todo" => "Note",
        "abstract" | "summary" | "tldr" => "Abstract",
        "tip" | "hint" | "important" => "Tip",
        "success" | "check" | "done" => "Success",
        "question" | "help" | "faq" => "Question",
        "warning" | "caution" | "attention" => "Warning",
        "failure" | "fail" | "missing" => "Failure",
        "danger" | "error" => "Danger",
        "bug" => "Bug",
        "example" => "Example",
        "quote" | "cite" => "Quote",
        _ => "Callout",
    }
}

fn callout_color(kind: &str, palette: &Palette) -> Color {
    match kind.to_lowercase().as_str() {
        "tip" | "hint" | "important" | "success" | "check" | "done" => palette.success,
        "question" | "help" | "faq" | "warning" | "caution" | "attention" => palette.warning,
        "danger" | "error" | "failure" | "fail" | "missing" | "bug" => palette.danger,
        _ => palette.accent,
    }
}

// --- styles ---------------------------------------------------------------

fn code_block_style(palette: &Palette) -> cosmic::widget::container::Style {
    cosmic::widget::container::Style {
        background: Some(Background::Color(palette.surface)),
        border: border::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn embed_style(palette: &Palette) -> cosmic::widget::container::Style {
    cosmic::widget::container::Style {
        border: border::Border {
            radius: 4.0.into(),
            width: 1.0,
            color: with_alpha(palette.muted, 0.5),
        },
        ..Default::default()
    }
}

fn callout_style(color: Color, palette: &Palette) -> cosmic::widget::container::Style {
    cosmic::widget::container::Style {
        background: Some(Background::Color(with_alpha(color, 0.10))),
        border: border::Border {
            radius: 4.0.into(),
            width: 1.0,
            color: with_alpha(color, 0.5),
        },
        text_color: Some(palette.text),
        ..Default::default()
    }
}

fn quote_bar_style(palette: &Palette) -> cosmic::widget::container::Style {
    cosmic::widget::container::Style {
        background: Some(Background::Color(with_alpha(palette.muted, 0.6))),
        ..Default::default()
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use vault_core::{NoteSource, Vault, VaultIndex};
    use vault_md::WikiTarget;

    fn ctx_for(vault: &'static Vault, from: &str) -> Ctx<'static> {
        // The Ctx borrows everything it needs; tests leak the small pieces
        // for the process lifetime so the borrows are trivially 'static.
        let index: &'static VaultIndex = Box::leak(Box::new(vault.index()));
        let folds: &'static HashSet<u64> = Box::leak(Box::new(HashSet::new()));
        let images: &'static RefCell<HashMap<String, ImageHandle>> =
            Box::leak(Box::new(RefCell::new(HashMap::new())));
        Ctx {
            vault,
            index,
            from: from.to_string(),
            strict_breaks: false,
            palette: Palette {
                text: Color::WHITE,
                muted: Color::WHITE,
                accent: Color::WHITE,
                success: Color::WHITE,
                warning: Color::WHITE,
                danger: Color::WHITE,
                surface: Color::BLACK,
                is_dark: true,
            },
            font: cosmic::font::default(),
            size: 16.0,
            folds,
            images,
        }
    }

    #[test]
    fn headings_scale_with_the_reading_size() {
        // At the default 16-pixel base the sizes match the fixed ones
        // the renderer used before.
        assert_eq!(heading_size(1, 16.0), 26.0);
        assert_eq!(heading_size(2, 16.0), 22.0);
        assert_eq!(heading_size(3, 16.0), 19.0);
        assert_eq!(heading_size(4, 16.0), 18.0);
        assert_eq!(heading_size(5, 16.0), 16.0);
        // A bigger base scales the whole hierarchy.
        assert_eq!(heading_size(1, 32.0), 51.0);
        assert_eq!(heading_size(5, 32.0), 32.0);
        // The headings stay in order and above the body text.
        let mut previous = heading_size(1, 16.0);
        for level in 2..=6 {
            let size = heading_size(level, 16.0);
            assert!(
                size <= previous,
                "level {level} is not smaller than level {}",
                level - 1
            );
            assert!(size >= 16.0, "level {level} is not at least the body size");
            previous = size;
        }
    }

    #[test]
    fn wikilinks_route_to_a_note_or_to_creation() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Glass.md"), "# Glass\n").unwrap();
        std::fs::write(tmp.path().join("Source.md"), "from source\n").unwrap();
        let vault: &'static Vault = Box::leak(Box::new(Vault::open(tmp.path()).unwrap()));

        let ctx = ctx_for(vault, "Source.md");

        // Resolved wikilink -> open that note.
        let link = WikiLink {
            target: WikiTarget {
                note: "Glass".to_string(),
                heading: None,
                block: None,
            },
            alias: None,
        };
        assert_eq!(label_text(&wikilink_label(&link)), "Glass");
        let (uri, _) = wikilink_target(&link, &ctx);
        assert_eq!(uri, "note:Glass.md");

        // Unresolved wikilink -> offer to create it, remembering the source.
        let missing = WikiLink {
            target: WikiTarget {
                note: "Missing".to_string(),
                heading: None,
                block: None,
            },
            alias: None,
        };
        let (uri, _) = wikilink_target(&missing, &ctx);
        assert_eq!(uri, "create:Missing|Source.md");

        // `[[Note#Heading]]` reads "Note > Heading" (confirmed, M3).
        let headed = WikiLink {
            target: WikiTarget {
                note: "Glass".to_string(),
                heading: Some("Intro".to_string()),
                block: None,
            },
            alias: None,
        };
        assert_eq!(label_text(&wikilink_label(&headed)), "Glass > Intro");
        let (uri, _) = wikilink_target(&headed, &ctx);
        assert_eq!(uri, "note:Glass.md");

        // Same-document anchors are inert until M4b wires scrolling.
        let anchor = WikiLink {
            target: WikiTarget {
                note: String::new(),
                heading: Some("Intro".to_string()),
                block: None,
            },
            alias: None,
        };
        assert_eq!(label_text(&wikilink_label(&anchor)), "Intro");
        let (uri, _) = wikilink_target(&anchor, &ctx);
        assert_eq!(uri, "scroll:Intro");
    }

    /// The readable text of a wikilink label.
    fn label_text(label: &WikiLabel) -> String {
        match label {
            WikiLabel::Custom(inlines) => plain_inlines(inlines),
            WikiLabel::Target(text) => text.clone(),
        }
    }

    #[test]
    fn a_wikilink_label_keeps_its_inline_formatting() {
        // `[[Note|**bold**]]` — the label is inline markdown.
        let doc = parse_note("see [[Note|**bold** and *em*]]\n");
        let link = match &doc.blocks[0] {
            Block::Paragraph { content, .. } => match &content[1] {
                Inline::WikiLink(link) => link.clone(),
                other => panic!("expected a wikilink, got {other:?}"),
            },
            other => panic!("expected a paragraph, got {other:?}"),
        };
        // The label is parsed, not kept as raw text.
        let alias = link.alias.expect("the link has a label");
        assert!(matches!(alias[0], Inline::Strong(_)), "{alias:?}");
        assert!(matches!(alias[2], Inline::Emph(_)), "{alias:?}");
        // And it reads as its plain text.
        assert_eq!(plain_inlines(&alias), "bold and em");
    }

    #[test]
    fn the_footnotes_are_rendered_last_under_the_rule() {
        let doc = parse_note("Text[^1]\n\nMore text.\n\n[^1]: The note.\n");
        let (prose, footnotes) = reading_order(&doc.blocks);
        // The prose keeps its source order…
        assert_eq!(prose, vec![0, 1]);
        // …and the definitions come last, where the rule goes.
        assert_eq!(footnotes, vec![2]);
    }

    #[test]
    fn footnote_numbers_become_superscript_digits() {
        assert_eq!(superscript_digits("1").as_deref(), Some("¹"));
        assert_eq!(superscript_digits("12").as_deref(), Some("¹²"));
        assert_eq!(superscript_digits("0").as_deref(), Some("⁰"));
        // A named footnote has no digits to raise.
        assert_eq!(superscript_digits("note"), None);
        assert_eq!(superscript_digits(""), None);
        assert_eq!(superscript_digits("1a"), None);
    }

    #[test]
    fn outline_lists_top_level_headings_with_block_indices() {
        let doc = parse_note("# One\n\ntext\n\n## Two\n\n- a list\n- items\n\n# Three\n");
        let entries = outline(&doc);
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert_eq!(entries[0].block, 0);
        assert_eq!(entries[0].level, 1);
        assert_eq!(entries[0].text, "One");
        assert_eq!(entries[1].block, 2);
        assert_eq!(entries[1].level, 2);
        assert_eq!(entries[2].text, "Three");
    }

    #[test]
    fn find_block_with_text_descends_into_nested_structures() {
        let doc = parse_note("# Top\n\ntarget is here\n\n> [!note] Callout\n> the needle inside\n");
        // Plain paragraph.
        assert_eq!(find_block_with_text(&doc, "target is"), Some(1));
        // Inside the callout body.
        assert_eq!(find_block_with_text(&doc, "needle inside"), Some(2));
        assert_eq!(find_block_with_text(&doc, "absent"), None);
        assert_eq!(find_block_with_text(&doc, "  "), None);
    }

    #[test]
    fn media_targets_are_recognized_by_extension() {
        assert!(is_image("a/b/photo.PNG"));
        assert!(is_image("photo.jpeg"));
        assert!(!is_image("notes.md"));
        assert!(is_other_media("clip.mp4"));
        assert!(is_other_media("paper.pdf"));
    }
}
