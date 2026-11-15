# Acrylic Notes

![Rust](https://img.shields.io/badge/language-Rust-orange?style=flat-square)
![Status](https://img.shields.io/badge/status-vibe--coded--slop-red?style=flat-square)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-blue?style=flat-square)

## ⚠️🛑🚨 VIBE-CODED SLOP WARNING 🚨🛑⚠️

**THIS SOFTWARE WAS VIBE-CODED.**

It was written by **clankers** (LLMs), not by a human being with a functioning prefrontal cortex. A classical school teacher with seven children and zero free time fed prompts into a machine and this came out the other end. It compiles. Probably. Maybe. We do not know why. It might format your hard drive. It might send your grocery list to Richard Hooker. It might spontaneously generate a 481,000-word *Lord of the Rings* context-window measurement unit (the **LOTRL**) and use it to judge your life choices.

**You have been warned.** Use at your own risk. Back up your vault. Do not store your will, your tax returns, or your only copy of the family Christmas letter in here. If it breaks, you get to keep both pieces.

---

### What is this?

Acrylic Notes is a lightweight, Obsidian-compatible Markdown notes app written in Rust. It opens an existing Obsidian vault and renders notes, wikilinks, tags, footnotes, callouts, embeds, etc.

Out of scope for v1: **graph view, plugin system.** Those are fancy features for people who have time. Caleb does not have time. Caleb is a classical school teacher with 7 children and has no time to code himself anymore, so he just makes clankers do it.

Live preview was implemented on the COSMIC side and is no longer listed as out of scope.

Three layers; only the last depends on a GUI toolkit:

1. `vault-core` — scan, watch, index, resolve, frontmatter, atomic writes, CLI. UI-free.
2. `vault-md` — comrak parse + Obsidian extensions → IR (`Doc`/`Block`/`Inline`). The IR is the contract between frontends.
3. Frontends: `app` (libcosmic GUI), `vault-web` (axum, server-rendered HTML, no JS), `frontends/app-windows` (WinUI 3).

---

### The author, in vibe

The person behind this writes like a scholastic theologian with a Twitter brain: precise and earnest one moment, then slipping in a "TBH" or a dad joke the next. He teaches at a classical Christian school, is married, has seven children, runs cross-country with at least one of them, once had strong feelings about a Demi Lovato song, believes Thomas Aquinas would endorse baths for sadness, and considers footnotes a side stage for chatty self-contradiction. He writes late at night, signs off with C-adverbs, and refers to students by last names as a coping mechanism. He also hates BCML, loves Dr. Elmo beyond "Grandma Got Run Over by a Reindeer," and thinks angels would toddler-parent sinless children off cliffs before the Fall.

None of those specifics matter for using this app. What matters is that he has seven children, a full-time teaching job, and absolutely no time to code, so he outsources the work to clankers and then pretends he supervised.

---

### Key features (accurate, no hype)

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

### Architecture (do not violate)

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

---

### Roadmap (slices)

- Slice 3 (done): custom horizontal tab strip for the WinUI shell.
- Slice 4 (next): status bar via `vault_app::editor::count_words` at the bottom of the note column only, plus empty states.
- Slice 5 (polish): 480px narrow fold via `context.on_window_size(WindowSize)`, sidebar card rounding/alpha, reading margins, theme brushing.
- Images: local vault images via `EncodedImage::source_data`.

---

### The clanker disclosure

This README, like most of the code, was produced by an LLM at the direction of a man with seven children and a full-time teaching job. If it sounds overly enthusiastic, over-structured, full of emojis, and slightly unhinged — that is the Claude writing style being parodied, not an accident. The project itself is real. The vault is real. The rooster is deceased.

---

*"I rely not so much on justifying myself as begging for divine mercy and counting on a justification based on a righteousness extra nos."*
— The author, probably
