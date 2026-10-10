//! account_screen — the Account section: the sign-in card when signed out and
//! the profile card when signed in, plus the "Confirm logout" screen.
//!
//! Faithful port of the Termius desktop Account screen. The chrome is driven by
//! the recovered original components:
//!
//! * **user row** ← `UserListItem-958a4b71.js` — a `flex-row`, `gap: 10px`,
//!   `min-height: 60px`, `padding: 10px`, `borderRadius:
//!   --corner-radius-large` row: a leading avatar tile, the name
//!   (`R14P`) over a secondary line (`R12S`), and a trailing ⋯
//!   (`dots.svg`). Hover fills `--card-b` (light) / `--card-c` (dark).
//! * **sign-in card** ← `SignInScreen-dffa5e15.js` — `padding: 30px`,
//!   `borderRadius: 23px` content over a `--card-a` surface, with the MUI
//!   outlined email/password fields and the local-stub note.
//! * **Confirm logout** ← `LogoutMainScreen-42cea11d.js` (`L`) — a full-height
//!   column: a centred icon + heading (`Confirm logout`) + body, over a footer
//!   (`padding: 20px`, `borderTop: 1px solid --border-light`) holding `Back`
//!   (left) and `Log out` (right, destructive).
//!
//! [`account_screen`] builds a small self-contained view (GPUI renders the
//! returned entity as a child, exactly like `HostList` / `SettingsScreen`), so
//! the screen reads [`TermiusState::account`] from its own `Context<Self>` and
//! drives the state through `Entity::update`.
//!
//! PORT-TODO(auth): sign-in is a **local stub** — it records an
//! [`AccountInfo`] on [`TermiusState::account`] without touching the
//! network, so the signed-in chrome is reviewable before auth lands. The
//! real flow is `termius_sync::SyncClient::signin`
//! (`POST /api/v3.3/auth/device/login/` → `Credentials` + `bulk_account`);
//! swap [`AccountScreen::sign_in`] when the sync integration wave lands.

use gpui::{
    div, px, AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window,
};

use crate::app_state::{AccountInfo, TermiusState};
use crate::primitives::{Button, InputField, SettingsText, SettingsTitle};
use crate::theme::{text, theme_of, TermiusTheme, ThemeMode};

/// Termius' Free plan caps synced devices at two.
const FREE_DEVICE_LIMIT: usize = 2;
/// Email shown (muted) in the display-only email box; the local stub signs
/// in with exactly this address so the profile card reads consistently.
const STUB_EMAIL: &str = "you@example.com";
/// Password-box placeholder (the box is display-only; see `primitives`).
const PASSWORD_PLACEHOLDER: &str = "••••••••";
/// Column width for this settings-style page (`SettingsSection` `maxWidth`).
const COLUMN_WIDTH: f32 = 700.0;
/// The leading avatar tile (`UserListItem` `iconSize` `extraLarge`).
const AVATAR_SIZE: f32 = 40.0;
/// Glyph inside the avatar tile.
const AVATAR_GLYPH: f32 = 20.0;
/// Minimum height of a user row (`UserListItem` `minHeight: 60px`).
const ROW_MIN_HEIGHT: f32 = 60.0;
/// Icon for the signed-in user's avatar.
const AVATAR_ICON: &str = "userWithGradient.svg";
/// The local-stub caveat under the sign-in form.
const STUB_NOTE: &str = "Local stub sign-in — real Termius cloud auth arrives \
                         with the termius-sync integration wave.";
/// The Account header title.
const TITLE: &str = "Account";

/// The account a local (stub) sign-in records: Free plan, this device.
fn stub_account(email: &str) -> AccountInfo {
    AccountInfo {
        email: email.to_owned(),
        plan: "Free".to_owned(),
        device_count: 1,
    }
}

/// The device line shown on the profile card: Free plans show the two-device
/// cap ("1 of 2 devices used", like Termius), paid plans just count.
fn device_summary(device_count: usize, plan: &str) -> String {
    if plan.eq_ignore_ascii_case("free") {
        format!("{device_count} of {FREE_DEVICE_LIMIT} devices used")
    } else {
        format!(
            "{device_count} device{} synced",
            if device_count == 1 { "" } else { "s" }
        )
    }
}

/// The name line on the user card: the sign-in email, or the personal label
/// when the stub account carries none (`UserListItem` `full_name || email`).
fn display_name(account: &AccountInfo) -> String {
    let email = account.email.trim();
    if email.is_empty() {
        "Personal".to_owned()
    } else {
        email.to_owned()
    }
}

/// The Account screen view (see the module docs).
pub struct AccountScreen {
    state: Entity<TermiusState>,
    /// Whether the "Confirm logout" screen is showing (LogoutMainScreen `L`).
    confirm_logout: bool,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Account screen for `state`.
///
/// Returns the view entity as an element: the shell inserts it wherever the
/// Account section is routed. Store the result once (the same way `AppShell`
/// holds `HostList`) — calling this every render would mint a new entity per
/// frame.
///
/// Note on gpui 0.2 leases: a `Context<TermiusState>` only exists inside
/// the state's own update lease (reading that entity through the handle
/// here would panic), so this factory mints a separate view entity and all
/// state access happens in [`AccountScreen`]'s `Context<Self>` — a
/// different entity, a safe `Entity::read`/`update`.
pub fn account_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<AccountScreen> {
    cx.new(|cx| AccountScreen::new(state, cx))
}

impl AccountScreen {
    /// Create the screen and subscribe it to state + theme changes.
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let observe_theme = cx.observe_global::<TermiusTheme>(|_, cx| cx.notify());
        Self {
            state,
            confirm_logout: false,
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// LOCAL STUB: record a Free account instead of calling
    /// `termius_sync::SyncClient::signin` (follow-up wave).
    fn sign_in(&mut self, email: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.account = Some(stub_account(email));
            state.status_text = "Signed in (local stub)".to_owned();
            cx.notify();
        });
    }

    /// Clear the signed-in account (Termius' Sign Out) and leave the confirm
    /// screen.
    fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.confirm_logout = false;
        self.state.update(cx, |state, cx| {
            state.account = None;
            state.status_text = "Signed out".to_owned();
            cx.notify();
        });
    }

    /// Show the "Confirm logout" screen (LogoutMainScreen `L`).
    fn request_logout(&mut self, cx: &mut Context<Self>) {
        self.confirm_logout = true;
        cx.notify();
    }

    /// The Upgrade affordance is visual only; the purchase flow is out of
    /// scope, so it surfaces a note in the status bar.
    fn note_upgrade(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.status_text =
                "Upgrade — plan checkout arrives with the termius-sync wave.".to_owned();
            cx.notify();
        });
    }
}

impl Render for AccountScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // The full-height confirm screen replaces everything else.
        if self.confirm_logout {
            return logout_confirm(theme, cx);
        }

        let account = self.state.read(cx).account.clone();

        // Header: title (leading icon + `subtitle1`) and the primary action.
        let action = match &account {
            Some(_) => Button::new("Upgrade").primary().on_click(
                theme,
                cx.listener(|this, _event, _window, cx| this.note_upgrade(cx)),
            ),
            None => Button::new("Sign In").primary().on_click(
                theme,
                cx.listener(|this, _event, _window, cx| this.sign_in(STUB_EMAIL, cx)),
            ),
        };
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
                    .child(crate::icon("person.svg").w(px(16.)).h(px(16.)))
                    .child(SettingsTitle::new(TITLE).element(theme)),
            )
            .child(action);

        let body = match &account {
            Some(account) => profile_card(account, theme, cx),
            None => sign_in_card(theme),
        };

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
                    .child(body),
            )
    }
}

/// The card surface (`SettingsSection` metrics: `--card-a`, radius medium,
/// `padding: 20px`).
fn card(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(8.))
        .p(px(20.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.card_a)
        .text_color(theme.title)
}

/// The leading round avatar tile (`UserListItem`'s `Avatar`, `--card-c` dark /
/// `--card-b` light fill, muted glyph).
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

/// The trailing ⋯ overflow affordance (`dots.svg`).
fn dots(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(20.))
        .h(px(20.))
        .flex_shrink_0()
        .text_color(theme.muted)
        .child(crate::icon("dots.svg").w(px(12.)).h(px(4.)))
}

/// One user row (`UserListItem-958a4b71.js`): avatar, name over secondary,
/// trailing ⋯.
fn user_row(
    theme: TermiusTheme,
    icon_name: &str,
    name: SharedString,
    secondary: SharedString,
) -> gpui::Stateful<Div> {
    let hover_fill = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
    div()
        .id("account-user-row")
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .min_h(px(ROW_MIN_HEIGHT))
        .p(px(10.))
        .rounded(px(theme.corner_radius_large))
        .hover(move |hover| hover.bg(hover_fill))
        .child(avatar_tile(theme, icon_name))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    text::R14P
                        .style(div())
                        .truncate()
                        .text_color(theme.title)
                        .child(name),
                )
                .child(
                    text::R12S
                        .style(div())
                        .truncate()
                        .text_color(theme.text_common)
                        .child(secondary),
                ),
        )
        .child(dots(theme))
}

/// A muted key/value detail row on the profile card.
fn detail_row(theme: TermiusTheme, label: &str, value: String) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(
            text::R12S
                .style(div())
                .text_color(theme.text_common)
                .child(SharedString::from(label.to_owned())),
        )
        .child(
            text::R14P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from(value)),
        )
}

/// A `1px` hairline in `--border-light`.
fn divider(theme: TermiusTheme) -> Div {
    div().w_full().h(px(1.)).bg(theme.border_light)
}

/// The "Account" card shown while signed out: email + password boxes, the
/// `Create Account` affordance and the local-stub note. (The primary `Sign In`
/// action lives in the screen header.)
fn sign_in_card(theme: TermiusTheme) -> Div {
    card(theme)
        .child(
            InputField::new("Email", "")
                .placeholder(STUB_EMAIL)
                .element(theme),
        )
        .child(
            InputField::new("Password", "")
                .placeholder(PASSWORD_PLACEHOLDER)
                .element(theme),
        )
        // Visual only: the real app opens termius.com sign-up.
        .child(Button::new("Create Account").element(theme))
        .child(SettingsText::new(STUB_NOTE).element(theme))
}

/// The "Account" card shown while signed in: the user row, the plan/device
/// details and the `Sign Out` (destructive) action.
fn profile_card(
    account: &AccountInfo,
    theme: TermiusTheme,
    cx: &mut Context<AccountScreen>,
) -> Div {
    let user = user_row(
        theme,
        AVATAR_ICON,
        SharedString::from(display_name(account)),
        SharedString::from(format!("{} plan", account.plan)),
    );
    let details = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(detail_row(theme, "Plan", account.plan.clone()))
        .child(detail_row(
            theme,
            "Devices",
            device_summary(account.device_count, &account.plan),
        ));

    card(theme)
        .child(user)
        .child(divider(theme))
        .child(details)
        .child(Button::new("Sign Out").danger().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| this.request_logout(cx)),
        ))
}

/// The full-height "Confirm logout" screen (`LogoutMainScreen-42cea11d.js`,
/// `L`): centred icon + heading + body over a `Back` / `Log out` footer.
fn logout_confirm(theme: TermiusTheme, cx: &mut Context<AccountScreen>) -> Div {
    let content = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .flex_1()
        .min_h(px(0.))
        .gap(px(30.))
        .p(px(20.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted)
                .child(crate::icon("logout.svg").w(px(48.)).h(px(48.))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .w(px(460.))
                .child(
                    text::B16P
                        .style(div())
                        .text_center()
                        .text_color(theme.title)
                        .child(SharedString::from("Confirm logout")),
                )
                .child(
                    text::R14S
                        .style(div())
                        .text_center()
                        .text_color(theme.text_common)
                        .child(SharedString::from(
                            "Logging out will erase all local information on this device. \
                             Log in again to access your cloud vault.",
                        )),
                ),
        );

    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .p(px(20.))
        .border_t_1()
        .border_color(theme.border_light)
        .child(Button::new("Back").secondary().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| {
                this.confirm_logout = false;
                cx.notify();
            }),
        ))
        .child(Button::new("Log out").danger().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| this.sign_out(cx)),
        ));

    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme.background)
        .text_color(theme.foreground)
        .child(content)
        .child(footer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_account_is_a_free_single_device_account() {
        let account = stub_account("dev@example.com");
        assert_eq!(account.email, "dev@example.com");
        assert_eq!(account.plan, "Free");
        assert_eq!(account.device_count, 1);
    }

    #[test]
    fn free_plans_show_the_device_cap() {
        assert_eq!(device_summary(1, "Free"), "1 of 2 devices used");
        // Plan matching is case-insensitive.
        assert_eq!(device_summary(2, "free"), "2 of 2 devices used");
    }

    #[test]
    fn paid_plans_count_devices() {
        assert_eq!(device_summary(1, "Pro"), "1 device synced");
        assert_eq!(device_summary(3, "Pro"), "3 devices synced");
    }

    #[test]
    fn display_name_falls_back_to_personal() {
        assert_eq!(
            display_name(&stub_account("dev@example.com")),
            "dev@example.com"
        );
        let mut blank = stub_account("");
        blank.email = "  ".to_owned();
        assert_eq!(display_name(&blank), "Personal");
    }
}
