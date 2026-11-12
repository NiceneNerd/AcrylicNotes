//! Decoding dropped files and folders.
//!
//! The native payload shape differs per toolkit; the frontend converts
//! whatever it received into the raw string list (paths or textual
//! representations) and this service normalizes it into items.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One dropped filesystem item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedItem {
    pub path: PathBuf,
    pub is_folder: bool,
}

/// Why a drop payload could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileDropError {
    #[serde(rename = "undecodable")]
    Undecodable { message: String },
}

impl std::fmt::Display for FileDropError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Undecodable { message } => write!(f, "drop payload undecodable: {message}"),
        }
    }
}

/// Normalizes a native drag-and-drop payload into filesystem items.
pub trait FileDropService: Send + Sync {
    /// Decode the raw payload (one entry per dropped representation).
    fn decode(&self, raw: Vec<String>) -> Result<Vec<DroppedItem>, FileDropError>;
}
