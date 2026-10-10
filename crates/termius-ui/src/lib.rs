//! GPUI (Zed's native UI framework) views replacing the Electron React/Redux renderer: host list sidebar, terminal tabs, SFTP browser, port-forwarding manager, snippets panel, keychain manager, settings, login/sync UI. Native, GPU-accelerated, no webview.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).
//!
//! # Layout of this crate
//!
//! * [`app_state`] — [`TermiusState`], the Redux-replacement entity: library
//!   records, selection, open sessions (main-thread `Terminal` + engine
//!   bridge), tabs, UI flags, plus the screen contract: current section,
//!   active dialog, port forwards / known hosts / settings / account and the
//!   CRUD helpers the screens drive.
//! * [`navigation`] — [`Section`], the sidebar routing enum (+ labels/icons
//!   and the settings sub-tabs).
//! * [`primitives`] — the reusable components every screen composes from:
//!   [`EmptyState`], [`Button`], [`InputField`], [`Switch`], [`ListItem`],
//!   [`SectionHeader`], [`SettingsSection`]/[`SettingsTitle`]/[`SettingsText`]
//!   and [`DialogFrame`].
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

pub mod navigation;

pub mod primitives;

pub mod theme;

pub mod views;

pub use app_state::{
    active_after_close, dialog_after, parent_path, AccountInfo, Dialog, DialogIntent, Library,
    Session, SessionStatus, SettingsState, SftpState, TermiusState,
};
pub use error::{Result, UiError};
pub use navigation::{settings_tabs, sidebar_items, Section, SidebarItem, SETTINGS_TABS};
pub use primitives::{
    Button, ButtonVariant, DialogFrame, EmptyState, InputField, ListItem, SectionHeader,
    SettingsSection, SettingsText, SettingsTitle, Switch,
};
pub use theme::{theme_of, TermiusTheme, ThemeMode};
pub use views::{
    init, launch, open_window, AppShell, HostList, SftpPanel, TabBar, TerminalPane, TreeRow,
};
