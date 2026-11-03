//! Vault Notes read-only web server (M5).
//!
//! Serves one vault over HTTP with the same rendering pipeline as the GUI:
//! `vault-core` for the vault model and search, `vault-md` for Markdown →
//! HTML. Server-rendered only — no JavaScript. Every user-controlled path
//! is confined to the vault by [`guard::safe_resolve`], auth defaults to a
//! generated token, and the server binds the loopback interface only unless
//! told otherwise.

pub mod config;
pub mod guard;
pub mod page;
pub mod server;
