use comrak::{nodes::NodeValue, parse_document, Arena, Options};
fn main() {
    let arena = Arena::new();
    let mut o = Options::default();
    o.extension.footnotes = true;
    o.extension.front_matter_delimiter = Some("---".into());
    // wikilinks deliberately OFF: we lower them ourselves
    let src = "See [[Note|alias]] and ![[img.png|300]] and #tag, ==hi== %%c%%.";
    let root = parse_document(&arena, src, &o);
    for n in root.descendants() {
        if let NodeValue::Text(t) = &n.data.borrow().value { println!("Text node: {:?}", t); }
    }
}
