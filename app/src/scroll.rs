//! Scroll-to-anchor plumbing for the reading view (M4b): outline clicks,
//! search-hit jumps and same-document `[[#heading]]` links all need to move
//! the reading scrollable to a specific top-level block. The live preview
//! pane (R3) reuses the same two-phase op with its own scrollable and
//! per-region anchors.
//!
//! iced exposes `scroll_to(id, AbsoluteOffset)` but not "scroll to widget",
//! so this runs a two-phase widget operation:
//!
//! 1. **Measure** — an operation walks the tree and captures the target
//!    block container's global Y and the reading scrollable's content Y
//!    (both in the same coordinate space during one layout pass).
//! 2. **Scroll** — `Task::then` turns the measured pair into a
//!    `scroll_to(offset = block_y - content_y - margin)` task.
//!
//! The reading scrollable and the block anchors are tagged in view/reading
//! (`reading-scroll`, `block-{index}`).

use cosmic::app::Task;
use cosmic::iced::core::widget::operation::scrollable::AbsoluteOffset;
use cosmic::iced::core::widget::operation::{Operation, Outcome};
use cosmic::iced::core::widget::Id;
use cosmic::iced::core::{Rectangle, Vector};
use cosmic::iced::runtime::task as iced_task;
use cosmic::Action;

/// Id of the reading pane's scrollable.
pub const READING_SCROLL: &str = "reading-scroll";

/// Id of the live preview pane's scrollable (R3 scroll-into-view).
pub const LIVE_SCROLL: &str = "live-scroll";

/// The Id for top-level block `index` in the reading view.
pub fn block_id(index: usize) -> Id {
    Id::from(format!("block-{index}"))
}

/// The Id for live-preview region `index` (tagged in `view::live_region`).
pub fn live_region_id(index: usize) -> Id {
    Id::from(format!("live-region-{index}"))
}

/// The Id of the focused region's block editor in the live preview.
///
/// This is *not* a shared constant on purpose. This build of iced
/// persists named-widget state in a global map and restores it by id
/// when the view is diffed, matching tree children by id — so an editor
/// id that stays constant while the editor hops between sibling
/// positions (clicking from one block into another) restores the
/// editor's state into the *old* tree node and leaves the node the
/// editor actually renders in holding the neighbouring region's state.
/// The next layout then downcasts the wrong state type and panics
/// ("Downcast widget state").
///
/// Per-region ids sidestep that: when focus moves, the old id vanishes
/// and the region falls back to positional diffing, where the
/// editor/mouse-area widget-type change forces a clean subtree rebuild.
/// While a region stays focused, its id and tree index are both
/// stable, so the normal by-id state persistence (cursor, focus)
/// still applies.
pub fn live_editor_id(region: usize) -> Id {
    Id::from(format!("live-editor-{region}"))
}

/// Measures the anchor's position inside the scrollable.
struct Measure {
    scrollable: Id,
    target: Id,
    block_y: Option<f32>,
    content_y: Option<f32>,
}

impl Operation<Option<(f32, f32)>> for Measure {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<(f32, f32)>>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.target) {
            self.block_y = Some(bounds.y);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn cosmic::iced::core::widget::operation::scrollable::Scrollable,
    ) {
        if id == Some(&self.scrollable) {
            self.content_y = Some(content_bounds.y);
        }
    }

    fn finish(&self) -> Outcome<Option<(f32, f32)>> {
        Outcome::Some(self.block_y.zip(self.content_y))
    }
}

/// A two-phase "scroll widget into view" task for one scrollable: measure
/// the anchor's and the scrollable content's positions during one layout
/// pass, then scroll by their difference (minus a top margin). If the
/// anchor or the scrollable is not in the tree, the task is a no-op.
fn scroll_to_anchor<Message: Send + 'static>(
    scrollable: &'static str,
    target: Id,
    margin: f32,
) -> Task<Message> {
    let scrollable_id = Id::new(scrollable);
    let measure = iced_task::widget::<Option<(f32, f32)>>(Measure {
        scrollable: scrollable_id.clone(),
        target,
        block_y: None,
        content_y: None,
    });
    let target = scrollable_id.clone();
    measure.then(move |found| match found {
        Some((block_y, content_y)) => {
            cosmic::iced::widget::scrollable::scroll_to::<Action<Message>>(
                target.clone(),
                AbsoluteOffset {
                    x: None,
                    y: Some((block_y - content_y - margin).max(0.0)),
                },
            )
        }
        None => Task::none(),
    })
}

/// A task that scrolls the reading view so top-level block `index` sits near
/// the top of the viewport. If the anchor or the scrollable is not in the
/// tree (e.g. the pane is in source mode), the task is a no-op.
pub fn to_block<Message: Send + 'static>(index: usize, margin: f32) -> Task<Message> {
    scroll_to_anchor::<Message>(READING_SCROLL, block_id(index), margin)
}

/// A task that scrolls the live pane so region `index` sits near the top
/// of the viewport. No-op when the region or the scrollable is missing
/// (e.g. the pane is not in live mode).
pub fn to_live_region<Message: Send + 'static>(index: usize, margin: f32) -> Task<Message> {
    scroll_to_anchor::<Message>(LIVE_SCROLL, live_region_id(index), margin)
}

/// A task that scrolls the file tree so the row of the note at `path`
/// sits near the top of the viewport. No-op when the row or the
/// scrollable is missing (the tree is not on screen, or the note is
/// not in it).
pub fn to_tree_note<Message: Send + 'static>(path: &str, margin: f32) -> Task<Message> {
    scroll_to_anchor::<Message>(
        crate::drag::SCROLL_ID,
        crate::drag::row_id(false, path),
        margin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_ids_encode_their_index() {
        // The ids must be stable across renders: the view tags each
        // region/block container with exactly these ids, and the
        // measure op looks them up by string equality.
        assert_eq!(block_id(2), Id::new("block-2"));
        assert_eq!(live_region_id(3), Id::new("live-region-3"));
        assert_eq!(live_editor_id(1), Id::new("live-editor-1"));
        // Region ids never collide with block ids, and editor ids never
        // collide with either.
        assert_ne!(block_id(1).clone(), live_region_id(1));
        assert_ne!(live_editor_id(1), live_region_id(1));
        assert_ne!(live_editor_id(1), block_id(1));
    }
}
