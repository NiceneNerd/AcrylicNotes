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
//!   test-windows-native run the shared-crate tests on a real Windows
//!                       machine over SSH (a local VM, e.g. QEMU):
//!                       catches Wine-vs-Windows divergence the Wine lane
//!                       cannot. `test-windows-native ui` additionally
//!                       builds and runs app-windows (needs Windows 11:
//!                       windows-reactor links Win11-only APIs). Configure
//!                       with WINDOWS_VM_HOST / WINDOWS_VM_USER /
//!                       WINDOWS_VM_KEY
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
        Some("test-windows-native") => {
            // `test-windows-native [target] [ui]`: the first positional arg,
            // when present, is either the target triple or the literal
            // `ui` (meaning "include app-windows on the default target").
            let (target, include_ui) = match args.get(2).map(String::as_str) {
                Some("ui") => ("x86_64-pc-windows-gnu", true),
                Some(target) => (target, args.get(3).is_some_and(|arg| arg == "ui")),
                None => ("x86_64-pc-windows-gnu", false),
            };
            run_windows_native_tests(target, include_ui);
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

/// Builds test binaries and runs them on a real Windows machine over SSH.
///
/// The Wine lane (`test-windows-ui`) runs the app-windows UI binaries on
/// every push without Windows; this lane runs the shared-crate tests on
/// actual Windows (a local VM is enough) so Wine shims cannot hide a
/// divergence. It found one on its first run: windows-reactor 0.100.0's
/// WinUI bindings hard-link `AddPackageDependency`/`TryCreatePackageDependency`
/// from `api-ms-win-appmodel-runtime-l1-1-5.dll`, which exists only on
/// Windows 11 — so app-windows binaries cannot even load on Windows 10.
/// The UI crate is therefore behind the optional `ui` argument (needs a
/// Windows 11 guest); the shared crates run everywhere.
///
/// Configuration (env):
///
/// - `WINDOWS_VM_HOST` (default `192.168.122.51`, the QEMU user network)
/// - `WINDOWS_VM_USER` (default `nn`)
/// - `WINDOWS_VM_KEY`  (default `~/.ssh/win10_vaultnotes`)
///
/// The guest must run OpenSSH Server and authorize the key. Binaries are
/// copied into the user's profile (whose parent always exists, so no
/// remote mkdir) and executed by absolute path; system-DLL-only imports
/// mean no runtime DLLs need copying.
/// Builds test binaries and runs them on a real Windows machine over SSH.
///
/// The Wine lane (`test-windows-ui`) runs the app-windows UI binaries on
/// every push without Windows; this lane runs them on actual Windows so
/// Wine shims cannot hide a divergence. It found one on its first run:
/// windows-reactor 0.100.0's WinUI bindings hard-link
/// `AddPackageDependency`/`TryCreatePackageDependency` from
/// `api-ms-win-appmodel-runtime-l1-1-5.dll`, which exists only on
/// Windows 11 — so app-windows binaries cannot even load on Windows 10.
/// The UI crate is therefore behind the optional `include_ui` flag.
///
/// Configuration (env):
///
/// - `WINDOWS_VM_HOST` (default `192.168.122.51`, the QEMU user network)
/// - `WINDOWS_VM_USER` (default `nn`)
/// - `WINDOWS_VM_KEY`  (default `~/.ssh/win10_vaultnotes`)
///
/// The guest must run OpenSSH Server and authorize the key. Binaries are
/// copied into the user's profile (whose parent always exists, so no
/// remote mkdir) and executed by absolute path; system-DLL-only imports
/// mean no runtime DLLs need copying.
/// Builds test binaries and runs them on a real Windows machine over SSH.
///
/// The Wine lane (`test-windows-ui`) runs the app-windows UI binaries on
/// every push without Windows; this lane runs them on actual Windows so
/// Wine shims cannot hide a divergence. It found one on its first run:
/// windows-reactor 0.100.0's WinUI bindings hard-link
/// `AddPackageDependency`/`TryCreatePackageDependency` from
/// `api-ms-win-appmodel-runtime-l1-1-5.dll`, which exists only on
/// Windows 11 — so app-windows binaries cannot even load on Windows 10.
/// The UI crate is therefore behind the optional `include_ui` flag.
///
/// Configuration (env):
///
/// - `WINDOWS_VM_HOST` (default `192.168.122.51`, the QEMU user network)
/// - `WINDOWS_VM_USER` (default `nn`)
/// - `WINDOWS_VM_KEY`  (default `~/.ssh/win10_vaultnotes`)
///
/// The guest must run OpenSSH Server and authorize the key. Binaries are
/// copied into the user's profile (whose parent always exists, so no
/// remote mkdir) and executed by absolute path; system-DLL-only imports
/// mean no runtime DLLs need copying.
///
/// Note: the remote target uses the inline `user@host` form because
/// scp's `-o User=` option breaks its CLI parsing; ssh_opts carries only
/// key/strict-host/connect-timeout options.
fn run_windows_native_tests(target: &str, include_ui: bool) -> ! {
    let host = std::env::var("WINDOWS_VM_HOST").unwrap_or_else(|_| String::from("192.168.122.51"));
    let user = std::env::var("WINDOWS_VM_USER").unwrap_or_else(|_| String::from("nn"));
    let key = std::env::var("WINDOWS_VM_KEY").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/root"));
        format!("{home}/.ssh/win10_vaultnotes")
    });
    let ssh_opts = [
        "-i",
        key.as_str(),
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
    ];
    // Inline `user@host` form: scp's `-o User=` breaks its CLI parsing.
    let remote = format!("{user}@{host}");

    // 1. Build the test harnesses and collect their paths from cargo's JSON.
    //    Only `"test":true` artifacts are harnesses; plain bin artifacts
    //    (like the app-windows skeleton) would exit non-zero by design.
    let mut crates: Vec<&str> = WINDOWS_SHARED_CRATES.to_vec();
    if include_ui {
        crates.push("app-windows");
    }
    let mut build = vec!["test", "--target", target, "--no-run", "--message-format=json"];
    for crate_name in &crates {
        build.push("-p");
        build.push(crate_name);
    }
    println!("== building test harnesses for {target} ({}) ==", crates.join(", "));
    let output = Command::new("cargo")
        .args(&build)
        .output()
        .expect("cargo not found");
    if !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        exit(output.status.code().unwrap_or(1));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let executables: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("\"test\":true"))
        .filter_map(|line| {
            let at = line.find("\"executable\":\"")? + "\"executable\":\"".len();
            let rest = &line[at..];
            let end = rest.find('"')?;
            let exe = &rest[..end];
            // Test harnesses are `<crate>-<hex-hash>.exe`; the crate's bin
            // (`app-windows.exe`) ends in a non-hex segment and is not a
            // test target (its main exits non-zero by design).
            let stem = exe.trim_end_matches(".exe");
            let last = stem.split('-').last().unwrap_or("");
            if !last.is_empty() && last.chars().all(|c| c.is_ascii_hexdigit()) {
                Some(exe)
            } else {
                None
            }
        })
        .collect();
    if executables.is_empty() {
        eprintln!("error: cargo reported no test executables");
        exit(1);
    }

    // 2. Stage fixture-based test data next to the executables. The
    //    fixture tests fall back to `<exe_dir>/vaultnotes-fixtures/...`
    //    when their compile-time manifest path is absent (another
    //    machine). PowerShell creates intermediate dirs (cmd's mkdir has
    //    no -p), then `scp -r src/. dst/` avoids nesting on re-runs.
    let fixtures_remote = format!("vaultnotes-fixtures/vault-md-tests");
    let mkdir_cmd = format!(
        "powershell -Command \"New-Item -ItemType Directory -Path 'C:\\\\Users\\\\{user}\\\\{fixtures_remote}' -Force\""
    );
    let _ = Command::new("ssh")
        .args(&ssh_opts)
        .arg(remote.as_str())
        .arg(&mkdir_cmd)
        .status();
    // Stage both fixture trees next to the executables.
    for (src, dst) in [("vault-md/tests", "vault-md-tests"), ("tests/fixtures", "fixtures")] {
        let fixtures_remote = format!("vaultnotes-fixtures/{dst}");
        let fixtures = Command::new("scp")
            .args(&ssh_opts)
            .arg("-r").arg(format!("{src}/."))
            .arg(format!("{remote}:{fixtures_remote}/"))
            .status()
            .expect("scp not found");
        if !fixtures.success() {
            eprintln!("error: staging the {src} test fixtures failed");
            exit(1);
        }
    }

    // 3. Copy each binary into the guest's profile and run it there.
    //    Destination names are simple sequential names (vnt_1.exe, etc.)
    //    because the Windows OpenSSH SFTP server refuses certain cargo
    //    hash-based filenames (e.g. vault_core-cfeb8305af893f9e.exe) with
    //    "dest open ... Failure" even when no such file exists.
    let mut failed = false;
    for (i, exe) in executables.iter().enumerate() {
        let name = format!("vnt_{}.exe", i + 1);
        let remote_path = format!("C:/Users/{user}/{name}");

        println!("== copying {} ==", name);
        let scp = Command::new("scp")
            .args(&ssh_opts)
            .arg(exe)
            .arg(format!("{remote}:{remote_path}"))
            .status()
            .expect("scp not found");
        if !scp.success() {
            eprintln!("error: scp of {name} failed");
            exit(1);
        }

        println!("== running {name} on {remote} ==");
        let run = Command::new("ssh")
            .args(&ssh_opts)
            .arg(remote.as_str())
            .arg(&remote_path)
            .output()
            .expect("ssh not found");
        let stdout = String::from_utf8_lossy(&run.stdout);
        let stderr = String::from_utf8_lossy(&run.stderr);
        print!("{stdout}");
        eprint!("{stderr}");
        let has_result = stdout.contains("test result:");
        if !run.status.success() || stdout.contains("FAILED") || !has_result {
            failed = true;
        }
    }

    if failed {
        eprintln!("error: the native Windows lane reported failures");
        exit(1);
    }
    println!("native Windows lane: all test binaries passed");
    exit(0);
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
    println!(
        "  test-windows-native [target] [ui]  run the shared-crate tests on a \
         real Windows machine over SSH (WINDOWS_VM_HOST/USER/KEY). Pass the \
         target triple first, or `ui` alone to include app-windows on the \
         default target (app-windows needs Windows 11)."
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
