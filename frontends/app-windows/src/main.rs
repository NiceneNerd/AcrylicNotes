//! Binary entry point for the WinUI 3 frontend.
//!
//! On Windows this is where the shell boots: bootstrap the Windows App
//! Runtime, build the shared [`WindowsApp`] kernel host over the live
//! platform adapters, and mount the [`Shell`] component through
//! windows-reactor's live message loop
//!
//!     windows_reactor::App::run_component::<Shell>(ShellHandle::new(app));
//!
//! which blocks until the application window closes.
//!
//! The live platform adapters (the vault repository, the external
//! opener, and the settings store) are the p11 platform services and
//! land on a real Windows machine; they do not exist yet, so this binary
//! is a skeleton that exits with an explanatory message. The settings
//! store's file-backed adapter already exists
//! (`FileSettingsStore::in_config_dir`); the vault adapter must also
//! supply the vault root the search engine scans. Native execution and
//! UI verification are user-side, never claimed via Wine.
//!
//! On non-Windows targets this binary is a skeleton; the library code is
//! compiled and unit-tested on every target.

fn main() {
    eprintln!(
        "app-windows: the native Windows UI is not wired yet; the live \
         platform adapters (vault repository, external opener) land with \
         the p11 platform services on a real Windows machine. Once they \
         exist, mount the shell with \
         `windows_reactor::App::run_component::<Shell>(ShellHandle::new(app))`. \
         See docs/parity.md APPWIN rows."
    );
    std::process::exit(1);
}
