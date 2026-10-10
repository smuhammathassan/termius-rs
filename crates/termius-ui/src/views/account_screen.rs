//! account_screen — the Account section: sign-in card when signed out, the
//! profile card when signed in.
//!
//! Port of the Termius desktop Account screen: email/password sign-in
//! against the Termius cloud, the plan + device summary, an upgrade
//! affordance and sign-out. [`account_screen`] builds a small
//! self-contained view (GPUI renders the returned entity as a child, exactly
//! like `HostList` / `SettingsScreen`), so the screen reads
//! [`TermiusState::account`] from its own `Context<Self>` and drives the
//! state through `Entity::update`.
//!
//! PORT-TODO(auth): sign-in is a **local stub** — it records an
//! [`AccountInfo`] on [`TermiusState::account`] without touching the
//! network, so the signed-in chrome is reviewable before auth lands. The
//! real flow is `termius_sync::SyncClient::signin`
//! (`POST /api/v3.3/auth/device/login/` → `Credentials` + `bulk_account`);
//! swap [`AccountScreen::sign_in`] when the sync integration wave lands.

use gpui::{
    div, px, AppContext as _, Context, Div, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window,
};

use crate::app_state::{AccountInfo, TermiusState};
use crate::primitives::{Button, InputField, SettingsSection, SettingsText, SettingsTitle};
use crate::theme::{theme_of, TermiusTheme};

/// Termius' Free plan caps synced devices at two.
const FREE_DEVICE_LIMIT: usize = 2;
/// Email shown (muted) in the display-only email box; the local stub signs
/// in with exactly this address so the profile card reads consistently.
const STUB_EMAIL: &str = "you@example.com";
/// Password-box placeholder (the box is display-only; see `primitives`).
const PASSWORD_PLACEHOLDER: &str = "••••••••";
/// Column width for this settings-style page.
const COLUMN_WIDTH: f32 = 560.0;
/// The local-stub caveat under the sign-in form.
const STUB_NOTE: &str = "Local stub sign-in — real Termius cloud auth arrives \
                         with the termius-sync integration wave.";

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
        format!("{device_count} device{} synced", if device_count == 1 { "" } else { "s" })
    }
}

/// The Account screen view (see the module docs).
pub struct AccountScreen {
    state: Entity<TermiusState>,
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

    /// Clear the signed-in account (Termius' Sign Out).
    fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.account = None;
            state.status_text = "Signed out".to_owned();
            cx.notify();
        });
    }
}

impl Render for AccountScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let account = self.state.read(cx).account.clone();

        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .max_w(px(COLUMN_WIDTH))
            .child(SettingsTitle::new("Account").element(theme));
        column = match account {
            Some(account) => column.child(profile_card(&account, theme, cx)),
            None => column.child(sign_in_card(theme, cx)),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .p(px(16.))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(column)
    }
}

/// The "Account" card shown while signed out: email + password boxes, the
/// primary `Sign In` (local stub) and the `Create Account` affordance.
fn sign_in_card(theme: TermiusTheme, cx: &mut Context<AccountScreen>) -> Div {
    let form = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .px(px(12.))
        .py(px(12.))
        .child(InputField::new("Email", "").placeholder(STUB_EMAIL).element(theme))
        .child(InputField::new("Password", "").placeholder(PASSWORD_PLACEHOLDER).element(theme))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(Button::new("Sign In").primary().on_click(
                    theme,
                    cx.listener(|this, _event, _window, cx| this.sign_in(STUB_EMAIL, cx)),
                ))
                // Visual only: the real app opens termius.com sign-up.
                .child(Button::new("Create Account").element(theme)),
        )
        .child(SettingsText::new(STUB_NOTE).element(theme));

    SettingsSection::new("Account").child(form).element(theme)
}

/// The "Profile" card shown while signed in: email, plan, devices, plus the
/// visual `Upgrade` and the state-clearing `Sign Out`.
fn profile_card(
    account: &AccountInfo,
    theme: TermiusTheme,
    cx: &mut Context<AccountScreen>,
) -> Div {
    let rows = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .px(px(12.))
        .py(px(12.))
        .child(div().text_sm().child(SharedString::from(account.email.clone())))
        .child(SettingsText::new(format!("{} plan", account.plan)).element(theme))
        .child(
            SettingsText::new(device_summary(account.device_count, &account.plan)).element(theme),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                // Visual only: the upgrade purchase flow is out of scope.
                .child(Button::new("Upgrade").element(theme))
                .child(Button::new("Sign Out").danger().on_click(
                    theme,
                    cx.listener(|this, _event, _window, cx| this.sign_out(cx)),
                )),
        );

    SettingsSection::new("Profile").child(rows).element(theme)
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
}
