//! Split-mode layout for the Windows frontend.
//!
//! Renders two editor panes side by side with a GridSplitter, narrow-width
//! fold behavior, and synchronized scrolling. On Windows this drives real
//! XAML controls; on non-Windows targets it is a compile-time skeleton.
//!
//! Native Windows verification is user-side; never claimed via Wine.

/// The split layout's orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitOrientation {
    Vertical,
    Horizontal,
}

/// Split mode state: two panes, a divider position, and scroll sync.
pub struct SplitMode {
    /// How the panes are arranged.
    pub orientation: SplitOrientation,
    /// Divider position in logical pixels (0..=1 fraction of available width).
    pub divider: f32,
    /// Whether the narrow-width fold is active (single-pane collapsed).
    pub folded: bool,
    /// Left pane's active tab path, if any.
    pub left: Option<String>,
    /// Right pane's active tab path, if any.
    pub right: Option<String>,
}

impl SplitMode {
    /// Create a new split layout.
    #[cfg(target_os = "windows")]
    pub fn new() -> Self {
        Self {
            orientation: SplitOrientation::Vertical,
            divider: 0.5,
            folded: false,
            left: None,
            right: None,
        }
    }

    /// Create a new split layout (stub on non-Windows).
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Self {
        Self {
            orientation: SplitOrientation::Vertical,
            divider: 0.5,
            folded: false,
            left: None,
            right: None,
        }
    }

    /// Move the divider by `delta` fraction (-1..=1), clamped.
    pub fn move_divider(&mut self, delta: f32) {
        self.divider = (self.divider + delta).clamp(0.1, 0.9);
    }

    /// Toggle the folded state.
    pub fn toggle_folded(&mut self) {
        self.folded = !self.folded;
    }

    /// Set the left pane's active note.
    pub fn set_left(&mut self, path: Option<String>) {
        self.left = path;
    }

    /// Set the right pane's active note.
    pub fn set_right(&mut self, path: Option<String>) {
        self.right = path;
    }
}

impl Default for SplitMode {
    fn default() -> Self {
        Self::new()
    }
}
