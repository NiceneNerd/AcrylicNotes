# COMPAT — Obsidian compatibility notes

Every claim here is either **confirmed** (observed against real Obsidian or
the user's vault) or an **ASSUMPTION** (implemented on a documented guess).
Anything not listed here is either verified by a test or still unimplemented.

## Confirmed

Behaviors the user verified against real Obsidian (2026-10-31), and that the
code now implements:

1. **Footnote numbering follows reference order.** `[^b]` referenced before
   `[^a]` is 1 and 2 — not the definition order. Definitions that are never
   referenced take the numbers after the referenced ones, in definition
   order. (`vault-md/src/html.rs`: `collect_footnote_refs` →
   `assign_unused_def_numbers`.)
2. **Tags inside `%%comments%%` are indexed** (they show up in Obsidian's tag
   pane). The indexer therefore counts them; the renderer still drops the
   whole comment, so no tag is rendered — exactly like Obsidian's reading
   view.
3. **Ambiguous basenames resolve nearest-folder-first, then shallowest.**
   `VaultIndex::candidates` sorts by (exact path, shared folder depth,
   depth, name).
4. **Aliases do NOT resolve links.** `aliases:` only feeds autocomplete and
   unlinked mentions; `[[MyAlias]]` stays unresolved.
5. **Non-`.md` targets need their extension.** `[[cat]]` does not resolve to
   `cat.png` — only `.md` is stripped from link targets.
6. **An escaped wikilink is not a link.** `\[[not a link]]` renders as
   literal text.
7. **No links inside math.** `$a[[b]]c$` and `$$…[[b]]…$$` contain no
   links.
8. **Raw HTML blocks are literal.** `<div>[[x]]</div>` renders as HTML with
   literal `[[x]]`, not a link.
9. **`[[Note#Heading]]` reads "Note > Heading"** in the reading view.
10. **A lone `^id` line is invisible.** It renders nothing; in our
    implementation it attaches its id to the preceding block (paragraph, or
    the last item of a preceding list). The attachment is our inference from
    Obsidian's block-reference semantics; the invisibility is confirmed.
11. **Block references show only the note name.** `[[Note#^bid]]` renders as
    "Note" (what Obsidian's autocomplete inserts), not "Note > ^bid".
12. **A `#` directly after `(` or `)` does not start a tag.** `(#tag)` is not
    a tag. (Other punctuation — whitespace, `,`, `]` — still starts one;
    that part remains an assumption, see below.)
13. **Tags in headings are indexed.** `## Section #tag` puts `#tag` in the
    tag pane.

Previously confirmed (M2):

- **Stale relative links stay unresolved.** `[[../Prayer and Such/Discipuli]]`
  from `Journal/2026/04/15.md` (wrong `../` depth; target exists at vault
  root) shows as unresolved in real Obsidian. Therefore relative link
  targets resolve strictly against the note's folder with NO basename
  fallback. `VaultIndex::resolve_relative` implements this.
- **`](#anchor)` destinations are not tags.** The 114 `](#footnote-N)` /
  `](#fmN)` markdown-link destinations in the vault do not appear in the
  Obsidian tag pane. Link destinations and HTML attribute values are blanked
  before tag scanning; `vault index` lists exactly 4 tags for the vault
  (Immortality, Honor, kobo — all real; `import` is a Typst `#import` in
  prose that Obsidian's own rules also index).
- **`strictLineBreaks: true`** is set in this vault's `.obsidian/app.json`
  (read at render time; `RenderOptions::strict_line_breaks`).

## ASSUMPTIONS (still unverified)

1. **Callout DOM.** comrak 0.59 parses `> [!kind]`, `|metadata` inside the
   brackets, `+`/`-` fold markers, and default titles (capitalized type).
   We emit `data-callout="kind"` / `data-callout-foldable` /
   `data-callout-open`, mirroring Obsidian's documented markup, but the live
   DOM was NOT diffed (deferred by the user on 2026-10-31).
2. **Block-ID character set and conflicts.** ids are alphanumeric plus
   `-`/`_`; a trailing `^id` on a paragraph or list item becomes that
   block's id. When a block already has an id and a lone `^id` line follows,
   the FIRST id wins. A `^id` after a heading has no block to attach to and
   stays invisible.
3. **Tag boundaries beyond parens.** A tag starts after whitespace or
   punctuation, never mid-word; a hyphen counts as a word character, so
   `word-#tag` is not a tag. Only the paren case is confirmed.
4. **Heading anchors.** `[[Note#Heading]]` emits
   `href="Note.md#<Heading text>"` — the anchor scheme is ours; the M5
   server must generate matching heading ids (slug = raw heading text).
   Path-qualified links (`[[Folder/Note]]`) display the path as written.
5. **Math vs currency.** comrak's `math_dollars` rules keep `$5 and $10` as
   text (closing `$` followed by a digit is not math). Golden-tested; real
   vault currency notes not yet swept for false math.
6. **Sanitization is stricter than Obsidian.** Raw HTML is ammonia-cleaned
   (fragments only): `class`/`id`/`title` survive, `style` does NOT, scripts
   and event handlers never do. Obsidian allows `style`; revisit if vault
   CSS needs it.
7. **Embed depth limit = 3** with cycle detection (Obsidian has a similar
   guard with an unknown limit — deferred by the user on 2026-10-31).
8. **Indented code vs list items.** The pre-pass treats a 4-space-indented
   line after a blank line as code even inside a list item; Obsidian's
   behavior there is unverified (HANDOFF 4.3).
9. **Section transclusion.** `![[Note#Heading]]` takes blocks from the
   matching heading (case-insensitive text match) to the next heading of
   the same or higher level; `![[Note#^id]]` takes exactly that block.
   Heading-matching rules in Obsidian are unverified.
10. **Footnote DOM and placement.** Footnote definitions are rendered where
    they appear in the source (with `data-footnote-number`), not collected
    into a trailing `<section class="footnotes">` as Obsidian does. The
    live DOM comparison was deferred by the user on 2026-10-31.
11. **`%%` comment edge cases.** comrak's ofm_comments rules are followed
    (block-spanning supported, contents never interpreted as markdown);
    unterminated `%%` stays literal. Obsidian edge cases (e.g. `%%` inside
    tables) unverified.
12. **macOS NFD filenames.** Matching normalizes both sides with NFC +
    casefold; we have no macOS machine to verify against real NFD storage
    (explicitly untested per user instruction).

### Closed

- **HANDOFF 4.3 extraction gaps** — escaped `\[[x]]`, `$a[[b]]c$` and raw
  HTML blocks each used to become link tokens; all three now extract zero
  tokens (confirmed against Obsidian and enforced by unit tests plus the
  `vault-md --example gaps` smoke test).

## Windows / macOS status (M0-M2)

- **Windows:** cross-compiled with [`cross`](https://github.com/cross-rs/cross)
  (Docker container, `:main` windows-gnu image — the 0.2.5 default image's
  binutils rejects modern rustc `.drectve` directives). **71 vault-core +
  vault-md tests, including the golden suite, run and PASS under Wine in the
  container** (2026-10-31). Caveat: Wine is not real Windows.
  libcosmic itself does not compile for Windows yet — fails in upstream
  `accesskit_winit` (E0782, pre-2021 `&Trait` argument syntax) on both the
  host mingw toolchain and cross's container toolchain — so the GUI remains
  unbuildable there until that pin is updated. Also note: **Wine rejects
  filenames containing combining characters** (ERROR_INVALID_NAME) — real
  NTFS accepts them, but we cannot verify that without real Windows, so the
  corpus's NFD-named fixture is exercised on Unix only (its test is
  `#[cfg_attr(windows, ignore)]`).
- **macOS:** untested — no machine, no SDK for cross-compilation.

## GUI reading view (M4a)

The libcosmic reading view renders `vault-md`'s IR directly into widgets
(iced `rich_text` spans), not HTML. Choices where a widget tree cannot
express what HTML does — all honest approximations, revisitable later:

- **Raw HTML** blocks/spans render as plain monospace text (the sanitized
  HTML path is the web renderer's job).
- **Remote images** (`http(s)`) are text placeholders — no network fetch in
  v1; local images render from disk with a per-path `Handle` cache.
- **Same-document `[[#Heading]]` links** emit a `scroll:` URI; since M4b
  they scroll the reading view to the matching heading (case-insensitive
  match on the rendered heading text). `[[#^block-id]]` links still no-op —
  the reading renderer's anchors are positional (`block-{index}`), not
  Obsidian block ids, so an id→index map is still needed.
- **Tag clicks** (`tag:`) since M4b open the sidebar Tags panel with that
  tag selected; **footnote-reference clicks** (`fnref:`) scroll to the
  footnote's definition block.
- **Task checkboxes** in lists are display-only; toggling from reading view
  is deferred (it rewrites the source line).
- **Callout fold state** is per-tab and keyed by traversal order, so it
  resets when the note is edited (ids are positional).
- Link routing uses URI schemes over span links (`note:`, `wiki:`,
  `create:`, `open:`); `create:<name>|<from>` creates the note in the vault
  root (Obsidian's default new-note location), refusing traversal/dotfiles.
- Source-editor highlighting is syntect's Markdown grammar (libcosmic
  `highlighter` feature, `token: "md"`) — theme follows system light/dark.
  A hand-rolled Markdown token set is a possible later upgrade.
- New-note creation and frontmatter edits save through the same guarded
  path: `Vault::save_note` (body-only, frontmatter bytes provably reused)
  or `Vault::save_note_full` (full text, added in M4a).
- **External changes and saves conflict-check**: a save's sentinel is the
  tab's base text (what the edits were based on), so a save after an
  external change is REFUSED rather than silently clobbering. The watcher
  poll classifies changes by content: an mtime-only `touch` is silent;
  a real change auto-reloads clean tabs (informational banner) and raises
  a conflict banner for dirty tabs — Reload from disk / Keep mine / Save
  mine anyway (the deliberate overwrite passes the fresh disk text as the
  sentinel).

## GUI navigation (M4b)

Sidebar panels (Files / Search / Tags / Links / Outline) and the quick
switcher, with the behaviors and limits decided for M4:

- **Vault search is literal ripgrep as a library** (the `grep` + `ignore`
  crates — ripgrep's own engine, no subprocess, no content index). Smart
  case like ripgrep: a lowercase query matches case-insensitively, any
  uppercase makes it case-sensitive; a query that is not a valid regex
  falls back to a literal (escaped) match. Scope is `.md` notes only,
  hidden entries (`.obsidian/`) skipped, ignore files not honored — the
  same scan surface as vault-core's index. Runs off the UI thread on
  submit (Enter or the Search button), capped at 200 hits. Opening a hit
  opens the note and scrolls the reading view to the first block whose
  plain text contains the query.
- **Quick switcher (Ctrl+P)** is nucleo fuzzy matching (helix's engine)
  over note paths, smart case + smart normalization, top 20. Enter
  activates, arrows move, Esc closes; clicking a row opens it directly.
  Inert when no vault is open.
- **Outline panel** lists top-level headings with their block index;
  clicking scrolls the reading view there. **Tags panel** lists all tags
  (frontmatter + inline) with note counts, then the notes of the selected
  tag (click the tag again to clear). **Links panel** shows backlinks of
  the active note.
- **Scroll-to-anchor** is a two-phase iced widget operation: measure the
  target block container's Y and the reading scrollable's content Y in one
  layout pass, then `scroll_to` the difference (minus a small margin). No
  frame deferral is needed even when the pane just switched Source→Split:
  iced's runner rebuilds all user interfaces *before* running the actions
  returned by update, so the measure op always sees the fresh tree.
- **Keyboard shortcuts** flow through a global `listen_with` subscription
  that ignores capture status (so they work while the editor has focus):
  Ctrl+P toggles the switcher, Ctrl+S saves. Everything else is filtered
  out in `update`.
- **Known fmt-gate note:** `cargo fmt --all` formats every non-registry
  package in the dependency graph — including the whole pinned libcosmic
  checkout at `/cella/Git/libcosmic` (113 files). The gate therefore uses
  `cargo fmt -p vault-glass -p vault-core -p vault-md -p app` so our
  formatting never rewrites the external checkout. (As of 2026-11-01 that
  checkout also contains a broken iced example,
  `iced/examples/sctk_subsurface_img/src/main.rs` — invalid pattern,
  never compiled — which is what surfaced this.)

## GUI polish + safety (M4c)

Daily notes, autosave, glass, themes, shortcuts, and tree file
manipulation, per the M4c plan and user decisions:

- **Daily notes** are explicit only (header "Daily" button or Ctrl+D —
  never auto-opened). Configuration comes from
  `.obsidian/daily-notes.json` (`folder`, `format`, `template` keys);
  a missing file falls back to `Journal/YYYY/MM/DD`, and a file without
  a format uses Obsidian's plugin default `YYYY-MM-DD`. The moment.js
  format tokens daily notes use are translated to chrono (longest token
  first, `[bracket]` escapes literal, unknown tokens verbatim).
  Templates fill `{{date}}`, `{{time}}`, `{{date:FMT}}`, `{{time:FMT}}`,
  `{{title}}` (unknown placeholders survive verbatim). Creating a note
  makes its folder chain and goes through the same guarded, atomic,
  watcher-bracketed write path as manual saves; dot-directory folders
  are refused. An existing daily note is simply opened.
- **Autosave** fires ~2 s (user-tuned) after the last editor activity,
  through the same guarded save path: an external change refuses the
  write and raises the conflict banner instead of clobbering. Clean
  tabs never re-save. Closing a tab (Ctrl+W or the tab's ✕) saves
  unsaved edits first; a refused save keeps the tab open.
- **Glass**: at startup `vault-glass` probes the platform (`platform::native()`);
  when glass is enabled and the session can blur (Wayland), the app sends
  libcosmic `Action::BlurEnabled` → `iced::window::enable_blur`
  (`ext-background-effect-v1`). Whether blur is visible remains up to the
  compositor and the theme's frosted-windows setting; without support the
  translucent surfaces (editor dark overlay, switcher dialog) simply stay
  unblurred — never an error. The offered style shows as "Backdrop:
  <style>" on the empty state.
- **Theme**: dark/light toggle in the header (label shows the target
  theme), applied via `Action::AppThemeChange` and persisted in
  `AppSettings.theme` (`system` follows the desktop). `AppSettings` also
  gained `glass_enabled` (default true); pre-M4c settings files keep
  working through serde defaults, and vault open load-modify-stores
  instead of overwriting.
- **Keyboard shortcuts** (all through the global `listen_with`
  subscription): Ctrl+P switcher, Ctrl+S save, Ctrl+D daily note,
  Ctrl+Shift+F search panel, Ctrl+W close tab, Esc cancels an open
  inline tree edit (after the switcher's Esc).
- **Tree right-click menu** (cosmic `context_menu`, one menu per row):
  New note, New folder, Rename, Delete. The opened menu records its
  target (`on_open`) since cosmic's `menu::Action` is `Copy` and cannot
  carry a path. Commands open an auto-focused inline name input under
  (or replacing) the row; Enter submits, Esc cancels. Empty folders
  materialize in the tree (`Vault::folders` + `build_with_folders`) so
  they can receive notes. Semantics:
  - **New note/folder** land beside the target (inside a folder; in a
    note's parent folder). Names may contain `/` to nest; dot-directory
    components are rejected.
  - **Rename** moves the file or folder (target must not exist). Links
    in other notes are NOT rewritten — their text keeps pointing at the
    old name and shows as unresolved (Obsidian parity is future work).
  - **Delete** moves the file/folder into the vault's `.trash/`
    (Obsidian-compatible: recoverable; dot-ignored so it vanishes from
    the app index). Trash names never collide (`name (2).md`, …).
    Unsaved edits of affected tabs are saved first so nothing lands in
    the trash half-written; a refused save aborts the deletion. Tabs of
    deleted paths close; folder renames remap tab paths and collapse
    state.
- **Tests** grew to 150: daily-note settings/translation/template/path
  and creation (vault-core), folder/rename/delete/trash semantics
  (vault-core), tree edit targets + empty-folder materialization (app),
  autosave decision + settings serde defaults (app), glass plan (app).

- **Theme toggle and blur** (fix): switching dark/light loads the user's
  *system* theme configs (`cosmic::theme::system_dark()`/`system_light()`),
  never the built-in default themes. libcosmic's `AppThemeChange` handler
  recomputes window blur from the incoming theme's `frosted_windows` flag
  and the accent from the theme itself, so a synthetic default theme would
  turn frosted windows (blur) off and drop the accent — the loaded system
  configs keep both intact.

## Read-only web server (M5)

New `vault-web` crate: axum, same core and IR as everything else,
server-rendered HTML, no JavaScript. Added to the workspace's
default-members, so plain `cargo fmt`/`clippy`/`test` cover it (fmt gate
now `-p vault-glass -p vault-core -p vault-md -p app -p vault-web`).

- **Run**: `vault-web [VAULT_PATH] [--port N] [--bind ADDR] [--token T]
  [--no-auth]`. The vault defaults to the GUI's last-used vault
  (same settings.json). Binds `127.0.0.1:8177` by default.
- **Routes**: `/` (index: notes grouped by folder + tag list),
  `/n/{*path}` (rendered note), `/tag/{*tag}` (wildcard so nested tags
  work), `/search?q=` (literal ripgrep, 200-hit cap), `/files/{*path}`
  (attachments, guessed MIME). Non-notes under `/n/` redirect to
  `/files/`.
- **Search moved to vault-core** (`vault_core::search`): the literal
  ripgrep module lifted out of the app so GUI and server share one
  implementation; grep/ignore/regex deps moved with it. No content
  index, per the M4 decision.
- **Link routing** (vault-md `RenderOptions` web mode): `note_base`/
  `file_base`/`tag_base` prefixes — resolved wikilinks → `/n/…`, media
  embeds and resolvable relative markdown images → `/files/…`,
  relative markdown links to notes → `/n/…`, tags → `/tag/…`.
  Absolute URLs and anchors pass through; `javascript:`/`vbscript:`/
  `data:` schemes are defused to `#`. Defaults are `None`, so the GUI
  HTML goldens are byte-identical to before.
- **Security** (the milestone's core): every user-controlled path passes
  one guard — `..` and dotfile components are rejected before any
  filesystem call (covers `..`, `%2e%2e`, `..%2F`, double-encoded), then
  the path is canonicalized and must stay inside the canonical vault
  root, which stops symlink escapes (notes and directories). Notes use
  the same guard because `note_source` alone would follow a symlinked
  note out of the vault. Every response carries `Content-Security-Policy:
  default-src 'none'` (inline stylesheet excepted) and
  `X-Content-Type-Options: nosniff`; raw HTML in notes goes through the
  existing ammonia sanitization.
- **Auth**: on by default — a token is generated and printed at startup;
  pass it as `?token=…` or `Authorization: Bearer …`. `--token T` sets
  it, `--no-auth` disables it (refused together with a non-loopback
  bind — a LAN-exposed vault must have an explicit token).
- **Snapshot semantics**: the index is built at startup; files added or
  changed on disk after startup appear only after a restart. In-vault
  changes to note bodies ARE picked up live for rendering
  (`note_source` reads from disk), but the link index is the startup
  snapshot.
- **Tests** (29 in vault-web): CLI parsing, the confinement unit tests,
  and 17 integration tests through the real router — including the
  traversal suite (`..`, encoded, double-encoded, dotfiles, symlinked
  file + directory escapes) and auth/headers. Plus a live curl smoke
  test against the running binary. 185 tests workspace-wide.

- **Web fixes from first real use** (user-reported): the index page
  opened a `<ul>` per folder but closed only the last one — runaway
  nesting for every folder, siblings included. It is now flat: one
  `<h2>` + one closed `<ul>` per folder, empty folders shown as
  `(empty)` (parity with the GUI tree). Authentication now survives
  navigation: the first `?token=`-authenticated response sets an
  `HttpOnly; SameSite=Lax` session cookie (`vault_token`), which the
  middleware accepts as a third credential alongside `?token=` and
  `Authorization: Bearer`. This also fixed broken image embeds — `<img>`
  fetches are separate requests that carry no token/header, so they
  401'd before the cookie existed. The cookie is session-scoped (dies
  with the browser); the token still works standalone for curl.

- **Web polish from second round of use** (user-requested): the index is
  now an actual folder tree — nested `<details>`/`<summary>` elements
  (pure HTML/CSS, no JS), collapsed by default like the GUI sidebar,
  each summary showing the recursive note count and each folder carrying
  its path as the anchor `id`. Note pages gained a visible
  `<h1 class="note-title">` (frontmatter title or file stem) and
  breadcrumbs (vault → each folder segment), where folder segments link
  to their tree entry on the index (`/#Notes/Sub`). Footnote
  definitions now render as one numbered `<section class="footnotes">`
  (`<ol>` with `<li value=…>` so visible numbers always match the
  reference-order numbers even when definition order differs) with `↩`
  back-links to the first reference; footnote definitions are pulled to
  the end of the page as Obsidian does. This changed the M2 golden
  `03-footnotes.html` deliberately (regenerated with `UPDATE_EXPECT=1`);
  numbering semantics (reference order) are unchanged. New tree module
  (`vault-web/src/tree.rs`) is pure data + pure rendering, unit-tested.

- **Light/dark theme switcher** (user-requested): pure HTML/CSS, no JS.
  The choice lives in a cookie `vault_theme=light|dark|auto` (Path=/,
  SameSite=Lax, HttpOnly, Max-Age 1 year); any URL accepts
  `?theme=light|dark|auto`, which sets the cookie and 303-redirects to
  the same location with the parameter stripped (other query parameters
  survive, e.g. `/search?q=x&theme=dark` → `/search?q=x`). An invalid
  value is ignored (rendered normally, no cookie). Themes are CSS
  custom properties scoped by `[data-theme='light'|'dark']` on `<html>`;
  `auto` (or no cookie) emits no attribute and a
  `@media (prefers-color-scheme: dark)` branch decides, so the default
  follows the OS. The nav bar gains Light / Dark / Auto links back to
  the current location (using `?` or `&` as appropriate), with the
  active choice highlighted. When a token is configured, a first visit
  with `?token=…&theme=…` sets both cookies and redirects (one click
  signs in and themes). Palette: the dark theme is the user's Substack
  palette verbatim (bg rgb(27 22 33)/rgb(41 36 46)/rgb(58 54 63), fg
  white/rgb(164 162 166)/rgb(124 121 128), accent rgb(161 136 101),
  accent-hover rgb(147 123 90), bg-accent rgb(255 103 25 / 0.2)) — note
  that palette ships identical values for both modes, so the light
  theme is derived: warm-paper bg rgb(250 248 244), bg-secondary white,
  bg-tertiary rgb(233 229 222), fg = the dark bg rgb(27 22 33),
  fg-secondary rgb(91 88 96), accents unchanged. Links across the site
  now use the tan accent instead of blue; `mark` uses bg-accent.
  CSP is unchanged (still no script; the inline stylesheet remains the
  only style exception).
