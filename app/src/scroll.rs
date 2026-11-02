//! Scroll-to-anchor plumbing for the reading view (M4b): outline clicks,
//! search-hit jumps and same-document `[[#heading]]` links all need to move
//! the reading scrollable to a specific top-level block.
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

/// The Id for top-level block `index` in the reading view.
pub fn block_id(index: usize) -> Id {
    Id::from(format!("block-{index}"))
}

/// Measures the anchor's position inside the reading scrollable.
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

/// A task that scrolls the reading view so top-level block `index` sits near
/// the top of the viewport. If the anchor or the scrollable is not in the
/// tree (e.g. the pane is in source mode), the task is a no-op.
pub fn to_block<Message: Send + 'static>(index: usize, margin: f32) -> Task<Message> {
    let scrollable_id = Id::new(READING_SCROLL);
    let measure = iced_task::widget::<Option<(f32, f32)>>(Measure {
        scrollable: scrollable_id.clone(),
        target: block_id(index),
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
