//! team_screen — the Team section: the team card (name + member list +
//! copy-invitation link) and the Security section.
//!
//! Port of the Termius desktop Team settings tab (`rt` = `Ve`,
//! `assets/index-1baf28ac.js`; see `analysis/recon/31-account-team.md` §2). The
//! chrome is driven by the recovered original components:
//!
//! * **team card** ← `index-1baf28ac.js:548-588` — a `SettingsSection`
//!   (`className=teamContainer`, `gap: 20px`) holding the team-name row
//!   (`subtitle1`, `bolder`, ellipsis) with a **`Manage` link** +
//!   `openExternal.svg` (`:562-579`), the member list (`useZe`), and the
//!   **copy-invitation-link** (`Pe`/`Fe`, `:44-162`).
//! * **member rows** ← `UserListItem-958a4b71.js` — `flex-row`, `gap: 10px`,
//!   `min-height: 60px`, `padding: 10px`, radius large: a leading avatar tile,
//!   the name (`R14P`) with the owner (`crownIcon.svg`) / `YOU` badges over a
//!   secondary line (`R12S`), and the trailing `Active` / `Pending access`
//!   status (`:397-412`).
//! * **Security section** ← `index-1baf28ac.js:589-621` — a `SettingsSection`
//!   "Security" with the "Multiplayer **Beta**" and "Require 2FA for all team
//!   members" rows, each an `Enabled` / `Disabled` dropdown (`useA`,
//!   `:168-225`).
//!
//! The **Vaults** card and the **Sharing** note that used to live here belong
//! to the separate `/vaults` tab (`Vaults-ef2e0d94.js`) and are intentionally
//! gone (see `analysis/recon/PARITY.md` §Settings/Account/Team).
//!
//! [`team_screen`] builds a small self-contained view (GPUI renders the
//! returned entity as a child, exactly like `HostList` / `SettingsScreen`).
//!
//! # PORT-TODOs
//!
//! * `TermiusState` has no team/member slice, so the member list shows only
//!   the signed-in owner (derived from [`AccountInfo`]); flip [`team_members`]
//!   to `state.read(cx).team_members` once the slice lands. The real team name
//!   (`bm(fZ)`) and pending invites (`wF`/`BC`) are likewise absent.
//! * `Multiplayer` / `Require 2FA` are view-local toggles; the original calls
//!   `eQ(n, BA, { [name]: d })` (`realtime_collaboration` / `two_factor_auth`).
//! * `Manage` opens the external team-management flow and `Copy invitation
//!   link` copies to the clipboard — both stubbed behind the status bar until
//!   the termius-sync wave.

use gpui::{
    div, px, AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::app_state::{AccountInfo, TermiusState};
use crate::primitives::{EmptyState, SettingsSection};
use crate::theme::{text, theme_of, TermiusTheme, ThemeMode};

/// Height of an empty placeholder card body.
const EMPTY_HEIGHT: f32 = 160.0;
/// The leading avatar tile (`UserListItem` `iconSize` `extraLarge`).
const AVATAR_SIZE: f32 = 40.0;
/// Glyph inside the avatar tile.
const AVATAR_GLYPH: f32 = 20.0;
/// Minimum height of a member row (`UserListItem` `minHeight: 60px`).
const ROW_MIN_HEIGHT: f32 = 60.0;
/// The team name shown on the card (the wizard's `teamName: "My Team"`).
const TEAM_NAME: &str = "My Team";
/// Copy-invitation-link labels (`Pe`/`Fe`, `:44-162`).
const COPY_LINK: &str = "Copy invitation link";
const COPY_LINK_DONE: &str = "Invitation link copied!";
/// The `Manage` link label (`index-1baf28ac.js:567-571`).
const MANAGE: &str = "Manage";
/// The team-name row's icon.
const TEAM_ICON: &str = "team.svg";
/// The statuses the member list shows (`index-1baf28ac.js:397-412`).
const STATUS_ACTIVE: &str = "Active";
const STATUS_PENDING: &str = "Pending access";

// ---------------------------------------------------------------------------
// Pure row helpers (unit-tested below)
// ---------------------------------------------------------------------------

/// One member row the member list renders.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemberRow {
    id: String,
    /// The name line (`UserListItem` `full_name || email`).
    name: String,
    /// The secondary line under the name.
    secondary: String,
    /// The role tag ("Owner" / "Editor" / "Member").
    role: &'static str,
    /// The trailing status ([`STATUS_ACTIVE`] / [`STATUS_PENDING`]).
    status: &'static str,
    /// Whether this member is the signed-in user (drives the `YOU` badge).
    is_current: bool,
    /// Whether the row is a pending invite (drives the resend affordance).
    pending: bool,
}

impl MemberRow {
    /// The role tag shown in the owner badge.
    fn role_label(&self) -> &'static str {
        self.role
    }
}

/// The name line for the signed-in account (`full_name || email`, falling
/// back to a generic label when the stub carries no email).
fn member_name(account: &AccountInfo) -> String {
    let email = account.email.trim();
    if email.is_empty() {
        "You".to_owned()
    } else {
        email.to_owned()
    }
}

/// The team name for the card (a real team slice has not landed yet).
fn team_name(account: Option<&AccountInfo>) -> &'static str {
    match account {
        Some(_) => TEAM_NAME,
        None => "No team",
    }
}

/// The members this screen lists.
///
/// PORT-TODO: `TermiusState` carries no team/member slice, so the list holds
/// only the signed-in owner (derived from [`AccountInfo`]); flip this to
/// `state.read(cx).team_members` once the slice lands.
fn team_members(account: Option<&AccountInfo>) -> Vec<MemberRow> {
    match account {
        Some(account) => vec![MemberRow {
            id: "owner".to_owned(),
            name: member_name(account),
            secondary: format!("{} plan", account.plan),
            role: "Owner",
            status: STATUS_ACTIVE,
            is_current: true,
            pending: false,
        }],
        None => Vec::new(),
    }
}

/// The copy-invitation-link label for the current state (`Fe` states).
fn copy_link_label(copied: bool) -> &'static str {
    if copied {
        COPY_LINK_DONE
    } else {
        COPY_LINK
    }
}

/// The `Enabled` / `Disabled` value of a Security row (`useA`).
fn security_value(on: bool) -> &'static str {
    if on {
        "Enabled"
    } else {
        "Disabled"
    }
}

/// The Security section's view-local toggles (`useA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SecurityToggles {
    /// `realtime_collaboration` — the "Multiplayer Beta" row.
    multiplayer: bool,
    /// `two_factor_auth` — "Require 2FA for all team members".
    require_2fa: bool,
}

impl Default for SecurityToggles {
    fn default() -> Self {
        // Termius ships both off until the team owner enables them.
        Self {
            multiplayer: false,
            require_2fa: false,
        }
    }
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Team screen view (see the module docs).
pub struct TeamScreen {
    state: Entity<TermiusState>,
    /// The Security section's toggles (no team slice yet).
    security: SecurityToggles,
    /// Whether the invitation link was just copied (copy-link state machine).
    invite_link_copied: bool,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Team screen for `state`.
///
/// Returns the view entity as an element: the shell inserts it wherever the
/// Team section is routed. Store the result once (the same way `AppShell`
/// holds `HostList`) — calling this every render would mint a new entity per
/// frame. All state access happens in [`TeamScreen`]'s `Context<Self>` (see
/// the lease note on `views::account_screen::account_screen`).
pub fn team_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<TeamScreen> {
    cx.new(|cx| TeamScreen::new(state, cx))
}

impl TeamScreen {
    /// Create the screen and subscribe it to state + theme changes.
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let observe_theme = cx.observe_global::<TermiusTheme>(|_, cx| cx.notify());
        Self {
            state,
            security: SecurityToggles::default(),
            invite_link_copied: false,
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// Note a status-bar message through the shared state.
    fn note(&mut self, message: &str, cx: &mut Context<Self>) {
        let message = message.to_owned();
        self.state.update(cx, |state, cx| {
            state.status_text = message;
            cx.notify();
        });
    }

    /// Open the external team-management flow (`Manage`; sync wave).
    fn manage_team(&mut self, cx: &mut Context<Self>) {
        self.note(
            "Manage — the team-management flow opens in the sync wave.",
            cx,
        );
    }

    /// Copy the invitation link (clipboard lands with the sync wave).
    fn copy_invite_link(&mut self, cx: &mut Context<Self>) {
        self.invite_link_copied = true;
        self.note(COPY_LINK_DONE, cx);
    }

    /// Flip the "Multiplayer Beta" toggle (`realtime_collaboration`).
    fn toggle_multiplayer(&mut self, cx: &mut Context<Self>) {
        self.security.multiplayer = !self.security.multiplayer;
        let message = if self.security.multiplayer {
            "Multiplayer enabled for this team."
        } else {
            "Multiplayer disabled for this team."
        };
        self.note(message, cx);
    }

    /// Flip "Require 2FA for all team members" (`two_factor_auth`).
    fn toggle_require_2fa(&mut self, cx: &mut Context<Self>) {
        self.security.require_2fa = !self.security.require_2fa;
        let message = if self.security.require_2fa {
            "Two-factor authentication is now required for all team members."
        } else {
            "Two-factor authentication is no longer required for team members."
        };
        self.note(message, cx);
    }
}

impl Render for TeamScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let account = self.state.read(cx).account.clone();

        let mut column = div()
            .id("team-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .pb(px(60.))
            .overflow_y_scroll();

        if account.is_none() {
            column = column.child(
                section_card(theme).child(
                    div().h(px(EMPTY_HEIGHT)).child(
                        EmptyState::new(
                            "No team",
                            "Sign in to create a team and invite teammates.",
                        )
                        .element(theme),
                    ),
                ),
            );
        } else {
            let members = team_members(account.as_ref());
            column = column.child(team_card(theme, account.as_ref(), &members, self, cx));
            column = column.child(security_section(theme, self, cx));
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(column)
    }
}

// ---------------------------------------------------------------------------
// Card / row builders
// ---------------------------------------------------------------------------

/// The `SettingsSection` card chrome without a forced heading: `padding: 20px`,
/// `--card-a`, radius medium, `maxWidth: 700px`, centred with a `30px` top gap.
fn section_card(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .max_w(px(700.))
        .mx_auto()
        .mt(px(30.))
        .p(px(20.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.card_a)
        .text_color(theme.title)
}

/// The team card (`teamContainer`): the name row, the member list and the
/// copy-invitation link, stacked with a `20px` gap.
fn team_card(
    theme: TermiusTheme,
    account: Option<&AccountInfo>,
    members: &[MemberRow],
    this: &TeamScreen,
    cx: &mut Context<TeamScreen>,
) -> Div {
    let mut card = section_card(theme).gap(px(20.));
    card = card.child(team_name_row(theme, account, cx));

    if members.is_empty() {
        card = card.child(
            div().h(px(EMPTY_HEIGHT)).child(
                EmptyState::new("No members", "Invite teammates to collaborate.").element(theme),
            ),
        );
    } else {
        card = card.child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .child(members_header_row(theme))
                .children(members.iter().map(|member| member_row(theme, member))),
        );
    }

    card.child(copy_link_element(theme, this.invite_link_copied, cx))
}

/// The team card's top row: the team icon + name (ellipsis) and the `Manage`
/// external-link affordance (`index-1baf28ac.js:562-579`).
fn team_name_row(
    theme: TermiusTheme,
    account: Option<&AccountInfo>,
    cx: &mut Context<TeamScreen>,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w(px(0.))
                .child(
                    crate::icon(TEAM_ICON)
                        .w(px(16.))
                        .h(px(16.))
                        .text_color(theme.primary),
                )
                .child(
                    text::B14P
                        .style(div())
                        .truncate()
                        .text_color(theme.title)
                        .child(SharedString::from(team_name(account).to_owned())),
                ),
        )
        .child(
            div()
                .id("team-manage-link")
                .flex()
                .items_center()
                .gap(px(6.))
                .flex_shrink_0()
                .cursor_pointer()
                .text_color(theme.primary)
                .child(
                    text::R12P
                        .style(div())
                        .font_weight(FontWeight::MEDIUM)
                        .child(SharedString::from(MANAGE)),
                )
                .child(
                    crate::icon("openExternal.svg")
                        .w(px(12.))
                        .h(px(12.))
                        .text_color(theme.primary),
                )
                .hover(move |style| style.text_color(theme.primary_light))
                .on_click(cx.listener(|this, _event, _window, cx| this.manage_team(cx))),
        )
}

/// The `Member` / `Status` header row above the member list
/// (`--card-b` light / `--card-c` dark, `padding: 10px 15px`, radius medium).
fn members_header_row(theme: TermiusTheme) -> Div {
    let fill = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(15.))
        .py(px(10.))
        .rounded(px(theme.corner_radius_medium))
        .bg(fill)
        .child(
            text::B14P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from("Member")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .min_w(px(108.))
                .pl(px(10.))
                .child(
                    text::B14P
                        .style(div())
                        .text_color(theme.title)
                        .child(SharedString::from("Status")),
                ),
        )
}

/// The leading round avatar tile (`UserListItem`'s `Avatar`).
fn avatar_tile(theme: TermiusTheme, icon_name: &str) -> Div {
    let fill = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(AVATAR_SIZE))
        .h(px(AVATAR_SIZE))
        .flex_shrink_0()
        .rounded(px(AVATAR_SIZE / 2.0))
        .bg(fill)
        .text_color(theme.muted)
        .child(
            crate::icon(icon_name)
                .w(px(AVATAR_GLYPH))
                .h(px(AVATAR_GLYPH)),
        )
}

/// A small role badge pill (the `--gradient-light-main` owner tag; approximated
/// with the accent wash `theme.hover`).
fn role_badge(theme: TermiusTheme, label: SharedString) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .px(px(6.))
        .h(px(15.))
        .flex_shrink_0()
        .rounded(px(4.))
        .bg(theme.hover)
        .text_color(theme.primary)
        .child(crate::icon("crownIcon.svg").w(px(10.)).h(px(10.)))
        .child(
            div()
                .text_size(px(9.))
                .font_weight(FontWeight::BOLD)
                .line_height(px(10.))
                .child(label),
        )
}

/// The tiny `YOU` pill marking the current user (`fontSize: 7`).
fn you_badge(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .px(px(6.))
        .h(px(15.))
        .flex_shrink_0()
        .rounded(px(4.))
        .bg(theme.hover)
        .text_color(theme.primary)
        .child(
            div()
                .text_size(px(7.))
                .font_weight(FontWeight::BOLD)
                .line_height(px(10.))
                .child(SharedString::from("YOU")),
        )
}

/// One member row (`UserListItem-958a4b71.js`): avatar, name with the owner /
/// `YOU` badges over a secondary line, and the trailing status.
fn member_row(theme: TermiusTheme, member: &MemberRow) -> Stateful<Div> {
    let hover_fill = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
    let mut name_line = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .min_w(px(0.))
        .child(
            text::R14P
                .style(div())
                .truncate()
                .text_color(theme.title)
                .child(SharedString::from(member.name.clone())),
        )
        .child(role_badge(theme, SharedString::from(member.role_label())));
    if member.is_current {
        name_line = name_line.child(you_badge(theme));
    }

    let mut status = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .flex_shrink_0()
        .min_w(px(108.))
        .pl(px(10.))
        .child(
            text::R12S
                .style(div())
                .text_color(theme.text_common)
                .child(SharedString::from(member.status)),
        );
    if member.pending {
        status = status.child(
            text::R12P
                .style(div())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.primary)
                .child(SharedString::from("Resend invite")),
        );
    }

    div()
        .id(SharedString::from(format!("team-member-{}", member.id)))
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .min_h(px(ROW_MIN_HEIGHT))
        .p(px(10.))
        .rounded(px(theme.corner_radius_large))
        .hover(move |hover| hover.bg(hover_fill))
        .child(avatar_tile(theme, "person.svg"))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(name_line)
                .child(
                    text::R12S
                        .style(div())
                        .truncate()
                        .text_color(theme.text_common)
                        .child(SharedString::from(member.secondary.clone())),
                ),
        )
        .child(status)
}

/// The copy-invitation-link affordance (`Pe`/`Fe`): a 200×24 accent link with
/// a default / success state.
fn copy_link_element(
    theme: TermiusTheme,
    copied: bool,
    cx: &mut Context<TeamScreen>,
) -> Stateful<Div> {
    div()
        .id("team-copy-invite")
        .flex()
        .items_center()
        .h(px(24.))
        .w(px(200.))
        .flex_shrink_0()
        .cursor_pointer()
        .text_color(theme.primary)
        .child(
            text::R14P
                .style(div())
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(copy_link_label(copied))),
        )
        .hover(move |style| style.text_color(theme.primary_light))
        .on_click(cx.listener(|this, _event, _window, cx| this.copy_invite_link(cx)))
}

/// The **Security** section (`index-1baf28ac.js:589-621`): "Multiplayer Beta"
/// and "Require 2FA for all team members".
fn security_section(
    theme: TermiusTheme,
    this: &TeamScreen,
    cx: &mut Context<TeamScreen>,
) -> Div {
    SettingsSection::new("Security")
        .child(security_row(
            theme,
            "team-security-multiplayer",
            "Multiplayer",
            true,
            this.security.multiplayer,
            cx.listener(|this, _event, _window, cx| this.toggle_multiplayer(cx)),
        ))
        .child(security_row(
            theme,
            "team-security-2fa",
            "Require 2FA for all team members",
            false,
            this.security.require_2fa,
            cx.listener(|this, _event, _window, cx| this.toggle_require_2fa(cx)),
        ))
        .element(theme)
}

/// One Security row (`useA`): a label (with an optional `Beta` badge) on the
/// left and an `Enabled` / `Disabled` dropdown on the right.
fn security_row(
    theme: TermiusTheme,
    id: &'static str,
    label: &'static str,
    beta: bool,
    on: bool,
    listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let mut label_row = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            text::R12S
                .style(div())
                .font_weight(FontWeight(450.0))
                .text_color(theme.text_common)
                .child(SharedString::from(label)),
        );
    if beta {
        label_row = label_row.child(beta_badge(theme));
    }

    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .mt(px(15.))
        .cursor_pointer()
        .child(label_row)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    text::R12P
                        .style(div())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.title)
                        .child(SharedString::from(security_value(on))),
                )
                .child(
                    crate::icon("allSettingsChevron.svg")
                        .w(px(15.))
                        .h(px(6.))
                        .text_color(theme.muted),
                ),
        )
        .on_click(listener)
}

/// The uppercase "Beta" chip the Multiplayer row carries.
fn beta_badge(theme: TermiusTheme) -> Div {
    text::R10S
        .style(div())
        .font_weight(FontWeight::BOLD)
        .px(px(5.))
        .py(px(2.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme.text_common)
        .text_color(theme.text_common)
        .child(SharedString::from("BETA"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: &str, plan: &str) -> AccountInfo {
        AccountInfo {
            email: email.to_owned(),
            plan: plan.to_owned(),
            device_count: 1,
        }
    }

    #[test]
    fn signed_out_has_no_members() {
        assert!(team_members(None).is_empty());
        assert_eq!(team_name(None), "No team");
    }

    #[test]
    fn the_signed_in_account_is_the_owner() {
        let account = account("dev@example.com", "Team");
        let members = team_members(Some(&account));
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "dev@example.com");
        assert_eq!(members[0].secondary, "Team plan");
        assert_eq!(members[0].role, "Owner");
        assert_eq!(members[0].status, STATUS_ACTIVE);
        assert!(members[0].is_current);
        assert!(!members[0].pending);
        assert_eq!(team_name(Some(&account)), TEAM_NAME);
    }

    #[test]
    fn member_name_falls_back_for_a_blank_email() {
        assert_eq!(member_name(&account("", "Free")), "You");
        assert_eq!(member_name(&account("  ", "Free")), "You");
    }

    #[test]
    fn copy_link_and_security_states() {
        assert_eq!(copy_link_label(false), COPY_LINK);
        assert_eq!(copy_link_label(true), COPY_LINK_DONE);
        assert_eq!(security_value(true), "Enabled");
        assert_eq!(security_value(false), "Disabled");
    }

    #[test]
    fn security_toggles_default_off() {
        let toggles = SecurityToggles::default();
        assert!(!toggles.multiplayer);
        assert!(!toggles.require_2fa);
    }
}
