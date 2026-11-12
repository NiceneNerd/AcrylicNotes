# Feature Inventory

Every user-visible behavior of the current Linux (libcosmic) frontend, assigned a parity
ID. This is the contract both frontends must satisfy. Statuses live in
[parity.md](parity.md). All paths are repo-relative; line references are against the
baseline commit of Phase 0 (`main` at the restructure start).

Source of truth: the `Message` enum (`app/src/app.rs:317–541`) and its `update` handler
(`app.rs:3903–4916`); the non-test surface of `app.rs` ends at line 5169.

## 1. Window / chrome (WIN-*)

| ID | Behavior | Source |
| --- | --- | --- |
| WIN-001 | Vault path input, `VaultOpenSubmitted`, `VaultOpen(PathBuf)` (empty state) | app.rs |
| WIN-002 | `ToggleTheme` (cycles System→Light→Dark; System starts dark; loads system dark/light configs so accent survives; persisted) | app.rs:4057–4070, 1793–1809 |
| WIN-003 | `ToggleSidebar` (whole sidebar shown/hidden, 250 px, persisted) | app.rs, view.rs:283–343 |
| WIN-004 | `WindowResized(f32)` drives the narrow-window fold | app.rs:4991–4998 |
| WIN-005 | `Surface` — Wayland popup plumbing for context menus | app.rs:4088 |
| WIN-006 | Settings drawer toggle | app.rs:4410–4414, 4922–4933 |
| WIN-100 | Global key listener always subscribed; captured events skipped unless an app overlay input owns the keyboard | app.rs:1126 |
| WIN-101 | Modifier tracking across key/wheel events (wheel carries no modifiers) | app.rs:4793, 5009 |
| WIN-102 | Esc priority chain: switcher → tree-edit → find → wiki → settings drawer | app.rs:4806–4857 |
| WIN-103 | Ctrl+P — quick switcher | app.rs:4860 |
| WIN-104 | Ctrl+S — save active tab | app.rs:4861 |
| WIN-105 | Ctrl+D — open/create daily note | app.rs:4862 |
| WIN-106 | Ctrl+Shift+F — vault-wide Search panel | app.rs:4867–4872 |
| WIN-107 | Ctrl+F — in-note find bar (prefills from selection) | app.rs:4873 |
| WIN-108 | Ctrl+W — close active tab (saves dirty first) | app.rs:4874–4877 |
| WIN-109 | Ctrl+= / Ctrl++ zoom in; Ctrl+- zoom out (unshifted keys; both spellings) | app.rs:4882–4883 |
| WIN-110 | Ctrl+scroll wheel — zoom the pane in focus one step per notch | app.rs:3997–4005 |
| WIN-111 | Find-bar keys: Enter next, Shift+Enter prev, Esc close | app.rs:4821–4829 |
| WIN-112 | Switcher keys: arrows move, Esc closes (Enter via on_submit) | app.rs:4806–4815 |
| WIN-113 | Tree-edit keys: Esc cancels inline input | app.rs:4816–4820 |
| WIN-114 | Wiki-dropdown fallback keys: arrows cycle, Tab accepts, Ctrl+Tab cycles, Esc dismisses | app.rs:4830–4849 |
| WIN-115 | Documented absences: no Ctrl+Tab/Ctrl+PgUp-PgDn tab switching, no pane-focus keys, no Ctrl+N, no Ctrl+E, no Ctrl+O | — |

## 2. Vault / external (VAULT-*, PLAT-*)

| ID | Behavior | Source |
| --- | --- | --- |
| VAULT-001 | `WatchTick` — 2 s watcher poll; reload externally changed files, re-index | vault-core/src/vault.rs:97–165, app.rs:3756–3797 |
| VAULT-002 | `DailyNote` — path from `.obsidian` daily-notes settings, template rendering, folder chains | vault-core/src/vault.rs:400–466, app.rs:2027–2043 |
| VAULT-003 | Vault open restores `last_vault` on startup when the dir exists | app.rs:3892–3898 |
| PLAT-001 | `OpenExternal` — URLs go to the platform's default handler via `vault_platform::ExternalOpener` (Phase 3); the old non-Linux no-op discard is gone by construction. Linux adapter spawns `xdg-open` fire-and-forget; spawn failures surface on the error line | crates/vault-platform/src/opener.rs, crates/vault-platform-linux/src/opener.rs |
| PLAT-002 | `OpenAttachment` — vault-relative → absolute, handed to the opener as `OpenTarget::File`; same Linux adapter | app.rs `Message::OpenAttachment` |
| PLAT-003 | Glass probe behind `vault_platform::AppearanceService` (Phase 3): Linux adapter delegates to the vault-glass Wayland probe (X11 → Opaque); Blur requests `Action::BlurEnabled` | crates/vault-platform-linux/src/appearance.rs, app/src/glass.rs |
| PLAT-004 | Non-Linux glass backends exist in vault-glass (Mica/Acrylic/Vibrancy fallbacks) but GUI only consumes the probe | vault-glass/src/lib.rs:9–18 |
| PLAT-005 | Translucent surfaces: editor overlay 0.18, switcher 0.88, sidebar 0.9, popups 0.94 | view.rs, app.rs |
| PLAT-006 | Image rendering — local raster from disk (cached Handle), SVG via path; remote images are text placeholders (no network); pinned iced is `image-without-codecs` | reading.rs:1045–1158 |
| PLAT-007 | Non-renderable embeds (video/audio/pdf) → `[attachment: …]` placeholders | reading.rs:1068–1073 |
| PLAT-008 | Clipboard via iced clipboard (cross-platform) | app.rs:4360–4409 |
| PLAT-009 | Wayland/COSMIC popups via `Message::Surface`; per-target libcosmic features; APP_ID `dev.vault-handoff.Notes` | app/Cargo.toml:42–61 |
| PLAT-010 | Tree drag-and-drop is app-level pointer state (works on X11 too), not Wayland dnd | drag.rs:1–22 |
| PLAT-011 | Watcher: notify `RecommendedWatcher` + debouncer (inotify); our own writes suppressed | vault-core/src/vault.rs:9–11 |
| PLAT-012 | Paths: `dirs` for XDG config, `~` expansion | settings.rs:295–297 |
| PLAT-013 | Window setup: default 1180×760 | main.rs:23–27 |

## 3. Tabs (TAB-*)

| ID | Behavior | Source |
| --- | --- | --- |
| TAB-001 | `TabOpen(String)` — open-or-focus note; existing notes never duplicated | app.rs:2000–2025 |
| TAB-002 | `TabClose(usize)` / `TabCloseRequested(usize)` — close-save flow | SAVE-009 |
| TAB-003 | `TabActivated(usize)` — drops previous tab's live state, dismisses overlays, syncs title/preview/focus | app.rs:4114–4128 |
| TAB-100 | Tab labels: stem title, `•` dirty prefix, dimmed folder path (120 px end-ellipsis beyond 16 chars) | app.rs:1755–1767, view.rs:1007–1070 |
| TAB-101 | Tab strip: active tinted accent 0.14, per-tab close, horizontal scroll, "No open tabs" placeholder | view.rs:987–1005 |

## 4. Editor (EDIT-*)

| ID | Behavior | Source |
| --- | --- | --- |
| EDIT-001 | Syntax highlighting — syntect Markdown; Base16Ocean (dark) / InspiredGitHub (light) | view.rs:1180–1210 |
| EDIT-002 | Wikilink highlighting over syntax theme (target accent, alias success), code-fence aware | highlight.rs |
| EDIT-003 | Contrast guarantees (body ≥10:1, placeholder ≥5.5:1 vs composited surface) | editor_text.rs:32–41 |
| EDIT-004 | Editor surface: transparent over black 0.18 overlay, accent focus border | view.rs:1148–1165 |
| EDIT-005 | Editor fonts: mono toggle overridden by custom family, unknown names fall back | editor_text.rs:179–224 |
| EDIT-006 | Word wrap toggle in both editors | view.rs:1195–1199 |
| EDIT-007 | Undo/redo — per-tab snapshot stacks (text+cursor), limit 100, redo cleared by edits | app.rs:800–842 |
| EDIT-008 | Formatting: Bold/Italic/Strike/Code/Highlight/Link — wrap selection or insert empty markers | app.rs:292–314, 847–872 |
| EDIT-009 | Footnote insertion (Ctrl+Alt+F): `[^N]` + `[^N]: ` definition; N = first undefined; definition blank-line normalization; code-fence aware | app.rs:884–1005 |
| EDIT-010 | Word delete (Ctrl+Backspace/Delete): unicode word boundaries, line-break crossing, one undo step | app.rs:1007–1023 |
| EDIT-011 | Auto-pairs `( [ { " ' \` \` \` around selection (stays selected for nesting); step-over closer; boundary-checked quotes; setting off inserts raw | app.rs:1417–1592 |
| EDIT-012 | Clipboard Copy/Cut/Paste/Select-All routed through app messages (join undo/dirty bookkeeping); paste is async | app.rs:4360–4409 |
| EDIT-013 | Editor context menu: Undo, Redo, Cut, Copy, Paste, Select All | view.rs:512–557 |
| EDIT-014 | Editor focus persistence — stable `note-editor` id + popup slot | view.rs |
| EDIT-015 | Find bar UI: query, live count "N of M", up/down, case "Aa" toggle, replace row | view.rs:1075–1136 |
| EDIT-016 | Find matching: literal substring per line, ASCII case folding (byte-stable offsets) | app.rs:1130–1234 |
| EDIT-017 | Find navigation: first match at/after cursor, wrap next/prev, match selected (live: region caret lifted to file coords) | app.rs:2816–2925 |
| EDIT-018 | Replace one/all — single undo step; selection points at following match | app.rs:2927–3024 |
| EDIT-019 | Wiki completion trigger: unclosed `[[`, re-evaluated per edit, setting-gated | app.rs:3153–3205 |
| EDIT-020 | Wiki dropdown: 8 title rows, dimmed parents, click accepts, bottom-left anchored | view.rs:1256–1294 |
| EDIT-021 | Wiki acceptance: replaces partial per `link_path_format`, keeps custom label, consumes auto-paired `]]`, one undo step, works in live buffers | app.rs:3207–3318 |
| EDIT-022 | Unicode entry overlay (Ctrl+Shift+U): hex buffer (max 6), Space/Enter commit, Backspace unravels, Esc cancels | app.rs:1685–1712, 4524–4567 |
| EDIT-100 | Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y undo/redo | app.rs:1609–1614 |
| EDIT-101 | Ctrl+B/I/K/\`, Ctrl+Shift+X/H formatting shortcuts | app.rs:1615–1620 |
| EDIT-102 | Ctrl+Alt+F footnote (Ctrl+Shift+F belongs to vault search) | app.rs:1621–1623 |
| EDIT-103 | Ctrl+Shift+U Unicode entry | app.rs:1624–1625 |
| EDIT-104 | Ctrl+Backspace / Ctrl+Delete word delete | app.rs:1629–1634 |
| EDIT-105 | Plain bracket/quote → auto-pair (uses produced text, layout-independent) | app.rs:1635–1645 |
| EDIT-106 | Unhandled keys fall through to editor widget bindings | view.rs:1577–1580 |

## 5. Panes / modes (PANE-*)

| ID | Behavior | Source |
| --- | --- | --- |
| PANE-001 | PaneMode Live (default) / Split / Source / Reading; persisted `default_pane` | settings.rs:174–186 |
| PANE-002 | Header segmented control; highlight follows the rendered pane | app.rs:1736–1753 |
| PANE-003 | `PaneActivated` cancels narrow-restore, drops live state, dismisses overlays | app.rs:4100–4113 |
| PANE-004 | Narrow-window fold: below 480 px Split → Source; restores unless user chose otherwise | app.rs:559, 2145–2154 |
| PANE-005 | Split layout: fixed 1:1, no ratio drag | view.rs:969–975 |
| PANE-006 | Zoom target tracking (Reading/Source/Live-focused-block/Split last-touched) | app.rs:2111–2136 |
| PANE-007 | Zoom steps ±1 px, clamp 8..=72, persisted | settings.rs:241–249 |
| PANE-008 | Scroll sync (Split): reading follows caret's top-level block on line change; off in Source; no reverse direction | app.rs:2653–2672 |
| PANE-009 | JumpToBlock: Live focuses region; Source switches to Split first; reading scrolls block to top | app.rs:2158–2181, scroll.rs:106–138 |
| PANE-010 | Reading click marks reading as zoom target | view.rs:1325–1339 |
| PANE-011 | Reading margin (None/Compact/Comfortable/Spacious → 0/16/32/56 px) + 8 px vertical padding, 8 px scrollbar spacing | view.rs:31–38, 1343–1351 |
| PANE-012 | Live pane: regions per top-level block + frontmatter region; click edits raw source; Esc commits | live.rs:53–101 |
| PANE-013 | Live auto-focus: empty note opens body region ("Start writing…"); re-ensured on tab/pane switch | app.rs:2216–2231 |
| PANE-014 | Live region navigation: arrows cross region boundaries preserving column (visual-line aware) | app.rs:2441–2491 |
| PANE-015 | Live newline merging: Backspace/Delete at region edges deletes joining newline, one undo step | app.rs:2496–2577 |
| PANE-016 | Live word delete across region edges | app.rs:2585–2645 |
| PANE-017 | Live splice: region buffer spliced back (line-range replace), re-parse, re-aim caret, re-eval wiki | app.rs:2310–2388 |
| PANE-018 | Live undo/redo: whole-note snapshots, editor re-aimed at restored caret's region | app.rs:4180–4221 |
| PANE-019 | Live scroll-into-view: `live-region-N` anchors + `live-editor-N` per-region editor ids | scroll.rs:38–63 |
| PANE-020 | Frontmatter in live: foldable; click focuses raw-source region | view.rs:1449–1455 |
| PANE-100 | Live editor Esc commits region (wiki dropdown wins) | view.rs:1616–1620 |
| PANE-101 | Live arrows cross regions / walk wiki dropdown | view.rs:1623–1628 |
| PANE-102–107 | Live editor undo/redo, formatting, backspace/delete, word delete, auto-pair, widget bindings | view.rs:1597–1670 |

## 6. Navigation (NAV-*)

| ID | Behavior | Source |
| --- | --- | --- |
| NAV-001 | Sidebar shell: COSMIC nav bar, 250 px, frosted panel, corner radius | view.rs:283–343 |
| NAV-002 | Panel switching (click + programmatic), dismisses editor overlays | app.rs:4089–4099 |
| NAV-003 | Files panel: tree + toolbar, "No notes.", root-level inline new input | view.rs:345–376 |
| NAV-004 | Files toolbar: sort cycle + menu, Collapse all/others, Expand all, Reveal current | view.rs:388–477 |
| NAV-005 | Tree rows: chevron+folder icons, 14 px indent/depth, `●` active / `•` dirty, `.md` hidden per setting | view.rs:595–709 |
| NAV-006 | Search panel: input + button, worker-thread search, "No matches."/"N matches", title + path:line + 64-char snippet | view.rs:713–747 |
| NAV-007 | Search engine: ripgrep library, `.md` only, hidden skipped, gitignore NOT respected, smart-case, regex w/ literal fallback, 200-hit cap | vault-core/src/search.rs:29–80 |
| NAV-008 | `SearchOpen`: opens note, Source→Split, scrolls reading to first block containing query | app.rs:4719–4738 |
| NAV-009 | Tags panel: sorted tags with counts, back button, click-clears filter | view.rs:750–796 |
| NAV-010 | Links panel: backlinks of active note, nearest-first resolution | view.rs:800–826 |
| NAV-011 | Outline panel: headings indented 14 px × (level−1) capped 4; click jumps (PANE-009) | view.rs:829–858 |
| NAV-012 | Quick switcher: 20 items max; empty query = recent-first; fuzzy per-word conjunctive (nucleo) | app.rs:2046–2073, fuzzy.rs:26–120 |
| NAV-013 | Switcher UI: auto-focused input, `▸` selection, dimmed backdrop, max width 560, opaque layer | view.rs:882–940 |
| NAV-014 | Switcher activation: Enter/click/Esc; no-op without vault | app.rs:4750–4792 |

## 7. Tree (TREE-*)

| ID | Behavior | Source |
| --- | --- | --- |
| TREE-001 | `TreeMenu` actions: NewNote, NewFolder, Rename, Delete (context menu) | app.rs:707 |
| TREE-002 | Inline rename/create input ("Name", auto-focused, Enter/Esc) | view.rs:572–593 |
| TREE-003 | `ToggleFolder`, `ToggleFold` persistence of expanded state | app.rs |
| TREE-004 | `TreeSort` (4 orders), collapse/expand-all, collapse-others, reveal current | view.rs:388–477 |
| TREE-005 | Drag-and-drop move (app-level pointer state, 6 px threshold, measured row bounds) | drag.rs |
| TREE-006 | Delete acts immediately — **no confirmation dialog**; files go to `.trash/` (flagged for parity) | app.rs:3692–3738 |
| TREE-007 | Sort semantics: folders first (case-insensitive name), notes per order, attachments sink last in date orders | tree.rs:40–155 |
| TREE-008 | Attachment tree filter: normalized extensions, case-insensitive, empty shows all | app.rs:768–785 |

## 8. Links (LINK-*)

| ID | Behavior | Source |
| --- | --- | --- |
| LINK-001 | Click routing: `note:`/`wiki:`/`create:` resolve-then-open-or-create; `open:` OS opener; `scroll:` same-doc heading; `fnref:` definition; `tag:` Tags panel | app.rs:4631–4689 |
| LINK-002 | Generated link spelling: Shortest/Relative/Absolute per `link_path_format` | app.rs:1354–1415 |
| LINK-003 | Link rewrite on rename/move: backlink snapshot (notes + embed-attachments), prompt only when resolution changes, guarded saves, undoable in open tabs | app.rs:3524–3687 |
| LINK-004 | Link-update prompt banner: "Update links" / "Leave as is" | view.rs:232–268 |
| LINK-005 | New-note name guards: reject empty/traversal/dot-dirs/backslashes | app.rs:5140–5157 |

## 9. Persistence (SAVE-*)

| ID | Behavior | Source |
| --- | --- | --- |
| SAVE-001 | Ctrl+S / Save button saves only the active tab | app.rs:4568 |
| SAVE-002 | Save plan: body-only write (frontmatter bytes verbatim) when BOM+FM match disk; full text otherwise — **shared** in `vault_app::workflow::save_plan` since Phase 4 | crates/vault-app/src/workflow.rs |
| SAVE-003 | Guarded save: base-text sentinel; external change refused (`ExternallyModified`) → conflict banner — **shared** in `vault_app::workflow::guarded_save` over the `NoteRepository` trait (live `Vault` adapter, `MemoryRepository` fake) since Phase 4 | crates/vault-app/src/{workflow,repository}.rs |
| SAVE-004 | Autosave: 2 s tick, active tab, idle ≥ `autosave_secs` (default 2, clamp 1..=60) — the idle policy is **shared** in `vault_app::workflow::autosave_due` since Phase 4 | crates/vault-app/src/workflow.rs, app.rs tick |
| SAVE-005 | Watcher poll with own-write suppression | vault-core/src/vault.rs:97–165 |
| SAVE-006 | External-change classification: identical → silent; clean → auto-reload + info banner; dirty → conflict banner | app.rs:5030–5052 |
| SAVE-007 | Conflict banner: Reload from disk / Keep mine / Save mine anyway (fresh sentinel) — the decision application is **shared** in `vault_app::workflow::resolve_conflict` since Phase 4 | crates/vault-app/src/workflow.rs, app.rs banner handlers |
| SAVE-008 | Reloaded info banner + Dismiss | view.rs:195–208 |
| SAVE-009 | Close-tab: dirty saves first; refused save keeps tab open | app.rs:2719–2745 |
| SAVE-010 | Delete pre-save: dirty affected tabs saved first; refusal aborts deletion | app.rs:3692–3709 |
| SAVE-011 | Dirty definition: buffer text ≠ base text | app.rs:231–233 |
| SAVE-012 | Error surfacing: dismissible error line; save success clears it | app.rs:2699, 3966–3971 |

## 10. Settings (SET-*)

Persisted JSON at `$XDG_CONFIG_HOME/vault-handoff/settings.json`; pretty, atomic
tmp+rename, corrupt/missing → defaults, unknown legacy keys ignored; per-change
load-modify-store (SET-021/022).

| ID | Key | Type / default |
| --- | --- | --- |
| SET-001 | `last_vault` | Option<PathBuf> |
| SET-002 | `theme` | ThemeChoice: system (default) / dark / light |
| SET-003 | `glass_enabled` | bool, true |
| SET-004 | `hide_md_extension` | bool, true |
| SET-005 | `scroll_sync` | bool, true |
| SET-006 | `auto_pair` | bool, true |
| SET-007 | `wiki_autocomplete` | bool, true |
| SET-008 | `word_wrap` | bool, true |
| SET-009 | `editor_font_mono` | bool, true |
| SET-010 | `editor_font_family` | String, empty |
| SET-011 | `editor_font_size` | u16, 14 (clamp 8..=72) |
| SET-012 | `reading_font_family` | String, empty |
| SET-013 | `reading_font_size` | u16, 16 |
| SET-014 | `reading_margin` | none / compact(16) / comfortable(32, default) / spacious(56) |
| SET-015 | `sidebar_open` | bool, true |
| SET-016 | `autosave_secs` | u64, 2 (clamp 1..=60) |
| SET-017 | `default_pane` | live (default) / split / source / reading |
| SET-018 | `attachment_extensions` | Vec<String>, normalized |
| SET-019 | `link_path_format` | shortest (default) / relative / absolute |
| SET-020 | `tree_sort` | name-asc (default) / name-desc / newest / oldest |
| SET-023 | Settings drawer UI: Editor / Reading / View / Files & saving sections | view.rs:1696–1933 |
| SET-024 | Drawer open/close (header button, Esc) | app.rs:4410–4414 |

## 11. Dialogs / overlays (DLG-*)

| ID | Behavior |
| --- | --- |
| DLG-001 | Quick switcher overlay (NAV-013) |
| DLG-002 | Conflict + reloaded banner (SAVE-007/008) |
| DLG-003 | Link-update prompt banner (LINK-004) |
| DLG-004 | Dismissible error line |
| DLG-005 | Inline tree-edit input |
| DLG-006 | Tree row context menu (Wayland popup): New note, New folder, Rename, Delete |
| DLG-007 | Sort context menu (4 checkbox items) |
| DLG-008 | Editor context menu (EDIT-013) |
| DLG-009 | Settings context drawer |
| DLG-010 | Find bar (non-modal strip) |
| DLG-011 | Wiki suggestion dropdown |
| DLG-012 | Unicode entry popup |
| DLG-013 | **No delete confirmation** — Delete acts immediately (TREE-006) |
| DLG-014 | Esc routing order (WIN-102) |

## 12. Rendering / misc (MISC-*, STATUS-*)

| ID | Behavior |
| --- | --- |
| STATUS-001 | Status bar "<words> words · <chars> characters" (whitespace-run counting) |
| STATUS-002 | Selection counts appended when non-empty |
| STATUS-003 | Counts cached per tab, refreshed with doc |
| MISC-100 | Window/desktop title "note — vault" / vault / "Vault Notes" |
| MISC-101 | Header buttons: sidebar, Save, Daily note, Theme, pane segmented control, Settings |
| MISC-102 | Empty state: icon, explainer, vault path input, "Backdrop: <style>", error |
| MISC-103 | Note-area placeholder |
| MISC-104 | Reading renderer: heading factors 1.6/1.35/1.2/1.1/1.0, callouts (Obsidian kinds, tinted), quotes, code blocks (mono, scrollable), math, tables, rules, HTML-as-text, tasks ☑/☐ |
| MISC-105 | Callout folding `+`/`-`, per-tab toggle ids, positional ids survive count-preserving edits |
| MISC-106 | Frontmatter rendering: foldable header, two-column table with tree guides, raw fallback |
| MISC-107 | Footnote rendering: definitions at end under rule; superscript numbered refs (accent, clickable); named keep brackets |
| MISC-108 | Embeds: depth limit 4, cycle detection, `#h`/`^id` slicing |
| MISC-109 | Soft/hard breaks; `.obsidian` strict_line_breaks honored |
| MISC-110 | Link click routing (LINK-001) |
| MISC-111 | Generated link spelling (LINK-002) |
| MISC-112 | Link rewrite on rename/move (LINK-003) |
| MISC-113 | New-name guards (LINK-005) |
| MISC-114 | Daily notes (VAULT-002) |
| MISC-115 | Tree sort semantics (TREE-007) |
| MISC-116 | Attachment filter (TREE-008) |
| MISC-117 | Scroll plumbing: two-phase measure+scroll ops (margins 12/12/4 px) |
| MISC-118 | Constants: SWITCHER 20, WIKI 8, SEARCH 200, UNDO 100, SIDEBAR 250/0.9, NARROW 480, DRAG 6 px, EMBED 4, UNICODE 6, fonts 8..72 |
| MISC-119 | Theme switching (WIN-002) |

## Explicit Linux-only items today

- **PLAT-001/002** (Phase 3 update): external opening now runs through the
  `vault_platform::ExternalOpener` trait with a Linux xdg-open adapter; the
  silent non-Linux no-op no longer exists (the adapter object always exists).
  Real xdg-open/document behavior is a user manual check on a Linux desktop.
- **PLAT-003**: blur request is Wayland-only (X11 probes Opaque), reported
  through the `AppearanceService` trait.

Everything else — drag-and-drop, watcher, clipboard, settings — is platform-neutral by design.

## Shared since Phase 4 (persistence workflow)

- **SAVE-002/003** (Phase 4 update): the save-plan decision and the guarded
  save moved from the cosmic shell into `vault_app::workflow`
  (`save_plan`, `guarded_save`), executed against the new
  `vault_app::repository::NoteRepository` trait. The production adapter is
  the live `Vault`; tests inject the deterministic `MemoryRepository`
  fake. The cosmic frontend only orchestrates outcomes now.
- **SAVE-004/007** (Phase 4 update): the autosave idle policy
  (`autosave_due`) and the conflict-banner decision application
  (`resolve_conflict`) are shared kernel functions, unit-tested at the
  reducer/workflow level before the UI was touched (plan §"add fake
  repository tests before changing UI").
