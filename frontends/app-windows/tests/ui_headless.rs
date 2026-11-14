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

// --- helpers ----------------------------------------------------------------

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
