//! The native WinUI shell component (p11).
//!
//! One Reactor component over the shared [`WindowsApp`] kernel: native
//! events become [`WindowsMessage`]s (the same envelope the runner
//! consumes), `update` hands them to `WindowsApp::handle_native`, and
//! `view` renders kernel state into WinUI controls. This file holds no
//! feature logic — decisions stay in the kernel reducer; the shell only
//! maps controls to commands.
//!
//! Headless verification lives in `tests/ui_headless.rs`: the shell runs
//! under the `RecordingRuntime` (built for windows-gnu, executed under
//! Wine via `cargo xtask test-windows-ui`), where clicks are injected as
//! recorded events and the reconciled tree is asserted.

use std::cell::RefCell;
use std::rc::Rc;

use windows_reactor::{
    Button, ChildrenControl, Component, ComponentContext, ContentControl, KeyedView, Orientation,
    StackPanel, TextBlock, TextBox, TreeNode, TreeView, TreeViewExt, TreeViewSelectionMode, View,
    ViewContext,
};

use vault_app::commands::UiCommand;
use vault_app::tree as kernel_tree;
use vault_md::parse_note;

use crate::app::WindowsApp;
use crate::file_tree::FileTree;
use crate::reading::WindowsMarkdownRenderer;
use crate::reading_view::doc_view;
use crate::windows_message::{NativeEvent, WindowsMessage};

/// The shell component's input: a shared handle to the kernel host.
///
/// Pointer equality (the component never re-creates when the same app
/// handle is re-passed).
#[derive(Clone)]
pub struct ShellHandle(Rc<RefCell<WindowsApp>>);

impl PartialEq for ShellHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl ShellHandle {
    /// Wrap the kernel host for shell construction.
    #[must_use]
    pub fn new(app: Rc<RefCell<WindowsApp>>) -> Self {
        Self(app)
    }
}

/// The native shell: tabs, editor, and error line over the shared kernel.
pub struct Shell {
    handle: ShellHandle,
    /// The editor text as last rendered programmatically. A real WinUI
    /// control raises `TextChanged` for property sets as well as user
    /// typing; an incoming change equal to this is that echo of the
    /// shell's own sync and is suppressed instead of being fed back to
    /// the buffer (plan §11.1). `RefCell` because `view` (which takes
    /// `&self`) records it while `update` compares against it. Only the
    /// source-TextBox branch records it: while the reading view is
    /// shown no TextBox exists, so no echoes can be generated.
    rendered_editor: RefCell<String>,
    /// Whether the editor area shows the rendered reading view instead
    /// of the source TextBox. Shell-local presentation state (the
    /// kernel tracks pane layout, not this toggle); flipped by the
    /// `ToggleReading` native event, which `update` intercepts.
    reading: bool,
    /// The vault's file tree: kernel `TreeNode`s plus `TreeState`
    /// (expansion/selection). Built once from the vault's note paths;
    /// expansion state persists across renders here.
    file_tree: FileTree,
}

impl Component for Shell {
    type Input = ShellHandle;
    type Message = WindowsMessage;

    fn create(input: &Self::Input, _context: &ComponentContext<Self>) -> Self {
        let file_tree = FileTree::from_paths(&input.0.borrow().note_paths());
        Shell {
            handle: input.clone(),
            rendered_editor: RefCell::new(String::new()),
            reading: false,
            file_tree,
        }
    }

    fn input_changed(&mut self, input: &Self::Input, _context: &ComponentContext<Self>) {
        self.handle = input.clone();
    }

    fn update(&mut self, message: Self::Message, _context: &ComponentContext<Self>) {
        if let WindowsMessage::Native(NativeEvent::ToggleReading) = &message {
            // Shell-local presentation state: flip the editor area
            // between the source TextBox and the rendered reading view.
            // The kernel never sees this.
            self.reading = !self.reading;
            return;
        }
        if let WindowsMessage::Native(NativeEvent::EditorEdited { text }) = &message {
            // Echo suppression: equal to the last rendered text means
            // the echo of our own programmatic sync (initial text set,
            // tab switch, reload), not a user edit.
            if *text == *self.rendered_editor.borrow() {
                return;
            }
        }
        self.handle.0.borrow_mut().handle_native(message);
    }

    fn view(&self, _input: &Self::Input, context: &mut ViewContext<Self>) -> View {
        let app = self.handle.0.borrow();
        let state = &app.shared.state;

        // File-tree sidebar: one TreeView node per vault folder/note,
        // keyed by vault path. Invoking a note row asks the kernel to
        // open it (`UiCommand::OpenNote`); expansion state lives in
        // `FileTree`'s `TreeState` and persists across renders.
        let tree_view = TreeView::new()
            .selection_mode(TreeViewSelectionMode::Single)
            .on_item_invoked(
                context
                    .callback(|path: String| WindowsMessage::Command(UiCommand::OpenNote { path })),
            )
            .nodes(build_tree_nodes(&self.file_tree));

        // Tab bar: one activate button and one close button per tab,
        // keyed by tab id so reconciliation preserves identity as tabs
        // open and close. The active tab's label is bracketed.
        let tabs: Vec<KeyedView> = state
            .tabs
            .order()
            .iter()
            .map(|&tab| {
                let path = state.tabs.path(tab).unwrap_or("?").to_string();
                let is_active = state.tabs.active() == Some(tab);
                let label = if is_active {
                    format!("[{path}]")
                } else {
                    path.clone()
                };
                let position = state.tabs.position(tab).unwrap_or(0);
                let activate = Button::new()
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::ActivateTab { position })
                    }))
                    .content(TextBlock::new().text(label));
                let close = Button::new()
                    .on_click(context.callback(move |()| {
                        WindowsMessage::Command(UiCommand::CloseTab { position })
                    }))
                    .content(TextBlock::new().text(format!("close {path}")));
                // The tab path doubles as the key: the kernel never
                // holds two tabs for one path (open_or_focus dedupes).
                KeyedView::new(
                    path.clone(),
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .children([activate, close]),
                )
            })
            .collect();

        // Error line: the kernel's last actionable error (empty when
        // everything is fine).
        let error_line = TextBlock::new().text(
            state
                .last_error
                .clone()
                .unwrap_or_else(|| String::from("(no errors)")),
        );

        // Reading/source toggle (shell-local presentation state): the
        // button offers the view you would switch TO.
        let reading_label = if self.reading { "source" } else { "reading" };
        let reading_toggle = Button::new()
            .on_click(context.callback(|()| WindowsMessage::Native(NativeEvent::ToggleReading)))
            .content(TextBlock::new().text(reading_label));

        // Editor area: the source TextBox, or — when the reading toggle
        // is on and a tab is active — the parsed buffer rendered as
        // native reading elements. The doc is parsed per render from
        // the current buffer text (the Linux app caches per edit;
        // caching is a later slice). With no active buffer the source
        // editor renders empty, reading toggle or not.
        let active_buffer = state.tabs.active().and_then(|tab| app.buffers.get(&tab));
        let (editor_key, editor_area) = match (self.reading, active_buffer) {
            (true, Some(buffer)) => {
                let doc = parse_note(&buffer.text);
                (
                    "reading-view",
                    doc_view(&doc, &WindowsMarkdownRenderer::default()),
                )
            }
            _ => {
                // User edits arrive as `TextBoxTextChanged` callbacks and
                // become `EditorEdited` native events; the buffer update
                // and echo suppression live in `update`. Composition and
                // caret/selection tracking are later adapter slices
                // (plan §11.1); this is Stage A sync.
                let editor_text = active_buffer.map(|b| b.text.clone()).unwrap_or_default();
                *self.rendered_editor.borrow_mut() = editor_text.clone();
                let editor = TextBox::new()
                    .on_text_changed(context.callback(|text: String| {
                        WindowsMessage::Native(NativeEvent::EditorEdited { text })
                    }))
                    .text(editor_text);
                ("editor", View::from(editor))
            }
        };

        StackPanel::new()
            .orientation(Orientation::Vertical)
            .keyed_children([
                KeyedView::new("tree-view", tree_view),
                KeyedView::new("error-line", error_line),
                KeyedView::new(
                    "tab-bar",
                    StackPanel::new()
                        .orientation(Orientation::Vertical)
                        .keyed_children(tabs),
                ),
                // Save button (the toolbar stand-in for Ctrl+S).
                KeyedView::new(
                    "save",
                    Button::new()
                        .on_click(
                            context
                                .callback(|()| WindowsMessage::Command(UiCommand::SaveActiveTab)),
                        )
                        .content(TextBlock::new().text("save")),
                ),
                KeyedView::new("reading-toggle", reading_toggle),
                KeyedView::new(editor_key, editor_area),
            ])
    }
}

/// Convert the kernel tree (folders + notes, expansion state from
/// `TreeState`) into the hierarchy the native TreeView renders. Folder
/// and note keys are their vault paths; invoking a note row reports its
/// path to `on_item_invoked`.
fn build_tree_nodes(tree: &FileTree) -> Vec<TreeNode> {
    fn convert(node: &kernel_tree::TreeNode, tree: &FileTree) -> TreeNode {
        match node {
            kernel_tree::TreeNode::Folder {
                name,
                path,
                children,
            } => TreeNode::new(path.as_str(), name.as_str())
                .expanded(tree.state.is_expanded(path))
                .children(children.iter().map(|child| convert(child, tree))),
            kernel_tree::TreeNode::Note { name, path } => {
                TreeNode::new(path.as_str(), name.as_str())
            }
        }
    }
    tree.nodes.iter().map(|node| convert(node, tree)).collect()
}
