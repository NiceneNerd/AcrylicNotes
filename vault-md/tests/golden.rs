//! Golden tests: input .md -> expected .html for every Obsidian feature in
//! HANDOFF section 6, plus the sentinel cases from 4.1.
//!
//! Expected files are generated with `UPDATE_EXPECT=1 cargo test` and then
//! REVIEWED BY EYE before committing: a golden file is a spec, not an
//! output dump. Never regenerate blindly.

use std::collections::HashMap;
use std::path::Path;

use vault_core::{NoteSource, VaultIndex};
use vault_md::{parse_note, render_html, RenderContext, RenderOptions};

struct FixtureNotes(HashMap<String, String>);

impl NoteSource for FixtureNotes {
    fn note_source(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
}

fn fixture_vault() -> (VaultIndex, FixtureNotes) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/vault");
    let mut files = Vec::new();
    let mut notes = HashMap::new();
    for entry in std::fs::read_dir(&dir).expect("fixture vault dir") {
        let path = entry.unwrap().path();
        if path.is_file() {
            let rel = path.file_name().unwrap().to_string_lossy().to_string();
            files.push(rel.clone());
            // Only notes have readable source; attachments are index-only.
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                notes.insert(rel, std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    files.push("fixture.md".to_string());
    (VaultIndex::new(files), FixtureNotes(notes))
}

#[test]
fn golden_fixtures_match() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let (index, notes) = fixture_vault();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    names.sort();

    let mut checked = 0;
    for path in &names {
        let src = std::fs::read_to_string(path).unwrap();
        let doc = parse_note(&src);
        let ctx = RenderContext::new(&index, "fixture.md", &notes, RenderOptions::default());
        let actual = render_html(&doc, &ctx);
        let expected_path = path.with_extension("html");
        if std::env::var("UPDATE_EXPECT").is_ok() {
            std::fs::write(&expected_path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("missing expected file {}", expected_path.display()));
        assert_eq!(actual, expected, "golden mismatch for {}", path.display());
        checked += 1;
    }
    assert!(
        checked >= 15,
        "expected the full fixture set, found {checked}"
    );
}

#[test]
fn strict_line_breaks_render_soft_breaks_as_newlines() {
    let (index, notes) = fixture_vault();
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/14-linebreaks.md"),
    )
    .unwrap();
    let doc = parse_note(&src);
    let mut options = RenderOptions::default();
    options.strict_line_breaks = true; // the real vault's .obsidian/app.json
    let ctx = RenderContext::new(&index, "fixture.md", &notes, options);
    let html = render_html(&doc, &ctx);
    assert!(
        !html.contains("<br>"),
        "strict mode must not emit <br>: {html}"
    );
    assert!(html.contains("first line\nsecond line"));
}
