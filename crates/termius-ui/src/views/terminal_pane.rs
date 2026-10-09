//! terminal_pane — renders a [`GridSnapshot`] and routes input to the engine.
//!
//! * **Paint** — one flex row per terminal row; cells with equal styling are
//!   merged into [`TextRun`]s so an 80×24 screen costs a few hundred elements,
//!   not two thousand. The block cursor and the selection are color overrides
//!   applied while runs are built (both purely functional → unit-tested).
//! * **Input** — `KeyDownEvent` → [`map_key`] → [`TermiusState::send_keys`];
//!   the state encodes with `Terminal::key_down` and the bytes ride the
//!   bridge's input channel to the PTY. ⌘C/⌘V go through the clipboard.
//! * **Resize** — the pane measures the window minus the chrome constants
//!   (`views::app_shell`), divides by `CELL_WIDTH_PX`/`CELL_HEIGHT_PX`, and
//!   defers the resulting `TerminalSize` into the state (never mutates state
//!   mid-render).

use gpui::{
    div, px, AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    Rgba, ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window,
};
use termius_terminal::{GridSnapshot, Key as TermKey, TerminalSize, CELL_HEIGHT_PX, CELL_WIDTH_PX};

use crate::app_state::TermiusState;
use crate::theme::{theme_of, TermiusTheme};
use crate::views::app_shell::{SIDEBAR_WIDTH, SFTP_WIDTH, STATUS_BAR_HEIGHT, TAB_BAR_HEIGHT};

/// Inner padding of the terminal viewport (both axes).
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 8.0;
/// Painted monospace face; the cell metrics come from the terminal crate, so
/// the size is chosen to match `CELL_WIDTH_PX` (Menlo advance = 0.6em → 15px
/// = 9px).
/// PORT-TODO(gpui 0.2): measure via `window.text_system()` like the reference
/// implementation instead of assuming the fallback face metrics.
const TERMINAL_FONT: &str = "Menlo";
const TERMINAL_FONT_SIZE: f32 = 15.0;

/// One merged run of same-styled cells.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub fg: Rgba,
    pub bg: Rgba,
    pub bold: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

/// Build the styled runs for one row: paint every cell through the theme,
/// then coalesce neighbors that paint identically. The cursor cell always
/// becomes its own run (its background differs), and an empty cursor cell
/// still shows a space so the block is visible at end-of-line.
pub fn row_runs(
    frame: &GridSnapshot,
    row: usize,
    theme: &TermiusTheme,
    selected: &dyn Fn(usize) -> bool,
    cursor_col: Option<usize>,
) -> Vec<TextRun> {
    let Some(cells) = frame.cells.get(row) else { return Vec::new() };
    let mut runs: Vec<TextRun> = Vec::with_capacity(16);
    for (col, cell) in cells.iter().enumerate() {
        let is_cursor = cursor_col == Some(col);
        let paint = theme.paint_cell(cell, selected(col), is_cursor);
        let mut text = cell.text.as_str();
        if is_cursor && text.is_empty() {
            text = " ";
        }
        match runs.last_mut() {
            Some(run)
                if run.fg == paint.fg
                    && run.bg == paint.bg
                    && run.bold == paint.bold
                    && run.underline == paint.underline
                    && run.strikethrough == paint.strikethrough =>
            {
                run.text.push_str(text);
            }
            _ => runs.push(TextRun {
                text: text.to_owned(),
                fg: paint.fg,
                bg: paint.bg,
                bold: paint.bold,
                underline: paint.underline,
                strikethrough: paint.strikethrough,
            }),
        }
    }
    runs
}

/// Translate a GPUI keystroke into the terminal key sequence to send.
///
/// `key_char` carries the shifted/alt'd printable character; named keys
/// (`"up"`, `"f5"`, `"space"`, …) arrive in `key`. Control chords on letters
/// collapse to C0 codes; alt prefixes ESC. Sequences the `termius-terminal`
/// encoder cannot express (ctrl+arrow, bare ctrl+non-letter) return empty and
/// are dropped — see the PORT-TODO.
pub(crate) fn map_key(
    key: &str,
    key_char: Option<&str>,
    control: bool,
    alt: bool,
) -> Vec<TermKey> {
    // ctrl chords on letters → C0 control code (ctrl+c → ETX, …); ctrl+space
    // has no single-char key name, so it is matched by name here.
    if control && key == "space" {
        return vec![TermKey::Char('\0')];
    }
    if control && key.len() == 1 {
        let ch = key.chars().next().unwrap_or(' ');
        if ch.is_ascii_alphabetic() {
            return vec![TermKey::Char((ch.to_ascii_lowercase() as u8 - b'a' + 1) as char)];
        }
        if ch == ' ' || ch == '@' {
            return vec![TermKey::Char('\0')];
        }
        // PORT-TODO(gpui 0.2): ctrl+digit/punctuation need the terminal
        // encoder to accept pre-encoded bytes; drop them until then.
        return Vec::new();
    }

    let named = match key {
        "enter" => Some(TermKey::Enter),
        "tab" => Some(TermKey::Tab),
        "backspace" => Some(TermKey::Backspace),
        "delete" => Some(TermKey::Delete),
        "escape" => Some(TermKey::Escape),
        "up" => Some(TermKey::Up),
        "down" => Some(TermKey::Down),
        "left" => Some(TermKey::Left),
        "right" => Some(TermKey::Right),
        "home" => Some(TermKey::Home),
        "end" => Some(TermKey::End),
        "pageup" => Some(TermKey::PageUp),
        "pagedown" => Some(TermKey::PageDown),
        "insert" => Some(TermKey::Insert),
        "space" => Some(TermKey::Char(' ')),
        "f1" => Some(TermKey::F(1)),
        "f2" => Some(TermKey::F(2)),
        "f3" => Some(TermKey::F(3)),
        "f4" => Some(TermKey::F(4)),
        "f5" => Some(TermKey::F(5)),
        "f6" => Some(TermKey::F(6)),
        "f7" => Some(TermKey::F(7)),
        "f8" => Some(TermKey::F(8)),
        "f9" => Some(TermKey::F(9)),
        "f10" => Some(TermKey::F(10)),
        "f11" => Some(TermKey::F(11)),
        "f12" => Some(TermKey::F(12)),
        _ => None,
    };
    if let Some(named) = named {
        return if alt { vec![TermKey::Escape, named] } else { vec![named] };
    }

    // Printable: key_char honors shift/alt; fall back to a bare single char.
    let printable = key_char
        .and_then(|text| text.chars().next())
        .or_else(|| {
            let mut chars = key.chars();
            let first = chars.next()?;
            chars.next().is_none().then_some(first)
        })
        .filter(|ch| !ch.is_control());
    match printable {
        Some(ch) if alt => vec![TermKey::Escape, TermKey::Char(ch)],
        Some(ch) => vec![TermKey::Char(ch)],
        None => Vec::new(),
    }
}

/// The terminal pane view.
pub struct TerminalPane {
    state: Entity<TermiusState>,
    focus: FocusHandle,
}

impl TerminalPane {
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        Self { state, focus: cx.focus_handle() }
    }

    /// Focus the pane so keystrokes reach the active session.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    fn active_id(&self, cx: &App) -> Option<String> {
        self.state.read(cx).active_session.clone()
    }

    // ----- input ----------------------------------------------------------

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let Some(session_id) = self.active_id(cx) else { return };

        // Platform chords belong to the app: copy / paste.
        if keystroke.modifiers.platform {
            match keystroke.key.as_str() {
                "c" => {
                    if let Some(text) = self.state.read(cx).selection_text(&session_id) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.state
                            .update(cx, |state, cx| state.send_paste(&session_id, &text, cx));
                    }
                }
                _ => {}
            }
            return;
        }
        // fn-chords (media keys…) stay with the window.
        if keystroke.modifiers.function {
            return;
        }

        // Shift+arrows drive the selection.
        if keystroke.modifiers.shift
            && matches!(
                keystroke.key.as_str(),
                "up" | "down" | "left" | "right"
            )
        {
            let key = keystroke.key.clone();
            self.state.update(cx, |state, cx| state.selection_key(&session_id, &key, cx));
            return;
        }

        let keys = map_key(
            &keystroke.key,
            keystroke.key_char.as_deref(),
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
        );
        if keys.is_empty() {
            return;
        }
        self.state.update(cx, |state, cx| state.send_keys(&session_id, &keys, cx));
    }

    fn handle_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        // PORT-TODO(gpui 0.2): mouse-drag selection (begin/extend via
        // `on_mouse_down`/`on_mouse_move`) needs the grid origin measured
        // from the pane bounds; today selection is shift+arrows only.
        let Some(session_id) = self.active_id(cx) else { return };
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            // PORT-TODO(gpui 0.2): confirm wheel sign; a natural-scroll trackpad
            // may need this negated.
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / CELL_HEIGHT_PX,
        };
        let lines = lines.round() as i32;
        self.state.update(cx, |state, cx| state.scroll_session(&session_id, lines, cx));
    }

    /// Apply a geometry deferred from `render` (state must not be mutated
    /// while rendering).
    fn apply_resize(&mut self, session_id: &str, size: TerminalSize, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.resize_session(session_id, size, cx));
    }

    // ----- geometry -------------------------------------------------------

    /// Grid size fitting the window minus chrome and padding.
    fn fit_geometry(
        window: &Window,
        sidebar: bool,
        sftp: bool,
    ) -> (u16, u16) {
        let size = window.bounds().size;
        let chrome_w =
            (if sidebar { SIDEBAR_WIDTH } else { 0.0 }) + (if sftp { SFTP_WIDTH } else { 0.0 });
        let chrome_h = TAB_BAR_HEIGHT + STATUS_BAR_HEIGHT;
        let avail_w = f32::from(size.width) - chrome_w - PAD_X * 2.0;
        let avail_h = f32::from(size.height) - chrome_h - PAD_Y * 2.0;
        let cols = (avail_w / CELL_WIDTH_PX).floor().max(2.0) as u16;
        let rows = (avail_h / CELL_HEIGHT_PX).floor().max(2.0) as u16;
        (cols, rows)
    }
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let (sidebar, sftp) = {
            let state = self.state.read(cx);
            (state.sidebar_visible, state.sftp_visible)
        };
        let (fit_cols, fit_rows) = Self::fit_geometry(window, sidebar, sftp);

        let mut pending_resize: Option<(String, TerminalSize)> = None;
        let mut badge: Option<String> = None;

        // Everything below borrows the state; listeners and the deferred
        // resize are attached afterwards so the borrow ends first.
        let body: AnyElement = {
            let state = self.state.read(cx);
            match state.active_session() {
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted)
                    .child(SharedString::from(
                        "Select a host on the left and press ⏎ to connect",
                    ))
                    .into_any_element(),
                Some(session) => {
                    if !session.status.is_live() {
                        badge = Some(session.status.describe());
                    } else if (fit_cols, fit_rows) != (session.size.cols, session.size.rows) {
                        pending_resize = Some((
                            session.id.clone(),
                            TerminalSize {
                                cols: fit_cols,
                                rows: fit_rows,
                                width_px: None,
                                height_px: None,
                            },
                        ));
                    }

                    let frame = &session.frame;
                    let cols = frame.cols;
                    let term = &session.term;
                    let mut grid = div()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .font_family(TERMINAL_FONT)
                        .text_size(px(TERMINAL_FONT_SIZE))
                        .bg(theme.term_background)
                        .text_color(theme.term_foreground);

                    for row in 0..usize::from(frame.rows) {
                        let cursor_col = frame
                            .cursor
                            .point
                            .filter(|(at_row, _)| usize::from(*at_row) == row)
                            .map(|(_, at_col)| usize::from(at_col))
                            .filter(|_| frame.cursor.visible);
                        let runs = row_runs(frame, row, &theme, &|col| {
                            term.selection_model().contains(row as u16, col as u16, cols)
                        }, cursor_col);

                        let mut row_el = div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .h(px(CELL_HEIGHT_PX));
                        for run in runs {
                            let mut cell_el = div().text_color(run.fg).bg(run.bg);
                            if run.bold {
                                cell_el = cell_el.font_weight(FontWeight::BOLD);
                            }
                            if run.underline || run.strikethrough {
                                // PORT-TODO(gpui 0.2): text-decoration styling
                                // per run (gpui exposes underline variants with
                                // a style argument in some releases).
                                cell_el = cell_el.border_b_1().border_color(run.fg);
                            }
                            cell_el = cell_el.child(SharedString::from(run.text));
                            row_el = row_el.child(cell_el);
                        }
                        grid = grid.child(row_el);
                    }
                    grid.into_any_element()
                }
            }
        };

        let mut root = div()
            .id("terminal-pane")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
            .p(px(PAD_Y))
            .px(px(PAD_X))
            .bg(theme.term_background)
            .child(body);

        if let Some(badge) = badge {
            root = root.child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(theme.tab_active)
                    .text_color(theme.muted)
                    .child(SharedString::from(badge)),
            );
        }

        // Geometry changes discovered during render are applied after it:
        // state must not be mutated while rendering.
        if let Some((session_id, size)) = pending_resize {
            cx.defer_in(window, move |this, _window, cx| {
                this.apply_resize(&session_id, size, cx);
            });
        }

        root.track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key(event, cx);
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                this.handle_scroll(event, cx);
            }))
            .on_click(cx.listener(|this, _event, window, cx| {
                this.focus(window, cx);
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termius_terminal::{CellFlags, CursorState, StyledCell, TerminalColor};

    fn frame_1x3() -> GridSnapshot {
        GridSnapshot {
            cols: 3,
            rows: 1,
            cells: vec![vec![
                StyledCell::blank(),
                StyledCell::blank(),
                StyledCell::blank(),
            ]],
            cursor: CursorState { point: None, shape: Default::default(), visible: false },
            alt_screen: false,
            generation: 1,
        }
    }

    fn cell(text: &str, fg: TerminalColor, bg: TerminalColor) -> StyledCell {
        StyledCell {
            text: text.to_owned(),
            width: 1,
            fg,
            bg,
            flags: CellFlags::default(),
        }
    }

    #[test]
    fn merges_runs_with_identical_style() {
        let theme = TermiusTheme::dark();
        let mut frame = frame_1x3();
        frame.cells[0] = vec![
            cell("a", TerminalColor::Indexed(1), TerminalColor::Default),
            cell("b", TerminalColor::Indexed(1), TerminalColor::Default),
            cell("c", TerminalColor::Indexed(2), TerminalColor::Default),
        ];
        let none = |_col: usize| false;
        let runs = row_runs(&frame, 0, &theme, &none, None);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].fg, theme.ansi[1]);
        assert_eq!(runs[1].text, "c");
        assert_eq!(runs[1].fg, theme.ansi[2]);
    }

    #[test]
    fn cursor_cell_paints_even_when_blank() {
        let theme = TermiusTheme::dark();
        let frame = frame_1x3();
        let none = |_col: usize| false;
        let runs = row_runs(&frame, 0, &theme, &none, Some(2));
        assert_eq!(runs.len(), 2);
        // Blank cells render as empty text; the cursor cell gets a space so
        // the block is visible at end-of-line.
        assert_eq!(runs[0].text, "");
        // Cursor run carries the cursor fill and stops before it.
        assert_eq!(runs[1].text, " ");
        assert_eq!(runs[1].bg, theme.term_cursor);
        assert_eq!(runs[1].fg, theme.term_background);
    }

    #[test]
    fn selection_forces_a_run_boundary() {
        let theme = TermiusTheme::dark();
        let mut frame = frame_1x3();
        frame.cells[0] = vec![
            cell("a", TerminalColor::Default, TerminalColor::Default),
            cell("b", TerminalColor::Default, TerminalColor::Default),
            cell("c", TerminalColor::Default, TerminalColor::Default),
        ];
        let selected = |col: usize| col == 1;
        let runs = row_runs(&frame, 0, &theme, &selected, None);
        // The selected middle cell breaks the row into three runs.
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "a");
        assert_eq!(runs[1].text, "b");
        assert_eq!(runs[2].text, "c");
        assert_ne!(runs[1].bg, runs[0].bg);
        assert_eq!(runs[0].bg, runs[2].bg);
    }

    #[test]
    fn out_of_range_row_is_empty() {
        let theme = TermiusTheme::dark();
        let frame = frame_1x3();
        let none = |_col: usize| false;
        assert!(row_runs(&frame, 5, &theme, &none, None).is_empty());
    }

    #[test]
    fn maps_named_keys() {
        assert_eq!(map_key("up", None, false, false), vec![TermKey::Up]);
        assert_eq!(map_key("enter", None, false, false), vec![TermKey::Enter]);
        assert_eq!(map_key("f5", None, false, false), vec![TermKey::F(5)]);
        assert_eq!(map_key("space", None, false, false), vec![TermKey::Char(' ')]);
        assert_eq!(map_key("pageup", None, false, false), vec![TermKey::PageUp]);
    }

    #[test]
    fn maps_printables_with_shift_via_key_char() {
        assert_eq!(map_key("a", Some("A"), false, false), vec![TermKey::Char('A')]);
        assert_eq!(map_key("a", None, false, false), vec![TermKey::Char('a')]);
        // Alt prefixes ESC.
        assert_eq!(map_key("x", Some("x"), false, true), vec![TermKey::Escape, TermKey::Char('x')]);
        // Named key + alt also prefixes ESC.
        assert_eq!(map_key("left", None, false, true), vec![TermKey::Escape, TermKey::Left]);
    }

    #[test]
    fn maps_control_chords_to_c0_codes() {
        assert_eq!(map_key("c", None, true, false), vec![TermKey::Char('\x03')]);
        assert_eq!(map_key("d", None, true, false), vec![TermKey::Char('\x04')]);
        assert_eq!(map_key("space", None, true, false), vec![TermKey::Char('\0')]);
        // Unencodable control chords are dropped, not mangled.
        assert!(map_key("1", Some("1"), true, false).is_empty());
        // Unknown named keys are dropped too.
        assert!(map_key("metaleft", None, false, false).is_empty());
    }
}
