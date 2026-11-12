//! Recording fakes for the service traits.
//!
//! Contract tests here and frontend tests inject these to make platform
//! behavior deterministic. Every fake records the calls it received and
//! lets the test script the outcome.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::appearance::{AppearanceService, Backdrop};
use crate::capabilities::{Capability, CapabilityStatus, PlatformCapabilities};
use crate::clipboard::{ClipboardError, ClipboardService};
use crate::dialog::{ConfirmDecision, ConfirmPrompt, DialogError, DialogService, FileFilter};
use crate::file_drop::{DroppedItem, FileDropError, FileDropService};
use crate::notification::NotificationService;
use crate::opener::{ExternalOpener, OpenError};
use crate::timer::{TimerHandle, TimerService};
use crate::window::WindowService;

/// One recorded [`ExternalOpener`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenCall {
    Url(String),
    File(PathBuf),
    Reveal(PathBuf),
}

/// An opener fake that records calls and returns a scripted outcome.
#[derive(Debug)]
pub struct RecordingOpener {
    outcome: Mutex<Result<(), OpenError>>,
    calls: Mutex<Vec<OpenCall>>,
}

impl Default for RecordingOpener {
    fn default() -> Self {
        Self {
            outcome: Mutex::new(Ok(())),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl RecordingOpener {
    /// An opener whose operations succeed.
    #[must_use]
    pub fn succeeding() -> Self {
        Self::default()
    }

    /// An opener whose operations fail with `error`.
    #[must_use]
    pub fn failing(error: OpenError) -> Self {
        Self {
            outcome: Mutex::new(Err(error)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The calls received so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<OpenCall> {
        self.calls.lock().expect("poisoned").clone()
    }
}

impl ExternalOpener for RecordingOpener {
    fn open_url(&self, url: &str) -> Result<(), OpenError> {
        self.calls
            .lock()
            .expect("poisoned")
            .push(OpenCall::Url(url.to_string()));
        self.outcome.lock().expect("poisoned").clone()
    }

    fn open_file(&self, path: &Path) -> Result<(), OpenError> {
        self.calls
            .lock()
            .expect("poisoned")
            .push(OpenCall::File(path.to_path_buf()));
        self.outcome.lock().expect("poisoned").clone()
    }

    fn reveal_path(&self, path: &Path) -> Result<(), OpenError> {
        self.calls
            .lock()
            .expect("poisoned")
            .push(OpenCall::Reveal(path.to_path_buf()));
        self.outcome.lock().expect("poisoned").clone()
    }
}

/// An appearance fake answering with a fixed backdrop.
#[derive(Debug, Clone, Copy)]
pub struct FixedAppearance {
    pub backdrop: Backdrop,
}

impl AppearanceService for FixedAppearance {
    fn preferred_backdrop(&self) -> Backdrop {
        self.backdrop
    }
}

/// A clipboard fake holding one scripted read result and the last write.
#[derive(Debug)]
pub struct RecordingClipboard {
    read: Mutex<Result<Option<String>, ClipboardError>>,
    written: Mutex<Option<String>>,
}

impl Default for RecordingClipboard {
    fn default() -> Self {
        Self {
            read: Mutex::new(Ok(None)),
            written: Mutex::new(None),
        }
    }
}

impl RecordingClipboard {
    /// A clipboard that reads as empty and accepts writes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next read outcome.
    pub fn set_read(&self, outcome: Result<Option<String>, ClipboardError>) {
        *self.read.lock().expect("poisoned") = outcome;
    }

    /// The last text written to this clipboard.
    #[must_use]
    pub fn written(&self) -> Option<String> {
        self.written.lock().expect("poisoned").clone()
    }
}

impl ClipboardService for RecordingClipboard {
    fn read_text(&self) -> Result<Option<String>, ClipboardError> {
        self.read.lock().expect("poisoned").clone()
    }

    fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        *self.written.lock().expect("poisoned") = Some(text.to_string());
        Ok(())
    }
}

/// A dialog fake scripting the confirm answer and recording prompts.
#[derive(Debug)]
pub struct RecordingDialogs {
    answer: Mutex<Result<ConfirmDecision, DialogError>>,
    prompts: Mutex<Vec<ConfirmPrompt>>,
}

impl RecordingDialogs {
    /// Dialogs that answer `decision` to every confirmation.
    #[must_use]
    pub fn answering(decision: ConfirmDecision) -> Self {
        Self {
            answer: Mutex::new(Ok(decision)),
            prompts: Mutex::new(Vec::new()),
        }
    }

    /// The prompts this fake was asked to confirm.
    #[must_use]
    pub fn prompts(&self) -> Vec<ConfirmPrompt> {
        self.prompts.lock().expect("poisoned").clone()
    }
}

impl DialogService for RecordingDialogs {
    fn pick_vault_folder(&self) -> Result<Option<PathBuf>, DialogError> {
        Ok(None)
    }

    fn pick_file(&self, _filter: FileFilter) -> Result<Option<PathBuf>, DialogError> {
        Ok(None)
    }

    fn confirm_destructive(&self, prompt: ConfirmPrompt) -> Result<ConfirmDecision, DialogError> {
        self.prompts.lock().expect("poisoned").push(prompt);
        self.answer.lock().expect("poisoned").clone()
    }
}

/// A timer fake that records schedules without firing callbacks.
#[derive(Debug, Default)]
pub struct RecordingTimers {
    once: Mutex<Vec<Duration>>,
    periodic: Mutex<Vec<Duration>>,
}

impl RecordingTimers {
    /// The durations scheduled one-shot so far.
    #[must_use]
    pub fn once_scheduled(&self) -> Vec<Duration> {
        self.once.lock().expect("poisoned").clone()
    }

    /// The intervals scheduled periodically so far.
    #[must_use]
    pub fn periodic_scheduled(&self) -> Vec<Duration> {
        self.periodic.lock().expect("poisoned").clone()
    }
}

impl TimerService for RecordingTimers {
    fn schedule_once(&self, after: Duration, _callback: Box<dyn FnOnce() + Send>) {
        self.once.lock().expect("poisoned").push(after);
    }

    fn schedule_periodic(
        &self,
        interval: Duration,
        _callback: Box<dyn Fn() + Send>,
    ) -> TimerHandle {
        self.periodic.lock().expect("poisoned").push(interval);
        TimerHandle::new(Box::new(|| {}))
    }
}

/// A notification fake collecting every message it is shown.
#[derive(Debug, Default)]
pub struct RecordingNotifications {
    messages: Mutex<Vec<(bool, String)>>,
}

impl RecordingNotifications {
    /// The recorded `(is_error, message)` pairs in order.
    #[must_use]
    pub fn messages(&self) -> Vec<(bool, String)> {
        self.messages.lock().expect("poisoned").clone()
    }
}

impl NotificationService for RecordingNotifications {
    fn info(&self, message: &str) {
        self.messages
            .lock()
            .expect("poisoned")
            .push((false, message.to_string()));
    }

    fn error(&self, message: &str) {
        self.messages
            .lock()
            .expect("poisoned")
            .push((true, message.to_string()));
    }
}

/// A window fake recording the requests it received.
#[derive(Debug, Default)]
pub struct RecordingWindow {
    titles: Mutex<Vec<String>>,
    backdrops: Mutex<Vec<Backdrop>>,
    closes: Mutex<u32>,
    focuses: Mutex<u32>,
}

impl RecordingWindow {
    /// Titles set so far, in order.
    #[must_use]
    pub fn titles(&self) -> Vec<String> {
        self.titles.lock().expect("poisoned").clone()
    }

    /// Backdrops requested so far, in order.
    #[must_use]
    pub fn backdrops(&self) -> Vec<Backdrop> {
        self.backdrops.lock().expect("poisoned").clone()
    }

    /// How many close requests arrived.
    #[must_use]
    pub fn closes(&self) -> u32 {
        *self.closes.lock().expect("poisoned")
    }

    /// How many focus requests arrived.
    #[must_use]
    pub fn focuses(&self) -> u32 {
        *self.focuses.lock().expect("poisoned")
    }
}

impl WindowService for RecordingWindow {
    fn set_title(&self, title: &str) {
        self.titles
            .lock()
            .expect("poisoned")
            .push(title.to_string());
    }

    fn focus(&self) {
        *self.focuses.lock().expect("poisoned") += 1;
    }

    fn request_close(&self) {
        *self.closes.lock().expect("poisoned") += 1;
    }

    fn set_backdrop(&self, backdrop: Backdrop) {
        self.backdrops.lock().expect("poisoned").push(backdrop);
    }
}

/// A file-drop fake decoding a scripted item list.
#[derive(Debug)]
pub struct RecordingFileDrop {
    items: Mutex<Result<Vec<DroppedItem>, FileDropError>>,
}

impl Default for RecordingFileDrop {
    fn default() -> Self {
        Self {
            items: Mutex::new(Ok(Vec::new())),
        }
    }
}

impl RecordingFileDrop {
    /// A decoder returning `items` for every payload.
    #[must_use]
    pub fn decoding(items: Vec<DroppedItem>) -> Self {
        Self {
            items: Mutex::new(Ok(items)),
        }
    }

    /// A decoder failing with `error`.
    #[must_use]
    pub fn failing(error: FileDropError) -> Self {
        Self {
            items: Mutex::new(Err(error)),
        }
    }
}

impl FileDropService for RecordingFileDrop {
    fn decode(&self, _raw: Vec<String>) -> Result<Vec<DroppedItem>, FileDropError> {
        self.items.lock().expect("poisoned").clone()
    }
}

/// A capabilities fake answering from a fixed table.
#[derive(Debug, Default)]
pub struct FixedCapabilities {
    statuses: Mutex<Vec<(Capability, CapabilityStatus)>>,
}

impl FixedCapabilities {
    /// Report `status` for `capability`; unlisted capabilities are
    /// reported unavailable with a generic reason.
    pub fn set(&self, capability: Capability, status: CapabilityStatus) {
        self.statuses
            .lock()
            .expect("poisoned")
            .push((capability, status));
    }
}

impl PlatformCapabilities for FixedCapabilities {
    fn status(&self, capability: Capability) -> CapabilityStatus {
        self.statuses
            .lock()
            .expect("poisoned")
            .iter()
            .find(|(cap, _)| *cap == capability)
            .map(|(_, status)| status.clone())
            .unwrap_or(CapabilityStatus::Unsupported {
                reason: "not configured in this fake".into(),
            })
    }
}
