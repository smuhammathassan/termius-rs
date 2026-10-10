//! top_bar — the absolute 51px application title strip.
//!
//! Port of the original Electron shell's top tab strip (`00-shell.md` §10):
//! `BDe` renders `yDe` (= `TitleBar`, class `horizontalTabs`), which is
//! `position: absolute; top: 0; height: var(--horizontal-tabs-height)` = **51px**,
//! padded `11px 10px 10px 9px` with a `1px` bottom border, and overlays the
//! normal-flow 51px spacer `qDe`.
//!
//! Its children, left to right: the macOS traffic-light spacer (`sd_1`), the
//! fixed **Vault · SFTP** shortcut tabs (`RDe`), the connection/terminal tabs
//! (`ADe` → `hDe`, supplied here by [`super::TabBar`]), and the right-side
//! cluster (`zDe`: update pill · notification bell · window actions).
//!
//! The builders here return plain styled elements so [`super::app_shell`] can
//! wire them with its own `cx.listener` callbacks.

use gpui::{
    div, px, Div, InteractiveElement as _, ParentElement as _, SharedString, Stateful, Styled as _,
};

use crate::assets::icon;
use crate::theme::{text, TermiusTheme};

/// Height of the title strip — `var(--horizontal-tabs-height)` (`51px`).
pub const TOP_BAR_HEIGHT: f32 = 51.0;
/// Leading spacer reserved for the macOS window traffic lights.
pub const TRAFFIC_LIGHT_WIDTH: f32 = 78.0;
/// Square icon-button size in the top bar's right cluster.
pub const ICON_BUTTON_SIZE: f32 = 28.0;
/// Height of a top-strip tab (`zN` root: `min-height: 30px`).
pub const TAB_HEIGHT: f32 = 30.0;
/// Minimum top-strip tab width (the original `TDe` Vault/SFTP tab floor).
pub const MIN_TAB_WIDTH: f32 = 80.0;
/// Width a selected top-strip tab grows to (`TDe.selected`: `140px`).
pub const SELECTED_TAB_WIDTH: f32 = 140.0;
/// Glyph size inside a top-strip tab (`& svg { max-width/height: 14px }`).
const TAB_ICON_SIZE: f32 = 14.0;
/// Glyph size inside a right-cluster icon button.
const CLUSTER_ICON_SIZE: f32 = 16.0;
/// Font size of a shortcut-tab label.
const TAB_LABEL_SIZE: f32 = 14.0;

/// A fixed top-strip shortcut tab (the Vault · SFTP pair, `RDe`/`TDe`).
///
/// `min-width: 80px`, `height: 30px`, `border-radius:
/// var(--corner-radius-medium)`, an icon beside a 14px label, and a selected
/// fill (`terminalTabTokens.backgroundColorSelected`). Returns a `Stateful<Div>`
/// so the caller chains `.on_click(...)` with its own listener.
pub fn shortcut_tab(
    theme: TermiusTheme,
    id: impl Into<SharedString>,
    label: &'static str,
    icon_name: &'static str,
    active: bool,
) -> Stateful<Div> {
    let color = if active { theme.title } else { theme.muted };
    let hover_fill = if active { theme.tab_active } else { theme.hover };
    let label = SharedString::from(label);

    let mut tab = div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(TAB_HEIGHT))
        .min_w(px(MIN_TAB_WIDTH))
        .px(px(10.))
        .rounded(px(theme.corner_radius_medium))
        .cursor_pointer()
        .text_color(color);
    if active {
        tab = tab.bg(theme.tab_active);
    }
    tab.child(icon(icon_name).w(px(TAB_ICON_SIZE)).h(px(TAB_ICON_SIZE)).text_color(color))
        .child(
            text::R14P
                .style(div())
                .text_size(px(TAB_LABEL_SIZE))
                .text_color(color)
                .whitespace_nowrap()
                .child(label),
        )
        .hover(move |style| style.bg(hover_fill))
}

/// A square top-strip icon button (bell / key / gear / person / team / theme).
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
        .child(icon(icon_name).w(px(CLUSTER_ICON_SIZE)).h(px(CLUSTER_ICON_SIZE)).text_color(color));
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
        let _ = shortcut_tab(theme, "vault-tab", "Vault", "Vault.svg", true);
        let _ = shortcut_tab(theme, "sftp-tab", "SFTP", "Sftp.svg", false);
        let _ = icon_button(theme, "topbar-bell", "bell.svg", false);
        let _ = icon_button(theme, "topbar-gear", "gear.svg", true);
        let _ = update_pill(theme);
    }
}
