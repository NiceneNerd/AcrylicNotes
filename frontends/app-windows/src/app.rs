//! Native WinUI 3 frontend for VaultNotes (windows-reactor).
//!
//! The shell owns exactly what the thin cosmic shell owns: the shared
//! [`App`] kernel (state + reducer), text buffers for the open tabs (the
//! kernel tabs model holds paths and ids), and native handles (the
//! runner's platform services). Native events become [`UiCommand`]s,
//! the kernel decides, effects run against the injected services, and
//! outcomes feed back as `AppEvent`s. This file holds no feature logic.
//!
//! Everything here compiles and is unit-tested on every platform; only
//! the native window loop (see main.rs) is Windows-only. Native Windows
//! UI verification is user-side (no Windows runner in this session; never
//! claimed via Wine). See restructure plan phase 9.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use vault_app::commands::UiCommand;
use vault_app::effects::{AppEvent, Effect, EffectKind};
use vault_app::ids::{EffectId, TabId};
use vault_app::repository::NoteRepository;
use vault_app::workflow::SaveOutcome;
use vault_app::App;
use vault_platform::ExternalOpener;

use crate::windows_effect_runner::WindowsEffectRunner;
use crate::windows_message::{NativeEvent, WindowsMessage};

/// The text buffer the shell renders for one open tab.
///
/// The kernel's tabs model tracks paths, order, and conflicts; note text
/// stays frontend-side (mirroring the cosmic shell's `Tab` payloads).
/// `base` is the disk snapshot the edits were made against — the sentinel
/// for guarded saves — so `text != base` means the tab is dirty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteBuffer {
    /// The text currently shown in the editor.
    pub text: String,
    /// The disk text the edits started from.
    pub base: String,
}

impl NoteBuffer {
    /// A buffer whose text equals its base (freshly loaded, clean).
    #[must_use]
    pub fn clean(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            base: text.clone(),
            text,
        }
    }

    /// Whether the buffer has unsaved edits.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.text != self.base
    }
}

/// The WinUI 3 application host.
pub struct WindowsApp {
    /// The shared application kernel (state + reducer).
    pub shared: App,
    /// The platform services effects run against (repository, opener).
    pub runner: WindowsEffectRunner,
    /// The shell's text buffers, keyed by tab id.
    pub buffers: HashMap<TabId, NoteBuffer>,
}

impl WindowsApp {
    /// Construct an application around injected services.
    ///
    /// Production (Windows) wiring passes the live vault adapter and
    /// shell launcher; tests inject a memory repository and a recording
    /// opener, so the whole flow runs on Linux.
    #[must_use]
    pub fn new(repo: Rc<RefCell<dyn NoteRepository>>, opener: Rc<dyn ExternalOpener>) -> Self {
        Self {
            shared: App::new(),
            runner: WindowsEffectRunner::new(repo, opener),
            buffers: HashMap::new(),
        }
    }

    /// Every note path in the vault, sorted vault-relative. Builds the
    /// file-tree sidebar's node list.
    pub fn note_paths(&self) -> Vec<String> {
        self.runner.note_paths()
    }

    /// Handle one message from the native UI: dispatch commands, absorb
    /// late effect outcomes, and translate raw native events. Effects
    /// never come back out — the runner's outcomes go straight into the
    /// reducer via `App::handle`.
    pub fn handle_native(&mut self, message: WindowsMessage) {
        match message {
            WindowsMessage::Command(command) => {
                let effects = self.shared.dispatch(command);
                self.run_effects(effects);
            }
            WindowsMessage::EffectCompleted(event) => {
                // Late outcomes (real async I/O on Windows) feed the same
                // reducer path as inline completions; stale ids are
                // rejected by the kernel.
                self.shared.handle(event);
            }
            WindowsMessage::Native(native) => self.handle_native_event(native),
        }
    }

    /// Run the reducer's effects against the platform services, feeding
    /// each immediate outcome back through `App::handle`. Mirrors the
    /// cosmic shell's effect runner: saves and external opens execute
    /// inline; settings persistence and the vault-wide search backend
    /// are wired with the p11 services (the cosmic runner no-ops those
    /// two today as well).
    fn run_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match &effect.kind {
                EffectKind::SaveTab { tab } => self.save_tab(*tab, effect.id),
                EffectKind::ReloadTab { tab } => self.reload_tab(*tab, effect.id),
                EffectKind::OpenExternal { target } => {
                    let event = match self.runner.open_external(target) {
                        Ok(()) => AppEvent::OpenCompleted { effect: effect.id },
                        Err(error) => AppEvent::OpenFailed {
                            effect: effect.id,
                            error: format!("open failed: {error}"),
                        },
                    };
                    self.shared.handle(event);
                }
                EffectKind::PersistSettings | EffectKind::SearchVault { .. } => {}
            }
        }
    }

    /// Save one tab's buffer through the shared guarded-save workflow and
    /// report the outcome to the reducer.
    fn save_tab(&mut self, tab: TabId, effect: EffectId) {
        let Some(path) = self.shared.state.tabs.path(tab).map(str::to_string) else {
            // The tab vanished mid-flight: drain the in-flight tracking
            // the same way the cosmic runner does.
            self.shared.handle(AppEvent::SaveCompleted { effect });
            return;
        };
        let Some(buffer) = self.buffers.get(&tab) else {
            self.shared.handle(AppEvent::SaveCompleted { effect });
            return;
        };
        let text = buffer.text.clone();
        let base = buffer.base.clone();
        match self.runner.save(&path, &text, &base) {
            SaveOutcome::Saved => {
                // The buffer text is now the disk state (the new base).
                if let Some(buffer) = self.buffers.get_mut(&tab) {
                    buffer.base = text;
                }
                self.shared.handle(AppEvent::SaveCompleted { effect });
            }
            SaveOutcome::ExternallyModified => {
                self.shared.handle(AppEvent::SaveFailed {
                    effect,
                    error: "changed on disk while you edited".to_string(),
                });
            }
            SaveOutcome::Failed(message) => {
                self.shared.handle(AppEvent::SaveFailed {
                    effect,
                    error: message,
                });
            }
        }
    }

    /// Re-read one tab's note from disk: the buffer takes the disk text
    /// and the reducer resolves the conflict. Read failures keep the
    /// conflict open and surface the reason (`ReloadFailed`).
    fn reload_tab(&mut self, tab: TabId, effect: EffectId) {
        let Some(path) = self.shared.state.tabs.path(tab).map(str::to_string) else {
            // The tab vanished mid-flight: drain the tracking with an
            // empty completion (the reducer applies nothing for a tab
            // that no longer exists).
            self.shared.handle(AppEvent::ReloadCompleted {
                effect,
                text: String::new(),
            });
            return;
        };
        match self.runner.read_note(&path) {
            Ok(text) => {
                if let Some(buffer) = self.buffers.get_mut(&tab) {
                    *buffer = NoteBuffer::clean(text.clone());
                }
                self.shared
                    .handle(AppEvent::ReloadCompleted { effect, text });
            }
            Err(error) => {
                self.shared.handle(AppEvent::ReloadFailed { effect, error });
            }
        }
    }

    /// Translate a raw native event into kernel work.
    fn handle_native_event(&mut self, event: NativeEvent) {
        match event {
            NativeEvent::WindowClosing => {
                // Flush every dirty buffer through the guarded-save path
                // (WIN-108 semantics: closing never silently discards
                // edits). The shell owns the dirty decision (it holds the
                // buffers); the kernel tracks the save effects.
                let dirty: Vec<TabId> = self
                    .shared
                    .state
                    .tabs
                    .order()
                    .iter()
                    .filter(|&&tab| self.buffers.get(&tab).is_some_and(NoteBuffer::is_dirty))
                    .copied()
                    .collect();
                let effects = self.shared.dispatch(UiCommand::SaveTabs { tabs: dirty });
                self.run_effects(effects);
            }
            NativeEvent::EditorEdited { text } => self.set_active_buffer_text(text),
        }
    }

    /// Take a genuine user edit from the editor control: the active
    /// tab's buffer takes the text. The base (disk) text is untouched —
    /// dirtiness stays exactly "buffer text differs from disk text".
    /// Edits with no active tab or no buffer yet (open still in flight)
    /// are dropped, not buffered for later.
    pub fn set_active_buffer_text(&mut self, text: String) {
        let Some(tab) = self.shared.state.tabs.active() else {
            return;
        };
        if let Some(buffer) = self.buffers.get_mut(&tab) {
            buffer.text = text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_app::persistence::ConflictDecision;
    use vault_app::repository::testing::MemoryRepository;
    use vault_app::settings::PaneMode;
    use vault_platform::testing::{OpenCall, RecordingOpener};
    use vault_platform::OpenError;

    /// An app over a memory vault seeded with `notes`, plus the handles
    /// the tests assert against.
    fn app_with(
        notes: &[(&str, &str)],
    ) -> (
        WindowsApp,
        Rc<RefCell<MemoryRepository>>,
        Rc<RecordingOpener>,
    ) {
        let repo = Rc::new(RefCell::new(MemoryRepository::new()));
        for (path, text) in notes {
            repo.borrow_mut().insert(path, text);
        }
        let opener = Rc::new(RecordingOpener::succeeding());
        (WindowsApp::new(repo.clone(), opener.clone()), repo, opener)
    }

    /// Open `path` as a tab and give it a buffer with the given text over
    /// the given base (disk) text.
    fn open_tab(app: &mut WindowsApp, path: &str, text: &str, base: &str) -> TabId {
        app.handle_native(WindowsMessage::Command(UiCommand::OpenNote {
            path: path.to_string(),
        }));
        let tab = app.shared.state.tabs.id_of(path).expect("tab opened");
        app.buffers.insert(
            tab,
            NoteBuffer {
                text: text.to_string(),
                base: base.to_string(),
            },
        );
        tab
    }

    #[test]
    fn note_buffer_tracks_dirtiness_against_its_base() {
        let clean = NoteBuffer::clean("same");
        assert!(!clean.is_dirty());
        let dirty = NoteBuffer {
            text: "edited".into(),
            base: "same".into(),
        };
        assert!(dirty.is_dirty());
    }

    #[test]
    fn an_editor_edit_lands_in_the_active_buffer_and_tracks_dirtiness() {
        let (mut app, _repo, _opener) = app_with(&[("a.md", "old\n")]);
        open_tab(&mut app, "a.md", "old\n", "old\n");

        app.handle_native(WindowsMessage::Native(NativeEvent::EditorEdited {
            text: "typed\n".into(),
        }));
        let tab = app.shared.state.tabs.id_of("a.md").unwrap();
        let buffer = &app.buffers[&tab];
        assert_eq!(buffer.text, "typed\n");
        assert_eq!(buffer.base, "old\n", "disk truth is untouched");
        assert!(buffer.is_dirty());
        // No error, no effect: an edit is buffer-level state only.
        assert_eq!(app.shared.state.last_error, None);
    }

    #[test]
    fn an_editor_edit_is_dropped_without_an_active_buffer() {
        let (mut app, _repo, _opener) = app_with(&[]);
        // No tabs at all.
        app.set_active_buffer_text("orphan\n".into());
        assert!(app.buffers.is_empty());

        // A tab open but its buffer not inserted yet (open in flight).
        app.handle_native(WindowsMessage::Command(UiCommand::OpenNote {
            path: "b.md".into(),
        }));
        app.set_active_buffer_text("orphan\n".into());
        assert!(app.buffers.is_empty(), "no buffer was created for the edit");
    }

    #[test]
    fn an_editor_edit_applies_to_whichever_tab_is_active() {
        let (mut app, _repo, _opener) = app_with(&[("a.md", "a\n"), ("b.md", "b\n")]);
        open_tab(&mut app, "a.md", "a text\n", "a\n");
        open_tab(&mut app, "b.md", "b text\n", "b\n");
        // open_or_focus focused b.md; edits land in b.
        app.handle_native(WindowsMessage::Native(NativeEvent::EditorEdited {
            text: "edited b\n".into(),
        }));
        let b = app.shared.state.tabs.id_of("b.md").unwrap();
        assert_eq!(app.buffers[&b].text, "edited b\n");

        // Switch back to a; edits now land in a.
        app.handle_native(WindowsMessage::Command(UiCommand::ActivateTab {
            position: 0,
        }));
        app.handle_native(WindowsMessage::Native(NativeEvent::EditorEdited {
            text: "edited a\n".into(),
        }));
        let a = app.shared.state.tabs.id_of("a.md").unwrap();
        assert_eq!(app.buffers[&a].text, "edited a\n");
    }

    #[test]
    fn commands_change_shared_state_and_nothing_else() {
        let (mut app, _repo, _opener) = app_with(&[]);
        app.handle_native(WindowsMessage::Command(UiCommand::SetPaneMode {
            mode: vault_app::settings::PaneMode::Split,
        }));
        app.handle_native(WindowsMessage::Command(UiCommand::SetSidebarVisible {
            open: false,
        }));
        assert_eq!(app.shared.state.pane, vault_app::settings::PaneMode::Split);
        assert!(!app.shared.state.sidebar_open);
        // No effects were involved; no error surfaced.
        assert_eq!(app.shared.state.last_error, None);
    }

    #[test]
    fn saving_persists_through_the_repository_and_cleans_the_buffer() {
        let (mut app, repo, _opener) = app_with(&[("a.md", "old\n")]);
        open_tab(&mut app, "a.md", "new body\n", "old\n");

        app.handle_native(WindowsMessage::Command(UiCommand::SaveActiveTab));

        // The disk text is the new body; the buffer's base moved with it.
        assert_eq!(
            repo.borrow().note_text("a.md"),
            Some("new body\n".to_string())
        );
        let tab = app.shared.state.tabs.id_of("a.md").unwrap();
        assert_eq!(app.buffers[&tab].base, "new body\n");
        assert!(!app.buffers[&tab].is_dirty());
        // No conflict, no error, nothing left in flight.
        assert!(app.shared.state.conflicts.is_empty());
        assert_eq!(app.shared.state.last_error, None);
    }

    #[test]
    fn a_failed_save_marks_the_conflict_and_the_error_line() {
        let (mut app, repo, _opener) = app_with(&[("a.md", "old\n")]);
        repo.borrow_mut().fail_saves_with("io broke");
        open_tab(&mut app, "a.md", "new body\n", "old\n");
        let tab = app.shared.state.tabs.id_of("a.md").unwrap();

        app.handle_native(WindowsMessage::Command(UiCommand::SaveActiveTab));

        assert!(app.shared.state.is_conflicted(tab));
        assert_eq!(
            app.shared.state.last_error.as_deref(),
            Some("save failed: io broke")
        );
        // The buffer keeps its edits (nothing was written).
        assert_eq!(app.buffers[&tab].text, "new body\n");
        assert_eq!(repo.borrow().note_text("a.md"), Some("old\n".to_string()));
    }

    #[test]
    fn pane_mode_commands_drive_the_kernel_layout() {
        let (mut app, _repo, _opener) = app_with(&[("a.md", "a\n")]);
        // The default pane is Live (the settings default).
        assert_eq!(app.shared.state.pane, PaneMode::Live);

        app.handle_native(WindowsMessage::Command(UiCommand::SetPaneMode {
            mode: PaneMode::Split,
        }));
        assert_eq!(app.shared.state.pane, PaneMode::Split);
        assert_eq!(app.shared.state.last_error, None);

        app.handle_native(WindowsMessage::Command(UiCommand::SetPaneMode {
            mode: PaneMode::Source,
        }));
        assert_eq!(app.shared.state.pane, PaneMode::Source);
    }

    #[test]
    fn note_paths_lists_every_vault_note_sorted() {
        let (app, _repo, _opener) =
            app_with(&[("z.md", "z\n"), ("a.md", "a\n"), ("Notes/b.md", "b\n")]);
        assert_eq!(
            app.note_paths(),
            vec![
                "Notes/b.md".to_string(),
                "a.md".to_string(),
                "z.md".to_string(),
            ]
        );
        // An empty vault lists nothing.
        let (empty, _repo, _opener) = app_with(&[]);
        assert!(empty.note_paths().is_empty());
    }

    #[test]
    fn an_externally_modified_note_raises_the_conflict_without_writing() {
        let (mut app, repo, _opener) = app_with(&[("a.md", "changed on disk\n")]);
        // The buffer's base is stale: the note changed after it was loaded.
        open_tab(&mut app, "a.md", "my edits\n", "stale base\n");
        let tab = app.shared.state.tabs.id_of("a.md").unwrap();

        app.handle_native(WindowsMessage::Command(UiCommand::SaveActiveTab));

        assert!(app.shared.state.is_conflicted(tab));
        assert_eq!(
            app.shared.state.last_error.as_deref(),
            Some("changed on disk while you edited")
        );
        // The external change was not clobbered.
        assert_eq!(
            repo.borrow().note_text("a.md"),
            Some("changed on disk\n".to_string())
        );
    }

    #[test]
    fn reloading_from_disk_takes_the_disk_text_and_resolves_the_conflict() {
        let (mut app, repo, _opener) = app_with(&[("a.md", "disk truth\n")]);
        let tab = open_tab(&mut app, "a.md", "my edits\n", "stale base\n");
        app.handle_native(WindowsMessage::Command(UiCommand::SaveActiveTab));
        assert!(app.shared.state.is_conflicted(tab));

        app.handle_native(WindowsMessage::Command(UiCommand::ResolveConflict {
            tab,
            decision: ConflictDecision::ReloadFromDisk,
        }));

        // The buffer took the disk text (clean), the conflict is gone.
        assert_eq!(app.buffers[&tab].text, "disk truth\n");
        assert_eq!(app.buffers[&tab].base, "disk truth\n");
        assert!(!app.shared.state.is_conflicted(tab));
        assert_eq!(
            repo.borrow().save_records().len(),
            0,
            "reload writes nothing"
        );
    }

    #[test]
    fn a_failed_reload_keeps_the_conflict_and_explains_why() {
        let (mut app, _repo, _opener) = app_with(&[]);
        // The note does not exist in the vault at all: the save fails
        // ("note not found") and the conflict banner is raised.
        let tab = open_tab(&mut app, "gone.md", "text\n", "text\n");
        app.handle_native(WindowsMessage::Command(UiCommand::SaveActiveTab));
        assert!(app.shared.state.is_conflicted(tab));

        // Asking to reload from disk cannot read the note either: the
        // banner must come back, with the reload reason this time.
        app.handle_native(WindowsMessage::Command(UiCommand::ResolveConflict {
            tab,
            decision: ConflictDecision::ReloadFromDisk,
        }));

        assert!(app.shared.state.is_conflicted(tab));
        assert_eq!(
            app.shared.state.last_error.as_deref(),
            Some("reload failed: note not found: gone.md")
        );
    }

    #[test]
    fn external_opens_report_through_the_opener() {
        let (mut app, _repo, opener) = app_with(&[]);
        app.handle_native(WindowsMessage::Command(UiCommand::OpenExternal {
            target: vault_platform::OpenTarget::Url("https://example.com".into()),
        }));
        assert_eq!(
            opener.calls(),
            vec![OpenCall::Url("https://example.com".into())]
        );
        assert_eq!(app.shared.state.last_error, None);
    }

    #[test]
    fn a_refused_external_open_surfaces_on_the_error_line() {
        let repo = Rc::new(RefCell::new(MemoryRepository::new()));
        let opener = Rc::new(RecordingOpener::failing(OpenError::NotFound {
            program: "xdg-open".into(),
        }));
        let mut app = WindowsApp::new(repo, opener.clone());

        app.handle_native(WindowsMessage::Command(UiCommand::OpenExternal {
            target: vault_platform::OpenTarget::File("/tmp/attachment.png".into()),
        }));

        assert_eq!(
            opener.calls(),
            vec![OpenCall::File("/tmp/attachment.png".into())]
        );
        assert_eq!(
            app.shared.state.last_error.as_deref(),
            Some("open failed: opener xdg-open not found")
        );
    }

    #[test]
    fn window_closing_flushes_only_the_dirty_buffers() {
        let (mut app, repo, _opener) =
            app_with(&[("a.md", "a on disk\n"), ("b.md", "b on disk\n")]);
        open_tab(&mut app, "a.md", "a edited\n", "a on disk\n"); // dirty
        open_tab(&mut app, "b.md", "b on disk\n", "b on disk\n"); // clean

        app.handle_native(WindowsMessage::Native(NativeEvent::WindowClosing));

        // Only the dirty tab hit the disk.
        assert_eq!(
            repo.borrow().note_text("a.md"),
            Some("a edited\n".to_string())
        );
        assert_eq!(
            repo.borrow().note_text("b.md"),
            Some("b on disk\n".to_string())
        );
        assert_eq!(repo.borrow().save_records().len(), 1);
        assert!(app.shared.state.conflicts.is_empty());
    }

    #[test]
    fn late_or_stale_effect_completions_are_ignored_by_the_kernel() {
        let (mut app, repo, _opener) = app_with(&[("a.md", "old\n")]);
        open_tab(&mut app, "a.md", "new\n", "old\n");
        // Dispatch directly to capture the effect id, then complete it.
        let effects = app.shared.dispatch(UiCommand::SaveActiveTab);
        let id = effects[0].id;
        app.shared.handle(AppEvent::SaveCompleted { effect: id });

        // The same completion again (and one for a never-dispatched id)
        // must be rejected, not panic or corrupt state.
        app.handle_native(WindowsMessage::EffectCompleted(AppEvent::SaveCompleted {
            effect: id,
        }));
        app.handle_native(WindowsMessage::EffectCompleted(AppEvent::SaveFailed {
            effect: vault_app::ids::EffectId::new(999),
            error: "stale failure".into(),
        }));
        assert_eq!(app.shared.state.last_error, None);
        // The earlier save still went through (the test completed it
        // without running it; the repo is untouched here).
        assert_eq!(repo.borrow().save_records().len(), 0);
    }

    #[test]
    fn a_save_for_a_tab_closed_mid_flight_is_drained() {
        let (mut app, repo, _opener) = app_with(&[("a.md", "old\n")]);
        open_tab(&mut app, "a.md", "new\n", "old\n");
        let tab = app.shared.state.tabs.id_of("a.md").unwrap();
        let effects = app.shared.dispatch(UiCommand::SaveTabs { tabs: vec![tab] });
        let id = effects[0].id;
        // The tab closes before the runner executes the save.
        app.shared.state.tabs.close(tab);

        app.run_effects(effects);

        // The vanished-tab rule drained the completion: no conflict, no
        // error, nothing written.
        assert!(app.shared.state.conflicts.is_empty());
        assert_eq!(app.shared.state.last_error, None);
        assert_eq!(repo.borrow().save_records().len(), 0);
        assert!(!app.shared.is_in_flight(id));
    }
}
