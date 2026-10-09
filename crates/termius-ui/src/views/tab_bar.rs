//! tab_bar — the open-session tab strip.
//!
//! One tab per [`Session`](crate::app_state::Session): status dot, label, and
//! a close affordance (also reachable with ⌘W via the shell's `CloseActiveTab`).

use gpui::{
    div, px, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::app_state::{SessionStatus, TermiusState};
use crate::theme::{theme_of, TermiusTheme};

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
            .h(px(super::app_shell::TAB_BAR_HEIGHT))
            .px(px(6.))
            .gap(px(4.))
            .bg(theme.tab_background)
            .border_b_1()
            .border_color(theme.border);

        if tabs.is_empty() {
            strip = strip.child(
                div()
                    .px(px(8.))
                    .text_color(theme.muted)
                    .child(SharedString::from("No open sessions")),
            );
        }

        for (id, label, status) in tabs {
            let is_active = active.as_deref() == Some(id.as_str());
            let dot = status_color(&status, &theme);
            let select_id = id.clone();

            let mut tab = div()
                .id(SharedString::from(format!("tab-{id}")))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded(px(4.))
                .text_color(if is_active { theme.foreground } else { theme.muted });
            if is_active {
                tab = tab.bg(theme.tab_active);
            }

            tab = tab
                .child(div().size(px(7.)).rounded(px(3.5)).bg(dot))
                .child(SharedString::from(label))
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{id}")))
                        .px(px(3.))
                        .text_color(theme.muted)
                        .child(SharedString::from("✕"))
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

        strip
    }
}
