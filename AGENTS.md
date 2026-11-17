# AGENTS.md — Vault Notes (AcrylicNotes)

Guidance for coding agents working in this repository. Read this before touching code,
alongside `HANDOFF.md` and `COMPAT.md`.

## 1. Project identity

**Vault Notes** is a lightweight, Obsidian-compatible Markdown notes app written in Rust.
It opens an existing Obsidian vault and renders notes, wikilinks, tags, footnotes,
callouts, embeds, etc. Graph view and the plugin system are intentionally not planned
as part of the lightweight goals.

Three layers; only the last depends on a GUI toolkit:

1. `vault-core` — scan, watch, index, resolve, frontmatter, atomic writes, CLI.
2. `vault-md` — comrak parse + Obsidian extensions → IR (`Doc`/`Block`/`Inline`). The IR
   is the contract between frontends: an IR→HTML renderer (web) and an IR→widgets
   renderer (GUI) both consume `Doc`.
3. Frontends: `app` (libcosmic GUI, built on Linux and Windows) and `vault-web`
   (axum, server-rendered HTML, no JS). Glass lives in the small `vault-glass` crate
   behind a `Backdrop` trait.

## 2. Repository layout (workspace)

`Cargo.toml` workspace members:

| Member | Purpose |
| --- | --- |
| `crates/vault-app` | GUI-free application kernel (state, commands, effects, view models). |
| `crates/vault-platform` | Platform service contracts (traits + DTOs). |
| `crates/vault-platform-linux` | Linux adapters (xdg-open, Wayland glass probe). |
| `vault-glass` | Backdrop trait + per-OS impls. |
| `vault-core` | UI-free vault. |
| `vault-md` | Markdown pipeline. |
| `app` | libcosmic GUI (network/git deps; Linux and Windows). |
| `vault-web` | Read-only web server. |
| `xtask` | Maintenance tasks. |

`default-members` excludes `app` (libcosmic/iced git deps), so plain `cargo` commands
build/test the UI-free layers only. Use `--workspace` where network/git deps are available.

Other top-level: `docs/`, `tests/fixtures/` (shared mini-vaults), `feature-split-test/`,
`.cargo/config.toml`, `Cross.toml`, `COMPAT.md`, `HANDOFF.md`.

## 3. Architecture rules (do not violate)

- Files are the source of truth. Never re-serialize what the user did not edit. Keep
  frontmatter bytes verbatim (parse a copy). Preserve line endings (CRLF vs LF). Write
  atomically (temp file in same dir, then rename). Detect external modification before
  save (mtime/size/hash).
- Never write into `.obsidian/`. Read it selectively (attachment folder, link format,
  daily-notes settings). Keep app UI state in the OS config dir.
- Index in memory, rebuilt at startup, kept fresh by a `notify` watcher. No database in v1.
- `vault-core` and `vault-md` must stay free of GUI dependencies.
- `vault-app` must never depend on libcosmic, iced, windows, windows-reactor, HWND/COM
  types, or any frontend message enum.
- Platform services cross `vault-platform` traits only: DTOs are plain data, signatures
  are synchronous, every fallible op returns an explicit `Result`.
- Avoid a webview for rendering.

## 4. Build / test / lint commands

Toolchain: Rust 1.100 (no rust-toolchain pin checked in; CI uses `dtolnay/rust-toolchain@stable`).

- Format: `cargo fmt --all -- --check` (CI). The historical gate uses a subset:
  `cargo fmt -p vault-glass -p vault-core -p vault-md -p app -p vault-web`.
- Clippy: `cargo clippy --workspace -- -D warnings`
  (cross-target: `cargo clippy --target x86_64-pc-windows-gnu -p vault-core -p vault-md -p vault-app -p vault-platform --tests -- -D warnings`).
- Tests (UI-free): `cargo test --workspace --no-fail-fast` (default members exclude `app`).
- Tests including GUI: `cargo test -p app` (needs network/git).
- Windows cross-compile shared crates:
  `cargo check --tests --target x86_64-pc-windows-gnu -p vault-core -p vault-md -p vault-app -p vault-platform`.

xtask (`cargo run -p xtask -- <cmd>`):

- `check-shared` / `test-shared` / `test-linux` / `build-linux`
- `test-windows-core [target]`
- `test-windows-native [target]` (real Windows over SSH)
- `parity-report`

Binaries:

- `cargo run -p vault-core --bin vault -- index|resolve|backlinks <dir> ...`
- `cargo run -p vault-web -- [VAULT_PATH] [--port N] [--bind ADDR] [--token T] [--no-auth]`
- `cargo run -p app` (libcosmic GUI; Linux and Windows)

Windows cross tooling: `cross` (Docker, `:main` windows-gnu image) or local mingw + wine
(`.cargo/config.toml` sets linker/runner). `Cross.toml` pins the windows-gnu image to `:main`.

## 5. Platform notes

- Linux: libcosmic with `wayland` feature; glass via Wayland `ext-background-effect-v1`
  (compositor-dependent; X11 probes Opaque).
- Windows: the same libcosmic app builds with `winit` and `accesskit` dropped;
  `window-vibrancy` Mica/Acrylic/blur; requires Windows 11 for glass.
- macOS: untested (no machine/SDK).
- libcosmic pinned commit `95236a944bfd51c63a2fc65fe50a592c47168d40` (2026-10-27);
  vendors iced (pinned submodule). Per-target feature selection: `wayland` on Linux,
  `winit` elsewhere.
- Image decoding: vendored iced built with `image-without-codecs`; remote images are text
  placeholders; local raster read from disk with a per-path `Handle` cache.
- Key versions: Rust 1.100, comrak 0.59, axum 0.8, nucleo 0.5.

## 6. Testing conventions

- Test-first/evidence-first: every behavior claim needs a test or a command output;
  otherwise label it as an assumption in `COMPAT.md`.
- Run `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` before reporting done.
- Fixtures in `tests/fixtures/` are golden data: copy to a temp dir before mutating;
  changing a fixture requires updating golden expectations in the same commit with a rationale.
- `vault-md/tests/golden/` holds input `.md` → expected `.html` pairs; regenerate with
  `UPDATE_EXPECT=1`.
- The NFD-filename test is `#[cfg_attr(windows, ignore)]` (Wine rejects combining chars).
- Windows builds the same libcosmic GUI; cross-target checks run under `cargo
  xtask test-windows-core`. Real rendering/IME/DPI/packaging verification on
  Windows is user-side only.

## 7. Configuration & paths

- App settings: `$XDG_CONFIG_HOME/vault-handoff/settings.json` (pretty, atomic tmp+rename,
  corrupt/missing → defaults, unknown keys ignored, per-change load-modify-store).
- Vault's `.obsidian/daily-notes.json` drives daily notes; `.obsidian/app.json`
  `strictLineBreaks` is honored at render time.
- `last_vault` is restored on startup if the dir exists.
- `vault-web` defaults to the GUI's last-used vault.

## 8. Compatibility / behavior reference

- `COMPAT.md` — confirmed Obsidian behaviors vs. assumptions (footnote numbering, tag
  indexing, ambiguous basename resolution, aliases don't resolve links, escaped wikilinks,
  no links in math, raw HTML literal, block refs, tag boundaries, heading anchors,
  math vs currency, sanitization, embed depth 3, etc.).
- `HANDOFF.md` — original architecture decisions, milestones M0–M6, working rules.

## 9. Agent-specific guidance

- Prefer editing shared crates (`vault-app`, `vault-core`, `vault-md`, `vault-platform`) so
  both frontends benefit; frontend shells should only orchestrate.
- When adding behavior, add to `vault_app` view models/reducer and update the shell.
- Keep the `vault-md` IR stable; renderers consume `Doc`.
- Web security: every user-controlled path goes through the single guard (`..`/dotfile
  rejection + canonicalize + must stay in vault root); CSP + `nosniff`; raw HTML through
  ammonia sanitization.
- Link rewriting on rename uses `vault-md::link_spans` / `rewrite_link_targets`
  (span-preserving; skips frontmatter, code, math, raw HTML).
- Do not claim native Windows verification via Wine; real-Windows checks are user-side.
- Never modify files in a test vault except through the app's own save path under test.
- When blocked on something only the user can check (real Obsidian behavior, a Windows/macOS
  machine), stop and list the exact question.