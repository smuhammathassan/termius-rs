//! app_shell — the root view: absolute top strip + spacer + pane split + dialog
//! overlay.
//!
//! Owns the single [`TermiusState`] entity, installs the app keybindings and
//! theme global ([`init`]), and opens the main window ([`open_window`]).
//!
//! # Structure (reconstructed from `analysis/recon/00-shell.md`)
//!
//! ```text
//! window
//! ├── TOP STRIP (absolute, h=51px, bottom 1px border, over the spacer)
//! │   ├── mac traffic-light spacer
//! │   ├── "Vault" · "SFTP" shortcut tabs
//! │   ├── connection/session tabs (TabBar) + trailing "＋"
//! │   └── right cluster — update pill · bell · key · gear · person · team · theme
//! ├── spacer (51px, normal flow)
//! └── paneSplit (h = 100% - 51px)
//!     ├── pane1  185px (⌘B; width 0 when the terminal is the active view) —
//!     │           the vertical section nav ([`LeftPanel`]) + trial promo
//!     └── pane2  flex-1 — exactly ONE of the active screen or the terminal
//! ```
//!
//! The section navigation is the **left vertical list**, never a top bar: the
//! top strip holds *connection/session* tabs, not sections. `pane2` shows a
//! single [`Pane2View`] — the routed section's screen, or the terminal — and
//! never stacks the host list beside the terminal. When the terminal is the
//! active view (or the SFTP section is routed) `pane1` collapses to width 0 and
//! `pane2` takes the full width (`isLeftPanelVisible`, `_main.js:112242`).

use gpui::{
    actions, div, point, px, size, AnyElement, App, AppContext as _, Bounds, BorrowAppContext,
    Context, Div, Entity, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions,
};

use crate::app_state::{Dialog, TermiusState};
use crate::navigation::Section;
use crate::primitives::{Button, DialogFrame, EmptyState, SettingsText};
use crate::theme::{theme_of, with_alpha, TermiusTheme};
use crate::views::account_screen::{account_screen, AccountScreen};
use crate::views::host_dialog::host_dialog_body;
use crate::views::keychain_screen::{keychain_screen, KeychainScreen};
use crate::views::keys_screen::{key_dialog_body, keys_screen, KeysScreen};
use crate::views::left_panel::{self, LeftPanel};
use crate::views::port_forwarding_screen::{
    port_forward_dialog_body, port_forwarding_screen, PortForwardList,
};
use crate::views::settings_screen::{settings_screen, SettingsScreen};
use crate::views::snippets_screen::{snippet_dialog_body, snippets_screen, SnippetsScreen};
use crate::views::team_screen::{team_screen, TeamScreen};
use crate::views::top_bar::{self, TRAFFIC_LIGHT_WIDTH};
use crate::views::{HostList, SftpPanel, TabBar, TerminalPane};

// Global actions bound app-wide (no key context: they fire from anywhere).
actions!(termius, [ToggleSidebar, ToggleSftp, CloseActiveTab]);

/// Left nav-rail width (`pane1.visible`: `185px`, `_main.js:112361`).
///
/// Also consumed by [`crate::views::terminal_pane`] to subtract the nav rail
/// from its terminal-grid geometry.
pub const SIDEBAR_WIDTH: f32 = left_panel::LEFT_PANEL_WIDTH;
/// The Hosts screen's host-list column width (inside `pane2`).
pub const HOST_LIST_WIDTH: f32 = 300.0;
/// SFTP panel width (same contract).
pub const SFTP_WIDTH: f32 = 300.0;
/// Tab strip height.
///
/// Kept for the terminal pane's chrome maths; the session tabs now live inside
/// the [`top_bar`] strip (its own [`top_bar::TOP_BAR_HEIGHT`]).
pub const TAB_BAR_HEIGHT: f32 = 40.0;
/// Status bar height.
///
/// The old fake status bar is gone; this constant survives only because
/// [`crate::views::terminal_pane`] still folds it into its content-height
/// calculation.
pub const STATUS_BAR_HEIGHT: f32 = 26.0;

/// The single view `pane2` renders.
///
/// The original shell has **one** `currentPath` that is either a base-tab
/// screen id or a horizontal terminal-tab id (`uFe.render()`,
/// `_main.js:112278`–`112301`); `GDe` swaps the routed screen while `jA` hosts
/// the terminal pages, and only one is on screen at a time. `TermiusState` has
/// no such field, so the shell derives this from `current_section` +
/// `active_session` (see [`AppShell::reconcile_view`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane2View {
    /// The routed section's screen.
    Screen(Section),
    /// The active terminal session.
    Terminal,
}

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
    crate::assets::load_fonts(cx);
    // The reference app runs the LIGHT appearance (dark top strip + light body),
    // so default to light to match; `TermiusTheme::system(cx)` follows the OS and
    // the top-strip toggle switches at runtime.
    cx.set_global(TermiusTheme::light());
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
        // The original Termius window is frameless: its own top strip carries
        // the traffic lights, so hide the system titlebar and place them inside
        // our strip (see `top_bar::TRAFFIC_LIGHT_WIDTH`).
        titlebar: Some(TitlebarOptions {
            title: Some("Termius".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(14.))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(120.), px(90.)),
            size: size(px(1280.), px(800.)),
        })),
        ..WindowOptions::default()
    };
    Ok(cx.open_window(options, |_window, cx| cx.new(|cx| AppShell::new(cx)))?)
}

/// Convenience entry point for `termius-app`: run the GPUI application with
/// this UI installed and the main window open.
pub fn launch() {
    gpui::Application::new()
        .with_assets(crate::assets::TermiusAssets)
        .run(|cx| {
        init(cx);
        if let Err(err) = open_window(cx) {
            tracing::error!(error = %err, "failed to open the main window");
        }
    });
}

/// The root view: composes the strip, the pane split, routes by section, hosts
/// dialogs, and handles the global actions.
pub struct AppShell {
    state: Entity<TermiusState>,
    /// The 185px vertical section rail (`pane1`).
    left_panel: Entity<LeftPanel>,
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
    /// Which single view `pane2` shows (derived, see [`Self::reconcile_view`]).
    pane2_view: Pane2View,
    /// Last routed section seen, to detect a nav-row click.
    last_section: Section,
    /// Last active session id seen, to detect a terminal-tab change.
    last_active_session: Option<String>,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

impl AppShell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| TermiusState::new(cx));
        let left_panel = cx.new(|cx| LeftPanel::new(state.clone(), cx));
        let host_list = cx.new(|cx| HostList::new(state.clone(), cx));
        // Hand the tab strip a weak handle back to the shell so selecting a
        // session tab can route `pane2` to the terminal (`TabBar` only owns the
        // state entity otherwise).
        let shell = cx.weak_entity();
        let tab_bar = cx.new(|_cx| TabBar::new(state.clone()).with_shell(shell));
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

        // Seed the derived active view from the state's initial routing.
        let (section, active) = {
            let state = state.read(cx);
            (state.current_section, state.active_session.clone())
        };

        let observe_state = cx.observe(&state, |this, _state, cx| {
            this.reconcile_view(cx);
            cx.notify();
        });
        // gpui 0.2: `Context::observe_global` hands the observer the entity
        // plus its context (`FnMut(&mut V, &mut Context<V>)`).
        let observe_theme =
            cx.observe_global::<TermiusTheme>(|_this, cx| cx.notify());
        Self {
            state,
            left_panel,
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
            pane2_view: Pane2View::Screen(section),
            last_section: section,
            last_active_session: active,
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

    /// Route the shell to a section **and** bring its screen to `pane2`.
    ///
    /// Unlike a bare [`TermiusState::set_section`], this always forces
    /// [`Pane2View::Screen`] even when `section` is already current (so the
    /// top-strip shortcut for the current section still leaves the terminal).
    fn show_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.pane2_view = Pane2View::Screen(section);
        self.last_section = section;
        self.state.update(cx, |state, cx| state.set_section(section, cx));
        cx.notify();
    }

    /// Bring the terminal to `pane2` (the top-strip session tabs call this).
    pub(crate) fn show_terminal(&mut self, cx: &mut Context<Self>) {
        self.pane2_view = Pane2View::Terminal;
        cx.notify();
    }

    /// Re-derive [`Pane2View`] from a [`TermiusState`] change.
    ///
    /// `TermiusState` has no explicit "active view" field, so the shell mirrors
    /// the original's single `currentPath` here: a **section** change means a
    /// nav row (or shortcut tab) was chosen, so show that screen; a **session**
    /// change means a terminal tab became (or stopped being) the active tab, so
    /// show the terminal.
    fn reconcile_view(&mut self, cx: &mut Context<Self>) {
        let (section, active) = {
            let state = self.state.read(cx);
            (state.current_section, state.active_session.clone())
        };
        if section != self.last_section {
            self.pane2_view = Pane2View::Screen(section);
        } else if active != self.last_active_session {
            self.pane2_view = if active.is_some() {
                Pane2View::Terminal
            } else {
                Pane2View::Screen(section)
            };
        }
        self.last_section = section;
        self.last_active_session = active;
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

    // ----- top strip ------------------------------------------------------

    /// The absolute 51px title strip: traffic-light spacer, the Vault/SFTP
    /// shortcut tabs, the connection tabs (via [`TabBar`]) and the right-hand
    /// cluster. It overlays the normal-flow spacer in [`Self::render`].
    fn top_strip(
        &mut self,
        theme: TermiusTheme,
        current: Section,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut strip = div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(top_bar::TOP_BAR_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .pl(px(9.))
            .pr(px(10.))
            .pt(px(11.))
            .pb(px(10.))
            .bg(theme.chrome_background())
            .text_color(theme.title)
            .border_b_1()
            .border_color(theme.border);

        // Spacer for the macOS traffic lights.
        strip = strip.child(
            div()
                .flex_none()
                .w(px(TRAFFIC_LIGHT_WIDTH))
                .h_full(),
        );

        // Fixed Vault · SFTP shortcut tabs (`RDe`).
        strip = strip.child(
            top_bar::shortcut_tab(theme, "vault-tab", "Vault", "Vault.svg", current == Section::Hosts)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.show_section(Section::Hosts, cx)
                })),
        );
        strip = strip.child(
            top_bar::shortcut_tab(theme, "sftp-tab", "SFTP", "Sftp.svg", current == Section::Sftp)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.show_section(Section::Sftp, cx)
                })),
        );

        // Divider between the shortcut tabs and the connection tabs.
        strip = strip.child(
            div().flex_none().w(px(1.)).h(px(20.)).mx(px(2.)).bg(theme.border),
        );

        // Connection/session tabs (title · status dot · close ×) + trailing ＋.
        strip = strip.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .items_center()
                .child(self.tab_bar.clone()),
        );

        // Right cluster, pinned to the far edge.
        strip = strip.child(self.top_bar_right(theme, current, cx));
        strip.into_any_element()
    }

    /// The top-strip right cluster (`zDe`): the "Update" pill, notification
    /// bell, and the off-nav screens' shortcuts — Keys, Settings (gear),
    /// Account (person), Team — plus a theme toggle.
    fn top_bar_right(
        &mut self,
        theme: TermiusTheme,
        current: Section,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut cluster = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .flex_none()
            .ml_auto()
            .pr(px(10.));

        cluster = cluster.child(top_bar::update_pill(theme));
        // Notification bell (no panel yet — styled affordance only).
        cluster = cluster.child(top_bar::icon_button(theme, "topbar-bell", "bell.svg", false));
        cluster = cluster.child(
            top_bar::icon_button(theme, "topbar-keys", "key.svg", current == Section::Keys)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.show_section(Section::Keys, cx)
                })),
        );
        cluster = cluster.child(
            top_bar::icon_button(theme, "topbar-gear", "gear.svg", current == Section::Settings)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.show_section(Section::Settings, cx)
                })),
        );
        cluster = cluster.child(
            top_bar::icon_button(theme, "topbar-account", "person.svg", current == Section::Account)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.show_section(Section::Account, cx)
                })),
        );
        cluster = cluster.child(
            top_bar::icon_button(theme, "topbar-team", "team.svg", current == Section::Team)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.show_section(Section::Team, cx)
                })),
        );
        cluster = cluster.child(
            top_bar::icon_button(theme, "topbar-theme", "themePaint.svg", false).on_click(
                cx.listener(|this, _event, _window, cx| this.toggle_theme(cx)),
            ),
        );
        cluster.into_any_element()
    }

    // ----- pane 2 (the single active view) --------------------------------

    /// The single active view (`GDe` **or** `jA`, never both).
    ///
    /// * [`Pane2View::Terminal`] — the active session's terminal, full width.
    /// * [`Pane2View::Screen`] — that section's screen, full width: the host
    ///   list for `Hosts`, the SFTP browser for `Sftp`, the section's screen
    ///   entity otherwise (an [`EmptyState`] stub for `Logs`).
    fn pane2(&self, theme: TermiusTheme, view: Pane2View) -> AnyElement {
        match view {
            Pane2View::Terminal => screen_slot(self.terminal.clone()).into_any_element(),
            Pane2View::Screen(section) => match section {
                Section::Hosts => screen_slot(self.host_list.clone()).into_any_element(),
                Section::Sftp => screen_slot(self.sftp.clone()).into_any_element(),
                Section::Snippets => screen_slot(self.snippets.clone()).into_any_element(),
                Section::Keys => screen_slot(self.keys.clone()).into_any_element(),
                Section::PortForwarding => {
                    screen_slot(self.port_forwarding.clone()).into_any_element()
                }
                Section::Keychain => screen_slot(self.keychain.clone()).into_any_element(),
                Section::Team => screen_slot(self.team.clone()).into_any_element(),
                Section::Settings => screen_slot(self.settings.clone()).into_any_element(),
                Section::Account => screen_slot(self.account.clone()).into_any_element(),
                Section::Logs => screen_slot(
                    EmptyState::new("Logs", "Session logs arrive with their screen.").element(theme),
                )
                .into_any_element(),
            },
        }
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
        let (sidebar_visible, section, dialog) = {
            let state = self.state.read(cx);
            (
                state.sidebar_visible,
                state.current_section,
                state.active_dialog.clone(),
            )
        };
        let view = self.pane2_view;

        // `isLeftPanelVisible` (`_main.js:112242`): the rail collapses when the
        // terminal is the active view, or the SFTP section is routed.
        let rail_visible = sidebar_visible
            && matches!(view, Pane2View::Screen(section) if section != Section::Sftp);

        // The absolute top strip (paints above the spacer).
        let strip = self.top_strip(theme, section, cx);

        // `paneSplit` — the 185px rail + the single active view.
        let mut split = div().flex().flex_row().flex_1().min_h(px(0.)).w_full();
        if rail_visible {
            split = split.child(
                div()
                    .flex_none()
                    .h_full()
                    .w(px(SIDEBAR_WIDTH))
                    .min_w(px(SIDEBAR_WIDTH))
                    .child(self.left_panel.clone()),
            );
        }
        split = split.child(self.pane2(theme, view));

        let mut root = div()
            .id("app-shell")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_sftp))
            .on_action(cx.listener(Self::on_close_active_tab))
            // `qDe`: the normal-flow 51px spacer under the absolute strip.
            .child(div().flex_none().h(px(top_bar::TOP_BAR_HEIGHT)))
            .child(split)
            .child(strip);

        // Dialog overlay paints last, above everything.
        if let Some(dialog) = dialog.as_ref() {
            root = root.child(self.dialog_overlay(dialog, theme, cx));
        }
        root
    }
}
