//! Workspace maintenance xtask.
//!
//! Run with: `cargo run -p xtask -- <command>` (or `cargo xtask <command>`
//! once installed with `cargo install --path xtask`).
//!
//! Commands:
//!   check-shared        cargo check on every UI-free workspace member
//!   test-shared         cargo test --no-fail-fast on every UI-free member
//!   test-linux          same as test-shared (Linux native)
//!   build-linux         cargo build on every UI-free member
//!   test-windows-core   cross-compile check of the platform-neutral shared
//!                       crates (vault-core, vault-md, vault-app, vault-platform)
//!                       for x86_64-pc-windows-gnu (target must be installed)
//!   check-windows-ui    cross-compile check of the app-windows frontend
//!                       (cfg(windows) code incl. the pinned windows-reactor)
//!                       for x86_64-pc-windows-gnu (target must be installed)
//!   test-windows-ui     build app-windows UI tests for x86_64-pc-windows-gnu
//!                       (target must be installed) and run them under Wine:
//!                       headless windows-reactor RecordingRuntime tests, no
//!                       Windows machine needed (Wine + mingw required)
//!   parity-report       summarize docs/parity.md matrix rows by Windows status

use std::process::{exit, Command};

const APP_CRATE: &str = "app";

/// The GUI-free workspace members (the default-members set): the layers that
/// must compile and test without libcosmic, iced, windows, or windows-reactor.
/// Documented here; `check-shared`/`test-shared` enforce it via
/// `cargo ... --workspace --exclude app`.
#[allow(dead_code)]
const SHARED_CRATES: &[&str] = &[
    "vault-core",
    "vault-md",
    "vault-app",
    "vault-platform",
    "vault-platform-linux",
    "vault-glass",
    "vault-web",
];

/// Platform-neutral shared crates: these are the ones expected to
/// cross-compile for x86_64-pc-windows-gnu. vault-platform-linux and
/// vault-glass are Linux-only and excluded from the Windows target.
const WINDOWS_SHARED_CRATES: &[&str] = &["vault-core", "vault-md", "vault-app", "vault-platform"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("check-shared") => run_cargo(&["check", "--workspace", "--exclude", APP_CRATE]),
        Some("test-shared") => run_cargo(&[
            "test",
            "--workspace",
            "--no-fail-fast",
            "--exclude",
            APP_CRATE,
        ]),
        Some("test-linux") => run_cargo(&[
            "test",
            "--workspace",
            "--no-fail-fast",
            "--exclude",
            APP_CRATE,
        ]),
        Some("build-linux") => run_cargo(&["build", "--workspace", "--exclude", APP_CRATE]),
        Some("test-windows-core") => {
            let target = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("x86_64-pc-windows-gnu");
            let mut cmd = vec!["check", "--tests", "--target", target];
            for crate_name in WINDOWS_SHARED_CRATES {
                cmd.push("-p");
                cmd.push(crate_name);
            }
            run_cargo(&cmd);
        }
        Some("check-windows-ui") => {
            let target = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("x86_64-pc-windows-gnu");
            run_cargo(&["check", "--target", target, "-p", "app-windows"]);
        }
        Some("test-windows-ui") => {
            let target = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("x86_64-pc-windows-gnu");
            run_windows_ui_tests(target);
        }
        Some("parity-report") => parity_report(),
        Some("help") | Some("--help") | Some("-h") => print_usage(),
        Some(other) => {
            eprintln!("error: unknown xtask command `{other}`");
            print_usage();
            exit(2);
        }
        None => {
            print_usage();
            exit(2);
        }
    }
}

fn run_cargo(cargo_args: &[&str]) -> ! {
    run_cargo_env(cargo_args, &[])
}

/// Runs cargo with extra environment variables. Used by `test-windows-ui` to
/// set the `CARGO_TARGET_<triple>_RUNNER` shim so cargo itself launches the
/// windows-gnu test binaries under Wine (cargo cannot exec PE binaries
/// natively on Linux: "Exec format error").
fn run_cargo_env(cargo_args: &[&str], envs: &[(String, String)]) -> ! {
    let mut status = Command::new("cargo");
    status.args(cargo_args);
    for (key, value) in envs {
        status.env(key, value);
    }
    let status = status.status().expect("cargo not found");
    exit(status.code().unwrap_or(1));
}

/// Builds and runs the app-windows UI tests for the Windows target.
///
/// On a non-Windows host the test binaries run under Wine (installed
/// separately): the `CARGO_TARGET_*_RUNNER` env is set to `wine` unless the
/// caller already provides a runner or a `WINE` executable override. On a
/// real Windows host cargo runs the tests natively and no runner is needed.
fn run_windows_ui_tests(target: &str) -> ! {
    let runner_key = format!(
        "CARGO_TARGET_{}_RUNNER",
        target.replace('-', "_").to_uppercase()
    );
    let mut envs: Vec<(String, String)> = Vec::new();
    if std::env::var_os(&runner_key).is_none() && !cfg!(target_os = "windows") {
        let wine = std::env::var("WINE").unwrap_or_else(|_| String::from("wine"));
        envs.push((runner_key, wine));
        // Keep the Wine output readable: only the test harness's lines.
        let wine_debug = std::env::var("WINEDEBUG").unwrap_or_else(|_| String::from("-all"));
        envs.push((String::from("WINEDEBUG"), wine_debug));
    }
    run_cargo_env(
        &[
            "test",
            "--target",
            target,
            "-p",
            "app-windows",
            "--no-fail-fast",
        ],
        &envs,
    );
}

fn print_usage() {
    println!("Usage: cargo run -p xtask -- <command>");
    println!();
    println!("Commands:");
    println!("  check-shared          cargo check on every UI-free workspace member");
    println!("  test-shared           cargo test --no-fail-fast on every UI-free member");
    println!("  test-linux            same as test-shared (Linux native)");
    println!("  build-linux           cargo build on every UI-free member");
    println!(
        "  test-windows-core [target]  cross-compile check for x86_64-pc-windows-gnu \
         (default) of vault-core, vault-md, vault-app, vault-platform"
    );
    println!(
        "  check-windows-ui [target]   cross-compile check of app-windows \
         (cfg(windows) code + windows-reactor pin) for x86_64-pc-windows-gnu (default)"
    );
    println!(
        "  test-windows-ui [target]    build app-windows UI tests for \
         x86_64-pc-windows-gnu (default) and run them under Wine \
         (headless RecordingRuntime lane)"
    );
    println!("  parity-report         summarize docs/parity.md matrix rows by Windows status");
    println!();
    println!("Example: cargo run -p xtask -- test-windows-core x86_64-pc-windows-gnu");
}

fn parity_report() {
    let md = match std::fs::read_to_string("docs/parity.md") {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: cannot read docs/parity.md: {e}");
            exit(1);
        }
    };
    let mut total = 0usize;
    let mut by_status: Vec<(String, usize)> = Vec::new();
    let mut not_started: Vec<String> = Vec::new();

    // The matrix table rows look like:
    // | WIN-* | Header/commands | — | — | Existing (reference) | Not started |
    for line in md.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || trimmed.contains("Parity ID(s)") || trimmed.contains("---")
        {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .split('|')
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        if cells.len() < 6 {
            continue;
        }
        let windows_status = cells[5].to_string();
        total += 1;
        if let Some(pos) = by_status.iter().position(|(s, _)| s == &windows_status) {
            by_status[pos].1 += 1;
        } else {
            by_status.push((windows_status.clone(), 1));
        }
        if windows_status == "Not started" {
            not_started.push(cells[0].to_string());
        }
    }

    println!("Parity report (docs/parity.md)");
    println!("==============================");
    println!("matrix rows: {total}");
    for (status, count) in &by_status {
        println!("  {status}: {count}");
    }
    if !not_started.is_empty() {
        println!();
        println!("Windows status 'Not started':");
        for id in &not_started {
            println!("  - {id}");
        }
    }
    println!();
    println!("Legend: Not started / Shell only / Functional / Parity / Verified");
}
