//! File-tree view for the Windows sidebar.
//!
//! A thin view over the kernel: [`vault_app::tree`] builds the nodes and
//! owns the expansion/selection state ([`TreeState`]); this struct holds
//! the built nodes plus that state and exposes the flattened rows the
//! native TreeView renders. All decisions (sorting, folder-first order,
//! collapsed-by-default, visible-row flattening) are shared behavior,
//! unit-tested in the kernel.

use vault_app::settings::TreeSort;
use vault_app::tree::{self, TreeNode, TreeRow, TreeState};

/// The sidebar file-tree view.
#[derive(Debug, Default)]
pub struct FileTree {
    /// The built tree (folders first, sorted; kernel builder output).
    pub nodes: Vec<TreeNode>,
    /// Kernel-owned expansion and selection state.
    pub state: TreeState,
}

impl FileTree {
    /// Build the tree from the vault's note paths (the shared builder;
    /// folder chains materialize so they can receive new notes).
    #[must_use]
    pub fn from_paths(paths: &[String]) -> Self {
        let entries: Vec<tree::Entry<'_>> = paths
            .iter()
            .map(|path| tree::Entry::new(path, None))
            .collect();
        Self {
            nodes: tree::build_with_folders(&entries, &[], TreeSort::NameAsc),
            state: TreeState::default(),
        }
    }

    /// Expand or collapse the folder at `path` (kernel state).
    pub fn toggle_folder(&mut self, path: &str) {
        self.state.toggle(path);
    }

    /// Select the note or folder at `path` (kernel state).
    pub fn select(&mut self, path: impl Into<String>) {
        self.state.select(path);
    }

    /// The flattened rows the native TreeView renders: depth-first, with
    /// the children of collapsed folders hidden (kernel logic).
    #[must_use]
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        self.state.visible_rows(&self.nodes)
    }

    /// Every folder path in the tree, depth-first (kernel helper, used to
    /// expand-all or to seed the native control).
    #[must_use]
    pub fn folder_paths(&self) -> Vec<String> {
        tree::folder_paths(&self.nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        vec![
            "Notes/Glass.md".to_string(),
            "Notes/nested/Deep.md".to_string(),
            "root.md".to_string(),
        ]
    }

    #[test]
    fn from_paths_builds_the_shared_tree() {
        let tree = FileTree::from_paths(&paths());
        // The kernel builder's shape: folders first, then notes.
        let names: Vec<&str> = tree
            .nodes
            .iter()
            .map(|node| match node {
                TreeNode::Folder { name, .. } | TreeNode::Note { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(names, vec!["Notes", "root.md"]);
        assert_eq!(
            tree.folder_paths(),
            vec!["Notes".to_string(), "Notes/nested".to_string()]
        );
    }

    #[test]
    fn visible_rows_follow_the_kernel_state() {
        let mut tree = FileTree::from_paths(&paths());

        // Collapsed by default: top level only.
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "Notes");
        assert!(rows[0].is_folder);
        assert!(!rows[0].expanded);

        // Expanding reveals the subtree; the shared state drives it.
        tree.toggle_folder("Notes");
        let rows = tree.visible_rows();
        assert_eq!(
            rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            vec!["Notes", "Notes/nested", "Notes/Glass.md", "root.md"]
        );

        // Selection is kernel state too.
        tree.select("Notes/Glass.md");
        assert_eq!(tree.state.selected(), Some("Notes/Glass.md"));
    }

    #[test]
    fn an_empty_vault_builds_an_empty_tree() {
        let tree = FileTree::from_paths(&[]);
        assert!(tree.nodes.is_empty());
        assert!(tree.visible_rows().is_empty());
    }
}
