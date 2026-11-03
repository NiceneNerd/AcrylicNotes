//! Server-rendered page chrome: HTML escaping and one small template.
//! No JavaScript anywhere (M5 spec); a strict CSP is sent alongside.

/// Escape text for interpolation into HTML content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{26}' => out.push_str("\u{26}amp;"),
            '<' => out.push_str("\u{26}lt;"),
            '>' => out.push_str("\u{26}gt;"),
            '"' => out.push_str("\u{26}quot;"),
            '\'' => out.push_str("\u{26}#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The stylesheet: one screen, readable, no assets. `style-src
/// 'unsafe-inline'` in the CSP covers exactly this block.
const CSS: &str = "
:root { color-scheme: light dark; }
body { font: 16px/1.6 system-ui, sans-serif; margin: 0; padding: 2rem 1rem 4rem; }
main { max-width: 46rem; margin: 0 auto; }
a { color: #06c; text-decoration: none; }
a:hover { text-decoration: underline; }
.internal-link { color: #06c; }
.internal-link.is-unresolved { color: #999; }
mark { background: #ffdf5d; }
code, pre { font: 0.9em ui-monospace, monospace; background: color-mix(in srgb, currentColor 8%, transparent); border-radius: 4px; }
code { padding: 0.1em 0.3em; }
pre { padding: 0.8em; overflow-x: auto; }
pre code { background: none; padding: 0; }
blockquote { border-left: 3px solid #999; margin-inline: 0; padding-inline: 1em; color: #555; }
table { border-collapse: collapse; }
th, td { border: 1px solid #888; padding: 0.3em 0.6em; }
img { max-width: 100%; }
h1, h2, h3 { line-height: 1.25; }
nav { max-width: 46rem; margin: 0 auto 1.5rem; display: flex; gap: 1rem; align-items: baseline; }
nav form { margin-left: auto; display: flex; gap: 0.3rem; }
footer { max-width: 46rem; margin: 3rem auto 0; color: #888; font-size: 0.85em; }
.muted { color: #888; }
.tag { color: #06c; }
.crumbs { color: #888; font-size: 0.9em; }
.crumbs a { color: #888; }
.note-title { margin-top: 0.2em; }
.footnotes { margin-top: 2em; border-top: 1px solid #999; padding-top: 0.4em; font-size: 0.92em; }
.footnotes p { margin: 0.2em 0; }
.footnote-back { text-decoration: none; color: #888; }
.tree details { margin: 0.15rem 0; }
.tree details details { margin-left: 1.1rem; border-left: 1px solid color-mix(in srgb, currentColor 25%, transparent); padding-left: 0.5rem; }
.tree summary { cursor: pointer; user-select: none; }
.tree summary .count { color: #888; font-size: 0.85em; }
.tree ul { list-style: none; margin: 0.15rem 0 0.3rem; padding-left: 1.4rem; }
.tree li { margin: 0.1rem 0; }
";

/// Wrap rendered note/page HTML in the site chrome.
pub fn page(title: &str, vault_name: &str, body_html: &str) -> String {
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} — {vault}</title>\n\
         <style>{css}</style>\n\
         </head>\n\
         <body>\n\
         <nav><a href=\"/\">{vault}</a>\
         <form action=\"/search\" method=\"get\">\
         <input type=\"search\" name=\"q\" placeholder=\"Search notes…\">\
         </form></nav>\n\
         <main>\n{body}\n</main>\n\
         <footer>read-only view · <a href=\"/\">index</a></footer>\n\
         </body>\n\
         </html>\n",
        title = escape(title),
        vault = escape(vault_name),
        css = CSS,
        body = body_html,
    )
}

/// The Content-Security-Policy sent with every response: no script, no
/// remote anything; the inline stylesheet is the one exception.
pub const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; form-action 'self'; base-uri 'none'";

/// Percent-encode a vault-relative path for use in an href.
pub fn path_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
