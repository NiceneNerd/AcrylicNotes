use comrak::{nodes::NodeValue, parse_document, Arena, Options};
use acrylic_notes::inline::{extract, lower_text};
fn main() {
    for src in ["[[a *b* c]] and [[Note|*alias*]]", "[[Note]](paren) #tag", "![[img.png|300]](x)"] {
        let e = extract(src);
        let arena = Arena::new();
        let root = parse_document(&arena, &e.text, &Options::default());
        print!("{:<38} =>", src);
        for n in root.descendants() {
            match &n.data.borrow().value {
                NodeValue::Text(t) => print!(" {:?}", lower_text(t, &e.tokens)),
                NodeValue::Link(l) => print!(" <a:{}>", l.url),
                _ => {}
            }
        }
        println!();
    }
}
