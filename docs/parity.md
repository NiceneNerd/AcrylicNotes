# Parity Matrix

Tracks cross-platform feature parity between the Linux frontend (`app-cosmic`, libcosmic)
and the Windows frontend (`app-windows`, WinUI 3 via windows-reactor).

Every row needs an owner, an automated test, Linux status, Windows status, and manual
evidence where native interaction cannot be fully automated. IDs reference
[feature-inventory.md](feature-inventory.md).

## Status vocabulary

| Status | Meaning |
| --- | --- |
| **Not started** | No Windows implementation. |
| **Shell only** | Control exists but is not connected to shared behavior. |
| **Functional** | Primary path works; edge cases remain. |
| **Parity** | Shared fixtures and platform smoke tests pass. |
| **Verified** | Real Windows manual test passed and packaging path exercised. |

A feature is not "ported" merely because a similar Windows control appears. It is complete
when the same user intent reaches the shared command, produces the same state/effect
decisions, handles failure and cancellation, persists correctly, and passes acceptance
scenarios.

## Matrix

| Parity ID(s) | Area | Owner | Automated test | Linux status | Windows status |
| --- | --- | --- | --- | --- | --- |
| WIN-* | Header/commands | — | — | Existing (reference) | Not started |
| NAV-* | Sidebar sections | — | — | Existing (reference) | Not started |
| TREE-* | File tree | — | — | Existing (reference) | Not started |
| TAB-* | Tabs | — | — | Existing (reference) | Not started |
| EDIT-* | Source editor | — | — | Existing (reference; pure editor text ops shared via `vault_app::editor` since Phase 6) | Not started |
| PANE-* | Reading view | — | — | Existing (reference) | Functional: relative Markdown links render as clickable HyperlinkButtons (`resolve_relative` → `OpenNote` for notes, `OpenExternal::File` for attachments); tags render as tag-panel links (`TagToggled`); footnote refs and outline entries jump via `JumpOutline` (a visible Border marker anchors the block until the toolkit grows a scroll API); real-Windows visual confirmation user-side |
| PANE-* | Split mode | — | — | Existing (reference) | Not started |
| EDIT-*, PANE-* | Live mode | — | — | Existing (reference; word/offset math shared via `vault_app::editor` since Phase 6) | Not started |
| NAV-* | Search | — | — | Existing (reference) | Not started |
| NAV-* | Tags/backlinks | — | — | Existing (reference) | Functional: the shared `NoteIndex` adapter (kernel `vault_app::navigation`, real impl over vault-core's index, `MemoryRepository` fake for tests) feeds the shell's Tags panel (counts, shell-local drill-down) and Links panel (backlinks of the active note), rows open notes through `OpenNote` with index resolution of bare titles; headless-verified (Wine) |
| NAV-* | Outline | — | — | Existing (reference) | Functional: outline entries render in the sidebar panel (indented by level) and clicking one sets the kernel jump anchor (`JumpOutline`), rendered as a Border marker; real scrolling deferred to a toolkit scroll API; headless-verified (Wine) |
| NAV-* | Quick switcher | — | — | Existing (reference; switcher/fuzzy/tags/backlinks/outline/tree view models shared via `vault_app::{navigation,tree,fuzzy}` since Phase 5) | Not started |
| EDIT-* | Find/replace | — | — | Existing (reference; matching shared via `vault_app::editor` since Phase 6) | Not started |
| DLG-* | Dialogs | — | — | Existing (reference) | Not started |
| PLAT-* | Attachments | — | — | Existing (reference) | Not started |
| SET-* | Theme | — | — | Existing (reference) | Functional: kernel `SetTheme` persists the snapshot; the shell maps it to the window's requested theme (`window_visuals`) with system/light/dark buttons, headless-verified (Wine); visual confirmation on real Windows is user-side |
| MISC-* | Glass/backdrop | — | — | Existing (reference) | Functional: kernel `SetGlassEnabled` persists; the shell requests the Mica backdrop when on, headless-verified (Wine) |
| SET-* | Settings | — | — | Existing (reference) | Functional: `FileSettingsStore` (atomic JSON) persists the kernel-owned fields (boot pane, theme, glass) load-modify-store, and the session boots into the persisted snapshot; remaining fields (fonts, margins, tree sort) land with their features |
| PLAT-* | Clipboard | — | — | Existing (reference) | Not started |
| PLAT-* | External links | — | — | Existing (reference; behind `vault_platform::ExternalOpener` since Phase 3) | Not started |
| SAVE-* | Watcher/conflicts | — | — | Existing (reference; save plan, guarded save, autosave policy, and conflict decisions shared via `vault_app::workflow` + `NoteRepository` since Phase 4) | Not started |
| MISC-* | Daily note | — | — | Existing (reference) | Not started |
| STATUS-* | Status bar | — | — | Existing (reference) | Not started |

## APPWIN-* implementation status (Windows frontend)

The matrix above tracks **user-visible feature parity**; its Windows column stays
"Not started" until native controls exist (p11+). This section tracks the state of
`frontends/app-windows` itself (WinUI 3 via windows-reactor, pinned `0.100.0`).
Implementation detail lives in the APPWIN map in
[feature-inventory.md](feature-inventory.md).

**Native Windows compilation of the UI path is type-checked from Linux**
(`cargo run -p xtask -- check-windows-ui` plus cross-target clippy
`-D warnings`, both in CI). **UI logic is additionally exercised headlessly
on every push**: windows-reactor's `RecordingRuntime` runs the same component
code the real WinUI loop runs and records every native command for
assertions — built for `x86_64-pc-windows-gnu` and executed under Wine
(`cargo run -p xtask -- test-windows-ui`, CI + local; Wine and the mingw
cross-linker required). What Wine cannot verify — real WinUI rendering, IME,
DPI, theming, pickers, MSIX packaging — remains user-side on a real Windows
machine and is never claimed via Wine.

| ID | Piece | Status |
| --- | --- | --- |
| APPWIN-001 | Kernel wiring (`WindowsApp` over the shared `App`) | Done, Linux-tested |
| APPWIN-002 | Effect execution against injected services (saves/reloads/external opens/settings persistence/vault search) | Done, Linux-tested: settings persist through the `SettingsStore` service (kernel `SetDefaultPane` -> `PersistSettings`; file adapter in `settings_store.rs`) and the vault-wide search runs the shared engine (`vault_core::search`, 200-hit cap) feeding `SearchCompleted` |
| APPWIN-003 | Message envelope + window-close flush (`UiCommand::SaveTabs`) | Done, Linux-tested |
| APPWIN-004 | File tree view over kernel `tree` + `TreeState` | Done, Linux-tested; rows render headlessly (`tests/ui_headless.rs`) |
| APPWIN-005 | Source editor caret/find math via `vault_app::editor` | Done, Linux-tested |
| APPWIN-006 | Reading-mode plain-text projection | Mostly done: native element construction (`reading_view.rs`) headless-tested and mounted in the shell — the kernel's pane modes (Live/Split/Reading) render the parsed buffer as TextBlocks/HyperlinkButtons; block-level Live editing (plan 11.4) and image rendering remain; link clicks route to the kernel (OpenExternal via the ExternalOpener service, OpenNote for wiki links) |
| APPWIN-007 | Split layout state in `vault_app::layout` | Done, Linux-tested (kernel); rendered by the Windows shell (Source/Split/Live/Reading pane buttons -> SetPaneMode, headless-tested) |
| APPWIN-008 | Native window loop, WinUI controls, live adapters | In progress (p11): `Shell` renders TreeView sidebar, tab bar, pane-mode buttons, source editor, reading view, and error line from kernel state, all headless-verified (RecordingRuntime under Wine); `main.rs` window loop awaits the p11 platform adapters; keyboard routing is blocked by windows-reactor 0.100.0's limited AcceleratorKey set; real-rendering/IME/DPI/pickers/packaging checks remain user-side |
| APPWIN-009 | Cross-target verification (xtask + CI) | Done, runs on Linux: cross-target checks + headless Wine UI tests; real-Windows smoke remains user-side |
| APPWIN-010 | Appearance (window theme, glass backdrop, title, boot pane) | Done, headless-verified (Wine): the shell declares `window_visuals` from the kernel's persisted theme/glass every publication, themes are switched through kernel `SetTheme`/`SetGlassEnabled` (persisted load-modify-store), the session boots into the persisted `default_pane`/sidebar, and the window is titled VaultNotes |
| APPWIN-011 | Vault-index adapter + Tags/Links panels | Done, headless-verified (Wine): the `NoteIndex` trait in `vault_app::navigation` (tag counts/notes, backlinks, wiki-link + folder-relative resolution) is implemented by the live `Vault` (same `Rc` as the repository in `main.rs`) and by the `MemoryRepository` fake; the shell renders the Tags panel (counts, drill-down as shell-local state via `NativeEvent::TagToggled`) and the Links panel (backlinks, empty state), and `OpenNote` resolves bare titles through the index before opening |
| APPWIN-012 | Window layout skeleton (sidebar vs. content) | Done, headless-verified (Wine): the shell's flat vertical stack became a root `Grid` (title-bar row + banner row + body row) over a `SplitView` — the sidebar sits in the pane slot (fixed 250px, Linux parity), the note area in the content slot, the error line became a full-width `InfoBar` banner above the sidebar, and the editor/reading view are wrapped in `ScrollViewer`s. `is_pane_open` follows the kernel's `sidebar_open` and the pane write-back dispatches `SetSidebarVisible`. A WinUI-idiomatic `TitleBar` (content extends into the title bar, `preferred_height(Tall)`, built-in pane toggle writing `SetSidebarVisible`) hosts the toolbar (save, pane-mode switcher, appearance row) in its `Content` slot; `window_visuals.client_size(1180, 760)` matches Linux's initial size. The horizontal tab strip (slice 3), the note-column status bar plus the no-note empty state (slice 4), and the narrow-window fold, rounded sidebar card, and reading gutter (slice 5) have since landed |
| APPWIN-013 | File-tree click routing + scrollable panes | Done, headless-verified (Wine): the native TreeView reports the invoked row's label (its content string), not its key, so clicks on same-named notes from different folders resolved through the index's proximity ranking and could open the wrong note, and folder clicks opened a bogus tab. The shell now labels rows uniquely (a shared basename is shown as its full vault path) and routes every click through a label→row map: a note click opens the exact row clicked, a folder click toggles expansion (shell-local `TreeState`, no tab). The note area and the sidebar became Grids with Auto nav/tab rows and Star body rows — and Split renders editor/reading side by side (Linux's fixed 50/50) — so the ScrollViewers get bounded viewports and every pane scrolls (a StackPanel hands children infinite height, which is why nothing scrolled) |

## Baseline (Phase 0)

- Linux gates on branch `restructure/cross-platform` at commit `871e868` (parent):
  `cargo fmt --all -- --check` clean; `cargo clippy --workspace -- -D warnings` clean;
  `cargo test --workspace` — 438 passed across 14 suites, 0 failed.
- The Linux frontend is the parity reference: its current behavior is the contract.
