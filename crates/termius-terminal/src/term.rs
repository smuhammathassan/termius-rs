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
//! PORT-TODO(alacritty_terminal 0.26): this crate targets an API that has
//! shifted between minor versions. Every uncertain call site carries a
//! `PORT-TODO`; they are all confined to this file plus two helpers in
//! [`crate::grid_viewport`].

use std::cell::RefCell;
use std::rc::Rc;

use termius_core::TerminalSize;

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Column, Line, Point, Scroll},
    term::{Term, TermMode},
    vte::Processor,
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

/// `EventListener` that appends every event to a shared buffer the UI drains.
#[derive(Debug, Clone)]
pub(crate) struct CollectingListener(Rc<RefCell<Vec<Event>>>);

impl EventListener for CollectingListener {
    fn send_event(&mut self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

/// A VT terminal: parser state machine + screen model + selection + events.
pub struct Terminal {
    term: Term<CollectingListener>,
    processor: Processor,
    events: Rc<RefCell<Vec<Event>>>,
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
        // PORT-TODO(alacritty_terminal 0.26): `Term::new(dimensions, listener)`;
        // some versions take the dimensions by reference (`&dimensions`).
        let term = Term::new(make_dimensions(size), listener);
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
        // PORT-TODO(alacritty_terminal 0.26): `Processor::advance(&mut term,
        // bytes, &mut renderer)` is the documented 0.26 signature. If the third
        // argument must be a concrete renderer type (e.g. a provided
        // `NoopRenderer`, or a custom `vte::ansi::Renderer` impl), swap
        // `&mut ()` here — this is the single call site. A local copy is used
        // because past versions accepted either `&[u8]` or `&mut [u8]`.
        let mut buf = bytes.to_vec();
        self.processor.advance(&mut self.term, &mut buf[..], &mut ());
        // Any non-empty output can change the grid; invalidate unconditionally
        // rather than trusting the event cadence.
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
        // PORT-TODO(alacritty_terminal 0.26): `Term::resize(dimensions, force)`;
        // drop the trailing `false` if 0.26 dropped the force flag.
        self.term.resize(make_dimensions(size), false);
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
        let pending: Vec<Event> = self.events.borrow_mut().drain(..).collect();
        pending.into_iter().map(map_event).collect()
    }

    /// True while un-drained events are pending.
    pub fn has_pending_events(&self) -> bool {
        !self.events.borrow().is_empty()
    }

    /// Take a full repaint unit of the visible viewport.
    pub fn snapshot(&mut self) -> GridSnapshot {
        build_snapshot(&mut self.term, self.size, self.generation)
    }

    /// True while the active buffer is the alternate screen (fullscreen apps).
    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALTERNATE_SCREEN)
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
            self.term.scroll_display(Scroll::Lines(lines));
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
    pub fn begin_selection(&mut self, row: u16, col: u16, mode: SelectionMode) {
        let point = self.viewport_point(row, col);
        match mode {
            SelectionMode::Char => self.term.simple_selection(point),
            SelectionMode::Word => self.term.semantic_selection(point),
            SelectionMode::Line => self.term.line_selection(point),
            // PORT-TODO(alacritty_terminal 0.26): `Term::selection_all()` may
            // not exist; fallback is start_selection at the first cell and
            // update_selection at the last grid cell.
            SelectionMode::All => self.term.selection_all(),
        }
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
        if self.selection_model.is_empty() {
            return;
        }
        let point = self.viewport_point(row, col);
        // PORT-TODO(alacritty_terminal 0.26): `Term::update_selection(point)`;
        // some versions also took a SelectionType/alt flag.
        self.term.update_selection(point);
        self.selection_model.extend(CellPosition::new(row, col));
        self.dirty.mark_all();
    }

    /// Drop the active selection (mouse-up on empty region, Esc, copy done).
    pub fn clear_selection(&mut self) {
        // PORT-TODO(alacritty_terminal 0.26): `Term::selection_clear()`;
        // some versions passed a `physical_alt: bool` flag.
        self.term.selection_clear();
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
        // PORT-TODO(alacritty_terminal 0.26): `Selection::is_empty()`.
        !self.term.selection().is_empty()
    }

    /// The UI-facing selection geometry (for painting the highlight).
    pub fn selection_model(&self) -> &SelectionModel {
        &self.selection_model
    }

    // ----- internals ------------------------------------------------------

    /// Topmost visible grid-absolute line, derived from the display iterator
    /// (robust against scrollback offset representation changes).
    fn viewport_top_line(&mut self) -> usize {
        let mut top: Option<usize> = None;
        for (point, _) in self.term.grid_mut().display_iter() {
            let line = point.line.0;
            top = Some(match top {
                Some(t) if t < line => t,
                _ => line,
            });
        }
        // A viewport always yields cells; fall back defensively.
        top.unwrap_or(0)
    }

    /// Translate viewport coordinates to a grid-absolute alacritty [`Point`],
    /// clamped to the live geometry.
    fn viewport_point(&mut self, row: u16, col: u16) -> Point {
        let row = row.min(self.size.rows.saturating_sub(1));
        let col = col.min(self.size.cols.saturating_sub(1));
        let top = self.viewport_top_line();
        Point { line: Line(top + row as usize), col: Column(col as usize) }
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

/// Build the alacritty dimensions struct from the shared [`TerminalSize`].
///
/// The pixel box is derived from the requested cell count so the emulator
/// always agrees with the PTY about rows/cols regardless of font metrics.
fn make_dimensions(size: TerminalSize) -> alacritty_terminal::term::Dimensions {
    let width = size.cols as f32 * CELL_WIDTH_PX;
    let height = size.rows as f32 * CELL_HEIGHT_PX;
    // PORT-TODO(alacritty_terminal 0.26): this type was `term::SizeInfo` on
    // the 0.14–0.19 line and was renamed `Dimensions` for 1.0; the
    // `new(width, height, cell_width, cell_height, padding_x, padding_y)`
    // argument order has been stable across the rename.
    alacritty_terminal::term::Dimensions::new(
        width,
        height,
        CELL_WIDTH_PX,
        CELL_HEIGHT_PX,
        0.0,
        0.0,
    )
}

// `#[allow(unreachable_patterns)]`: some 0.26 builds may make one of the arms
// genuinely unreachable depending on which variants survive; keep them all.
#[allow(unreachable_patterns)]
fn map_event(event: Event) -> TermEvent {
    // PORT-TODO(alacritty_terminal 0.26): variant names below are the long-
    // stable ones; anything renamed (e.g. `Paste` → `Input`) lands in `Other`.
    match event {
        Event::PtyWrite(data) => TermEvent::DataToWrite(data),
        Event::Title(title) => TermEvent::Title(title),
        Event::Advance => TermEvent::Advance,
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
        let err = term
            .resize(TerminalSize { cols: 0, rows: 10, width_px: None, height_px: None })
            .expect_err("zero cols must fail");
        assert!(matches!(err, TerminalError::InvalidSize { .. }));
    }

    #[test]
    fn new_rejects_degenerate_geometry() {
        let err = Terminal::new(TerminalSize { cols: 80, rows: 0, width_px: None, height_px: None })
            .expect_err("zero rows must fail");
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
