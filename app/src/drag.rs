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

use cosmic::iced::core::widget::operation::{self, Operation, Outcome};
use cosmic::iced::core::widget::Id;
use cosmic::iced::core::{Point, Rectangle, Vector};
use cosmic::iced::runtime::task as iced_task;

/// The id prefix of a tree row's measured container. The kind rides in
/// the id: `f` for folders, `n` for notes (attachments carry no row id —
/// they are not draggable).
const ROW_PREFIX: &str = "tree-row-";

/// The id of the sidebar's tree scrollable. Its translation converts the
/// content-space bounds reported by the rows into window coordinates.
pub const SCROLL_ID: &str = "tree-scroll";

/// The id of the tree area container: the root hover/drop region.
pub const AREA_ID: &str = "tree-area";

/// Movement (in px) from the press position before a drag activates.
pub const DRAG_THRESHOLD: f32 = 6.0;

/// The widget id of a tree row's bounds-measuring container. The kind
/// rides in the id — `f/<path>` for folders, `n/<path>` for notes.
pub fn row_id(is_folder: bool, path: &str) -> Id {
    let kind = if is_folder { 'f' } else { 'n' };
    Id::from(format!("{ROW_PREFIX}{kind}/{path}"))
}

/// The (is_folder, path) encoded in a `tree-row-*` id, if any.
fn parse_row_id(text: &str) -> Option<(bool, &str)> {
    let rest = text.strip_prefix(ROW_PREFIX)?;
    let (kind, path) = rest.split_at(1);
    let path = path.strip_prefix('/')?;
    Some((kind == "f", path))
}

/// Visible tree rows' bounds plus the tree area, measured in one layout
/// pass. Bounds are WINDOW coordinates captured at measure time (the
/// tree scrollable's translation is already subtracted), so they can be
/// hit-tested against the raw cursor position from the global mouse
/// subscription.
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
struct Collect {
    bounds: TreeBounds,
    /// The tree scrollable's translation, captured when its callback
    /// fires (after its children — DFS post-order).
    translation: Option<Vector>,
}

impl Operation<TreeBounds> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<TreeBounds>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        // Children inside a scrollable are laid out UNscrolled; iced
        // hands them a cursor shifted by exactly this translation
        // (scrollable.rs: `Cursor::Available(cursor + translation)`).
        // Our hit-testing uses the raw window cursor, so the same shift
        // is undone on the collected bounds in `finish`.
        if id.is_some_and(|id| id.to_string() == SCROLL_ID) {
            self.translation = Some(translation);
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        let Some(id) = id else { return };
        // Id exposes its string only through Display (fine: only
        // id-carrying containers reach the string allocation).
        let text = id.to_string();
        if text == AREA_ID {
            self.bounds.area = Some(bounds);
        } else if let Some((is_folder, path)) = parse_row_id(&text) {
            self.bounds.rows.push((path.to_string(), is_folder, bounds));
        }
    }

    fn finish(&self) -> Outcome<TreeBounds> {
        let mut bounds = self.bounds.clone();
        if let Some(t) = self.translation {
            // window = content − translation.
            for (_, _, rect) in &mut bounds.rows {
                rect.x -= t.x;
                rect.y -= t.y;
            }
            if let Some(area) = &mut bounds.area {
                area.x -= t.x;
                area.y -= t.y;
            }
        }
        Outcome::Some(bounds)
    }
}

/// A task that measures the tree rows' current bounds. Map the result
/// into a message and store it; see `VaultNotes::measure_tree_bounds`.
pub fn measure_tree_bounds() -> iced_task::Task<TreeBounds> {
    iced_task::widget::<TreeBounds>(Collect {
        bounds: TreeBounds::default(),
        translation: None,
    })
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

    #[test]
    fn row_ids_encode_kind_and_path_for_the_collector() {
        assert_eq!(row_id(true, "old").to_string(), "tree-row-f/old");
        assert_eq!(row_id(false, "old/n.md").to_string(), "tree-row-n/old/n.md");
        assert_eq!(parse_row_id("tree-row-f/old"), Some((true, "old")));
        assert_eq!(
            parse_row_id("tree-row-n/old/n.md"),
            Some((false, "old/n.md"))
        );
        // The area id is not a row.
        assert_eq!(parse_row_id(AREA_ID), None);
        // The kind letter must be followed by a separator: this is the
        // encoding `row_id` once emitted (`tree-row-fold`), which the
        // collector silently dropped — leaving no rows to hit-test and
        // killing every drag. Pinned so a format drift fails loudly.
        assert_eq!(parse_row_id("tree-row-fold"), None);
    }
}

#[cfg(test)]
mod collect_tests {
    //! Runs the real `Collect` operation through iced's layout engine
    //! (null renderer): pins that id-carrying containers report their
    //! bounds with the right paths and kinds, and that the tree
    //! scrollable's translation converts content-space bounds into
    //! window coordinates (the scrolled case).

    use super::*;
    use cosmic::iced::core::layout::{Layout, Limits, Node};
    use cosmic::iced::core::widget::operation as op;
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::widget::{container, scrollable, Column};
    use cosmic::iced::{Element, Length, Size, Theme};

    type Elem = Element<'static, (), Theme, ()>;

    fn row(path: &str, is_folder: bool) -> Elem {
        container(Column::new())
            .id(row_id(is_folder, path))
            .width(Length::Fill)
            .height(Length::Fixed(24.0))
            .into()
    }

    /// The tree: an area container (the scrollable's body) holding three
    /// rows plus enough filler to scroll (272px of content, 100px
    /// viewport) — mirroring `tree_view`'s structure.
    fn tree() -> Elem {
        let rows = Column::new()
            .width(Length::Fill)
            .push(row("old", true))
            .push(row("old/n.md", false))
            .push(row("deep/n.md", false))
            .push(Column::new().height(Length::Fixed(200.0)));
        container(
            scrollable::Scrollable::new(container(rows).id(Id::new(AREA_ID)).width(Length::Fill))
                .id(Id::new(SCROLL_ID)),
        )
        .width(Length::Fixed(200.0))
        .height(Length::Fixed(100.0))
        .into()
    }

    /// Lay out like the runtime does, then run `Collect` over the tree.
    /// The Tree is built once and shared: the scrollable's state (its
    /// scroll offset) lives in it, so scroll-then-measure must reuse it.
    /// Mirrors the runtime exactly: `operate` first, then `finish` —
    /// that second call is where the scroll translation correction
    /// lands. `black_box` swallows `finish`, so it runs on the inner op
    /// (the runtime reaches it through `operation::map`).
    fn collect(elem: &mut Elem, tree: &mut Tree, node: &Node) -> TreeBounds {
        let mut collect = Collect {
            bounds: TreeBounds::default(),
            translation: None,
        };
        {
            let mut boxed = op::black_box(&mut collect);
            elem.as_widget_mut()
                .operate(tree, Layout::new(node), &(), &mut boxed);
        }
        match collect.finish() {
            Outcome::Some(bounds) => bounds,
            _ => panic!("Collect must produce its bounds from finish"),
        }
    }

    /// Scrolls the tree scrollable to an absolute offset, the way the
    /// runtime's scroll_to task would.
    struct ScrollTo(f32);

    impl Operation<()> for ScrollTo {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
            _translation: Vector,
            state: &mut dyn op::Scrollable,
        ) {
            if id.is_some_and(|id| id.to_string() == SCROLL_ID) {
                state.scroll_to(op::scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(self.0),
                });
            }
        }
    }

    fn scroll_to(elem: &mut Elem, tree: &mut Tree, node: &Node, y: f32) {
        let mut scroll = ScrollTo(y);
        let mut boxed = op::black_box(&mut scroll);
        elem.as_widget_mut()
            .operate(tree, Layout::new(node), &(), &mut boxed);
    }

    fn y_of(bounds: &TreeBounds, path: &str) -> f32 {
        bounds
            .rows
            .iter()
            .find(|(p, _, _)| p == path)
            .map(|(_, _, r)| r.y)
            .unwrap_or(f32::NAN)
    }

    #[test]
    fn collect_gathers_rows_and_area_in_window_coordinates() {
        let mut elem = tree();
        let mut tree = Tree::new(elem.as_widget());
        let node = elem.as_widget_mut().layout(
            &mut tree,
            &(),
            &Limits::new(Size::ZERO, Size::new(200.0, 100.0)),
        );
        let bounds = collect(&mut elem, &mut tree, &node);
        let paths: Vec<&str> = bounds.rows.iter().map(|(p, _, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["old", "old/n.md", "deep/n.md"]);
        assert!(bounds.rows[0].1, "old is a folder");
        assert!(!bounds.rows[1].1, "old/n.md is a note");
        // Unscrolled: content and window coordinates coincide.
        assert_eq!(y_of(&bounds, "old"), 0.0);
        assert_eq!(y_of(&bounds, "old/n.md"), 24.0);
        assert_eq!(y_of(&bounds, "deep/n.md"), 48.0);
        let area = bounds.area.expect("area collected");
        assert_eq!((area.x, area.y, area.width), (0.0, 0.0, 200.0));
    }

    #[test]
    fn collect_converts_scrolled_bounds_to_window_coordinates() {
        let mut elem = tree();
        let mut tree = Tree::new(elem.as_widget());
        let node = elem.as_widget_mut().layout(
            &mut tree,
            &(),
            &Limits::new(Size::ZERO, Size::new(200.0, 100.0)),
        );
        // Scroll within the SAME tree: the offset lives in its state.
        scroll_to(&mut elem, &mut tree, &node, 100.0);
        let bounds = collect(&mut elem, &mut tree, &node);
        // Scrolled by 100: every row shifts up by exactly that amount,
        // matching the cursor space the global mouse subscription uses.
        assert_eq!(y_of(&bounds, "old"), -100.0);
        assert_eq!(y_of(&bounds, "old/n.md"), -76.0);
        assert_eq!(y_of(&bounds, "deep/n.md"), -52.0);
        assert_eq!(bounds.area.expect("area collected").y, -100.0);
    }
}
