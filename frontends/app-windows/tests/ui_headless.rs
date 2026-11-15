//! Headless UI tests for the Windows frontend.
//!
//! windows-reactor's `RecordingRuntime` (feature `test`) runs the same
//! component code the real WinUI loop runs, but records every native
//! command into an inspectable tree instead of talking to WinUI. The test
//! binaries are built for `x86_64-pc-windows-gnu` and executed under Wine
//! (`cargo run -p xtask -- test-windows-ui`), so the UI logic is verified
//! on every push without a Windows machine. What Wine cannot verify
//! (real WinUI rendering, IME, DPI, theming, pickers, MSIX packaging)
//! stays user-side on real hardware.
//!
//! The message-injection pattern below is the seed for p11: a component
//! stashes its `LocalSender` (thread-local) in `create`, the test sends
//! kernel-shaped messages through it, and asserts the re-rendered tree.
#![cfg(target_os = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use windows_reactor::test::{NodeId, PropertyId, PropertyValue, Pump, RecordingRuntime};
use windows_reactor::{
    ChildrenControl, Component, ComponentContext, KeyedView, LocalSender, Orientation, StackPanel,
    TextBlock, View, ViewContext,
};

/// The sidebar file-tree view (kernel state, see `file_tree.rs`).
use app_windows::file_tree::FileTree;

#[test]
fn headless_mount_records_the_tree() {
    let mut pump = Pump::new(RecordingRuntime::default());
    let view = StackPanel::new()
        .orientation(Orientation::Vertical)
        .keyed_children([
            KeyedView::new("first", TextBlock::new().text("hello")),
            KeyedView::new("second", TextBlock::new().text("world")),
        ]);
    pump.mount_view(view).expect("mount");
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    // Walk: window -> panel -> texts.
    let panel = rt.node(window).expect("window recorded").children()[0];
    let kids = rt.node(panel).expect("panel recorded").children();
    assert_eq!(kids.len(), 2, "both texts recorded");
    let first = rt.node(kids[0]).expect("first text recorded");
    assert_eq!(
        first.property(PropertyId::TextBlockText),
        Some(&PropertyValue::Str(String::from("hello")))
    );
    let second = rt.node(kids[1]).expect("second text recorded");
    assert_eq!(
        second.property(PropertyId::TextBlockText),
        Some(&PropertyValue::Str(String::from("world")))
    );
}

// --- component message round-trip -------------------------------------------

/// Shell-shaped component: messages arrive through the (stashed) sender,
/// state changes, and the view re-renders. This is the same flow the real
/// shell will use for kernel commands.
struct Counter {
    count: i32,
}

#[derive(Clone, Debug)]
enum CounterMessage {
    Increment,
}

thread_local! {
    /// Populated by the test, filled in by `Counter::create`: the test's
    /// handle into the component's message queue.
    static COUNTER_SENDER: RefCell<Option<SenderSink>> = const { RefCell::new(None) };
}

/// Shared sink for the component's message sender (see `create`).
type SenderSink = Rc<RefCell<Option<LocalSender<CounterMessage>>>>;

impl Component for Counter {
    type Input = ();
    type Message = CounterMessage;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let sender = context.sender();
        COUNTER_SENDER.with(|cell| {
            if let Some(sink) = &*cell.borrow() {
                *sink.borrow_mut() = Some(sender.clone());
            }
        });
        Counter { count: 0 }
    }

    fn update(&mut self, message: CounterMessage, _context: &ComponentContext<Self>) {
        match message {
            CounterMessage::Increment => self.count += 1,
        }
    }

    fn view(&self, _input: &(), _context: &mut ViewContext<Self>) -> View {
        TextBlock::new()
            .text(format!("count: {}", self.count))
            .into()
    }
}

#[test]
fn headless_component_updates_and_rerenders() {
    let mut pump = Pump::new(RecordingRuntime::default());
    // The component hands its message sender to the test via create.
    let sender_cell: SenderSink = Rc::new(RefCell::new(None));
    COUNTER_SENDER.with(|cell| *cell.borrow_mut() = Some(sender_cell.clone()));
    pump.mount_view(View::component::<Counter>(()))
        .expect("mount");
    pump.dispatch_components(10).expect("dispatch");
    let window = pump.window().expect("window node");
    {
        let rt = pump.runtime();
        let text = find_text(rt, window).expect("component view recorded");
        assert_eq!(
            rt.node(text).unwrap().property(PropertyId::TextBlockText),
            Some(&PropertyValue::Str(String::from("count: 0")))
        );
    }

    // Send a message the way the real shell will (kernel command ->
    // component message), let the pump deliver it, and assert the
    // re-rendered tree.
    let sender = sender_cell.borrow().clone().expect("sender stashed");
    assert!(sender.send(CounterMessage::Increment), "send accepted");
    pump.dispatch_components(10)
        .expect("dispatch after message");
    {
        let rt = pump.runtime();
        let text = find_text(rt, window).expect("re-rendered view recorded");
        assert_eq!(
            rt.node(text).unwrap().property(PropertyId::TextBlockText),
            Some(&PropertyValue::Str(String::from("count: 1")))
        );
    }
}

// --- file tree rows through the recording runtime ---------------------------

/// Render the flattened tree rows as a StackPanel of indented TextBlocks,
/// keyed by vault-relative path (the reconciliation identity the real
/// TreeView adapter will key on).
fn file_tree_view(tree: &FileTree) -> View {
    let rows: Vec<KeyedView> = tree
        .visible_rows()
        .iter()
        .map(|row| {
            let label = format!("{}{}", "  ".repeat(row.depth), row.name);
            KeyedView::new(row.path.clone(), TextBlock::new().text(label))
        })
        .collect();
    StackPanel::new()
        .orientation(Orientation::Vertical)
        .keyed_children(rows)
}

#[test]
fn headless_file_tree_rows_render_and_collapse() {
    let mut tree = FileTree::from_paths(&[
        String::from("a.md"),
        String::from("notes/b.md"),
        String::from("notes/c.md"),
    ]);

    let mut pump = Pump::new(RecordingRuntime::default());
    pump.mount_view(file_tree_view(&tree)).expect("mount");

    // Collapsed by default (kernel behavior): the folder row sorts first,
    // its children are hidden.
    let window = pump.window().expect("window node");
    {
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        assert_eq!(texts, vec![String::from("notes"), String::from("a.md")]);
    }

    // Toggling the folder (kernel state) reveals its children through the
    // same mounted window: keyed rows insert by path, order is depth-first.
    tree.toggle_folder("notes");
    pump.update_view(file_tree_view(&tree)).expect("update");
    {
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        assert_eq!(
            texts,
            vec![
                String::from("notes"),
                String::from("  b.md"),
                String::from("  c.md"),
                String::from("a.md"),
            ]
        );
    }

    // Collapsing again removes the children rows.
    tree.toggle_folder("notes");
    pump.update_view(file_tree_view(&tree)).expect("update");
    {
        let rt = pump.runtime();
        let mut texts = Vec::new();
        collect_texts(rt, window, &mut texts);
        assert_eq!(texts, vec![String::from("notes"), String::from("a.md")]);
    }
}

// --- the shell component over the kernel ------------------------------------

use app_windows::app::NoteBuffer;
use app_windows::shell::{Shell, ShellHandle};
use app_windows::windows_message::WindowsMessage;
use app_windows::WindowsApp;
use vault_app::commands::UiCommand;
use vault_app::navigation::NoteIndex;
use vault_app::repository::{testing::MemoryRepository, NoteRepository};
use vault_app::settings::{AppSettings, MemorySettingsStore, ThemeChoice};
use vault_platform::testing::{OpenCall, RecordingOpener};
use windows_reactor::test::{EventId, EventPayload, QueuedEvent};
use windows_reactor::{WindowBackdrop, WindowTheme, WindowVisuals};

/// A kernel host over a memory vault with a succeeding opener.
fn shell_app_with(notes: &[(&str, &str)]) -> Rc<RefCell<WindowsApp>> {
    let (app, _settings) = shell_app_with_settings(notes);
    app
}

/// Like [`shell_app_with`], also handing back the settings store so a
/// test can assert what the shell persisted. The search root is the
/// system temp dir (the headless tests do not search).
fn shell_app_with_settings(
    notes: &[(&str, &str)],
) -> (Rc<RefCell<WindowsApp>>, Rc<RefCell<MemorySettingsStore>>) {
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    for (path, text) in notes {
        repo.borrow_mut().insert(path, text);
    }
    let settings = Rc::new(RefCell::new(MemorySettingsStore::default()));
    let app = WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        Rc::new(RecordingOpener::succeeding()),
        settings.clone(),
        std::env::temp_dir(),
    );
    (Rc::new(RefCell::new(app)), settings)
}

/// Like [`shell_app_with`], over an explicit settings document (the
/// persisted snapshot a previous session left behind).
fn shell_app_with_document(
    notes: &[(&str, &str)],
    document: AppSettings,
) -> (Rc<RefCell<WindowsApp>>, Rc<RefCell<MemorySettingsStore>>) {
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    for (path, text) in notes {
        repo.borrow_mut().insert(path, text);
    }
    let settings = Rc::new(RefCell::new(MemorySettingsStore::starting_with(document)));
    let app = WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        Rc::new(RecordingOpener::succeeding()),
        settings.clone(),
        std::env::temp_dir(),
    );
    (Rc::new(RefCell::new(app)), settings)
}

/// Open `path` through the kernel (the shell's own message path) and give
/// it a buffer with the given text over the given base (disk) text.
fn shell_open_tab(app: &Rc<RefCell<WindowsApp>>, path: &str, text: &str, base: &str) {
    app.borrow_mut()
        .handle_native(WindowsMessage::Command(UiCommand::OpenNote {
            path: path.to_string(),
        }));
    let tab = app
        .borrow()
        .shared
        .state
        .tabs
        .id_of(path)
        .unwrap_or_else(|| panic!("tab opened: {path}"));
    app.borrow_mut().buffers.insert(
        tab,
        NoteBuffer {
            text: text.to_string(),
            base: base.to_string(),
        },
    );
}

/// Mount the shell over the app and pump its initial render.
fn mounted_shell(app: &Rc<RefCell<WindowsApp>>) -> Pump<RecordingRuntime> {
    let mut pump = Pump::new(RecordingRuntime::default());
    pump.mount_view(View::component::<Shell>(ShellHandle::new(app.clone())))
        .expect("mount");
    pump.dispatch_components(10).expect("dispatch");
    pump
}

/// Find the node owning the TextBlock child whose text is exactly
/// `label` (a button's content label makes its owner the button node).
fn find_owner_of_label(rt: &RecordingRuntime, from: NodeId, label: &str) -> Option<NodeId> {
    let node = rt.node(from)?;
    for &child in node.children() {
        if let Some(child_node) = rt.node(child) {
            if child_node.property(PropertyId::TextBlockText)
                == Some(&PropertyValue::Str(label.to_string()))
            {
                return Some(from);
            }
            if let Some(found) = find_owner_of_label(rt, child, label) {
                return Some(found);
            }
        }
    }
    None
}

/// Inject one click on the node (event revision from the pump, empty
/// payload like the real runtime sends for button clicks).
fn click(pump: &mut Pump<RecordingRuntime>, node: NodeId, event: EventId) {
    let revision = pump.event_revision(node, event).expect("event active");
    pump.queue_event(QueuedEvent::new(node, event, revision, EventPayload::Unit));
    pump.dispatch_events().expect("dispatch events");
    pump.dispatch_components(10).expect("dispatch components");
}

/// Switch the kernel pane mode by clicking its layout button (the pane
/// defaults to Live, which renders no source TextBox).
fn switch_pane(pump: &mut Pump<RecordingRuntime>, label: &str) {
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, label)
            .unwrap_or_else(|| panic!("{label} pane button recorded"))
    };
    click(pump, target, EventId::ButtonClick);
}

#[test]
fn headless_shell_renders_tabs_editor_and_error_line() {
    let app = shell_app_with(&[("a.md", "a body\n")]);
    // Open the note through the kernel before mounting (real startup
    // order: the shell renders existing state).
    shell_open_tab(&app, "a.md", "a body\n", "a body\n");

    let mut pump = mounted_shell(&app);
    // The pane defaults to Live (reading view only); the source editor
    // exists in Source mode.
    switch_pane(&mut pump, "source");
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    // Active tab renders bracketed; the editor shows the buffer text.
    assert!(
        find_owner_of_label(rt, window, "[a.md]").is_some(),
        "active tab button recorded"
    );
    let editor = find_node_with_property_text(rt, window, PropertyId::TextBoxText, "a body\n")
        .expect("editor recorded");
    assert_eq!(
        rt.node(editor).unwrap().property(PropertyId::TextBoxText),
        Some(&PropertyValue::Str(String::from("a body\n")))
    );
}

/// Inject one `TextBoxTextChanged` on the editor node (the payload the
/// real runtime sends: the control's full text).
fn type_text(pump: &mut Pump<RecordingRuntime>, editor: NodeId, text: &str) {
    let revision = pump
        .event_revision(editor, EventId::TextBoxTextChanged)
        .expect("text-changed event active");
    pump.queue_event(QueuedEvent::new(
        editor,
        EventId::TextBoxTextChanged,
        revision,
        EventPayload::Str(text.to_string()),
    ));
    pump.dispatch_events().expect("dispatch events");
    pump.dispatch_components(10).expect("dispatch components");
}

#[test]
fn headless_shell_editor_typing_reaches_the_buffer_and_rerenders() {
    let app = shell_app_with(&[("a.md", "old\n")]);
    shell_open_tab(&app, "a.md", "a body\n", "old\n");

    let mut pump = mounted_shell(&app);
    switch_pane(&mut pump, "source");
    let window = pump.window().expect("window node");
    let editor = {
        let rt = pump.runtime();
        find_node_with_property_text(rt, window, PropertyId::TextBoxText, "a body\n")
            .expect("editor recorded")
    };

    // The user types: the control reports its full text.
    type_text(&mut pump, editor, "typed by the user\n");

    // The edit landed in the active buffer: text moved, base (disk
    // truth) untouched, so the buffer is dirty and would be saved.
    let borrowed = app.borrow();
    let tab = borrowed.shared.state.tabs.id_of("a.md").unwrap();
    let buffer = &borrowed.buffers[&tab];
    assert_eq!(buffer.text, "typed by the user\n");
    assert_eq!(buffer.base, "old\n");
    assert!(buffer.is_dirty());
    assert_eq!(borrowed.shared.state.last_error, None);
}

#[test]
fn headless_shell_tab_switch_syncs_the_editor_programmatically() {
    let app = shell_app_with(&[("a.md", "a\n"), ("b.md", "b\n")]);
    shell_open_tab(&app, "a.md", "a text\n", "a\n");
    shell_open_tab(&app, "b.md", "b text\n", "b\n");

    let mut pump = mounted_shell(&app);
    switch_pane(&mut pump, "source");

    // b.md is focused (last opened); click a.md's tab button.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "a.md").expect("a.md tab button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // The editor is a property update (programmatic sync): the
    // reconciled control now shows a.md's buffer, and b.md's buffer is
    // untouched by the switch.
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    let editor = find_node_with_property_text(rt, window, PropertyId::TextBoxText, "a text\n")
        .expect("editor re-rendered with the newly active buffer");
    assert_eq!(
        rt.node(editor).unwrap().property(PropertyId::TextBoxText),
        Some(&PropertyValue::Str(String::from("a text\n")))
    );
    let borrowed = app.borrow();
    let b = borrowed.shared.state.tabs.id_of("b.md").unwrap();
    assert_eq!(borrowed.buffers[&b].text, "b text\n");
}

#[test]
fn headless_shell_editor_echo_of_the_rendered_text_is_suppressed() {
    let app = shell_app_with(&[("a.md", "old\n")]);
    shell_open_tab(&app, "a.md", "a body\n", "old\n");

    let mut pump = mounted_shell(&app);
    switch_pane(&mut pump, "source");
    let window = pump.window().expect("window node");
    let editor = {
        let rt = pump.runtime();
        find_node_with_property_text(rt, window, PropertyId::TextBoxText, "a body\n")
            .expect("editor recorded")
    };

    // Simulate a programmatic buffer change that has NOT been rendered
    // yet (out of band, like a mid-flight effect result): the shell's
    // "last rendered" tracker still holds the old text.
    {
        let mut borrowed = app.borrow_mut();
        let tab = borrowed.shared.state.tabs.id_of("a.md").unwrap();
        borrowed.buffers.get_mut(&tab).unwrap().text = "outside\n".into();
    }

    // The echo of the last programmatic sync arrives ("a body\n" equals
    // the last rendered text). It must be suppressed: without the §11.1
    // guard it would clobber the pending buffer change.
    type_text(&mut pump, editor, "a body\n");
    {
        let borrowed = app.borrow();
        let tab = borrowed.shared.state.tabs.id_of("a.md").unwrap();
        assert_eq!(borrowed.buffers[&tab].text, "outside\n");
    }

    // A genuine user edit (text different from the last render) still
    // flows to the buffer.
    type_text(&mut pump, editor, "typed by the user\n");
    let borrowed = app.borrow();
    let tab = borrowed.shared.state.tabs.id_of("a.md").unwrap();
    assert_eq!(borrowed.buffers[&tab].text, "typed by the user\n");
    assert!(borrowed.buffers[&tab].is_dirty());
}

#[test]
fn headless_shell_click_activates_the_clicked_tab() {
    let app = shell_app_with(&[("a.md", "a\n"), ("b.md", "b\n")]);
    shell_open_tab(&app, "a.md", "a\n", "a\n");
    shell_open_tab(&app, "b.md", "b\n", "b\n");

    let mut pump = mounted_shell(&app);

    // The last opened tab is focused (open_or_focus), so b.md is active
    // and a.md's button is unbracketed. Click it to switch back.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "a.md").expect("a.md tab button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // The kernel switched, and the shell re-rendered: a is bracketed, b
    // is not.
    assert_eq!(
        app.borrow().shared.state.tabs.active_path(),
        Some("a.md"),
        "kernel switched tabs"
    );
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(find_owner_of_label(rt, window, "[a.md]").is_some());
    assert!(find_owner_of_label(rt, window, "b.md").is_some());
}

#[test]
fn headless_shell_click_closes_a_tab() {
    let app = shell_app_with(&[("a.md", "a\n"), ("b.md", "b\n")]);
    shell_open_tab(&app, "a.md", "a\n", "a\n");
    shell_open_tab(&app, "b.md", "b\n", "b\n");

    let mut pump = mounted_shell(&app);

    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "close a.md").expect("close button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    assert_eq!(
        app.borrow().shared.state.tabs.active_path(),
        Some("b.md"),
        "kernel closed a.md and b.md stays"
    );
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_owner_of_label(rt, window, "a.md").is_none(),
        "the closed tab vanished from the render"
    );
}

#[test]
fn headless_shell_save_click_surfaces_the_kernel_error_line() {
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    repo.borrow_mut().insert("a.md", "old\n");
    repo.borrow_mut().fail_saves_with("io broke");
    let app = Rc::new(RefCell::new(WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        Rc::new(RecordingOpener::succeeding()),
        Rc::new(RefCell::new(MemorySettingsStore::default())),
        std::env::temp_dir(),
    )));
    shell_open_tab(&app, "a.md", "edited\n", "old\n");

    let mut pump = mounted_shell(&app);

    // Click save: the guarded save fails, the kernel records the error,
    // and the shell re-renders the error line.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "save").expect("save button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    assert_eq!(
        app.borrow().shared.state.last_error.as_deref(),
        Some("save failed: io broke")
    );
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    let error_line = find_node_with_property_text(
        rt,
        window,
        PropertyId::TextBlockText,
        "save failed: io broke",
    )
    .expect("error line re-rendered");
    // The error line is its own TextBlock node (not the button content).
    assert_ne!(error_line, target, "error line is a distinct node");
}

#[test]
fn headless_shell_save_click_persists_the_buffer() {
    // The full vertical slice: render -> click save -> repository write.
    let app = shell_app_with(&[("a.md", "old\n")]);
    shell_open_tab(&app, "a.md", "typed on Windows\n", "old\n");

    let mut pump = mounted_shell(&app);

    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "save").expect("save button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let borrowed = app.borrow();
    assert_eq!(
        borrowed.shared.state.last_error, None,
        "clean save raises nothing"
    );
    // The buffer's base moved with the save (clean); the repository
    // assert lives with the kernel tests (MemoryRepository is not shared
    // through this handle).
    let buffer = &borrowed.buffers[&borrowed.shared.state.tabs.id_of("a.md").unwrap()];
    assert_eq!(buffer.text, "typed on Windows\n");
    assert_eq!(buffer.base, "typed on Windows\n");
    assert!(!buffer.is_dirty());
}

/// Depth-first search for a node with a given property text value.
fn find_node_with_property_text(
    rt: &RecordingRuntime,
    from: NodeId,
    property: PropertyId,
    value: &str,
) -> Option<NodeId> {
    let node = rt.node(from)?;
    if node.property(property) == Some(&PropertyValue::Str(value.to_string())) {
        return Some(from);
    }
    for &child in node.children() {
        if let Some(found) = find_node_with_property_text(rt, child, property, value) {
            return Some(found);
        }
    }
    None
}

fn find_text(rt: &RecordingRuntime, from: NodeId) -> Option<NodeId> {
    let node = rt.node(from)?;
    if node.property(PropertyId::TextBlockText).is_some() {
        return Some(from);
    }
    for &child in node.children() {
        if let Some(found) = find_text(rt, child) {
            return Some(found);
        }
    }
    None
}

/// Find the TreeView node by probing for the node the recording runtime
/// tracks `tree_nodes` for (`MountedKind` is not publicly exported, so
/// we detect the TreeView via `RecordingRuntime::tree_nodes`).
fn find_tree_view(rt: &RecordingRuntime, from: NodeId) -> Option<NodeId> {
    if rt.tree_nodes(from).is_some() {
        return Some(from);
    }
    for &child in rt.node(from)?.children() {
        if let Some(found) = find_tree_view(rt, child) {
            return Some(found);
        }
    }
    None
}

#[test]
fn headless_shell_tree_renders_vault_notes_and_folders() {
    let app = shell_app_with(&[("notes/b.md", "b\n"), ("a.md", "a\n")]);
    let pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    let tree_view = find_tree_view(rt, window).expect("tree view recorded");
    // Two top-level nodes: the "notes" folder and "a.md".
    assert_eq!(
        rt.tree_nodes(tree_view).map(|nodes| nodes.len()),
        Some(2),
        "folder plus note rendered as top-level tree nodes"
    );
}

#[test]
fn headless_shell_invoke_note_row_opens_it() {
    let app = shell_app_with(&[("notes/b.md", "b\n"), ("a.md", "a\n")]);
    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let tree_view = {
        let rt = pump.runtime();
        find_tree_view(rt, window).expect("tree view recorded")
    };
    let revision = pump
        .event_revision(tree_view, EventId::TreeViewItemInvoked)
        .expect("item-invoked event active");
    // The user invokes the "notes/b.md" row: the control echoes its key
    // (the vault path) as `TreeViewItemInvoked`.
    pump.queue_event(QueuedEvent::new(
        tree_view,
        EventId::TreeViewItemInvoked,
        revision,
        EventPayload::Str(String::from("notes/b.md")),
    ));
    pump.dispatch_events().expect("dispatch events");
    pump.dispatch_components(10).expect("dispatch components");

    assert_eq!(
        app.borrow().shared.state.tabs.active_path(),
        Some("notes/b.md"),
        "invoking a note row opens it"
    );
}

// --- pane-mode layout (kernel-driven) ----------------------------------------

#[test]
fn headless_shell_live_pane_renders_the_reading_view_only() {
    // Live is the settings default, so the boot render is the reading view.
    let app = shell_app_with(&[("a.md", "# Title\n\nbody text\n")]);
    shell_open_tab(
        &app,
        "a.md",
        "# Title\n\nbody text\n",
        "# Title\n\nbody text\n",
    );

    let pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "Title").is_some(),
        "the parsed heading renders"
    );
    assert!(
        find_node_with_property_text(
            rt,
            window,
            PropertyId::TextBoxText,
            "# Title\n\nbody text\n"
        )
        .is_none(),
        "Live shows no source TextBox (block editing is plan 11.4)"
    );
    // The active mode renders bracketed.
    assert!(find_owner_of_label(rt, window, "[live]").is_some());
}

#[test]
fn headless_shell_pane_split_shows_source_and_reading() {
    let app = shell_app_with(&[("a.md", "# Title\n\nbody text\n")]);
    shell_open_tab(
        &app,
        "a.md",
        "# Title\n\nbody text\n",
        "# Title\n\nbody text\n",
    );

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "split").expect("split button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // Both panes: the source TextBox and the parsed reading view.
    assert_eq!(
        app.borrow().shared.state.pane,
        vault_app::settings::PaneMode::Split
    );
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(
            rt,
            window,
            PropertyId::TextBoxText,
            "# Title\n\nbody text\n"
        )
        .is_some(),
        "the source TextBox renders"
    );
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "Title").is_some(),
        "the reading view renders"
    );
    assert!(find_owner_of_label(rt, window, "[split]").is_some());
}

#[test]
fn headless_shell_pane_source_shows_the_editor_only() {
    let app = shell_app_with(&[("a.md", "# Title\n\nbody text\n")]);
    shell_open_tab(
        &app,
        "a.md",
        "# Title\n\nbody text\n",
        "# Title\n\nbody text\n",
    );

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "source").expect("source button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(
            rt,
            window,
            PropertyId::TextBoxText,
            "# Title\n\nbody text\n"
        )
        .is_some(),
        "the source TextBox renders"
    );
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "Title").is_none(),
        "the reading view is gone in Source mode"
    );
}

#[test]
fn headless_shell_reading_link_click_opens_the_url_through_the_kernel() {
    // The full vertical slice: render -> click a link -> kernel
    // OpenExternal command -> ExternalOpener service records the call.
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    repo.borrow_mut()
        .insert("a.md", "see [the docs](https://example.com)\n");
    let opener = Rc::new(RecordingOpener::succeeding());
    let app = Rc::new(RefCell::new(WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        opener.clone(),
        Rc::new(RefCell::new(MemorySettingsStore::default())),
        std::env::temp_dir(),
    )));
    shell_open_tab(
        &app,
        "a.md",
        "see [the docs](https://example.com)\n",
        "see [the docs](https://example.com)\n",
    );

    let mut pump = mounted_shell(&app);
    // Live (the boot default) renders the reading view with the link.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "the docs").expect("link button recorded")
    };
    click(&mut pump, target, EventId::HyperlinkButtonClick);

    assert_eq!(
        opener.calls(),
        vec![OpenCall::Url(String::from("https://example.com"))],
        "the kernel handed the URL to the platform opener"
    );
    assert_eq!(
        app.borrow().shared.state.last_error,
        None,
        "a successful open raises nothing"
    );
}

#[test]
fn headless_shell_reading_wiki_link_click_opens_the_note() {
    let app = shell_app_with(&[("a.md", "see [[b.md]]\n"), ("b.md", "b\n")]);
    shell_open_tab(&app, "a.md", "see [[b.md]]\n", "see [[b.md]]\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "b.md").expect("wiki link button recorded")
    };
    click(&mut pump, target, EventId::HyperlinkButtonClick);

    assert_eq!(
        app.borrow().shared.state.tabs.active_path(),
        Some("b.md"),
        "invoking a wiki link opens the target note's tab"
    );
}

#[test]
fn headless_shell_pane_typing_in_split_updates_the_reading_view() {
    let app = shell_app_with(&[("a.md", "old body\n")]);
    shell_open_tab(&app, "a.md", "old body\n", "old body\n");

    let mut pump = mounted_shell(&app);
    // Switch to Split so both panes are mounted.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "split").expect("split button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // Type into the source TextBox.
    let window = pump.window().expect("window node");
    let editor = {
        let rt = pump.runtime();
        find_node_with_property_text(rt, window, PropertyId::TextBoxText, "old body\n")
            .expect("editor recorded")
    };
    type_text(&mut pump, editor, "# Fresh\n\ntyped text\n");

    // The reading view re-parses the CURRENT buffer text.
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "Fresh").is_some(),
        "the edited heading renders in the reading view"
    );
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "typed text").is_some(),
        "the edited paragraph renders in the reading view"
    );
}

// --- navigation panels -------------------------------------------------------

/// Like [`shell_app_with`], over a real vault root (the shared search
/// engine scans the filesystem, not the repository fake).
fn shell_app_with_root(notes: &[(&str, &str)], root: &std::path::Path) -> Rc<RefCell<WindowsApp>> {
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    for (path, text) in notes {
        repo.borrow_mut().insert(path, text);
    }
    let app = WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        Rc::new(RecordingOpener::succeeding()),
        Rc::new(RefCell::new(MemorySettingsStore::default())),
        root,
    );
    Rc::new(RefCell::new(app))
}

#[test]
fn headless_shell_nav_row_switches_the_sidebar_panel() {
    let app = shell_app_with(&[("a.md", "body\n")]);
    shell_open_tab(&app, "a.md", "body\n", "body\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    {
        let rt = pump.runtime();
        // Files is the default panel: the tree is mounted and bracketed.
        assert!(find_tree_view(rt, window).is_some());
        assert!(find_owner_of_label(rt, window, "[files]").is_some());
    }

    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "search").expect("search nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert_eq!(
        app.borrow().shared.state.panel,
        vault_app::navigation::NavSection::Search
    );
    // The search panel replaced the tree, and the nav row marks the
    // active section.
    assert!(
        find_tree_view(rt, window).is_none(),
        "the file tree is replaced by the search panel"
    );
    assert!(find_owner_of_label(rt, window, "[search]").is_some());
    // The query box exists (empty until the user types; Live mounts no
    // source editor, so it is the only TextBox).
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBoxText, "").is_some(),
        "the search query box is mounted"
    );
}

#[test]
fn headless_shell_search_panel_runs_the_scan_and_opens_a_hit() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.md"), "alpha\nneedle here\n").unwrap();
    let app = shell_app_with_root(&[("a.md", "alpha\nneedle here\n")], dir.path());
    shell_open_tab(&app, "a.md", "alpha\nneedle here\n", "alpha\nneedle here\n");

    let mut pump = mounted_shell(&app);
    // Switch the sidebar to Search.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "search").expect("search nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // Type the query into the kernel-bound box, then run the scan.
    let window = pump.window().expect("window node");
    let search_box = {
        let rt = pump.runtime();
        find_node_with_property_text(rt, window, PropertyId::TextBoxText, "")
            .expect("search box recorded")
    };
    type_text(&mut pump, search_box, "needle");
    assert_eq!(
        app.borrow().shared.state.search.query(),
        "needle",
        "typing updates the kernel session"
    );

    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "search").expect("run-search button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // The runner's shared engine fed the session; the panel lists the hit.
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert_eq!(app.borrow().shared.state.search.hits().len(), 1);
    assert!(
        find_owner_of_label(rt, window, "a.md:2 needle here").is_some(),
        "the hit row renders path:line and text"
    );

    // Clicking the hit opens (or focuses) its note.
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "a.md:2 needle here").expect("hit row recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);
    assert_eq!(
        app.borrow().shared.state.tabs.active_path(),
        Some("a.md"),
        "clicking a hit opens its note"
    );
}

#[test]
fn headless_shell_outline_panel_lists_headings_and_follows_edits() {
    let app = shell_app_with(&[("a.md", "# Title\n\n## Sub\n\nbody\n")]);
    shell_open_tab(
        &app,
        "a.md",
        "# Title\n\n## Sub\n\nbody\n",
        "# Title\n\n## Sub\n\nbody\n",
    );

    let mut pump = mounted_shell(&app);
    // Source pane: the reading view would also render the heading text,
    // and this test asserts the outline rows specifically.
    switch_pane(&mut pump, "source");
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "outline").expect("outline nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // Heading rows, indented by level.
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "Title").is_some(),
        "h1 row renders"
    );
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "  Sub").is_some(),
        "h2 row renders indented"
    );

    // Editing the buffer re-parses the outline.
    let editor = {
        let rt = pump.runtime();
        find_node_with_property_text(
            rt,
            window,
            PropertyId::TextBoxText,
            "# Title\n\n## Sub\n\nbody\n",
        )
        .expect("editor recorded")
    };
    type_text(&mut pump, editor, "# Fresh\n\nbody\n");
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "Fresh").is_some(),
        "the outline follows the edited buffer"
    );
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBlockText, "  Sub").is_none(),
        "the removed heading leaves the outline"
    );
}

fn collect_texts(rt: &RecordingRuntime, from: NodeId, out: &mut Vec<String>) {
    if let Some(node) = rt.node(from) {
        if let Some(PropertyValue::Str(text)) = node.property(PropertyId::TextBlockText) {
            out.push(text.clone());
        }
        for &child in node.children() {
            collect_texts(rt, child, out);
        }
    }
}

// --- appearance (theme, glass, title, boot pane) -----------------------------

#[test]
fn headless_shell_applies_the_persisted_theme_and_glass_to_the_window() {
    let (app, _settings) = shell_app_with_document(
        &[("a.md", "body\n")],
        AppSettings {
            theme: ThemeChoice::Dark,
            glass_enabled: false,
            ..AppSettings::default()
        },
    );
    shell_open_tab(&app, "a.md", "body\n", "body\n");

    let pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    let visuals = rt.window_visuals(window).expect("window visuals declared");
    assert_eq!(
        visuals,
        WindowVisuals::new()
            .theme(WindowTheme::Dark)
            .backdrop(WindowBackdrop::None),
        "persisted dark theme, glass off"
    );
    assert_eq!(rt.window_title(window), Some("VaultNotes"));
    // The appearance row brackets the active choices.
    assert!(find_owner_of_label(rt, window, "[dark]").is_some());
    assert!(find_owner_of_label(rt, window, "glass").is_some());
}

#[test]
fn headless_shell_theme_clicks_retheme_the_window_and_persist() {
    let (app, settings) = shell_app_with_document(&[("a.md", "body\n")], AppSettings::default());
    shell_open_tab(&app, "a.md", "body\n", "body\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "light").expect("light theme button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // The live window re-themed and the choice persisted.
    assert_eq!(app.borrow().shared.state.theme, ThemeChoice::Light);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    let visuals = rt.window_visuals(window).expect("window visuals declared");
    assert_eq!(
        visuals,
        WindowVisuals::new()
            .theme(WindowTheme::Light)
            .backdrop(WindowBackdrop::Mica),
    );
    assert_eq!(settings.borrow().last().theme, ThemeChoice::Light);
    assert!(find_owner_of_label(rt, window, "[light]").is_some());

    // Dark: same route through the kernel.
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "dark").expect("dark theme button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);
    assert_eq!(app.borrow().shared.state.theme, ThemeChoice::Dark);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert_eq!(
        rt.window_visuals(window).expect("visuals"),
        WindowVisuals::new()
            .theme(WindowTheme::Dark)
            .backdrop(WindowBackdrop::Mica),
    );
    assert_eq!(settings.borrow().last().theme, ThemeChoice::Dark);
}

#[test]
fn headless_shell_glass_toggle_flips_the_backdrop_and_persists() {
    // Defaults: glass on (the settings default) -> Mica.
    let (app, settings) = shell_app_with_document(&[("a.md", "body\n")], AppSettings::default());
    shell_open_tab(&app, "a.md", "body\n", "body\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert_eq!(
        rt.window_visuals(window).expect("visuals"),
        WindowVisuals::new()
            .theme(WindowTheme::System)
            .backdrop(WindowBackdrop::Mica),
        "glass on requests the frosted backdrop"
    );
    assert!(find_owner_of_label(rt, window, "[glass]").is_some());

    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "[glass]").expect("glass button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(!app.borrow().shared.state.glass_enabled);
    assert_eq!(
        rt.window_visuals(window).expect("visuals"),
        WindowVisuals::new()
            .theme(WindowTheme::System)
            .backdrop(WindowBackdrop::None),
    );
    assert!(!settings.borrow().last().glass_enabled);
    assert!(find_owner_of_label(rt, window, "glass").is_some());

    // Toggling again restores the frosted backdrop.
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "glass").expect("glass button recorded")
    };
    click(&mut pump, target, EventId::ButtonClick);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert_eq!(
        rt.window_visuals(window).expect("visuals"),
        WindowVisuals::new()
            .theme(WindowTheme::System)
            .backdrop(WindowBackdrop::Mica),
    );
    assert!(settings.borrow().last().glass_enabled);
}

#[test]
fn headless_shell_boots_into_the_persisted_default_pane() {
    // The persisted snapshot said Split: the source editor must be
    // mounted at boot without any pane click.
    let (app, _settings) = shell_app_with_document(
        &[("a.md", "a body\n")],
        AppSettings {
            default_pane: vault_app::settings::PaneMode::Split,
            ..AppSettings::default()
        },
    );
    shell_open_tab(&app, "a.md", "a body\n", "a body\n");

    let pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_node_with_property_text(rt, window, PropertyId::TextBoxText, "a body\n").is_some(),
        "the session boots into the persisted split pane"
    );
    assert!(find_owner_of_label(rt, window, "[split]").is_some());
}

// --- tags and links panels (the vault index adapter) --------------------------

/// A shell whose repository fake is seeded with notes, tags, and links,
/// handing back the repo so tests can seed index data before mounting.
fn shell_app_with_index(
    notes: &[(&str, &str)],
    tags: &[(&str, &str)],
    links: &[(&str, &str)],
) -> (Rc<RefCell<WindowsApp>>, Rc<RefCell<MemoryRepository>>) {
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    for (path, text) in notes {
        repo.borrow_mut().insert(path, text);
    }
    for (path, tag) in tags {
        repo.borrow_mut().add_tag(path, tag);
    }
    for (from, to) in links {
        repo.borrow_mut().link(from, to);
    }
    let app = WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        Rc::new(RecordingOpener::succeeding()),
        Rc::new(RefCell::new(MemorySettingsStore::default())),
        std::env::temp_dir(),
    );
    (Rc::new(RefCell::new(app)), repo)
}

#[test]
fn headless_shell_tags_panel_lists_counts_and_drills_down() {
    let (app, _repo) = shell_app_with_index(
        &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n")],
        &[("a.md", "work"), ("b.md", "work"), ("c.md", "home")],
        &[],
    );
    shell_open_tab(&app, "c.md", "c\n", "c\n");

    let mut pump = mounted_shell(&app);
    // Switch the sidebar to Tags.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "tags").expect("tags nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // The tag list shows each tag with its count.
    let window = pump.window().expect("window node");
    {
        let rt = pump.runtime();
        assert!(find_owner_of_label(rt, window, "#work (2)").is_some());
        assert!(find_owner_of_label(rt, window, "#home (1)").is_some());
        assert_eq!(
            app.borrow().shared.state.panel,
            vault_app::navigation::NavSection::Tags,
            "the kernel owns the section"
        );
    }

    // Clicking a tag drills down to its notes (shell-local filter).
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "#work (2)").expect("work tag row")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_owner_of_label(rt, window, "← #work").is_some(),
        "the drill-down shows a back row"
    );
    assert!(find_owner_of_label(rt, window, "a.md").is_some());
    assert!(find_owner_of_label(rt, window, "b.md").is_some());
    assert!(
        find_owner_of_label(rt, window, "#work (2)").is_none(),
        "the tag list is replaced by the drill-down"
    );

    // Clicking the back row clears the filter.
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "← #work").expect("back row")
    };
    click(&mut pump, target, EventId::ButtonClick);
    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(find_owner_of_label(rt, window, "#work (2)").is_some());
    assert!(find_owner_of_label(rt, window, "a.md").is_none());
}

#[test]
fn headless_shell_tags_drill_down_click_opens_the_note() {
    let (app, _repo) = shell_app_with_index(
        &[("a.md", "a\n"), ("b.md", "b\n")],
        &[("a.md", "work"), ("b.md", "work")],
        &[],
    );
    // Only b.md is open: clicking the a.md row must open it.
    shell_open_tab(&app, "b.md", "b\n", "b\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "tags").expect("tags nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "#work (2)").expect("work tag row")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "a.md").expect("a.md note row")
    };
    click(&mut pump, target, EventId::ButtonClick);

    // The kernel opened the note and its buffer is populated.
    assert_eq!(app.borrow().shared.state.tabs.active_path(), Some("a.md"));
}

#[test]
fn headless_shell_links_panel_lists_backlinks_and_opens_one() {
    // b.md links to a.md: with a.md active, the links panel lists b.md.
    let (app, _repo) = shell_app_with_index(
        &[("a.md", "a\n"), ("b.md", "b\n")],
        &[],
        &[("b.md", "a.md")],
    );
    shell_open_tab(&app, "a.md", "a\n", "a\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "links").expect("links nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    {
        let rt = pump.runtime();
        assert!(
            find_owner_of_label(rt, window, "b.md").is_some(),
            "the backlink source is listed"
        );
    }

    // Clicking the backlink opens its source note.
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "b.md").expect("b.md backlink row")
    };
    click(&mut pump, target, EventId::ButtonClick);

    assert_eq!(app.borrow().shared.state.tabs.active_path(), Some("b.md"));
}

#[test]
fn headless_shell_links_panel_shows_the_empty_state_without_a_note() {
    let (app, _repo) = shell_app_with_index(&[("a.md", "a\n")], &[], &[]);

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "links").expect("links nav button")
    };
    click(&mut pump, target, EventId::ButtonClick);

    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert!(
        find_owner_of_label(rt, window, "(open a note to see its backlinks)").is_some(),
        "the no-active-note hint is shown"
    );
}

// --- reading-view interactions (relative links, tags, footnotes) ------------

#[test]
fn headless_shell_reading_relative_link_click_opens_the_note() {
    let app = shell_app_with(&[("a.md", "see [b](b.md)\n"), ("b.md", "b\n")]);
    shell_open_tab(&app, "a.md", "see [b](b.md)\n", "see [b](b.md)\n");

    let mut pump = mounted_shell(&app);
    // Live (the boot default) renders the reading view with the link.
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "b").expect("relative link button recorded")
    };
    click(&mut pump, target, EventId::HyperlinkButtonClick);

    assert_eq!(
        app.borrow().shared.state.tabs.active_path(),
        Some("b.md"),
        "a relative link resolves through the index and opens the note"
    );
}

#[test]
fn headless_shell_reading_relative_link_to_attachment_opens_the_file() {
    // An attachment (not a note in the index) is unresolved by the index,
    // so the link is handed to the platform's default file handler.
    let repo = Rc::new(RefCell::new(MemoryRepository::new()));
    repo.borrow_mut().insert("a.md", "see [img](image.png)\n");
    let opener = Rc::new(RecordingOpener::succeeding());
    let app = Rc::new(RefCell::new(WindowsApp::new(
        repo.clone() as Rc<RefCell<dyn NoteRepository>>,
        repo.clone() as Rc<RefCell<dyn NoteIndex>>,
        opener.clone(),
        Rc::new(RefCell::new(MemorySettingsStore::default())),
        std::env::temp_dir(),
    )));
    shell_open_tab(
        &app,
        "a.md",
        "see [img](image.png)\n",
        "see [img](image.png)\n",
    );

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "img").expect("attachment link button recorded")
    };
    click(&mut pump, target, EventId::HyperlinkButtonClick);

    assert_eq!(
        opener.calls(),
        vec![OpenCall::File(std::path::PathBuf::from("image.png"))],
        "an unresolvable relative link is handed to the platform file opener"
    );
    assert_eq!(
        app.borrow().shared.state.last_error,
        None,
        "a successful open raises nothing"
    );
}

#[test]
fn headless_shell_reading_tag_click_switches_to_the_tags_panel() {
    // The note carries an inline `#work` tag and is indexed as tagged,
    // so clicking it drills down to the note itself.
    let (app, _repo) = shell_app_with_index(&[("a.md", "see #work\n")], &[("a.md", "work")], &[]);
    shell_open_tab(&app, "a.md", "see #work\n", "see #work\n");

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "#work").expect("tag link button recorded")
    };
    click(&mut pump, target, EventId::HyperlinkButtonClick);

    let window = pump.window().expect("window node");
    let rt = pump.runtime();
    assert_eq!(
        app.borrow().shared.state.panel,
        vault_app::navigation::NavSection::Tags,
        "a reading-view tag switches the sidebar to the tags panel"
    );
    // The tag was selected: the panel drills down to that tag's notes
    // with a back row (the same shell-local drill-down the sidebar uses).
    assert!(
        find_owner_of_label(rt, window, "← #work").is_some(),
        "the drill-down shows a back row"
    );
    assert!(
        find_owner_of_label(rt, window, "a.md").is_some(),
        "the drill-down lists the tagged note"
    );
}

#[test]
fn headless_shell_reading_footnote_ref_click_anchors_the_definition() {
    // Block 0 is the paragraph, block 1 is the footnote definition.
    let app = shell_app_with(&[("a.md", "text [^1]\n\n[^1]: definition\n")]);
    shell_open_tab(
        &app,
        "a.md",
        "text [^1]\n\n[^1]: definition\n",
        "text [^1]\n\n[^1]: definition\n",
    );

    let mut pump = mounted_shell(&app);
    let window = pump.window().expect("window node");
    let target = {
        let rt = pump.runtime();
        find_owner_of_label(rt, window, "^1").expect("footnote ref button recorded")
    };
    click(&mut pump, target, EventId::HyperlinkButtonClick);

    assert_eq!(
        app.borrow().shared.state.jump_anchor,
        Some(1),
        "a footnote ref anchors its definition block (the Border marker)"
    );
}
