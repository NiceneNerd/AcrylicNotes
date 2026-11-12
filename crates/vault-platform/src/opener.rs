//! Handing URLs and files to the operating system.
//!
//! The one service with a live consumer in this phase: `OpenExternal` and
//! `OpenAttachment` route through it, replacing the frontend's `cfg`-split
//! shell call (the old non-Linux branch silently discarded the URL).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the application wants the platform to open.
///
/// A URL goes to the platform's URL handler; a file goes to its
/// file-association handler. Attachments resolve to `File` before they
/// reach the platform; wikilink/http/mailto targets stay `Url`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenTarget {
    #[serde(rename = "url")]
    Url(String),
    #[serde(rename = "file")]
    File(PathBuf),
}

/// Why an open request could not even be handed to the platform.
///
/// Note the boundary: a successful spawn does not guarantee the document
/// opened (the child process owns that outcome); this error family covers
/// what the adapter can observe directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The opener program does not exist (e.g. `xdg-open` missing).
    NotFound { program: String },
    /// The platform refused to launch the opener.
    SpawnFailed { program: String, source: String },
    /// Any other failure with an actionable message.
    Failed { message: String },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { program } => {
                write!(f, "opener {program} not found")
            }
            Self::SpawnFailed { program, source } => {
                write!(f, "could not launch {program}: {source}")
            }
            Self::Failed { message } => write!(f, "{message}"),
        }
    }
}

/// Opens URLs and files with the platform's default handlers.
pub trait ExternalOpener: Send + Sync {
    /// Open a URL (http, mailto, custom schemes) in its default handler.
    fn open_url(&self, url: &str) -> Result<(), OpenError>;

    /// Open a file with its default application.
    fn open_file(&self, path: &Path) -> Result<(), OpenError>;

    /// Reveal a path in the platform's file manager. No consumer feature
    /// yet; defined now so the contract is complete.
    fn reveal_path(&self, path: &Path) -> Result<(), OpenError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_targets_round_trip_through_the_wire_format() {
        let targets = vec![
            OpenTarget::Url("https://example.com".into()),
            OpenTarget::File(PathBuf::from("/home/nn/Notes/file.png")),
        ];
        for target in targets {
            let json = serde_json::to_string(&target).unwrap();
            assert_eq!(
                serde_json::from_str::<OpenTarget>(&json).unwrap(),
                target,
                "round trip failed for {json}"
            );
        }
    }

    #[test]
    fn open_errors_are_actionable_and_displayable() {
        let errors = vec![
            OpenError::NotFound {
                program: "xdg-open".into(),
            },
            OpenError::SpawnFailed {
                program: "xdg-open".into(),
                source: "permission denied".into(),
            },
            OpenError::Failed {
                message: "no handler for scheme".into(),
            },
        ];
        for error in errors {
            let text = error.to_string();
            assert!(!text.is_empty());
            assert_eq!(error.clone(), error);
        }
    }
}
