//! grid_viewport — the rendered grid surface for the UI.
//!
//! The GPUI layer never talks to `alacritty_terminal` types directly; it works
//! exclusively with the plain data shapes defined here:
//!
//! * [`GridSnapshot`] — one full repaint unit: every visible cell, the cursor,
//!   buffer mode and a monotonically increasing `generation`.
//! * [`StyledCell`] — text + colors + [`CellFlags`] for a single cell.
//! * [`DirtyTracker`] — cheap "do I need to repaint?" bookkeeping.
//!
//! Colors are kept symbolic ([`TerminalColor::Default`], `Indexed`, `Rgb`)
//! instead of resolved so the UI theme owns the palette, mirroring how xterm.js
//! reported `ITheme`/`IRgb` colors to the Termius renderer.

use serde::{Deserialize, Serialize};

use alacritty_terminal::{
    event::EventListener,
    term::{
        cell::{Cell, Flags},
        color::Color,
        Term,
    },
};

use termius_core::TerminalSize;

/// Cell attributes relevant for painting, flattened from alacritty's `Flags`
/// bitset so the UI (and serde) see plain booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellFlags {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub reverse: bool,
    pub hidden: bool,
    pub blink: bool,
    /// First cell of a double-width character.
    pub wide: bool,
    /// Placeholder cell following a double-width character.
    pub wide_spacer: bool,
}

/// A color as the terminal reported it; unresolved colors stay symbolic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalColor {
    /// The terminal default foreground/background — resolve via the UI theme.
    Default,
    /// 256-color palette index (SGR 30–37 / 38;5;n).
    Indexed(u8),
    /// Truecolor (SGR 38;2;r;g;b).
    Rgb { r: u8, g: u8, b: u8 },
}

/// One painted cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyledCell {
    /// Cell text; `""` for blank/wide-spacer cells, `" "` is an explicit space.
    pub text: String,
    /// Display width in columns (0 for a wide-char spacer, 1, or 2).
    pub width: u8,
    pub fg: TerminalColor,
    pub bg: TerminalColor,
    pub flags: CellFlags,
}

impl StyledCell {
    /// A blank default-styled cell.
    pub fn blank() -> Self {
        Self {
            text: String::new(),
            width: 1,
            fg: TerminalColor::Default,
            bg: TerminalColor::Default,
            flags: CellFlags::default(),
        }
    }
}

/// Cursor presentation for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorState {
    /// Cursor position in viewport coordinates, `None` when hidden.
    pub point: Option<(u16, u16)>,
    pub shape: CursorShape,
    pub visible: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self { point: None, shape: CursorShape::Block, visible: false }
    }
}

/// Cursor shapes understood by the UI (xterm.js `cursorStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Beam,
}

/// A complete repaint unit for the visible viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridSnapshot {
    /// Terminal columns (also `cells[*].len()` after padding).
    pub cols: u16,
    /// Visible rows; `cells.len() == rows`.
    pub rows: u16,
    /// Row-major visible cells, index 0 = topmost visible row.
    pub cells: Vec<Vec<StyledCell>>,
    pub cursor: CursorState,
    /// True while the alternate screen buffer is active (fullscreen apps).
    pub alt_screen: bool,
    /// Monotonic counter; changes whenever the grid content may differ from
    /// the previously taken snapshot.
    pub generation: u64,
}

impl GridSnapshot {
    /// Styled cell at viewport coordinates, if inside the grid.
    pub fn cell(&self, row: u16, col: u16) -> Option<&StyledCell> {
        self.cells.get(row as usize)?.get(col as usize)
    }
}

/// Dirty bookkeeping for efficient repaint.
///
/// alacritty reports "something changed" (via `Event::Advance`) but not *which*
/// rows, so the tracker supports per-row invalidation for changes we can pin
/// down (scroll, explicit UI marks) and falls back to full-viewport
/// invalidation for parsed output.
#[derive(Debug, Clone)]
pub struct DirtyTracker {
    all: bool,
    rows: Vec<bool>,
}

impl DirtyTracker {
    pub fn new(rows: u16) -> Self {
        Self { all: true, rows: vec![false; rows as usize] }
    }

    /// Mark the whole viewport dirty (parsed output, resize, reset).
    pub fn mark_all(&mut self) {
        self.all = true;
        for row in &mut self.rows {
            *row = true;
        }
    }

    /// Mark a single viewport row dirty.
    pub fn mark_row(&mut self, row: u16) {
        let idx = row as usize;
        if let Some(slot) = self.rows.get_mut(idx) {
            *slot = true;
        }
    }

    /// Mark rows `start..end` (exclusive) dirty.
    pub fn mark_rows(&mut self, start: u16, end: u16) {
        for row in start..end {
            self.mark_row(row);
        }
    }

    /// True if anything is dirty.
    pub fn is_dirty(&self) -> bool {
        self.all || self.rows.iter().any(|r| *r)
    }

    /// True if the given row needs repainting.
    pub fn row_dirty(&self, row: u16) -> bool {
        self.all || self.rows.get(row as usize).copied().unwrap_or(false)
    }

    /// Drain the dirty state (returns whether anything was dirty, then resets).
    pub fn take(&mut self) -> bool {
        let dirty = self.is_dirty();
        self.all = false;
        for row in &mut self.rows {
            *row = false;
        }
        dirty
    }

    /// Re-fit the tracker to a new row count (marks everything dirty).
    pub fn resize(&mut self, rows: u16) {
        self.rows = vec![true; rows as usize];
        self.all = true;
    }
}

/// Total display width of a string in terminal columns.
pub fn text_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Map an alacritty cell onto the UI-facing [`StyledCell`].
pub(crate) fn styled_from_cell(cell: &Cell) -> StyledCell {
    let flags = CellFlags {
        bold: cell.flags.contains(Flags::BOLD),
        dim: cell.flags.contains(Flags::DIM),
        italic: cell.flags.contains(Flags::ITALIC),
        underline: cell.flags.contains(Flags::UNDERLINE),
        strikethrough: cell.flags.contains(Flags::STRIKE_THROUGH),
        reverse: cell.flags.contains(Flags::INVERSE),
        hidden: cell.flags.contains(Flags::HIDDEN),
        blink: cell.flags.contains(Flags::BLINK),
        wide: cell.flags.contains(Flags::WIDE_CHAR),
        wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
    };

    let text = if flags.wide_spacer {
        // Spacer cells carry no meaningful glyph.
        String::new()
    } else {
        cell.c.to_string()
    };

    let width = if flags.wide {
        2
    } else if flags.wide_spacer {
        0
    } else {
        1
    };

    StyledCell { text, width, fg: map_color(cell.fg), bg: map_color(cell.bg), flags }
}

fn map_color(color: Color) -> TerminalColor {
    match color {
        Color::Specified(rgb) => TerminalColor::Rgb { r: rgb.r, g: rgb.g, b: rgb.b },
        Color::Indexed(index) => TerminalColor::Indexed(index),
        // Named colors (foreground/background/cursor) stay symbolic; the UI
        // theme resolves them, mirroring xterm.js's `ITheme` indirection.
        Color::Named(_) => TerminalColor::Default,
    }
}

/// Walk the alacritty grid's display iterator and assemble a [`GridSnapshot`].
pub(crate) fn build_snapshot<T: EventListener>(
    term: &mut Term<T>,
    size: TerminalSize,
    generation: u64,
) -> GridSnapshot {
    let cols = size.cols;
    let rows = size.rows;
    let mut cells: Vec<Vec<StyledCell>> = vec![Vec::new(); rows as usize];

    // The display iterator yields `(Point, &Cell)` for the visible viewport in
    // grid-absolute line numbers; bucket rows relative to the topmost line.
    // Two passes because the iterator is not guaranteed to start at the
    // top-left cell (it may iterate column-major).
    // PORT-TODO(alacritty_terminal 0.26): `Grid::display_iter()` has been
    // `&self` in past versions; going through `grid_mut()` also compiles if it
    // requires `&mut self`.
    let mut top_line: Option<usize> = None;
    for (point, _) in term.grid_mut().display_iter() {
        let line = point.line.0;
        top_line = Some(match top_line {
            Some(t) if t < line => t,
            _ => line,
        });
    }
    let top_line = top_line.unwrap_or(0);
    for (point, cell) in term.grid_mut().display_iter() {
        let Some(row) = point.line.0.checked_sub(top_line) else { continue };
        if row >= cells.len() {
            continue;
        }
        cells[row].push(styled_from_cell(cell));
    }

    // Pad short rows so every row has `cols` entries (keeps UI math trivial).
    for row in &mut cells {
        while row.len() < cols as usize {
            row.push(StyledCell::blank());
        }
        row.truncate(cols as usize);
    }

    let cursor = cursor_state(term, size);

    let alt_screen = term.mode().contains(alacritty_terminal::term::TermMode::ALTERNATE_SCREEN);

    GridSnapshot { cols, rows, cells, cursor, alt_screen, generation }
}

fn cursor_state<T: EventListener>(term: &Term<T>, size: TerminalSize) -> CursorState {
    // PORT-TODO(alacritty_terminal 0.26): `Term::cursor() -> &Cursor` with
    // public `point`/`shape`/`hidden` fields is the modern accessor; older
    // versions exposed `cursor_point()` + `cursor_hidden()` + `cursor_style()`
    // instead. Adjust here if the shape of `Cursor` differs.
    let cursor = term.cursor();
    let shape = match_cursor_shape(cursor.shape);
    if cursor.hidden {
        return CursorState { point: None, shape, visible: false };
    }
    let row = cursor.point.line.0 as u16;
    let col = cursor.point.col.0 as u16;
    if row >= size.rows || col >= size.cols {
        return CursorState { point: None, shape, visible: false };
    }
    CursorState { point: Some((row, col)), shape, visible: true }
}

#[allow(unreachable_patterns)]
fn match_cursor_shape(shape: alacritty_terminal::term::cell::CursorShape) -> CursorShape {
    use alacritty_terminal::term::cell::CursorShape as AlacrittyShape;
    match shape {
        AlacrittyShape::Underline => CursorShape::Underline,
        AlacrittyShape::Beam => CursorShape::Beam,
        // Block plus any future shapes.
        _ => CursorShape::Block,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_tracker_row_and_full() {
        let mut tracker = DirtyTracker::new(3);
        assert!(tracker.take(), "fresh tracker is dirty");
        assert!(!tracker.is_dirty(), "taken tracker is clean");

        tracker.mark_row(1);
        assert!(tracker.row_dirty(1));
        assert!(!tracker.row_dirty(0));
        assert!(tracker.take());
        assert!(!tracker.is_dirty());

        tracker.mark_rows(0, 2);
        assert!(tracker.row_dirty(0) && tracker.row_dirty(1) && !tracker.row_dirty(2));
        tracker.take();

        tracker.mark_all();
        assert!(tracker.row_dirty(2));
        tracker.take();

        tracker.resize(5);
        assert!(tracker.is_dirty());
    }

    #[test]
    fn styled_cell_blank_defaults() {
        let cell = StyledCell::blank();
        assert_eq!(cell.text, "");
        assert_eq!(cell.width, 1);
        assert_eq!(cell.fg, TerminalColor::Default);
        assert_eq!(cell.bg, TerminalColor::Default);
    }

    #[test]
    fn text_width_unicode() {
        assert_eq!(text_width("abc"), 3);
        assert_eq!(text_width("你好"), 4);
        assert_eq!(text_width(""), 0);
    }

    #[test]
    fn snapshot_cell_accessor() {
        let snap = GridSnapshot {
            cols: 2,
            rows: 1,
            cells: vec![vec![StyledCell::blank(), StyledCell::blank()]],
            cursor: CursorState::default(),
            alt_screen: false,
            generation: 1,
        };
        assert!(snap.cell(0, 1).is_some());
        assert!(snap.cell(0, 2).is_none());
        assert!(snap.cell(1, 0).is_none());
    }
}
