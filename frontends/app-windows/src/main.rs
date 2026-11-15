//! Binary entry point for the WinUI 3 frontend.
//!
//! On Windows this is where the shell boots: bootstrap the Windows App
//! Runtime, build the shared [`WindowsApp`] kernel host over the live
//! platform adapters, and mount the [`Shell`] component through
//! windows-reactor's live message loop.
//!
//!     windows_reactor::App::run_component::<Shell>(ShellHandle::new(app));
//!
//! which blocks until the application window closes.
//!
//! Live platform adapters (p11):
//! * [`crate::win_opener::WinOpener`] — `ExternalOpener` via WinRT `Launcher`
//! * [`crate_core::Vault`] — `NoteRepository` (already implemented in
//!   `crates/vault-app/src/repository.rs`)
//! * [`crate::settings_store::FileSettingsStore`] — `SettingsStore`
//!
//! On non-Windows targets this binary is a skeleton; the library code is
//! compiled and unit-tested on every target. Native Windows UI
//! verification is user-side (real WinUI rendering, IME, DPI, theming,
//! pickers, MSIX packaging). Never claimed via Wine.

#[cfg(not(target_os = "windows"))]
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

#[cfg(target_os = "windows")]
fn main() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use vault_app::repository::NoteRepository;
    use vault_core::Vault;
    use windows_reactor::App as ReactorApp;

    use app_windows::app::WindowsApp;
    use app_windows::settings_store::FileSettingsStore;
    use app_windows::shell::{Shell, ShellHandle};
    use app_windows::win_opener::WinOpener;

    // 1. Settings store — load-modify-store through the file-backed store.
    let settings_store = FileSettingsStore::in_config_dir()
        .expect("could not resolve the Windows config directory for settings");
    let settings: Rc<RefCell<dyn vault_app::settings::SettingsStore>> =
        Rc::new(RefCell::new(settings_store));

    // 2. Find the vault root from the persisted settings snapshot.
    let document = settings.borrow().load();
    let vault_root = document
        .last_vault
        .as_deref()
        .expect("no last vault in settings; open a vault first");

    // 3. Open the vault — the real repository, backed by vault_core::Vault.
    //    Vault::open() canonicalizes the root and starts the file watcher.
    let vault = Vault::open(vault_root).expect("could not open the vault");
    let vault_root_buf = vault.root().to_path_buf();

    // 4. Platform services injected into the kernel host. The same live
    //    Vault backs the repository and the note index (tags, backlinks,
    //    link resolution).
    let vault = Rc::new(RefCell::new(vault));
    let repo: Rc<RefCell<dyn NoteRepository>> = vault.clone();
    let index: Rc<RefCell<dyn vault_app::navigation::NoteIndex>> = vault.clone();
    let opener: Rc<dyn vault_platform::ExternalOpener> = Rc::new(WinOpener::new());

    // 5. Build the kernel host over the live services.
    let app = Rc::new(RefCell::new(WindowsApp::new(
        repo,
        index,
        opener,
        settings,
        vault_root_buf,
    )));

    // 6. Mount the shell component over the shared kernel. Blocks until
    //    the application window closes.
    ReactorApp::run_component::<Shell>(ShellHandle::new(app))
        .expect("could not run the shell component");
}
