//! VT/xterm terminal emulation over alacritty_terminal: grid, parser, scrollback, selection, unicode width, resize, copy-paste. Replaces xterm.js in the renderer.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod term;

pub mod grid_viewport;

pub mod selection;

pub mod error;
