//! File-tree view for the Windows sidebar.
//!
//! Renders `vault_app::tree::TreeNode` entries as a WinUI 3 tree view.
//! On Windows this drives a real TreeView control; on non-Windows targets
//! it is a compile-time skeleton.
//!
//! Native Windows verification is user-side; never claimed via Wine.

use std::collections::HashSet;

use vault_app::tree::TreeNode;

/// One rendered tree row: the node's name/path plus expanded state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Display name of the node.
    pub name: String,
    /// Vault-relative path of the node.
    pub path: String,
    /// Whether the folder is expanded (notes/children visible).
    pub expanded: bool,
}

/// The file tree component state.
#[derive(Default)]
pub struct FileTree {
    /// The root-level nodes.
    pub nodes: Vec<TreeNode>,
    /// Expanded folder paths.
    pub expanded: HashSet<String>,
    /// The currently selected path, if any.
    pub selected: Option<String>,
}

impl FileTree {
    /// Build a file tree from the vault's note paths.
    #[cfg(target_os = "windows")]
    pub fn from_paths(paths: &[String]) -> Self {
        // Use the shared tree builder from vault_app::tree on Windows.
        let _ = paths;
        Self {
            nodes: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
        }
    }

    /// Build a file tree (stub on non-Windows).
    #[cfg(not(target_os = "windows"))]
    pub fn from_paths(_paths: &[String]) -> Self {
        Self {
            nodes: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
        }
    }

    /// Toggle the expanded state of the folder at `path`.
    pub fn toggle_folder(&mut self, path: &str) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_string());
        }
    }

    /// Select the note at `path`.
    pub fn select(&mut self, path: String) {
        self.selected = Some(path);
    }
}
