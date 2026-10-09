//! GPUI (Zed's native UI framework) views replacing the Electron React/Redux renderer: host list sidebar, terminal tabs, SFTP browser, port-forwarding manager, snippets panel, keychain manager, settings, login/sync UI. Native, GPU-accelerated, no webview.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod app_state;

pub mod theme;

pub mod views;

pub mod error;
