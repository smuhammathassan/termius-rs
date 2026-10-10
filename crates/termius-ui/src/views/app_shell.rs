//! app_shell — the root view: section sidebar + tab bar + routed content +
//! status bar + dialog overlay.
//!
//! Owns the single [`TermiusState`] entity, installs the app keybindings and
//! theme global ([`init`]), and opens the main window ([`open_window`]).
//!
//! # Routing
//!
//! ```text
//! AppShell
//! ├── sidebar (⌘B)
//! │   ├── section nav        — [`Section::ALL`], highlights `current_section`
//! │   └── contextual list    — Hosts → HostList; others → EmptyState stub
//! └── center
//!     ├── TabBar             — open sessions (always)
//!     ├── routed content     — Hosts → TerminalPane (+ SftpPanel ⌘⇧F)
//!     │                        Sftp  → TerminalPane + SftpPanel
//!     │                        Snippets/Keys/PortForwarding/Keychain/Team/
//!     │                        Settings/Account → the screen entity each
//!     │                        factory mints once in [`AppShell::new`]
//!     │                        Logs → EmptyState stub
//!     └── status bar
//! └── dialog overlay         — scrim + [`DialogFrame`] when a dialog is open,
//!                              the card filled in by the owning screen's
//!                              `*_dialog_body`
//! ```
//!
//! The section nav, the contextual list and the routed center all switch on
//! [`TermiusState::current_section`], and each screen owns its slice of
//! [`TermiusState`].

use gpui::{
    actions, div, px, AnyElement, App, AppContext as _, BorrowAppContext, Context, Div, Entity,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TitlebarOptions, Window,
    WindowHandle, WindowOptions,
};

use crate::app_state::{Dialog, TermiusState};
use crate::navigation::{sidebar_items, Section};
use crate::primitives::{Button, DialogFrame, EmptyState, SettingsText};
use crate::theme::{theme_of, with_alpha, TermiusTheme, ThemeMode};
use crate::views::account_screen::{account_screen, AccountScreen};
use crate::views::host_dialog::host_dialog_body;
use crate::views::keychain_screen::{keychain_screen, KeychainScreen};
use crate::views::keys_screen::{key_dialog_body, keys_screen, KeysScreen};
use crate::views::port_forwarding_screen::{
    port_forward_dialog_body, port_forwarding_screen, PortForwardList,
};
use crate::views::settings_screen::{settings_screen, SettingsScreen};
use crate::views::snippets_screen::{snippet_dialog_body, snippets_screen, SnippetsScreen};
use crate::views::team_screen::{team_screen, TeamScreen};
use crate::views::{HostList, SftpPanel, TabBar, TerminalPane};

// Global actions bound app-wide (no key context: they fire from anywhere).
actions!(termius, [ToggleSidebar, ToggleSftp, CloseActiveTab]);

/// Sidebar width; the terminal pane subtracts it from the window width.
pub const SIDEBAR_WIDTH: f32 = 260.0;
/// SFTP panel width (same contract).
pub const SFTP_WIDTH: f32 = 300.0;
/// Tab strip height.
pub const TAB_BAR_HEIGHT: f32 = 40.0;
/// Status bar height.
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
/// Sidebar section-nav row height.
const NAV_ROW_HEIGHT: f32 = 28.0;
/// Sidebar app-header height.
const SIDEBAR_HEADER_HEIGHT: f32 = 36.0;

/// A sized flex slot around one routed section screen.
///
/// The screen roots paint `size_full`, so they need a definite parent inside
/// the center flex row: this slot takes the remaining width (`flex_1`) and
/// the row's full height, and clips whatever the screen overflows.
fn screen_slot(screen: impl IntoElement) -> Div {
    div().flex_1().min_w(px(0.)).min_h(px(0.)).overflow_hidden().child(screen)
}

/// Install globals + keybindings. Call once from the app's `run` closure.
///
/// PORT-TODO: restore `settings.theme_mode` from the store here instead of
/// hardcoding dark (see [`TermiusState`](crate::app_state::TermiusState)).
pub fn init(cx: &mut App) {
    cx.set_global(TermiusTheme::dark());
    cx.bind_keys([
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-f", ToggleSftp, None),
        KeyBinding::new("ctrl-shift-f", ToggleSftp, None),
        KeyBinding::new("cmd-w", CloseActiveTab, None),
        KeyBinding::new("ctrl-w", CloseActiveTab, None),
    ]);
}

/// Open the main application window.
pub fn open_window(cx: &mut App) -> anyhow::Result<WindowHandle<AppShell>> {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Termius".into()),
            ..TitlebarOptions::default()
        }),
        ..WindowOptions::default()
    };
    Ok(cx.open_window(options, |_window, cx| cx.new(|cx| AppShell::new(cx)))?)
}

/// Convenience entry point for `termius-app`: run the GPUI application with
/// this UI installed and the main window open.
pub fn launch() {
    gpui::Application::new().run(|cx| {
        init(cx);
        if let Err(err) = open_window(cx) {
            tracing::error!(error = %err, "failed to open the main window");
        }
    });
}

/// The root view: composes the panels, routes by section, hosts dialogs, and
/// handles the global actions.
pub struct AppShell {
    state: Entity<TermiusState>,
    host_list: Entity<HostList>,
    tab_bar: Entity<TabBar>,
    terminal: Entity<TerminalPane>,
    sftp: Entity<SftpPanel>,
    /// One entity per routed section screen (built once, never per frame).
    snippets: Entity<SnippetsScreen>,
    keys: Entity<KeysScreen>,
    port_forwarding: Entity<PortForwardList>,
    settings: Entity<SettingsScreen>,
    account: Entity<AccountScreen>,
    team: Entity<TeamScreen>,
    keychain: Entity<KeychainScreen>,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

impl AppShell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| TermiusState::new(cx));
        let host_list = cx.new(|cx| HostList::new(state.clone(), cx));
        let tab_bar = cx.new(|_cx| TabBar::new(state.clone()));
        let terminal = cx.new(|cx| TerminalPane::new(state.clone(), cx));
        let sftp = cx.new(|_cx| SftpPanel::new(state.clone()));

        // Section screens: minted once, behind the state's update lease. The
        // factories only `cx.new` their view entity (no `Entity::read` of the
        // leased `TermiusState`), so this is the safe call site — each screen
        // reads the state later, in its own `Render` (see `snippets_screen`).
        let snippets = state.update(cx, |_, cx| snippets_screen(state.clone(), cx));
        let keys = state.update(cx, |_, cx| keys_screen(state.clone(), cx));
        let port_forwarding = state.update(cx, |_, cx| port_forwarding_screen(state.clone(), cx));
        let settings = state.update(cx, |_, cx| settings_screen(state.clone(), cx));
        let account = state.update(cx, |_, cx| account_screen(state.clone(), cx));
        let team = state.update(cx, |_, cx| team_screen(state.clone(), cx));
        let keychain = state.update(cx, |_, cx| keychain_screen(state.clone(), cx));

        let observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        // gpui 0.2: `Context::observe_global` hands the observer the entity
        // plus its context (`FnMut(&mut V, &mut Context<V>)`).
        let observe_theme =
            cx.observe_global::<TermiusTheme>(|_this, cx| cx.notify());
        Self {
            state,
            host_list,
            tab_bar,
            terminal,
            sftp,
            snippets,
            keys,
            port_forwarding,
            settings,
            account,
            team,
            keychain,
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.toggle_sidebar(cx));
    }

    fn toggle_sftp(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.toggle_sftp(cx));
    }

    fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        let active = self.state.read(cx).active_session.clone();
        if let Some(session_id) = active {
            self.state.update(cx, |state, cx| state.close_session(&session_id, cx));
        }
    }

    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_section(section, cx));
    }

    fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.close_dialog(cx));
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        cx.update_global::<TermiusTheme, _>(|theme, cx| {
            let next = theme.toggled();
            *theme = next;
            cx.notify();
        });
    }

    // ----- sidebar --------------------------------------------------------

    /// The left column: section nav + the section's contextual list.
    fn sidebar(
        &mut self,
        theme: TermiusTheme,
        current: Section,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut column = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar_background)
            .text_color(theme.foreground)
            .border_r_1()
            .border_color(theme.border);

        // App header.
        column = column.child(
            div()
                .flex()
                .items_center()
                .h(px(SIDEBAR_HEADER_HEIGHT))
                .px(px(12.))
                .border_b_1()
                .border_color(theme.border)
                .child(SharedString::from("Termius")),
        );

        // Section nav (order from `navigation::sidebar_items`).
        for section in sidebar_items() {
            let icon = section.icon();
            let label = section.label();
            let active = *section == current;
            let mut row = div()
                .id(SharedString::from(format!("nav-{label}")))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(NAV_ROW_HEIGHT))
                .px(px(12.))
                .rounded(px(4.))
                .text_color(if active { theme.foreground } else { theme.muted });
            if active {
                row = row.bg(theme.tab_active);
            }
            row = row.child(SharedString::from(icon)).child(SharedString::from(label));
            let target = *section;
            column = column.child(
                row.on_click(cx.listener(move |this, _event, _window, cx| {
                    this.set_section(target, cx);
                })),
            );
        }

        // Contextual list below the nav: the host tree today, stubs elsewhere
        // (later waves swap each arm for its own panel).
        let contextual: AnyElement = match current {
            Section::Hosts => self.host_list.clone().into_any_element(),
            other => EmptyState::new(
                format!("{} list", other.label()),
                "This section's list arrives with its screen.",
            )
            .element(theme)
            .into_any_element(),
        };
        column = column.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(contextual),
        );
        column.into_any_element()
    }

    // ----- dialog overlay -------------------------------------------------

    /// A full-window scrim + centered [`DialogFrame`] for `dialog`.
    ///
    /// Clicking the scrim (outside the card) dismisses the dialog.
    fn dialog_overlay(
        &mut self,
        dialog: &Dialog,
        theme: TermiusTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = dialog.title();

        // Dispatch to the owning screen's body while the state entity is
        // leased: every `*_dialog_body` takes `&TermiusState` + that entity's
        // own `Context`, which gpui only hands out together inside
        // `state.update(..)`. The snippet builder returns a complete
        // [`DialogFrame`] card (title bar + Delete/Cancel/Save already in
        // place); the host / key / port-forward builders return a bare body
        // that carries its own action row, so it is framed without adding a
        // second one.
        let (body, complete_card) = self.state.update(cx, |state, cx| {
            if let Some(card) = snippet_dialog_body(dialog, state, cx) {
                (Some(card), true)
            } else if let Some(body) = host_dialog_body(dialog, state, cx) {
                (Some(body), false)
            } else if let Some(body) = key_dialog_body(dialog, state, cx) {
                (Some(body), false)
            } else if let Some(body) = port_forward_dialog_body(dialog, state, cx) {
                (Some(body), false)
            } else {
                (None, false)
            }
        });

        let card: AnyElement = match body {
            Some(body) if complete_card => body,
            Some(body) => DialogFrame::new(title).child(body).element(theme).into_any_element(),
            None => {
                // Confirmations and unknown dialogs keep the generic card.
                let section_label = dialog
                    .section()
                    .map(|section| section.label())
                    .unwrap_or("Termius")
                    .to_owned();
                let message = match dialog.message() {
                    Some(message) => message.to_owned(),
                    None => format!("{title} — the {section_label} screen fills this dialog in."),
                };
                DialogFrame::new(title)
                    .child(SettingsText::new(message).element(theme))
                    .action(
                        Button::new("Close").primary().on_click(
                            theme,
                            cx.listener(|this, _event, _window, cx| this.close_dialog(cx)),
                        ),
                    )
                    .element(theme)
                    .into_any_element()
            }
        };

        // Overlay pattern from gpui 0.2.2's own `window/prompts.rs`: absolute
        // scrim + absolute centered layer, both sized to the window.
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .size_full()
                    .absolute()
                    .top_0()
                    .left_0()
                    .bg(with_alpha(gpui::rgb(0x00_0000), 0.55))
                    .id("dialog-scrim")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.close_dialog(cx)
                    })),
            )
            .child(
                div()
                    .size_full()
                    .absolute()
                    .top_0()
                    .left_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(card),
            )
            .into_any_element()
    }

    // ----- actions --------------------------------------------------------

    fn on_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_sidebar(cx);
    }

    fn on_toggle_sftp(&mut self, _: &ToggleSftp, _window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sftp(cx);
    }

    fn on_close_active_tab(
        &mut self,
        _: &CloseActiveTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_active_tab(cx);
    }
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let (sidebar, sftp, status, sessions, section, dialog) = {
            let state = self.state.read(cx);
            (
                state.sidebar_visible,
                state.sftp_visible,
                state.status_text.clone(),
                state.sessions.len(),
                state.current_section,
                state.active_dialog.clone(),
            )
        };

        // Status bar (built before the body: it registers click listeners).
        let mut status_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(STATUS_BAR_HEIGHT))
            .px(px(10.))
            .bg(theme.status_background)
            .border_t_1()
            .border_color(theme.border)
            .text_color(theme.muted);
        status_bar = status_bar.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(SharedString::from(section.label()))
                .child(SharedString::from(status))
                .child(SharedString::from(format!("{sessions} open"))),
        );

        let sidebar_btn = div()
            .id("status-toggle-sidebar")
            .px(px(6.))
            .rounded(px(3.))
            .text_color(theme.foreground)
            .child(SharedString::from("Hosts ⌘B"))
            .on_click(cx.listener(|this, _event, _window, cx| this.toggle_sidebar(cx)));
        let sftp_btn = div()
            .id("status-toggle-sftp")
            .px(px(6.))
            .rounded(px(3.))
            .text_color(theme.foreground)
            .child(SharedString::from("Files ⌘⇧F"))
            .on_click(cx.listener(|this, _event, _window, cx| this.toggle_sftp(cx)));
        let theme_label = if theme.mode == ThemeMode::Dark { "☾ dark" } else { "☀ light" };
        let theme_btn = div()
            .id("status-toggle-theme")
            .px(px(6.))
            .rounded(px(3.))
            .text_color(theme.foreground)
            .child(SharedString::from(theme_label))
            .on_click(cx.listener(|this, _event, _window, cx| this.toggle_theme(cx)));
        status_bar = status_bar.child(
            div()
                .flex()
                .flex_row()
                .gap(px(4.))
                .child(sidebar_btn)
                .child(sftp_btn)
                .child(theme_btn),
        );

        // Center column: tabs → routed content → status bar.
        let mut center = div().flex().flex_col().flex_1().min_h(px(0.));
        center = center.child(self.tab_bar.clone());

        let mut middle = div().flex().flex_row().flex_1().min_h(px(0.));
        match section {
            Section::Hosts => {
                middle = middle.child(self.terminal.clone());
                if sftp {
                    middle = middle.child(
                        div().w(px(SFTP_WIDTH)).min_w(px(SFTP_WIDTH)).child(self.sftp.clone()),
                    );
                }
            }
            Section::Sftp => {
                // The SFTP screen is the browser beside the live terminal
                // (`TermiusState::set_section` opens the panel on entry).
                middle = middle.child(self.terminal.clone());
                middle = middle.child(
                    div().w(px(SFTP_WIDTH)).min_w(px(SFTP_WIDTH)).child(self.sftp.clone()),
                );
            }
            Section::Snippets => middle = middle.child(screen_slot(self.snippets.clone())),
            Section::Keys => middle = middle.child(screen_slot(self.keys.clone())),
            Section::PortForwarding => {
                middle = middle.child(screen_slot(self.port_forwarding.clone()))
            }
            Section::Keychain => middle = middle.child(screen_slot(self.keychain.clone())),
            Section::Team => middle = middle.child(screen_slot(self.team.clone())),
            Section::Settings => middle = middle.child(screen_slot(self.settings.clone())),
            Section::Account => middle = middle.child(screen_slot(self.account.clone())),
            other => {
                middle = middle.child(
                    EmptyState::new(
                        format!("{} — coming soon", other.label()),
                        "This screen is filled in by its section wave.",
                    )
                    .element(theme),
                );
            }
        }
        center = center.child(middle);
        center = center.child(status_bar);

        // Body: section sidebar + center.
        let mut body = div().flex().flex_row().flex_1().min_h(px(0.));
        if sidebar {
            body = body.child(
                div()
                    .w(px(SIDEBAR_WIDTH))
                    .min_w(px(SIDEBAR_WIDTH))
                    .child(self.sidebar(theme, section, cx)),
            );
        }
        body = body.child(center);

        let mut root = div().id("app-shell")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_sftp))
            .on_action(cx.listener(Self::on_close_active_tab))
            .child(body);

        // Dialog overlay paints last, above everything.
        if let Some(dialog) = dialog.as_ref() {
            root = root.child(self.dialog_overlay(dialog, theme, cx));
        }
        root
    }
}
