//! left_panel — the 185px vertical section nav (`pane1`).
//!
//! Port of the original's left panel (`dFe` / `lFe` / `aFe`, `00-shell.md`
//! §6–§7): a **column** of 36px icon+label rows, one per `leftPanelTabs` entry
//! (`baseTabs` minus SFTP), plus the trial-promo host (`oFe`) pinned at the
//! bottom.
//!
//! The rows are [`crate::primitives::NavItem`] — the `aFe` port: a full-width 36px row
//! (`width: calc(100% - 20px)`, `margin: 8px 10px`,
//! `border-radius: var(--corner-radius-medium)`) with a 14px label, a selected
//! fill and a hover wash (`--background-selected-color` /
//! `--background-hover-color`).
//!
//! The panel is its own view entity so it can own its click listeners and
//! repaint the selected row without rebuilding the whole shell.

use gpui::{
    div, px, Context, Entity, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::app_state::TermiusState;
use crate::navigation::{sidebar_items, Section};
use crate::primitives::{Button, NavItem};
use crate::theme::{text, theme_of, TermiusTheme};

/// Width of the left nav rail (`pane1.visible`: `185px`, `_main.js:112361`).
pub const LEFT_PANEL_WIDTH: f32 = 185.0;
/// Width of the icon-only rail (`@media (max-width: 800px)`: `68px`).
pub const LEFT_PANEL_COMPACT_WIDTH: f32 = 68.0;

/// The vertical section rail (`pane1`).
pub struct LeftPanel {
    state: Entity<TermiusState>,
    /// Re-render when the routed section changes.
    _observe: Subscription,
}

impl LeftPanel {
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        let _observe = cx.observe(&state, |_, _, cx| cx.notify());
        Self { state, _observe }
    }

    fn select(&mut self, section: Section, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_section(section, cx));
    }
}

/// The bottom trial-promo host (`oFe`): the in-app "Pro" upsell card.
fn trial_promo(theme: TermiusTheme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .m(px(10.))
        .p(px(12.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.card_a)
        .text_color(theme.title)
        .child(text::B12P.style(div()).child("Termius Pro"))
        .child(
            text::R10S
                .style(div())
                .text_color(theme.text_common)
                .whitespace_normal()
                .child("Sync hosts, keys and teams across your devices."),
        )
        .child(Button::new("Upgrade").primary().element(theme).w_full())
}

impl Render for LeftPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let current = self.state.read(cx).current_section;

        // `dFe.tabsContainer`: a vertical, scrolling list (`padding-top: 2px`).
        let mut list = div()
            .id("left-panel-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .pt(px(2.));

        for section in sidebar_items() {
            let selected = *section == current;
            let target = *section;
            list = list.child(
                NavItem::new(section.label(), section.icon(), selected)
                    .element(theme)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.select(target, cx)
                    })),
            );
        }

        // `dFe.root`: a column with a right hairline divider; the promo host
        // (`oFe`) sits below the scrolling list.
        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar_background)
            .text_color(theme.foreground)
            .border_r_1()
            .border_color(theme.border)
            .child(list)
            .child(trial_promo(theme))
    }
}
