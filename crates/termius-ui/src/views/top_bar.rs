//! top_bar — the full-width application title strip.
//!
//! Port of the original Electron shell's top tab strip (`_main.js`,
//! `ConnectedTwoPanelMode` / `useDx` around line 110300): a leading spacer for
//! the macOS traffic lights, the page tabs (`iconType` icon + label, the active
//! one highlighted), then the connection/session tabs, then a right cluster
//! with the "Update" pill, notification bell and settings gear.
//!
//! The builders here return plain styled elements so [`super::app_shell`] can
//! wire them with its own `cx.listener` callbacks (the [`super::TabBar`] entity
//! supplies the session tabs + "＋" in the middle of the strip).

use gpui::{
    div, px, Div, InteractiveElement as _, ParentElement as _, SharedString, Stateful, Styled as _,
};

use crate::assets::icon;
use crate::navigation::Section;
use crate::theme::{text, TermiusTheme};

/// Height of the application top bar (the original title strip is ~44px).
pub const TOP_BAR_HEIGHT: f32 = 44.0;
/// Leading spacer reserved for the macOS window traffic lights.
pub const TRAFFIC_LIGHT_WIDTH: f32 = 78.0;
/// Square icon-button size in the top bar's right cluster.
pub const ICON_BUTTON_SIZE: f32 = 28.0;
/// Top-bar glyph size (the original caps tab icons at 14px; 16 reads better
/// against the CircularXX labels at the same row height).
const ICON_SIZE: f32 = 16.0;

/// One page-tab button: its section icon beside the label, highlighted when the
/// section is the routed one.
///
/// Returns a `Stateful<Div>` so the caller chains `.on_click(...)` with its own
/// listener (`StatefulInteractiveElement` must be in scope there).
pub fn page_tab(theme: TermiusTheme, section: Section, active: bool) -> Stateful<Div> {
    let color = if active { theme.title } else { theme.muted };
    let label = SharedString::from(section.label());
    let id = SharedString::from(format!("page-tab-{}", section.label()));
    let hover_fill = if active { theme.tab_active } else { theme.hover };

    let mut tab = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(28.))
        .px(px(10.))
        .rounded(px(theme.corner_radius_small))
        .cursor_pointer()
        .text_color(color);
    if active {
        tab = tab.bg(theme.tab_active);
    }
    tab.child(icon(section.icon()).w(px(ICON_SIZE)).h(px(ICON_SIZE)).text_color(color))
        .child(text::R12P.style(div()).text_color(color).whitespace_nowrap().child(label))
        .hover(move |style| style.bg(hover_fill))
}

/// A square top-bar icon button (bell / gear / theme toggle).
///
/// `active` paints the routed highlight (the gear lights up on Settings).
/// Returns a `Stateful<Div>`; chain `.on_click(...)` where an action exists.
pub fn icon_button(
    theme: TermiusTheme,
    id: impl Into<SharedString>,
    icon_name: &str,
    active: bool,
) -> Stateful<Div> {
    let color = if active { theme.title } else { theme.muted };
    let hover_fill = if active { theme.tab_active } else { theme.hover };

    let mut button = div()
        .id(id.into())
        .flex()
        .items_center()
        .justify_center()
        .size(px(ICON_BUTTON_SIZE))
        .rounded(px(theme.corner_radius_small))
        .cursor_pointer()
        .text_color(color)
        .child(icon(icon_name).w(px(ICON_SIZE)).h(px(ICON_SIZE)).text_color(color));
    if active {
        button = button.bg(theme.tab_active);
    }
    button.hover(move |style| style.bg(hover_fill))
}

/// The "Update" chip in the right cluster (a quiet pill — no action is wired
/// until the updater lands, so it is a plain element, not a dead button).
pub fn update_pill(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(20.))
        .px(px(8.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(theme.border_basic)
        .bg(theme.card_c)
        .child(
            text::R12P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from("Update")),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_build_against_the_theme_without_a_window() {
        let theme = TermiusTheme::dark();
        let _ = page_tab(theme, Section::Hosts, true);
        let _ = page_tab(theme, Section::Sftp, false);
        let _ = icon_button(theme, "topbar-bell", "bell.svg", false);
        let _ = icon_button(theme, "topbar-gear", "gear.svg", true);
        let _ = update_pill(theme);
    }
}
