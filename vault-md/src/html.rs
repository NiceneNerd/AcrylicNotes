//! IR → HTML.
//!
//! The reading-view renderer: resolves links through a [`VaultIndex`],
//! transcludes embeds with cycle detection and a depth limit, and sanitizes
//! raw HTML fragments with ammonia. Obsidian-compatible class names are used
//! (`internal-link`, `callout`, `tag`, …) so vault CSS keeps working.
//!
//! Sanitization applies to RAW HTML FRAGMENTS ONLY — never to the whole
//! document, which would strip this renderer's own classes and ids.

use std::cell::RefCell;
use std::fmt::Write as _;

use vault_core::{LinkKind, VaultIndex};

use crate::ir::{Alignment, Block, Doc, Fold, Inline};

/// Read-only inputs the renderer needs beyond the IR.
pub struct RenderContext<'a> {
    /// Resolution index over notes AND attachments.
    pub index: &'a VaultIndex,
    /// Vault-relative path of the note being rendered (`[[#Heading]]` needs it).
    pub from_note: &'a str,
    /// Raw text of other notes, for transclusion.
    pub notes: &'a dyn vault_core::NoteSource,
    pub options: RenderOptions,
    /// Paths currently being transcluded (cycle detection).
    chain: RefCell<Vec<String>>,
    /// Footnote label -> display number, in document definition order.
    footnote_defs: RefCell<std::collections::HashMap<String, usize>>,
}

#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Obsidian `strictLineBreaks`: soft breaks stay soft (newline) instead
    /// of becoming `<br>`.
    pub strict_line_breaks: bool,
    /// Transclusion depth limit (Obsidian has a similar guard).
    pub max_embed_depth: usize,
    /// Sanitize raw HTML fragments with ammonia.
    pub sanitize_html: bool,
    /// Web-server mode (M5): route prefix for resolved note links, e.g.
    /// `/n`. `None` (default) keeps relative hrefs — the GUI goldens depend
    /// on that.
    pub note_base: Option<String>,
    /// Web-server mode: route prefix for resolved attachment/embed sources,
    /// e.g. `/files`.
    pub file_base: Option<String>,
    /// Web-server mode: route prefix for tag links, e.g. `/tag`.
    pub tag_base: Option<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            strict_line_breaks: false,
            max_embed_depth: 3,
            sanitize_html: true,
            note_base: None,
            file_base: None,
            tag_base: None,
        }
    }
}

impl<'a> RenderContext<'a> {
    pub fn new(
        index: &'a VaultIndex,
        from_note: &'a str,
        notes: &'a dyn vault_core::NoteSource,
        options: RenderOptions,
    ) -> Self {
        Self {
            index,
            from_note,
            notes,
            options,
            chain: RefCell::new(vec![from_note.to_string()]),
            footnote_defs: RefCell::new(std::collections::HashMap::new()),
        }
    }

    /// The display number for a footnote label (definition order; unknown
    /// labels get numbers after the definitions).
    fn footnote_number(&self, label: &str) -> usize {
        let mut defs = self.footnote_defs.borrow_mut();
        let next = defs.len() + 1;
        *defs.entry(label.to_string()).or_insert(next)
    }

    fn entering(&self, path: &str) -> bool {
        let mut chain = self.chain.borrow_mut();
        if chain.len() > self.options.max_embed_depth || chain.iter().any(|p| p == path) {
            return false;
        }
        chain.push(path.to_string());
        true
    }

    fn leaving(&self) {
        self.chain.borrow_mut().pop();
    }

    /// Whether web-server link rewriting is active (any route prefix set).
    fn web_mode(&self) -> bool {
        self.options.note_base.is_some() || self.options.file_base.is_some()
    }

    /// An href for a resolved note path: prefixed in web mode, else relative
    /// (the GUI goldens depend on the relative form).
    fn note_href(&self, path: &str) -> String {
        match &self.options.note_base {
            Some(base) => format!("{base}/{}", href_encode(path)),
            None => href_encode(path),
        }
    }

    /// A src/href for a resolved attachment path.
    fn file_href(&self, path: &str) -> String {
        match &self.options.file_base {
            Some(base) => format!("{base}/{}", href_encode(path)),
            None => href_encode(path),
        }
    }

    /// Rewrite a markdown-authored URL for the web: dangerous schemes are
    /// defused, relative paths that resolve inside the vault get the right
    /// route prefix (notes under `note_base`, everything else under
    /// `file_base`); absolute URLs and anchors pass through untouched.
    /// Inactive (default options) the URL is returned as authored.
    fn web_url(&self, url: &str, is_image: bool) -> String {
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("javascript:")
            || lower.starts_with("vbscript:")
            || lower.starts_with("data:")
        {
            return "#".to_string();
        }
        if !self.web_mode() || is_external_or_anchor(url) {
            return url.to_string();
        }
        match self
            .index
            .resolve_relative(url, self.from_note)
            .map(str::to_string)
        {
            Some(path) => {
                if !is_image && path.ends_with(".md") {
                    self.note_href(&path)
                } else {
                    self.file_href(&path)
                }
            }
            // Unresolvable relative URL: leave as authored.
            None => url.to_string(),
        }
    }
}

/// Render a whole note to HTML.
pub fn render_html(doc: &Doc, ctx: &RenderContext) -> String {
    // Footnote definitions get numbers in document order; references then
    // display numbers like Obsidian does, not internal labels.
    //
    // Numbering follows REFERENCE order (confirmed against real Obsidian in
    // M3): the first reference to a label is 1, the next new label 2, and so
    // on. Definitions that are never referenced take the numbers after the
    // referenced ones, in definition order.
    for block in &doc.blocks {
        collect_footnote_refs(block, ctx);
    }
    for block in &doc.blocks {
        assign_unused_def_numbers(block, ctx);
    }
    let mut out = String::new();
    for block in &doc.blocks {
        render_block(block, ctx, &mut out);
    }
    out
}

/// Walk the document in reading order and number every footnote label the
/// first time it is REFERENCED.
fn collect_footnote_refs(block: &Block, ctx: &RenderContext) {
    match block {
        Block::Paragraph { content, .. } | Block::Heading { content, .. } => {
            collect_inline_refs(content, ctx);
        }
        Block::Callout { title, body, .. } => {
            collect_inline_refs(title, ctx);
            for b in body {
                collect_footnote_refs(b, ctx);
            }
        }
        Block::List { items, .. } => {
            for item in items {
                for b in &item.body {
                    collect_footnote_refs(b, ctx);
                }
            }
        }
        Block::Quote(body) | Block::FootnoteDef { body, .. } => {
            for b in body {
                collect_footnote_refs(b, ctx);
            }
        }
        Block::Table { header, rows, .. } => {
            for cell in header {
                collect_inline_refs(cell, ctx);
            }
            for row in rows {
                for cell in row {
                    collect_inline_refs(cell, ctx);
                }
            }
        }
        Block::Code { .. } | Block::Math(_) | Block::Rule | Block::Html(_) => {}
        Block::EmbedBlock(_) => {}
    }
}

/// Number the inlines' footnote references (including nested containers).
fn collect_inline_refs(inlines: &[Inline], ctx: &RenderContext) {
    for inline in inlines {
        match inline {
            Inline::FootnoteRef(label) => {
                ctx.footnote_number(label);
            }
            Inline::Emph(children)
            | Inline::Strong(children)
            | Inline::Strike(children)
            | Inline::Highlight(children)
            | Inline::InlineFootnote(children) => collect_inline_refs(children, ctx),
            Inline::Link { content, .. } => collect_inline_refs(content, ctx),
            _ => {}
        }
    }
}

/// Give every definition that was never referenced the next free number, in
/// definition order.
fn assign_unused_def_numbers(block: &Block, ctx: &RenderContext) {
    match block {
        Block::FootnoteDef { label, body } => {
            ctx.footnote_number(label);
            for b in body {
                assign_unused_def_numbers(b, ctx);
            }
        }
        Block::List { items, .. } => {
            for item in items {
                for b in &item.body {
                    assign_unused_def_numbers(b, ctx);
                }
            }
        }
        Block::Quote(body) | Block::Callout { body, .. } => {
            for b in body {
                assign_unused_def_numbers(b, ctx);
            }
        }
        Block::EmbedBlock(_) => {}
        _ => {}
    }
}

fn render_block(block: &Block, ctx: &RenderContext, out: &mut String) {
    match block {
        Block::Paragraph { content, .. } => {
            if content.is_empty() {
                return;
            }
            out.push_str("<p>");
            render_inlines(content, ctx, out);
            out.push_str("</p>\n");
        }
        Block::Heading { level, content, id } => {
            let _ = write!(out, "<h{level}");
            if let Some(id) = id {
                let _ = write!(out, " id=\"{}\"", escape_attr(id));
            }
            out.push('>');
            render_inlines(content, ctx, out);
            let _ = writeln!(out, "</h{level}>");
        }
        Block::List {
            ordered,
            start,
            tight,
            items,
        } => {
            let tag = if *ordered { "ol" } else { "ul" };
            match start {
                Some(n) if *ordered => {
                    let _ = writeln!(out, "<{tag} start=\"{n}\">");
                }
                _ => out.push_str(&format!("<{tag}>\n")),
            }
            for item in items {
                out.push_str("<li");
                if let Some(done) = item.task {
                    let _ = write!(
                        out,
                        " class=\"task-list-item{}\"",
                        if done { " is-checked" } else { "" }
                    );
                }
                out.push('>');
                if let Some(done) = item.task {
                    let _ = write!(
                        out,
                        "<input type=\"checkbox\" disabled{}> ",
                        if done { " checked" } else { "" }
                    );
                }
                if *tight {
                    // Tight list: item content renders without <p> wrappers.
                    for block in &item.body {
                        match block {
                            Block::Paragraph { content, .. } => render_inlines(content, ctx, out),
                            other => render_block(other, ctx, out),
                        }
                    }
                } else {
                    for block in &item.body {
                        render_block(block, ctx, out);
                    }
                }
                out.push_str("</li>\n");
            }
            let _ = writeln!(out, "</{tag}>");
        }
        Block::Quote(body) => {
            out.push_str("<blockquote>\n");
            for block in body {
                render_block(block, ctx, out);
            }
            out.push_str("</blockquote>\n");
        }
        Block::Callout {
            kind,
            fold,
            metadata,
            title,
            body,
        } => {
            out.push_str("<div");
            if let Some(meta) = metadata {
                let _ = write!(out, " data-callout-metadata=\"{}\"", escape_attr(meta));
            }
            let _ = write!(
                out,
                " class=\"callout\" data-callout=\"{}\"",
                escape_attr(kind)
            );
            match fold {
                Fold::Open => out.push_str(" data-callout-foldable data-callout-open"),
                Fold::Closed => out.push_str(" data-callout-foldable"),
                Fold::None => {}
            }
            out.push_str(">\n<div class=\"callout-title\"><div class=\"callout-title-inner\">");
            render_inlines(title, ctx, out);
            out.push_str("</div></div>\n");
            // Obsidian renders folded content lazily; we always render it and
            // let the `data-callout-foldable` attributes drive the CSS/JS.
            out.push_str("<div class=\"callout-content\">\n");
            for block in body {
                render_block(block, ctx, out);
            }
            out.push_str("</div>\n</div>\n");
        }
        Block::Code { lang, text } => {
            out.push_str("<pre><code");
            if let Some(lang) = lang {
                let _ = write!(out, " class=\"language-{}\"", escape_attr(lang));
            }
            out.push('>');
            out.push_str(&escape(text));
            out.push_str("</code></pre>\n");
        }
        Block::Math(text) => {
            let _ = writeln!(
                out,
                "<span class=\"math math-display\">$$ {} $$</span>",
                escape(text)
            );
        }
        Block::Table {
            alignments,
            header,
            rows,
        } => {
            out.push_str("<table>\n");
            if !header.is_empty() {
                out.push_str("<thead>\n<tr>\n");
                for (i, cell) in header.iter().enumerate() {
                    let _ = write!(out, "<th{}>", align_attr(alignments.get(i)));
                    render_inlines(cell, ctx, out);
                    out.push_str("</th>\n");
                }
                out.push_str("</tr>\n</thead>\n");
            }
            if !rows.is_empty() {
                out.push_str("<tbody>\n");
                for row in rows {
                    out.push_str("<tr>\n");
                    for (i, cell) in row.iter().enumerate() {
                        let _ = write!(out, "<td{}>", align_attr(alignments.get(i)));
                        render_inlines(cell, ctx, out);
                        out.push_str("</td>\n");
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</tbody>\n");
            }
            out.push_str("</table>\n");
        }
        Block::Rule => out.push_str("<hr>\n"),
        Block::EmbedBlock(embed) => render_embed_block(embed, ctx, out),
        Block::FootnoteDef { label, body } => {
            // The number is the reference-order number, so the definition
            // shows the same number the reference used.
            let number = ctx.footnote_number(label);
            let _ = writeln!(
                out,
                "<div class=\"footnote-def\" id=\"fn-{}\" data-footnote-number=\"{number}\">",
                escape_attr(label)
            );
            for block in body {
                render_block(block, ctx, out);
            }
            out.push_str("</div>\n");
        }
        Block::Html(html) => out.push_str(&sanitize(html, ctx)),
    }
}

fn render_embed_block(embed: &crate::ir::Embed, ctx: &RenderContext, out: &mut String) {
    let target = &embed.target;
    if is_media(&target.note) {
        out.push_str("<p>");
        render_embed_media(embed, ctx, out);
        out.push_str("</p>\n");
        return;
    }
    render_embed_note(target, ctx, out);
}

/// Media embeds (`![[img.png|300]]`) render as <img>/<video> elements.
fn render_embed_media(embed: &crate::ir::Embed, ctx: &RenderContext, out: &mut String) {
    let target = &embed.target;
    let resolved = ctx.index.resolve(&target.note, ctx.from_note);
    let src = resolved.map(|p| ctx.file_href(p)).unwrap_or_default();
    let _ = write!(
        out,
        "<img class=\"internal-embed\" src=\"{}\" alt=\"{}\"",
        src,
        escape_attr(&target.note)
    );
    if let Some((w, h)) = embed.size {
        let _ = write!(out, " width=\"{w}\"");
        if let Some(h) = h {
            let _ = write!(out, " height=\"{h}\"");
        }
    }
    if resolved.is_none() {
        out.push_str(" data-unresolved");
    }
    out.push('>');
}

/// Note/section/block transclusion (`![[Note]]`, `![[Note#Heading]]`).
fn render_embed_note(target: &crate::ir::WikiTarget, ctx: &RenderContext, out: &mut String) {
    let Some(path) = ctx
        .index
        .resolve(&target.note, ctx.from_note)
        .map(str::to_string)
    else {
        let _ = writeln!(
            out,
            "<div class=\"internal-embed is-unresolved\" data-target=\"{}\">{}</div>",
            escape_attr(&target.note),
            escape(&target.note)
        );
        return;
    };

    let Some(source) = ctx.notes.note_source(&path) else {
        let _ = writeln!(
            out,
            "<div class=\"internal-embed is-unresolved\" data-target=\"{}\"></div>",
            escape_attr(&path)
        );
        return;
    };
    if !ctx.entering(&path) {
        // Depth limit or cycle: say so instead of recursing forever.
        let _ = writeln!(
            out,
            "<div class=\"internal-embed is-unresolved\" data-target=\"{}\">[embed depth limit]</div>",
            escape_attr(&path)
        );
        return;
    }
    let doc = crate::lower::parse_note(&source);
    let mut rendered = String::new();
    let blocks: &[Block] = match (&target.heading, &target.block) {
        (Some(heading), _) => section(&doc.blocks, heading),
        (None, Some(block_id)) => slice_block(&doc.blocks, block_id),
        _ => &doc.blocks,
    };
    for block in blocks {
        render_block(block, ctx, &mut rendered);
    }
    ctx.leaving();
    if rendered.is_empty() {
        let _ = writeln!(
            out,
            "<div class=\"internal-embed is-unresolved\" data-target=\"{}\"></div>",
            escape_attr(&path)
        );
    } else {
        let _ = write!(
            out,
            "<div class=\"internal-embed\" data-target=\"{}\">\n{}</div>\n",
            escape_attr(&path),
            rendered
        );
    }
}

/// The blocks of `#Heading` up to the next heading of the same or higher level.
fn section<'a>(blocks: &'a [Block], heading: &str) -> &'a [Block] {
    let wanted = heading.trim().to_lowercase();
    let mut started_at: Option<(usize, u8)> = None;
    let mut end = blocks.len();
    for (i, block) in blocks.iter().enumerate() {
        if let Block::Heading { level, content, .. } = block {
            let text = plain(content).to_lowercase();
            match started_at {
                None => {
                    if text == wanted {
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

/// The block carrying `#^block-id` (paragraph or list item).
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

fn render_inlines(inlines: &[Inline], ctx: &RenderContext, out: &mut String) {
    // Runs of adjacent raw-HTML and text inlines are buffered and sanitized
    // TOGETHER, so paired tags like <abbr>…</abbr> are cleaned as one
    // fragment instead of being closed prematurely.
    let mut raw_run = String::new();
    for inline in inlines {
        match inline {
            Inline::Html(html) => raw_run.push_str(html),
            Inline::Text(text) if !raw_run.is_empty() => {
                raw_run.push_str(&escape(text));
            }
            _ => {
                flush_raw_run(&mut raw_run, ctx, out);
                render_inline(inline, ctx, out);
            }
        }
    }
    flush_raw_run(&mut raw_run, ctx, out);
}

fn flush_raw_run(raw_run: &mut String, ctx: &RenderContext, out: &mut String) {
    if raw_run.is_empty() {
        return;
    }
    out.push_str(&sanitize(raw_run, ctx));
    raw_run.clear();
}

fn render_inline(inline: &Inline, ctx: &RenderContext, out: &mut String) {
    match inline {
        Inline::Text(text) => out.push_str(&escape(text)),
        Inline::Emph(children) => wrap_inline("em", children, ctx, out),
        Inline::Strong(children) => wrap_inline("strong", children, ctx, out),
        Inline::Strike(children) => wrap_inline("del", children, ctx, out),
        Inline::Highlight(children) => wrap_inline("mark", children, ctx, out),
        Inline::Code(code) => {
            let _ = write!(out, "<code>{}</code>", escape(code));
        }
        Inline::Math(text) => {
            let _ = write!(
                out,
                "<span class=\"math math-inline\">$ {} $</span>",
                escape(text)
            );
        }
        Inline::Link {
            url,
            title,
            content,
        } => {
            let _ = write!(
                out,
                "<a href=\"{}\"{}>",
                escape_attr(&ctx.web_url(url, false)),
                title
                    .as_ref()
                    .map(|t| format!(" title=\"{}\"", escape_attr(t)))
                    .unwrap_or_default()
            );
            render_inlines(content, ctx, out);
            out.push_str("</a>");
        }
        Inline::Image { url, alt } => {
            let _ = write!(
                out,
                "<img src=\"{}\" alt=\"{}\">",
                escape_attr(&ctx.web_url(url, true)),
                escape_attr(alt)
            );
        }
        Inline::WikiLink(link) => render_wikilink(link, ctx, out),
        Inline::Embed(embed) => render_embed_media(embed, ctx, out),
        Inline::Tag(name) => {
            let href = match &ctx.options.tag_base {
                Some(base) => format!("{base}/{}", href_encode(name)),
                None => format!("#{name}"),
            };
            let _ = write!(
                out,
                "<a class=\"tag\" href=\"{}\">#{}</a>",
                escape_attr(&href),
                escape(name)
            );
        }
        Inline::FootnoteRef(label) => {
            let number = ctx.footnote_number(label);
            let _ = write!(
                out,
                "<sup class=\"footnote-ref\"><a href=\"#fn-{}\">{}</a></sup>",
                escape_attr(label),
                number
            );
        }
        Inline::InlineFootnote(children) => {
            // comrak normalizes inline footnotes to refs + defs; kept for
            // IR completeness.
            wrap_inline("span", children, ctx, out);
        }
        Inline::Html(html) => out.push_str(&sanitize(html, ctx)),
        Inline::SoftBreak => {
            if ctx.options.strict_line_breaks {
                out.push('\n');
            } else {
                out.push_str("<br>\n");
            }
        }
        Inline::HardBreak => out.push_str("<br>\n"),
    }
}

/// Wikilinks resolve through the index; unresolved ones get `is-unresolved`
/// and keep their target in `data-target`. `[[Note#Heading]]` displays as
/// "Note > Heading" unless an alias is given.
fn render_wikilink(link: &crate::ir::WikiLink, ctx: &RenderContext, out: &mut String) {
    let target = &link.target;
    let label = match &link.alias {
        Some(alias) => alias.clone(),
        None => {
            let note_display = if target.note.is_empty() {
                String::new()
            } else {
                target.note.clone()
            };
            match (&target.heading, &target.block) {
                // `[[Note#Heading]]` reads "Note > Heading" (confirmed, M3).
                (Some(h), _) if !note_display.is_empty() => format!("{note_display} > {h}"),
                (Some(h), _) => h.clone(),
                // A block reference shows only the note name (confirmed, M3:
                // Obsidian's autocomplete inserts the bare note name).
                (None, Some(b)) if !note_display.is_empty() => note_display,
                // Same-document block reference: no note name to show, so the
                // block id itself is the label (ASSUMPTION, see COMPAT.md).
                (None, Some(b)) => format!("^{b}"),
                (None, None) => note_display,
            }
        }
    };

    // Same-document links ([[#Heading]]): anchor only.
    if target.note.is_empty() {
        let anchor = match (&target.heading, &target.block) {
            (Some(h), _) => format!("#{}", escape_attr(h)),
            (None, Some(b)) => format!("#^{}", escape_attr(b)),
            _ => String::new(),
        };
        let _ = write!(
            out,
            "<a class=\"internal-link\" href=\"{anchor}\">{}</a>",
            escape(&label)
        );
        return;
    }

    let resolved = ctx
        .index
        .resolve(&target.note, ctx.from_note)
        .map(str::to_string);
    let fragment = match (&target.heading, &target.block) {
        (Some(h), _) => format!("#{}", escape_attr(h)),
        (None, Some(b)) => format!("#^{}", escape_attr(b)),
        (None, None) => String::new(),
    };
    match resolved {
        Some(path) => {
            let _ = write!(
                out,
                "<a class=\"internal-link\" href=\"{}{fragment}\" data-href=\"{}\">{}</a>",
                ctx.note_href(&path),
                escape_attr(&path),
                escape(&label)
            );
        }
        None => {
            let _ = write!(
                out,
                "<a class=\"internal-link is-unresolved\" data-href=\"{}\">{}</a>",
                escape_attr(&target.note),
                escape(&label)
            );
        }
    }
    let _ = LinkKind::Wiki; // resolution semantics live in VaultIndex
}

fn wrap_inline(tag: &str, children: &[Inline], ctx: &RenderContext, out: &mut String) {
    let _ = write!(out, "<{tag}>");
    render_inlines(children, ctx, out);
    let _ = write!(out, "</{tag}>");
}

fn align_attr(alignment: Option<&Alignment>) -> String {
    match alignment {
        Some(Alignment::Left) => " style=\"text-align: left;\"".into(),
        Some(Alignment::Center) => " style=\"text-align: center;\"".into(),
        Some(Alignment::Right) => " style=\"text-align: right;\"".into(),
        _ => String::new(),
    }
}

fn is_media(path: &str) -> bool {
    let lower = path.to_lowercase();
    [
        "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif", "mp4", "webm", "mov",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Sanitize a raw HTML fragment. `class`/`id`/`title` survive so vault CSS
/// keeps working; relative URLs pass through (they are vault paths); scripts
/// and event handlers never do. Applied to RAW FRAGMENTS ONLY — never to the
/// whole document, which would strip this renderer's own classes and ids.
fn sanitize(fragment: &str, ctx: &RenderContext) -> String {
    if !ctx.options.sanitize_html {
        return fragment.to_string();
    }
    let mut builder = ammonia::Builder::default();
    let mut generic: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    generic.insert("class");
    generic.insert("id");
    generic.insert("title");
    builder.generic_attributes(generic);
    builder.url_relative(ammonia::UrlRelative::PassThrough);
    builder.clean(fragment).to_string()
}

fn plain(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => text.push_str(t),
            Inline::Code(c) => text.push_str(c),
            Inline::Emph(inner)
            | Inline::Strong(inner)
            | Inline::Strike(inner)
            | Inline::Highlight(inner)
            | Inline::InlineFootnote(inner) => text.push_str(&plain(inner)),
            Inline::Link { content, .. } => text.push_str(&plain(content)),
            _ => {}
        }
    }
    text
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{26}' => out.push_str("\u{26}amp;"),
            '<' => out.push_str("\u{26}lt;"),
            '>' => out.push_str("\u{26}gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(text: &str) -> String {
    escape(text).replace('"', "\u{26}quot;")
}

/// Whether a URL must pass through untouched: an in-page anchor,
/// protocol-relative, mailto, or any absolute scheme (`https://`, …).
fn is_external_or_anchor(url: &str) -> bool {
    url.starts_with('#')
        || url.starts_with("//")
        || url.starts_with("mailto:")
        || url.contains("://")
}

/// Percent-encode a vault-relative path for use in an href/src.
fn href_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::parse_note;

    struct NoNotes;

    impl vault_core::NoteSource for NoNotes {
        fn note_source(&self, _path: &str) -> Option<String> {
            None
        }
    }

    fn render(src: &str) -> String {
        let doc = parse_note(src);
        let index = VaultIndex::new(["Note.md"]);
        let notes = NoNotes;
        let ctx = RenderContext::new(&index, "fixture.md", &notes, RenderOptions::default());
        render_html(&doc, &ctx)
    }

    /// Web-mode render: route prefixes set, index over notes + an attachment.
    fn render_web(src: &str) -> String {
        let doc = parse_note(src);
        let index = VaultIndex::new(["Note.md", "Note2.md", "attachments/img.png"]);
        let notes = NoNotes;
        let options = RenderOptions {
            note_base: Some("/n".to_string()),
            file_base: Some("/files".to_string()),
            tag_base: Some("/tag".to_string()),
            ..RenderOptions::default()
        };
        let ctx = RenderContext::new(&index, "Note.md", &notes, options);
        render_html(&doc, &ctx)
    }

    #[test]
    fn web_mode_prefixes_resolved_links_and_embeds() {
        // Wikilink to a note and a media embed land under their routes.
        let html = render_web("see [[Note2]] and ![[attachments/img.png]]\n");
        assert!(
            html.contains("href=\"/n/Note2.md\""),
            "wikilink must be routed: {html}"
        );
        assert!(
            html.contains("src=\"/files/attachments/img.png\""),
            "embed must be routed: {html}"
        );
    }

    #[test]
    fn web_mode_rewrites_relative_markdown_links_and_images() {
        let html = render_web(
            "[note](Note2.md) [file](attachments/img.png) ![img](attachments/img.png)\n",
        );
        assert!(html.contains("href=\"/n/Note2.md\""), "{html}");
        assert!(
            html.contains("href=\"/files/attachments/img.png\""),
            "{html}"
        );
        assert!(
            html.contains("src=\"/files/attachments/img.png\""),
            "{html}"
        );
    }

    #[test]
    fn web_mode_passes_through_anchors_and_absolute_urls() {
        let html = render_web("[anchor](#heading) [site](https://example.net/x)\n");
        assert!(html.contains("href=\"#heading\""), "{html}");
        assert!(html.contains("href=\"https://example.net/x\""), "{html}");
    }

    #[test]
    fn web_mode_defuses_dangerous_schemes() {
        let html = render_web("[click](javascript:alert(1)) [data](data:text/html,hi)\n");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(!html.contains("data:text/html"), "{html}");
        assert!(html.contains("href=\"#\""), "{html}");
    }

    #[test]
    fn web_mode_routes_tags() {
        let html = render_web("a #project/deep tag\n");
        assert!(html.contains("href=\"/tag/project/deep\""), "{html}");
    }

    #[test]
    fn default_options_keep_relative_hrefs() {
        // The GUI goldens depend on the unprefixed form.
        let html = render("see [[Note]]\n");
        assert!(html.contains("href=\"Note.md\""), "{html}");
    }

    /// Confirmed against real Obsidian (M3): footnote numbers follow
    /// REFERENCE order, not definition order.
    #[test]
    fn footnote_numbers_follow_reference_order() {
        let html = render("beta[^b] then alpha[^a].\n\n[^a]: alpha\n\n[^b]: beta\n");
        // The first reference is 1 even though its definition comes second.
        assert!(html.contains("href=\"#fn-b\">1</a>"), "{html}");
        assert!(html.contains("href=\"#fn-a\">2</a>"), "{html}");
        // The definitions carry the same numbers.
        assert!(
            html.contains("id=\"fn-b\" data-footnote-number=\"1\""),
            "{html}"
        );
        assert!(
            html.contains("id=\"fn-a\" data-footnote-number=\"2\""),
            "{html}"
        );
    }

    /// Confirmed against real Obsidian (M3): a block reference shows only
    /// the note name.
    #[test]
    fn block_reference_label_is_the_note_name() {
        let html = render("see [[Note#^bid]] here\n");
        assert!(html.contains(">Note</a>"), "{html}");
        assert!(!html.contains("^bid</a>"), "{html}");
    }

    /// Confirmed against real Obsidian (M3): `[[Note#Heading]]` reads
    /// "Note > Heading".
    #[test]
    fn heading_link_label_is_note_and_heading() {
        let html = render("see [[Note#Heading]] here\n");
        assert!(html.contains("Note &gt; Heading</a>"), "{html}");
    }
}
