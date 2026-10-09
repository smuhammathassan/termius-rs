//! GPUI (Zed's native UI framework) views replacing the Electron React/Redux renderer: host list sidebar, terminal tabs, SFTP browser, port-forwarding manager, snippets panel, keychain manager, settings, login/sync UI. Native, GPU-accelerated, no webview.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).
//!
//! # Layout of this crate
//!
//! * [`app_state`] — [`TermiusState`], the Redux-replacement entity: library
//!   records, selection, open sessions (main-thread `Terminal` + engine
//!   bridge), tabs, UI flags.
//! * [`theme`] — the centralized dark/light palette and terminal ANSI
//!   resolution.
//! * [`views`] — one `Render` impl per panel, composed by
//!   [`views::AppShell`].
//! * [`error`] — [`UiError`] wrapping engine/storage failures.
//!
//! # Entry point
//!
//! ```ignore
//! // in termius-app's main:
//! termius_ui::launch();           // Application::new + init + open_window
//! // or, driving the app yourself:
//! gpui::Application::new().run(|cx| {
//!     termius_ui::init(cx);
//!     let _ = termius_ui::open_window(cx);
//! });
//! ```

pub mod app_state;

pub mod error;

pub mod theme;

pub mod views;

pub use app_state::{
    active_after_close, parent_path, Library, Session, SessionStatus, SftpState, TermiusState,
};
pub use error::{Result, UiError};
pub use theme::{theme_of, TermiusTheme, ThemeMode};
pub use views::{
    init, launch, open_window, AppShell, HostList, SftpPanel, TabBar, TerminalPane, TreeRow,
};
