//! app_shell — the root view: sidebar + tab bar + terminal panes + status bar.
//!
//! Owns the single [`TermiusState`] entity, installs the app keybindings and
//! theme global ([`init`]), and opens the main window ([`open_window`]).

use gpui::{
    actions, div, px, App, AppContext as _, BorrowAppContext, Context, Entity,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TitlebarOptions, Window,
    WindowHandle, WindowOptions,
};

use crate::app_state::TermiusState;
use crate::theme::{theme_of, TermiusTheme, ThemeMode};
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

/// Install globals + keybindings. Call once from the app's `run` closure.
///
/// PORT-TODO: persist the theme mode through `store.settings()` and restore
/// it here instead of hardcoding dark.
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

/// The root view: composes the four panels and the status bar, owns the
/// state, and handles the global actions.
pub struct AppShell {
    state: Entity<TermiusState>,
    host_list: Entity<HostList>,
    tab_bar: Entity<TabBar>,
    terminal: Entity<TerminalPane>,
    sftp: Entity<SftpPanel>,
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

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        cx.update_global::<TermiusTheme, _>(|theme, cx| {
            let next = theme.toggled();
            *theme = next;
            cx.notify();
        });
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
        let (sidebar, sftp, status, sessions) = {
            let state = self.state.read(cx);
            (
                state.sidebar_visible,
                state.sftp_visible,
                state.status_text.clone(),
                state.sessions.len(),
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

        // Center column: tabs → (terminal | files).
        let mut center = div().flex().flex_col().flex_1().min_h(px(0.));
        center = center.child(self.tab_bar.clone());
        let mut middle = div().flex().flex_row().flex_1().min_h(px(0.));
        middle = middle.child(self.terminal.clone());
        if sftp {
            middle = middle.child(
                div().w(px(SFTP_WIDTH)).min_w(px(SFTP_WIDTH)).child(self.sftp.clone()),
            );
        }
        center = center.child(middle);
        center = center.child(status_bar);

        // Body: sidebar + center.
        let mut body = div().flex().flex_row().flex_1().min_h(px(0.));
        if sidebar {
            body = body.child(
                div().w(px(SIDEBAR_WIDTH)).min_w(px(SIDEBAR_WIDTH)).child(self.host_list.clone()),
            );
        }
        body = body.child(center);

        div().id("app-shell")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_sftp))
            .on_action(cx.listener(Self::on_close_active_tab))
            .child(body)
    }
}
