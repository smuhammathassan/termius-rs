//! views — the GPUI view tree replacing Termius' React components.
//!
//! Layout (see [`app_shell::AppShell`]):
//!
//! ```text
//! AppShell  (root: keybindings, status bar, owns TermiusState)
//! ├── HostList      — group tree + host rows (sidebar)
//! └── main column
//!     ├── TabBar    — open sessions
//!     ├── row: TerminalPane  +  SftpPanel (toggleable)
//!     └── status bar
//! ```

pub mod app_shell;
pub mod host_list;
pub mod sftp_panel;
pub mod tab_bar;
pub mod terminal_pane;

pub use app_shell::{init, launch, open_window, AppShell};
pub use host_list::{build_tree, HostList, TreeRow};
pub use sftp_panel::SftpPanel;
pub use tab_bar::TabBar;
pub use terminal_pane::{row_runs, TerminalPane, TextRun};
