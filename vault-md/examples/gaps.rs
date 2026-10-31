//! Demonstrates the HANDOFF 4.3 extraction gaps and their status.
//!
//! Definite gaps (must be 0, enforced by unit tests in `inline.rs`):
//!   * indented code blocks — fixed via `indented_code_mask`
//!   * frontmatter — fixed via `extract_document` (split first)
//! Ambiguous gaps (stay 1 until confirmed against real Obsidian in M3):
//!   * escaped `\[[x]]`, inline math `$a[[b]]c$`, raw HTML blocks
use vault_md::inline::{extract, extract_document};

fn main() {
    let definite = [
        (
            "indented code block",
            extract("para\n\n    [[in-code]]\n").tokens.len(),
        ),
        (
            "frontmatter wikilink",
            extract_document("---\nrelated: \"[[Note]]\"\n---\nbody\n")
                .extracted
                .tokens
                .len(),
        ),
    ];
    let ambiguous = [
        ("escaped wikilink", extract("\\[[not a link]]").tokens.len()),
        ("inline math", extract("$a[[b]]c$").tokens.len()),
        ("html block", extract("<div>[[x]]</div>").tokens.len()),
    ];
    println!("definite gaps (must be 0):");
    for (n, n_tokens) in definite {
        println!("  {:<22} tokens extracted: {} (ideal: 0)", n, n_tokens);
    }
    println!("ambiguous gaps (M3: confirm against real Obsidian):");
    for (n, n_tokens) in ambiguous {
        println!("  {:<22} tokens extracted: {} (ideal: ?)", n, n_tokens);
    }
}
