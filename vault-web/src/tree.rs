//! The index page's note tree: `<details>`/`<summary>` folders with note
//! links as leaves. Pure HTML + CSS — collapsible without JavaScript, and
//! the collapsed-by-default default matches the GUI sidebar.

use crate::page::{escape, path_encode};

/// One folder node: children are subfolders, `notes` are the notes directly
/// inside (not in subfolders).
pub struct TreeFolder {
    pub name: String,
    pub path: String,
    pub children: Vec<TreeFolder>,
    /// (vault-relative path, display title) of direct member notes.
    pub notes: Vec<(String, String)>,
}

impl TreeFolder {
    /// Note count including every subfolder (shown in the summary line).
    fn total_notes(&self) -> usize {
        self.notes.len()
            + self
                .children
                .iter()
                .map(|child| child.total_notes())
                .sum::<usize>()
    }
}

/// Build the forest of top-level folders from sorted folder paths and the
/// notes grouped by their exact folder (`""` = root, handled by the caller).
pub fn build(
    folders: &[String],
    notes_by_folder: &std::collections::BTreeMap<String, Vec<(String, String)>>,
) -> Vec<TreeFolder> {
    fn ensure<'a>(level: &'a mut Vec<TreeFolder>, name: &str, path: &str) -> &'a mut TreeFolder {
        if let Some(index) = level.iter().position(|folder| folder.name == name) {
            return &mut level[index];
        }
        level.push(TreeFolder {
            name: name.to_string(),
            path: path.to_string(),
            children: Vec::new(),
            notes: Vec::new(),
        });
        let last = level.len() - 1;
        &mut level[last]
    }

    fn find<'a>(level: &'a mut [TreeFolder], path: &str) -> Option<&'a mut TreeFolder> {
        for folder in level.iter_mut() {
            if folder.path == path {
                return Some(folder);
            }
            if path.starts_with(&format!("{}/", folder.path)) {
                if let Some(found) = find(&mut folder.children, path) {
                    return Some(found);
                }
            }
        }
        None
    }

    let mut roots: Vec<TreeFolder> = Vec::new();
    for folder in folders {
        let mut level: &mut Vec<TreeFolder> = &mut roots;
        let mut parent = String::new();
        for part in folder.split('/') {
            let path = if parent.is_empty() {
                part.to_string()
            } else {
                format!("{parent}/{part}")
            };
            level = &mut ensure(level, part, &path).children;
            parent = path;
        }
    }
    // Attach each folder's direct notes.
    for (folder, notes) in notes_by_folder {
        if folder.is_empty() {
            continue;
        }
        if let Some(node) = find(&mut roots, folder) {
            node.notes = notes.clone();
        }
    }
    fn sort_level(level: &mut Vec<TreeFolder>) {
        level.sort_by_key(|folder| folder.name.to_lowercase());
        for folder in level {
            sort_level(&mut folder.children);
        }
    }
    sort_level(&mut roots);
    roots
}

/// Render the forest to HTML: `<details>` per folder (with a recursive note
/// count), note links as leaves. `id` is the folder path so breadcrumbs can
/// link straight to a folder on the index page.
pub fn render(folders: &[TreeFolder]) -> String {
    fn walk(folders: &[TreeFolder], out: &mut String) {
        for folder in folders {
            let count = folder.total_notes();
            let count_html = if count > 0 {
                format!(" <span class=\"count\">({count})</span>")
            } else {
                String::new()
            };
            out.push_str(&format!(
                "<details id=\"{}\">\n<summary>{}{count_html}</summary>\n",
                path_encode(&folder.path),
                escape(&folder.name),
            ));
            if !folder.notes.is_empty() {
                out.push_str("<ul>\n");
                for (path, title) in &folder.notes {
                    out.push_str(&format!(
                        "<li><a href=\"/n/{}\">{}</a></li>\n",
                        path_encode(path),
                        escape(title)
                    ));
                }
                out.push_str("</ul>\n");
            }
            walk(&folder.children, out);
            out.push_str("</details>\n");
        }
    }
    let mut out = String::from("<div class=\"tree\">\n");
    walk(folders, &mut out);
    out.push_str("</div>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn groups(pairs: &[(&str, &str, &str)]) -> BTreeMap<String, Vec<(String, String)>> {
        let mut map: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for (folder, path, title) in pairs {
            map.entry((*folder).to_string())
                .or_default()
                .push(((*path).to_string(), (*title).to_string()));
        }
        map
    }

    fn collect_names(folders: &[TreeFolder]) -> Vec<&str> {
        folders.iter().map(|folder| folder.name.as_str()).collect()
    }

    #[test]
    fn nested_folders_build_a_forest() {
        let tree = build(
            &["Notes/Sub".to_string(), "Journal".to_string()],
            &groups(&[
                ("", "Top.md", "Top"),
                ("Notes", "Notes/N.md", "N"),
                ("Notes/Sub", "Notes/Sub/Deep.md", "Deep"),
            ]),
        );
        // Folders sorted by name; subfolders nested under parents.
        assert_eq!(collect_names(&tree), ["Journal", "Notes"]);
        let notes_folder = &tree[1];
        assert_eq!(collect_names(&notes_folder.children), ["Sub"]);
        assert_eq!(notes_folder.notes.len(), 1);
        assert_eq!(notes_folder.children[0].notes.len(), 1);
    }

    #[test]
    fn counts_are_recursive() {
        let tree = build(
            &["a".to_string(), "a/b".to_string()],
            &groups(&[
                ("a", "a/1.md", "1"),
                ("a/b", "a/b/2.md", "2"),
                ("a/b", "a/b/3.md", "3"),
            ]),
        );
        assert_eq!(tree[0].total_notes(), 3);
        assert_eq!(tree[0].children[0].total_notes(), 2);
    }

    #[test]
    fn render_emits_details_summary_leaves_and_ids() {
        let tree = build(
            &["Notes".to_string()],
            &groups(&[("Notes", "Notes/N.md", "My Note")]),
        );
        let html = render(&tree);
        assert!(html.contains("<div class=\"tree\">"), "{html}");
        assert!(html.contains("<details id=\"Notes\">"), "{html}");
        assert!(
            html.contains("<summary>Notes <span class=\"count\">(1)</span></summary>"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"/n/Notes/N.md\">My Note</a>"),
            "{html}"
        );
        assert!(html.ends_with("</details>\n</div>\n"), "{html}");
    }

    #[test]
    fn empty_folders_render_without_a_list() {
        let tree = build(&["Empty".to_string()], &groups(&[]));
        let html = render(&tree);
        assert!(html.contains("<summary>Empty</summary>"), "{html}");
        assert!(!html.contains("<ul>"), "{html}");
    }
}
