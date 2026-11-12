//! Native pickers and confirm dialogs.
//!
//! The cosmic frontend's destructive tree delete deliberately has *no*
//! confirmation today (inventory DLG-013); when that changes, or when a
//! native vault/file picker replaces the empty-state path input, this is
//! the contract both frontends use.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A file-type filter for pickers (lowercase extensions, without dots).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFilter {
    pub label: String,
    pub extensions: Vec<String>,
}

/// Everything a destructive-action confirmation needs to render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmPrompt {
    pub title: String,
    pub body: String,
    pub confirm_label: String,
    pub cancel_label: String,
}

/// The user's answer to a confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfirmDecision {
    #[serde(rename = "confirmed")]
    Confirmed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

/// Why a dialog or picker interaction failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogError {
    /// The platform dialog system is unavailable.
    #[serde(rename = "unavailable")]
    Unavailable { message: String },
    /// The interaction failed before the user decided anything.
    #[serde(rename = "failed")]
    Failed { message: String },
}

impl std::fmt::Display for DialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { message } => write!(f, "dialogs unavailable: {message}"),
            Self::Failed { message } => write!(f, "dialog failed: {message}"),
        }
    }
}

/// Native vault/file pickers and destructive-action confirmations.
///
/// `Ok(None)` from a picker means the user cancelled — a normal outcome,
/// not an error.
pub trait DialogService: Send + Sync {
    /// Pick a folder to open as a vault.
    fn pick_vault_folder(&self) -> Result<Option<PathBuf>, DialogError>;

    /// Pick a single file matching `filter`.
    fn pick_file(&self, filter: FileFilter) -> Result<Option<PathBuf>, DialogError>;

    /// Ask the user to confirm a destructive action.
    fn confirm_destructive(&self, prompt: ConfirmPrompt) -> Result<ConfirmDecision, DialogError>;
}
