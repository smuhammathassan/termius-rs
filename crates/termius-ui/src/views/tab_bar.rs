//! tab_bar — the open-session (connection) tabs.
//!
//! Port of the original top-strip terminal tab (`F7`/`zN`, `00-shell.md` §10):
//! `min-height: 30px`, `padding: 0 10px 0 5px`, `border-radius:
//! var(--corner-radius-medium)`, a leading glyph that swaps to a square close
//! button, and `terminalTabTokens` hover/selected fills. The strip is embedded
//! in the shell's absolute top strip (see [`super::top_bar`]); it carries no
//! background of its own so it sits flush on the title strip.
//!
//! One tab per [`Session`](crate::app_state::Session): a status dot, the label
//! (OSC title when the remote set one, else the host label) and a close
//! affordance, plus a trailing "＋" that opens the Add Host dialog.
//!
//! Closing is also reachable with ⌘W via the shell's `CloseActiveTab` action.

use gpui::{
    div, px, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::app_state::{Dialog, SessionStatus, TermiusState};
use crate::assets::icon;
use crate::theme::{text, theme_of, TermiusTheme};

use super::top_bar::{self, TAB_HEIGHT};

/// Width a selected session tab grows to (`--selected-tab-width` feel).
const SELECTED_TAB_WIDTH: f32 = 140.0;
/// Minimum session-tab width.
const MIN_TAB_WIDTH: f32 = 80.0;
/// Square close-button size (`tabActionButton`: 23×23).
const CLOSE_BUTTON_SIZE: f32 = 23.0;
/// Glyph inside the close button.
const CLOSE_GLYPH_SIZE: f32 = 11.0;
/// Diameter of the leading status dot.
const STATUS_DOT_SIZE: f32 = 7.0;

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
            .gap(px(5.))
            .h(px(TAB_HEIGHT))
            .min_w(px(0.))
            .overflow_hidden();

        for (id, label, status) in tabs {
            let is_active = active.as_deref() == Some(id.as_str());
            let fg = if is_active { theme.title } else { theme.muted };
            let dot = status_color(&status, &theme);
            let select_id = id.clone();
            let hover_fill = if is_active { theme.tab_active } else { theme.hover };

            let mut tab = div()
                .id(SharedString::from(format!("tab-{id}")))
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(TAB_HEIGHT))
                .min_w(px(MIN_TAB_WIDTH))
                .max_w(px(SELECTED_TAB_WIDTH))
                .pl(px(5.))
                .pr(px(10.))
                .rounded(px(theme.corner_radius_medium))
                .cursor_pointer()
                .overflow_hidden()
                .text_color(fg);
            if is_active {
                tab = tab.bg(theme.tab_active).min_w(px(SELECTED_TAB_WIDTH));
            }

            tab = tab
                .child(
                    div()
                        .flex_shrink_0()
                        .size(px(STATUS_DOT_SIZE))
                        .rounded(px(STATUS_DOT_SIZE / 2.0))
                        .bg(dot),
                )
                .child(
                    text::R12P
                        .style(div())
                        .flex_1()
                        .min_w(px(0.))
                        .text_color(fg)
                        .truncate()
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{id}")))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .size(px(CLOSE_BUTTON_SIZE))
                        .rounded(px(theme.corner_radius_small))
                        .cursor_pointer()
                        .text_color(fg)
                        .hover(move |style| style.bg(theme.hover))
                        .child(
                            icon("closeIcon.svg")
                                .w(px(CLOSE_GLYPH_SIZE))
                                .h(px(CLOSE_GLYPH_SIZE))
                                .text_color(fg),
                        )
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            cx.stop_propagation();
                            this.close(id.clone(), cx);
                        })),
                )
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.select(select_id.clone(), cx);
                }));

            strip = strip.child(tab.hover(move |style| style.bg(hover_fill)));
        }

        // Trailing "＋": open a new connection (the Add Host form).
        strip = strip.child(
            top_bar::icon_button(theme, "session-tab-new", "plusThin.svg", false)
                .on_click(cx.listener(|this, _event, _window, cx| this.new_host(cx))),
        );

        strip
    }
}
