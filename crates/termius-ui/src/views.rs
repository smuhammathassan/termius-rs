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
//! The center routes on `TermiusState::current_section`; `Hosts` and `Sftp`
//! host real panels today, every other section shows an `EmptyState` stub
//! until its screen wave lands. An active `TermiusState::active_dialog`
//! paints a scrim + `DialogFrame` above everything.

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
