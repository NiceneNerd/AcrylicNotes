//! The deterministic reducer: commands in, state + effects out.
//!
//! `dispatch` mutates [`AppState`] and returns the effects to execute;
//! `handle` consumes an executed effect's result. Both are pure with
//! respect to I/O, so the whole loop is testable without a GUI (the
//! Windows frontend runs the exact same calls).

use std::collections::HashMap;

use crate::app_state::AppState;
use crate::commands::UiCommand;
use crate::effects::{AppEvent, Effect, EffectKind};
use crate::ids::{EffectId, IdAllocator, TabId};

/// The shared application: state plus the reducer that owns it.
#[derive(Debug, Default)]
pub struct App {
    /// The state both frontends render.
    pub state: AppState,
    effect_ids: IdAllocator,
    /// Effects requested but not yet completed, keyed by id, mapping to
    /// the tab they act on. Used to reject stale completions.
    in_flight: HashMap<EffectId, TabId>,
}

impl App {
    /// An application with default state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a user intent, returning the effects the frontend must run.
    pub fn dispatch(&mut self, command: UiCommand) -> Vec<Effect> {
        match command {
            UiCommand::SetPaneMode(mode) => {
                self.state.pane = mode;
                Vec::new()
            }
            UiCommand::SetSidebarVisible(open) => {
                self.state.sidebar_open = open;
                Vec::new()
            }
            UiCommand::SetNavSection(section) => {
                self.state.panel = section;
                Vec::new()
            }
            UiCommand::OpenNote { path } => {
                self.state.tabs.open_or_focus(&path);
                Vec::new()
            }
            UiCommand::ActivateTab { position } => {
                if let Some(id) = self.state.tabs.id_at(position) {
                    self.state.tabs.activate(id);
                }
                Vec::new()
            }
            UiCommand::CloseTab { position } => {
                if let Some(id) = self.state.tabs.id_at(position) {
                    self.state.tabs.close(id);
                }
                Vec::new()
            }
            UiCommand::SaveActiveTab => {
                let Some(tab) = self.state.tabs.active() else {
                    return Vec::new();
                };
                let id = self.effect_ids.allocate();
                self.in_flight.insert(id, tab);
                vec![Effect::new(id, EffectKind::SaveTab { tab })]
            }
            UiCommand::ResolveConflict { tab, decision } => match decision {
                crate::persistence::ConflictDecision::KeepMine => {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                    Vec::new()
                }
                crate::persistence::ConflictDecision::ReloadFromDisk => {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                    let id = self.effect_ids.allocate();
                    self.in_flight.insert(id, tab);
                    vec![Effect::new(id, EffectKind::ReloadTab { tab })]
                }
                crate::persistence::ConflictDecision::SaveMineAnyway => {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                    let id = self.effect_ids.allocate();
                    self.in_flight.insert(id, tab);
                    vec![Effect::new(id, EffectKind::SaveTab { tab })]
                }
            },
        }
    }

    /// Consume an executed effect's result. Events for unknown (stale or
    /// already-completed) effect ids are ignored.
    pub fn handle(&mut self, event: AppEvent) {
        let (effect_id, tab) = match &event {
            AppEvent::SaveCompleted { effect }
            | AppEvent::SaveFailed { effect, .. }
            | AppEvent::ReloadCompleted { effect, .. } => match self.in_flight.get(effect) {
                Some(&tab) => (*effect, tab),
                None => return, // stale or duplicate completion
            },
        };
        match event {
            AppEvent::SaveCompleted { .. } => {
                // The disk now matches the buffer: the conflict is resolved.
                self.state.conflicts.retain(|&candidate| candidate != tab);
            }
            AppEvent::SaveFailed { error, .. } => {
                // A refused save (externally modified or I/O error) leaves
                // the tab conflicted until the user decides.
                if !self.state.is_conflicted(tab) {
                    self.state.conflicts.push(tab);
                }
                self.state.last_error = Some(error);
            }
            AppEvent::ReloadCompleted { .. } => {
                // The note was re-read; the conflict is resolved (the
                // frontend applies the returned text to its payload).
                self.state.conflicts.retain(|&candidate| candidate != tab);
            }
        }
        self.in_flight.remove(&effect_id);
    }

    /// Whether an effect with this id is still awaiting completion.
    #[must_use]
    pub fn is_in_flight(&self, id: EffectId) -> bool {
        self.in_flight.contains_key(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TabId as _;
    use crate::persistence::ConflictDecision;

    fn app_with_tabs(paths: &[&str]) -> App {
        let mut app = App::new();
        for path in paths {
            app.state.tabs.open(path);
        }
        app
    }

    fn tab_at(app: &App, position: usize) -> TabId {
        app.state.tabs.id_at(position).unwrap()
    }

    #[test]
    fn pane_sidebar_and_section_commands_update_the_state() {
        let mut app = App::new();
        assert_eq!(
            app.dispatch(UiCommand::SetPaneMode(crate::settings::PaneMode::Split)),
            vec![]
        );
        assert_eq!(app.state.pane, crate::settings::PaneMode::Split);
        assert_eq!(app.dispatch(UiCommand::SetSidebarVisible(false)), vec![]);
        assert!(!app.state.sidebar_open);
        assert_eq!(
            app.dispatch(UiCommand::SetNavSection(
                crate::navigation::NavSection::Tags
            )),
            vec![]
        );
        assert_eq!(app.state.panel, crate::navigation::NavSection::Tags);
    }

    #[test]
    fn open_and_activate_commands_follow_tab_order() {
        let mut app = App::new();
        assert_eq!(
            app.dispatch(UiCommand::OpenNote {
                path: "a.md".into()
            }),
            vec![]
        );
        assert_eq!(
            app.dispatch(UiCommand::OpenNote {
                path: "b.md".into()
            }),
            vec![]
        );
        assert_eq!(app.state.tabs.active_path(), Some("b.md"));
        // Opening the same note twice never duplicates the tab.
        assert_eq!(
            app.dispatch(UiCommand::OpenNote {
                path: "a.md".into()
            }),
            vec![]
        );
        assert_eq!(app.state.tabs.len(), 2);
        // Activating position 0 focuses the first tab again.
        assert_eq!(app.dispatch(UiCommand::ActivateTab { position: 0 }), vec![]);
        assert_eq!(app.state.tabs.active_path(), Some("a.md"));
        // An out-of-range position is a no-op.
        assert_eq!(app.dispatch(UiCommand::ActivateTab { position: 9 }), vec![]);
    }

    #[test]
    fn close_command_falls_back_like_the_tab_model() {
        let mut app = app_with_tabs(&["a.md", "b.md", "c.md"]);
        assert_eq!(app.dispatch(UiCommand::CloseTab { position: 2 }), vec![]);
        assert_eq!(app.state.tabs.active_path(), Some("b.md"));
        assert_eq!(app.dispatch(UiCommand::CloseTab { position: 1 }), vec![]);
        assert_eq!(app.state.tabs.active_path(), Some("a.md"));
        // Out-of-range closes are no-ops.
        assert_eq!(app.dispatch(UiCommand::CloseTab { position: 5 }), vec![]);
        assert_eq!(app.state.tabs.len(), 1);
    }

    #[test]
    fn save_active_tab_emits_one_tracked_effect() {
        let mut app = app_with_tabs(&["a.md", "b.md"]);
        let effects = app.dispatch(UiCommand::SaveActiveTab);
        assert_eq!(effects.len(), 1);
        let b = tab_at(&app, 1);
        assert_eq!(effects[0].kind, EffectKind::SaveTab { tab: b });
        assert!(app.is_in_flight(effects[0].id));
        // With no tabs open there is nothing to save.
        let mut empty = App::new();
        assert_eq!(empty.dispatch(UiCommand::SaveActiveTab), vec![]);
    }

    #[test]
    fn save_completion_clears_the_conflict_and_failures_record_one() {
        let mut app = app_with_tabs(&["a.md"]);
        let tab = tab_at(&app, 0);
        app.state.conflicts.push(tab);
        let effects = app.dispatch(UiCommand::SaveActiveTab);
        let id = effects[0].id;
        app.handle(AppEvent::SaveCompleted { effect: id });
        assert!(app.state.conflicts.is_empty());
        assert!(!app.is_in_flight(id));

        // A failed save re-marks the conflict and surfaces the error.
        let effects = app.dispatch(UiCommand::SaveActiveTab);
        let id = effects[0].id;
        app.state.conflicts.clear();
        app.handle(AppEvent::SaveFailed {
            effect: id,
            error: "changed on disk".into(),
        });
        assert!(app.state.is_conflicted(tab));
        assert_eq!(app.state.last_error.as_deref(), Some("changed on disk"));
    }

    #[test]
    fn stale_or_duplicate_completions_are_ignored() {
        let mut app = app_with_tabs(&["a.md"]);
        let effects = app.dispatch(UiCommand::SaveActiveTab);
        let id = effects[0].id;
        app.handle(AppEvent::SaveCompleted { effect: id });
        // The same completion arriving twice must not panic or corrupt.
        app.handle(AppEvent::SaveCompleted { effect: id });
        // A completion for an id that was never dispatched is ignored.
        app.handle(AppEvent::SaveCompleted {
            effect: EffectId::new(999),
        });
        assert_eq!(app.state.last_error, None);
        assert!(!app.is_in_flight(id));
    }

    #[test]
    fn conflict_decisions_route_their_effect_and_clear_the_banner() {
        let mut app = app_with_tabs(&["a.md"]);
        let tab = tab_at(&app, 0);
        app.state.conflicts.push(tab);

        // Keep mine: just clear the banner.
        assert_eq!(
            app.dispatch(UiCommand::ResolveConflict {
                tab,
                decision: ConflictDecision::KeepMine
            }),
            vec![]
        );
        assert!(app.state.conflicts.is_empty());

        // Reload from disk: one tracked reload effect.
        app.state.conflicts.push(tab);
        let effects = app.dispatch(UiCommand::ResolveConflict {
            tab,
            decision: ConflictDecision::ReloadFromDisk,
        });
        assert_eq!(effects[0].kind, EffectKind::ReloadTab { tab });
        app.handle(AppEvent::ReloadCompleted {
            effect: effects[0].id,
            text: "fresh".into(),
        });
        assert!(app.state.conflicts.is_empty());
        assert!(!app.is_in_flight(effects[0].id));

        // Save mine anyway: one tracked save effect.
        app.state.conflicts.push(tab);
        let effects = app.dispatch(UiCommand::ResolveConflict {
            tab,
            decision: ConflictDecision::SaveMineAnyway,
        });
        assert_eq!(effects[0].kind, EffectKind::SaveTab { tab });
    }

    #[test]
    fn reload_completion_for_a_closed_tab_is_dropped() {
        let mut app = app_with_tabs(&["a.md"]);
        let tab = tab_at(&app, 0);
        app.state.conflicts.push(tab);
        let effects = app.dispatch(UiCommand::ResolveConflict {
            tab,
            decision: ConflictDecision::ReloadFromDisk,
        });
        // The tab is closed while the reload is in flight.
        app.state.tabs.close(tab);
        app.handle(AppEvent::ReloadCompleted {
            effect: effects[0].id,
            text: "fresh".into(),
        });
        assert!(!app.is_in_flight(effects[0].id));
    }
}
