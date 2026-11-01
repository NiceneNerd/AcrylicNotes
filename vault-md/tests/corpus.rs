//! M3 compatibility corpus (HANDOFF M3): a miniature vault exercising every
//! structural edge case — duplicate basenames, nested folders, attachments
//! with spaces, aliases, nested tags, callouts folded and unfolded, block
//! refs, comments, embeds with sizes, CRLF files, and non-ASCII names.
//!
//! The corpus is COPIED to a temp dir for every run: the render tests only
//! read, but the save test writes through the app's save path, and the
//! committed corpus must stay pristine.

use std::fs;
use std::path::{Path, PathBuf};

use vault_core::{NoteSource, Vault};
use vault_md::{parse_note, render_html, RenderContext, RenderOptions};

fn corpus_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vault")
}

/// Copy the corpus tree (files and folders, any depth) into a temp dir.
fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else if let Err(e) = fs::copy(&from, &to) {
            // Wine (the Windows test runner) rejects filenames containing
            // combining characters with ERROR_INVALID_NAME; real NTFS
            // accepts them, but we have no real Windows to prove it. Skip
            // such fixtures there — the NFD-matching unit test in vault-core
            // covers normalization on every platform in memory.
            if cfg!(windows) && e.kind() == std::io::ErrorKind::InvalidFilename {
                continue;
            }
            panic!("copy {} failed: {e}", from.display());
        }
    }
}

fn open_corpus() -> (tempfile::TempDir, Vault) {
    let tmp = tempfile::TempDir::new().unwrap();
    copy_tree(&corpus_source(), tmp.path());
    let vault = Vault::open(tmp.path()).expect("open corpus vault");
    (tmp, vault)
}

fn render(vault: &Vault, path: &str) -> String {
    let source = vault
        .note_source(path)
        .unwrap_or_else(|| panic!("{path} missing"));
    let doc = parse_note(&source);
    let index = vault.index();
    let ctx = RenderContext::new(&index, path, vault, RenderOptions::default());
    render_html(&doc, &ctx)
}

#[test]
fn duplicate_basenames_resolve_nearest_then_shallowest() {
    let (_tmp, vault) = open_corpus();
    let index = vault.index();
    // Confirmed (M3): nearest folder wins; a root file with the same name
    // does not beat the folder the linking note lives in.
    assert_eq!(
        index.resolve("Glass", "Notes/Source.md"),
        Some("Notes/Glass.md")
    );
    assert_eq!(
        index.resolve("Glass", "Archive/inner.md"),
        Some("Archive/Glass.md")
    );
    assert_eq!(index.resolve("Glass", "Glass.md"), Some("Glass.md"));
    // Far away: shallowest wins (all three are equally far).
    assert_eq!(
        index.resolve("Glass", "Callouts.md"),
        Some("Glass.md"),
        "shallowest path wins from an unrelated folder"
    );
    // Path-qualified targets are unambiguous.
    assert_eq!(
        index.resolve("Archive/Glass", "Notes/Source.md"),
        Some("Archive/Glass.md")
    );
    // All three are candidates for the bare name.
    assert_eq!(index.candidates("Glass", "Callouts.md").len(), 3);
}

#[test]
fn nested_folders_and_attachments_with_spaces_resolve() {
    let (_tmp, vault) = open_corpus();
    let index = vault.index();

    // Wiki links resolve vault-wide by (suffix) path.
    assert_eq!(
        index.resolve("attachments with spaces/my image.png", "Notes/Source.md"),
        Some("attachments with spaces/my image.png")
    );

    // Markdown links resolve relative to the note's folder, percent-decoded.
    let source = vault.get_note("Notes/Source.md").unwrap();
    let md_links: Vec<&str> = source
        .outgoing_links
        .iter()
        .filter(|l| matches!(l.kind, vault_core::LinkKind::Markdown))
        .map(|l| l.target.as_str())
        .collect();
    assert_eq!(
        md_links,
        ["../attachments with spaces/my image.png"],
        "percent-escapes must be decoded in the index"
    );
    assert_eq!(
        index.resolve_relative(md_links[0], "Notes/Source.md"),
        Some("attachments with spaces/my image.png")
    );
    // The attachment is indexed even though it is not a note.
    assert!(vault
        .attachments_sorted()
        .iter()
        .any(|a| a.ends_with("my image.png")));
}

#[test]
fn aliases_do_not_resolve_links() {
    let (_tmp, vault) = open_corpus();
    // The alias is on the note.
    assert_eq!(
        vault.get_note("Alias Note.md").unwrap().meta.aliases,
        ["MyAlias"]
    );
    // Confirmed (M3): aliases never resolve links.
    let index = vault.index();
    assert_eq!(index.resolve("MyAlias", "Notes/Source.md"), None);
    // And the rendered link is marked unresolved.
    let html = render(&vault, "Notes/Source.md");
    assert!(
        html.contains("internal-link is-unresolved\" data-href=\"MyAlias\""),
        "{html}"
    );
}

#[test]
fn nested_tags_and_confirmed_tag_rules() {
    let (_tmp, vault) = open_corpus();
    let tags = vault.all_tags();

    for expected in [
        "frontmatter",      // frontmatter tags
        "nested/fm",        // frontmatter nested
        "headingtag",       // confirmed: tags in headings ARE indexed
        "root/nested/leaf", // nested inline tags
        "also/leaf",
        "tag",       // trailing slash trimmed
        "incomment", // confirmed: tags inside %%comments%% ARE indexed
    ] {
        assert!(
            tags.iter().any(|t| t == expected),
            "missing {expected} in {tags:?}"
        );
    }
    for rejected in ["2024", "mid", "paren", "notag"] {
        assert!(
            !tags.iter().any(|t| t == rejected),
            "{rejected} must not be a tag"
        );
    }
    assert_eq!(vault.notes_for_tag("root/nested/leaf"), ["tags.md"]);
    // Escaped hashes are not tags; a hash after a paren is not a tag.
    assert_eq!(vault.notes_for_tag("paren"), Vec::<String>::new());
}

#[test]
fn comments_are_dropped_from_renders_but_indexed() {
    let (_tmp, vault) = open_corpus();
    let html = render(&vault, "Comments.md");
    // Reading view hides the whole comment, links and tags inside it.
    assert!(!html.contains("hidden"), "{html}");
    assert!(!html.contains("NotALinkTarget"), "{html}");
    assert!(!html.contains("incomment"), "{html}");
    assert!(html.contains("After the comment"), "{html}");
    // Confirmed (M3): the tag pane still counts tags inside comments.
    assert_eq!(vault.notes_for_tag("incomment"), ["Comments.md"]);
}

#[test]
fn callouts_folded_and_unfolded() {
    let (_tmp, vault) = open_corpus();
    let html = render(&vault, "Callouts.md");
    assert!(html.contains("data-callout=\"note\""), "{html}");
    assert!(html.contains("Plain callout"), "{html}");
    // `+` = foldable and initially open.
    assert!(
        html.contains("data-callout=\"tip\" data-callout-foldable data-callout-open"),
        "{html}"
    );
    // `-` = foldable and initially closed (no open marker).
    assert!(
        html.contains("data-callout=\"warning\" data-callout-foldable>"),
        "{html}"
    );
}

#[test]
fn block_refs_index_and_transclude() {
    let (_tmp, vault) = open_corpus();
    let blocks = vault.get_note("Blocks.md").unwrap();
    // The lone `^lone` line attaches to the paragraph above it.
    assert_eq!(
        blocks.block_ids,
        ["para1", "item1", "lone"],
        "paragraph id, list-item id, attached lone id"
    );

    // Confirmed (M3): the lone id line is invisible in reading view.
    let html = render(&vault, "Blocks.md");
    assert!(!html.contains("^lone"), "{html}");
    assert!(!html.contains("^para1"), "{html}");
    assert!(html.contains("Paragraph with an id"), "{html}");

    // Block transclusion pulls exactly the referenced block.
    let index = vault.index();
    let source = vault.note_source("Embeds.md").unwrap();
    let doc = parse_note(&source);
    let ctx = RenderContext::new(&index, "Embeds.md", &vault, RenderOptions::default());
    let html = render_html(&doc, &ctx);
    assert!(html.contains("Paragraph with an id"), "{html}");
    assert!(
        !html.contains("item with id"),
        "block embed must not pull the whole note: {html}"
    );
}

#[test]
fn embeds_honor_sizes_and_transclude_notes() {
    let (_tmp, vault) = open_corpus();
    let html = render(&vault, "Embeds.md");
    // Image embed with a size, from a folder whose name has spaces.
    assert!(
        html.contains(
            "<p><img class=\"internal-embed\" src=\"attachments%20with%20spaces/my%20image.png\" \
             alt=\"attachments with spaces/my image.png\" width=\"300\"></p>"
        ),
        "{html}"
    );
    // Note transclusion renders the embedded note's content.
    assert!(html.contains("Glass in Notes"), "{html}");
    // The block id paragraph transcluded from Blocks.md.
    assert!(html.contains("Paragraph with an id"), "{html}");
}

#[test]
fn crlf_note_saves_losslessly_through_the_app() {
    let (_tmp, mut vault) = open_corpus();
    let before = vault.note_source("CRLF.md").unwrap();
    assert!(before.contains("\r\n"), "fixture must be CRLF");
    // It indexes and resolves like any note.
    assert!(vault.get_note("CRLF.md").unwrap().tags.is_empty());
    assert!(vault.backlinks("Glass.md").contains(&"CRLF.md".to_string()));

    // Edit the body through the app's save path.
    let split = vault_core::split_frontmatter(&before);
    let mut body = split.body.clone();
    body.push_str("Added CRLF line.\r\n");
    vault.save_note("CRLF.md", &before, &body).expect("save");

    let after = fs::read_to_string(vault.root().join("CRLF.md")).unwrap();
    assert_eq!(
        after,
        format!("{}{}", split.raw.as_deref().unwrap_or(""), body)
    );
    assert_eq!(after.matches("\r\n").count(), after.matches('\n').count());
    assert_eq!(
        vault
            .get_note("CRLF.md")
            .unwrap()
            .frontmatter_raw
            .as_deref(),
        split.raw.as_deref(),
        "frontmatter bytes verbatim"
    );
}

#[test]
// The NFD fixture cannot exist under Wine (ERROR_INVALID_NAME), so this runs
// on Unix only; NFC matching itself is covered on every platform by
// vault-core's in-memory unit test.
#[cfg_attr(windows, ignore)]
fn non_ascii_names_resolve_across_normalizations() {
    let (_tmp, vault) = open_corpus();
    // The fixture filename is stored NFD on disk; queries arrive NFC.
    let nfd_path = vault
        .all_notes()
        .map(|n| n.path.as_str())
        .find(|p| p.contains("Cafe"))
        .expect("non-ASCII note indexed")
        .to_string();
    assert_ne!(nfd_path, "Café.md", "fixture must be stored NFD");

    let index = vault.index();
    assert_eq!(
        index.resolve("Café", "Notes/Source.md"),
        Some(nfd_path.as_str())
    );
    // The rendered link keeps the NFC label.
    let html = render(&vault, "Notes/Source.md");
    assert!(html.contains(">Café</a>"), "{html}");
}

#[test]
fn backlinks_follow_resolved_targets() {
    let (_tmp, vault) = open_corpus();
    // [[Glass]] from Notes/Source.md resolves to Notes/Glass.md.
    assert!(vault
        .backlinks("Notes/Glass.md")
        .contains(&"Notes/Source.md".to_string()));
    // Embeds count as links too.
    assert!(vault
        .backlinks("Notes/Glass.md")
        .contains(&"Embeds.md".to_string()));
    // The unresolved alias link produces no backlink for Alias Note.md.
    assert!(vault.backlinks("Alias Note.md").is_empty());
}
