//! team_screen — the Team section: the team card (name + member rows), the
//! vault list ("clusters" in Termius), the Invite-members action and the
//! sharing note.
//!
//! Port of the Termius desktop Team screen. The chrome is driven by the
//! recovered original components:
//!
//! * **team card** ← `index-1baf28ac.js` (the Team settings tab) — a card with
//!   the team name (`subtitle1`, `bolder`, ellipsis) + a `Manage` link, over a
//!   `Member` / `Status` header row (`--card-b` light / `--card-c` dark,
//!   `padding: 10px 15px`, radius medium) and the member list.
//! * **member rows** ← `UserListItem-958a4b71.js` — `flex-row`, `gap: 10px`,
//!   `min-height: 60px`, `padding: 10px`, radius large: a leading avatar tile,
//!   the name (`R14P`) with the owner (`crownIcon.svg`) / `YOU` badges over a
//!   secondary line (`R12S`), and the trailing status.
//! * **Invite members** ← `CreateTeam-ddcd7efb.js` /
//!   `InviteMembersTrialOnboarding-1dfbe173.js` — "Invite your team" opens the
//!   invite flow; the sync is stubbed until the termius-sync wave.
//!
//! Every signed-in account has a personal Default vault plus (for teams)
//! shared vaults; the vault card lists them, raises the new-vault
//! confirmation, and explains sharing.
//!
//! [`team_screen`] builds a small self-contained view (GPUI renders the
//! returned entity as a child, exactly like `HostList` / `SettingsScreen`).
//!
//! PORT-TODO: `TermiusState` has no `vaults: Vec<Vault>` slice yet — the
//! `vaults` storage table (`termius_storage`) and [`Vault`] domain type
//! already exist, but the state field has not landed, so the list renders
//! empty and the screen shows its [`EmptyState`]. [`team_vaults`] is the
//! single read site to flip once the slice arrives. There is likewise no team
//! / member slice, so the member list shows only the signed-in owner (derived
//! from [`AccountInfo`]).

use termius_core::Vault;

use gpui::{
    div, px, AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window,
};

use crate::app_state::{AccountInfo, Dialog, TermiusState};
use crate::primitives::{
    Button, EmptyState, ListItem, SectionHeader, SettingsSection, SettingsText, SettingsTitle,
};
use crate::theme::{text, theme_of, TermiusTheme, ThemeMode};

/// Height of an empty placeholder card body.
const EMPTY_HEIGHT: f32 = 160.0;
/// Column width for this settings-style page (`SettingsSection` `maxWidth`).
const COLUMN_WIDTH: f32 = 700.0;
/// The leading avatar tile (`UserListItem` `iconSize` `extraLarge`).
const AVATAR_SIZE: f32 = 40.0;
/// Glyph inside the avatar tile.
const AVATAR_GLYPH: f32 = 20.0;
/// Minimum height of a member row (`UserListItem` `minHeight: 60px`).
const ROW_MIN_HEIGHT: f32 = 60.0;
/// The Account header title.
const TITLE: &str = "Team";
/// The team name shown on the card (the wizard's `teamName: "My Team"`).
const TEAM_NAME: &str = "My Team";
/// The Sharing explainer under the vault list.
const SHARING_NOTE: &str = "Vaults you share with teammates appear here. Teammates \
                            get every host, key, and snippet stored in a shared \
                            vault — end-to-end encrypted like the rest of your library.";
/// What the New Vault confirmation says (creation syncs with the cloud).
const NEW_VAULT_NOTE: &str = "Vault creation syncs with the Termius cloud; it \
                               arrives with the termius-sync integration. This \
                               dialog confirms the action for now.";
/// What the Invite-members confirmation says (invites sync with the cloud).
const INVITE_NOTE: &str = "Inviting teammates syncs with the Termius cloud; it \
                           arrives with the termius-sync integration. This dialog \
                           confirms the action for now.";

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
    /// The trailing status ("Active" / "Pending access").
    status: &'static str,
    /// Whether this member is the signed-in user (drives the `YOU` badge).
    is_current: bool,
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
            status: "Active",
            is_current: true,
        }],
        None => Vec::new(),
    }
}

/// One vault row the list renders.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultRow {
    id: String,
    title: String,
    is_default: bool,
}

impl VaultRow {
    /// The muted right-hand tag: "Default" for the personal vault, "Team"
    /// for a shared/team vault (Termius' vault kinds).
    fn subtitle(&self) -> &'static str {
        if self.is_default {
            "Default"
        } else {
            "Team"
        }
    }
}

/// Map vaults to rows: the personal (default) vault first, then by title.
fn vault_rows(vaults: &[Vault]) -> Vec<VaultRow> {
    let mut rows: Vec<VaultRow> = vaults
        .iter()
        .map(|vault| VaultRow {
            id: vault.id.clone(),
            title: vault.title.clone(),
            is_default: vault.is_default,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.title.cmp(&b.title))
    });
    rows
}

/// The vaults this screen lists.
///
/// PORT-TODO: `TermiusState` does not carry `vaults: Vec<Vault>` yet, so
/// this is always empty and the Team list falls through to its
/// [`EmptyState`]. When the slice lands (storage + `termius_core::Vault`
/// already exist), this becomes `state.read(cx).vaults.clone()` from the
/// owning [`TeamScreen`]'s render.
fn team_vaults(_state: &Entity<TermiusState>) -> Vec<Vault> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Team screen view (see the module docs).
pub struct TeamScreen {
    state: Entity<TermiusState>,
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
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// Open the Invite-members flow (the shell hosts the dialog).
    ///
    /// Inviting syncs with the cloud; the stub stays until the sync wave wires
    /// invites (`CreateTeam` / `InviteMembersTrialOnboarding`).
    fn invite_members(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.open_dialog(
                Dialog::Confirm {
                    title: "Invite members".to_owned(),
                    message: INVITE_NOTE.to_owned(),
                },
                cx,
            );
        });
    }

    /// Raise the New Vault confirmation (the shell hosts the dialog).
    ///
    /// Creation itself is a stub until the sync wave wires vault writes.
    fn confirm_new_vault(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.open_dialog(
                Dialog::Confirm {
                    title: "New Vault".to_owned(),
                    message: NEW_VAULT_NOTE.to_owned(),
                },
                cx,
            );
        });
    }
}

impl Render for TeamScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let account = self.state.read(cx).account.clone();
        let members = team_members(account.as_ref());
        let rows = vault_rows(&team_vaults(&self.state));

        // Header: title (leading icon + `subtitle1`) and the primary action.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(theme.title)
                    .child(crate::icon("team.svg").w(px(16.)).h(px(16.)))
                    .child(SettingsTitle::new(TITLE).element(theme)),
            )
            .child(Button::new("Invite members").primary().on_click(
                theme,
                cx.listener(|this, _event, _window, cx| this.invite_members(cx)),
            ));

        // ----- team card: name + members -----------------------------------
        let mut team_card = section_card(theme).child(team_name_row(theme, account.as_ref()));
        if members.is_empty() {
            team_card = team_card.child(
                div().h(px(EMPTY_HEIGHT)).child(
                    EmptyState::new("No team", "Sign in to create a team and invite teammates.")
                        .element(theme),
                ),
            );
        } else {
            team_card = team_card.child(members_header_row(theme));
            let mut list = div().flex().flex_col().px(px(8.)).py(px(4.));
            for member in &members {
                list = list.child(member_row(theme, member));
            }
            team_card = team_card.child(list);
        }

        // ----- vault card: vault list + New Vault --------------------------
        let mut vault_card = section_card(theme).child(
            div()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme.border_light)
                .child(SectionHeader::new("Vaults").element(theme)),
        );
        if rows.is_empty() {
            vault_card = vault_card.child(
                div().h(px(EMPTY_HEIGHT)).child(
                    EmptyState::new(
                        "No team vaults",
                        "Sign in and connect a team to share hosts, keys, and snippets in vaults.",
                    )
                    .element(theme),
                ),
            );
        } else {
            let mut list = div().flex().flex_col().px(px(8.)).py(px(4.));
            for row in rows {
                let subtitle = row.subtitle();
                let VaultRow { id, title, .. } = row;
                list = list.child(
                    ListItem::new(title, subtitle)
                        .id(SharedString::from(id))
                        .element(theme),
                );
            }
            vault_card = vault_card.child(list);
        }
        vault_card = vault_card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .border_t_1()
                .border_color(theme.border_light)
                // Opens the shell-hosted confirm dialog (a real state change:
                // `TermiusState::active_dialog`); creation itself is a stub.
                .child(Button::new("New Vault").on_click(
                    theme,
                    cx.listener(|this, _event, _window, cx| {
                        this.confirm_new_vault(cx);
                    }),
                )),
        );

        let sharing = SettingsSection::new("Sharing")
            .child(
                div()
                    .px(px(12.))
                    .py(px(12.))
                    .child(SettingsText::new(SHARING_NOTE).element(theme)),
            )
            .element(theme);

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(px(COLUMN_WIDTH))
                    .mx_auto()
                    .gap(px(16.))
                    .p(px(16.))
                    .child(header)
                    .child(team_card)
                    .child(vault_card)
                    .child(sharing),
            )
    }
}

// ---------------------------------------------------------------------------
// Card / row builders
// ---------------------------------------------------------------------------

/// The card surface (`padding: 0`, `--card-a`, radius medium) holding rows.
fn section_card(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(theme.corner_radius_medium))
        .overflow_hidden()
        .bg(theme.card_a)
        .text_color(theme.title)
}

/// The team card's top row: the team icon + name (ellipsis) and a `Manage`
/// external-link affordance.
fn team_name_row(theme: TermiusTheme, account: Option<&AccountInfo>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .px(px(15.))
        .py(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w(px(0.))
                .child(
                    crate::icon("role-gradient.svg")
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
            // PORT-TODO: Manage opens the team-management flow (sync wave).
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .flex_shrink_0()
                .text_color(theme.primary)
                .child(text::R12P.style(div()).child(SharedString::from("Manage")))
                .child(
                    crate::icon("openExternal.svg")
                        .w(px(12.))
                        .h(px(12.))
                        .text_color(theme.primary),
                ),
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
        .mx(px(10.))
        .rounded(px(theme.corner_radius_medium))
        .bg(fill)
        .child(
            text::B14P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from("Member")),
        )
        .child(
            text::B14P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from("Status")),
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

/// A small role badge pill (the `--gradient-light-main` owner tag).
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

/// The tiny `YOU` pill marking the current user.
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
                .text_size(px(9.))
                .font_weight(FontWeight::BOLD)
                .line_height(px(10.))
                .child(SharedString::from("YOU")),
        )
}

/// One member row (`UserListItem-958a4b71.js`): avatar, name with the owner /
/// `YOU` badges over a secondary line, and the trailing status.
fn member_row(theme: TermiusTheme, member: &MemberRow) -> gpui::Stateful<Div> {
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
        .child(
            div()
                .flex()
                .flex_shrink_0()
                .min_w(px(108.))
                .pl(px(10.))
                .child(
                    text::R12S
                        .style(div())
                        .text_color(theme.text_common)
                        .child(SharedString::from(member.status)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(id: &str, title: &str, is_default: bool) -> Vault {
        Vault {
            id: id.into(),
            title: title.into(),
            is_default,
            ..Vault::default()
        }
    }

    fn account(email: &str, plan: &str) -> AccountInfo {
        AccountInfo {
            email: email.to_owned(),
            plan: plan.to_owned(),
            device_count: 1,
        }
    }

    #[test]
    fn vault_rows_label_default_and_team() {
        let rows = vault_rows(&[vault("v1", "Ops", false), vault("v0", "Default", true)]);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_default);
        assert_eq!(rows[0].subtitle(), "Default");
        assert_eq!(rows[1].subtitle(), "Team");
    }

    #[test]
    fn vault_rows_default_sorts_first_then_by_title() {
        let rows = vault_rows(&[
            vault("v2", "Zeta", false),
            vault("v1", "Alpha", false),
            vault("v0", "Personal", true),
        ]);
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["v0", "v1", "v2"]);
    }

    #[test]
    fn vault_rows_empty_stays_empty() {
        assert!(vault_rows(&[]).is_empty());
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
        assert_eq!(members[0].status, "Active");
        assert!(members[0].is_current);
        assert_eq!(team_name(Some(&account)), TEAM_NAME);
    }

    #[test]
    fn member_name_falls_back_for_a_blank_email() {
        assert_eq!(member_name(&account("", "Free")), "You");
        assert_eq!(member_name(&account("  ", "Free")), "You");
    }
}
