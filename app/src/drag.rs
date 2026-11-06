//! Drag-and-drop moves in the file tree: notes and folders are dragged
//! onto folder rows (or the tree's root area) to move them.
//!
//! This is deliberately NOT iced's `dnd_source`/`dnd_destination` widgets:
//! those are Wayland-protocol-based and unreliable on X11. Instead the
//! whole drag is app-level pointer state, so it behaves identically on
//! every platform iced runs on:
//!
//! 1. A left-press arms a candidate at the cursor position (global mouse
//!    subscription — a row's `Button` captures press events, so per-row
//!    detection is impossible).
//! 2. Movement past a threshold activates the drag; the source row is
//!    identified by hit-testing the press position against measured row
//!    bounds.
//! 3. While active, the hovered folder (or the tree area = vault root) is
//!    highlighted; release performs the move through the ordinary rename
//!    flow — so the rename link-update prompt comes for free.
//!
//! Row bounds come from a widget operation (see `scroll.rs` for the same
//! two-phase pattern): every visible tree row carries a `tree-row-*`
//! container id, and the tree area carries `tree-area`.

use cosmic::iced::core::widget::operation::{Operation, Outcome};
use cosmic::iced::core::widget::Id;
use cosmic::iced::core::{Point, Rectangle};
use cosmic::iced::runtime::task as iced_task;

/// The id prefix of a tree row's measured container. The kind rides in
/// the id: `f` for folders, `n` for notes (attachments carry no row id —
/// they are not draggable).
const ROW_PREFIX: &str = "tree-row-";

/// The id of the tree area container: the root hover/drop region.
pub const AREA_ID: &str = "tree-area";

/// Movement (in px) from the press position before a drag activates.
pub const DRAG_THRESHOLD: f32 = 6.0;

/// The widget id of a tree row's bounds-measuring container.
pub fn row_id(is_folder: bool, path: &str) -> Id {
    Id::from(format!(
        "{ROW_PREFIX}{}{path}",
        if is_folder { 'f' } else { 'n' }
    ))
}

/// Visible tree rows' bounds plus the tree area, measured in one layout
/// pass. Bounds are viewport coordinates captured at measure time.
#[derive(Debug, Clone, Default)]
pub struct TreeBounds {
    /// (path, is_folder, bounds) for every visible note/folder row.
    pub rows: Vec<(String, bool, Rectangle)>,
    /// The whole tree area (the root hover/drop region).
    pub area: Option<Rectangle>,
}

impl TreeBounds {
    /// The topmost row whose bounds contain `pos`, if any.
    pub fn row_at(&self, pos: Point) -> Option<(String, bool)> {
        self.rows
            .iter()
            .rev()
            .find(|(_, _, bounds)| bounds.contains(pos))
            .map(|(path, is_folder, _)| (path.clone(), *is_folder))
    }

    /// The drop target under `pos`: a folder row, the tree area (vault
    /// root, empty string), or nothing (outside the tree).
    pub fn hover_target(&self, pos: Point) -> Option<String> {
        for (path, is_folder, bounds) in self.rows.iter().rev() {
            if *is_folder && bounds.contains(pos) {
                return Some(path.clone());
            }
        }
        self.area
            .filter(|area| area.contains(pos))
            .map(|_| String::new())
    }
}

/// Collects every `tree-row-*` / `tree-area` container's bounds.
struct Collect(TreeBounds);

impl Operation<TreeBounds> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<TreeBounds>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        let Some(id) = id else { return };
        // Id exposes its string only through Display (fine: only
        // id-carrying containers reach the string allocation).
        let text = id.to_string();
        if text == AREA_ID {
            self.0.area = Some(bounds);
        } else if let Some(rest) = text.strip_prefix(ROW_PREFIX) {
            // `f/<path>` or `n/<path>` — the slash after the kind letter
            // belongs to the path encoding, not the vault path.
            let (kind, path) = rest.split_at(1);
            if let Some(path) = path.strip_prefix('/') {
                self.0.rows.push((path.to_string(), kind == "f", bounds));
            }
        }
    }

    fn finish(&self) -> Outcome<TreeBounds> {
        Outcome::Some(self.0.clone())
    }
}

/// A task that measures the tree rows' current bounds. Map the result
/// into a message and store it; see `VaultNotes::measure_tree_bounds`.
pub fn measure_tree_bounds() -> iced_task::Task<TreeBounds> {
    iced_task::widget::<TreeBounds>(Collect(TreeBounds::default()))
}

/// The vault path a drop produces: `target/Name` for a folder target,
/// `Name` for the root. `None` when the move is a no-op or refused
/// (same path, or a folder into itself/descendant).
pub fn drop_path(source: &str, is_folder: bool, target: &str) -> Option<String> {
    let name = source.rsplit_once('/').map_or(source, |(_, name)| name);
    let new_path = if target.is_empty() {
        name.to_string()
    } else {
        format!("{target}/{name}")
    };
    if new_path == source {
        return None;
    }
    if is_folder && new_path.starts_with(&format!("{source}/")) {
        return None;
    }
    Some(new_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn drop_paths_refuse_noops_and_self_nesting() {
        // Note into a folder, folder into a folder, both to the root.
        assert_eq!(
            drop_path("a/b.md", false, "Notes"),
            Some("Notes/b.md".to_string())
        );
        assert_eq!(
            drop_path("a/sub", true, "Notes"),
            Some("Notes/sub".to_string())
        );
        assert_eq!(drop_path("a", true, "Notes"), Some("Notes/a".to_string()));
        assert_eq!(drop_path("a/b.md", false, ""), Some("b.md".to_string()));
        assert_eq!(drop_path("a/sub", true, ""), Some("sub".to_string()));
        // Same path: a no-op, not a rename.
        assert_eq!(drop_path("Notes/b.md", false, "Notes"), None);
        assert_eq!(drop_path("b.md", false, ""), None);
        // A folder into itself or a descendant: refused.
        assert_eq!(drop_path("a", true, "a"), None);
        assert_eq!(drop_path("a", true, "a/sub"), None);
    }

    #[test]
    fn hover_and_row_hit_tests() {
        let bounds = TreeBounds {
            rows: vec![
                ("old".into(), true, rect(0.0, 0.0, 200.0, 24.0)),
                ("old/n.md".into(), false, rect(0.0, 24.0, 200.0, 24.0)),
                ("other".into(), true, rect(0.0, 48.0, 200.0, 24.0)),
            ],
            area: Some(rect(0.0, 0.0, 200.0, 200.0)),
        };
        // A folder row wins over the root area.
        assert_eq!(
            bounds.hover_target(Point::new(10.0, 8.0)),
            Some("old".into())
        );
        // Note rows fall through to the root area.
        assert_eq!(bounds.hover_target(Point::new(10.0, 30.0)), Some("".into()));
        // Outside the tree: no target, the drop cancels.
        assert_eq!(bounds.hover_target(Point::new(10.0, 300.0)), None);
        // Row identification by position.
        assert_eq!(
            bounds.row_at(Point::new(10.0, 30.0)),
            Some(("old/n.md".into(), false))
        );
        assert_eq!(
            bounds.row_at(Point::new(10.0, 55.0)),
            Some(("other".into(), true))
        );
        assert_eq!(bounds.row_at(Point::new(10.0, 300.0)), None);
    }
}
