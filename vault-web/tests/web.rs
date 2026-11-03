//! M5 acceptance tests: routes, auth, and the path-traversal suite.
//!
//! The traversal cases are the heart of the milestone: `..`, encoded `..`,
//! double-encoded `..`, dotfiles, and symlinks out of the vault must all be
//! refused, while everything inside the vault keeps working.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;
use vault_core::Vault;
use vault_web::server::{router, ServerState};

const TOKEN: &str = "tok123";

/// A vault plus an outside secret, symlinks pre-created so the scan sees
/// them (WalkDir does not follow symlinks; the guards must still refuse).
fn setup() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("vault");
    std::fs::create_dir_all(root.join("Notes")).unwrap();
    std::fs::create_dir_all(root.join("attachments")).unwrap();
    std::fs::create_dir_all(root.join(".obsidian")).unwrap();
    std::fs::write(
        root.join("Note.md"),
        "---\ntitle: Note One\ntags: [mytag]\n---\n\n# Hello\n\nsee [[Notes/Nested]] and ![[img.png]] ![[attachments/img.png]]\n\ntagged #mytag here\n",
    )
    .unwrap();
    std::fs::write(root.join("Notes/Nested.md"), "some glassware notes\n").unwrap();
    std::fs::write(root.join("attachments/img.png"), b"\x89PNG-fake").unwrap();
    std::fs::write(root.join(".obsidian/app.json"), "{}").unwrap();
    std::fs::write(root.join(".hidden.md"), "hidden\n").unwrap();

    // The secret that must never be reachable, outside the vault.
    std::fs::write(tmp.path().join("secret.md"), "TOP SECRET\n").unwrap();

    // Symlinked note pointing outside; symlinked directory pointing outside;
    // symlink that stays inside (must keep working).
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tmp.path().join("secret.md"), root.join("leak.md")).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("note.md"), "outside note\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("mirror")).unwrap();
        std::os::unix::fs::symlink(root.join("Note.md"), root.join("alias.md")).unwrap();
    }
    (tmp, root)
}

fn app_with(root: &std::path::Path, auth: Option<String>) -> Router {
    let vault = Vault::open(root).unwrap();
    router(Arc::new(ServerState::new(vault, auth)))
}

fn app(root: &std::path::Path) -> Router {
    app_with(root, None)
}

fn token_app(root: &std::path::Path) -> Router {
    app_with(root, Some(TOKEN.to_string()))
}

/// GET a URI; returns status, headers, and the body as text.
async fn get(app: Router, uri: &str) -> (StatusCode, HeaderMap, String) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = BodyExt::collect(body).await.unwrap().to_bytes();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

// --- routes ----------------------------------------------------------------

#[tokio::test]
async fn note_renders_with_routed_links() {
    let (_tmp, root) = setup();
    let (status, _headers, body) = get(app(&root), "/n/Note.md").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Wikilink, embed, and tag all route through the server.
    assert!(body.contains("href=\"/n/Notes/Nested.md\""), "{body}");
    assert!(
        body.contains("src=\"/files/attachments/img.png\""),
        "{body}"
    );
    assert!(body.contains("href=\"/tag/mytag\""), "{body}");
    // The frontmatter title is the page title.
    assert!(body.contains("<title>Note One — vault</title>"), "{body}");
    // Raw HTML in the note is sanitized (no script survives rendering).
}

#[tokio::test]
async fn note_missing_404s() {
    let (_tmp, root) = setup();
    let (status, _, _) = get(app(&root), "/n/Missing.md").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_note_redirects_to_files() {
    let (_tmp, root) = setup();
    let (status, headers, _) = get(app(&root), "/n/attachments/img.png").await;
    assert!(status.is_redirection(), "{status}");
    assert_eq!(headers[header::LOCATION], "/files/attachments/img.png");
}

#[tokio::test]
async fn index_lists_notes_and_tags() {
    let (_tmp, root) = setup();
    let (status, _, body) = get(app(&root), "/").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("href=\"/n/Note.md\""), "{body}");
    assert!(body.contains("Note One"), "{body}");
    assert!(body.contains("href=\"/n/Notes/Nested.md\""), "{body}");
    assert!(body.contains("href=\"/tag/mytag\""), "{body}");
}

#[tokio::test]
async fn index_groups_are_flat_and_every_list_closes() {
    let (_tmp, root) = setup();
    let (status, _, body) = get(app(&root), "/").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // One ul per group, every one of them closed (the runaway-indent bug
    // opened a ul per folder and closed none but the last).
    assert_eq!(
        body.matches("<ul>").count(),
        body.matches("</ul>").count(),
        "{body}"
    );
    // Folders are siblings: each header starts a fresh group.
    assert!(body.contains("<h2>Notes</h2>\n<ul>"), "{body}");
    // Empty folders appear too (attachments holds no notes).
    assert!(body.contains("<h2>attachments</h2>"), "{body}");
    assert!(body.contains("(empty)"), "{body}");
}

#[tokio::test]
async fn cookie_session_keeps_you_signed_in() {
    let (_tmp, root) = setup();
    let app = token_app(&root);

    // First visit with the token in the query: 200 + session cookie.
    let (status, headers, _) = get(app.clone(), "/?token=tok123").await;
    assert_eq!(status, StatusCode::OK);
    let set_cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.starts_with("vault_token=tok123"), "{set_cookie}");
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");

    // A plain follow-up navigation with only the cookie: still 200.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/n/Note.md")
                .header(header::COOKIE, "vault_token=tok123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // A wrong cookie is still refused.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, "vault_token=nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn embeds_resolve_and_load_with_the_cookie() {
    let (_tmp, root) = setup();
    let app = token_app(&root);

    // The note page routes both embeds (bare name AND direct path) to /files.
    let (status, _, body) = get(app.clone(), "/n/Note.md?token=tok123").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("src=\"/files/attachments/img.png\""),
        "{body}"
    );

    // A browser fetches the image as its own request; the cookie set by the
    // page visit must carry it through (this was the broken-images bug).
    let response = app
        .oneshot(
            Request::builder()
                .uri("/files/attachments/img.png")
                .header(header::COOKIE, "vault_token=tok123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn tag_page_lists_notes() {
    let (_tmp, root) = setup();
    let (status, _, body) = get(app(&root), "/tag/mytag").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("href=\"/n/Note.md\""), "{body}");
}

#[tokio::test]
async fn nested_tag_route_reaches_the_tag_page() {
    let (_tmp, root) = setup();
    // Unknown tag: the page renders (empty), it does not 404.
    let (status, _, body) = get(app(&root), "/tag/other/deep").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("#other/deep"), "{body}");
}

#[tokio::test]
async fn search_finds_notes() {
    let (_tmp, root) = setup();
    let (status, _, body) = get(app(&root), "/search?q=glassware").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("href=\"/n/Notes/Nested.md\""), "{body}");
    assert!(body.contains("glassware"), "{body}");
}

#[tokio::test]
async fn files_serve_bytes_with_mime() {
    let (_tmp, root) = setup();
    let (status, headers, body) = get(app(&root), "/files/attachments/img.png").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    // 0x89 is not valid UTF-8, so the lossy body keeps only the ASCII tail.
    assert!(body.contains("PNG-fake"), "{body}");
}

#[tokio::test]
async fn files_missing_404s() {
    let (_tmp, root) = setup();
    let (status, _, _) = get(app(&root), "/files/attachments/none.png").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- path traversal (the M5 acceptance core) --------------------------------

#[tokio::test]
async fn traversal_paths_are_refused() {
    let (_tmp, root) = setup();
    let app = app(&root);
    for uri in [
        // Plain dot-dot.
        "/n/../secret.md",
        "/files/../secret.md",
        // Percent-encoded dot-dot (axum decodes before the handler).
        "/n/%2e%2e/secret.md",
        "/files/%2e%2e/secret.md",
        // Encoded slash traversal.
        "/n/..%2Fsecret.md",
        "/n/Notes/..%2F..%2Fsecret.md",
        "/files/..%2F..%2Fsecret.md",
        // Dot-dot that would land back inside is still refused.
        "/n/Notes/../Note.md",
        // Double-encoded.
        "/n/%252e%252e/secret.md",
    ] {
        let (status, _, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
}

#[tokio::test]
async fn dotfile_paths_are_refused() {
    let (_tmp, root) = setup();
    let app = app(&root);
    for uri in [
        "/n/.obsidian/app.json",
        "/files/.obsidian/app.json",
        "/n/.hidden.md",
        "/files/.hidden.md",
        "/n/Notes/.obsidian/config",
    ] {
        let (status, _, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
}

#[tokio::test]
#[cfg(unix)]
async fn symlinks_out_of_the_vault_are_refused() {
    let (_tmp, root) = setup();
    let app = app(&root);
    for uri in ["/n/leak.md", "/files/leak.md", "/n/mirror/note.md"] {
        let (status, _, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
    // A symlink that stays inside the vault keeps working.
    let (status, _, body) = get(app, "/n/alias.md").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// --- auth -------------------------------------------------------------------

#[tokio::test]
async fn auth_required_when_token_configured() {
    let (_tmp, root) = setup();
    let app = token_app(&root);
    for uri in ["/", "/n/Note.md", "/files/attachments/img.png"] {
        let (status, _, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
    }
}

#[tokio::test]
async fn token_accepted_via_query_and_bearer_header() {
    let (_tmp, root) = setup();
    let app = token_app(&root);

    let (status, _, body) = get(app.clone(), "/?token=tok123").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::AUTHORIZATION, "Bearer tok123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, _, _) = get(app, "/?token=wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_disabled_when_configured_so() {
    let (_tmp, root) = setup();
    let (status, _, _) = get(app_with(&root, None), "/n/Note.md").await;
    assert_eq!(status, StatusCode::OK);
}

// --- security headers -------------------------------------------------------

#[tokio::test]
async fn security_headers_on_every_response() {
    let (_tmp, root) = setup();
    let app = token_app(&root);

    let (status, headers, _) = get(app.clone(), "/?token=tok123").await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    // Refusals carry the headers too.
    let (status, headers, _) = get(app, "/n/Note.md").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}

// --- sanitization -----------------------------------------------------------

#[tokio::test]
async fn raw_html_in_notes_is_sanitized() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("vault");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("XSS.md"),
        "<script>alert(1)</script>\n\n<img src=x onerror=alert(2)>\n",
    )
    .unwrap();
    let (status, _, body) = get(app(&root), "/n/XSS.md").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.contains("<script>"), "{body}");
    assert!(!body.contains("onerror"), "{body}");
}
