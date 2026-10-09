//! selection — text selection model on top of alacritty's `Selection`.
//!
//! The model mirrors the xterm.js selection surface Termius used:
//! `simpleSelection` (charwise drag), `semanticSelection` (double-click word),
//! `lineSelection` (triple-click line), `selectAll` and `clearSelection`, plus
//! selection-change notifications for the right-click-to-paste flow.
//!
//! Geometry lives in viewport coordinates (`CellPosition`) so the UI can paint
//! highlights without knowing about alacritty's grid-absolute `Point` space;
//! [`crate::Terminal`] translates at the boundary and delegates text
//! extraction to `Term::selection_to_string()`.

use serde::{Deserialize, Serialize};

/// Selection granularity, matching xterm.js `SelectionType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionMode {
    /// Charwise drag selection (mouse down + move).
    #[default]
    Char,
    /// Word selection (double-click / double-tap).
    Word,
    /// Whole-line selection (triple-click / triple-tap).
    Line,
    /// Entire buffer (Cmd/Ctrl+A "select all").
    All,
}

impl SelectionMode {
    /// True for modes that always span entire lines when painted.
    pub fn is_full_line(&self) -> bool {
        matches!(self, SelectionMode::Word | SelectionMode::Line | SelectionMode::All)
    }

    /// Map onto the alacritty `SelectionType` used for text extraction.
    ///
    /// [`SelectionMode::All`] has no alacritty counterpart; callers build a
    /// grid-spanning [`Lines`](alacritty_terminal::selection::SelectionType::Lines)
    /// selection instead, so this returns `Lines` for it as well.
    pub(crate) fn alacritty_type(&self) -> alacritty_terminal::selection::SelectionType {
        match self {
            SelectionMode::Char => alacritty_terminal::selection::SelectionType::Simple,
            SelectionMode::Word => alacritty_terminal::selection::SelectionType::Semantic,
            SelectionMode::Line | SelectionMode::All => {
                alacritty_terminal::selection::SelectionType::Lines
            },
        }
    }
}

/// A viewport cell coordinate (row 0 = top of the visible grid).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellPosition {
    pub row: u16,
    pub col: u16,
}

impl CellPosition {
    pub fn new(row: u16, col: u16) -> Self {
        Self { row, col }
    }

    /// Ordering key for normalizing anchor/focus.
    fn key(self) -> (u16, u16) {
        (self.row, self.col)
    }
}

/// An active selection: where the user started (`anchor`), where the pointer
/// is now (`focus`) and the granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSelection {
    pub anchor: CellPosition,
    pub focus: CellPosition,
    pub mode: SelectionMode,
}

impl TerminalSelection {
    pub fn new(anchor: CellPosition, mode: SelectionMode) -> Self {
        Self { anchor, focus: anchor, mode }
    }

    /// Move the focus end (mouse drag / shift-extension).
    pub fn extend(&mut self, focus: CellPosition) {
        self.focus = focus;
    }

    /// `(start, end)` in reading order, inclusive.
    pub fn normalized(&self) -> (CellPosition, CellPosition) {
        if self.anchor.key() <= self.focus.key() {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    /// True if the selection runs backwards (focus above/left of anchor).
    pub fn is_reversed(&self) -> bool {
        self.focus.key() < self.anchor.key()
    }

    /// Whether the given viewport cell falls inside the selection range.
    ///
    /// For [`SelectionMode::Word`]/[`SelectionMode::Line`]/
    /// [`SelectionMode::All`] the range is row-exact and spans every column;
    /// for charwise selections the (row, col) box is applied.
    pub fn contains(&self, row: u16, col: u16, cols: u16) -> bool {
        let (start, end) = self.normalized();
        if row < start.row || row > end.row {
            return false;
        }
        let first_col = if row == start.row && self.mode == SelectionMode::Char { start.col } else { 0 };
        let last_col = if row == end.row && self.mode == SelectionMode::Char {
            end.col
        } else {
            cols.saturating_sub(1)
        };
        col >= first_col && col <= last_col
    }
}

/// The UI-facing selection state machine.
///
/// The model is pure geometry; binding it to the alacritty `Term` (which owns
/// the authoritative selection used for text extraction) is
/// [`crate::Terminal`]'s job — every mutation here is mirrored into the term.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionModel {
    selection: Option<TerminalSelection>,
}

impl SelectionModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently active selection, if any.
    pub fn selection(&self) -> Option<&TerminalSelection> {
        self.selection.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.selection.is_none()
    }

    /// Begin a selection of the given mode (mouse-down / double / triple click).
    pub fn begin(&mut self, at: CellPosition, mode: SelectionMode) {
        self.selection = Some(TerminalSelection::new(at, mode));
    }

    /// Extend the active selection to `at`; no-op when nothing is selected.
    pub fn extend(&mut self, at: CellPosition) {
        if let Some(selection) = &mut self.selection {
            selection.extend(at);
        }
    }

    /// Replace the active selection wholesale (e.g. re-anchoring after the
    /// underlying buffer changed).
    pub fn set(&mut self, selection: TerminalSelection) {
        self.selection = Some(selection);
    }

    /// Drop the selection (mouse-up with empty region, Esc, Ctrl+C copy…).
    pub fn clear(&mut self) {
        self.selection = None;
    }

    /// True if the given viewport cell is inside the active selection.
    pub fn contains(&self, row: u16, col: u16, cols: u16) -> bool {
        self.selection
            .as_ref()
            .map(|s| s.contains(row, col, cols))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_extend_clear() {
        let mut model = SelectionModel::new();
        assert!(model.is_empty());

        model.begin(CellPosition::new(0, 2), SelectionMode::Char);
        assert!(!model.is_empty());
        assert_eq!(model.selection().map(|s| s.mode), Some(SelectionMode::Char));

        model.extend(CellPosition::new(1, 4));
        let (start, end) = model.selection().copied().unwrap().normalized();
        assert_eq!(start, CellPosition::new(0, 2));
        assert_eq!(end, CellPosition::new(1, 4));

        model.clear();
        assert!(model.is_empty());
    }

    #[test]
    fn reversed_selection_normalizes() {
        let mut model = SelectionModel::new();
        model.begin(CellPosition::new(2, 0), SelectionMode::Char);
        model.extend(CellPosition::new(0, 5));
        assert!(model.selection().unwrap().is_reversed());
        let (start, end) = model.selection().unwrap().normalized();
        assert_eq!(start, CellPosition::new(0, 5));
        assert_eq!(end, CellPosition::new(2, 0));
    }

    #[test]
    fn contains_charwise_box() {
        let selection = TerminalSelection::new(CellPosition::new(0, 2), SelectionMode::Char);
        let mut selection = selection;
        selection.extend(CellPosition::new(1, 3));

        assert!(!selection.contains(0, 1, 80), "before anchor col");
        assert!(selection.contains(0, 2, 80));
        assert!(selection.contains(0, 79, 80), "middle row is fully covered");
        assert!(selection.contains(1, 0, 80));
        assert!(selection.contains(1, 3, 80));
        assert!(!selection.contains(1, 4, 80), "after focus col");
        assert!(!selection.contains(2, 0, 80), "outside rows");
    }

    #[test]
    fn contains_linewise_spans_full_rows() {
        let selection = TerminalSelection::new(CellPosition::new(1, 5), SelectionMode::Line);
        assert!(selection.contains(1, 0, 80));
        assert!(selection.contains(1, 79, 80));
        assert!(!selection.contains(0, 0, 80));
    }

    #[test]
    fn model_contains_uses_active_selection_only() {
        let mut model = SelectionModel::new();
        assert!(!model.contains(0, 0, 80));
        model.begin(CellPosition::new(0, 0), SelectionMode::Char);
        model.extend(CellPosition::new(0, 2));
        assert!(model.contains(0, 1, 80));
        assert!(!model.contains(0, 3, 80));
    }

    #[test]
    fn extend_without_begin_is_noop() {
        let mut model = SelectionModel::new();
        model.extend(CellPosition::new(0, 0));
        assert!(model.is_empty());
    }

    #[test]
    fn full_line_modes_flag() {
        assert!(!SelectionMode::Char.is_full_line());
        assert!(SelectionMode::Word.is_full_line());
        assert!(SelectionMode::Line.is_full_line());
        assert!(SelectionMode::All.is_full_line());
    }
}
