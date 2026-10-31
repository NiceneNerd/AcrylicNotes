//! The contract between the markdown pipeline and every frontend.
//! HTML renderer (web) and widget renderer (GUI) both consume `Doc`.

/// Frontmatter is kept as raw text, byte-for-byte. Parse a *copy* for
/// metadata (tags, aliases); never re-serialize the original on save.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub frontmatter_raw: Option<String>,
    pub blocks: Vec<Block>,
}

/// Table column alignment, from `|:-:|` delimiter rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        content: Vec<Inline>,
        /// Explicit id from a `{#custom-id}` attribute (comrak `header_attributes`).
        id: Option<String>,
    },
    Paragraph {
        content: Vec<Inline>,
        block_id: Option<String>,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        /// GFM tightness: tight lists render items without paragraph wrappers.
        tight: bool,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Callout {
        kind: String,
        fold: Fold,
        /// Callout metadata after `|` in the header (`[!note|meta]`).
        metadata: Option<String>,
        title: Vec<Inline>,
        body: Vec<Block>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Math(String),
    Table {
        alignments: Vec<Alignment>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
    /// `![[Note]]` alone on its line: transclude a note/section/block.
    EmbedBlock(Embed),
    FootnoteDef {
        label: String,
        body: Vec<Block>,
    },
    /// Raw HTML block (sanitized at render time).
    Html(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub task: Option<bool>, // Some(done) for `- [ ]` / `- [x]`
    /// `^block-id` at the end of the item.
    pub block_id: Option<String>,
    pub body: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fold {
    None,
    Open,
    Closed,
} // `[!tip]`, `[!tip]+`, `[!tip]-`

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    Highlight(Vec<Inline>),
    Code(String),
    Math(String),
    Link {
        url: String,
        title: Option<String>,
        content: Vec<Inline>,
    },
    Image {
        url: String,
        alt: String,
    },
    WikiLink(WikiLink),
    Embed(Embed),
    Tag(String), // without the leading '#'
    FootnoteRef(String),
    InlineFootnote(Vec<Inline>),
    /// Raw inline HTML (e.g. `<abbr title="…">`); sanitized at render time.
    Html(String),
    SoftBreak,
    HardBreak,
}

/// `[[note#heading]]` or `[[note#^block]]`. Empty `note` = same document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WikiTarget {
    pub note: String,
    pub heading: Option<String>,
    pub block: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WikiLink {
    pub target: WikiTarget,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Embed {
    pub target: WikiTarget,
    /// `![[img.png|300]]` or `|300x200`
    pub size: Option<(u32, Option<u32>)>,
    pub alias: Option<String>,
}
