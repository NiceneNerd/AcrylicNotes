# Vault Notes (AcrylicNotes)

A lightweight, Obsidian-compatible Markdown notes app written in Rust. It opens
an existing Obsidian vault and renders notes, wikilinks, tags, footnotes,
callouts, embeds, and more — with frosted-glass ("glass") window backdrops on
Linux (COSMIC/Wayland) and Windows (Mica/Acrylic).

Graph view and the plugin system are intentionally not planned as part of the
lightweight goals.

# ⚠️🛑🚨 VIBE-CODED SLOP WARNING 🚨🛑⚠️

This software is **almost entirely** produced by LLMs. As I have long said, AI agents are very curious tools: it's like having a highly talented assistant with ADD and substance abuse issues. So why do I use them? Well, they can accomplish quite a bit, even if poorly, and my condition of life no longer permits me quite so much time for involved programming projects as I once had. In particular, I have an excellent wife, seven children, several chickens (fewer every week, though), and an honestly-more-than-full-time job as a humanities teacher at a classical Christian school. I love this life and render thanks to God for it daily, but it does put limits on how much code I can crank out.

Thus, clankers to the rescue. They can do an awful lot in the time it would take for me to research and implement new projects on my own. I can't say the quality is great (not that the quality of my own work has always been stellar; just look at [BCML](https://github.com/NiceneNerd/BCML)), but I can say that I usually know enough about what's going on to make sure it's not a **complete** mess.

Do be suspicious of this work. Feel free to add human review. You don't have to trust the clankers. My only assurance is that, if I bothered posting a README, it probably means the output was good enough to satisfy my own daily use.

## Architecture

Three layers; only the last depends on a GUI toolkit:

1. **`vault-core`** — UI-free vault: scan, file watching (`notify`), in-memory
   index, wikilink resolution, frontmatter handling, atomic/safe writes, CLI.
2. **`vault-md`** — comrak parse plus Obsidian extensions, lowered into a
   frontend-neutral IR (`Doc`/`Block`/`Inline`). The IR is the contract between
   frontends: an IR→HTML renderer (web) and an IR→widgets renderer (GUI).
3. **Frontends**:
   - `app` — libcosmic GUI, built on Linux (Wayland glass via
     `ext-background-effect-v1`) and Windows (Mica/Acrylic via
     `window-vibrancy`; build with `accesskit` dropped)
   - `vault-web` — read-only axum web server (server-rendered HTML, no JS, no webview)

Supporting crates: `vault-glass` (a `Backdrop` trait with per-OS impls),
`crates/vault-app` (GUI-free application kernel: state, commands, view models),
`crates/vault-platform` + `crates/vault-platform-linux` (platform service
contracts as traits + plain-data DTOs), and `xtask` (maintenance tasks).

### Core principles

- **Files are the source of truth.** Never re-serialize what the user did not
  edit: frontmatter bytes are kept verbatim, line endings (CRLF/LF) preserved,
  writes are atomic (temp file + rename), and external modification is detected
  before saving (mtime/size/hash).
- **Never write into `.obsidian/`** — read it selectively (attachment folder,
  link format, daily-notes settings). App UI state lives in the OS config dir.
- **Index in memory**, rebuilt at startup, kept fresh by a watcher. No database.
- **No webview** for rendering.

## Workspace layout

| Member | Purpose |
| --- | --- |
| `crates/vault-app` | GUI-free application kernel (state, commands, effects, view models) |
| `crates/vault-platform` | Platform service contracts (traits + DTOs) |
| `crates/vault-platform-linux` | Linux adapters (xdg-open, Wayland glass probe) |
| `vault-glass` | `Backdrop` trait + per-OS implementations |
| `vault-core` | UI-free vault (scan/watch/index/resolve/save/CLI) |
| `vault-md` | Markdown pipeline: comrak + Obsidian extensions → IR + HTML renderer |
| `app` | libcosmic GUI |
| `vault-web` | Read-only web server |
| `xtask` | Maintenance tasks (cross-tests, parity report) |

`default-members` excludes `app` (libcosmic/iced git dependencies), so plain
`cargo` commands build and test the UI-free layers only. Use `--workspace` where
network/git dependencies are available.

## Requirements

- Rust 1.100 (stable toolchain)
- Linux GUI: libcosmic with the `wayland` feature (pinned commit); glass on
  Wayland is compositor-dependent (X11 probes Opaque)
- Windows GUI: the same libcosmic app builds on Windows (with `winit` and
  `accesskit` dropped); requires Windows 11 for Mica/Acrylic glass
- Key dependency versions: comrak 0.59, axum 0.8, nucleo 0.5

## Building and running

```sh
# UI-free layers (default members)
cargo build
cargo test --workspace --no-fail-fast

# GUI app (needs network for git deps)
cargo run -p app                 # libcosmic GUI (Linux and Windows)

# Read-only web server (defaults to the GUI's last-used vault)
cargo run -p vault-web -- [VAULT_PATH] [--port N] [--bind ADDR] [--token T] [--no-auth]

# CLI
cargo run -p vault-core --bin vault -- index|resolve|backlinks <dir> ...
```

Formatting and linting:

```sh
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
```

Windows cross-compilation of the shared crates uses `cross` (Docker) or local
mingw + Wine; see `.cargo/config.toml`, `Cross.toml`, and the `xtask` commands
(`cargo run -p xtask -- check-shared|test-shared|test-linux|build-linux|test-windows-core|test-windows-native`).

## Compatibility

- `COMPAT.md` — confirmed Obsidian behaviors (verified against real Obsidian)
  vs. documented assumptions: footnote numbering, tag indexing, ambiguous
  basename resolution, aliases, escaped wikilinks, embed depth, sanitization,
  and more.
- `HANDOFF.md` — original architecture decisions and milestones M0–M6.
- `AGENTS.md` — guidance for coding agents working in this repository.

## Testing

- Fixtures in `tests/fixtures/` are golden data — copy to a temp dir before
  mutating.
- `vault-md/tests/golden/` holds input `.md` → expected `.html` pairs;
  regenerate with `UPDATE_EXPECT=1`.
- Headless Windows UI tests run under Wine; real rendering/IME/DPI/packaging
  verification on Windows is user-side.
