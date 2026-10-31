//! `vault-md` — Markdown pipeline: comrak parse + Obsidian extensions -> IR.
//!
//! The IR (`ir.rs`) is the contract between frontends: an IR-to-HTML renderer
//! (web) and an IR-to-widgets renderer (GUI) both consume `Doc`.
//!
//! Modules:
//! * `ir` — the document/inline model.
//! * `inline` — pre-comrak extraction of Obsidian syntax (sentinel pass) and
//!   lowering of comrak text nodes back into IR inlines.

pub mod inline;
pub mod ir;
