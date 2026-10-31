use comrak::{nodes::NodeValue, parse_document, Arena, Options};
fn show(src: &str) {
    let arena = Arena::new();
    let mut o = Options::default();
    o.extension.footnotes = true;
    let root = parse_document(&arena, src, &o);
    print!("{:<34} =>", format!("{:?}", src));
    for n in root.descendants() {
        match &n.data.borrow().value {
            NodeValue::Text(t) => print!(" T{:?}", t),
            NodeValue::Emph => print!(" <em>"),
            NodeValue::Strong => print!(" <strong>"),
            NodeValue::Link(l) => print!(" <a {:?}>", l.url),
            NodeValue::FootnoteReference(_) => print!(" <fnref>"),
            _ => {}
        }
    }
    println!();
}
fn main() {
    for s in [
        "[[snake_case_note]]",
        "[[a *b* c]]",
        "[[Note|*alias*]]",
        "[[Note]](paren)",
        "[[Note]][^1]\n\n[^1]: x",
        "[[Note]]\n\n[[Note]]: not a ref def",
        "tag #a_b_c and #x*y*",
        "![[img.png]](x)",
        "[[a]] [text](u) [[b]]",
    ] { show(s); }
}
