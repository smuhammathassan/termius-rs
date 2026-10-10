//! tab_bar — the open-session (connection) tabs.
//!
//! One tab per [`Session`](crate::app_state::Session): a status dot, the label
//! (OSC title when the remote set one, else the host label) and a close
//! affordance, plus a trailing "＋" that opens the Add Host dialog. The strip
//! is embedded in the shell's top bar (see [`super::top_bar`]); it carries no
//! background of its own so it sits flush on the title strip.
//!
//! Closing is also reachable with ⌘W via the shell's `CloseActiveTab` action.

use gpui::{
    div, px, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::app_state::{Dialog, SessionStatus, TermiusState};
use crate::assets::icon;
use crate::theme::{text, theme_of, TermiusTheme};

use super::top_bar::{self, TOP_BAR_HEIGHT};

/// The tab strip view.
pub struct TabBar {
    state: Entity<TermiusState>,
}

impl TabBar {
    pub fn new(state: Entity<TermiusState>) -> Self {
        Self { state }
    }

    fn select(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.select_session(&session_id, cx));
    }

    fn close(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.close_session(&session_id, cx));
    }

    /// The trailing "＋" affordance: open the New Host form.
    fn new_host(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.open_dialog(Dialog::AddHost, cx));
    }
}

/// The status dot color for a session's lifecycle.
pub(crate) fn status_color(status: &SessionStatus, theme: &TermiusTheme) -> gpui::Rgba {
    match status {
        SessionStatus::Connecting => theme.muted,
        SessionStatus::Ready => theme.success,
        SessionStatus::Closed { .. } => theme.muted,
        SessionStatus::Failed { .. } => theme.danger,
    }
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let (tabs, active) = {
            let state = self.state.read(cx);
            let tabs: Vec<(String, String, SessionStatus)> = state
                .sessions
                .iter()
                .map(|session| (session.id.clone(), session.tab_label(), session.status.clone()))
                .collect();
            (tabs, state.active_session.clone())
        };

        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .h(px(TOP_BAR_HEIGHT))
            .min_w(px(0.))
            .overflow_hidden();

        for (id, label, status) in tabs {
            let is_active = active.as_deref() == Some(id.as_str());
            let fg = if is_active { theme.title } else { theme.muted };
            let dot = status_color(&status, &theme);
            let select_id = id.clone();

            let mut tab = div()
                .id(SharedString::from(format!("tab-{id}")))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(28.))
                .px(px(10.))
                .rounded(px(theme.corner_radius_small))
                .cursor_pointer()
                .text_color(fg);
            if is_active {
                tab = tab.bg(theme.tab_active);
            }

            tab = tab
                .child(div().size(px(7.)).rounded(px(3.5)).bg(dot))
                .child(
                    text::R12P
                        .style(div())
                        .text_color(fg)
                        .whitespace_nowrap()
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{id}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(14.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .text_color(theme.muted)
                        .child(icon("closeIcon.svg").w(px(10.)).h(px(10.)).text_color(theme.muted))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            cx.stop_propagation();
                            this.close(id.clone(), cx);
                        })),
                )
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.select(select_id.clone(), cx);
                }));

            strip = strip.child(tab);
        }

        // Trailing "＋": open a new connection (the Add Host form).
        strip = strip.child(
            top_bar::icon_button(theme, "session-tab-new", "addCircle.svg", false)
                .on_click(cx.listener(|this, _event, _window, cx| this.new_host(cx))),
        );

        strip
    }
}
