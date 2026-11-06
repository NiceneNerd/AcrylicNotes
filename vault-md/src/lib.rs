//! `vault-md` — Markdown pipeline: comrak parse + Obsidian extensions -> IR
//! -> HTML.
//!
//! The IR (`ir.rs`) is the contract between frontends: an IR-to-HTML renderer
//! (web) and an IR-to-widgets renderer (GUI) both consume `Doc`.
//!
//! Modules:
//! * `ir` — the document/inline model.
//! * `inline` — pre-comrak extraction of Obsidian syntax (sentinel pass) and
//!   lowering of comrak text nodes back into IR inlines.
//! * `lower` — comrak AST -> IR.
//! * `html` — IR -> HTML (resolution, transclusion, sanitization).

pub mod html;
pub mod inline;
pub mod ir;
pub mod lower;

pub use html::{render_html, RenderContext, RenderOptions};
pub use inline::{link_spans, rewrite_link_targets, LinkSpan};
pub use ir::{Alignment, Block, Doc, Embed, Fold, Inline, ListItem, WikiLink, WikiTarget};
pub use lower::parse_note;
