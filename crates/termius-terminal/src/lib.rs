//! VT/xterm terminal emulation over [`alacritty_terminal`] — replaces xterm.js
//! in the Termius renderer.
//!
//! Ported from the Termius Electron app (v10.1.3): the renderer ran xterm.js
//! (with headless addons) for VT emulation; this crate wraps
//! `alacritty_terminal::Term` + its escape-sequence `Processor` behind a small
//! API the GPUI layer drives:
//!
//! ```ignore
//! use termius_terminal::{Key, SelectionMode, Terminal, TermEvent};
//! use termius_core::TerminalSize;
//!
//! let mut term = Terminal::new(TerminalSize::default())?;
//!
//! // SSH/serial stream -> emulator
//! term.advance(&ssh_stream_bytes);
//!
//! // Repaint loop (GPUI render)
//! if term.take_dirty() {
//!     let frame = term.snapshot(); // Vec<Vec<StyledCell>> + cursor + flags
//!     paint(frame);
//! }
//!
//! // Keyboard/paste -> PTY channel
//! pty.write(&term.key_down(Key::Up));
//! pty.write(&term.paste(&clipboard_text));
//!
//! // Emulator responses (DSR etc.) -> PTY channel
//! for event in term.drain_events() {
//!     if let TermEvent::DataToWrite(data) = event {
//!         pty.write(data.as_bytes());
//!     }
//! }
//!
//! // Selection (mouse / Cmd+C)
//! term.begin_selection(row, col, SelectionMode::Word);
//! let text = term.selection_text();
//! ```

pub mod error;

pub mod grid_viewport;

pub mod selection;

pub mod term;

pub use error::{Result, TerminalError};
pub use grid_viewport::{
    text_width, CellFlags, CursorShape, CursorState, DirtyTracker, GridSnapshot, StyledCell,
    TerminalColor,
};
pub use selection::{CellPosition, SelectionMode, SelectionModel, TerminalSelection};
pub use term::{Key, TermEvent, Terminal, CELL_HEIGHT_PX, CELL_WIDTH_PX};

/// Re-export of the shared geometry contract from `termius-core` so UI code
/// can depend on this crate alone for terminal types.
pub use termius_core::TerminalSize;
