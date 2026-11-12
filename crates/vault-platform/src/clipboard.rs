//! Clipboard read/write for plain text.

use serde::{Deserialize, Serialize};

/// Why a clipboard operation failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardError {
    /// The platform clipboard could not be reached.
    #[serde(rename = "unavailable")]
    Unavailable { message: String },
    /// Writing the content was refused.
    #[serde(rename = "write-refused")]
    WriteRefused { message: String },
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { message } => write!(f, "clipboard unavailable: {message}"),
            Self::WriteRefused { message } => write!(f, "clipboard write refused: {message}"),
        }
    }
}

/// Reads and writes plain text on the platform clipboard.
///
/// Rich content (images, file lists) is out of scope until a feature
/// needs it; the contract grows then, not before.
pub trait ClipboardService: Send + Sync {
    /// The clipboard's text content, or `None` when it holds non-text
    /// data (or nothing).
    fn read_text(&self) -> Result<Option<String>, ClipboardError>;

    /// Replace the clipboard content with `text`.
    fn write_text(&self, text: &str) -> Result<(), ClipboardError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_errors_display_actionably() {
        let unavailable = ClipboardError::Unavailable {
            message: "no clipboard".into(),
        };
        assert_eq!(
            unavailable.to_string(),
            "clipboard unavailable: no clipboard"
        );
        let refused = ClipboardError::WriteRefused {
            message: "read-only".into(),
        };
        assert_eq!(refused.to_string(), "clipboard write refused: read-only");
    }
}
