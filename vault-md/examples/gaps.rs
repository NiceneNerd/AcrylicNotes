//! Demonstrates the HANDOFF 4.3 extraction gaps and their status.
//!
//! Every case below must extract ZERO tokens; the example panics if one
//! does, so it doubles as a smoke test of the confirmed rules.
//!
//! Confirmed against real Obsidian (M3, 2026-10-31):
//!   * escaped `\[[x]]` — not a link
//!   * inline math `$a[[b]]c$` — no links inside math
//!   * raw HTML blocks — literal text
//!
//! Fixed earlier (M2):
//!   * indented code blocks — via `indented_code_mask`
//!   * frontmatter — via `extract_document` (split first)
use vault_md::inline::{extract, extract_document};

fn main() {
    let cases = [
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
        ("escaped wikilink", extract("\\[[not a link]]").tokens.len()),
        ("inline math", extract("$a[[b]]c$").tokens.len()),
        ("html block", extract("<div>[[x]]</div>").tokens.len()),
    ];

    println!("HANDOFF 4.3 extraction gaps (all must be 0):");
    let mut leaked = 0;
    for (name, tokens) in cases {
        println!("  {name:<22} tokens extracted: {tokens} (ideal: 0)");
        leaked += tokens;
    }
    assert_eq!(leaked, 0, "a gap leaked a token again");
    println!("all gaps closed");
}
