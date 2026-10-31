//! `vault-core` — UI-free vault operations.
//!
//! * Scan and index an Obsidian-compatible vault.
//! * File watching with debouncing.
//! * Link resolution.
//! * YAML frontmatter handling (preserve bytes verbatim).
//! * Safe atomic writes with external-change detection.

pub mod frontmatter;
pub mod links;
pub mod resolve;
pub mod vault;
pub mod write;

pub use frontmatter::{
    parse_frontmatter_meta, split_frontmatter, FrontmatterMeta, SplitFrontmatter,
};
pub use links::{LinkKind, RawLink};
pub use resolve::VaultIndex;
pub use vault::{Heading, Note, Vault};
pub use write::{atomic_write, atomic_write_raw, WriteError, WriteResult};
