//! settings_screen — the Settings section as a visual port of the original
//! Termius desktop Settings window (the one the in-app gear opens).
//!
//! [`settings_screen`] builds a small self-contained view (GPUI renders the
//! returned entity as a child, exactly like `HostList`), so the active sub-tab
//! lives with the screen instead of on [`TermiusState`]. Every row is a
//! [`crate::primitives`] component, so a theme switch repaints this screen for
//! free.
//!
//! ```text
//! SettingsScreen                                        (root: --foreground)
//! ├── header            — "Settings" bar (h 80, 25px top pad, border-bottom)
//! └── body row
//!     ├── tab column    — 220px; one 36px tab row per reachable tab
//!     │                   (Account · [SSH ID] · Terminal · SFTP · Logs ·
//!     │                    Shortcuts; Team/Vaults/Invite People are PORT-TODO)
//!     └── tab content   — scrolls; bg = --tab-content-color, one SettingsSection
//!                         card (20px pad, --card-a, 10px radius, max 700) per
//!                         group of controls
//! ```
//!
//! The layout mirrors the recovered sources: `setting-process-7c44cf6f.js`
//! (the window shell: `settingsHeader`, 220px `sidebar`, `settingsTabContent`)
//! and the per-tab pages (`index-13781084.js` Terminal, `SftpSettings-*`,
//! `LogsSettings-*`, `index-7370652f.js` Shortcuts).
//!
//! # The real tab set
//!
//! The authoritative nav is the `settingsNavigationSelector` in
//! `reconnectSaga-f0db0c3c.js` (see `analysis/recon/30-settings.md §1.1`):
//! **Team, Account, Vaults, SSH ID, Invite People, Terminal, SFTP, Logs,
//! Shortcuts**, filtered by ownership/team/auth/promo flags, with `defaultPath`
//! `/manage-team` for a team owner else `/account`. The list is built locally
//! by [`reachable_tabs`] (not `crate::navigation::settings_tabs`, which still
//! holds the old invented list) so this file can carry the real order without
//! touching an off-limits module. Only the tabs reachable with the current
//! local model render: **Account** (always), **SSH ID** (signed in),
//! **Terminal / SFTP / Logs / Shortcuts** (always). **Team / Vaults / Invite
//! People** need ownership / team / trial-promo fields the model does not carry
//! yet (PORT-TODO).
//!
//! # What actually mutates state
//!
//! * **Terminal** — `font_size`, `font_family`, `scrollback_lines` and
//!   `auto_reconnect` read straight from [`SettingsState`]; `Autoreconnect`
//!   writes `auto_reconnect` back. Rows the model does not carry (Option-as-Meta,
//!   Local Terminal Path, Keepalive, Detect OS, terminal theme, emulation) flip
//!   view-local [`LocalToggles`] (PORT-TODO: hoist them once `SettingsState`
//!   grows).
//! * **Logs** — the per-vault "Log retention" row opens the shell-hosted
//!   destructive confirmation via [`Dialog::Confirm`]; the toggle itself is
//!   view-local (PORT-TODO: `state.vaults` + a log-retention slice).
//!
//! Numeric rows (`font_family`, `scrollback_lines`, keepalive) are display-only:
//! [`crate::primitives::InputField`] has no editing yet (see the PORT-TODO on
//! that primitive). Dropdown rows have no popup primitive in gpui 0.2.2, so a
//! click cycles the options in place — the value is still live.

use gpui::{
    div, px, AnyElement, App, AppContext as _, ClickEvent, Context, Div, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::app_state::{Dialog, SettingsState, TermiusState};
use crate::assets::icon;
use crate::primitives::{
    Button, InputField, ListItem, SettingsSection, SettingsText, SettingsTitle, Switch,
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
/// Terminal colour schemes the scheme selector offers.
///
/// PORT-TODO: the original reads the real scheme registry
/// (`currentColorSchemeName` + `getColorScheme`, `index-13781084.js`); this is a
/// representative set until the palette model lands.
const TERMINAL_THEMES: [&str; 3] = ["Default", "Solarized Dark", "Solarized Light"];
/// Local terminal shells the path selector offers.
///
/// PORT-TODO: the original reads `localTerminalPath.customPaths` plus
/// `defaultLocalTerminalShell` (`index-13781084.js`).
const LOCAL_TERMINAL_PATHS: [&str; 3] = ["/bin/zsh", "/bin/bash", "/bin/sh"];
/// The personal vault the Logs tab lists while `TermiusState` has no vault
/// slice (PORT-TODO: read `state.vaults` once it lands).
const DEFAULT_VAULT_NAME: &str = "Default";
/// The Detect-OS script the accordion shows, verbatim from
/// `index-13781084.js` (`Vt`, `:843-867`).
const DETECT_OS_SCRIPT: &str = r#"# get the shell name to choose the script for detecting OS
  echo $SHELL

# for routeros
  :put [/system resource get platform]

# for fish shell
  if set name (uname) = "Linux"
      cat /etc/*release
  else
      uname
  end

# for others
  HISTFILE=;
  SA_OS_TYPE="Linux"
  REAL_OS_NAME=`uname`
  if [ "$REAL_OS_NAME" != "$SA_OS_TYPE" ] ;
  then
  echo `uname`
  else
  DISTRIB_ID="`cat /etc/*release`"
  echo $DISTRIB_ID;
  fi;
  exit;"#;

// ---------------------------------------------------------------------------
// Tab set (the original `settingsNavigationSelector`)
// ---------------------------------------------------------------------------

/// One Settings tab, in the original selector's order.
///
/// The original array is `Team, Account, Vaults, SSH ID, Invite People,
/// Terminal, SFTP, Logs, Shortcuts`; "Shortcuts" is the real label (the port
/// used to invent "Keyboard"), and there is no "Advanced" tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TabKind {
    Team,
    Account,
    Vaults,
    SshId,
    InvitePeople,
    Terminal,
    Sftp,
    Logs,
    Shortcuts,
}

impl TabKind {
    /// The sidebar label Termius shows (`title` in the selector).
    fn label(self) -> &'static str {
        match self {
            Self::Team => "Team",
            Self::Account => "Account",
            Self::Vaults => "Vaults",
            Self::SshId => "SSH ID",
            Self::InvitePeople => "Invite People",
            Self::Terminal => "Terminal",
            Self::Sftp => "SFTP",
            Self::Logs => "Logs",
            Self::Shortcuts => "Shortcuts",
        }
    }
}

impl Default for TabKind {
    /// `defaultPath` is `/account` for a non-owner (and `/manage-team` for a
    /// team owner — unreachable until ownership lands, see [`reachable_tabs`]).
    fn default() -> Self {
        Self::Account
    }
}

/// The tabs reachable with the current selector inputs, in original order.
///
/// Mirrors `settingsNavigationSelector` (`reconnectSaga-f0db0c3c.js`):
/// * **Team** when `is_team_owner` (also `defaultPath`),
/// * **Account** always,
/// * **Vaults** when `is_team`,
/// * **SSH ID** when `is_authorized`,
/// * **Invite People** when a team-trial promo shows && `is_authorized`,
/// * **Terminal / SFTP / Logs / Shortcuts** always.
///
/// The owning render passes the flags it can read; ownership / team / promo
/// have no model field yet, so it passes `false` for those (PORT-TODO).
fn reachable_tabs(
    is_team_owner: bool,
    is_team: bool,
    is_authorized: bool,
    has_team_trial_promo: bool,
) -> Vec<TabKind> {
    let mut tabs = Vec::new();
    if is_team_owner {
        tabs.push(TabKind::Team);
    }
    tabs.push(TabKind::Account);
    if is_team {
        tabs.push(TabKind::Vaults);
    }
    if is_authorized {
        tabs.push(TabKind::SshId);
    }
    if has_team_trial_promo && is_authorized {
        tabs.push(TabKind::InvitePeople);
    }
    tabs.push(TabKind::Terminal);
    tabs.push(TabKind::Sftp);
    tabs.push(TabKind::Logs);
    tabs.push(TabKind::Shortcuts);
    tabs
}

/// The tab actually shown: `active` when it is still reachable, else the first
/// reachable tab (Account, which is always present).
fn visible_tab(active: TabKind, tabs: &[TabKind]) -> TabKind {
    if tabs.contains(&active) {
        active
    } else {
        tabs.first().copied().unwrap_or(TabKind::Account)
    }
}

// ---------------------------------------------------------------------------
// Flags + view-local toggles
// ---------------------------------------------------------------------------

/// A bool on [`TermiusState::settings`] a `Switch` can flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingFlag {
    AutoReconnect,
}

/// Flip `flag` in `settings` (the caller notifies the state entity).
fn flip_setting_flag(settings: &mut SettingsState, flag: SettingFlag) {
    let slot = match flag {
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
    OptionAsMeta,
    AllowLocalSsh,
    BrightenBold,
    KeywordHighlighting,
    PostQuantum,
    DetectOs,
    DetectOsScriptOpen,
}

/// Flip `flag` in `local` (the caller notifies the view).
fn flip_local_flag(local: &mut LocalToggles, flag: LocalFlag) {
    let slot = match flag {
        LocalFlag::Autocomplete => &mut local.autocomplete,
        LocalFlag::ImportShellHistory => &mut local.import_shell_history,
        LocalFlag::CopyPaste => &mut local.copy_paste,
        LocalFlag::BellSound => &mut local.bell_sound,
        LocalFlag::OptionAsMeta => &mut local.option_as_meta,
        LocalFlag::AllowLocalSsh => &mut local.allow_local_ssh,
        LocalFlag::BrightenBold => &mut local.brighten_bold,
        LocalFlag::KeywordHighlighting => &mut local.keyword_highlighting,
        LocalFlag::PostQuantum => &mut local.post_quantum,
        LocalFlag::DetectOs => &mut local.detect_os,
        LocalFlag::DetectOsScriptOpen => &mut local.detect_os_script_open,
    };
    *slot = !*slot;
}

/// Tabs whose switches have no field on [`SettingsState`] yet.
///
/// Kept view-local so every `Switch` still flips a real bool instead of
/// painting a dead pill (PORT-TODO: hoist into `SettingsState` once the model
/// grows, then delete this).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalToggles {
    // Terminal settings (index-13781084.js).
    /// Inline autocomplete (the "Beta" row).
    autocomplete: bool,
    /// Import the shell history on connect.
    import_shell_history: bool,
    /// Select-to-copy + right-click-to-paste.
    copy_paste: bool,
    /// Beep on the terminal bell.
    bell_sound: bool,
    /// macOS: send Option as Meta (`U().platform === "mac"`).
    option_as_meta: bool,
    /// Sandbox: allow local SSH/SFTP connections.
    allow_local_ssh: bool,
    /// Bright ANSI colours for bold text.
    brighten_bold: bool,
    /// Highlight keywords in output.
    keyword_highlighting: bool,
    /// Post-quantum key exchange.
    post_quantum: bool,
    /// Execute a script on first connect to detect the host OS.
    detect_os: bool,
    /// Whether the "Detect OS Script" accordion is expanded.
    detect_os_script_open: bool,
    /// Selected emulation type (index into [`EMULATION_TYPES`]).
    emulation: usize,
    /// Selected terminal colour scheme (index into [`TERMINAL_THEMES`]).
    terminal_theme: usize,
    /// Selected local shell (index into [`LOCAL_TERMINAL_PATHS`]).
    local_terminal_path: usize,
    /// SSH keepalive interval in seconds (0 disables).
    keepalive_interval: u32,
    // Logs.
    /// Log retention for the personal vault (`LogsSettings-1defb994.js`).
    log_retention: bool,
}

impl Default for LocalToggles {
    fn default() -> Self {
        Self {
            autocomplete: true,
            import_shell_history: false,
            copy_paste: true,
            bell_sound: true,
            option_as_meta: false,
            allow_local_ssh: false,
            brighten_bold: false,
            keyword_highlighting: true,
            post_quantum: false,
            detect_os: false,
            detect_os_script_open: false,
            emulation: 0,
            terminal_theme: 0,
            local_terminal_path: 0,
            keepalive_interval: 0,
            log_retention: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Element helpers
// ---------------------------------------------------------------------------

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
/// in as a single column so their own 15/20px margins set the rhythm.
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

/// A `1px` hairline in `--background-entity` (`LogsSettings` `.divider`).
fn divider(theme: TermiusTheme) -> Div {
    div().w_full().h(px(1.)).bg(theme.card_b)
}

/// One keyword-highlighting colour swatch (`index-13781084.js` `Nt`):
/// `marginTop:15px`, name left, a `50×25` colour rectangle right.
fn swatch_row(theme: TermiusTheme, name: &'static str, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .mt(px(15.))
        .child(row_label(theme, name))
        .child(
            div()
                .w(px(50.))
                .h(px(25.))
                .rounded(px(theme.corner_radius_small))
                .bg(gpui::rgb(color)),
        )
}

/// The Detect-OS script accordion (`index-13781084.js` `useQt`): a
/// `--light-grey-6` / `--dark-grey-4` rounded container whose header toggles a
/// monospace script body.
///
/// PORT-TODO: the chevron does not rotate (gpui 0.2.2 only rotates `Svg` via
/// `Transformation`); the body simply shows/hides.
fn detect_os_accordion(
    theme: TermiusTheme,
    open: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let fill = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
    let header = div()
        .id("settings-detect-os-script")
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .py(px(10.))
        .cursor_pointer()
        .child(SettingsTitle::new("Detect OS Script").element(theme))
        .child(icon("allSettingsChevron.svg").w(px(15.)).h(px(6.)).text_color(theme.text_common))
        .on_click(listener);

    let mut container = div()
        .flex()
        .flex_col()
        .mt(px(15.))
        .px(px(10.))
        .rounded(px(theme.corner_radius_small))
        .bg(fill)
        .child(header);
    if open {
        let mut script = div().flex().flex_col().pb(px(10.));
        for line in DETECT_OS_SCRIPT.lines() {
            script = script.child(
                div()
                    .font_family("monospace")
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .whitespace_nowrap()
                    .text_color(theme.text_common)
                    .child(SharedString::from(line.to_owned())),
            );
        }
        container = container.child(script);
    }
    container
}

/// One per-vault "Log retention" row (`LogsSettings-1defb994.js` `M`): vault
/// icon + name left, an Enabled/Disabled value + chevron right.
fn vault_log_row(
    theme: TermiusTheme,
    name: &'static str,
    enabled: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let value = if enabled { "Enabled" } else { "Disabled" };
    div()
        .id(SharedString::from(format!("log-vault-{name}")))
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .p(px(10.))
        .rounded(px(theme.corner_radius_medium))
        .cursor_pointer()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .pr(px(10.))
                .min_w(px(0.))
                .child(icon("Vault.svg").w(px(24.)).h(px(24.)).text_color(theme.muted))
                .child(
                    text::R14P
                        .style(div())
                        .truncate()
                        .text_color(theme.title)
                        .child(SharedString::from(name)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    text::R12P
                        .style(div())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.title)
                        .child(SharedString::from(value)),
                )
                .child(icon("allSettingsChevron.svg").w(px(15.)).h(px(6.)).text_color(theme.text_common)),
        )
        .hover(move |style| style.bg(theme.hover))
        .on_click(listener)
}

/// Step the font size by `delta`, clamped to [`MIN_FONT_SIZE`] /
/// [`MAX_FONT_SIZE`] (never widens past either end).
fn clamp_font_size(current: u16, delta: i32) -> u16 {
    let next = i32::from(current) + delta;
    next.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE) as u16
}

/// The next index in a cycling chooser (wraps; empty stays at 0).
fn next_index(current: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (current + 1) % len
    }
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Settings screen: sidebar tab column + the active tab's controls.
///
/// Built by [`settings_screen`] and rendered by the shell as a child element
/// (like `HostList`); it owns only the active tab and the toggles that have no
/// [`SettingsState`] field yet.
pub struct SettingsScreen {
    state: Entity<TermiusState>,
    /// The active [`TabKind`] (re-validated against the reachable list each
    /// render, so sign-in/out cannot leave a blank pane).
    active_tab: TabKind,
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
            active_tab: TabKind::default(),
            local: LocalToggles::default(),
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// The tab actually shown (falls back when the stored tab left the list).
    fn visible_tab(&self, tabs: &[TabKind]) -> TabKind {
        visible_tab(self.active_tab, tabs)
    }

    // ----- actions --------------------------------------------------------

    fn select_tab(&mut self, tab: TabKind, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    /// Flip a persisted setting (`TermiusState` notifies its observers).
    fn flip_setting(&mut self, flag: SettingFlag, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            flip_setting_flag(&mut state.settings, flag);
            cx.notify();
        });
    }

    /// Flip a view-local toggle (Terminal / Detect OS).
    fn flip_local(&mut self, flag: LocalFlag, cx: &mut Context<Self>) {
        flip_local_flag(&mut self.local, flag);
        cx.notify();
    }

    /// The emulation dropdown: step to the next [`EMULATION_TYPES`] entry.
    fn cycle_emulation(&mut self, cx: &mut Context<Self>) {
        self.local.emulation = next_index(self.local.emulation, EMULATION_TYPES.len());
        cx.notify();
    }

    /// The terminal-theme selector: step to the next [`TERMINAL_THEMES`] entry.
    fn cycle_terminal_theme(&mut self, cx: &mut Context<Self>) {
        self.local.terminal_theme = next_index(self.local.terminal_theme, TERMINAL_THEMES.len());
        cx.notify();
    }

    /// The local-path selector: step to the next [`LOCAL_TERMINAL_PATHS`] entry.
    fn cycle_local_path(&mut self, cx: &mut Context<Self>) {
        self.local.local_terminal_path =
            next_index(self.local.local_terminal_path, LOCAL_TERMINAL_PATHS.len());
        cx.notify();
    }

    /// Set the personal vault's log retention (the enable direction is direct;
    /// disabling is gated behind the confirmation).
    fn set_log_retention(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.local.log_retention = enabled;
        cx.notify();
    }

    /// Raise the destructive "Confirm disabling logs" dialog
    /// (`LogsSettings-1defb994.js` `usePe`).
    ///
    /// PORT-TODO: the shell's generic [`Dialog::Confirm`] cannot run the
    /// original `Disable Logs` action on confirm, so the retention flag stays
    /// enabled until the real confirm host lands.
    fn request_disable_logs(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.open_dialog(
                Dialog::Confirm {
                    title: "Confirm disabling logs".to_owned(),
                    message: "Termius will stop storing logs for this vault. You can turn \
                              them back on at any time."
                        .to_owned(),
                },
                cx,
            );
        });
    }

    /// "Let us know" on the keyword-highlighting card opens ProductBoard in the
    /// original; surface a note until that bridge exists.
    fn open_keyword_feedback(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.status_text =
                "Keyword-highlighting feedback opens ProductBoard (PORT-TODO).".to_owned();
            cx.notify();
        });
    }

    fn adjust_font_size(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.settings.font_size = clamp_font_size(state.settings.font_size, delta);
            cx.notify();
        });
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

    /// Terminal: the `index-13781084.js` `Jt` section order — Terminal
    /// settings, Font, Terminal theme, Keyword highlighting, Detect Host OS,
    /// Local Terminal Path, Keepalive Interval, Scrollback, Post-Quantum.
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
        let terminal_theme = TERMINAL_THEMES[local.terminal_theme.min(TERMINAL_THEMES.len() - 1)];
        let local_path =
            LOCAL_TERMINAL_PATHS[local.local_terminal_path.min(LOCAL_TERMINAL_PATHS.len() - 1)];

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
                    "Import shell history",
                    local.import_shell_history,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::ImportShellHistory, cx);
                    }),
                )
                .into_any_element(),
                toggle(
                    theme,
                    "Select text to copy & Right click to paste",
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
                // macOS only in the original (`U().platform === "mac"`).
                // PORT-TODO: no platform field on the model.
                toggle(
                    theme,
                    "Use Option as Meta key",
                    local.option_as_meta,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::OptionAsMeta, cx);
                    }),
                )
                .into_any_element(),
                // Sandbox only in the original. PORT-TODO: no sandbox flag.
                toggle(
                    theme,
                    "Allow local SSH/SFTP connections",
                    local.allow_local_ssh,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::AllowLocalSsh, cx);
                    }),
                )
                .into_any_element(),
                SettingsText::new(
                    "Affects this device only, saved credentials will not be synced",
                )
                .element(theme)
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
            "Terminal theme",
            vec![
                dropdown_row(
                    theme,
                    "settings-terminal-theme",
                    row_label(theme, "Terminal theme"),
                    terminal_theme,
                    cx.listener(|this, _event, _window, cx| this.cycle_terminal_theme(cx)),
                )
                .into_any_element(),
                SettingsText::new("Colour scheme for the terminal. Source: App Preferences.")
                    .element(theme)
                    .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Keyword highlighting",
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
                swatch_row(theme, "Errors", 0xf2_5e_61).into_any_element(),
                swatch_row(theme, "Warnings", 0xe5_c0_7b).into_any_element(),
                swatch_row(theme, "Success", 0x21_b5_68).into_any_element(),
                swatch_row(theme, "Info", 0x56_b6_c2).into_any_element(),
                SettingsText::new(
                    "Would you like to add custom rules to highlight output in the terminal?",
                )
                .element(theme)
                .into_any_element(),
                Button::new("Let us know").primary().on_click(
                    theme,
                    cx.listener(|this, _event, _window, cx| this.open_keyword_feedback(cx)),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Detect Host Operating System",
            vec![
                SettingsText::new(
                    "This option will execute a script when connecting to a host for the first \
                     time to detect its operating system. This will allow Termius to show a \
                     corresponding OS icon in the list of hosts.",
                )
                .element(theme)
                .into_any_element(),
                SettingsText::new(
                    "The script will be executed only if server didn't return anything about its \
                     OS.",
                )
                .element(theme)
                .into_any_element(),
                toggle(
                    theme,
                    "Detect OS",
                    local.detect_os,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::DetectOs, cx);
                    }),
                )
                .into_any_element(),
                detect_os_accordion(
                    theme,
                    local.detect_os_script_open,
                    cx.listener(|this, _event, _window, cx| {
                        this.flip_local(LocalFlag::DetectOsScriptOpen, cx);
                    }),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Local Terminal Path",
            vec![dropdown_row(
                theme,
                "settings-local-path",
                row_label(theme, "Shell"),
                local_path,
                cx.listener(|this, _event, _window, cx| this.cycle_local_path(cx)),
            )
            .into_any_element()],
        ))
        .child(settings_card(
            theme,
            "Keepalive Interval",
            vec![
                SettingsText::new(
                    "How often (in seconds) to send SSH-level keepalive packets to the server. \
                     Set to 0 to disable.",
                )
                .element(theme)
                .into_any_element(),
                InputField::new("Interval", local.keepalive_interval.to_string())
                    .element(theme)
                    .into_any_element(),
                SettingsText::new("second(s)").element(theme).into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Scrollback",
            vec![
                SettingsText::new("Limit number of terminal rows. Set to 0 to maximum limit size.")
                    .element(theme)
                    .into_any_element(),
                InputField::new("Number of rows", settings.scrollback_lines.to_string())
                    .element(theme)
                    .into_any_element(),
            ],
        ))
        .child(settings_card(
            theme,
            "Post-Quantum Key Exchange",
            vec![Switch::new("Post-Quantum Key Exchange", local.post_quantum)
                .description("Turn off if you're experiencing issues with legacy devices")
                .element(theme)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.flip_local(LocalFlag::PostQuantum, cx);
                }))
                .into_any_element()],
        ))
    }

    /// SFTP: **only** the "File type associations" card
    /// (`SftpSettings-460ca9ed.js`). The original shows the associations table
    /// when one exists, else this empty hero.
    fn sftp_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        body.child(settings_card(
            theme,
            "File type associations",
            vec![
                SettingsTitle::new("No file type associations yet")
                    .element(theme)
                    .into_any_element(),
                SettingsText::new(
                    "Apps you choose using the “Open with…” option to open files via SFTP will \
                     appear here.",
                )
                .element(theme)
                .into_any_element(),
            ],
        ))
    }

    /// Logs: the per-vault "Log retention" list (`LogsSettings-1defb994.js`).
    /// No log-level chooser and no open-folder action exist in the original.
    fn logs_tab(&self, theme: TermiusTheme, body: Div, cx: &mut Context<Self>) -> Div {
        let enabled = self.local.log_retention;
        let row = vault_log_row(
            theme,
            DEFAULT_VAULT_NAME,
            enabled,
            cx.listener(|this, _event, _window, cx| {
                if this.local.log_retention {
                    this.request_disable_logs(cx);
                } else {
                    this.set_log_retention(true, cx);
                }
            }),
        );

        body.child(settings_card(
            theme,
            "Log retention",
            vec![
                SettingsText::new("Control how logs are stored in your vaults.")
                    .element(theme)
                    .into_any_element(),
                divider(theme).into_any_element(),
                row.into_any_element(),
            ],
        ))
    }

    /// Shortcuts: the scheme's shortcut list. The full scheme editor
    /// (select/rename/delete schemes, per-action rebinding, `KeyboardKey`) is
    /// PORT-TODO (`index-7370652f.js`); the rows render Shortcut | Action.
    fn shortcuts_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        const SHORTCUTS: [(&str, &str); 6] = [
            ("⌘B", "Toggle sidebar"),
            ("⌘⇧F", "Toggle SFTP panel"),
            ("⌘W", "Close tab"),
            ("⌘⇧C", "Copy selection"),
            ("⌘⇧V", "Paste from clipboard"),
            ("⌘K", "Clear terminal"),
        ];

        let mut shortcuts = SettingsSection::new("Shortcuts");
        for (keys, action) in SHORTCUTS {
            shortcuts = shortcuts.child(ListItem::new(keys, action).element(theme));
        }

        body.child(
            SettingsText::new(
                "Default scheme. The full scheme editor is PORT-TODO (index-7370652f.js).",
            )
            .element(theme),
        )
        .child(shortcuts.element(theme))
    }

    /// Account: the original `/account` tab is the `Profile` page
    /// (`index-fce1214f.js`: plan banner, subscription, 2FA, email, delete
    /// account, Synchronization). The port carries it as the dedicated Account
    /// section; this card points there (PORT-TODO: port the Profile tab).
    fn account_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        body.child(settings_card(
            theme,
            "Account",
            vec![SettingsText::new(
                "Plan, two-factor authentication, email verification, account deletion and \
                 Synchronization live in the Account section. PORT-TODO: port the Profile tab \
                 (index-fce1214f.js).",
            )
            .element(theme)
            .into_any_element()],
        ))
    }

    /// SSH ID: reachable when signed in (`isUserAuthorized`). PORT-TODO: the
    /// setup flow (`/sshid/setup`) is not ported yet.
    fn ssh_id_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        body.child(settings_card(
            theme,
            "SSH ID",
            vec![SettingsText::new(
                "Register an SSH ID to authenticate without passwords. PORT-TODO: the SSH ID \
                 setup flow (/sshid/setup) is not ported yet.",
            )
            .element(theme)
            .into_any_element()],
        ))
    }

    /// A tab the current model cannot reach yet (Team / Vaults / Invite
    /// People): render its name and a PORT-TODO note.
    fn pending_tab(&self, theme: TermiusTheme, tab: TabKind, body: Div) -> Div {
        body.child(settings_card(
            theme,
            tab.label(),
            vec![SettingsText::new(format!(
                "The \"{}\" settings tab needs ownership / team / promo fields the local model \
                 does not carry yet (PORT-TODO).",
                tab.label()
            ))
            .element(theme)
            .into_any_element()],
        ))
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let (settings, signed_in) = {
            let state = self.state.read(cx);
            (state.settings.clone(), state.account.is_some())
        };

        // PORT-TODO: `isTeamOwner`, `isTeam` and the team-trial promo have no
        // model field yet, so only the always-on tabs + SSH ID (signed in)
        // render today. Flip these three once the team slice lands.
        let tabs = reachable_tabs(false, false, signed_in, false);
        let active = self.visible_tab(&tabs);

        // Header: the original `settingsHeader` (h 80, 25px top pad, bottom rule).
        let header = div()
            .flex()
            .items_center()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .pt(px(25.))
            .px(px(20.))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.card_a)
            .child(SettingsTitle::new("Settings").element(theme));

        // Sidebar: the original 220px tab column (text-only rows with
        // hairline rules, selected row filled).
        let mut nav = div().flex().flex_col().w_full();
        for (position, tab) in tabs.iter().enumerate() {
            let tab = *tab;
            let selected = tab == active;
            let last = position + 1 == tabs.len();
            nav = nav.child(
                tab_button(theme, position, tab.label(), selected, last).on_click(cx.listener(
                    move |this, _event, _window, cx| this.select_tab(tab, cx),
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
        content = match active {
            TabKind::Terminal => self.terminal_tab(theme, &settings, content, cx),
            TabKind::Sftp => self.sftp_tab(theme, content),
            TabKind::Logs => self.logs_tab(theme, content, cx),
            TabKind::Shortcuts => self.shortcuts_tab(theme, content),
            TabKind::Account => self.account_tab(theme, content),
            TabKind::SshId => self.ssh_id_tab(theme, content),
            TabKind::Team | TabKind::Vaults | TabKind::InvitePeople => {
                self.pending_tab(theme, active, content)
            }
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
    fn full_tab_set_matches_the_original_order() {
        let tabs = reachable_tabs(true, true, true, true);
        let labels: Vec<&str> = tabs.iter().map(|tab| tab.label()).collect();
        assert_eq!(
            labels,
            [
                "Team",
                "Account",
                "Vaults",
                "SSH ID",
                "Invite People",
                "Terminal",
                "SFTP",
                "Logs",
                "Shortcuts",
            ]
        );
    }

    #[test]
    fn signed_out_shows_only_the_always_on_tabs() {
        let tabs = reachable_tabs(false, false, false, false);
        let labels: Vec<&str> = tabs.iter().map(|tab| tab.label()).collect();
        assert_eq!(labels, ["Account", "Terminal", "SFTP", "Logs", "Shortcuts"]);
        // The default path is Account for a non-owner.
        assert_eq!(tabs.first().copied(), Some(TabKind::Account));
    }

    #[test]
    fn signing_in_adds_ssh_id_after_account() {
        let tabs = reachable_tabs(false, false, true, false);
        let labels: Vec<&str> = tabs.iter().map(|tab| tab.label()).collect();
        assert_eq!(
            labels,
            ["Account", "SSH ID", "Terminal", "SFTP", "Logs", "Shortcuts"]
        );
    }

    #[test]
    fn team_owner_leads_with_team_and_the_promo_adds_invite_people() {
        let owner = reachable_tabs(true, false, false, false);
        assert_eq!(owner.first().copied(), Some(TabKind::Team));
        // Invite People needs the promo *and* authorization.
        assert!(!reachable_tabs(false, true, true, false).contains(&TabKind::InvitePeople));
        assert!(reachable_tabs(false, true, true, true).contains(&TabKind::InvitePeople));
    }

    #[test]
    fn labels_are_exact_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for tab in reachable_tabs(true, true, true, true) {
            let label = tab.label();
            assert!(!label.is_empty());
            assert!(seen.insert(label), "duplicate settings tab `{label}`");
        }
        assert_eq!(TabKind::SshId.label(), "SSH ID");
        assert_eq!(TabKind::InvitePeople.label(), "Invite People");
        assert_eq!(TabKind::Shortcuts.label(), "Shortcuts");
    }

    #[test]
    fn visible_tab_falls_back_when_the_active_tab_left_the_list() {
        let tabs = reachable_tabs(false, false, false, false);
        assert_eq!(visible_tab(TabKind::SshId, &tabs), TabKind::Account);
        assert_eq!(visible_tab(TabKind::Terminal, &tabs), TabKind::Terminal);
        assert_eq!(visible_tab(TabKind::SshId, &[] as &[TabKind]), TabKind::Account);
    }

    #[test]
    fn setting_flags_flip_the_stored_bools() {
        let mut settings = SettingsState::default();
        assert!(settings.auto_reconnect);

        flip_setting_flag(&mut settings, SettingFlag::AutoReconnect);
        assert!(!settings.auto_reconnect);
        flip_setting_flag(&mut settings, SettingFlag::AutoReconnect);
        assert!(settings.auto_reconnect);

        // Untouched fields stay untouched (cursor_blink is no longer a row).
        assert_eq!(settings.theme_mode, ThemeMode::Dark);
        assert_eq!(settings.font_size, 13);
        assert!(settings.cursor_blink);
    }

    #[test]
    fn local_flags_flip_the_view_toggles() {
        let mut local = LocalToggles::default();
        assert!(local.autocomplete);
        assert!(!local.post_quantum);
        assert!(!local.option_as_meta);
        assert!(!local.detect_os_script_open);

        flip_local_flag(&mut local, LocalFlag::Autocomplete);
        flip_local_flag(&mut local, LocalFlag::PostQuantum);
        flip_local_flag(&mut local, LocalFlag::OptionAsMeta);
        flip_local_flag(&mut local, LocalFlag::DetectOsScriptOpen);
        assert!(!local.autocomplete);
        assert!(local.post_quantum);
        assert!(local.option_as_meta);
        assert!(local.detect_os_script_open);

        // Log retention is not a bool flag; it stays on its default.
        assert!(local.log_retention);
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
    fn cycling_choosers_wrap() {
        assert_eq!(next_index(0, 4), 1);
        assert_eq!(next_index(3, 4), 0);
        assert_eq!(next_index(0, 0), 0);
        // The emulation picker starts at the first type.
        assert_eq!(EMULATION_TYPES[0], "xterm-256color");
        assert_eq!(TERMINAL_THEMES[0], "Default");
        assert_eq!(LOCAL_TERMINAL_PATHS[0], "/bin/zsh");
    }

    #[test]
    fn detect_os_script_is_the_original() {
        assert!(DETECT_OS_SCRIPT.contains("SA_OS_TYPE"));
        assert!(DETECT_OS_SCRIPT.contains("/system resource get platform"));
        assert!(DETECT_OS_SCRIPT.contains("DISTRIB_ID"));
    }

    #[test]
    fn controls_build_against_the_theme_without_a_window() {
        // Constructing (not painting) the interactive helpers must not need a
        // window: this is what each tab does inside `render`.
        let theme = TermiusTheme::dark();
        let _ = toggle(theme, "Detect OS", true, |_, _, _| {});
        let _ = toggle(theme, "Autoreconnect", false, |_, _, _| {});
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
        let _ = divider(theme);
        let _ = swatch_row(theme, "Errors", 0xf2_5e_61);
        let _ = detect_os_accordion(theme, true, |_, _, _| {});
        let _ = vault_log_row(theme, "Default", true, |_, _, _| {});
    }
}
