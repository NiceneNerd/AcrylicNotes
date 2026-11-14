//! Windows implementation of `ExternalOpener` via WinRT `Launcher`.
//!
//! Hands URLs and files to the Windows shell's default handlers using
//! `Windows.System.Launcher`. This is the p11 live adapter the effect
//! runner's `OpenExternal` arm calls.
//!
//! Each WinRT async operation is blocked on with `IAsyncOperation::get()`.
//! The UI thread is STA, so this is safe.

use std::path::Path;

use vault_platform::{ExternalOpener, OpenError};
use windows::Foundation::Uri;

/// Opens URLs and files using Windows' default handlers via WinRT.
#[derive(Debug, Default)]
pub struct WinOpener;

impl WinOpener {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ExternalOpener for WinOpener {
    fn open_url(&self, url: &str) -> Result<(), OpenError> {
        let hstring = windows::core::HSTRING::from(url);
        let uri = Uri::CreateUri(&hstring).map_err(|e| OpenError::Failed {
            message: format!("invalid URL: {e}"),
        })?;
        windows::System::Launcher::LaunchUriAsync(&uri)
            .map_err(|e| OpenError::Failed {
                message: format!("could not start URL launch: {e}"),
            })?
            .get()
            .map_err(|e| OpenError::Failed {
                message: format!("URL launch failed: {e}"),
            })?;
        Ok(())
    }

    fn open_file(&self, path: &Path) -> Result<(), OpenError> {
        let hstring = windows::core::HSTRING::from(path.to_string_lossy().as_ref());
        let file = windows::Storage::StorageFile::GetFileFromPathAsync(&hstring)
            .map_err(|e| OpenError::Failed {
                message: format!("could not access file: {e}"),
            })?
            .get()
            .map_err(|e| OpenError::Failed {
                message: format!("could not open file: {e}"),
            })?;
        windows::System::Launcher::LaunchFileAsync(&file)
            .map_err(|e| OpenError::Failed {
                message: format!("could not start file launch: {e}"),
            })?
            .get()
            .map_err(|e| OpenError::Failed {
                message: format!("file launch failed: {e}"),
            })?;
        Ok(())
    }

    fn reveal_path(&self, path: &Path) -> Result<(), OpenError> {
        let parent = path.parent().unwrap_or(path);
        // A file:// URI launched through LaunchUriAsync opens the folder
        // in File Explorer.
        let uri_str = format!(
            "file:///{}",
            parent.display().to_string().replace('\\', "/")
        );
        let hstring = windows::core::HSTRING::from(uri_str.as_str());
        let uri = Uri::CreateUri(&hstring).map_err(|e| OpenError::Failed {
            message: format!("invalid folder URI: {e}"),
        })?;
        windows::System::Launcher::LaunchUriAsync(&uri)
            .map_err(|e| OpenError::Failed {
                message: format!("could not start folder launch: {e}"),
            })?
            .get()
            .map_err(|e| OpenError::Failed {
                message: format!("folder launch failed: {e}"),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_opener_is_default_constructible() {
        let _ = WinOpener;
    }
}
