# Acrylic Notes

![Rust](https://img.shields.io/badge/language-Rust-orange?style=flat-square)
![Status](https://img.shields.io/badge/status-vibe--coded--slop-red?style=flat-square)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-blue?style=flat-square)

## ⚠️🛑🚨 VIBE-CODED SLOP WARNING 🚨🛑⚠️

This software is **almost entirely** produced by LLMs. As I have long said, AI agents are very curious tools: it's like having a highly talented assistant with ADD and substance abuse issues. So why do I use them? Well, they can accomplish quite a bit, even if poorly, and my condition of life no longer permits me quite so much time for involved programming projects as I once had. In particular, I have an excellent wife, seven children, several chickens (fewer every week, though), and an honestly-more-than-full-time job as a humanities teacher at a classical Christian school. I love this life and render thanks to God for it daily, but it does put limits on how much code I can crank out.

Thus, clankers to the rescue. They can do an awful lot in the time it would take for me to research and implement new projects on my own. I can't say the quality is great (not that the quality of my own work has always been stellar; just look at [BCML](https://github.com/NiceneNerd/BCML)), but I can say that I usually know enough about what's going on to make sure it's not a **complete** mess.

Do be suspicious of this work. Feel free to add human review. You don't have to trust the clankers. My only assurance is that, if I bothered posting a README, it probably means the output was good enough to satisfy my own daily use.

---

### What is this?

Acrylic Notes is a lightweight, Obsidian-compatible Markdown notes app written in Rust. It opens an existing Obsidian vault and renders notes, wikilinks, tags, footnotes, callouts, embeds, etc.

Specifically chosen exclusions: **graph view, plugin system.**

Three layers; only the last depends on a GUI toolkit:

1. `vault-core` — scan, watch, index, resolve, frontmatter, atomic writes, CLI. UI-free.
2. `vault-md` — comrak parse + Obsidian extensions → IR (`Doc`/`Block`/`Inline`). The IR is the contract between frontends.
3. Frontends: `app` (libcosmic GUI), `vault-web` (axum, server-rendered HTML, no JS), `frontends/app-windows` (WinUI 3).

---

### Key features

- **Obsidian-compatible vault reader.** Points at your existing `.md` vault. Does not touch `.obsidian/`.
- **Wikilinks, tags, footnotes, callouts, embeds.** Rendered through a shared intermediate representation (`Doc`/`Block`/`Inline`).
- **Atomic writes.** Temp file in same directory, then rename. Frontmatter bytes preserved verbatim. Line endings preserved (CRLF vs LF).
- **In-memory index.** Rebuilt at startup, kept fresh by a `notify` watcher. No database in v1.
- **No graph view.** Out of scope.
- **No plugin system.** Out of scope.
- **Settings** at `$XDG_CONFIG_HOME/vault-handoff/settings.json` (pretty, atomic tmp+rename, corrupt/missing → defaults, unknown keys ignored).
- **CLI:** `cargo run -p vault-core --bin vault -- index|resolve|backlinks <dir> ...`
- **Web server:** `cargo run -p vault-web -- [VAULT_PATH] [--port N] [--bind ADDR] [--token T] [--no-auth]`

---

### Architecture

- Files are the source of truth. Never re-serialize what the user did not edit.
- `vault-core` and `vault-md` stay free of GUI dependencies.
- `vault-app` never depends on libcosmic, iced, windows, windows-reactor, HWND/COM types, or any frontend message enum.
- Platform services cross `vault-platform` traits only: DTOs are plain data, signatures are synchronous, every fallible op returns an explicit `Result`.
- Avoid a webview for rendering.

---

### Build / test / lint commands

Toolchain: Rust 1.100.

```
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace --no-fail-fast
cargo test -p app-windows
cargo run -p app-windows
cargo run -p vault-web -- [VAULT_PATH]
```

Note: `cargo test --workspace --no-fail-fast` fails for two pre-existing, unrelated reasons: `vault-md --test golden` (CRLF vs LF golden mismatch) and `vault-platform-linux --lib` (`true` program not found on Windows). Use default members instead; `app` (libcosmic) fails to compile due to a pre-existing `accesskit_winit` git dependency error.

Windows cross-compile: `cross` (Docker, `:main` windows-gnu image) or local mingw + wine. `.cargo/config.toml` sets linker/runner.
