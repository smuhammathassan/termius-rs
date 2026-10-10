//! theme — the centralized dark/light palette.
//!
//! Replaces the React renderer's CSS variables / Termius `ITheme` indirection:
//! every view reads colors from a single [`TermiusTheme`] value, and terminal
//! cells resolve their symbolic [`TerminalColor`] through it, so an app theme
//! switch repaints the terminal too.
//!
//! The theme is registered as a GPUI [`Global`] so any view can read it with
//! [`theme_of`]; `termius_ui::views::init` installs the dark variant.

use gpui::{px, App, FontWeight, Global, Rgba, Styled, WindowAppearance};

use termius_terminal::{StyledCell, TerminalColor};

/// The UI font family (Lineto Circular, embedded via [`crate::assets`]).
pub use crate::assets::UI_FONT;

/// `--horizontal-tabs-height` — the top strip / spacer height (`51px`).
///
/// Source: `analysis/termius-theme.json` (`--horizontal-tabs-height: 51px`,
/// dark + light) and `analysis/recon/00-shell.md` §4/§5 (`qDe` spacer height,
/// `paneSplit` `calc(100% - 51px)`).
pub const HORIZONTAL_TABS_HEIGHT: f32 = 51.0;

/// `pane1.visible` width — the left section-nav rail (`185px`).
///
/// Source: `analysis/recon/00-shell.md` §5 (`pane1.visible { width: 185px }`).
pub const LEFT_PANEL_WIDTH: f32 = 185.0;

/// `pane1.visible` width under `@media (max-width: 800px)` — the icon-only
/// rail (`68px`). Source: `analysis/recon/00-shell.md` §5.
pub const LEFT_PANEL_COMPACT_WIDTH: f32 = 68.0;

/// One entry of the original Termius typography scale.
///
/// These are the exact tokens from the renderer's `theme.typography.fonts`
/// object (see `analysis/readable/_main.js` and the recovered
/// `reconnectSaga-*.js`): `{size, weight, line-height}` in CSS px. Token names
/// follow the original (`r`egular/`b`old + size + colour role).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextToken {
    /// Font size in px.
    pub size: f32,
    /// Font weight (CSS numeric, e.g. 450 / 700).
    pub weight: f32,
    /// Line height in px.
    pub line_height: f32,
    /// Letter spacing in px (0 = none).
    pub letter_spacing: f32,
    /// Whether the label is rendered upper-case.
    pub uppercase: bool,
}

impl TextToken {
    /// A token from size/weight/line-height (letter-spacing 0, no uppercase).
    pub const fn new(size: f32, weight: f32, line_height: f32) -> Self {
        Self { size, weight, line_height, letter_spacing: 0.0, uppercase: false }
    }

    /// The same token with letter spacing.
    pub const fn spaced(mut self, spacing: f32) -> Self {
        self.letter_spacing = spacing;
        self
    }

    /// The same token rendered upper-case.
    pub const fn upper(mut self) -> Self {
        self.uppercase = true;
        self
    }

    /// Apply the token to any GPUI text element (size, weight, line height,
    /// family). Callers add `text_color` from the resolved theme colour.
    pub fn style<E: Styled>(&self, element: E) -> E {
        element
            .font_family(UI_FONT)
            .font_weight(FontWeight(self.weight))
            .text_size(px(self.size))
            .line_height(px(self.line_height))
    }
}

/// The Termius typography scale (`theme.typography.fonts`).
///
/// Names mirror the original tokens: `r`egular / `b`old, size in px, and the
/// colour role (`p` = primary/main colour, `w` = white, `s` = secondary/dim).
pub mod text {
    use super::TextToken;

    /// 22px bold white — screen titles.
    pub const B22W: TextToken = TextToken::new(22.0, 700.0, 28.0);
    /// 18px regular white — large headings.
    pub const R18W: TextToken = TextToken::new(18.0, 450.0, 23.0);
    /// 17px regular dim — section headings.
    pub const R17W: TextToken = TextToken::new(17.0, 450.0, 22.0);
    /// 16px bold primary — dialog titles.
    pub const B16P: TextToken = TextToken::new(16.0, 700.0, 20.0);
    /// 14px regular primary — body text.
    pub const R14P: TextToken = TextToken::new(14.0, 450.0, 18.0);
    /// 14px regular dim — secondary body / descriptions.
    pub const R14S: TextToken = TextToken::new(14.0, 450.0, 18.0);
    /// 14px bold primary — emphasised body.
    pub const B14P: TextToken = TextToken::new(14.0, 700.0, 18.0);
    /// 12px regular primary — captions, list meta.
    pub const R12P: TextToken = TextToken::new(12.0, 450.0, 15.0);
    /// 12px regular white — captions on coloured surfaces.
    pub const R12W: TextToken = TextToken::new(12.0, 450.0, 15.0);
    /// 12px regular dim.
    pub const R12S: TextToken = TextToken::new(12.0, 450.0, 15.0);
    /// 12px bold primary — list item titles.
    pub const B12P: TextToken = TextToken::new(12.0, 700.0, 15.0);
    /// 12px bold white, 23px line, 1px tracking, upper-case — group headers.
    pub const B12WU: TextToken = TextToken::new(12.0, 700.0, 23.0).spaced(1.0).upper();
    /// 11px bold white.
    pub const B11W: TextToken = TextToken::new(11.0, 700.0, 14.0);
    /// 10px regular dim — micro captions.
    pub const R10S: TextToken = TextToken::new(10.0, 450.0, 13.0);
    /// 9px bold, 1.13px tracking, 23px line, upper-case — nav section labels.
    pub const R9U: TextToken = TextToken::new(9.0, 700.0, 23.0).spaced(1.13).upper();
    /// 8px bold white.
    pub const R8W: TextToken = TextToken::new(8.0, 700.0, 10.0);
}


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
    // --- further recovered tokens (additive; analysis/termius-theme.json) ------
    /// `--surface-lowest` (dark `--dark-grey-1` / light `--light-grey-5`).
    pub surface_lowest: Rgba,
    /// `--surface-base` (dark `--dark-grey-2` / light `--light-grey-6`).
    pub surface_base: Rgba,
    /// `--surface-high` (dark `--dark-grey-3` / light `--light-grey-7`).
    pub surface_high: Rgba,
    /// `--surface-highest` (dark `--dark-grey-4` / light `--white`).
    pub surface_highest: Rgba,
    /// `--entity-item-background` (dark `--dark-grey-3` / light `--white`).
    pub entity_item_background: Rgba,
    /// `--list-hover` (dark `--dark-grey-5` / light `--light-grey-5`).
    pub list_hover: Rgba,
    /// `--list-hover-hover` (dark `--dark-grey-4` / light `--light-grey-5`).
    pub list_hover_hover: Rgba,
    /// `--list-select` (dark `--dark-grey-4` / light `--light-grey-4`).
    pub list_select: Rgba,
    /// `--text-primary` (dark `--white` / light `--dark-grey-1`).
    pub text_primary: Rgba,
    /// `--text-secondary` (dark `--dark-grey-7` / light `--light-grey-1`).
    pub text_secondary: Rgba,
    /// `--text-color` — the left-panel section-row label colour
    /// (`00-shell.md` §7, `sFe.root`): dark `--white` / light `--dark-grey-1`.
    pub text_color: Rgba,
    /// `--main-color` — the `p`-role typography colour
    /// (`01-design-system.md` §0.2): dark `--white` / light `--dark-grey-4`.
    pub main_color: Rgba,
    /// `--background-color` — the left panel's own surface
    /// (`00-shell.md` §6, `dFe.root`): dark `--dark-grey-3` /
    /// light `--light-grey-7`.
    pub background_color: Rgba,
    /// `--background-hover-color` — section-row hover (`00-shell.md` §7,
    /// `sFe.root`): dark `--dark-grey-4` / light `--light-grey-6`.
    pub background_hover_color: Rgba,
    /// `--background-selected-color` — section-row selected (`00-shell.md` §7,
    /// `sFe.root`): dark `--dark-grey-5` / light `--light-grey-5`.
    pub background_selected_color: Rgba,
    /// `--dark-blue-solid` (the same `#004878` in both modes).
    pub dark_blue_solid: Rgba,
    /// `--foreground` — the dialog-panel surface (`01-design-system.md` §5.1,
    /// `DialogPanel`): dark `--dark-grey-3` / light `--light-grey-7`.
    ///
    /// Distinct from [`Self::foreground`], which is the `--c-title` *text*
    /// role; this is the `--foreground` *surface* token.
    pub dialog_foreground: Rgba,
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
            surface_lowest: rgb(0x14, 0x17, 0x29),  // --surface-lowest (dark-grey-1)
            surface_base: rgb(0x1d, 0x20, 0x33),    // --surface-base (dark-grey-2)
            surface_high: rgb(0x28, 0x2b, 0x3d),    // --surface-high (dark-grey-3)
            surface_highest: rgb(0x32, 0x36, 0x4a), // --surface-highest (dark-grey-4)
            entity_item_background: rgb(0x28, 0x2b, 0x3d), // --entity-item-background
            list_hover: rgb(0x3e, 0x42, 0x57),      // --list-hover (dark-grey-5)
            list_hover_hover: rgb(0x32, 0x36, 0x4a),// --list-hover-hover (dark-grey-4)
            list_select: rgb(0x32, 0x36, 0x4a),     // --list-select (dark-grey-4)
            text_primary: rgb(0xff, 0xff, 0xff),    // --text-primary (white)
            text_secondary: rgb(0x8d, 0x91, 0xa5),  // --text-secondary (dark-grey-7)
            text_color: rgb(0xff, 0xff, 0xff),      // --text-color (white)
            main_color: rgb(0xff, 0xff, 0xff),      // --main-color (white)
            background_color: rgb(0x28, 0x2b, 0x3d), // --background-color (dark-grey-3)
            background_hover_color: rgb(0x32, 0x36, 0x4a),    // --background-hover-color
            background_selected_color: rgb(0x3e, 0x42, 0x57), // --background-selected-color
            dark_blue_solid: rgb(0x00, 0x48, 0x78), // --dark-blue-solid
            dialog_foreground: rgb(0x28, 0x2b, 0x3d), // --foreground (dark-grey-3)
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
            surface_lowest: rgb(0xe6, 0xeb, 0xed),  // --surface-lowest (light-grey-5)
            surface_base: rgb(0xed, 0xf1, 0xf2),    // --surface-base (light-grey-6)
            surface_high: rgb(0xf7, 0xf9, 0xfa),    // --surface-high (light-grey-7)
            surface_highest: rgb(0xff, 0xff, 0xff), // --surface-highest (white)
            entity_item_background: rgb(0xff, 0xff, 0xff), // --entity-item-background
            list_hover: rgb(0xe6, 0xeb, 0xed),      // --list-hover (light-grey-5)
            list_hover_hover: rgb(0xe6, 0xeb, 0xed),// --list-hover-hover (light-grey-5)
            list_select: rgb(0xd5, 0xdd, 0xe0),     // --list-select (light-grey-4)
            text_primary: rgb(0x14, 0x17, 0x29),    // --text-primary (dark-grey-1)
            text_secondary: rgb(0x79, 0x8c, 0x94),  // --text-secondary (light-grey-1)
            text_color: rgb(0x14, 0x17, 0x29),      // --text-color (dark-grey-1)
            main_color: rgb(0x32, 0x36, 0x4a),      // --main-color (dark-grey-4)
            background_color: rgb(0xf7, 0xf9, 0xfa), // --background-color (light-grey-7)
            background_hover_color: rgb(0xed, 0xf1, 0xf2),    // --background-hover-color
            background_selected_color: rgb(0xe6, 0xeb, 0xed), // --background-selected-color
            dark_blue_solid: rgb(0x00, 0x48, 0x78), // --dark-blue-solid
            dialog_foreground: rgb(0xf7, 0xf9, 0xfa), // --foreground (light-grey-7)
        }
    }

    /// The palette for a mode.
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// The palette matching a window/system appearance.
    ///
    /// `VibrantLight` maps to [`Self::light`] and `VibrantDark` to
    /// [`Self::dark`] — the same mapping gpui's own `Colors::for_appearance`
    /// uses (`gpui-0.2.2/src/colors.rs:36`).
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
        }
    }

    /// The palette following the current window/system appearance.
    ///
    /// Reads [`App::window_appearance`] (gpui 0.2.2 `app.rs:1029`, reachable
    /// straight from `&App` — no `Window` needed). The shell should select the
    /// theme with this and install it, e.g. in `AppShell::render` or an
    /// appearance observer:
    ///
    /// ```ignore
    /// let theme = TermiusTheme::system(cx); // cx: &App
    /// cx.set_global(theme);
    /// ```
    ///
    /// If only a [`Window`](gpui::Window) is on hand, use
    /// `TermiusTheme::for_appearance(window.appearance())` instead.
    pub fn system(cx: &App) -> Self {
        Self::for_appearance(cx.window_appearance())
    }

    /// The top-strip chrome colour — dark in *both* app themes.
    ///
    /// Sources:
    /// * `analysis/recon/00-shell.md` §10 (`yDe` / `CDe.horizontalTabs`): the
    ///   strip's `--background-color` is `--dark-grey-3` (`#282b3d`) in the
    ///   light app theme and `--dark-grey-1` (`#141729`) under
    ///   `.termius-dark-theme`.
    /// * `00-shell.md` §4 (`qDe.colored`): the spacer under the strip paints
    ///   `themeColors.backgroundColor` — the terminal chrome colour. Its
    ///   default split-view scheme is always `"Termius Dark"`
    ///   (`splitViewTheme = colorSchemes["termius dark"]`, `backgroundColor`
    ///   `#141729` = `--dark-grey-1`), so the chrome stays dark in both modes.
    pub fn chrome_background(&self) -> Rgba {
        match self.mode {
            ThemeMode::Dark => rgb(0x14, 0x17, 0x29),  // --dark-grey-1
            ThemeMode::Light => rgb(0x28, 0x2b, 0x3d), // --dark-grey-3
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
    fn for_appearance_maps_to_palette() {
        assert_eq!(
            TermiusTheme::for_appearance(WindowAppearance::Light).mode,
            ThemeMode::Light
        );
        assert_eq!(
            TermiusTheme::for_appearance(WindowAppearance::VibrantLight).mode,
            ThemeMode::Light
        );
        assert_eq!(
            TermiusTheme::for_appearance(WindowAppearance::Dark).mode,
            ThemeMode::Dark
        );
        assert_eq!(
            TermiusTheme::for_appearance(WindowAppearance::VibrantDark).mode,
            ThemeMode::Dark
        );
    }

    #[test]
    fn new_tokens_are_complete_and_mode_specific() {
        let dark = TermiusTheme::dark();
        let light = TermiusTheme::light();
        // Every newly added surface/entity/list/text token differs between modes.
        let pairs = [
            (dark.surface_lowest, light.surface_lowest),
            (dark.surface_base, light.surface_base),
            (dark.surface_high, light.surface_high),
            (dark.surface_highest, light.surface_highest),
            (dark.entity_item_background, light.entity_item_background),
            (dark.list_hover, light.list_hover),
            (dark.list_hover_hover, light.list_hover_hover),
            (dark.list_select, light.list_select),
            (dark.text_primary, light.text_primary),
            (dark.text_secondary, light.text_secondary),
            (dark.text_color, light.text_color),
            (dark.main_color, light.main_color),
            (dark.background_color, light.background_color),
            (dark.background_hover_color, light.background_hover_color),
            (dark.background_selected_color, light.background_selected_color),
            (dark.dialog_foreground, light.dialog_foreground),
        ];
        for (d, l) in pairs {
            assert_ne!(d, l);
        }
        // Spot-check exact token values (analysis/termius-theme.json).
        assert_eq!(dark.surface_high, rgb(0x28, 0x2b, 0x3d));
        assert_eq!(light.surface_high, rgb(0xf7, 0xf9, 0xfa));
        assert_eq!(dark.text_primary, rgb(0xff, 0xff, 0xff));
        assert_eq!(light.text_primary, rgb(0x14, 0x17, 0x29));
        assert_eq!(dark.list_hover, rgb(0x3e, 0x42, 0x57));
        assert_eq!(light.list_select, rgb(0xd5, 0xdd, 0xe0));
        assert_eq!(light.text_secondary, rgb(0x79, 0x8c, 0x94));
        assert_eq!(dark.main_color, rgb(0xff, 0xff, 0xff));
        assert_eq!(light.main_color, rgb(0x32, 0x36, 0x4a));
        // `--dark-blue-solid` is the one mode-invariant token.
        assert_eq!(dark.dark_blue_solid, light.dark_blue_solid);
        assert_eq!(dark.dark_blue_solid, rgb(0x00, 0x48, 0x78));
    }

    #[test]
    fn chrome_is_dark_in_both_modes() {
        // Dark: --dark-grey-1; light: --dark-grey-3 — both dark.
        let dark = TermiusTheme::dark().chrome_background();
        let light = TermiusTheme::light().chrome_background();
        assert_eq!(dark, rgb(0x14, 0x17, 0x29));
        assert_eq!(light, rgb(0x28, 0x2b, 0x3d));
        // Every channel stays below mid-grey in both modes.
        for c in [dark, light] {
            assert!(c.r < 0.5 && c.g < 0.5 && c.b < 0.5);
        }
    }

    #[test]
    fn shell_geometry_constants_match_the_recon() {
        assert_eq!(HORIZONTAL_TABS_HEIGHT, 51.0);
        assert_eq!(LEFT_PANEL_WIDTH, 185.0);
        assert_eq!(LEFT_PANEL_COMPACT_WIDTH, 68.0);
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
