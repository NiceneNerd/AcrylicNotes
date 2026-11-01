# HANDOFF: lightweight Rust Obsidian-compatible notes app

Written 2026-10-31. Audience: a local coding agent continuing this work.
Read this whole file before touching code.

## 0. Kickoff prompt (paste to the agent)

> Read HANDOFF.md fully. Work through the milestones in section 5 in order,
> one at a time. Verify every claim with a test or a command; if you cannot
> verify something, say so explicitly instead of estimating. After each
> milestone, report: what was built, which tests prove it, what remains
> unverified. Do not start a milestone until the previous one's acceptance
> criteria pass. Never modify files inside a test vault except through the
> app's own save path under test.

## 1. Goal and requirements (from the user)

A lightweight replacement for Obsidian:

- Compatible with Obsidian's Markdown flavor and rendering, especially
  footnotes and wikilinks, plus YAML frontmatter.
- Built in Rust. Ideally libcosmic; Freya or plain Iced are acceptable fallbacks.
- Glass: COSMIC frosted glass, Windows Acrylic/Mica, macOS glass.
- Opens and uses an existing Obsidian vault. Primarily plain-text notes and
  journals, links, tags. Explicitly NOT needed: graph view and other fancy
  features. No plugin system.
- Bonus: lightweight view-only web server for the vault.

## 2. Architecture decisions already made

Three layers; only the last depends on a GUI toolkit.

1. `vault-core` (UI-free): vault scan, file watching, in-memory index, link
   resolution, frontmatter handling, safe file writes.
2. `vault-md`: comrak parse plus an Obsidian-extension pass, lowered into an
   own IR (`ir.rs`). The IR is the contract between frontends: an IR-to-HTML
   renderer (web) and an IR-to-widgets renderer (GUI).
3. Frontends: `app` (libcosmic GUI), `serve` (axum, server-rendered HTML,
   no JS). Glass lives in a small `vault-glass` crate behind a trait.

Target workspace layout (the current code is a single crate, `acrylic-notes`):

```
vault-core/   scan, watch, index, resolve, frontmatter, atomic writes, CLI
vault-md/     ir.rs, inline.rs (extract/lower), ast lowering, html renderer
vault-glass/  trait Backdrop + per-platform impls
app/          libcosmic GUI
serve/        axum read-only server (separate bin or --serve flag)
```

Key principles (do not violate):

- Files are the source of truth. Never re-serialize what the user did not
  edit. Keep frontmatter bytes verbatim; parse a COPY for metadata. Preserve
  line endings (CRLF vs LF). Write atomically (temp file in same dir, then
  rename). Detect external modification before save (mtime/size/hash), since
  real Obsidian, Syncthing, or git may touch the same vault.
- Never write into `.obsidian/`. Read it selectively (attachment folder,
  link format, daily-notes settings). Keep this app's UI state in the OS
  config dir.
- Index in memory, rebuilt at startup, kept fresh by a watcher (`notify`).
  No database in v1.
- Avoid a webview (wry) for rendering: it defeats "lightweight" and
  complicates blur compositing.

## 3. What exists in this zip

`acrylic-notes/` (std + comrak 0.29.0 only; 10 unit tests pass):

- `src/ir.rs`: `Doc`, `Block`, `Inline`, `WikiLink`, `WikiTarget`, `Embed`.
- `src/inline.rs`: `extract()` (pre-comrak pass: wikilinks, embeds, drops
  `%%comments%%`, skips fenced code and inline code, emits sentinels) and
  `lower_text()` (sentinels back to IR, plus `#tag` recognition).
- `src/resolve.rs`: `VaultIndex::{resolve, candidates}`.
- `examples/ast_spike.rs`, `ast_spike2.rs`: how comrak splits text nodes.
- `examples/e2e.rs`: extract, then comrak, then lower.
- `examples/gaps.rs`: demonstrates the known gaps in section 4.3.

`feature-split-test/`: stand-in crate proving per-target feature selection
(see 4.1). Run `cargo tree --target <triple> -e features` inside `app/`.

Commands: `cd acrylic-notes && cargo test && cargo run --example e2e`.
NOTE: it was built with a very old cargo (1.75), so comrak is pinned
`=0.29.0`. On a current toolchain, remove the pin, upgrade comrak, and
re-run everything (see M2).

## 4. Findings

### 4.1 Verified by running code

- comrak 0.29 (wikilinks extension on): `[[Note|alias]]` and
  `[[Note#Heading]]` work, but `[[Note#Heading]]` label is the raw
  `Note#Heading` (Obsidian shows "Note > Heading"); `#^id` is
  percent-encoded. Frontmatter is stripped from output. Footnotes work.
  Not handled at all: `![[embeds]]`, `#tags`, `> [!note]` callouts,
  `==highlight==`, `%%comments%%`, `^block-id`, `^[inline footnote]`.
- comrak splits inline nodes at emphasis: `[[a *b* c]]` and
  `[[Note|*alias*]]` are split across nodes; `[[Note]](paren)` parses as a
  normal link. Hence the pre-comrak `extract()` design, with comrak's own
  wikilinks extension turned OFF.
- Per-target feature selection works: with resolver 2, one dependency
  declared in three tables (default / `cfg(target_os="linux")` /
  `cfg(not(target_os="linux"))`) selects `wayland` on Linux and `winit`
  elsewhere, with no conflict. Verified with a stand-in crate only; NOT with
  real libcosmic.

### 4.2 From documentation/news (fast-moving; re-verify at build time)

- COSMIC 1.3 ships frosted glass, implemented in cosmic-comp and libcosmic
  via the standard Wayland `ext-background-effect-v1` protocol (not iced
  itself). Third-party apps must update libcosmic to get it.
  https://linuxiac.com/cosmic-desktop-1-3-shines-with-its-new-frosted-glass-design/
  https://linuxiac.com/cosmic-desktop-big-visual-upgrade-moves-closer-to-release/
- The same protocol is supported by KWin (Plasma 6.7) and Mutter (GNOME 51),
  so one Linux code path should cover all three.
  https://www.phoronix.com/news/GNOME-Mutter-Background-Blur
- libcosmic features `wayland` and `winit` conflict; libcosmic recently began
  bundling icons for macOS as well as Windows, but it is COSMIC-first, so
  expect rough edges off Linux. https://pop-os.github.io/libcosmic/cosmic/
  CORRECTED 2026-10-31 by reading the Cargo.toml at the pinned commit: the
  `wayland` feature IMPLIES `winit` (plus `a11y` and `multi-window`), so the
  two never conflict. Per-target feature tables still work, but on Linux they
  are effectively cosmetic — everything the `wayland` feature needs comes in
  through the implied `winit` anyway.
- `window-vibrancy` (Tauri team) takes any raw-window-handle: Windows
  `apply_mica` (Win11 only), `apply_acrylic` (documented perf problems when
  resizing/dragging on some builds), `apply_blur`; macOS `apply_vibrancy`
  and a liquid-glass API (macOS 26+). https://github.com/tauri-apps/window-vibrancy
- comrak (newer versions) documents options for inline footnotes, alerts,
  math, highlight, and wikilinks in both pipe orders:
  https://docs.rs/comrak/latest/comrak/options/struct.Extension.html
- Freya: only 2023-24 pages were found, describing it as WIP. Current state
  UNVERIFIED. Check before considering it.

### 4.3 Known gaps in the current code (confirmed by `examples/gaps.rs`)

RESOLVED in M2/M3 — the list below is kept for reference; every item was
closed and is now covered by a test. `cargo run -p vault-md --example gaps`
prints the five cases and asserts that none of them extracts a token.

`extract()` used to turn a wikilink into a token in each of these cases:

- Indented (4-space) code blocks. Definitely wrong; must skip.
  → FIXED (`indented_code_mask`).
- Frontmatter. Definitely wrong: split frontmatter FIRST and run `extract`
  on the body only. (Obsidian properties often hold quoted `"[[Note]]"`.)
  → FIXED (`extract_document`; the split is lossless).
- Escaped `\[[x]]`, inline math `$a[[b]]c$`, raw HTML blocks. Check what
  real Obsidian does for each, then match it.
  → CHECKED against real Obsidian (2026-10-31): none of the three is a
  link. All three now extract zero tokens.

Other open items: `parse_target` stores nested `#H1#H2` raw in `heading`;
tag rules were taken from Obsidian's docs (letters, digits, `_ - /`, at
least one non-digit) and boundary behavior (`foo#bar`) is assumed; the
resolver tie-break (nearest folder, then shallowest) is a GUESS; aliases are
assumed NOT to resolve links (autocomplete only); macOS filename Unicode
normalization (NFD/NFC) and Windows case-insensitivity are not handled yet;
non-`.md` link targets (pdf, canvas, images with spaces) are only lightly
covered.

→ RESOLVED/STATUS: the tie-break is CONFIRMED (nearest, then shallowest) and
implemented — the corpus caught that an "exact path" bonus wrongly let a
root file beat the nearest duplicate. Aliases CONFIRMED not to resolve
links. Tag boundaries: `(#tag)` confirmed NOT a tag, `foo#bar` assumed.
Nested `#H1#H2` headings and macOS NFD storage remain open (NFD matching
itself is implemented and unit-tested; only real-macOS storage is
untested). Non-`.md` targets are now covered by the corpus (attachments
with spaces, wiki and percent-encoded Markdown links).

## 5. Milestones (do in order)

### M0. Glass spike (highest risk; decides libcosmic vs plain Iced)

Build a minimal window per OS with a translucent background and real
backdrop blur.

- Linux: use libcosmic with `wayland`. READ libcosmic's source/examples for
  how an app opts into frosted glass; do not hand-roll the protocol unless
  required. Needs COSMIC >= 1.3 (or KDE 6.7 / GNOME 51) to see the effect.
- Windows: `window-vibrancy` Mica (Win11) with Acrylic/blur fallback.
- macOS: vibrancy, plus liquid glass when available.
- Define `trait Backdrop { fn apply(&self, win, GlassStyle) -> Result<()> }`
  with a capability probe and graceful opaque fallback (X11, old OS).
- Design rule: glass on chrome (sidebar, tabs); keep the note text surface
  noticeably more opaque for legibility.

Acceptance: one binary source tree builds on Linux (`wayland`) and
Windows/macOS (`winit`) using the Cargo target-table pattern from 4.1;
screenshots or notes per OS; any OS you could not test is listed explicitly
as untested (CI or a VM is fine). Record whether libcosmic is viable off
Linux. If NOT, fall back to plain iced + winit and keep the rest of the plan.

### M1. Workspace restructure and `vault-core`

- Convert to the workspace layout in section 2; move `resolve` to
  `vault-core`, `ir`/`inline` to `vault-md`. Tests stay green.
- `split_frontmatter(src) -> (Option<raw>, body)`: starts at byte 0 with
  `---`, closes with `---` or `...` on its own line; handle BOM and CRLF.
  Parse a copy with a maintained YAML crate (serde_yaml is archived; verify
  the crate you pick builds and round-trips). Extract `tags` (list or string
  forms), `aliases`, `cssclasses`.
- Scan: ignore dot-directories (`.obsidian`, `.git`, `.trash`); `.md` are
  notes, everything else is an attachment for resolution. Normalize for
  matching: case-fold, Unicode NFC, internal paths use `/`.
- Index: per note title, aliases, tags (frontmatter and inline), headings,
  block ids, outgoing links; derive backlinks and tag lists. Watcher with
  debounce that ignores the app's own writes.
- Safe write path (see section 2) with unit tests, including a test that an
  unedited frontmatter block is byte-identical after a body edit.
- CLI: `vault index <dir>`, `vault resolve <dir> <from> <target>`,
  `vault backlinks <dir> <note>`.

Acceptance: CLI works on a real vault (user will supply one; until then
generate a synthetic vault in tests); fix all section 4.3 definite gaps.

### M2. Markdown lowering and HTML renderer

- Upgrade comrak to the current release on a current Rust toolchain. Re-run
  every test and example. Enable footnotes, inline footnotes, tables,
  tasklists, strikethrough, math, highlight, and (if suitable) alerts.
  Keep comrak's wikilinks extension OFF.
- Lower comrak AST to `ir::Doc`: sentinel substitution in text nodes, also
  inside headings, link text, table cells, image alt; callouts from
  blockquote first line `[!kind]`, `[!kind]+`, `[!kind]-` with title;
  `^block-id` at paragraph end; block-level `![[Note]]` to `EmbedBlock`;
  tasks; footnote definitions.
- IR to HTML: wikilinks resolve through `VaultIndex` (unresolved links get a
  distinct class); `[[Note#Heading]]` displays "Note > Heading"; embeds of
  notes/sections/blocks are transcluded with a depth limit and cycle
  detection; image embeds honor size. Output must be safe: sanitize raw HTML
  (e.g. `ammonia`) when the output may leave localhost.

Acceptance: golden tests (input .md to expected .html) for every Obsidian
feature in section 6; sentinel handling proven for the cases in 4.1.

### M3. Compatibility corpus

Build `tests/vault/` containing duplicate basenames, nested folders,
attachments with spaces, aliases, nested tags, callouts of several kinds
(folded and unfolded), block refs, comments, embeds with sizes, CRLF files,
non-ASCII names. For each ambiguous behavior in 4.3, ask the user to confirm
against real Obsidian (or document it as an assumption in `COMPAT.md`).
Update the resolver tie-break and alias behavior accordingly.

### M4. GUI (libcosmic, or iced if M0 says so)

Elm-style `Message/update/view`; all logic calls `vault-core`/`vault-md`.

1. v1 shell: file tree, tabs, source-mode editor (iced `text_editor` with
   Markdown highlighting), reading-view pane rendered from the IR (compose
   rich-text spans/widgets; as far as known, iced's built-in markdown widget
   lacks wikilinks/footnotes/callouts, verify against the iced fork
   libcosmic pins). Clicking a link navigates; unresolved link offers
   "create note".
2. Navigation: quick switcher (fuzzy, e.g. `nucleo`), backlinks panel,
   outline, tag list, vault-wide search.
3. Journaling: daily note creation honoring `.obsidian/daily-notes.json`
   (date format, folder, template) if present.
4. Glass applied through `vault-glass` to chrome only.
5. Autosave via the safe write path; external-change reload prompt.

Do NOT build Obsidian-style live preview here; it needs a custom editor
widget and is a separate later project (M6).

### M5. Read-only web server

axum, same core and IR, server-rendered HTML, no JS. Routes: `/n/*path`,
`/tag/:t`, `/search?q=`, `/files/*` (attachments). Security: canonicalize and
confine every path to the vault root; deny dotfiles and `.obsidian`; bind
127.0.0.1 by default; optional token auth; sanitize HTML; add tests for path
traversal (`..`, encoded `..`, symlinks out of the vault).

### M6. Optional: live preview

Custom editor widget that renders Markdown inline when the cursor is not on
a line. Only after M0-M5 are solid.

## 6. Obsidian features to cover (checklist for tests)

Wikilinks (`[[n]]`, `[[n|alias]]`, `[[n#h]]`, `[[n#^id]]`, `[[#h]]`),
embeds (`![[note]]`, `![[note#h]]`, `![[img.png|300]]`, `|300x200`),
standard footnotes and inline `^[...]`, YAML frontmatter, inline `#tags`
(nested `a/b`, not all-numeric, not mid-word), frontmatter `tags`,
callouts (any type, `+`/`-` folding, custom title), `==highlight==`,
`%%comments%%` (can span blocks), `^block-id`, `$math$`/`$$math$$`, tasks,
tables (note `\|` inside wikilinks in tables), code fences, standard
CommonMark/GFM.

## 7. Working rules for the agent

- Test-first and evidence-first: every behavior claim needs a test or a
  command output. Label anything else as an assumption in `COMPAT.md`.
- Keep `vault-core` and `vault-md` free of GUI dependencies.
- Run `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` before
  reporting a milestone done.
- Library APIs here move fast (libcosmic, iced fork, comrak). Re-check
  signatures against the version you actually pin; do not rely on the
  snippets or version numbers in this file.
- When blocked on something only the user can check (real Obsidian behavior,
  a Windows/macOS machine), stop and list the exact question.
