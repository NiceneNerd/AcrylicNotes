//! Layout view models shared by every frontend that renders split panes.
//!
//! The state math (divider clamping, narrow-width fold, pane assignment)
//! is behavior, not shell detail, so it lives here: the Windows shell
//! binds it to its native GridSplitter and the Linux frontend can adopt
//! the same model if it grows the feature (restructure plan §5.1).

/// How two panes are arranged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SplitOrientation {
    /// Side by side.
    #[default]
    Vertical,
    /// Stacked.
    Horizontal,
}

/// State of a two-pane split layout: arrangement, divider position,
/// narrow-width fold, and which note each pane shows.
///
/// The divider is a fraction of the available span, clamped to
/// `[0.1, 0.9]` so neither pane can collapse to zero by dragging.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitState {
    /// How the panes are arranged.
    pub orientation: SplitOrientation,
    /// Divider position as a fraction of the available span.
    pub divider: f32,
    /// Narrow-width fold: only one pane is shown.
    pub folded: bool,
    /// The first pane's note path, if any.
    pub left: Option<String>,
    /// The second pane's note path, if any.
    pub right: Option<String>,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            orientation: SplitOrientation::default(),
            divider: 0.5,
            folded: false,
            left: None,
            right: None,
        }
    }
}

impl SplitState {
    /// Move the divider by `delta` fractions of the span (negative moves
    /// toward the first pane), clamped to the usable range.
    pub fn move_divider(&mut self, delta: f32) {
        self.divider = (self.divider + delta).clamp(0.1, 0.9);
    }

    /// Toggle the narrow-width fold.
    pub fn toggle_folded(&mut self) {
        self.folded = !self.folded;
    }

    /// Set the first pane's note (`None` empties it).
    pub fn set_left(&mut self, path: Option<String>) {
        self.left = path;
    }

    /// Set the second pane's note (`None` empties it).
    pub fn set_right(&mut self, path: Option<String>) {
        self.right = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_divider_clamps_to_the_usable_range() {
        let mut split = SplitState::default();
        assert_eq!(split.divider, 0.5);

        split.move_divider(0.6);
        assert_eq!(split.divider, 0.9, "clamped at the right edge");

        split.move_divider(-2.0);
        assert_eq!(split.divider, 0.1, "clamped at the left edge");

        split.move_divider(0.05);
        assert!((split.divider - 0.15).abs() < 1e-6);
    }

    #[test]
    fn the_fold_toggles() {
        let mut split = SplitState::default();
        assert!(!split.folded);
        split.toggle_folded();
        assert!(split.folded);
        split.toggle_folded();
        assert!(!split.folded);
    }

    #[test]
    fn panes_hold_their_notes_independently() {
        let mut split = SplitState::default();
        split.set_left(Some("a.md".into()));
        split.set_right(Some("b.md".into()));
        assert_eq!(split.left.as_deref(), Some("a.md"));
        assert_eq!(split.right.as_deref(), Some("b.md"));
        split.set_right(None);
        assert_eq!(split.right, None);
        assert_eq!(split.left.as_deref(), Some("a.md"));
    }

    #[test]
    fn orientation_defaults_to_side_by_side() {
        assert_eq!(
            SplitState::default().orientation,
            SplitOrientation::Vertical
        );
        assert_eq!(SplitOrientation::default(), SplitOrientation::Vertical);
    }
}
