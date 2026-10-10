//! settings_screen — the Settings section as a visual port of the original
//! Termius desktop Settings window (the one the in-app gear opens).
//!
//! [`settings_screen`] builds a small self-contained view (GPUI renders the
//! returned entity as a child, exactly like `HostList`), so the active sub-tab
//! lives with the screen instead of on [`TermiusState`]. The tab labels come
//! from [`settings_tabs`]; every row is a [`crate::primitives`] component, so a
//! theme switch repaints this screen for free.
//!
//! ```text
//! SettingsScreen                                        (root: --foreground)
//! ├── header            — "Settings" bar (h 80, border-bottom)
//! └── body row
//!     ├── tab column    — 220px; one 36px tab row per settings_tabs()
//!     │                   (Terminal · SFTP · Logs · Advanced · Keyboard · Team)
//!     └── tab content   — scrolls; bg = --tab-content-color, one SettingsSection
//!                         card (20px pad, --card-a, 10px radius, max 700) per
//!                         group of controls
//! ```
//!
//! The layout mirrors the recovered sources: `setting-process-7c44cf6f.js`
//! (the window shell: `settingsHeader`, 220px `sidebar`, `settingsTabContent`)
//! and the per-tab pages (`index-13781084.js` Terminal, `SftpSettings-*`,
//! `LogsSettings-*`, `DeveloperTools-*`, `index-7370652f.js` Keyboard).
//!
//! # What actually mutates state
//!
//! * **Terminal** — `theme_mode`, `font_size`, `cursor_blink` and
//!   `auto_reconnect` write straight into [`TermiusState::settings`];
//!   `theme_mode` additionally repaints the [`TermiusTheme`] global.
//! * **SFTP / Advanced / Logs** — the controls that have no field on
//!   [`SettingsState`] yet flip view-local [`LocalToggles`] bools / the
//!   [`LogLevel`] (PORT-TODO: hoist them into `SettingsState` once the model
//!   grows). Every `Switch` still flips a real bool; none of them is a dead
//!   pill.
//!
//! Numeric rows (`font_family`, `scrollback_lines`, paths) are display-only:
//! [`crate::primitives::InputField`] has no editing yet (see the PORT-TODO on
//! that primitive). Dropdown rows have no popup primitive in gpui 0.2.2, so a
//! click cycles the options in place — the value is still live.

use gpui::{
    div, px, AnyElement, App, AppContext as _, BorrowAppContext as _, ClickEvent, Context, Div,
    Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::app_state::{SettingsState, TermiusState};
use crate::assets::icon;
use crate::navigation::settings_tabs;
use crate::primitives::{
    Button, EmptyState, InputField, ListItem, SettingsSection, SettingsText, SettingsTitle, Switch,
};
use crate::theme::{text, theme_of, TermiusTheme, ThemeMode};

// ---------------------------------------------------------------------------
// Geometry + options (from the original CSS)
// ---------------------------------------------------------------------------

/// Settings sidebar width (`setting-process-7c44cf6f.js`: `width: 220px`).
const SIDEBAR_WIDTH: f32 = 220.0;
/// Settings header height (`--header-height: 80px`).
const HEADER_HEIGHT: f32 = 80.0;
/// One sidebar tab row (`height: 36px`).
const TAB_HEIGHT: f32 = 36.0;
/// Smallest font size the stepper allows (points).
const MIN_FONT_SIZE: i32 = 6;
/// Largest font size the stepper allows (points).
const MAX_FONT_SIZE: i32 = 48;
/// Terminal emulation types Termius offers (`index-13781084.js` `Bt`).
const EMULATION_TYPES: [&str; 4] = ["xterm-256color", "xterm", "linux", "vt100"];

// ---------------------------------------------------------------------------
// Tab dispatch
// ---------------------------------------------------------------------------

/// Which pane a [`settings_tabs`] label opens.
///
/// Dispatching on a name (instead of an index) keeps the screen honest if a
/// tab is renamed: an unknown label renders a note rather than panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TabKind {
    Terminal,
    Sftp,
    Logs,
    Advanced,
    Keyboard,
    Team,
    Unknown,
}

/// Map a settings tab label onto its [`TabKind`].
fn tab_kind(label: &str) -> TabKind {
    match label {
        "Terminal" => TabKind::Terminal,
        "SFTP" => TabKind::Sftp,
        "Logs" => TabKind::Logs,
        "Advanced" => TabKind::Advanced,
        "Keyboard" => TabKind::Keyboard,
        "Team" => TabKind::Team,
        _ => TabKind::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Flags + view-local toggles
// ---------------------------------------------------------------------------

/// A bool on [`TermiusState::settings`] a `Switch` can flip.
///
/// A (tiny) enum instead of a `fn(&mut SettingsState) -> &mut bool` pointer so
/// the click listener stays a plain `Fn` with no higher-ranked closure
/// coercion to guess at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingFlag {
    CursorBlink,
    AutoReconnect,
}

/// Flip `flag` in `settings` (the caller notifies the state entity).
fn flip_setting_flag(settings: &mut SettingsState, flag: SettingFlag) {
    let slot = match flag {
        SettingFlag::CursorBlink => &mut settings.cursor_blink,
        SettingFlag::AutoReconnect => &mut settings.auto_reconnect,
    };
    *slot = !*slot;
}

/// A bool that only lives on this view for now (see [`LocalToggles`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalFlag {
    Autocomplete,
    ImportShellHistory,
    CopyPaste,
    BellSound,
    BrightenBold,
    KeywordHighlighting,
    PostQuantum,
    SftpPreserveMtime,
    SftpFollowSymlinks,
    SftpShowHidden,
    UsageData,
    AutoUpdate,
    ConfirmClose,
}

/// Flip `flag` in `local` (the caller notifies the view).
fn flip_local_flag(local: &mut LocalToggles, flag: LocalFlag) {
    let slot = match flag {
        LocalFlag::Autocomplete => &mut local.autocomplete,
        LocalFlag::ImportShellHistory => &mut local.import_shell_history,
        LocalFlag::CopyPaste => &mut local.copy_paste,
        LocalFlag::BellSound => &mut local.bell_sound,
        LocalFlag::BrightenBold => &mut local.brighten_bold,
        LocalFlag::KeywordHighlighting => &mut local.keyword_highlighting,
        LocalFlag::PostQuantum => &mut local.post_quantum,
        LocalFlag::SftpPreserveMtime => &mut local.sftp_preserve_mtime,
        LocalFlag::SftpFollowSymlinks => &mut local.sftp_follow_symlinks,
        LocalFlag::SftpShowHidden => &mut local.sftp_show_hidden,
        LocalFlag::UsageData => &mut local.usage_data,
        LocalFlag::AutoUpdate => &mut local.auto_update,
        LocalFlag::ConfirmClose => &mut local.confirm_close,
    };
    *slot = !*slot;
}

/// Log verbosity the Logs tab picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    /// Every level, quietest first (the chooser's order).
    pub const ALL: [LogLevel; 4] = [Self::Debug, Self::Info, Self::Warning, Self::Error];

    /// The label Termius shows in the log-level picker.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

/// Tabs whose switches have no field on [`SettingsState`] yet.
///
/// Kept view-local so every `Switch` still flips a real bool instead of
/// painting a dead pill (PORT-TODO: hoist into `SettingsState` once the model
/// grows, then delete this).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalToggles {
    // Terminal (index-13781084.js).
    /// Terminal settings: inline autocomplete (the "Beta" row).
    autocomplete: bool,
    /// Terminal settings: import the shell history on connect.
    import_shell_history: bool,
    /// Terminal settings: select-to-copy + right-click-to-paste.
    copy_paste: bool,
    /// Terminal settings: beep on the terminal bell.
    bell_sound: bool,
    /// Terminal settings: bright ANSI colours for bold text.
    brighten_bold: bool,
    /// Terminal settings: highlight keywords in output.
    keyword_highlighting: bool,
    /// Terminal settings: post-quantum key exchange.
    post_quantum: bool,
    /// Terminal settings: selected emulation type (index into [`EMULATION_TYPES`]).
    emulation: usize,
    // SFTP.
    /// SFTP: keep the remote mtime on downloads/uploads.
    sftp_preserve_mtime: bool,
    /// SFTP: follow symbolic links while transferring.
    sftp_follow_symlinks: bool,
    /// SFTP: list dotfiles in the browser.
    sftp_show_hidden: bool,
    // Logs.
    /// Log level shown by the Logs tab.
    log_level: LogLevel,
    /// Whether the "open log folder" note is visible.
    log_folder_note: bool,
    // Advanced.
    /// Advanced: send anonymous usage statistics.
    usage_data: bool,
    /// Advanced: download updates in the background.
    auto_update: bool,
    /// Advanced: confirm before closing a session tab.
    confirm_close: bool,
}

impl Default for LocalToggles {
    fn default() -> Self {
        Self {
            autocomplete: true,
            import_shell_history: false,
            copy_paste: true,
            bell_sound: true,
            brighten_bold: false,
            keyword_highlighting: true,
            post_quantum: false,
            emulation: 0,
            sftp_preserve_mtime: true,
            sftp_follow_symlinks: false,
            sftp_show_hidden: false,
            log_level: LogLevel::Info,
            log_folder_note: false,
            usage_data: false,
            auto_update: true,
            confirm_close: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Element helpers
// ---------------------------------------------------------------------------

/// White used for text on an accent fill (the crate's `on_fill()` precedent).
fn on_accent() -> gpui::Rgba {
    gpui::rgb(0xff_ff_ff)
}

/// A [`Switch`] row with its click listener attached (the caller flips the
/// backing bool). Faithful to `SettingsSwitch-200210fd.js`.
fn toggle(
    theme: TermiusTheme,
    label: &'static str,
    on: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    Switch::new(label, on).element(theme).on_click(listener)
}

/// One pill of a small inline chooser (log level).
fn choice(
    id: SharedString,
    label: SharedString,
    active: bool,
    theme: TermiusTheme,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (fill, border) = if active {
        (theme.accent, theme.accent)
    } else {
        (theme.tab_background, theme.border)
    };
    let text_color = if active { on_accent() } else { theme.muted };

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.))
        .px(px(10.))
        .rounded(px(4.))
        .border_1()
        .border_color(border)
        .bg(fill)
        .text_sm()
        .text_color(text_color)
        .child(label)
        .on_click(listener)
}

/// A 12/500 row label (`--text-primary`), the left half of a settings row
/// (`index-13781084.js` `Ot` / `bt`).
fn row_label(theme: TermiusTheme, label: impl Into<SharedString>) -> Div {
    text::R12P
        .style(div())
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.title)
        .child(label.into())
}

/// The uppercase "Beta" chip the Autocomplete row carries.
fn beta_badge(theme: TermiusTheme) -> Div {
    text::R10S
        .style(div())
        .font_weight(FontWeight::BOLD)
        .ml(px(10.))
        .px(px(5.))
        .py(px(2.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme.text_common)
        .text_color(theme.text_common)
        .child(SharedString::from("BETA"))
}

/// A label-left / value-right row whose value acts as a dropdown.
///
/// The original opens a menu (`FiltersButton`); gpui 0.2.2 has no popup
/// primitive, so the row cycles its options on click while still showing the
/// live value.
fn dropdown_row(
    theme: TermiusTheme,
    id: &'static str,
    label: Div,
    value: impl Into<SharedString>,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let value: SharedString = value.into();
    div()
        .id(SharedString::from(id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .w_full()
        .mt(px(15.))
        .cursor_pointer()
        .child(label)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .child(
                    text::R12P
                        .style(div())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.title)
                        .child(value),
                )
                .child(icon("allSettingsChevron.svg").w(px(15.)).h(px(6.)).text_color(theme.muted)),
        )
        .on_click(listener)
}

/// A 36px square ghost icon button (the font-size stepper's `+` / `−`).
fn icon_button(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    icon_w: f32,
    icon_h: f32,
    disabled: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let color = if disabled { theme.text_common } else { theme.title };
    let mut button = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(36.))
        .h(px(36.))
        .rounded(px(theme.corner_radius_small))
        .text_color(color)
        .child(icon(icon_name).w(px(icon_w)).h(px(icon_h)).text_color(color));
    if disabled {
        button
    } else {
        button
            .cursor_pointer()
            .hover(move |style| style.bg(theme.hover))
            .on_click(listener)
    }
}

/// The centred font-size value between the stepper buttons (`width: 51px`).
fn size_box(theme: TermiusTheme, size: u16) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(51.))
        .h(px(36.))
        .mx(px(5.))
        .child(
            text::R14P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from(size.to_string())),
        )
}

/// One `SettingsSection` card wrapping `rows`.
///
/// The card chrome (20px pad, `--card-a`, 10px radius, max-width 700, centred
/// with a 30px top gap) comes from the ported [`SettingsSection`]; the rows go
/// in as a single column so their own 15/20px margins set the rhythm (the
/// original section has no internal gap either).
fn settings_card(theme: TermiusTheme, title: &'static str, rows: Vec<AnyElement>) -> Div {
    let mut body = div().flex().flex_col();
    for row in rows {
        body = body.child(row);
    }
    SettingsSection::new(title).child(body).element(theme)
}

/// A sidebar tab row (`setting-process-7c44cf6f.js` `pt.tab`): 36px tall,
/// `8px 10px` margins, 10px radius, 14px label, with hairline rules between
/// rows and the selected row filled.
fn tab_button(
    theme: TermiusTheme,
    index: usize,
    name: &'static str,
    active: bool,
    last: bool,
) -> Stateful<Div> {
    let hover_fill = if active { theme.card_c } else { theme.hover };
    let mut tab = div()
        .id(SharedString::from(format!("settings-tab-{name}")))
        .flex()
        .items_center()
        .h(px(TAB_HEIGHT))
        .px(px(15.))
        .mx(px(10.))
        .my(px(8.))
        .rounded(px(theme.corner_radius_medium))
        .cursor_pointer()
        .text_color(theme.title)
        .child(
            text::R14P
                .style(div())
                .text_color(theme.title)
                .whitespace_nowrap()
                .child(SharedString::from(name)),
        );
    if index > 0 {
        tab = tab.border_t_1().border_color(theme.border);
    }
    if last {
        tab = tab.border_b_1().border_color(theme.border);
    }
    if active {
        tab = tab.bg(theme.card_c);
    }
    tab.hover(move |style| style.bg(hover_fill))
}

/// Step the font size by `delta`, clamped to [`MIN_FONT_SIZE`] /
/// [`MAX_FONT_SIZE`] (never widens past either end).
fn clamp_font_size(current: u16, delta: i32) -> u16 {
    let next = i32::from(current) + delta;
    next.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE) as u16
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Settings screen: sidebar tab column + the active tab's controls.
///
/// Built by [`settings_screen`] and rendered by the shell as a child element
/// (like `HostList`); it owns only the active tab index and the toggles that
/// have no [`SettingsState`] field yet.
pub struct SettingsScreen {
    state: Entity<TermiusState>,
    /// Index into [`settings_tabs`].
    active_tab: usize,
    /// Toggle backing for the tabs not modelled on `SettingsState`.
    local: LocalToggles,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Settings screen for `state`.
///
/// Returns the view entity as an element: the shell inserts it wherever the
/// Settings section is routed. Store the result once (the same way
/// `AppShell` holds `HostList`) — calling this every render would mint a new
/// entity per frame.
pub fn settings_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<SettingsScreen> {
    cx.new(|cx| SettingsScreen::new(state, cx))
}

impl SettingsScreen {
    /// Create the screen and subscribe it to state + theme changes.
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let observe_theme = cx.observe_global::<TermiusTheme>(|_, cx| cx.notify());
        Self {
            state,
            active_tab: 0,
            local: LocalToggles::default(),
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// The tab actually shown (clamped, so a stale index can't go blank).
    fn visible_tab(&self) -> usize {
        self.active_tab.min(settings_tabs().len().saturating_sub(1))
    }

    // ----- actions --------------------------------------------------------

    fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= settings_tabs().len() {
            return;
        }
        self.active_tab = index;
        cx.notify();
    }

    /// Flip a persisted setting (`TermiusState` notifies its observers).
    fn flip_setting(&mut self, flag: SettingFlag, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            flip_setting_flag(&mut state.settings, flag);
            cx.notify();
        });
    }

    /// Flip a view-local toggle (SFTP / Advanced).
    fn flip_local(&mut self, flag: LocalFlag, cx: &mut Context<Self>) {
        flip_local_flag(&mut self.local, flag);
        cx.notify();
    }

    /// Pick the app palette: store the mode, then repaint the theme global so
    /// every screen (and the terminal) follows immediately.
    fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.settings.theme_mode = mode;
            cx.notify();
        });
        cx.update_global::<TermiusTheme, _>(|theme, cx| {
            *theme = TermiusTheme::for_mode(mode);
            cx.notify();
        });
    }

    /// The theme dropdown: flip Dark ⇄ Light.
    fn cycle_theme_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.state.read(cx).settings.theme_mode {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        self.set_theme_mode(next, cx);
    }

    /// The emulation dropdown: step to the next [`EMULATION_TYPES`] entry.
    fn cycle_emulation(&mut self, cx: &mut Context<Self>) {
        let count = EMULATION_TYPES.len();
        self.local.emulation = (self.local.emulation + 1) % count;
        cx.notify();
    }

    fn set_log_level(&mut self, level: LogLevel, cx: &mut Context<Self>) {
        self.local.log_level = level;
        cx.notify();
    }

    fn adjust_font_size(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.settings.font_size = clamp_font_size(state.settings.font_size, delta);
            cx.notify();
        });
    }

    /// "Open log folder": reveal the note until a shell bridge can do it.
    fn reveal_log_folder(&mut self, cx: &mut Context<Self>) {
        self.local.log_folder_note = true;
        cx.notify();
    }

    /// The "Text Size" row: label left, `−` / value / `+` right.
    fn text_size_row(
        &self,
        theme: TermiusTheme,
        settings: &SettingsState,
        cx: &mut Context<Self>,
    ) -> Div {
        let size = i32::from(settings.font_size);
        let can_decrease = size > MIN_FONT_SIZE;
        let can_increase = size < MAX_FONT_SIZE;

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .mt(px(15.))
            .child(row_label(theme, "Text Size"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(icon_button(
                        theme,
                        "font-size-decrease",
                        "minus__b94c20.svg",
                        10.0,
                        2.0,
                        !can_decrease,
                        cx.listener(|this, _event, _window, cx| this.adjust_font_size(-1, cx)),
                    ))
                    .child(size_box(theme, settings.font_size))
                    .child(icon_button(
                        theme,
                        "font-size-increase",
                        "plusFonts.svg",
                        10.0,
                        10.0,
                        !can_increase,
                        cx.listener(|this, _event, _window, cx| this.adjust_font_size(1, cx)),
                    )),
            )
    }

    // ----- tabs -----------------------------------------------------------

    /// Terminal: the inline `Autocomplete` row, the session switches, the font
    /// card, the emulation picker, highlighting, scrollback + post-quantum.
    fn terminal_tab(
        &self,
        theme: TermiusTheme,
        settings: &SettingsState,
        body: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let local = &self.local;
        let autocomplete_label = div()
            .flex()
            .flex_row()
            .items_center()
            .child(row_label(theme, "Autocomplete"))
            .child(beta_badge(theme));
        let emulation = EMULATION_TYPES[local.emulation.min(EMULATION_TYPES.len() - 1)];
        let autocomplete_value = if local.autocomplete { "Enabled" } else { "Disabled" };
        let theme_value = match settings.theme_mode {
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        };

        body.child(settings_card(
            theme,
            "Terminal settings",
            vec![
                dropdown_row(
                    theme,
                    "settings-autocomplete",
                    autocomplete_label,
                    autocomplete_value,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::Autocomplete, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Autoreconnect",
                    settings.auto_reconnect,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_setting(SettingFlag::AutoReconnect, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Cursor blink",
                    settings.cursor_blink,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_setting(SettingFlag::CursorBlink, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Import shell history",
                    local.import_shell_history,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::ImportShellHistory, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Select text to copy & right click to paste",
                    local.copy_paste,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::CopyPaste, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Bell sound",
                    local.bell_sound,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::BellSound, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Use bright colours for bold text",
                    local.brighten_bold,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::BrightenBold, cx);
                    }),
                )
                .into_any_element(),
                dropdown_row(
                    theme,
                    "settings-emulation",
                    row_label(theme, "Terminal emulation type"),
                    emulation,
                    cx.listener(|this, _event, _window, cx| this.cycle_emulation(cx)),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Font",
            vec![
                InputField::new("Font family", settings.font_family.clone())
                    .element(theme)
                    .into_any_element(),
                self.text_size_row(theme, settings, cx).into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Theme",
            vec![
                dropdown_row(
                    theme,
                    "settings-app-theme",
                    row_label(theme, "App theme"),
                    theme_value,
                    cx.listener(|this, _event, _window, cx| this.cycle_theme_mode(cx)),
                )
                .into_any_element(),
                SettingsText::new("Applies to the app chrome and the terminal palette.")
                    .element(theme)
                    .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Highlighting",
            vec![
                toggle(
                    theme,
                    "Keyword highlighting",
                    local.keyword_highlighting,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::KeywordHighlighting, cx);
                    }),
                )
                .into_any_element(),
                SettingsText::new("Highlight git, log and language output in the terminal.")
                    .element(theme)
                    .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Scrollback",
            vec![
                SettingsText::new("Limit the number of terminal rows. Set to 0 for the maximum.")
                    .element(theme)
                    .into_any_element(),
                InputField::new("Number of rows", settings.scrollback_lines.to_string())
                    .element(theme)
                    .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Connection",
            vec![Switch::new("Post-Quantum Key Exchange", local.post_quantum)
                .description("Turn off if you're experiencing issues with legacy devices")
                .element(theme)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.flip_local(LocalFlag::PostQuantum, cx);
                }))
                .into_any_element()],
        ))
    }

    /// SFTP: transfer toggles + path fields + the file-type-association card.
    fn sftp_tab(&self, theme: TermiusTheme, body: Div, cx: &mut Context<Self>) -> Div {
        let local = &self.local;
        body.child(settings_card(
            theme,
            "Transfers",
            vec![
                toggle(
                    theme,
                    "Preserve remote timestamps",
                    local.sftp_preserve_mtime,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::SftpPreserveMtime, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Follow symbolic links",
                    local.sftp_follow_symlinks,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::SftpFollowSymlinks, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Show hidden files",
                    local.sftp_show_hidden,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::SftpShowHidden, cx);
                    }),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Paths",
            vec![
                InputField::new("Download folder", "~/Downloads")
                    .element(theme)
                    .into_any_element(),
                InputField::new("Upload folder", "~/").element(theme).into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "File type associations",
            vec![
                SettingsTitle::new("No file type associations yet")
                    .element(theme)
                    .into_any_element(),
                SettingsText::new(
                    "Apps you choose using \"Open with...\" to open files via SFTP will appear \
                     here.",
                )
                .element(theme)
                .into_any_element(),
            ],
        ))
    }

    /// Logs: level chooser + the "open log folder" action.
    fn logs_tab(&self, theme: TermiusTheme, body: Div, cx: &mut Context<Self>) -> Div {
        let level = self.local.log_level;
        let mut chooser = div().flex().flex_row().gap(px(6.));
        for candidate in LogLevel::ALL {
            let active = candidate == level;
            chooser = chooser.child(choice(
                SharedString::from(format!("log-level-{}", candidate.label())),
                SharedString::from(candidate.label()),
                active,
                theme,
                cx.listener(move |this, _event, _window, cx| {
                    this.set_log_level(candidate, cx);
                }),
            ));
        }

        let mut files = SettingsSection::new("Log files").child(
            Button::new("Open log folder").secondary().on_click(
                theme,
                cx.listener(|this, _event, _window, cx| this.reveal_log_folder(cx)),
            ),
        );
        if self.local.log_folder_note {
            files = files.child(
                SettingsText::new(
                    "The log folder opens through the desktop shell bridge (PORT-TODO); logs \
                     are mirrored to the console meanwhile.",
                )
                .element(theme),
            );
        }

        body.child(settings_card(
            theme,
            "Level",
            vec![
                chooser.into_any_element(),
                SettingsText::new("Info is the daily default; Debug is very noisy.")
                    .element(theme)
                    .into_any_element(),
            ],
        ))
        .child(files.element(theme))
    }

    /// Advanced: application toggles + the data folder.
    fn advanced_tab(&self, theme: TermiusTheme, body: Div, cx: &mut Context<Self>) -> Div {
        let local = &self.local;
        body.child(settings_card(
            theme,
            "Application",
            vec![
                toggle(
                    theme,
                    "Send anonymous usage data",
                    local.usage_data,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::UsageData, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Download updates automatically",
                    local.auto_update,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::AutoUpdate, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Confirm before closing a session",
                    local.confirm_close,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::ConfirmClose, cx);
                    }),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Storage",
            vec![
                InputField::new("Data folder", "~/Library/Application Support/Termius")
                    .element(theme)
                    .into_any_element(),
                SettingsText::new("The data folder is read-only for now (PORT-TODO: picker).")
                    .element(theme)
                    .into_any_element(),
            ],
        ))
    }

    /// Keyboard: the desktop shortcut list (read-only rows until the keymap
    /// editor lands).
    fn keyboard_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        const SHORTCUTS: [(&str, &str); 6] = [
            ("Toggle sidebar", "⌘B"),
            ("Toggle SFTP panel", "⌘⇧F"),
            ("Close tab", "⌘W"),
            ("Copy selection", "⌘⇧C"),
            ("Paste from clipboard", "⌘⇧V"),
            ("Clear terminal", "⌘K"),
        ];

        let mut shortcuts = SettingsSection::new("Shortcuts");
        for (action, keys) in SHORTCUTS {
            shortcuts = shortcuts.child(ListItem::new(action, keys).element(theme));
        }

        body.child(
            SettingsText::new(
                "Desktop defaults; rebinding arrives with the keymap editor (PORT-TODO).",
            )
            .element(theme),
        )
        .child(shortcuts.element(theme))
    }

    /// Team: placeholder pointing at the Team section.
    fn team_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        body.child(
            div().h(px(240.)).child(
                EmptyState::new(
                    "Team & Vaults",
                    "Shared hosts, keys and vaults live in the Team section — sign in to sync \
                     them.",
                )
                .element(theme),
            ),
        )
        .child(
            SettingsText::new(
                "Team settings open in the Team sidebar section once an account is signed in.",
            )
            .element(theme),
        )
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let settings = self.state.read(cx).settings.clone();

        let tabs = settings_tabs();
        let index = self.visible_tab();
        let label = tabs.get(index).copied().unwrap_or("Terminal");

        // Header: the original `settingsHeader` (h 80, bottom rule).
        let header = div()
            .flex()
            .items_center()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .px(px(20.))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.card_a)
            .child(SettingsTitle::new("Settings").element(theme));

        // Sidebar: the original 220px tab column (text-only rows with
        // hairline rules, selected row filled).
        let mut nav = div().flex().flex_col().w_full();
        for (position, name) in tabs.iter().enumerate() {
            let name = *name;
            let active = position == index;
            let last = position + 1 == tabs.len();
            nav = nav.child(
                tab_button(theme, position, name, active, last).on_click(cx.listener(
                    move |this, _event, _window, cx| this.select_tab(position, cx),
                )),
            );
        }
        let sidebar = div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .min_w(px(SIDEBAR_WIDTH))
            .flex_none()
            .bg(theme.card_a)
            .border_r_1()
            .border_color(theme.border)
            .child(nav);

        // Content: the original `SettingsTabContent` (own bg, scrolls).
        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .pb(px(60.))
            .bg(theme.background);
        content = match tab_kind(label) {
            TabKind::Terminal => self.terminal_tab(theme, &settings, content, cx),
            TabKind::Sftp => self.sftp_tab(theme, content, cx),
            TabKind::Logs => self.logs_tab(theme, content, cx),
            TabKind::Advanced => self.advanced_tab(theme, content, cx),
            TabKind::Keyboard => self.keyboard_tab(theme, content),
            TabKind::Team => self.team_tab(theme, content),
            TabKind::Unknown => content.child(
                SettingsText::new(format!("The \"{label}\" tab has no controls yet."))
                    .element(theme),
            ),
        };
        let content = content.id("settings-content").overflow_y_scroll();

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.card_a)
            .text_color(theme.title)
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .child(sidebar)
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_settings_tab_maps_to_a_known_pane() {
        let mut seen = std::collections::HashSet::new();
        for label in settings_tabs() {
            let kind = tab_kind(label);
            assert_ne!(kind, TabKind::Unknown, "unmapped settings tab `{label}`");
            assert!(seen.insert(kind), "duplicate pane for tab `{label}`");
        }
        // Six tabs, six distinct panes, and the array matches Termius.
        assert_eq!(seen.len(), settings_tabs().len());
        assert_eq!(
            settings_tabs(),
            ["Terminal", "SFTP", "Logs", "Advanced", "Keyboard", "Team"].as_slice()
        );
    }

    #[test]
    fn setting_flags_flip_the_stored_bools() {
        let mut settings = SettingsState::default();
        assert!(settings.cursor_blink);
        assert!(settings.auto_reconnect);

        flip_setting_flag(&mut settings, SettingFlag::CursorBlink);
        assert!(!settings.cursor_blink);
        flip_setting_flag(&mut settings, SettingFlag::CursorBlink);
        assert!(settings.cursor_blink);

        flip_setting_flag(&mut settings, SettingFlag::AutoReconnect);
        assert!(!settings.auto_reconnect);
        // The untouched field stays untouched.
        assert_eq!(settings.theme_mode, ThemeMode::Dark);
        assert_eq!(settings.font_size, 13);
    }

    #[test]
    fn local_flags_flip_the_view_toggles() {
        let mut local = LocalToggles::default();
        assert!(local.sftp_preserve_mtime);
        assert!(!local.sftp_show_hidden);

        flip_local_flag(&mut local, LocalFlag::SftpPreserveMtime);
        flip_local_flag(&mut local, LocalFlag::SftpShowHidden);
        assert!(!local.sftp_preserve_mtime);
        assert!(local.sftp_show_hidden);

        flip_local_flag(&mut local, LocalFlag::UsageData);
        assert!(local.usage_data);
        flip_local_flag(&mut local, LocalFlag::ConfirmClose);
        assert!(!local.confirm_close);
        // Log level is not a bool flag; it stays on the default.
        assert_eq!(local.log_level, LogLevel::Info);
    }

    #[test]
    fn terminal_flags_flip_the_view_toggles() {
        let mut local = LocalToggles::default();
        assert!(local.autocomplete);
        assert!(!local.post_quantum);
        // The emulation picker starts at the first type.
        assert_eq!(EMULATION_TYPES[local.emulation], "xterm-256color");

        flip_local_flag(&mut local, LocalFlag::Autocomplete);
        flip_local_flag(&mut local, LocalFlag::PostQuantum);
        assert!(!local.autocomplete);
        assert!(local.post_quantum);
    }

    #[test]
    fn font_size_stepper_clamps_at_both_ends() {
        assert_eq!(clamp_font_size(13, -1), 12);
        assert_eq!(clamp_font_size(13, 1), 14);
        assert_eq!(clamp_font_size(13, 0), 13);
        assert_eq!(clamp_font_size(6, -10), MIN_FONT_SIZE as u16);
        assert_eq!(clamp_font_size(47, 10), MAX_FONT_SIZE as u16);
        // Never leaves the allowed window, whatever the step.
        for delta in -60..=60 {
            let stepped = clamp_font_size(13, delta);
            assert!((MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&i32::from(stepped)));
        }
    }

    #[test]
    fn log_levels_are_ordered_and_unique() {
        let labels: Vec<&str> = LogLevel::ALL.iter().map(|level| level.label()).collect();
        assert_eq!(labels, ["Debug", "Info", "Warning", "Error"]);
        let mut seen = std::collections::HashSet::new();
        for level in LogLevel::ALL {
            assert!(
                seen.insert(level.label()),
                "duplicate level `{}`",
                level.label()
            );
        }
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    #[test]
    fn controls_build_against_the_theme_without_a_window() {
        // Constructing (not painting) the interactive helpers must not need a
        // window: this is what each tab does inside `render`.
        let theme = TermiusTheme::dark();
        let _ = toggle(theme, "Cursor blink", true, |_, _, _| {});
        let _ = toggle(theme, "Auto-reconnect", false, |_, _, _| {});
        let _ = choice(
            SharedString::from("theme-mode-dark"),
            SharedString::from("Dark"),
            true,
            theme,
            |_, _, _| {},
        );
        let _ = choice(
            SharedString::from("theme-mode-light"),
            SharedString::from("Light"),
            false,
            theme,
            |_, _, _| {},
        );
        let _ = dropdown_row(
            theme,
            "settings-autocomplete",
            row_label(theme, "Autocomplete"),
            "Enabled",
            |_, _, _| {},
        );
        let _ = icon_button(theme, "font-size-increase", "plusFonts.svg", 10.0, 10.0, false, |_, _, _| {});
        let _ = icon_button(theme, "font-size-decrease", "minus__b94c20.svg", 10.0, 2.0, true, |_, _, _| {});
        let _ = size_box(theme, 13);
        let _ = beta_badge(theme);
        let _ = tab_button(theme, 0, "Terminal", true, false);
    }
}
