//! Server-rendered page chrome: HTML escaping, the twin themes (light/dark
//! from the user's palette), and the page template.
//!
//! No JavaScript anywhere (M5 spec); a strict CSP is sent alongside. The
//! theme is a cookie: `auto` (no cookie) follows `prefers-color-scheme`
//! via a CSS media query, explicit choices are rendered as a `data-theme`
//! attribute on `<html>`.

use axum::http::{header, HeaderMap};

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

/// The theme the current visitor has chosen (cookie `vault_theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePref {
    Auto,
    Light,
    Dark,
}

impl ThemePref {
    /// Parse a cookie/query value; anything unknown is `Auto`.
    pub fn parse(value: &str) -> Self {
        match value {
            "light" => ThemePref::Light,
            "dark" => ThemePref::Dark,
            _ => ThemePref::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::Auto => "auto",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        }
    }
}

/// Per-request chrome inputs: where the visitor is (for the theme toggle
/// links) and which theme they picked.
pub struct Chrome {
    /// Path + query of the current request, e.g. `/search?q=x`.
    pub location: String,
    pub theme: ThemePref,
}

/// Light-theme custom properties — the light twin of the user's palette.
/// Their Substack ships identical values for both modes, so the dark side
/// is verbatim and this side is derived: same hue family swapped in
/// luminance, tan accent and orange bg-accent unchanged.
const LIGHT_VARS: &str = "\
--bg: rgb(250 248 244);
--bg-secondary: rgb(255 255 255);
--bg-tertiary: rgb(233 229 222);
--bg-accent: rgb(255 103 25 / 0.2);
--fg: rgb(27 22 33);
--fg-secondary: rgb(91 88 96);
--fg-tertiary: rgb(124 121 128);
--accent: rgb(161 136 101);
--accent-hover: rgb(147 123 90);
--accent-contrast: rgb(255 255 255);";

/// Dark-theme custom properties — the user's palette verbatim.
const DARK_VARS: &str = "\
--bg: rgb(27 22 33);
--bg-secondary: rgb(41 36 46);
--bg-tertiary: rgb(58 54 63);
--bg-accent: rgb(255 103 25 / 0.2);
--fg: rgb(255 255 255);
--fg-secondary: rgb(164 162 166);
--fg-tertiary: rgb(124 121 128);
--accent: rgb(161 136 101);
--accent-hover: rgb(147 123 90);
--accent-contrast: rgb(255 255 255);";

/// The stylesheet: variables per theme, then rules that only use variables.
/// `style-src 'unsafe-inline'` in the CSP covers exactly this block.
fn stylesheet() -> String {
    format!(
        "\
:root {{ color-scheme: light dark; }}
[data-theme='light'] {{ color-scheme: light; {LIGHT_VARS} }}
[data-theme='dark'] {{ color-scheme: dark; {DARK_VARS} }}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme]) {{ color-scheme: dark; {DARK_VARS} }}
}}
body {{ font: 16px/1.6 system-ui, sans-serif; margin: 0; padding: 2rem 1rem 4rem; background: var(--bg); color: var(--fg); }}
::selection {{ background: var(--bg-accent); }}
main {{ max-width: 46rem; margin: 0 auto; }}
a {{ color: var(--accent); text-decoration: none; }}
a:hover {{ color: var(--accent-hover); text-decoration: underline; }}
.internal-link {{ color: var(--accent); }}
.internal-link.is-unresolved {{ color: var(--fg-tertiary); }}
mark {{ background: var(--bg-accent); color: var(--fg); }}
code, pre {{ font: 0.9em ui-monospace, monospace; background: var(--bg-secondary); border-radius: 4px; }}
code {{ padding: 0.1em 0.3em; }}
pre {{ padding: 0.8em; overflow-x: auto; border: 1px solid var(--bg-tertiary); }}
pre code {{ background: none; padding: 0; border: none; }}
blockquote {{ border-left: 3px solid var(--fg-tertiary); margin-inline: 0; padding-inline: 1em; color: var(--fg-secondary); }}
table {{ border-collapse: collapse; }}
th, td {{ border: 1px solid var(--fg-tertiary); padding: 0.3em 0.6em; }}
img {{ max-width: 100%; }}
h1, h2, h3 {{ line-height: 1.25; }}
nav {{ max-width: 46rem; margin: 0 auto 1.5rem; display: flex; gap: 1rem; align-items: baseline; }}
nav form {{ margin-left: auto; display: flex; gap: 0.3rem; }}
nav input {{ background: var(--bg-secondary); color: var(--fg); border: 1px solid var(--bg-tertiary); border-radius: 4px; padding: 0.15em 0.5em; }}
nav .theme {{ display: flex; gap: 0.4rem; font-size: 0.85em; }}
nav .theme a {{ color: var(--fg-tertiary); }}
nav .theme a.active {{ color: var(--accent); font-weight: 600; }}
footer {{ max-width: 46rem; margin: 3rem auto 0; color: var(--fg-tertiary); font-size: 0.85em; }}
.muted {{ color: var(--fg-secondary); }}
.tag {{ color: var(--accent); }}
.crumbs {{ color: var(--fg-secondary); font-size: 0.9em; }}
.crumbs a {{ color: var(--fg-secondary); }}
.note-title {{ margin-top: 0.2em; }}
.footnotes {{ margin-top: 2em; border-top: 1px solid var(--fg-tertiary); padding-top: 0.4em; font-size: 0.92em; }}
.footnotes p {{ margin: 0.2em 0; }}
.footnote-back {{ text-decoration: none; color: var(--fg-tertiary); }}
sup {{ line-height: 0; }}
.tree details {{ margin: 0.15rem 0; }}
.tree details details {{ margin-left: 1.1rem; border-left: 1px solid var(--bg-tertiary); padding-left: 0.5rem; }}
.tree summary {{ cursor: pointer; user-select: none; }}
.tree summary .count {{ color: var(--fg-tertiary); font-size: 0.85em; }}
.tree ul {{ list-style: none; margin: 0.15rem 0 0.3rem; padding-left: 1.4rem; }}
.tree li {{ margin: 0.1rem 0; }}"
    )
}

/// The theme toggle: Light / Dark / Auto links back to the current
/// location; the active choice is highlighted (`Auto` when no cookie).
fn theme_nav(chrome: &Chrome) -> String {
    let sep = if chrome.location.contains('?') {
        "&"
    } else {
        "?"
    };
    let link = |pref: ThemePref, label: &str| {
        let active = if chrome.theme == pref {
            " class=\"active\""
        } else {
            ""
        };
        format!(
            "<a{active} href=\"{}{}theme={}\">{label}</a>",
            escape(&chrome.location),
            sep,
            pref.as_str(),
        )
    };
    format!(
        "<span class=\"theme\">{} {} {}</span>",
        link(ThemePref::Light, "Light"),
        link(ThemePref::Dark, "Dark"),
        link(ThemePref::Auto, "Auto"),
    )
}

/// Wrap rendered note/page HTML in the site chrome.
pub fn page(title: &str, vault_name: &str, body_html: &str, chrome: &Chrome) -> String {
    // Auto (no cookie): no data-theme attribute — CSS prefers-color-scheme
    // decides. Explicit choices pin the attribute.
    let html_tag = match chrome.theme {
        ThemePref::Light => "<html lang=\"en\" data-theme=\"light\">",
        ThemePref::Dark => "<html lang=\"en\" data-theme=\"dark\">",
        ThemePref::Auto => "<html lang=\"en\">",
    };
    format!(
        "<!doctype html>\n\
         {html_tag}\n\
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
         </form>\
         {theme_nav}</nav>\n\
         <main>\n{body}\n</main>\n\
         <footer>read-only view · <a href=\"/\">index</a></footer>\n\
         </body>\n\
         </html>\n",
        title = escape(title),
        vault = escape(vault_name),
        css = stylesheet(),
        theme_nav = theme_nav(chrome),
        body = body_html,
    )
}

/// The Content-Security-Policy sent with every response: no script, no
/// remote anything; the inline stylesheet is the one exception.
pub const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; form-action 'self'; base-uri 'none'";

/// The name of the theme cookie (also used by the middleware).
pub const THEME_COOKIE: &str = "vault_theme";

/// Read the visitor's theme choice from the request's cookies.
pub fn theme_of_headers(headers: &HeaderMap) -> ThemePref {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == THEME_COOKIE)
        .map(|(_, value)| ThemePref::parse(value))
        .unwrap_or(ThemePref::Auto)
}

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
