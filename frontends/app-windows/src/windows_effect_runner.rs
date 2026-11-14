//! The platform services the shared reducer's effects run against.
//!
//! The runner is a service holder, not a decision maker: it owns the
//! [`NoteRepository`] (guarded saves, note reads) and the platform
//! [`ExternalOpener`] (shell hand-offs), injected at construction. Which
//! effects run and what they report back is `WindowsApp::run_effects`'s
//! job, mirroring the cosmic shell's effect runner.
//!
//! Tests inject [`vault_app::repository::testing::MemoryRepository`] and
//! [`vault_platform::testing::RecordingOpener`], so the whole effect path
//! — dispatch, execution, event feedback — runs and is asserted on Linux.
//! Production (Windows) wiring passes the live adapters, which arrive
//! with the p11 platform services. All types here are single-threaded
//! (`Rc`/`RefCell`); the native window loop runs the UI on one thread.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use vault_app::repository::{NoteRepository, RepositoryError};
use vault_app::search::{SearchHitView, SEARCH_MAX_HITS};
use vault_app::settings::{PaneMode, SettingsStore};
use vault_app::workflow::SaveOutcome;
use vault_platform::{ExternalOpener, OpenError, OpenTarget};

/// Holds the services the reducer's effects need.
pub struct WindowsEffectRunner {
    repo: Rc<RefCell<dyn NoteRepository>>,
    opener: Rc<dyn ExternalOpener>,
    settings: Rc<RefCell<dyn SettingsStore>>,
    /// The vault root the shared search engine scans (the Linux shell
    /// reads it from the open `Vault`; here it is injected, since the
    /// repository trait deliberately hides its backing store).
    root: PathBuf,
}

impl WindowsEffectRunner {
    /// Construct a runner around injected services.
    #[must_use]
    pub fn new(
        repo: Rc<RefCell<dyn NoteRepository>>,
        opener: Rc<dyn ExternalOpener>,
        settings: Rc<RefCell<dyn SettingsStore>>,
        root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo,
            opener,
            settings,
            root: root.into(),
        }
    }

    /// Perform a guarded save of `text` at `path`, with `base` as the
    /// sentinel the edits were made against (shared workflow decision).
    pub fn save(&mut self, path: &str, text: &str, base: &str) -> SaveOutcome {
        let mut repo = self.repo.borrow_mut();
        vault_app::workflow::guarded_save(&mut *repo, path, text, base)
    }

    /// The current disk text of the note at `path`, with a user-facing
    /// error message on failure (the `reload failed: ...` family the
    /// workflow reports).
    pub fn read_note(&self, path: &str) -> Result<String, String> {
        self.repo
            .borrow()
            .read_note(path)
            .map_err(|error| match error {
                RepositoryError::NotFound { path } => {
                    format!("reload failed: note not found: {path}")
                }
                RepositoryError::ExternallyModified => {
                    "reload failed: changed on disk while you edited".to_string()
                }
                RepositoryError::Failed { message } => format!("reload failed: {message}"),
            })
    }

    /// Every note path in the vault, sorted vault-relative. Builds the
    /// file-tree sidebar's node list.
    pub fn note_paths(&self) -> Vec<String> {
        self.repo.borrow().note_paths()
    }

    /// Hand an open target to the platform's default handler. Success is
    /// fire-and-forget (the reducer drains the tracking on
    /// `OpenCompleted`); spawn-level failures surface as `OpenFailed`.
    pub fn open_external(&self, target: &OpenTarget) -> Result<(), String> {
        let outcome = match target {
            OpenTarget::Url(url) => self.opener.open_url(url),
            OpenTarget::File(path) => self.opener.open_file(path),
        };
        outcome.map_err(|error: OpenError| error.to_string())
    }

    /// Whether the platform could open `path` in a file manager (part of
    /// the opener contract; no consumer feature yet).
    pub fn reveal_path(&self, path: &Path) -> Result<(), String> {
        self.opener
            .reveal_path(path)
            .map_err(|error| error.to_string())
    }

    /// Persist the kernel's settings snapshot: load-modify-store, so
    /// every field the kernel does not own survives the write. This is
    /// the `EffectKind::PersistSettings` arm's work.
    pub fn persist_settings(&mut self, default_pane: PaneMode) {
        let mut document = self.settings.borrow().load();
        document.default_pane = default_pane;
        self.settings.borrow_mut().store(&document);
    }

    /// Run the vault-wide search over the vault root with the shared
    /// engine and cap (the same literal ripgrep scan the Linux shell
    /// runs off-thread). A scan failure yields no hits, matching the
    /// Linux shell's `unwrap_or_default`.
    pub fn search(&self, query: &str) -> Vec<SearchHitView> {
        vault_core::search::search(&self.root, query, SEARCH_MAX_HITS)
            .unwrap_or_default()
            .into_iter()
            .map(SearchHitView::from)
            .collect()
    }
}
