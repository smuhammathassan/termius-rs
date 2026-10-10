//! views — the GPUI view tree replacing Termius' React components.
//!
//! Layout (see [`app_shell::AppShell`]):
//!
//! ```text
//! AppShell  (root: keybindings, status bar, owns TermiusState, dialog overlay)
//! ├── sidebar column
//! │   ├── section nav      — navigation::sidebar_items() (hosts/snippets/…)
//! │   └── Hosts arm        — group tree + host rows (other arms: stubs)
//! └── main column
//!     ├── TabBar    — open sessions
//!     ├── row: TerminalPane  +  SftpPanel (toggleable; forced under SFTP)
//!     └── status bar
//! ```
//!
//! The center routes on `TermiusState::current_section`: `Hosts` and `Sftp`
//! host the terminal/sftp panels, every other section renders the screen
//! entity `AppShell` builds once (`Snippets`, `Keys`, `PortForwarding`,
//! `Keychain`, `Team`, `Settings`, `Account`); `Logs` still shows an
//! `EmptyState`. An active `TermiusState::active_dialog` paints a scrim +
//! `DialogFrame` above everything, filled in by the owning screen's
//! `*_dialog_body`.

pub mod account_screen;
pub mod app_shell;
pub mod host_dialog;
pub mod host_list;
pub mod keychain_screen;
pub mod keys_screen;
pub mod port_forwarding_screen;
pub mod settings_screen;
pub mod snippets_screen;
pub mod sftp_panel;
pub mod tab_bar;
pub mod team_screen;
pub mod terminal_pane;

pub use account_screen::{account_screen, AccountScreen};
pub use app_shell::{init, launch, open_window, AppShell};
pub use host_dialog::host_dialog_body;
pub use host_list::{build_tree, HostList, TreeRow};
pub use keychain_screen::{keychain_screen, KeychainScreen};
pub use keys_screen::{key_dialog_body, keys_screen, KeysScreen};
pub use port_forwarding_screen::{
    port_forward_dialog_body, port_forwarding_screen, PortForwardList,
};
pub use settings_screen::{settings_screen, SettingsScreen};
pub use snippets_screen::{snippet_dialog_body, snippets_screen, SnippetsScreen};
pub use sftp_panel::SftpPanel;
pub use tab_bar::TabBar;
pub use team_screen::{team_screen, TeamScreen};
pub use terminal_pane::{row_runs, TerminalPane, TextRun};
