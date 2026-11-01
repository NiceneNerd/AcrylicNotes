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
