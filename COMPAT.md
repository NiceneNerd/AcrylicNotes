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
- **Known fmt-gate note:** libcosmic + iced now come from git repos pinned in
  `app/Cargo.toml` instead of a local checkout, so there is no external
  checkout on disk for `cargo fmt` to rewrite. The gate uses
  `cargo fmt -p vault-glass -p vault-core -p vault-md -p app` to keep
  formatting limited to our own crates (`cargo fmt --all -- --check` stays
  green).

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

## GUI chrome + editor polish (M4c follow-up)

Follow-up pass on the desktop app after user testing: COSMIC-native
chrome, readable spacing, and standard editing shortcuts.

- **Chrome**: the sidebar panel switcher is a real COSMIC nav bar (icon +
  label per panel: folder/edit-find/edit-tag/link/view-list `-symbolic`),
  the Source/Reading/Split switcher is a single-select segmented control,
  and open notes use the COSMIC tab strip (real close buttons, dirty dot
  in the label). The nav bar's default style paints a solid
  `primary.base` surface and fills the parent height; both are overridden
  (transparent, shrink-to-fit) so the sidebar reads as one surface. Tree
  rows use chevron/folder/file icons, search hits show title +
  `path:line` + snippet, and the reading pane centers the document at
  ~46rem (736 px) with padding.
- **Editor surface**: the text editor's border is 1 px (it was a full
  `space_xxxs`, 4 px) and the text is inset from it; the reading pane
  has padding around the document column.
- **Frontmatter** in the reading view is collapsible (a `▾ Frontmatter`
  toggle, default open) and reuses the callout fold mechanism under a
  fixed sentinel fold id that never collides with block fold ids.
- **Status bar** under the note area shows the active tab's word and
  character counts. Both are cached on the tab and refreshed with the
  document — never recomputed per frame.
- **Editor shortcuts** are editor-scoped (iced `text_editor.key_binding`,
  falling back to the editor's own bindings for everything else; custom
  bindings capture their events, so global shortcuts are unaffected):
  - Undo/redo: Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y. iced's editor has no
    history, so the app keeps a per-tab snapshot stack (text + cursor
    before each edit, capped at 100 entries, cleared when an external
    reload replaces the buffer). Snapshots are restored through
    `Content::with_text` + `move_to`.
  - Formatting: Ctrl+B `**bold**`, Ctrl+I `*italic*`, Ctrl+K `[link]()`,
    Ctrl+` `` `code` ``, Ctrl+Shift+X `~~strike~~`, Ctrl+Shift+H
    `==highlight==`. With a selection the markers wrap it; without one
    the empty markers are inserted and the cursor lands between them (for
    links, inside the parentheses).
  - Footnotes: Ctrl+Shift+F inserts a `[^N]` reference at the cursor and
    appends `[^N]: ` at the end of the document. N is the first number
    without an existing `[^N]:` definition — bare references do not
    reserve a number.
  - Word delete: Ctrl+Backspace / Ctrl+Delete select the word next to the
    cursor and delete it (a lone Backspace/Delete when a selection
    already exists).
- **Auto-pairs**: typing `(`, `[`, `{`, `"`, `'` or `` ` `` wraps the
  current selection in the pair, or inserts the pair with the cursor
  between the markers. Typing the closing character right before the
  same character steps over it instead of duplicating it. Quotes only
  pair at word boundaries, so an apostrophe inside a word stays a plain
  character ("don't" is never "don''t"). The pair table is one function
  (`pair_for`).

## Editor QOL rounds (M4d)

Testing feedback turned into four focused phases (A–C shipped so far).
All chord changes below supersede the M4c follow-up section above.

### Phase A — shortcuts, chrome details

- **Shortcut architecture**: the global key subscription now carries the
  iced `event::Status`. Captured events are skipped in `update` so
  editor-scoped bindings never double-fire global shortcuts — except
  while one of the app's own text inputs (quick switcher, inline tree
  rename, find bar) owns the keyboard: their keys are exactly what the
  global handler routes.
- **Chord changes**: Ctrl+F is *always* find-in-note (was vault search);
  vault search moves to Ctrl+Shift+F (both shifted spellings `f`/`F`
  route there); footnotes move from Ctrl+Shift+F to **Ctrl+Alt+F**.
  Ctrl+P / Ctrl+S / Ctrl+D / Ctrl+W are unchanged.
- **Window title**: the desktop window title (task switcher) is now
  `note — vault`, the vault name alone, or `Vault Notes` — synced on
  vault open and tab open/close/switch (`core.set_title` was never
  called before, so the title showed "unknown").
- **Reading pane padding** actually pads now: iced 0.14
  `Scrollable::padding` only insets its scrollbar, so the padding moved
  to the wrapping container.
- **Pane segmented control** gained button padding (its default was
  zero); the theme toggle is an icon button (sun/moon, showing the
  theme a click switches to).
- **Quick switcher** rows show the note's parent path dimmed beside the
  title.
- **File tree** hides the `.md` extension on note rows by default
  (Obsidian-like); `hide_md_extension` persists in settings (default
  true, UI toggle pending the settings drawer).

### Phase B — find in current note (Ctrl+F)

- An app-level find bar sits above the note area: query input, live
  `n of m` count, go-up/go-down, `Aa` case toggle, close, plus a
  replace row (`Replace` / `All`). It opens pre-filled from a
  single-line selection and focuses its input.
- iced's text editor has no find or match-painting API, so matching is
  app-level: a pure scanner over `text.split('\n')` produces
  (line, byte-start, byte-end) hits — byte columns, like the editor's
  cursor columns. Navigation selects the match in the editor (so the
  editor's own selection highlight shows it); it wraps, and opening
  jumps to the first match at/after the current selection anchor.
- Replace / Replace All splice the buffer byte-exactly, one undoable
  step each (per-tab snapshot stack). Only ASCII case folding is
  applied, which keeps byte offsets stable.
- While the bar is open the global handler routes Enter (next),
  Shift+Enter (previous), and Esc (close).

### Phase C — wikilink autocomplete

- Typing `[[` in the editor opens a suggestion dropdown (max 8 titles,
  ranked by the same nucleo fuzzy matcher as the quick switcher). It
  tracks the cursor: the unclosed `[[` in the cursor's line and the
  partial link text as the query. A closed link (`]`), an alias
  separator (`|`), or a `[[` on an earlier line closes it.
- Tab (or clicking a row) completes the link as `[[Title]]` — the
  partial text from `[` to the cursor is replaced in one undoable step.
  Ctrl+Tab / Ctrl+Shift+Tab cycle the highlight (wrapping); Esc
  dismisses.
- Esc is intercepted in the editor's `key_binding` (custom
  `WikiDismiss` binding before the default unfocus) because the editor
  owns that key; Tab and Ctrl+Tab are not editor-bound and reach the
  global handler, which routes them while the dropdown is open.
- The dropdown floats over the editor's bottom edge (iced exposes no
  caret pixel position), is not opaque — clicks outside it fall through
  to the editor — and shows only in Source and Split panes. It clears
  on pane/panel switches, tab open/close, undo/redo, find replaces, and
  any editor action that leaves the unclosed `[[`.

### Phase D — editor context menu

- Right-clicking the editor pane opens a COSMIC context menu with
  Copy, Cut, Paste, and Select All. Copy writes the selection to the
  clipboard; cut is copy + delete (one undo step); paste reads the
  clipboard and inserts at the cursor (one undo step); select-all uses
  the editor's own action.
- Paste is asynchronous (clipboard read returns a task), so it arrives
  as a follow-up message; a second paste request while one is in
  flight is ignored rather than double-inserting.

### Phase E — source spans (scroll-sync foundation)

- `Doc` now carries one `SourceSpan` (1-based inclusive file lines) per
  top-level block, parallel to the blocks. Front-matter lines shift
  spans; blank lines, comment-only paragraphs, and removed `^id`
  paragraphs attribute to the block above (spans stretch to the next
  block's start). `Doc::block_at_line` maps a file line to its block.
- The parser stays line-preserving: `extract` never consumes newline
  characters (locked in with a test), so spans computed from raw line
  counts stay valid through lowering.

### Phase F — scroll sync (editor → preview)

- In Split mode the reading pane follows the editor caret: when the
  caret changes line, the preview scrolls so the block containing that
  line is near the top (12 px margin). It also re-syncs on tab switch.
- Gated by the `scroll_sync` setting (default on) and fires only on a
  line change, so typing within a line never rescrolls.
- The reverse direction (preview click → caret) is not implementable:
  iced's editor exposes no programmatic scroll API.

### Phase G — settings drawer + attachments in the tree

- A gear button in the header opens the right-side COSMIC context
  drawer with every persisted preference: auto-pairs, wikilink
  suggestions, word wrap, monospace editor font, editor font size
  (10–28 px stepper), theme (System/Dark/Light), solid sidebar, default
  pane layout (Split/Source/Reading), hide `.md` in the tree, autosave
  delay (1–60 s stepper), and scroll sync.
- Each change applies immediately and persists through
  load-modify-store, so settings never clobber each other. The editor
  honors font family, size, and wrapping live; disabling auto-pairs or
  wikilink autocomplete short-circuits those features; autosave uses
  the configured delay; a fresh session starts in the saved layout.
- The "solid sidebar" toggle paints the sidebar with the theme surface
  instead of the translucent one (for compositors/window rules where
  the frosted-window blur is unwanted).
- Non-markdown vault files (attachments) now appear in the file tree
  with an attachment icon, sorted among the notes. Clicking one opens
  it with the platform handler (`xdg-open`). They deliberately get no
  rename row, dirty dots, or context menu — renaming wouldn't rewrite
  links to them — and they stay out of search and the switcher.
- Which file types appear is configurable in the settings drawer
  ("File types in tree"): an extension whitelist (lowercase, stored
  without the dot) with add/remove rows. An empty list — the default —
  shows every attachment; a non-empty list shows only matching files
  (case-insensitive). Notes are never filtered.

### Phase H — Unicode codepoint entry (Ctrl+Shift+U)

- GTK's Ctrl+Shift+U hex entry lives in its input module, which iced
  apps don't use, so the same interaction is implemented at the app
  level: pressing Ctrl+Shift+U in the focused editor opens codepoint
  entry, shown as a small popup at the editor's bottom edge with "u" +
  the digits typed so far, the resolved character once the codepoint
  is valid, and key hints.
- While entry is open the editor swallows plain keys: hex digits
  accumulate (any case, lowercased, capped at 6 digits), Space/Enter
  commit, Backspace unravels a digit (exiting when empty), Esc
  cancels, Tab is ignored. Chords with Ctrl/Alt still reach the
  normal shortcuts, and arrow keys move the caret; commit inserts at
  wherever the caret then sits.
- Commit inserts the codepoint as one undoable step. Invalid
  codepoints (surrogates, > U+10FFFF) end the mode without changing
  the buffer.
- The mode is dismissed by tab open/close/switch, pane switches, and
  sidebar navigation, and starting it dismisses the wikilink dropdown
  (the two editor popups never coexist).
