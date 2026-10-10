//! settings_screen — the Settings section: sub-tab strip + per-tab controls.
//!
//! Port of Termius' Settings window. [`settings_screen`] builds a small
//! self-contained view (GPUI renders the returned entity as a child, exactly
//! like `HostList`), so the active sub-tab lives with the screen instead of
//! on [`TermiusState`]. The tab labels come from
//! [`settings_tabs`]; every row is a [`crate::primitives`] component, so a
//! theme switch repaints this screen for free.
//!
//! ```text
//! SettingsScreen
//! ├── header          — SettingsTitle + SettingsText
//! ├── tab strip       — settings_tabs(): Terminal · SFTP · Logs · Advanced · Keyboard · Team
//! └── body (scrolls)  — one SettingsSection card per group of controls
//! ```
//!
//! # What actually mutates state
//!
//! * **Terminal** — `cursor_blink`, `auto_reconnect`, `theme_mode` and
//!   `font_size` write straight into [`TermiusState::settings`]; `theme_mode`
//!   additionally repaints the [`TermiusTheme`] global so the choice is
//!   visible immediately.
//! * **SFTP / Advanced** — these toggles have no field on
//!   [`SettingsState`] yet, so they flip view-local [`LocalToggles`] bools
//!   (PORT-TODO: hoist them into `SettingsState` once the model grows). Every
//!   `Switch` still flips a real bool; none of them is a dead pill.
//! * **Logs** — the level chooser writes `LocalToggles::log_level`; the
//!   "Open log folder" button reveals a note until the desktop shell bridge
//!   can open a Finder window.
//!
//! Numeric rows (`font_family`, `scrollback_lines`, paths) are display-only:
//! [`crate::primitives::InputField`] has no editing yet (see the PORT-TODO on
//! that primitive).

use gpui::{
    div, px, App, AppContext as _, BorrowAppContext as _, ClickEvent, Context, Div, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::app_state::{SettingsState, TermiusState};
use crate::navigation::settings_tabs;
use crate::primitives::{
    Button, EmptyState, InputField, ListItem, SettingsSection, SettingsText, SettingsTitle, Switch,
};
use crate::theme::{theme_of, TermiusTheme, ThemeMode};

/// Smallest font size the stepper allows (points).
const MIN_FONT_SIZE: i32 = 6;
/// Largest font size the stepper allows (points).
const MAX_FONT_SIZE: i32 = 48;

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
    /// SFTP: keep the remote mtime on downloads/uploads.
    sftp_preserve_mtime: bool,
    /// SFTP: follow symbolic links while transferring.
    sftp_follow_symlinks: bool,
    /// SFTP: list dotfiles in the browser.
    sftp_show_hidden: bool,
    /// Log level shown by the Logs tab.
    log_level: LogLevel,
    /// Whether the "open log folder" note is visible.
    log_folder_note: bool,
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

/// A [`Switch`] row with its click listener attached (the caller flips the
/// backing bool).
fn toggle(
    theme: TermiusTheme,
    label: &'static str,
    on: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    Switch::new(label, on).element(theme).on_click(listener)
}

/// One pill of a small inline chooser (theme mode, log level).
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
    let text = if active {
        gpui::rgb(0xff_ff_ff)
    } else {
        theme.muted
    };

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
        .text_color(text)
        .child(label)
        .on_click(listener)
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

/// The Settings screen: sub-tab strip + the active tab's controls.
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

    // ----- tabs -----------------------------------------------------------

    /// Terminal: theme, font, cursor + reconnect.
    fn terminal_tab(
        &self,
        theme: TermiusTheme,
        settings: &SettingsState,
        body: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let mode = settings.theme_mode;
        let theme_row = div()
            .flex()
            .flex_row()
            .gap(px(6.))
            .child(choice(
                SharedString::from("theme-mode-dark"),
                SharedString::from("Dark"),
                mode == ThemeMode::Dark,
                theme,
                cx.listener(|this, _event, _window, cx| {
                    this.set_theme_mode(ThemeMode::Dark, cx);
                }),
            ))
            .child(choice(
                SharedString::from("theme-mode-light"),
                SharedString::from("Light"),
                mode == ThemeMode::Light,
                theme,
                cx.listener(|this, _event, _window, cx| {
                    this.set_theme_mode(ThemeMode::Light, cx);
                }),
            ));

        let size_row = div()
            .flex()
            .flex_row()
            .items_end()
            .gap(px(8.))
            .child(
                InputField::new("Font size", format!("{} pt", settings.font_size)).element(theme),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(4.))
                    .child(Button::new("A-").secondary().on_click(
                        theme,
                        cx.listener(|this, _event, _window, cx| this.adjust_font_size(-1, cx)),
                    ))
                    .child(Button::new("A+").secondary().on_click(
                        theme,
                        cx.listener(|this, _event, _window, cx| this.adjust_font_size(1, cx)),
                    )),
            );

        body.child(SettingsTitle::new("Terminal").element(theme))
            .child(
                SettingsText::new("Font, cursor and session behaviour shared by every terminal.")
                    .element(theme),
            )
            .child(
                SettingsSection::new("Theme")
                    .child(theme_row)
                    .child(
                        SettingsText::new("Applies to the app chrome and to the terminal palette.")
                            .element(theme),
                    )
                    .element(theme),
            )
            .child(
                SettingsSection::new("Font")
                    .child(
                        InputField::new("Font family", settings.font_family.clone()).element(theme),
                    )
                    .child(size_row)
                    .child(
                        SettingsText::new(format!(
                            "Family is display-only until the text editor lands; size steps \
                         {MIN_FONT_SIZE}-{MAX_FONT_SIZE} pt (PORT-TODO)."
                        ))
                        .element(theme),
                    )
                    .element(theme),
            )
            .child(
                SettingsSection::new("Session")
                    .child(toggle(
                        theme,
                        "Cursor blink",
                        settings.cursor_blink,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_setting(SettingFlag::CursorBlink, cx);
                        }),
                    ))
                    .child(toggle(
                        theme,
                        "Auto-reconnect",
                        settings.auto_reconnect,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_setting(SettingFlag::AutoReconnect, cx);
                        }),
                    ))
                    .child(
                        InputField::new("Scrollback lines", settings.scrollback_lines.to_string())
                            .element(theme),
                    )
                    .child(
                        SettingsText::new(
                            "Scrollback and font family are display-only until the numeric/text \
                         editor lands (PORT-TODO).",
                        )
                        .element(theme),
                    )
                    .element(theme),
            )
    }

    /// SFTP: transfer toggles (view-local) + path fields.
    fn sftp_tab(&self, theme: TermiusTheme, body: Div, cx: &mut Context<Self>) -> Div {
        let local = &self.local;
        body.child(SettingsTitle::new("SFTP").element(theme))
            .child(
                SettingsText::new("Transfer and browser defaults for every session.")
                    .element(theme),
            )
            .child(
                SettingsSection::new("Transfers")
                    .child(toggle(
                        theme,
                        "Preserve remote timestamps",
                        local.sftp_preserve_mtime,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_local(LocalFlag::SftpPreserveMtime, cx);
                        }),
                    ))
                    .child(toggle(
                        theme,
                        "Follow symbolic links",
                        local.sftp_follow_symlinks,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_local(LocalFlag::SftpFollowSymlinks, cx);
                        }),
                    ))
                    .child(toggle(
                        theme,
                        "Show hidden files",
                        local.sftp_show_hidden,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_local(LocalFlag::SftpShowHidden, cx);
                        }),
                    ))
                    .element(theme),
            )
            .child(
                SettingsSection::new("Paths")
                    .child(InputField::new("Download folder", "~/Downloads").element(theme))
                    .child(InputField::new("Upload folder", "~/").element(theme))
                    .child(
                        SettingsText::new(
                            "Paths are display-only until the folder picker lands (PORT-TODO).",
                        )
                        .element(theme),
                    )
                    .element(theme),
            )
    }

    /// Logs: level chooser + "open log folder".
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

        body.child(SettingsTitle::new("Logs").element(theme))
            .child(
                SettingsText::new("Diagnostic output for this app (not the remote session).")
                    .element(theme),
            )
            .child(
                SettingsSection::new("Level")
                    .child(chooser)
                    .child(
                        SettingsText::new("Info is the daily default; Debug is very noisy.")
                            .element(theme),
                    )
                    .element(theme),
            )
            .child(files.element(theme))
    }

    /// Advanced: application toggles (view-local) + the data folder.
    fn advanced_tab(&self, theme: TermiusTheme, body: Div, cx: &mut Context<Self>) -> Div {
        let local = &self.local;
        body.child(SettingsTitle::new("Advanced").element(theme))
            .child(
                SettingsText::new("Desktop behaviour that is not tied to a session.")
                    .element(theme),
            )
            .child(
                SettingsSection::new("Application")
                    .child(toggle(
                        theme,
                        "Send anonymous usage data",
                        local.usage_data,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_local(LocalFlag::UsageData, cx);
                        }),
                    ))
                    .child(toggle(
                        theme,
                        "Download updates automatically",
                        local.auto_update,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_local(LocalFlag::AutoUpdate, cx);
                        }),
                    ))
                    .child(toggle(
                        theme,
                        "Confirm before closing a session",
                        local.confirm_close,
                        cx.listener(|this, _event, _window, cx| {
                            this.flip_local(LocalFlag::ConfirmClose, cx);
                        }),
                    ))
                    .element(theme),
            )
            .child(
                SettingsSection::new("Storage")
                    .child(
                        InputField::new("Data folder", "~/Library/Application Support/Termius")
                            .element(theme),
                    )
                    .child(
                        SettingsText::new(
                            "The data folder is read-only for now (PORT-TODO: folder picker).",
                        )
                        .element(theme),
                    )
                    .element(theme),
            )
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

        body.child(SettingsTitle::new("Keyboard").element(theme))
            .child(
                SettingsText::new(
                    "Desktop defaults; rebinding arrives with the keymap editor (PORT-TODO).",
                )
                .element(theme),
            )
            .child(shortcuts.element(theme))
    }

    /// Team: placeholder pointing at the Team section.
    fn team_tab(&self, theme: TermiusTheme, body: Div) -> Div {
        body.child(SettingsTitle::new("Team").element(theme))
            .child(
                div().h(px(240.)).child(
                    EmptyState::new(
                        "Team & Vaults",
                        "Shared hosts, keys and vaults live in the Team section — sign in \
                             to sync them.",
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

        // Sub-tab strip: click switches the pane below.
        let mut strip = div()
            .id("settings-tab-strip")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(theme.border);
        for (position, name) in tabs.iter().enumerate() {
            let active = position == index;
            let mut chip = div()
                .id(SharedString::from(format!("settings-tab-{name}")))
                .flex()
                .items_center()
                .h(px(26.))
                .px(px(10.))
                .rounded(px(4.))
                .text_sm()
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted
                });
            if active {
                chip = chip.bg(theme.tab_active);
            }
            chip = chip.child(SharedString::from(*name));
            strip = strip.child(chip.on_click(cx.listener(move |this, _event, _window, cx| {
                this.select_tab(position, cx);
            })));
        }

        // Scrollable body: one pane per tab. The pane builders take a plain
        // `Div`, so the scrolling styles are applied afterwards —
        // `overflow_y_scroll` lives on `StatefulInteractiveElement`, which
        // only exists once the container has an element id.
        let mut body = div().flex().flex_col().gap(px(12.)).p(px(16.)).flex_1().min_h(px(0.));
        body = match tab_kind(label) {
            TabKind::Terminal => self.terminal_tab(theme, &settings, body, cx),
            TabKind::Sftp => self.sftp_tab(theme, body, cx),
            TabKind::Logs => self.logs_tab(theme, body, cx),
            TabKind::Advanced => self.advanced_tab(theme, body, cx),
            TabKind::Keyboard => self.keyboard_tab(theme, body),
            TabKind::Team => self.team_tab(theme, body),
            TabKind::Unknown => body.child(
                SettingsText::new(format!("The \"{label}\" tab has no controls yet."))
                    .element(theme),
            ),
        };
        let body = body.id("settings-body").overflow_y_scroll();

        let header = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(16.))
            .pt(px(14.))
            .child(SettingsTitle::new("Settings").element(theme))
            .child(
                SettingsText::new("Preferences for this device — changes apply immediately.")
                    .element(theme),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(header)
            .child(strip)
            .child(body)
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
    }
}
