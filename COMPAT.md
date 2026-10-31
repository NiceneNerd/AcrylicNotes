# COMPAT — Obsidian compatibility notes

Every claim here is either **confirmed** (observed against real Obsidian or
the user's vault) or an **ASSUMPTION** (implemented on a documented guess,
to be confirmed in M3 against a real vault). Anything not listed here is
either verified by a test or still unimplemented.

## Confirmed

- **Stale relative links stay unresolved.** `[[../Prayer and Such/Discipuli]]`
  from `Journal/2026/04/15.md` (wrong `../` depth; target exists at vault
  root) shows as unresolved in real Obsidian (user-observed, 2026-10-31).
  Therefore relative link targets resolve strictly against the note's folder
  with NO basename fallback. `VaultIndex::resolve_relative` implements this.
- **`](#anchor)` destinations are not tags.** The 114 `](#footnote-N)` /
  `](#fmN)` markdown-link destinations in the vault do not appear in the
  Obsidian tag pane (user-observed). Link destinations and HTML attribute
  values are blanked before tag scanning; `vault index` now lists exactly 4
  tags for the vault (Immortality, Honor, kobo — all real; `import` is a
  Typst `#import` in prose that Obsidian's own rules also index).
- **`strictLineBreaks: true`** is set in this vault's `.obsidian/app.json`
  (read at render time; `RenderOptions::strict_line_breaks`).

## ASSUMPTIONS (confirm in M3)

1. **Callouts.** comrak 0.59 parses `> [!kind]`, `|metadata` inside the
   brackets, `+`/`-` fold markers, and default titles (capitalized type).
   We emit `data-callout="kind"` / `data-callout-foldable` /
   `data-callout-open`, mirroring Obsidian's DOM. The exact DOM class names
   and fold behavior are written from Obsidian's documented markup, not
   diffed against a live export.
2. **Block IDs.** A trailing `^id` on a paragraph or list item becomes that
   block's id; the id chars are alphanumeric plus `-`/`_`. A paragraph that
   is ONLY `^id` stays literal text (we guess Obsidian shows it too).
   Standalone-`^id`-on-own-line semantics are unverified.
3. **Tag boundaries.** A tag starts after whitespace or punctuation (not
   mid-word), per Obsidian's docs; escaped `\#` is never a tag. `(#tag)`
   after an open paren IS a tag here — unverified in Obsidian.
4. **Non-note targets need extensions.** `[[cat]]` does not resolve to
   `cat.png` (only `.md` is stripped). Unverified.
5. **Footnote rendering.** References are numbered in definition order and
   link to `#fn-<label>`; comrak auto-names inline footnotes (`__inline_N`).
   Obsidian's exact reading-view footnote DOM differs (it uses
   `<section class="footnotes">`); ours is functionally equivalent.
6. **Heading anchors.** `[[Note#Heading]]` emits
   `href="Note.md#<Heading text>"` — the anchor scheme is ours; the M5
   server must generate matching heading ids (slug = raw heading text).
7. **Math vs currency.** comrak's `math_dollars` rules keep `$5 and $10` as
   text (closing `$` followed by a digit is not math). Golden-tested; real
   vault currency notes not yet swept for false math.
8. **Sanitization is stricter than Obsidian.** Raw HTML is ammonia-cleaned
   (fragments only): `class`/`id`/`title` survive, `style` does NOT, scripts
   and event handlers never do. Obsidian allows `style`; revisit if vault
   CSS needs it.
9. **Embed depth limit = 3** with cycle detection (Obsidian has a similar
   guard with an unknown limit).
10. **Indented code vs list items.** The pre-pass treats a 4-space-indented
    line after a blank line as code even inside a list item; Obsidian's
    behavior there is unverified (HANDOFF 4.3).
11. **Section transclusion.** `![[Note#Heading]]` takes blocks from the
    matching heading (case-insensitive text match) to the next heading of
    the same or higher level; `![[Note#^id]]` takes exactly that block.
    Heading-matching rules in Obsidian are unverified.
12. **`%%` comments** follow comrak's ofm_comments rules (block-spanning
    supported, contents never interpreted as markdown). Unterminated `%%`
    stays literal. Obsidian edge cases (e.g. `%%` inside tables) unverified.
13. **4.3 ambiguous extractor cases still extract:** escaped `\[[x]]`,
    `$a[[b]]c$`, and raw-HTML-block wikilinks each become a token. What real
    Obsidian does for these three is still open (M3).
14. **macOS NFD filenames.** Matching normalizes both sides with NFC +
    casefold; we have no macOS machine to verify against real NFD storage
    (explicitly untested per user instruction).

## Windows / macOS status (M0-M2)

- **Windows:** cross-compiled with `x86_64-pc-windows-gnu`; all 53 vault-core
  and vault-md unit tests RUN and PASS under Wine (2026-10-31). Caveat: Wine
  is not real Windows. libcosmic itself does not compile for Windows yet
  (fails in upstream `accesskit_winit`, E0782) — the GUI remains unbuildable
  there.
- **macOS:** untested — no machine, no SDK for cross-compilation.
