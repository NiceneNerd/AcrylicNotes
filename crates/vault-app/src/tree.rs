//! File-tree model for the sidebar. Pure data plus a pure builder so it is
//! unit-testable without a GUI (M4 working rules).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::SystemTime;

use crate::settings::TreeSort;

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

/// One file of the tree: a note or an attachment.
#[derive(Debug, Clone)]
pub struct Entry<'a> {
    /// Vault-relative path ('/'-separated).
    pub path: &'a str,
    /// When the file was last changed. `None` for attachments: the
    /// vault does not timestamp them, so they cannot join a date order.
    pub modified: Option<SystemTime>,
}

impl<'a> Entry<'a> {
    /// A tree entry for `path`, changed at `modified`.
    pub fn new(path: &'a str, modified: Option<SystemTime>) -> Self {
        Self { path, modified }
    }
}

/// Build the sidebar tree from the vault's files.
///
/// Folders come before notes and always sort by name; the notes follow
/// `sort`. The folders in `folder_paths` materialize even when they hold
/// nothing, so they can receive new notes.
pub fn build_with_folders(
    entries: &[Entry<'_>],
    folder_paths: &[&str],
    sort: TreeSort,
) -> Vec<TreeNode> {
    #[derive(Default)]
    struct Folder {
        folders: std::collections::BTreeMap<String, Folder>,
        notes: Vec<(String, Option<SystemTime>)>,
    }

    fn insert(folder: &mut Folder, parts: &[&str], modified: Option<SystemTime>) {
        match parts {
            [name] => folder.notes.push(((*name).to_string(), modified)),
            [head, rest @ ..] => insert(
                folder.folders.entry((*head).to_string()).or_default(),
                rest,
                modified,
            ),
            [] => {}
        }
    }

    /// Ensure a folder chain exists without adding a note leaf.
    fn ensure_folder(folder: &mut Folder, parts: &[&str]) {
        if let [head, rest @ ..] = parts {
            ensure_folder(folder.folders.entry((*head).to_string()).or_default(), rest);
        }
    }

    fn convert(path: &str, folder: &mut Folder, sort: TreeSort) -> Vec<TreeNode> {
        let mut nodes: Vec<TreeNode> = folder
            .folders
            .iter_mut()
            .map(|(name, child)| {
                let child_path = join(path, name);
                TreeNode::Folder {
                    name: name.clone(),
                    path: child_path.clone(),
                    children: convert(&child_path, child, sort),
                }
            })
            .collect();
        // Folders always come first, sorted by name — the order the
        // user picked applies to the notes only.
        nodes.sort_by_key(|node| match node {
            TreeNode::Folder { name, .. } => name.to_lowercase(),
            TreeNode::Note { .. } => String::new(),
        });
        sort_notes(&mut folder.notes, sort);
        nodes.extend(folder.notes.iter().map(|(name, _)| TreeNode::Note {
            name: name.clone(),
            path: join(path, name),
        }));
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
    for entry in entries {
        let parts: Vec<&str> = entry.path.split('/').collect();
        insert(&mut root, &parts, entry.modified);
    }
    for path in folder_paths {
        let parts: Vec<&str> = path.split('/').collect();
        ensure_folder(&mut root, &parts);
    }
    convert("", &mut root, sort)
}

/// Order the notes of one folder by the chosen criterion.
fn sort_notes(notes: &mut [(String, Option<SystemTime>)], sort: TreeSort) {
    match sort {
        TreeSort::NameAsc => notes.sort_by_key(name_key),
        TreeSort::NameDesc => notes.sort_by_key(|note| std::cmp::Reverse(name_key(note))),
        // The date orders break ties by name, so the order is stable.
        TreeSort::Newest => notes
            .sort_by(|a, b| time_key(a.1, b.1, true).then_with(|| name_key(a).cmp(&name_key(b)))),
        TreeSort::Oldest => notes
            .sort_by(|a, b| time_key(a.1, b.1, false).then_with(|| name_key(a).cmp(&name_key(b)))),
    }
}

/// The case-insensitive name of a note.
fn name_key(note: &(String, Option<SystemTime>)) -> String {
    note.0.to_lowercase()
}

/// Compare two modification times. A file without a timestamp (an
/// attachment) sorts last, whichever order is chosen.
fn time_key(a: Option<SystemTime>, b: Option<SystemTime>, newest_first: bool) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            if newest_first {
                y.cmp(&x)
            } else {
                x.cmp(&y)
            }
        }
    }
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

/// The folders that hold `path`: the chain from the vault root down to
/// the file's own folder ("a/b/note.md" → ["a", "a/b"]).
///
/// These are the folders that have to stay open for the file to show.
pub fn ancestor_paths(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    let mut out = Vec::new();
    let mut prefix = String::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        out.push(prefix.clone());
    }
    out
}

/// One flattened row of the visible tree: the node a sidebar renders,
/// with its nesting depth and expansion state resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Display name of the node (extension included for notes).
    pub name: String,
    /// Vault-relative, '/'-separated path.
    pub path: String,
    /// Nesting depth (0 = vault root level).
    pub depth: usize,
    /// Whether the row is a folder.
    pub is_folder: bool,
    /// For folders: whether the children are currently shown. Always
    /// `false` for notes.
    pub expanded: bool,
}

/// Expansion and selection state for a sidebar tree.
///
/// Kernel-owned so both frontends fold, select, and flatten identically
/// (restructure plan §5.1: native widgets hold no duplicate decisions).
/// Folders start collapsed; the row flattening skips the children of
/// collapsed folders, mirroring the Linux sidebar.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TreeState {
    expanded: HashSet<String>,
    selected: Option<String>,
}

impl TreeState {
    /// Show the folder at `path` (idempotent).
    pub fn expand(&mut self, path: &str) {
        self.expanded.insert(path.to_string());
    }

    /// Hide the folder at `path`. Returns whether it was expanded before
    /// (children of collapsed folders disappear from the visible rows).
    pub fn collapse(&mut self, path: &str) -> bool {
        self.expanded.remove(path)
    }

    /// Expand the folder at `path` when collapsed, collapse it when
    /// expanded (the click on a folder row).
    pub fn toggle(&mut self, path: &str) {
        if !self.collapse(path) {
            self.expand(path);
        }
    }

    /// Whether the folder at `path` is expanded.
    #[must_use]
    pub fn is_expanded(&self, path: &str) -> bool {
        self.expanded.contains(path)
    }

    /// Select the note or folder at `path`.
    pub fn select(&mut self, path: impl Into<String>) {
        self.selected = Some(path.into());
    }

    /// The selected path, if any.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Depth-first visible rows of `nodes`: a folder's children appear
    /// only while the folder is expanded. The same nodes always produce
    /// the same rows for the same state, on every platform.
    #[must_use]
    pub fn visible_rows(&self, nodes: &[TreeNode]) -> Vec<TreeRow> {
        fn walk(state: &TreeState, nodes: &[TreeNode], depth: usize, out: &mut Vec<TreeRow>) {
            for node in nodes {
                match node {
                    TreeNode::Folder {
                        name,
                        path,
                        children,
                    } => {
                        let expanded = state.is_expanded(path);
                        out.push(TreeRow {
                            name: name.clone(),
                            path: path.clone(),
                            depth,
                            is_folder: true,
                            expanded,
                        });
                        if expanded {
                            walk(state, children, depth + 1, out);
                        }
                    }
                    TreeNode::Note { name, path } => out.push(TreeRow {
                        name: name.clone(),
                        path: path.clone(),
                        depth,
                        is_folder: false,
                        expanded: false,
                    }),
                }
            }
        }
        let mut out = Vec::new();
        walk(self, nodes, 0, &mut out);
        out
    }
}

/// A pending sidebar-tree edit, started from the right-click menu (M4c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEditKind {
    /// Create a note inside `folder` (empty string = vault root).
    NewNote { folder: String },
    /// Create a folder inside `folder` (empty string = vault root).
    NewFolder { folder: String },
    /// Rename the note or folder at `old`.
    Rename { old: String },
}

/// A pending tree edit plus the name the user has typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEdit {
    pub kind: TreeEditKind,
    pub value: String,
}

impl TreeEdit {
    /// The vault-relative path this edit would produce from `value`
    /// (extensionless for folders; notes gain `.md` at submit time).
    pub fn target_path(&self) -> String {
        let name = self.value.trim().trim_matches('/');
        match &self.kind {
            TreeEditKind::NewNote { folder } | TreeEditKind::NewFolder { folder } => {
                if folder.is_empty() {
                    name.to_string()
                } else {
                    format!("{folder}/{name}")
                }
            }
            TreeEditKind::Rename { old } => match old.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/{name}"),
                None => name.to_string(),
            },
        }
    }
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

    /// The entries of a tree, without timestamps.
    fn entries<'a>(paths: &[&'a str]) -> Vec<Entry<'a>> {
        paths.iter().map(|path| Entry::new(path, None)).collect()
    }

    #[test]
    fn nested_paths_build_a_sorted_tree() {
        let tree = build_with_folders(
            &entries(&[
                "Journal/2026/04.md",
                "Notes/Glass.md",
                "zeta.md",
                "Journal/index.md",
                "Alpha.md",
                "Notes/nested/Deep.md",
            ]),
            &[],
            TreeSort::NameAsc,
        );

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
        let tree = build_with_folders(&entries(&["a/b/c.md"]), &[], TreeSort::NameAsc);
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
        assert!(build_with_folders(&[], &[], TreeSort::NameAsc).is_empty());
    }

    #[test]
    fn folder_paths_lists_every_folder_depth_first() {
        let tree = build_with_folders(
            &entries(&["a/b/note.md", "a/top.md", "c.md"]),
            &[],
            TreeSort::NameAsc,
        );
        assert_eq!(
            folder_paths(&tree),
            vec!["a".to_string(), "a/b".to_string()]
        );
    }

    #[test]
    fn ancestor_paths_are_the_folders_that_hold_a_file() {
        assert_eq!(ancestor_paths("a/b/note.md"), vec!["a", "a/b"]);
        assert_eq!(ancestor_paths("a/note.md"), vec!["a"]);
        // A file at the vault root sits in no folder at all.
        assert_eq!(ancestor_paths("note.md"), Vec::<String>::new());
    }

    #[test]
    fn empty_folders_materialize_without_note_leaves() {
        let tree = build_with_folders(
            &entries(&["Notes/Glass.md", "zeta.md"]),
            &["Notes/Empty", "Deep/a/b"],
            TreeSort::NameAsc,
        );
        assert_eq!(names(&tree), ["Deep", "Notes", "zeta.md"]);

        let Some(TreeNode::Folder { path, children, .. }) = tree.first() else {
            panic!("first node must be a folder");
        };
        assert_eq!(path, "Deep");
        // The empty chain exists with no notes inside.
        let Some(TreeNode::Folder {
            path, children: _, ..
        }) = children.first()
        else {
            panic!("nested empty folder");
        };
        assert_eq!(path, "Deep/a");
        // Deep, Deep/a, Deep/a/b, Notes, Notes/Empty
        assert_eq!(folder_paths(&tree).len(), 5);
    }

    #[test]
    fn empty_folders_merge_with_note_folders() {
        // A folder path that already exists via notes must not duplicate.
        let tree = build_with_folders(&entries(&["Notes/Glass.md"]), &["Notes"], TreeSort::NameAsc);
        assert_eq!(names(&tree), ["Notes"]);
        assert_eq!(folder_paths(&tree), vec!["Notes".to_string()]);
    }

    /// A timestamp `seconds` after the epoch.
    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)
    }

    #[test]
    fn the_name_orders_sort_the_notes_both_ways() {
        let paths = ["b.md", "A.md", "c.md"];
        let tree = build_with_folders(&entries(&paths), &[], TreeSort::NameAsc);
        assert_eq!(names(&tree), ["A.md", "b.md", "c.md"]);

        let tree = build_with_folders(&entries(&paths), &[], TreeSort::NameDesc);
        assert_eq!(names(&tree), ["c.md", "b.md", "A.md"]);
    }

    #[test]
    fn the_date_orders_use_the_modification_time() {
        let paths = ["old.md", "new.md", "middle.md"];
        let entries = vec![
            Entry::new(paths[0], Some(at(100))),
            Entry::new(paths[1], Some(at(300))),
            Entry::new(paths[2], Some(at(200))),
        ];

        // Newest first: the most recent change leads.
        let tree = build_with_folders(&entries, &[], TreeSort::Newest);
        assert_eq!(names(&tree), ["new.md", "middle.md", "old.md"]);

        // Oldest first: the other way round.
        let tree = build_with_folders(&entries, &[], TreeSort::Oldest);
        assert_eq!(names(&tree), ["old.md", "middle.md", "new.md"]);
    }

    #[test]
    fn the_folders_sort_by_name_whatever_the_note_order_is() {
        let paths = ["b.md", "Folder/a.md", "a.md"];
        for sort in TreeSort::ALL {
            let tree = build_with_folders(&entries(&paths), &[], sort);
            // The folder leads, whatever the notes do.
            assert_eq!(names(&tree)[0], "Folder", "{sort:?}");
        }
    }

    #[test]
    fn a_file_without_a_timestamp_sorts_last() {
        // Attachments carry no timestamp: they cannot join a date order,
        // so they sink to the bottom.
        let paths = ["note.md", "image.png"];
        let entries = vec![
            Entry::new(paths[0], Some(at(100))),
            Entry::new(paths[1], None),
        ];
        for sort in [TreeSort::Newest, TreeSort::Oldest] {
            let tree = build_with_folders(&entries, &[], sort);
            assert_eq!(names(&tree), ["note.md", "image.png"], "{sort:?}");
        }
    }

    #[test]
    fn notes_of_the_same_age_keep_their_name_order() {
        let paths = ["b.md", "a.md"];
        let entries = vec![
            Entry::new(paths[0], Some(at(100))),
            Entry::new(paths[1], Some(at(100))),
        ];
        for sort in [TreeSort::Newest, TreeSort::Oldest] {
            let tree = build_with_folders(&entries, &[], sort);
            assert_eq!(names(&tree), ["a.md", "b.md"], "{sort:?}");
        }
    }

    #[test]
    fn tree_edit_target_paths() {
        let note = TreeEdit {
            kind: TreeEditKind::NewNote {
                folder: "Notes".to_string(),
            },
            value: " New ".to_string(),
        };
        assert_eq!(note.target_path(), "Notes/New");

        let root_folder = TreeEdit {
            kind: TreeEditKind::NewFolder {
                folder: String::new(),
            },
            value: "Docs".to_string(),
        };
        assert_eq!(root_folder.target_path(), "Docs");

        let rename = TreeEdit {
            kind: TreeEditKind::Rename {
                old: "Notes/Glass.md".to_string(),
            },
            value: "Prism".to_string(),
        };
        assert_eq!(rename.target_path(), "Notes/Prism");

        let rename_root = TreeEdit {
            kind: TreeEditKind::Rename {
                old: "top.md".to_string(),
            },
            value: "up".to_string(),
        };
        assert_eq!(rename_root.target_path(), "up");
    }

    // --- TreeState -------------------------------------------------------------

    fn sample_tree() -> Vec<TreeNode> {
        build_with_folders(
            &entries(&["Notes/Glass.md", "Notes/nested/Deep.md", "root.md"]),
            &[],
            TreeSort::NameAsc,
        )
    }

    fn row_summary(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                format!(
                    "{}{}{}",
                    "-".repeat(row.depth),
                    row.name,
                    if row.expanded { "+" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn folders_start_collapsed_and_toggle_open() {
        let nodes = sample_tree();
        let mut state = TreeState::default();

        // Collapsed by default: only the top level is visible.
        let rows = state.visible_rows(&nodes);
        assert_eq!(
            row_summary(&rows),
            vec!["Notes", "root.md"],
            "collapsed folders hide their children"
        );

        // Toggling the folder reveals its subtree (one level at a time;
        // inside "Notes" the subfolder sorts before the notes).
        state.toggle("Notes");
        assert!(state.is_expanded("Notes"));
        let rows = state.visible_rows(&nodes);
        assert_eq!(
            row_summary(&rows),
            vec!["Notes+", "-nested", "-Glass.md", "root.md"]
        );

        // Toggling again collapses it back.
        state.toggle("Notes");
        assert!(!state.is_expanded("Notes"));
        assert_eq!(
            row_summary(&state.visible_rows(&nodes)),
            vec!["Notes", "root.md"]
        );
    }

    #[test]
    fn nested_expansion_flattens_depth_first() {
        let nodes = sample_tree();
        let mut state = TreeState::default();
        state.expand("Notes");
        state.expand("Notes/nested");

        let rows = state.visible_rows(&nodes);
        assert_eq!(
            row_summary(&rows),
            vec!["Notes+", "-nested+", "--Deep.md", "-Glass.md", "root.md"]
        );
        // Row paths stay vault-relative and full.
        assert_eq!(rows[3].path, "Notes/Glass.md");
        assert!(!rows[3].is_folder);
    }

    #[test]
    fn selection_is_kernel_state() {
        let mut state = TreeState::default();
        assert_eq!(state.selected(), None);
        state.select("Notes/Glass.md");
        assert_eq!(state.selected(), Some("Notes/Glass.md"));
        state.select("root.md");
        assert_eq!(state.selected(), Some("root.md"));
    }

    #[test]
    fn collapsing_a_parent_hides_but_keeps_nested_expansion() {
        let nodes = sample_tree();
        let mut state = TreeState::default();
        state.expand("Notes");
        state.expand("Notes/nested");
        // Collapsing the parent hides the child rows...
        assert!(state.collapse("Notes"));
        assert_eq!(
            row_summary(&state.visible_rows(&nodes)),
            vec!["Notes", "root.md"]
        );
        // ...but the child's expansion state survives re-expansion.
        state.expand("Notes");
        assert!(state.is_expanded("Notes/nested"));
        assert_eq!(
            row_summary(&state.visible_rows(&nodes)),
            vec!["Notes+", "-nested+", "--Deep.md", "-Glass.md", "root.md"]
        );
        // Collapsing an already-collapsed folder reports false.
        assert!(!state.collapse("root.md"));
    }
}
