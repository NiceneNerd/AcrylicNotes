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
    /// the tab they act on (`None` for tab-less effects such as opening
    /// external targets). Used to reject stale completions.
    in_flight: HashMap<EffectId, Option<TabId>>,
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
            UiCommand::SetPaneMode { mode } => {
                self.state.pane = mode;
                Vec::new()
            }
            UiCommand::SetSidebarVisible { open } => {
                self.state.sidebar_open = open;
                Vec::new()
            }
            UiCommand::SetNavSection { section } => {
                self.state.panel = section;
                Vec::new()
            }
            UiCommand::SetSearchQuery { query } => {
                self.state.search.set_query(query);
                Vec::new()
            }
            UiCommand::SearchVault { query } => {
                self.state.search.set_query(query.clone());
                let request = self.state.search.begin_request();
                // The search completion is keyed by the request id inside
                // the effect; the session itself rejects stale requests.
                let id = self.effect_ids.allocate();
                vec![Effect::new(id, EffectKind::SearchVault { request, query })]
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
                self.in_flight.insert(id, Some(tab));
                vec![Effect::new(id, EffectKind::SaveTab { tab })]
            }
            UiCommand::OpenExternal { target } => {
                let id = self.effect_ids.allocate();
                self.in_flight.insert(id, None);
                vec![Effect::new(id, EffectKind::OpenExternal { target })]
            }
            UiCommand::ResolveConflict { tab, decision } => match decision {
                crate::persistence::ConflictDecision::KeepMine => {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                    Vec::new()
                }
                crate::persistence::ConflictDecision::ReloadFromDisk => {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                    let id = self.effect_ids.allocate();
                    self.in_flight.insert(id, Some(tab));
                    vec![Effect::new(id, EffectKind::ReloadTab { tab })]
                }
                crate::persistence::ConflictDecision::SaveMineAnyway => {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                    let id = self.effect_ids.allocate();
                    self.in_flight.insert(id, Some(tab));
                    vec![Effect::new(id, EffectKind::SaveTab { tab })]
                }
            },
        }
    }

    /// Consume an executed effect's result. Events for unknown (stale or
    /// already-completed) effect ids are ignored.
    pub fn handle(&mut self, event: AppEvent) {
        // Search completions are keyed by the session's request id: the
        // session itself drops stale requests, so there is nothing for
        // the in-flight table to check here.
        if let AppEvent::SearchCompleted { request, hits } = event {
            let _applied = self.state.search.complete(request, hits);
            return;
        }
        let (effect_id, tab) = match &event {
            AppEvent::SaveCompleted { effect }
            | AppEvent::SaveFailed { effect, .. }
            | AppEvent::ReloadCompleted { effect, .. }
            | AppEvent::OpenCompleted { effect }
            | AppEvent::OpenFailed { effect, .. } => match self.in_flight.get(effect) {
                Some(&tab) => (*effect, tab),
                None => return, // stale or duplicate completion
            },
            // Handled above.
            AppEvent::SearchCompleted { .. } => return,
        };
        match event {
            AppEvent::SaveCompleted { .. } => {
                // The disk now matches the buffer: the conflict is resolved.
                if let Some(tab) = tab {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                }
            }
            AppEvent::SaveFailed { error, .. } => {
                // A refused save (externally modified or I/O error) leaves
                // the tab conflicted until the user decides.
                if let Some(tab) = tab {
                    if !self.state.is_conflicted(tab) {
                        self.state.conflicts.push(tab);
                    }
                }
                self.state.last_error = Some(error);
            }
            AppEvent::ReloadCompleted { .. } => {
                // The note was re-read; the conflict is resolved (the
                // frontend applies the returned text to its payload).
                if let Some(tab) = tab {
                    self.state.conflicts.retain(|&candidate| candidate != tab);
                }
            }
            // The platform took the open request; nothing to record.
            AppEvent::OpenCompleted { .. } => {}
            AppEvent::OpenFailed { error, .. } => {
                self.state.last_error = Some(error);
            }
            // Handled above.
            AppEvent::SearchCompleted { .. } => {}
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
            app.dispatch(UiCommand::SetPaneMode {
                mode: crate::settings::PaneMode::Split
            }),
            vec![]
        );
        assert_eq!(app.state.pane, crate::settings::PaneMode::Split);
        assert_eq!(
            app.dispatch(UiCommand::SetSidebarVisible { open: false }),
            vec![]
        );
        assert!(!app.state.sidebar_open);
        assert_eq!(
            app.dispatch(UiCommand::SetNavSection {
                section: crate::navigation::NavSection::Tags
            }),
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

    #[test]
    fn search_commands_update_the_query_and_run_tracked_requests() {
        use crate::effects::EffectKind as Kind;
        let mut app = App::new();
        // Typing only records the query.
        assert_eq!(
            app.dispatch(UiCommand::SetSearchQuery {
                query: "needle".into()
            }),
            vec![]
        );
        assert_eq!(app.state.search.query(), "needle");
        assert!(app.state.search.hits().is_empty());

        // Submitting begins a request and emits one search effect.
        let effects = app.dispatch(UiCommand::SearchVault {
            query: "needle".into(),
        });
        assert_eq!(effects.len(), 1);
        let request = match &effects[0].kind {
            Kind::SearchVault { request, query } => {
                assert_eq!(query, "needle");
                *request
            }
            other => panic!("unexpected effect: {other:?}"),
        };
        assert!(app.state.search.is_current(request));

        // The completion replaces the hits.
        let hits = vec![crate::search::SearchHitView {
            path: "a.md".into(),
            line_no: 2,
            line: "needle here".into(),
        }];
        app.handle(AppEvent::SearchCompleted {
            request,
            hits: hits.clone(),
        });
        assert_eq!(app.state.search.hits(), hits.as_slice());

        // A newer request makes the first completion stale.
        let effects = app.dispatch(UiCommand::SearchVault {
            query: "needle".into(),
        });
        let newer = match &effects[0].kind {
            Kind::SearchVault { request, .. } => *request,
            other => panic!("unexpected effect: {other:?}"),
        };
        app.handle(AppEvent::SearchCompleted {
            request,
            hits: vec![],
        });
        assert_eq!(app.state.search.hits().len(), 1, "stale completion dropped");
        app.handle(AppEvent::SearchCompleted {
            request: newer,
            hits: vec![],
        });
        assert!(app.state.search.hits().is_empty());
    }

    #[test]
    fn open_external_commands_track_their_effect_until_completion() {
        use crate::effects::EffectKind as Kind;
        let mut app = App::new();
        let target = vault_platform::OpenTarget::Url("https://example.com".into());

        // The command emits exactly one tracked effect.
        let effects = app.dispatch(UiCommand::OpenExternal {
            target: target.clone(),
        });
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].kind, Kind::OpenExternal { target });
        assert!(app.is_in_flight(effects[0].id));

        // Success drains the tracking and records nothing.
        let id = effects[0].id;
        app.handle(AppEvent::OpenCompleted { effect: id });
        assert!(!app.is_in_flight(id));
        assert_eq!(app.state.last_error, None);

        // A failure surfaces on the error line and drains the tracking.
        let effects = app.dispatch(UiCommand::OpenExternal {
            target: vault_platform::OpenTarget::File("/tmp/a.png".into()),
        });
        let id = effects[0].id;
        app.handle(AppEvent::OpenFailed {
            effect: id,
            error: "opener xdg-open not found".into(),
        });
        assert!(!app.is_in_flight(id));
        assert_eq!(
            app.state.last_error.as_deref(),
            Some("opener xdg-open not found")
        );
    }

    #[test]
    fn stale_or_unknown_open_completions_are_ignored() {
        let mut app = App::new();
        let effects = app.dispatch(UiCommand::OpenExternal {
            target: vault_platform::OpenTarget::Url("https://example.com".into()),
        });
        let id = effects[0].id;
        app.handle(AppEvent::OpenCompleted { effect: id });

        // The same completion twice and a never-dispatched id both land
        // nowhere: no error line text appears from stale events.
        app.handle(AppEvent::OpenCompleted { effect: id });
        app.handle(AppEvent::OpenFailed {
            effect: crate::ids::EffectId::new(999),
            error: "stale failure".into(),
        });
        assert_eq!(app.state.last_error, None);
    }
}
