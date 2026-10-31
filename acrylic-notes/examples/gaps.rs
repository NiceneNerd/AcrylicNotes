use acrylic_notes::inline::extract;
fn main() {
    let cases = [
        ("indented code block", "para\n\n    [[in-code]]\n"),
        ("frontmatter wikilink", "---\nrelated: \"[[Note]]\"\n---\nbody\n"),
        ("escaped wikilink", "\\[[not a link]]"),
        ("inline math", "$a[[b]]c$"),
        ("html block", "<div>[[x]]</div>"),
    ];
    for (n, s) in cases {
        println!("{:<22} tokens extracted: {} (ideal: 0)", n, extract(s).tokens.len());
    }
}
