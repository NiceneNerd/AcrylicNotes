//! Contract tests: the fakes must behave exactly as the traits promise,
//! because both frontend test suites will rely on them.

use std::path::PathBuf;
use std::time::Duration;

use vault_platform::appearance::{AppearanceService, Backdrop};
use vault_platform::capabilities::{Capability, CapabilityStatus, PlatformCapabilities};
use vault_platform::clipboard::{ClipboardError, ClipboardService};
use vault_platform::dialog::{ConfirmDecision, ConfirmPrompt, DialogService, FileFilter};
use vault_platform::file_drop::{DroppedItem, FileDropService};
use vault_platform::notification::NotificationService;
use vault_platform::opener::{ExternalOpener, OpenError};
use vault_platform::testing::{
    FixedAppearance, FixedCapabilities, RecordingClipboard, RecordingDialogs, RecordingFileDrop,
    RecordingNotifications, RecordingOpener, RecordingTimers, RecordingWindow,
};
use vault_platform::timer::TimerService;
use vault_platform::window::WindowService;

#[test]
fn the_opener_fake_records_calls_and_scripts_outcomes() {
    let opener = RecordingOpener::succeeding();
    assert_eq!(opener.open_url("https://example.com"), Ok(()));
    assert_eq!(opener.open_file(&PathBuf::from("/tmp/a.png")), Ok(()));
    assert_eq!(opener.reveal_path(&PathBuf::from("/tmp")), Ok(()));
    assert_eq!(
        opener.calls(),
        vec![
            vault_platform::testing::OpenCall::Url("https://example.com".into()),
            vault_platform::testing::OpenCall::File("/tmp/a.png".into()),
            vault_platform::testing::OpenCall::Reveal("/tmp".into()),
        ]
    );

    let failing = RecordingOpener::failing(OpenError::NotFound {
        program: "xdg-open".into(),
    });
    assert!(failing.open_url("https://example.com").is_err());
    assert_eq!(failing.calls().len(), 1);
}

#[test]
fn the_appearance_fake_answers_its_fixed_backdrop() {
    let blur = FixedAppearance {
        backdrop: Backdrop::Blur,
    };
    assert_eq!(blur.preferred_backdrop(), Backdrop::Blur);
    let opaque = FixedAppearance {
        backdrop: Backdrop::Opaque,
    };
    assert_eq!(opaque.preferred_backdrop(), Backdrop::Opaque);
}

#[test]
fn the_clipboard_fake_scripts_reads_and_records_writes() {
    let clipboard = RecordingClipboard::new();
    clipboard.set_read(Ok(Some("hello".into())));
    assert_eq!(clipboard.read_text(), Ok(Some("hello".into())));
    clipboard.set_read(Ok(None));
    assert_eq!(clipboard.read_text(), Ok(None));
    clipboard.set_read(Err(ClipboardError::Unavailable {
        message: "gone".into(),
    }));
    assert!(clipboard.read_text().is_err());

    assert_eq!(clipboard.write_text("copied"), Ok(()));
    assert_eq!(clipboard.written().as_deref(), Some("copied"));
}

#[test]
fn the_dialog_fake_records_prompts_and_answers() {
    let dialogs = RecordingDialogs::answering(ConfirmDecision::Cancelled);
    let prompt = ConfirmPrompt {
        title: "Delete".into(),
        body: "Delete note.md?".into(),
        confirm_label: "Delete".into(),
        cancel_label: "Cancel".into(),
    };
    assert_eq!(
        dialogs.confirm_destructive(prompt.clone()),
        Ok(ConfirmDecision::Cancelled)
    );
    assert_eq!(dialogs.prompts(), vec![prompt]);
    assert_eq!(
        dialogs.pick_file(FileFilter {
            label: "Markdown".into(),
            extensions: vec!["md".into()],
        }),
        Ok(None)
    );
}

#[test]
fn the_timer_fake_records_schedules_without_firing() {
    let timers = RecordingTimers::default();
    let handle = timers.schedule_periodic(Duration::from_secs(2), Box::new(|| {}));
    timers.schedule_once(Duration::from_secs(1), Box::new(|| {}));
    assert_eq!(timers.once_scheduled(), vec![Duration::from_secs(1)]);
    assert_eq!(timers.periodic_scheduled(), vec![Duration::from_secs(2)]);
    handle.cancel();
}

#[test]
fn the_notification_and_window_fakes_record_requests() {
    let notifications = RecordingNotifications::default();
    notifications.info("reloaded");
    notifications.error("save failed: x");
    assert_eq!(
        notifications.messages(),
        vec![(false, "reloaded".into()), (true, "save failed: x".into()),]
    );

    let window = RecordingWindow::default();
    window.set_title("note — vault");
    window.focus();
    window.request_close();
    window.set_backdrop(Backdrop::Blur);
    assert_eq!(window.titles(), vec!["note — vault".to_string()]);
    assert_eq!(window.backdrops(), vec![Backdrop::Blur]);
    assert_eq!(window.focuses(), 1);
    assert_eq!(window.closes(), 1);
}

#[test]
fn the_file_drop_fake_scripts_its_decode_result() {
    let decoding = RecordingFileDrop::decoding(vec![DroppedItem {
        path: "/tmp/n.md".into(),
        is_folder: false,
    }]);
    assert_eq!(decoding.decode(vec!["/tmp/n.md".into()]).unwrap().len(), 1);

    let failing =
        RecordingFileDrop::failing(vault_platform::file_drop::FileDropError::Undecodable {
            message: "not paths".into(),
        });
    assert!(failing.decode(vec![]).is_err());
}

#[test]
fn the_capabilities_fake_answers_its_table_with_a_fallback() {
    let capabilities = FixedCapabilities::default();
    capabilities.set(Capability::ExternalOpening, CapabilityStatus::Available);
    assert!(capabilities
        .status(Capability::ExternalOpening)
        .is_available());
    let status = capabilities.status(Capability::Dialogs);
    assert!(!status.is_available());
    assert!(status.unsupported_reason().is_some());
}
