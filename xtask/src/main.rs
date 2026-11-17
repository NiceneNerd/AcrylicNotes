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
//!   test-windows-native run the shared-crate tests on a real Windows
//!                       machine over SSH (a local VM, e.g. QEMU):
//!                       catches Wine-vs-Windows divergence that Wine emulation
//!                       cannot. Configure with WINDOWS_VM_HOST /
//!                       WINDOWS_VM_USER / WINDOWS_VM_KEY

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
        Some("test-windows-native") => {
            // `test-windows-native [target]`: the optional first positional
            // arg is the target triple (default x86_64-pc-windows-gnu).
            let target = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("x86_64-pc-windows-gnu");
            run_windows_native_tests(target);
        }
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

/// Runs cargo with extra environment variables. Used by `test-windows-core` to
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

/// Builds test binaries and runs them on a real Windows machine over SSH.
///
/// This lane runs the shared-crate tests on actual Windows (a local VM is
/// enough) so Wine shims cannot hide a divergence.
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
fn run_windows_native_tests(target: &str) -> ! {
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
    //    (like any bin-only crate) would exit non-zero by design.
    let mut build = vec![
        "test",
        "--target",
        target,
        "--no-run",
        "--message-format=json",
    ];
    for crate_name in WINDOWS_SHARED_CRATES {
        build.push("-p");
        build.push(crate_name);
    }
    println!(
        "== building test harnesses for {target} ({}) ==",
        WINDOWS_SHARED_CRATES.join(", ")
    );
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
            // Test harnesses are `<crate>-<hex-hash>.exe`; a bare bin
            // (`<crate>.exe`) ends in a non-hex segment and is not a
            // test target (its main exits non-zero by design).
            let stem = exe.trim_end_matches(".exe");
            let last = stem.rsplit('-').next().unwrap_or("");
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
    let fixtures_remote = "vaultnotes-fixtures/vault-md-tests";
    let mkdir_cmd = format!(
        "powershell -Command \"New-Item -ItemType Directory -Path 'C:\\\\Users\\\\{user}\\\\{fixtures_remote}' -Force\""
    );
    let _ = Command::new("ssh")
        .args(ssh_opts)
        .arg(remote.as_str())
        .arg(&mkdir_cmd)
        .status();
    // Stage both fixture trees next to the executables.
    for (src, dst) in [
        ("vault-md/tests", "vault-md-tests"),
        ("tests/fixtures", "fixtures"),
    ] {
        let fixtures_remote = format!("vaultnotes-fixtures/{dst}");
        let fixtures = Command::new("scp")
            .args(ssh_opts)
            .arg("-r")
            .arg(format!("{src}/."))
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
            .args(ssh_opts)
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
            .args(ssh_opts)
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
        "  test-windows-native [target]  run the shared-crate tests on a real \
         Windows machine over SSH (WINDOWS_VM_HOST/USER/KEY)."
    );
    println!();
    println!("Example: cargo run -p xtask -- test-windows-core x86_64-pc-windows-gnu");
}
