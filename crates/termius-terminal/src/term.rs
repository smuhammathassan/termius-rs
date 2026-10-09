//! term — the `Terminal` wrapper over `alacritty_terminal::Term` + `Processor`.
//!
//! This is the only module that touches alacritty types (besides the grid
//! mapping in [`crate::grid_viewport`]). The intended wiring:
//!
//! * SSH/serial/telnet stream bytes → [`Terminal::advance`] (or
//!   [`Terminal::write`] for `&str`).
//! * GPUI keyboard/paste events → [`Terminal::key_down`] / [`Terminal::paste`];
//!   the returned bytes are written straight to the PTY channel.
//! * Output the emulator wants sent back to the PTY (DSR/DECRQSS replies,
//!   OSC sequences) arrives as [`TermEvent::DataToWrite`] from
//!   [`Terminal::drain_events`].
//! * Repaint: check [`Terminal::take_dirty`] (or the `generation` on
//!   [`crate::GridSnapshot`]) and call [`Terminal::snapshot`].
//!
//! All parser/terminal events are funnelled through a collecting
//! [`EventListener`] so the UI drains a flat `Vec<TermEvent>` each frame —
//! the same shape as xterm.js's `onData`/`onResize`/`onTitleChange`
//! subscriptions in the Electron renderer.
//!
//! Written against the real `alacritty_terminal` 0.26 API: `Term::new` takes
//! a `term::Config` plus any `grid::Dimensions` impl; the escape-sequence
//! parser is `vte::ansi::Processor` (re-exported as
//! `alacritty_terminal::vte`); grid indices live in `alacritty_terminal::index`;
//! selections are built from `alacritty_terminal::selection::Selection` and
//! installed on the public `Term::selection` field.

use std::cell::RefCell;
use std::rc::Rc;

use termius_core::TerminalSize;

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term, TermMode},
    vte::ansi::Processor,
};

use crate::error::{Result, TerminalError};
use crate::grid_viewport::{build_snapshot, GridSnapshot};
use crate::selection::{CellPosition, SelectionMode, SelectionModel};

/// Nominal cell width in px used to derive the emulator's pixel box from the
/// requested rows/cols. The GPUI renderer should paint with a font whose cell
/// matches these metrics so pixel-based queries (window ops, cell-size OSCs)
/// stay consistent.
pub const CELL_WIDTH_PX: f32 = 9.0;
/// Nominal cell height in px; see [`CELL_WIDTH_PX`].
pub const CELL_HEIGHT_PX: f32 = 18.0;

/// Normalized emulator events surfaced to the UI/engine.
///
/// Mirrors the xterm.js subscriptions Termius wired up: `onData` (bytes to
/// send to the shell), `onTitleChange`, plus dirty repaint notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// The terminal state changed; take a fresh [`Terminal::snapshot`].
    Advance,
    /// The emulator wants these bytes written to the PTY (cursor-position
    /// reports, DECRQSS replies, OSC 52/10/11 responses…).
    DataToWrite(String),
    /// Window title changed (OSC 0/2).
    Title(String),
    /// Any other internal event (wakeup, paste trigger, clipboard…).
    Other,
}

/// Keys the UI can send back to the shell.
///
/// This is the subset xterm.js's keyboard handler mapped for Termius; the
/// encoding honors DECCKM (application cursor keys) automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    /// Termius ran xterm.js with `backspace: "legacy"` → DEL (0x7f).
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    /// Function key `F1..=F12`.
    F(u8),
}

/// `EventListener` that maps every alacritty event onto the UI-facing
/// [`TermEvent`] shape and appends it to a shared buffer the UI drains.
#[derive(Debug, Clone)]
pub(crate) struct CollectingListener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for CollectingListener {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(map_event(event));
    }
}

/// A VT terminal: parser state machine + screen model + selection + events.
pub struct Terminal {
    term: Term<CollectingListener>,
    processor: Processor,
    events: Rc<RefCell<Vec<TermEvent>>>,
    size: TerminalSize,
    dirty: crate::grid_viewport::DirtyTracker,
    selection_model: SelectionModel,
    generation: u64,
}

impl Terminal {
    /// Create a terminal of the given geometry.
    pub fn new(size: TerminalSize) -> Result<Self> {
        validate_size(size)?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let listener = CollectingListener(events.clone());
        // `Term::new(config, dimensions, event_proxy)` — 0.26 takes a
        // `term::Config` first and borrows any `grid::Dimensions` impl.
        let term = Term::new(Config::default(), &TermDims::from(size), listener);
        Ok(Self {
            term,
            processor: Processor::new(),
            events,
            size,
            dirty: crate::grid_viewport::DirtyTracker::new(size.rows),
            selection_model: SelectionModel::new(),
            generation: 0,
        })
    }

    /// Feed raw PTY output through the escape-sequence parser into the term.
    pub fn advance(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // 0.26 signature: `Processor::advance(&mut self, handler: &mut H,
        // bytes: &[u8]) where H: Handler` — `Term<T>` implements `Handler`,
        // so no separate renderer argument is needed.
        self.processor.advance(&mut self.term, bytes);
        // Any non-empty output can change the grid; invalidate unconditionally
        // rather than trusting the event cadence, and surface the repaint
        // hint (0.26's `Term` only emits `Wakeup` from its event *loop*, not
        // from `Term` itself).
        self.events.borrow_mut().push(TermEvent::Advance);
        self.generation += 1;
        self.dirty.mark_all();
        tracing::trace!(len = bytes.len(), "terminal advanced");
    }

    /// Convenience wrapper: feed text output (`&str`) into the parser.
    pub fn write(&mut self, text: &str) {
        self.advance(text.as_bytes());
    }

    /// Encode a key press the way xterm.js would; the UI writes the returned
    /// bytes to the session's PTY channel.
    pub fn key_down(&self, key: Key) -> Vec<u8> {
        let app_cursor = self.term.mode().contains(TermMode::APP_CURSOR);
        encode_key(key, app_cursor).into_bytes()
    }

    /// Encode a paste. Newlines are normalized to CR (xterm.js paste
    /// behavior) and the payload is wrapped in bracketed-paste markers when
    /// the application enabled them (DECSET 2004).
    pub fn paste(&self, text: &str) -> Vec<u8> {
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{normalized}\x1b[201~").into_bytes()
        } else {
            normalized.into_bytes()
        }
    }

    /// Re-fit the emulator to a new geometry (window resize / font change).
    pub fn resize(&mut self, size: TerminalSize) -> Result<()> {
        validate_size(size)?;
        if size == self.size {
            return Ok(());
        }
        // 0.26 signature: `Term::resize<S: Dimensions>(&mut self, size: S)` —
        // takes the dimensions by value, no force flag.
        self.term.resize(TermDims::from(size));
        self.size = size;
        self.dirty.resize(size.rows);
        self.generation += 1;
        Ok(())
    }

    /// The current geometry.
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Monotonic content generation; bumped on processed output and resizes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Drain pending emulator events (empty vec when nothing happened).
    pub fn drain_events(&mut self) -> Vec<TermEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    /// True while un-drained events are pending.
    pub fn has_pending_events(&self) -> bool {
        !self.events.borrow().is_empty()
    }

    /// Take a full repaint unit of the visible viewport.
    pub fn snapshot(&mut self) -> GridSnapshot {
        build_snapshot(&self.term, self.size, self.generation)
    }

    /// True while the active buffer is the alternate screen (fullscreen apps).
    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Drain the dirty flag (true = a repaint is needed).
    pub fn take_dirty(&mut self) -> bool {
        self.dirty.take()
    }

    /// Whether a repaint is pending without consuming the flag.
    pub fn is_dirty(&self) -> bool {
        self.dirty.is_dirty()
    }

    /// Mark a specific viewport row as needing repaint (selection changes).
    pub fn mark_dirty_row(&mut self, row: u16) {
        self.dirty.mark_row(row);
    }

    /// Scroll the visible viewport; positive scrolls back into history.
    pub fn scroll_display(&mut self, lines: i32) {
        if lines != 0 {
            self.term.scroll_display(Scroll::Delta(lines));
        }
        self.dirty.mark_all();
    }

    /// Pin the viewport to the live buffer (scroll to bottom).
    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
        self.dirty.mark_all();
    }

    /// Pin the viewport to the top of history.
    pub fn scroll_to_top(&mut self) {
        self.term.scroll_display(Scroll::Top);
        self.dirty.mark_all();
    }

    // ----- selection ------------------------------------------------------

    /// Begin a selection at a viewport cell with the given granularity
    /// (mouse-down, double-click → [`SelectionMode::Word`], triple-click →
    /// [`SelectionMode::Line`], select-all → [`SelectionMode::All`]).
    ///
    /// 0.26 has no `Term::simple_selection` helpers; selections are
    /// `selection::Selection` values installed on the public `Term::selection`
    /// field.
    pub fn begin_selection(&mut self, row: u16, col: u16, mode: SelectionMode) {
        let point = self.viewport_point(row, col);
        let selection = match mode {
            SelectionMode::All => {
                // No "select all" primitive either: anchor a Lines selection
                // at the top-left of the grid and extend it to the
                // bottom-right; `to_range` expands it over whole lines.
                let top = self.term.topmost_line();
                let bottom = self.term.bottommost_line();
                let mut selection =
                    Selection::new(SelectionType::Lines, Point::new(top, Column(0)), Side::Left);
                selection.update(Point::new(bottom, self.term.last_column()), Side::Right);
                selection
            },
            _ => Selection::new(mode.alacritty_type(), point, Side::Left),
        };
        self.term.selection = Some(selection);
        let at = if mode == SelectionMode::All {
            CellPosition::new(0, 0)
        } else {
            CellPosition::new(row, col)
        };
        self.selection_model.begin(at, mode);
        self.dirty.mark_all();
    }

    /// Extend the active selection to a viewport cell (mouse-drag /
    /// shift-extension). No-op when nothing is selected.
    pub fn extend_selection(&mut self, row: u16, col: u16) {
        let Some(model) = self.selection_model.selection().copied() else {
            return;
        };
        let focus = self.viewport_point(row, col);
        // Rebuild from the model anchor so the cell sides follow the drag
        // direction (inclusive on both ends, whatever the order).
        let anchor_point = self.viewport_point(model.anchor.row, model.anchor.col);
        let (anchor_side, focus_side) = if focus >= anchor_point {
            (Side::Left, Side::Right)
        } else {
            (Side::Right, Side::Left)
        };
        let mut selection = Selection::new(model.mode.alacritty_type(), anchor_point, anchor_side);
        selection.update(focus, focus_side);
        self.term.selection = Some(selection);
        self.selection_model.extend(CellPosition::new(row, col));
        self.dirty.mark_all();
    }

    /// Drop the active selection (mouse-up on empty region, Esc, copy done).
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
        self.selection_model.clear();
        self.dirty.mark_all();
    }

    /// The selected text, as alacritty assembles it (word/line semantics
    /// included). `None` when the selection is empty.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// True while a non-empty selection exists in the term.
    pub fn has_selection(&self) -> bool {
        self.term.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// The UI-facing selection geometry (for painting the highlight).
    pub fn selection_model(&self) -> &SelectionModel {
        &self.selection_model
    }

    // ----- internals ------------------------------------------------------

    /// Topmost visible grid-absolute line, from the scrollback display
    /// offset (`0` at the live buffer, negative while scrolled back).
    fn viewport_top_line(&self) -> i32 {
        -(self.term.grid().display_offset() as i32)
    }

    /// Translate viewport coordinates to a grid-absolute alacritty [`Point`],
    /// clamped to the live geometry.
    fn viewport_point(&self, row: u16, col: u16) -> Point {
        let row = row.min(self.size.rows.saturating_sub(1));
        let col = col.min(self.size.cols.saturating_sub(1));
        let top = self.viewport_top_line();
        Point::new(Line(top + row as i32), Column(col as usize))
    }
}

fn validate_size(size: TerminalSize) -> Result<()> {
    if size.cols == 0 || size.rows == 0 {
        return Err(TerminalError::invalid_size(format!(
            "degenerate geometry {}x{}",
            size.cols, size.rows
        )));
    }
    Ok(())
}

/// Minimal cell-count [`Dimensions`] impl fed to `Term::new`/`Term::resize`.
///
/// 0.26's `Dimensions` is a *trait* (cell counts only — pixel metrics left
/// the terminal layer long ago); `Term` itself and `Grid` implement it. This
/// private newtype carries just the geometry the emulator needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TermDims {
    columns: usize,
    screen_lines: usize,
}

impl From<TerminalSize> for TermDims {
    fn from(size: TerminalSize) -> Self {
        Self { columns: size.cols as usize, screen_lines: size.rows as usize }
    }
}

impl Dimensions for TermDims {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

fn map_event(event: Event) -> TermEvent {
    match event {
        Event::PtyWrite(data) => TermEvent::DataToWrite(data),
        Event::Title(title) => TermEvent::Title(title),
        Event::Wakeup => TermEvent::Advance,
        _ => TermEvent::Other,
    }
}

fn encode_key(key: Key, app_cursor: bool) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        // xterm.js sends CR for Enter unless LNM is set (rare).
        Key::Enter => '\r'.to_string(),
        // Termius `backspace: "legacy"` → DEL.
        Key::Backspace => '\x7f'.to_string(),
        Key::Tab => '\t'.to_string(),
        Key::Escape => '\x1b'.to_string(),
        Key::Up => cursor_key('A', app_cursor),
        Key::Down => cursor_key('B', app_cursor),
        Key::Right => cursor_key('C', app_cursor),
        Key::Left => cursor_key('D', app_cursor),
        Key::Home => cursor_key('H', app_cursor),
        Key::End => cursor_key('F', app_cursor),
        Key::Insert => "\x1b[2~".to_string(),
        Key::Delete => "\x1b[3~".to_string(),
        Key::PageUp => "\x1b[5~".to_string(),
        Key::PageDown => "\x1b[6~".to_string(),
        Key::F(n) => function_key(n),
    }
}

fn cursor_key(letter: char, app_cursor: bool) -> String {
    if app_cursor {
        format!("\x1bO{letter}")
    } else {
        format!("\x1b[{letter}")
    }
}

fn function_key(n: u8) -> String {
    match n {
        1 => "\x1bOP",
        2 => "\x1bOQ",
        3 => "\x1bOR",
        4 => "\x1bOS",
        5 => "\x1b[15~",
        6 => "\x1b[17~",
        7 => "\x1b[18~",
        8 => "\x1b[19~",
        9 => "\x1b[20~",
        10 => "\x1b[21~",
        11 => "\x1b[23~",
        12 => "\x1b[24~",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> Terminal {
        Terminal::new(TerminalSize::default()).expect("80x24 is valid")
    }

    /// Concatenated cell text of one viewport row.
    fn row_text(snap: &GridSnapshot, row: u16) -> String {
        snap.cells[row as usize].iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn feeds_plain_text_into_grid() {
        let mut term = terminal();
        term.write("hello\r\nworld");
        let snap = term.snapshot();
        assert_eq!(row_text(&snap, 0), "hello");
        assert_eq!(row_text(&snap, 1), "world");
        assert_eq!(snap.cursor.point, Some((1, 5)));
        assert!(snap.cursor.visible);
        assert_eq!(snap.cols, 80);
        assert_eq!(snap.rows, 24);
        assert!(!snap.alt_screen);
    }

    #[test]
    fn sgr_color_and_bold() {
        let mut term = terminal();
        term.write("\x1b[1;31mX\x1b[0mY");
        let snap = term.snapshot();
        let x = snap.cell(0, 0).expect("cell exists");
        assert_eq!(x.text, "X");
        assert_eq!(x.fg, crate::grid_viewport::TerminalColor::Indexed(1));
        assert!(x.flags.bold);
        let y = snap.cell(0, 1).expect("cell exists");
        assert_eq!(y.fg, crate::grid_viewport::TerminalColor::Default);
        assert!(!y.flags.bold);
    }

    #[test]
    fn erase_display_clears_grid() {
        let mut term = terminal();
        term.write("hi");
        assert_eq!(row_text(&term.snapshot(), 0), "hi");
        term.write("\x1b[2J");
        let snap = term.snapshot();
        assert_eq!(row_text(&snap, 0), "");
        assert_eq!(row_text(&snap, 1), "");
    }

    #[test]
    fn resize_changes_geometry_and_keeps_content() {
        let mut term = terminal();
        term.write("hello");
        term.resize(TerminalSize { cols: 40, rows: 12, width_px: None, height_px: None })
            .expect("valid resize");
        let snap = term.snapshot();
        assert_eq!(snap.cols, 40);
        assert_eq!(snap.rows, 12);
        assert_eq!(row_text(&snap, 0), "hello");
        assert_eq!(term.size().cols, 40);
        assert_eq!(term.size().rows, 12);
    }

    #[test]
    fn resize_rejects_degenerate_geometry() {
        let mut term = terminal();
        let err = match term.resize(TerminalSize { cols: 0, rows: 10, width_px: None, height_px: None })
        {
            Err(err) => err,
            Ok(()) => panic!("zero cols must fail"),
        };
        assert!(matches!(err, TerminalError::InvalidSize { .. }));
    }

    #[test]
    fn new_rejects_degenerate_geometry() {
        let err = match Terminal::new(TerminalSize { cols: 80, rows: 0, width_px: None, height_px: None })
        {
            Err(err) => err,
            Ok(_) => panic!("zero rows must fail"),
        };
        assert!(matches!(err, TerminalError::InvalidSize { .. }));
    }

    #[test]
    fn parser_state_survives_chunk_boundaries() {
        let mut term = terminal();
        term.advance(b"\x1b[3");
        term.advance(b"1mZ");
        let snap = term.snapshot();
        let z = snap.cell(0, 0).expect("cell exists");
        assert_eq!(z.text, "Z");
        assert_eq!(z.fg, crate::grid_viewport::TerminalColor::Indexed(1));
    }

    #[test]
    fn output_marks_terminal_dirty_and_bumps_generation() {
        let mut term = terminal();
        term.take_dirty();
        let gen = term.generation();
        term.write("hi");
        assert!(term.take_dirty(), "parsed output invalidates the viewport");
        assert!(term.generation() > gen);
    }

    #[test]
    fn drain_events_surfaces_output() {
        let mut term = terminal();
        term.write("hi");
        assert!(term.has_pending_events());
        let events = term.drain_events();
        assert!(!events.is_empty(), "output should produce at least one event");
        assert!(!term.has_pending_events());
    }

    #[test]
    fn alt_screen_toggling() {
        let mut term = terminal();
        term.write("\x1b[?1049h");
        assert!(term.is_alt_screen());
        assert!(term.snapshot().alt_screen);
        term.write("\x1b[?1049l");
        assert!(!term.is_alt_screen());
    }

    #[test]
    fn bracketed_paste_wraps_and_normalizes() {
        let mut term = terminal();
        assert_eq!(term.paste("hi"), b"hi");
        assert_eq!(term.paste("a\nb"), b"a\rb");
        term.write("\x1b[?2004h");
        assert_eq!(term.paste("hi"), b"\x1b[200~hi\x1b[201~");
        assert_eq!(term.paste("a\nb"), b"\x1b[200~a\rb\x1b[201~");
    }

    #[test]
    fn key_encoding_honors_app_cursor_mode() {
        let mut term = terminal();
        assert_eq!(term.key_down(Key::Up), b"\x1b[A");
        assert_eq!(term.key_down(Key::Enter), b"\r");
        assert_eq!(term.key_down(Key::Backspace), b"\x7f");
        assert_eq!(term.key_down(Key::F(5)), b"\x1b[15~");
        assert_eq!(term.key_down(Key::Char('é')), "é".as_bytes());
        term.write("\x1b[?1h");
        assert_eq!(term.key_down(Key::Up), b"\x1bOA");
    }

    #[test]
    fn charwise_selection_extracts_text() {
        let mut term = terminal();
        term.write("hello world");
        term.begin_selection(0, 1, SelectionMode::Char);
        term.extend_selection(0, 4);
        assert!(term.has_selection());
        assert_eq!(term.selection_text().as_deref(), Some("ello"));
        term.clear_selection();
        assert!(!term.has_selection());
        assert_eq!(term.selection_text(), None);
    }

    #[test]
    fn word_selection_picks_semantic_word() {
        let mut term = terminal();
        term.write("hello world");
        term.begin_selection(0, 1, SelectionMode::Word);
        assert_eq!(term.selection_text().as_deref(), Some("hello"));
    }

    #[test]
    fn line_selection_picks_whole_line() {
        let mut term = terminal();
        term.write("hello world");
        term.begin_selection(0, 4, SelectionMode::Line);
        let text = term.selection_text().expect("line selected");
        assert_eq!(text.trim_end(), "hello world");
    }

    #[test]
    fn charwise_selection_across_rows() {
        let mut term = terminal();
        term.write("aaa\r\nbbb");
        term.begin_selection(0, 1, SelectionMode::Char);
        term.extend_selection(1, 1);
        assert_eq!(term.selection_text().as_deref(), Some("aa\nbb"));
    }

    #[test]
    fn select_all_covers_buffer() {
        let mut term = terminal();
        term.write("aaa\r\nbbb");
        term.begin_selection(0, 0, SelectionMode::All);
        let text = term.selection_text().expect("all selected");
        assert!(text.contains("aaa") && text.contains("bbb"), "got {text:?}");
    }

    #[test]
    fn selection_model_tracks_geometry() {
        let mut term = terminal();
        term.write("hello world");
        term.begin_selection(0, 3, SelectionMode::Char);
        term.extend_selection(0, 6);
        let selection = term.selection_model().selection().expect("selection");
        assert_eq!(selection.anchor, CellPosition::new(0, 3));
        assert_eq!(selection.focus, CellPosition::new(0, 6));
        assert!(term.selection_model().contains(0, 4, 80));
    }

    #[test]
    fn scroll_display_clamps_to_bottom() {
        let mut term = terminal();
        term.write("hi");
        term.scroll_display(500);
        term.scroll_to_bottom();
        term.scroll_display(-3);
        // No panic + still renderable is the contract here.
        let snap = term.snapshot();
        assert_eq!(snap.rows, 24);
    }
}
