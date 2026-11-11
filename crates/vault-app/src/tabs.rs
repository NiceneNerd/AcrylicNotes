//! Shared tab-bar state: ordering, activation, and identity.
//!
//! Tabs are identified by [`TabId`], never by vector position: a tab can
//! move, and async work addressed to it must keep pointing at the same
//! note (restructure plan §5.1). The native frontends keep their own
//! editor payloads keyed by `TabId`; this model is the single source of
//! truth for which tabs exist, in what order, and which is active.

use std::collections::HashMap;

use crate::ids::{IdAllocator, TabId};

/// What opening a note in the tab bar did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabOpenOutcome {
    /// The note already had a tab; it is now the active tab.
    Focused(TabId),
    /// A new tab was appended (and is now the active tab).
    Opened(TabId),
}

/// The ordered open tabs with one active tab.
///
/// The rules mirror the cosmic frontend's historical behavior exactly:
/// activating an existing path focuses its tab instead of duplicating it;
/// closing the active tab falls back to the neighbor that took its
/// position (the next tab, or the last one when at the end); a deletion
/// that removes tabs restores the previously active tab when it survives,
/// else the last remaining tab.
#[derive(Debug, Default)]
pub struct Tabs {
    order: Vec<TabId>,
    active: Option<TabId>,
    paths: HashMap<TabId, String>,
    allocator: IdAllocator,
}

impl Tabs {
    /// An empty tab bar.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a note: focus its existing tab, or append a new one (which
    /// becomes active). Every tab gets a fresh stable id.
    pub fn open_or_focus(&mut self, path: &str) -> TabOpenOutcome {
        if let Some(id) = self.id_of(path) {
            return TabOpenOutcome::Focused(id);
        }
        let id = self.allocator.allocate();
        self.paths.insert(id, path.to_string());
        self.order.push(id);
        self.active = Some(id);
        TabOpenOutcome::Opened(id)
    }

    /// Make `id` active. Returns the previously active tab when the
    /// active tab changed, so a frontend can drop per-tab native state
    /// (live-edit regions, focus) exactly when it must.
    pub fn activate(&mut self, id: TabId) -> Option<TabId> {
        if self.order.contains(&id) {
            let previous = self.active;
            if previous != Some(id) {
                self.active = Some(id);
                previous
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Open a note, returning its id regardless of whether the tab already
    /// existed (the id is what the frontend needs to attach a payload to).
    pub fn open(&mut self, path: &str) -> TabId {
        match self.open_or_focus(path) {
            TabOpenOutcome::Focused(id) | TabOpenOutcome::Opened(id) => id,
        }
    }

    /// Remove a tab. When it was active, the tab that took its position
    /// becomes active (the next tab, or the last one when the closed tab
    /// was at the end). Returns the active tab after the removal.
    pub fn close(&mut self, id: TabId) -> Option<TabId> {
        let Some(position) = self.position(id) else {
            return self.active;
        };
        self.order.remove(position);
        self.paths.remove(&id);
        if self.active == Some(id) {
            self.active = self
                .order
                .get(position)
                .or_else(|| self.order.last())
                .copied();
        }
        self.active
    }

    /// Drop every tab whose path fails `keep`. When the active tab
    /// survives it stays active; otherwise the last remaining tab
    /// becomes active. Returns the removed ids.
    pub fn retain_paths(&mut self, keep: impl Fn(&str) -> bool) -> Vec<TabId> {
        let kept: Vec<TabId> = self
            .order
            .iter()
            .copied()
            .filter(|&id| self.paths.get(&id).is_some_and(|p| keep(p)))
            .collect();
        let removed: Vec<TabId> = self
            .order
            .iter()
            .copied()
            .filter(|id| !kept.contains(id))
            .collect();
        let kept_active = self.active.filter(|id| kept.contains(id));
        self.order = kept;
        self.paths.retain(|id, _| self.order.contains(id));
        self.active = kept_active.or_else(|| self.order.last().copied());
        removed
    }

    /// Drop every tab (e.g. a different vault was opened).
    pub fn clear(&mut self) {
        self.order.clear();
        self.active = None;
        self.paths.clear();
    }

    /// Re-path every tab showing `old` (a note rename/move).
    pub fn rename_path(&mut self, old: &str, new: &str) {
        for path in self.paths.values_mut() {
            if path == old {
                *path = new.to_string();
            }
        }
    }

    /// Re-path every tab whose path starts with `old_prefix` (a folder
    /// rename/move; pass the trailing slash, as in `folder/`).
    pub fn rename_path_prefix(&mut self, old_prefix: &str, new_prefix: &str) {
        for path in self.paths.values_mut() {
            if let Some(rest) = path.strip_prefix(old_prefix) {
                *path = format!("{new_prefix}{rest}");
            }
        }
    }

    /// The active tab, if any.
    #[must_use]
    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    /// The active tab's note path, if any.
    #[must_use]
    pub fn active_path(&self) -> Option<&str> {
        self.active.and_then(|id| self.path(id))
    }

    /// The tab showing `path`, if any.
    #[must_use]
    pub fn id_of(&self, path: &str) -> Option<TabId> {
        self.order
            .iter()
            .copied()
            .find(|&id| self.paths.get(&id).is_some_and(|p| p == path))
    }

    /// The id at a display position (the tab bar is drawn in this order).
    #[must_use]
    pub fn id_at(&self, position: usize) -> Option<TabId> {
        self.order.get(position).copied()
    }

    /// The display position of `id`.
    #[must_use]
    pub fn position(&self, id: TabId) -> Option<usize> {
        self.order.iter().position(|&existing| existing == id)
    }

    /// The note path shown in `id`.
    #[must_use]
    pub fn path(&self, id: TabId) -> Option<&str> {
        self.paths.get(&id).map(String::as_str)
    }

    /// How many tabs are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether no tabs are open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The ids in display order.
    #[must_use]
    pub fn order(&self) -> &[TabId] {
        &self.order
    }

    /// Iterate `(id, path)` in display order.
    pub fn iter(&self) -> impl Iterator<Item = (TabId, &str)> {
        self.order
            .iter()
            .map(|&id| (id, self.paths.get(&id).map(String::as_str).unwrap_or("")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> TabId {
        TabId::new(raw)
    }

    #[test]
    fn opening_focuses_existing_tabs_and_appends_new_ones() {
        let mut tabs = Tabs::new();
        assert_eq!(
            tabs.open_or_focus("notes/a.md"),
            TabOpenOutcome::Opened(id(0))
        );
        assert_eq!(
            tabs.open_or_focus("notes/b.md"),
            TabOpenOutcome::Opened(id(1))
        );
        // A second open of the same note never duplicates the tab.
        assert_eq!(
            tabs.open_or_focus("notes/a.md"),
            TabOpenOutcome::Focused(id(0))
        );
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active(), Some(id(1)));
    }

    #[test]
    fn activate_reports_the_previous_tab_only_on_change() {
        let mut tabs = Tabs::new();
        let a = match tabs.open_or_focus("a.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let b = match tabs.open_or_focus("b.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        // Activating the already-active tab reports no change.
        assert_eq!(tabs.activate(b), None);
        // Activating another tab reports the previous one.
        assert_eq!(tabs.activate(a), Some(b));
        assert_eq!(tabs.active(), Some(a));
        // Activating an unknown id is a no-op.
        assert_eq!(tabs.activate(id(99)), None);
        assert_eq!(tabs.active(), Some(a));
    }

    #[test]
    fn closing_the_active_tab_falls_back_to_its_position() {
        let mut tabs = Tabs::new();
        let a = match tabs.open_or_focus("a.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let b = match tabs.open_or_focus("b.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let c = match tabs.open_or_focus("c.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        // Close the middle tab while it is active: the next tab takes over.
        tabs.activate(b);
        assert_eq!(tabs.close(b), Some(c));
        assert_eq!(tabs.order(), &[a, c]);
        // Close the last tab while it is active: the previous one takes over.
        assert_eq!(tabs.close(c), Some(a));
        // Close the last remaining tab: nothing is active.
        assert_eq!(tabs.close(a), None);
        assert!(tabs.is_empty());
    }

    #[test]
    fn closing_an_inactive_tab_keeps_the_active_one() {
        let mut tabs = Tabs::new();
        let a = match tabs.open_or_focus("a.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let b = match tabs.open_or_focus("b.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let c = match tabs.open_or_focus("c.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        tabs.activate(b);
        // Remove the first tab: the same tab stays active even though the
        // index-based view shifted.
        assert_eq!(tabs.close(a), Some(b));
        assert_eq!(tabs.order(), &[b, c]);
        // Removing an unknown id changes nothing.
        assert_eq!(tabs.close(id(77)), Some(b));
    }

    #[test]
    fn retaining_paths_keeps_a_surviving_active_tab_else_the_last() {
        let mut tabs = Tabs::new();
        let a = match tabs.open_or_focus("dir/a.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let b = match tabs.open_or_focus("dir/b.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let c = match tabs.open_or_focus("other/c.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        // The active tab survives: it stays active.
        tabs.activate(b);
        let removed = tabs.retain_paths(|path| path != "dir/a.md");
        assert_eq!(removed, vec![a]);
        assert_eq!(tabs.active(), Some(b));
        // The active tab is deleted: the last remaining tab becomes active.
        let removed = tabs.retain_paths(|path| path != "dir/b.md");
        assert_eq!(removed, vec![b]);
        assert_eq!(tabs.active(), Some(c));
        // Everything deleted: nothing is active.
        let removed = tabs.retain_paths(|_| false);
        assert_eq!(removed.len(), 1);
        assert_eq!(tabs.active(), None);
        assert!(tabs.is_empty());
    }

    #[test]
    fn positions_paths_and_lookups_stay_in_sync() {
        let mut tabs = Tabs::new();
        let a = match tabs.open_or_focus("a.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        let b = match tabs.open_or_focus("b.md") {
            TabOpenOutcome::Opened(id) => id,
            _ => unreachable!(),
        };
        assert_eq!(tabs.id_at(0), Some(a));
        assert_eq!(tabs.id_at(1), Some(b));
        assert_eq!(tabs.id_at(2), None);
        assert_eq!(tabs.position(b), Some(1));
        assert_eq!(tabs.path(a), Some("a.md"));
        assert_eq!(tabs.id_of("b.md"), Some(b));
        assert_eq!(tabs.id_of("missing.md"), None);
        assert_eq!(
            tabs.iter().collect::<Vec<_>>(),
            vec![(a, "a.md"), (b, "b.md")]
        );
        tabs.clear();
        assert!(tabs.is_empty());
        assert_eq!(tabs.active(), None);
    }
}
