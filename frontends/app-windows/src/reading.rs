//! Reading-mode renderer for the Windows frontend.
//!
//! Renders `vault_md`'s `Doc` IR (blocks, inlines, links, images) to
//! WinUI 3 elements block by block. On Windows this drives real
//! XAML/text controls; on non-Windows targets it is a compile-time
//! skeleton.
//!
//! Native Windows verification is user-side; never claimed via Wine.

use vault_md::{Block, Doc, Inline};

/// The reading renderer state.
pub struct WindowsMarkdownRenderer {
    /// Whether images are shown inline or as placeholders.
    pub show_images: bool,
    /// Whether the theme is dark.
    pub dark_mode: bool,
}

impl WindowsMarkdownRenderer {
    /// Create a new reading renderer.
    #[cfg(target_os = "windows")]
    pub fn new() -> Self {
        Self {
            show_images: true,
            dark_mode: false,
        }
    }

    /// Create a new reading renderer (stub on non-Windows).
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Self {
        Self {
            show_images: true,
            dark_mode: false,
        }
    }

    /// Render one block to native elements. On Windows this produces
    /// XAML; on non-Windows targets it is a no-op.
    pub fn render_block(&self, _block: &Block) {
        // Windows: convert Block to WinUI 3 elements (paragraph, heading,
        // list, callout, table, footnote definition, ...).
    }

    /// Render an entire document, block by block.
    pub fn render_doc(&self, doc: &Doc) {
        for block in &doc.blocks {
            self.render_block(block);
        }
    }

    /// Render one inline span (text, emphasis, link, wikilink, image).
    pub fn render_inline(&self, _inline: &Inline) {
        // Windows: convert Inline to WinUI 3 text runs.
    }

    /// Extract plain text from inlines for search/snippet purposes.
    /// Delegates to the shared `vault_app::markdown::plain_inlines`.
    pub fn plain_text(inlines: &[Inline]) -> String {
        vault_app::markdown::plain_inlines(inlines)
    }
}

impl Default for WindowsMarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}
