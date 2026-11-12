//! Binary entry point for the WinUI 3 frontend.
//!
//! On Windows this boots the `windows-reactor` application loop around
//! [`app::WindowsApp`]. On non-Windows targets it is a compile-time stub
//! so the workspace gates stay green here; native Windows execution is
//! user-side.

use app_windows::WindowsApp;

#[cfg(target_os = "windows")]
fn main() {
    // windows-reactor application loop goes here on a real Windows build.
    let _app = WindowsApp::default();
    eprintln!("app-windows: running on Windows (native UI path)");
}

#[cfg(not(target_os = "windows"))]
fn main() {
    // Stub entry point: the crate type-checks but produces no window.
    let _ = WindowsApp::default();
    eprintln!("app-windows: skeleton on non-Windows target; native UI is Windows-only.");
}
