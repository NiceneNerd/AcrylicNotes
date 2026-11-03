//! The HTTP surface: routes, auth + security-header middleware, handlers.
//!
//! Read-only by construction: nothing here writes to the vault. Every
//! user-controlled path goes through [`crate::guard::safe_resolve`], and
//! every response carries a script-free CSP (M5 spec).

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use vault_core::{NoteSource, ObsidianSettings, Vault, VaultIndex};
use vault_md::{parse_note, render_html, RenderContext, RenderOptions};

use crate::guard;
use crate::page;

/// Cap on vault-wide search results shown (same as the GUI).
const SEARCH_MAX_HITS: usize = 200;

/// Immutable snapshot of the vault for the server's lifetime: the vault
/// (read side only), its resolution index, and the request guards.
pub struct ServerState {
    pub vault: Vault,
    pub index: VaultIndex,
    pub root: PathBuf,
    pub vault_name: String,
    pub auth: Option<String>,
    pub strict_breaks: bool,
}

impl ServerState {
    pub fn new(vault: Vault, auth: Option<String>) -> Self {
        let root = vault.root().to_path_buf();
        let vault_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("vault")
            .to_string();
        let index = vault.index();
        let strict_breaks = ObsidianSettings::load(&root).strict_line_breaks;
        Self {
            vault,
            index,
            root,
            vault_name,
            auth,
            strict_breaks,
        }
    }
}

/// The read-only routes. Auth + security headers apply to everything.
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/n/{*path}", get(note_page))
        .route("/tag/{*tag}", get(tag_page))
        .route("/search", get(search_page))
        .route("/files/{*path}", get(file_bytes))
        .layer(middleware::from_fn_with_state(state.clone(), check_auth))
        .with_state(state)
}

/// Cookie that carries the token after the first authenticated visit, so
/// plain navigation, image fetches, and form submits stay authenticated
/// (browsers do not attach Authorization headers to ordinary requests).
const COOKIE_NAME: &str = "vault_token";

/// Whether the request's `Cookie` headers contain the expected token.
fn cookie_matches(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .any(|(name, value)| name == COOKIE_NAME && value == expected)
}

/// Auth gate (when a token is configured) plus the security headers every
/// response must carry: a CSP that allows no script at all, and nosniff.
/// Also honors `?theme=light|dark|auto` by setting the theme cookie and
/// redirecting to the same location without the parameter.
async fn check_auth(
    State(state): State<Arc<ServerState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // Theme switch requested? Set the cookie, redirect to the same location
    // without the parameter — works pre-render, so the very next response
    // already carries the chosen theme. Allowed before auth: it only sets a
    // cookie and redirects, it exposes nothing.
    let theme_redirect = theme_switch(request.uri());
    let mut response = if let Some(expected) = &state.auth {
        let bearer_ok = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.strip_prefix("Bearer ") == Some(expected.as_str()));
        let query_ok = request.uri().query().is_some_and(|query| {
            query.split('&').any(|pair| {
                pair.split_once('=')
                    .is_some_and(|(key, value)| key == "token" && value == expected)
            })
        });
        let cookie_ok = cookie_matches(request.headers(), expected);
        if bearer_ok || query_ok || cookie_ok {
            match &theme_redirect {
                Some((cleaned, theme_cookie)) => {
                    let mut response = axum::response::Redirect::to(cleaned).into_response();
                    // First visit: hand out the auth session cookie too, so
                    // one click both signs in and themes.
                    if !cookie_ok {
                        append_set_cookie(
                            &mut response,
                            &format!("{COOKIE_NAME}={expected}; HttpOnly; SameSite=Lax; Path=/"),
                        );
                    }
                    append_set_cookie(&mut response, theme_cookie);
                    response
                }
                None => {
                    let mut response = next.run(request).await;
                    // First token-authenticated visit (by query or header):
                    // hand the browser a session cookie so the token is not
                    // needed again.
                    if !cookie_ok {
                        append_set_cookie(
                            &mut response,
                            &format!("{COOKIE_NAME}={expected}; HttpOnly; SameSite=Lax; Path=/"),
                        );
                    }
                    response
                }
            }
        } else {
            unauthorized(&state)
        }
    } else {
        match &theme_redirect {
            Some((cleaned, theme_cookie)) => {
                let mut response = axum::response::Redirect::to(cleaned).into_response();
                append_set_cookie(&mut response, theme_cookie);
                response
            }
            None => next.run(request).await,
        }
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(page::CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Append a Set-Cookie header (skipped for values that are not valid
/// header text).
fn append_set_cookie(response: &mut Response, value: &str) {
    if let Ok(header_value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

/// If the request carries `?theme=light|dark|auto`, return the redirect
/// target with the parameter stripped plus the Set-Cookie value for the
/// choice. Unknown values are ignored (rendered normally).
fn theme_switch(uri: &axum::http::Uri) -> Option<(String, String)> {
    let query = uri.query()?;
    let mut theme_value = None;
    let mut kept: Vec<&str> = Vec::new();
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("theme", value)) if matches!(value, "light" | "dark" | "auto") => {
                theme_value = Some(value);
            }
            _ => kept.push(pair),
        }
    }
    let value = theme_value?;
    let target = if kept.is_empty() {
        uri.path().to_string()
    } else {
        format!("{}?{}", uri.path(), kept.join("&"))
    };
    Some((
        target,
        format!(
            "{}={value}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
            page::THEME_COOKIE
        ),
    ))
}

/// The per-request chrome for `page()`: current location + theme choice.
fn chrome(headers: &HeaderMap, uri: &axum::http::Uri) -> page::Chrome {
    page::Chrome {
        location: uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| uri.path().to_string()),
        theme: page::theme_of_headers(headers),
    }
}

fn unauthorized(state: &ServerState) -> Response {
    let body = page::page(
        "Unauthorized",
        &state.vault_name,
        "<h1>401 — unauthorized</h1><p>Open any page with <code>?token=…</code> \
         once (or an <code>Authorization: Bearer …</code> header); a session \
         cookie keeps you signed in from then on.</p>",
        &page::Chrome {
            location: "/".to_string(),
            theme: page::ThemePref::Auto,
        },
    );
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"vault-web\"")],
        Html(body),
    )
        .into_response()
}

fn not_found(state: &ServerState) -> Response {
    let body = page::page(
        "Not found",
        &state.vault_name,
        "<h1>404 — not in this vault</h1><p>The path does not exist inside the \
         vault, or it is hidden (dotfile) or escapes the vault.</p>",
        &page::Chrome {
            location: "/".to_string(),
            theme: page::ThemePref::Auto,
        },
    );
    (StatusCode::NOT_FOUND, Html(body)).into_response()
}

// --- handlers --------------------------------------------------------------

/// Index: the vault as a collapsible folder tree (pure `<details>`/CSS),
/// root-level notes beside it, tags below.
async fn index_page(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let chrome = chrome(&headers, &uri);
    // folder -> (path, title), "" for the root.
    let mut groups: std::collections::BTreeMap<String, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for note in state.vault.notes_sorted() {
        let folder = note
            .path
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        groups
            .entry(folder)
            .or_default()
            .push((note.path.clone(), note.title.clone()));
    }
    // Folders without notes still appear (they are valid navigation targets
    // from the GUI's perspective and new notes can land in them).
    let folder_list = state.vault.folders();

    let mut body = String::new();
    // The vault as a collapsible folder tree (pure <details>/CSS, collapsed
    // by default like the GUI sidebar).
    let tree = crate::tree::build(&folder_list, &groups);
    body.push_str(&crate::tree::render(&tree));

    // Root-level notes (the tree holds folders only).
    let root_notes = groups.get("").cloned().unwrap_or_default();
    if !root_notes.is_empty() {
        body.push_str("<h2>(root)</h2>\n<ul>\n");
        for (path, title) in &root_notes {
            body.push_str(&format!(
                "<li><a href=\"/n/{}\">{}</a></li>\n",
                path_encode(path),
                page::escape(title)
            ));
        }
        body.push_str("</ul>\n");
    }

    let tags = state.vault.all_tags();
    if !tags.is_empty() {
        body.push_str("<h2>Tags</h2>\n<p>");
        for tag in &tags {
            body.push_str(&format!(
                "<a class=\"tag\" href=\"/tag/{}\">#{}</a> ",
                path_encode(tag),
                page::escape(tag)
            ));
        }
        body.push_str("</p>\n");
    }
    Html(page::page("Index", &state.vault_name, &body, &chrome)).into_response()
}

/// One rendered note. Non-notes redirect to `/files/`.
async fn note_page(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(path): Path<String>,
) -> Response {
    let chrome = chrome(&headers, &uri);
    let rel = path.trim_start_matches('/');
    // Same confinement as attachments, BEFORE anything else: dot-dot and
    // dotfile components are refused (a redirect would leak their shape),
    // and symlinks may not escape the vault.
    if guard::safe_resolve(&state.root, rel).is_none() {
        return not_found(&state);
    }
    if !rel.ends_with(".md") {
        return Redirect::temporary(&format!("/files/{}", path_encode(rel))).into_response();
    }
    let Some(source) = state.vault.note_source(rel) else {
        return not_found(&state);
    };
    let doc = parse_note(&source);
    let options = RenderOptions {
        strict_line_breaks: state.strict_breaks,
        note_base: Some("/n".to_string()),
        file_base: Some("/files".to_string()),
        tag_base: Some("/tag".to_string()),
        ..RenderOptions::default()
    };
    let ctx = RenderContext::new(&state.index, rel, &state.vault, options);
    let rendered = render_html(&doc, &ctx);
    // Prefer the indexed (frontmatter) title; fall back to the file stem.
    let title = state
        .vault
        .get_note(rel)
        .map(|note| note.title.clone())
        .unwrap_or_else(|| title_of(rel).trim_end_matches(".md").to_string());
    // Breadcrumbs up the folder chain; folder segments link to their tree
    // entry on the index page.
    let mut crumbs = format!(
        "<nav class=\"crumbs\"><a href=\"/\">{}</a>",
        page::escape(&state.vault_name)
    );
    let mut walked = String::new();
    for segment in rel
        .split('/')
        .take(rel.split('/').count().saturating_sub(1))
    {
        walked = if walked.is_empty() {
            segment.to_string()
        } else {
            format!("{walked}/{segment}")
        };
        crumbs.push_str(&format!(
            " / <a href=\"/#{}\">{}</a>",
            page::path_encode(&walked),
            page::escape(segment)
        ));
    }
    crumbs.push_str("</nav>\n");
    let body = format!(
        "{crumbs}<h1 class=\"note-title\">{}</h1>\n{rendered}",
        page::escape(&title)
    );
    Html(page::page(&title, &state.vault_name, &body, &chrome)).into_response()
}

/// The notes of one tag (nested tags arrive as `a/b`).
async fn tag_page(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(tag): Path<String>,
) -> Response {
    let chrome = chrome(&headers, &uri);
    let tag = tag.trim_start_matches('/');
    let mut notes = state.vault.notes_for_tag(tag);
    notes.sort();
    let mut body = format!(
        "<h1>#{}</h1>\n<p><a href=\"/\">← all notes</a></p>\n<ul>\n",
        page::escape(tag)
    );
    for path in &notes {
        body.push_str(&format!(
            "<li><a href=\"/n/{}\">{}</a></li>\n",
            path_encode(path),
            page::escape(title_of(path))
        ));
    }
    body.push_str("</ul>\n");

    // All tags for navigation.
    let tags = state.vault.all_tags();
    if !tags.is_empty() {
        body.push_str("<h2>All tags</h2>\n<p>");
        for other in &tags {
            body.push_str(&format!(
                "<a class=\"tag\" href=\"/tag/{}\">#{}</a> ",
                path_encode(other),
                page::escape(other)
            ));
        }
        body.push_str("</p>\n");
    }
    Html(page::page(
        &format!("#{tag}"),
        &state.vault_name,
        &body,
        &chrome,
    ))
    .into_response()
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

/// Literal ripgrep over the vault's notes (same engine as the GUI).
async fn search_page(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<SearchQuery>,
) -> Response {
    let chrome = chrome(&headers, &uri);
    let q = query.q.clone().unwrap_or_default();
    let mut body = String::from(
        "<form action=\"/search\" method=\"get\">\
         <input type=\"search\" name=\"q\" placeholder=\"Search notes…\"> \
         <button type=\"submit\">Search</button></form>\n",
    );
    let trimmed = q.trim();
    if !trimmed.is_empty() {
        let hits =
            vault_core::search::search(&state.root, trimmed, SEARCH_MAX_HITS).unwrap_or_default();
        body.push_str(&format!(
            "<p>{} match{} for <code>{}</code></p>\n<ul>\n",
            hits.len(),
            if hits.len() == 1 { "" } else { "es" },
            page::escape(trimmed)
        ));
        for hit in &hits {
            body.push_str(&format!(
                "<li><a href=\"/n/{}\">{}</a> <code>{}</code></li>\n",
                path_encode(&hit.path),
                page::escape(title_of(&hit.path)),
                page::escape(&hit.line)
            ));
        }
        body.push_str("</ul>\n");
    }
    Html(page::page("Search", &state.vault_name, &body, &chrome)).into_response()
}

/// Attachments: raw bytes with a guessed content type. Same confinement as
/// notes; `nosniff` + CSP are added by the middleware.
async fn file_bytes(State(state): State<Arc<ServerState>>, Path(path): Path<String>) -> Response {
    let rel = path.trim_start_matches('/');
    let Some(full) = guard::safe_resolve(&state.root, rel) else {
        return not_found(&state);
    };
    match std::fs::read(&full) {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&full).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.essence_str())],
                bytes,
            )
                .into_response()
        }
        Err(_) => not_found(&state),
    }
}

// --- helpers ---------------------------------------------------------------

fn title_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

use crate::page::path_encode;
