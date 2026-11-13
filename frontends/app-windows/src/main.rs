//! Binary entry point for the WinUI 3 frontend.
//!
//! Everything in the library — the kernel wiring (`WindowsApp`), the
//! effect runner, the message flow — compiles and is unit-tested on
//! every platform. What is missing is the native bootstrapping: the
//! windows-reactor window loop, the WinUI 3 controls, and the live
//! platform adapters (vault repository, shell launcher). Those land with
//! the p11 platform services on a real Windows machine; native execution
//! and UI verification are user-side, never claimed via Wine.

fn main() {
    eprintln!(
        "app-windows: the native Windows UI is not wired yet (p11 platform \
         services pending); the kernel wiring is complete and tested. See \
         docs/parity.md APPWIN rows."
    );
    std::process::exit(1);
}
