//! `xdg-open` external opening.
//!
//! The freedesktop default-handler program covers URLs (http, mailto,
//! custom schemes) and files (via MIME association) alike, which is why
//! both `open_url` and `open_file` funnel into one spawn helper.
//! `reveal_path` opens the containing folder — the closest xdg-open
//! equivalent of "show in file manager".

use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

use vault_platform::{ExternalOpener, OpenError};

/// The freedesktop opener program.
pub const XDG_OPEN: &str = "xdg-open";

/// The Linux `ExternalOpener`.
#[derive(Debug, Clone, Copy, Default)]
pub struct XdgOpen;

impl ExternalOpener for XdgOpen {
    fn open_url(&self, url: &str) -> Result<(), OpenError> {
        spawn(XDG_OPEN, &[url])
    }

    fn open_file(&self, path: &Path) -> Result<(), OpenError> {
        spawn(XDG_OPEN, &[&path.to_string_lossy()])
    }

    fn reveal_path(&self, path: &Path) -> Result<(), OpenError> {
        // Opening the parent folder selects nothing but is the closest
        // portable equivalent of "reveal in file manager".
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        spawn(XDG_OPEN, &[&parent.to_string_lossy()])
    }
}

/// Spawn `program` fire-and-forget, mapping spawn failures to typed
/// errors. Factored out so tests can exercise the mapping without a
/// real desktop session.
pub(crate) fn spawn(program: &str, args: &[&str]) -> Result<(), OpenError> {
    let outcome = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match outcome {
        Ok(_) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Err(OpenError::NotFound {
            program: program.to_string(),
        }),
        Err(source) => Err(OpenError::SpawnFailed {
            program: program.to_string(),
            source: source.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_maps_to_not_found() {
        let error = spawn("vaultnotes-nonexistent-opener-xyz", &[]).unwrap_err();
        assert_eq!(
            error,
            OpenError::NotFound {
                program: "vaultnotes-nonexistent-opener-xyz".into()
            }
        );
        assert!(error.to_string().contains("not found"));
    }

    #[test]
    fn a_working_program_reports_success_without_waiting() {
        // `true` exists on every test machine we target (Linux CI); the
        // spawn succeeds and we never wait for the (instant) child.
        assert_eq!(spawn("true", &[]), Ok(()));
    }
}
