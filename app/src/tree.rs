//! File-tree model for the sidebar. Pure data plus a pure builder so it is
//! unit-testable without a GUI (M4 working rules).

/// One entry of the sidebar tree, keyed by vault-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeNode {
    Folder {
        name: String,
        path: String,
        children: Vec<TreeNode>,
    },
    Note {
        name: String,
        path: String,
    },
}

/// Build a sorted tree from vault-relative note paths ('/'-separated).
/// Folders come before notes; each level sorts case-insensitively by name.
pub fn build(paths: &[&str]) -> Vec<TreeNode> {
    #[derive(Default)]
    struct Folder {
        folders: std::collections::BTreeMap<String, Folder>,
        notes: Vec<String>,
    }

    fn insert(folder: &mut Folder, parts: &[&str]) {
        match parts {
            [name] => folder.notes.push((*name).to_string()),
            [head, rest @ ..] => {
                insert(folder.folders.entry((*head).to_string()).or_default(), rest)
            }
            [] => {}
        }
    }

    fn convert(path: &str, folder: &Folder) -> Vec<TreeNode> {
        let mut nodes: Vec<TreeNode> = folder
            .folders
            .iter()
            .map(|(name, child)| {
                let child_path = join(path, name);
                TreeNode::Folder {
                    name: name.clone(),
                    path: child_path.clone(),
                    children: convert(&child_path, child),
                }
            })
            .collect();
        nodes.extend(folder.notes.iter().map(|name| TreeNode::Note {
            name: name.clone(),
            path: join(path, name),
        }));
        nodes.sort_by_key(|node| match node {
            TreeNode::Folder { name, .. } => (0u8, name.to_lowercase()),
            TreeNode::Note { name, .. } => (1u8, name.to_lowercase()),
        });
        nodes
    }

    fn join(parent: &str, name: &str) -> String {
        if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        }
    }

    let mut root = Folder::default();
    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        insert(&mut root, &parts);
    }
    convert("", &root)
}

/// Every folder path in the tree, depth-first — used to start the sidebar
/// with all folders collapsed.
pub fn folder_paths(nodes: &[TreeNode]) -> Vec<String> {
    fn walk(nodes: &[TreeNode], out: &mut Vec<String>) {
        for node in nodes {
            if let TreeNode::Folder { path, children, .. } = node {
                out.push(path.clone());
                walk(children, out);
            }
        }
    }
    let mut paths = Vec::new();
    walk(nodes, &mut paths);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(nodes: &[TreeNode]) -> Vec<&str> {
        nodes
            .iter()
            .map(|node| match node {
                TreeNode::Folder { name, .. } | TreeNode::Note { name, .. } => name.as_str(),
            })
            .collect()
    }

    #[test]
    fn nested_paths_build_a_sorted_tree() {
        let tree = build(&[
            "Journal/2026/04.md",
            "Notes/Glass.md",
            "zeta.md",
            "Journal/index.md",
            "Alpha.md",
            "Notes/nested/Deep.md",
        ]);

        // Folders first, then notes; case-insensitive ("Alpha.md" < "zeta.md").
        // Obsidian's file explorer shows the extension.
        assert_eq!(names(&tree), ["Journal", "Notes", "Alpha.md", "zeta.md"]);

        let Some(TreeNode::Folder { children, .. }) = tree.first() else {
            panic!("first node must be a folder");
        };
        assert_eq!(names(children), ["2026", "index.md"]);

        let Some(TreeNode::Folder { children, .. }) = children.first() else {
            panic!("2026 must be a folder");
        };
        assert_eq!(names(children), ["04.md"]);
    }

    #[test]
    fn folder_paths_are_the_full_vault_relative_path() {
        let tree = build(&["a/b/c.md"]);
        let TreeNode::Folder { path, children, .. } = &tree[0] else {
            panic!();
        };
        assert_eq!(path, "a");
        let TreeNode::Folder { path, children, .. } = &children[0] else {
            panic!();
        };
        assert_eq!(path, "a/b");
        let TreeNode::Note { path, .. } = &children[0] else {
            panic!();
        };
        assert_eq!(path, "a/b/c.md");
    }

    #[test]
    fn empty_input_gives_an_empty_tree() {
        assert!(build(&[]).is_empty());
    }

    #[test]
    fn folder_paths_lists_every_folder_depth_first() {
        let tree = build(&["a/b/note.md", "a/top.md", "c.md"]);
        assert_eq!(
            folder_paths(&tree),
            vec!["a".to_string(), "a/b".to_string()]
        );
    }
}
