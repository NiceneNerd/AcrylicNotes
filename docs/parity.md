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
| EDIT-* | Source editor | — | — | Existing (reference) | Not started |
| PANE-* | Reading view | — | — | Existing (reference) | Not started |
| PANE-* | Split mode | — | — | Existing (reference) | Not started |
| EDIT-*, PANE-* | Live mode | — | — | Existing (reference) | Not started |
| NAV-* | Search | — | — | Existing (reference) | Not started |
| NAV-* | Tags/backlinks | — | — | Existing (reference) | Not started |
| NAV-* | Outline | — | — | Existing (reference) | Not started |
| NAV-* | Quick switcher | — | — | Existing (reference) | Not started |
| EDIT-* | Find/replace | — | — | Existing (reference) | Not started |
| DLG-* | Dialogs | — | — | Existing (reference) | Not started |
| PLAT-* | Attachments | — | — | Existing (reference) | Not started |
| SET-* | Theme | — | — | Existing (reference) | Not started |
| MISC-* | Glass/backdrop | — | — | Existing (reference) | Not started |
| SET-* | Settings | — | — | Existing (reference) | Not started |
| PLAT-* | Clipboard | — | — | Existing (reference) | Not started |
| PLAT-* | External links | — | — | Existing (reference; behind `vault_platform::ExternalOpener` since Phase 3) | Not started |
| SAVE-* | Watcher/conflicts | — | — | Existing (reference) | Not started |
| MISC-* | Daily note | — | — | Existing (reference) | Not started |
| STATUS-* | Status bar | — | — | Existing (reference) | Not started |

## Baseline (Phase 0)

- Linux gates on branch `restructure/cross-platform` at commit `871e868` (parent):
  `cargo fmt --all -- --check` clean; `cargo clippy --workspace -- -D warnings` clean;
  `cargo test --workspace` — 438 passed across 14 suites, 0 failed.
- The Linux frontend is the parity reference: its current behavior is the contract.
