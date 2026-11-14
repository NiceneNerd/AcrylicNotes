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
| PANE-* | Reading view | — | — | Existing (reference) | Not started |
| PANE-* | Split mode | — | — | Existing (reference) | Not started |
| EDIT-*, PANE-* | Live mode | — | — | Existing (reference; word/offset math shared via `vault_app::editor` since Phase 6) | Not started |
| NAV-* | Search | — | — | Existing (reference) | Not started |
| NAV-* | Tags/backlinks | — | — | Existing (reference) | Not started |
| NAV-* | Outline | — | — | Existing (reference) | Not started |
| NAV-* | Quick switcher | — | — | Existing (reference; switcher/fuzzy/tags/backlinks/outline/tree view models shared via `vault_app::{navigation,tree,fuzzy}` since Phase 5) | Not started |
| EDIT-* | Find/replace | — | — | Existing (reference; matching shared via `vault_app::editor` since Phase 6) | Not started |
| DLG-* | Dialogs | — | — | Existing (reference) | Not started |
| PLAT-* | Attachments | — | — | Existing (reference) | Not started |
| SET-* | Theme | — | — | Existing (reference) | Not started |
| MISC-* | Glass/backdrop | — | — | Existing (reference) | Not started |
| SET-* | Settings | — | — | Existing (reference) | Not started |
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
| APPWIN-002 | Effect execution against injected services (saves/reloads/external opens) | Done, Linux-tested; p11 services for settings + vault-wide search |
| APPWIN-003 | Message envelope + window-close flush (`UiCommand::SaveTabs`) | Done, Linux-tested |
| APPWIN-004 | File tree view over kernel `tree` + `TreeState` | Done, Linux-tested; rows render headlessly (`tests/ui_headless.rs`) |
| APPWIN-005 | Source editor caret/find math via `vault_app::editor` | Done, Linux-tested |
| APPWIN-006 | Reading-mode plain-text projection | Partial; native element construction is p11 |
| APPWIN-007 | Split layout state in `vault_app::layout` | Done, Linux-tested (kernel) |
| APPWIN-008 | Native window loop, WinUI controls, live adapters | In progress (p11): `Shell` component renders tabs/editor/error line from kernel state and routes clicks back as `WindowsMessage` commands (headless-verified: RecordingRuntime under Wine); `main.rs` window loop, native TreeView, and the editing adapter are next; real-rendering/IME/DPI/pickers/packaging checks remain user-side |
| APPWIN-009 | Cross-target verification (xtask + CI) | Done, runs on Linux: cross-target checks + headless Wine UI tests; real-Windows smoke remains user-side |

## Baseline (Phase 0)

- Linux gates on branch `restructure/cross-platform` at commit `871e868` (parent):
  `cargo fmt --all -- --check` clean; `cargo clippy --workspace -- -D warnings` clean;
  `cargo test --workspace` — 438 passed across 14 suites, 0 failed.
- The Linux frontend is the parity reference: its current behavior is the contract.
