//! theme — the centralized dark/light palette.
//!
//! Replaces the React renderer's CSS variables / Termius `ITheme` indirection:
//! every view reads colors from a single [`TermiusTheme`] value, and terminal
//! cells resolve their symbolic [`TerminalColor`] through it, so an app theme
//! switch repaints the terminal too.
//!
//! The theme is registered as a GPUI [`Global`] so any view can read it with
//! [`theme_of`]; `termius_ui::views::init` installs the dark variant.

use gpui::{App, Global, Rgba};

use termius_terminal::{StyledCell, TerminalColor};

/// Which palette the app is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

/// Pack 8-bit sRGB channels into an opaque [`Rgba`].
pub(crate) fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
}

/// The same color at a given alpha (`Rgba` has no `opacity` helper).
pub(crate) fn with_alpha(color: Rgba, a: f32) -> Rgba {
    Rgba { a, ..color }
}

/// Standard source-over blend of `top` (possibly translucent) onto `base`.
pub(crate) fn over(base: Rgba, top: Rgba) -> Rgba {
    let out_a = top.a + base.a * (1.0 - top.a);
    if out_a <= 0.0 {
        return Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    }
    Rgba {
        r: (top.r * top.a + base.r * base.a * (1.0 - top.a)) / out_a,
        g: (top.g * top.a + base.g * base.a * (1.0 - top.a)) / out_a,
        b: (top.b * top.a + base.b * base.a * (1.0 - top.a)) / out_a,
        a: out_a,
    }
}

/// How one cell should be painted, after all overrides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellPaint {
    pub fg: Rgba,
    pub bg: Rgba,
    pub bold: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

/// The app palette: chrome colors plus the terminal's ANSI table.
#[derive(Debug, Clone, Copy)]
pub struct TermiusTheme {
    pub mode: ThemeMode,
    // --- chrome ---------------------------------------------------------
    pub background: Rgba,
    pub foreground: Rgba,
    pub accent: Rgba,
    pub border: Rgba,
    pub selection: Rgba,
    pub sidebar_background: Rgba,
    pub tab_background: Rgba,
    pub tab_active: Rgba,
    pub status_background: Rgba,
    pub muted: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub hover: Rgba,
    // --- terminal -------------------------------------------------------
    pub term_background: Rgba,
    pub term_foreground: Rgba,
    pub term_cursor: Rgba,
    /// The 16 ANSI colors (`TerminalColor::Indexed(0..=15)`).
    pub ansi: [Rgba; 16],
    // --- real Termius design tokens (resolved from the original CSS vars) --
    pub card_a: Rgba,
    pub card_b: Rgba,
    pub card_c: Rgba,
    pub title: Rgba,
    pub text_common: Rgba,
    pub border_basic: Rgba,
    pub border_light: Rgba,
    pub border_strong: Rgba,
    pub border_accent: Rgba,
    pub primary: Rgba,
    pub primary_dark: Rgba,
    pub primary_light: Rgba,
    pub blueberry: Rgba,
    pub yellow: Rgba,
    pub green: Rgba,
    pub backdrop: Rgba,
    pub corner_radius_small: f32,
    pub corner_radius_medium: f32,
    pub corner_radius_large: f32,
}

impl Global for TermiusTheme {}

impl TermiusTheme {
    /// The default dark palette (Termius' default appearance).
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            background: rgb(0x1d, 0x20, 0x33),   // --main-bg (dark-grey-2)
            foreground: rgb(0xf7, 0xf9, 0xfa),   // --c-title
            accent: rgb(0x20, 0x91, 0xf6),        // --blue
            border: rgb(0x3e, 0x42, 0x57),        // --border-strong
            selection: with_alpha(rgb(0x20, 0x91, 0xf6), 0.50), // --blue-a50
            sidebar_background: rgb(0x14, 0x17, 0x29), // --dark-grey-1
            tab_background: rgb(0x14, 0x17, 0x29),
            tab_active: rgb(0x28, 0x2b, 0x3d),    // --card-a
            status_background: rgb(0x14, 0x17, 0x29),
            muted: rgb(0xa4, 0xb3, 0xba),         // --c-text-common
            danger: rgb(0xf2, 0x5e, 0x61),        // --red
            success: rgb(0x21, 0xb5, 0x68),       // --green
            hover: with_alpha(rgb(0x20, 0x91, 0xf6), 0.10), // --blue-a10
            term_background: rgb(0x14, 0x17, 0x29),
            term_foreground: rgb(0xf7, 0xf9, 0xfa),
            term_cursor: rgb(0x20, 0x91, 0xf6),
            ansi: [
                rgb(0x23, 0x25, 0x3b), //  0 black
                rgb(0xf2, 0x5e, 0x61), //  1 red
                rgb(0x21, 0xb5, 0x68), //  2 green
                rgb(0xe5, 0xc0, 0x7b), //  3 yellow
                rgb(0x20, 0x91, 0xf6), //  4 blue
                rgb(0xc6, 0x78, 0xdd), //  5 magenta
                rgb(0x56, 0xb6, 0xc2), //  6 cyan
                rgb(0xf7, 0xf9, 0xfa), //  7 white
                rgb(0x8d, 0x91, 0xa5), //  8 bright black
                rgb(0xff, 0x8b, 0x92), //  9 bright red
                rgb(0x7e, 0xe7, 0x87), // 10 bright green
                rgb(0xff, 0xcb, 0x00), // 11 bright yellow
                rgb(0x58, 0xad, 0xf8), // 12 bright blue
                rgb(0xd2, 0xa8, 0xff), // 13 bright magenta
                rgb(0x8b, 0xe9, 0xfd), // 14 bright cyan
                rgb(0xff, 0xff, 0xff), // 15 bright white
            ],
            card_a: rgb(0x28, 0x2b, 0x3d),
            card_b: rgb(0x1d, 0x20, 0x33),
            card_c: rgb(0x32, 0x36, 0x4a),
            title: rgb(0xf7, 0xf9, 0xfa),
            text_common: rgb(0xa4, 0xb3, 0xba),
            border_basic: with_alpha(rgb(0x8d, 0x91, 0xa5), 0.25),
            border_light: with_alpha(rgb(0x8d, 0x91, 0xa5), 0.10),
            border_strong: rgb(0x3e, 0x42, 0x57),
            border_accent: rgb(0x20, 0x91, 0xf6),
            primary: rgb(0x20, 0x91, 0xf6),
            primary_dark: rgb(0x18, 0x6c, 0xb5),
            primary_light: rgb(0x58, 0xad, 0xf8),
            blueberry: rgb(0x66, 0x66, 0xd2),
            yellow: rgb(0xff, 0xcb, 0x00),
            green: rgb(0x21, 0xb5, 0x68),
            backdrop: with_alpha(rgb(0x00, 0x00, 0x00), 0.50),
            corner_radius_small: 5.0,
            corner_radius_medium: 10.0,
            corner_radius_large: 15.0,
        }
    }

    /// The light palette.
    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            background: rgb(0xff, 0xff, 0xff),   // --main-bg (white)
            foreground: rgb(0x14, 0x17, 0x29),   // --c-title
            accent: rgb(0x20, 0x91, 0xf6),        // --blue
            border: rgb(0xd5, 0xdd, 0xe0),        // --border-strong
            selection: with_alpha(rgb(0x20, 0x91, 0xf6), 0.20),
            sidebar_background: rgb(0xf7, 0xf9, 0xfa), // --card-a
            tab_background: rgb(0xe6, 0xeb, 0xed),     // --card-b
            tab_active: rgb(0xff, 0xff, 0xff),         // --card-c
            status_background: rgb(0xf7, 0xf9, 0xfa),
            muted: rgb(0xa4, 0xb3, 0xba),         // --c-text-common
            danger: rgb(0xf2, 0x5e, 0x61),        // --red
            success: rgb(0x21, 0xb5, 0x68),       // --green
            hover: with_alpha(rgb(0x20, 0x91, 0xf6), 0.10),
            term_background: rgb(0xff, 0xff, 0xff),
            term_foreground: rgb(0x14, 0x17, 0x29),
            term_cursor: rgb(0x20, 0x91, 0xf6),
            ansi: [
                rgb(0x33, 0x36, 0x48), //  0 black
                rgb(0xf2, 0x5e, 0x61), //  1 red
                rgb(0x21, 0xb5, 0x68), //  2 green
                rgb(0xa8, 0x7c, 0x1a), //  3 yellow
                rgb(0x20, 0x91, 0xf6), //  4 blue
                rgb(0x9b, 0x4f, 0xbd), //  5 magenta
                rgb(0x1f, 0x8a, 0x99), //  6 cyan
                rgb(0xd6, 0xdd, 0xe0), //  7 white
                rgb(0xa4, 0xb3, 0xba), //  8 bright black
                rgb(0xe0, 0x55, 0x61), //  9 bright red
                rgb(0x57, 0xb2, 0x6f), // 10 bright green
                rgb(0xd9, 0xa6, 0x4a), // 11 bright yellow
                rgb(0x52, 0x8b, 0xff), // 12 bright blue
                rgb(0xc6, 0x78, 0xdd), // 13 bright magenta
                rgb(0x56, 0xb6, 0xc2), // 14 bright cyan
                rgb(0xff, 0xff, 0xff), // 15 bright white
            ],
            card_a: rgb(0xf7, 0xf9, 0xfa),
            card_b: rgb(0xe6, 0xeb, 0xed),
            card_c: rgb(0xff, 0xff, 0xff),
            title: rgb(0x14, 0x17, 0x29),
            text_common: rgb(0xa4, 0xb3, 0xba),
            border_basic: with_alpha(rgb(0x79, 0x8c, 0x94), 0.25),
            border_light: with_alpha(rgb(0x79, 0x8c, 0x94), 0.10),
            border_strong: rgb(0xd5, 0xdd, 0xe0),
            border_accent: rgb(0x20, 0x91, 0xf6),
            primary: rgb(0x20, 0x91, 0xf6),
            primary_dark: rgb(0x18, 0x6c, 0xb5),
            primary_light: rgb(0x58, 0xad, 0xf8),
            blueberry: rgb(0x66, 0x66, 0xd2),
            yellow: rgb(0xff, 0xcb, 0x00),
            green: rgb(0x21, 0xb5, 0x68),
            backdrop: with_alpha(rgb(0x14, 0x17, 0x29), 0.50),
            corner_radius_small: 5.0,
            corner_radius_medium: 10.0,
            corner_radius_large: 15.0,
        }
    }

    /// The palette for a mode.
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// The opposite palette (theme toggle).
    pub fn toggled(&self) -> Self {
        match self.mode {
            ThemeMode::Dark => Self::light(),
            ThemeMode::Light => Self::dark(),
        }
    }

    /// Resolve one symbolic terminal color: `Default` falls back to the
    /// theme's terminal default, `Indexed` goes through the palette (with the
    /// 256-color cube + grayscale ramp), `Rgb` is used verbatim.
    pub fn resolve(&self, color: TerminalColor, fallback: Rgba) -> Rgba {
        match color {
            TerminalColor::Default => fallback,
            TerminalColor::Rgb { r, g, b } => rgb(r, g, b),
            TerminalColor::Indexed(index) => self.indexed(index),
        }
    }

    /// Resolve a 256-color palette index.
    pub fn indexed(&self, index: u8) -> Rgba {
        match index {
            0..=15 => self.ansi[usize::from(index)],
            16..=231 => {
                let n = index - 16;
                let level = |x: u8| if x == 0 { 0 } else { 55 + 40 * x };
                rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            // 24 steps of 10 from 8 to 238.
            _ => {
                let value = 8 + u8::from(index - 232) * 10;
                rgb(value, value, value)
            }
        }
    }

    /// The full paint plan for one cell, applying (in order) the theme
    /// defaults, the reverse-video swap, selection, the block cursor, hidden
    /// text and dimming.
    pub fn paint_cell(&self, cell: &StyledCell, selected: bool, is_cursor: bool) -> CellPaint {
        let mut fg = self.resolve(cell.fg, self.term_foreground);
        let mut bg = self.resolve(cell.bg, self.term_background);
        if cell.flags.reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if selected {
            bg = over(bg, self.selection);
        }
        if is_cursor {
            // Block cursor: cursor-colored fill with the page color text.
            bg = self.term_cursor;
            fg = self.term_background;
        }
        if cell.flags.hidden {
            fg = bg;
        }
        if cell.flags.dim {
            fg = with_alpha(fg, 0.6);
        }
        CellPaint {
            fg,
            bg,
            bold: cell.flags.bold,
            underline: cell.flags.underline,
            strikethrough: cell.flags.strikethrough,
        }
    }
}

/// Read the registered theme (dark fallback when `init` never ran).
pub fn theme_of(cx: &App) -> TermiusTheme {
    cx.try_global::<TermiusTheme>()
        .copied()
        .unwrap_or_else(TermiusTheme::dark)
}

#[cfg(test)]
mod tests {
    use super::*;
    use termius_terminal::CellFlags;

    fn cell(fg: TerminalColor, bg: TerminalColor) -> StyledCell {
        StyledCell { text: "x".into(), width: 1, fg, bg, flags: CellFlags::default() }
    }

    #[test]
    fn resolves_symbolic_colors() {
        let theme = TermiusTheme::dark();
        let fallback = theme.term_foreground;
        assert_eq!(theme.resolve(TerminalColor::Default, fallback), fallback);
        assert_eq!(
            theme.resolve(TerminalColor::Rgb { r: 1, g: 2, b: 3 }, fallback),
            rgb(1, 2, 3)
        );
        assert_eq!(theme.resolve(TerminalColor::Indexed(1), fallback), theme.ansi[1]);
    }

    #[test]
    fn indexed_256_uses_cube_and_grayscale() {
        let theme = TermiusTheme::dark();
        // 16 → (0,0,0), 231 → (255,255,255) top of the cube.
        assert_eq!(theme.indexed(16), rgb(0, 0, 0));
        assert_eq!(theme.indexed(231), rgb(255, 255, 255));
        // 196 = n 180 → (5,0,0) → pure red.
        assert_eq!(theme.indexed(196), rgb(255, 0, 0));
        // Grayscale ramp: 232 → 8, 255 → 238.
        assert_eq!(theme.indexed(232), rgb(8, 8, 8));
        assert_eq!(theme.indexed(255), rgb(238, 238, 238));
    }

    #[test]
    fn reverse_video_swaps_resolved_colors() {
        let theme = TermiusTheme::dark();
        let mut reversed = cell(TerminalColor::Indexed(1), TerminalColor::Indexed(2));
        reversed.flags.reverse = true;
        let paint = theme.paint_cell(&reversed, false, false);
        assert_eq!(paint.fg, theme.ansi[2]);
        assert_eq!(paint.bg, theme.ansi[1]);
    }

    #[test]
    fn cursor_overrides_and_selection_blends() {
        let theme = TermiusTheme::dark();
        let plain = cell(TerminalColor::Default, TerminalColor::Default);
        let paint = theme.paint_cell(&plain, false, true);
        assert_eq!(paint.bg, theme.term_cursor);
        assert_eq!(paint.fg, theme.term_background);

        let selected = theme.paint_cell(&plain, true, false);
        assert_eq!(selected.bg, over(theme.term_background, theme.selection));
        // Selection must visibly differ from the untouched background.
        assert_ne!(selected.bg, theme.term_background);
    }

    #[test]
    fn light_and_dark_palettes_differ() {
        let dark = TermiusTheme::dark();
        let light = TermiusTheme::light();
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.ansi, light.ansi);
        assert_eq!(dark.toggled().mode, ThemeMode::Light);
        assert_eq!(light.toggled().mode, ThemeMode::Dark);
    }

    #[test]
    fn hidden_and_dim_adjust_foreground() {
        let theme = TermiusTheme::dark();
        let mut hidden = cell(TerminalColor::Indexed(1), TerminalColor::Indexed(2));
        hidden.flags.hidden = true;
        let paint = theme.paint_cell(&hidden, false, false);
        assert_eq!(paint.fg, paint.bg);

        let mut dim = cell(TerminalColor::Default, TerminalColor::Default);
        dim.flags.dim = true;
        let paint = theme.paint_cell(&dim, false, false);
        assert_eq!(paint.fg.a, 0.6);
    }
}
