//! `vault-md` — Markdown pipeline: comrak + Obsidian extensions -> IR -> HTML.

pub mod ir {
    pub use vault_core::ir::*;
}

pub mod inline {
    pub use vault_core::inline::*;
}

// Re-export vault-core types
pub use vault_core::{
    atomic_write, atomic_write_raw, FrontmatterMeta, Heading, Note, SplitFrontmatter, Vault,
    VaultIndex, WriteError, WriteResult,
};
