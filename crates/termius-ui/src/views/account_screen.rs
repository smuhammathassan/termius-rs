//! account_screen — the Account section: the stacked settings sections when
//! signed in, the sign-in card when signed out, and the "Confirm logout"
//! screen.
//!
//! Faithful port of the Termius desktop Account tab (`Profile`,
//! `assets/index-fce1214f.js`; see `analysis/recon/31-account-team.md` §3). The
//! chrome is driven by the recovered original components:
//!
//! * **stacked sections** ← `Ws` (`index-fce1214f.js:1354-1378`) — the tab is
//!   **not** a single card but a column of `SettingsSection`s:
//!   `Ls` (trial/suspended banner) → `useKs`/`gs` (subscription card with a
//!   status badge + Upgrade/Buy now/Manage) → `us` ("Termius for Teams") →
//!   `useLs` (**Account**: 2FA, email Verify/Change, Log out / Change username
//!   / Delete account) → `useNs` (**Synchronization**: "Sync keys and
//!   identities", last-sync, "Sync now").
//! * **sign-in card** ← `SignInScreen-dffa5e15.js:259-278` — a **470px** card,
//!   `padding: 30px`, `border-radius: 23px` over a `--light-grey-6` /
//!   `--dark-grey-2` surface hosting the authorization form.
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
//! # PORT-TODOs
//!
//! * `TermiusState`/`AccountInfo` carry only `email`, `plan`, `device_count`
//!   — there is **no** subscription state, username, verified flag, trial
//!   window, or team slice, so those controls read view-local state and the
//!   flows (Verify / Change / Delete / Enable sharing / Sync now / Manage) open
//!   a [`Dialog::Confirm`] stub instead of calling the network. Flip each to
//!   the `termius-sync` integration when it lands.
//! * Sign-in is a **local stub** — it records an [`AccountInfo`] on
//!   [`TermiusState::account`] without touching the network; the real flow is
//!   `termius_sync::SyncClient::signin`
//!   (`POST /api/v3.3/auth/device/login/` → `Credentials` + `bulk_account`).
//! * [`InputField`](crate::primitives::InputField) is display-only, so the
//!   sign-in boxes show placeholders rather than live input.

use gpui::{
    div, px, AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};

use crate::app_state::{AccountInfo, Dialog, TermiusState};
use crate::primitives::{
    Button, ButtonSize, InputField, SettingsSection, SettingsText, SettingsTitle, Switch,
};
use crate::theme::{text, theme_of, TermiusTheme};

/// Termius' Free plan caps synced devices at two.
const FREE_DEVICE_LIMIT: usize = 2;
/// Email shown (muted) in the display-only email box; the local stub signs
/// in with exactly this address so the profile reads consistently.
const STUB_EMAIL: &str = "you@example.com";
/// Password-box placeholder (the box is display-only; see `primitives`).
const PASSWORD_PLACEHOLDER: &str = "••••••••";
/// The sign-in card (`SignInScreen-dffa5e15.js:259-278`): 470px wide, 30px
/// padded, 23px radius.
const SIGNIN_CARD_WIDTH: f32 = 470.0;
const SIGNIN_CARD_PAD: f32 = 30.0;
const SIGNIN_CARD_RADIUS: f32 = 23.0;
/// The local-stub caveat under the sign-in form.
const STUB_NOTE: &str = "Local stub sign-in — real Termius cloud auth arrives \
                         with the termius-sync integration wave.";
/// The Account section title (`useLs` `SettingsTitle "Account"`).
const ACCOUNT_TITLE: &str = "Account";
/// The Synchronization section title (`useNs` `SettingsTitle "Synchronization"`).
const SYNC_TITLE: &str = "Synchronization";
/// The Teams section title (`us` `SettingsTitle`).
const TEAMS_TITLE: &str = "Termius for Teams";
/// The Teams section body (`us`).
const TEAMS_NOTE: &str = "Share hosts, keys, and snippets with teammates in \
                          end-to-end encrypted team vaults.";
/// Last-sync line when the local store has never synced (`useNs`).
const NO_SYNC_NOTE: &str = "Data has not been synchronized yet";
/// The plan banner's body (`Ls`) — a local-stub stand-in for the trial /
/// suspended banner (which also carries an "N days left" counter).
const BANNER_NOTE: &str = "Sync and collaboration are limited on the Free plan. \
                           Upgrade to unlock cloud vaults and multiplayer.";
/// Label for the plan banner's link (`Ls` `Manage plan`).
const MANAGE_PLAN: &str = "Manage plan";

/// The subscription status badge shown on the plan card (`gs` `ps`):
/// `Expired` (red) or `Active` (accent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanStatus {
    Active,
    Expired,
}

impl PlanStatus {
    /// The badge's upper-case label.
    fn label(self) -> &'static str {
        match self {
            PlanStatus::Active => "ACTIVE",
            PlanStatus::Expired => "EXPIRED",
        }
    }
}

/// The subscription status for a plan label.
///
/// PORT-TODO: `AccountInfo` has no subscription expiry, so a Free plan is
/// treated as an expired trial and any paid plan as active.
fn plan_status(plan: &str) -> PlanStatus {
    if is_pro(plan) {
        PlanStatus::Active
    } else {
        PlanStatus::Expired
    }
}

/// Whether a plan unlocks the pro-gated controls (`disabled: !pro_mode`).
fn is_pro(plan: &str) -> bool {
    !plan.trim().eq_ignore_ascii_case("free")
}

/// The primary subscription action (`ce`): `Upgrade` (free), `Buy now`
/// (expired paid), `Manage` (active paid).
fn plan_action_label(plan: &str) -> &'static str {
    match plan_status(plan) {
        PlanStatus::Expired if is_pro(plan) => "Buy now",
        PlanStatus::Expired => "Upgrade",
        PlanStatus::Active => "Manage",
    }
}

/// The account a local (stub) sign-in records: Free plan, this device.
fn stub_account(email: &str) -> AccountInfo {
    AccountInfo {
        email: email.to_owned(),
        plan: "Free".to_owned(),
        device_count: 1,
    }
}

/// The device line shown on the plan card: Free plans show the two-device cap
/// ("1 of 2 devices used", like Termius), paid plans just count.
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

/// The name line on the sign-in card: the sign-in email, or the personal label
/// when the stub account carries none (`full_name || email`).
fn display_name(account: &AccountInfo) -> String {
    let email = account.email.trim();
    if email.is_empty() {
        "Personal".to_owned()
    } else {
        email.to_owned()
    }
}

/// The email-or-username line of the Account section (`<email|username>`).
fn account_identity(account: &AccountInfo) -> String {
    let email = account.email.trim();
    if email.is_empty() {
        "username".to_owned()
    } else {
        email.to_owned()
    }
}

/// The username `Change <Username>` edits — the email local part (`account`
/// has no username field yet).
fn username_of(account: &AccountInfo) -> String {
    let email = account.email.trim();
    match email.split_once('@') {
        Some((local, _)) if !local.is_empty() => local.to_owned(),
        _ if !email.is_empty() => email.to_owned(),
        _ => "username".to_owned(),
    }
}

/// The verified-state suffix the email row appends (`(Verified|Not verified)`).
fn verified_label(verified: bool) -> &'static str {
    if verified {
        "Verified"
    } else {
        "Not verified"
    }
}

/// The Account screen view (see the module docs).
pub struct AccountScreen {
    state: Entity<TermiusState>,
    /// Whether the "Confirm logout" screen is showing (LogoutMainScreen `L`).
    confirm_logout: bool,
    /// 2FA toggle (`useLs` `cs`); no `AccountInfo` field yet.
    two_factor: bool,
    /// "Sync keys and identities" toggle (`useNs`); no model field yet.
    sync_keys: bool,
    /// Email verification state (`useLs`); no model field yet.
    email_verified: bool,
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
            // Termius defaults: 2FA off, key/identity sync on, email unverified.
            two_factor: false,
            sync_keys: true,
            email_verified: false,
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

    /// Flip the 2FA toggle (`useLs`; the real flow opens `OTPFlowDialog`).
    fn toggle_two_factor(&mut self, cx: &mut Context<Self>) {
        self.two_factor = !self.two_factor;
        let note = if self.two_factor {
            "Two-factor authentication enabled (local stub)."
        } else {
            "Two-factor authentication disabled."
        };
        self.state.update(cx, |state, cx| {
            state.status_text = note.to_owned();
            cx.notify();
        });
    }

    /// Flip the "Sync keys and identities" toggle (`useNs`).
    fn toggle_sync_keys(&mut self, cx: &mut Context<Self>) {
        self.sync_keys = !self.sync_keys;
        let note = if self.sync_keys {
            "Sync keys and identities enabled."
        } else {
            "Sync keys and identities disabled."
        };
        self.state.update(cx, |state, cx| {
            state.status_text = note.to_owned();
            cx.notify();
        });
    }

    /// Open a stubbed confirm dialog for an account flow (`cS`/`MessageDialog`).
    fn open_account_dialog(&mut self, title: &str, message: &str, cx: &mut Context<Self>) {
        let title = title.to_owned();
        let message = message.to_owned();
        self.state.update(cx, |state, cx| {
            state.open_dialog(Dialog::Confirm { title, message }, cx);
        });
    }

    /// Mark the email verified (`useLs` `Verify`).
    fn verify_email(&mut self, cx: &mut Context<Self>) {
        self.email_verified = true;
        self.state.update(cx, |state, cx| {
            state.status_text = "Verification email sent (local stub).".to_owned();
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

        let content = match &account {
            Some(account) => signed_in_tab(theme, account, self, cx).into_any_element(),
            None => signed_out_card(theme, cx).into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(content)
    }
}

// ---------------------------------------------------------------------------
// Signed-in: the stacked settings sections (`Ws`)
// ---------------------------------------------------------------------------

/// The signed-in Account tab: the section stack from `Ws`.
fn signed_in_tab(
    theme: TermiusTheme,
    account: &AccountInfo,
    this: &AccountScreen,
    cx: &mut Context<AccountScreen>,
) -> Stateful<Div> {
    let mut column = div()
        .id("account-scroll")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .pb(px(60.))
        .overflow_y_scroll();
    column = column.child(plan_banner(theme, account, cx));
    column = column.child(subscription_card(theme, account, cx));
    column = column.child(teams_section(theme, cx));
    column = column.child(account_section(theme, account, this, cx));
    column = column.child(sync_section(theme, account, this, cx));
    column
}

/// The trial/suspended banner (`Ls`): an icon, the plan context and a
/// `Manage plan` link. PORT-TODO: no trial window on `AccountInfo`, so the
/// "N days left" counter is omitted.
fn plan_banner(
    theme: TermiusTheme,
    account: &AccountInfo,
    cx: &mut Context<AccountScreen>,
) -> Div {
    let title = if is_pro(&account.plan) {
        format!("{} plan", account.plan)
    } else {
        "Free plan".to_owned()
    };
    let body = if is_pro(&account.plan) {
        "Your subscription is active. Manage billing and seats anytime."
    } else {
        BANNER_NOTE
    };
    let mut card = section_card(theme);
    card = card.child(
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(36.))
                    .flex_shrink_0()
                    .rounded(px(theme.corner_radius_medium))
                    .bg(theme.hover)
                    .text_color(theme.primary)
                    .child(crate::icon("rocket.svg").w(px(20.)).h(px(20.))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(2.))
                    .child(
                        text::B14P
                            .style(div())
                            .text_color(theme.title)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        text::R12S
                            .style(div())
                            .text_color(theme.text_common)
                            .whitespace_normal()
                            .child(SharedString::from(body)),
                    ),
            )
            .child(link_text(theme, MANAGE_PLAN).on_click(cx.listener(
                |this, _event, _window, cx| {
                    this.open_account_dialog(
                        "Manage plan",
                        "Plan checkout and billing arrive with the termius-sync wave.",
                        cx,
                    );
                },
            ))),
    );
    card
}

/// The subscription card (`gs`): the plan name + status badge + action, over
/// the device summary.
fn subscription_card(
    theme: TermiusTheme,
    account: &AccountInfo,
    cx: &mut Context<AccountScreen>,
) -> Div {
    let status = plan_status(&account.plan);
    let action_label = plan_action_label(&account.plan);
    let mut action = Button::new(action_label);
    action = match status {
        PlanStatus::Active => action.secondary(),
        PlanStatus::Expired => action.primary(),
    };
    let action = action.on_click(theme, cx.listener(|this, _event, _window, cx| {
        this.open_account_dialog(
            "Manage subscription",
            "Plan checkout and billing arrive with the termius-sync wave.",
            cx,
        );
    }));

    section_card(theme)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    text::B16P
                        .style(div())
                        .text_color(theme.title)
                        .child(SharedString::from(account.plan.clone())),
                )
                .child(status_badge(theme, status))
                .child(div().flex_grow())
                .child(action),
        )
        .child(
            text::R12S
                .style(div())
                .mt(px(8.))
                .text_color(theme.text_common)
                .child(SharedString::from(device_summary(
                    account.device_count,
                    &account.plan,
                ))),
        )
}

/// "Termius for Teams" (`us`): title, note and the group-sharing action.
fn teams_section(theme: TermiusTheme, cx: &mut Context<AccountScreen>) -> Div {
    SettingsSection::new(TEAMS_TITLE)
        .child(SettingsText::new(TEAMS_NOTE).element(theme))
        .child(
            div().mt(px(10.)).child(
                Button::new("Enable group sharing").on_click(theme, cx.listener(
                    |this, _event, _window, cx| {
                        this.open_account_dialog(
                            "Enable group sharing",
                            "Creating a team and inviting members arrives with the \
                             termius-sync wave.",
                            cx,
                        );
                    },
                )),
            ),
        )
        .element(theme)
}

/// The **Account** section (`useLs`): 2FA, the email row, and the account
/// actions.
fn account_section(
    theme: TermiusTheme,
    account: &AccountInfo,
    this: &AccountScreen,
    cx: &mut Context<AccountScreen>,
) -> Div {
    let pro = is_pro(&account.plan);
    let identity = account_identity(account);

    let mut section = SettingsSection::new(ACCOUNT_TITLE).child(
        Switch::new("Enable 2FA", this.two_factor)
            .on_click(theme, cx.listener(|this, _event, _window, cx| {
                this.toggle_two_factor(cx);
            })),
    );

    // Email row: `<email> (Verified|Not verified)` + Verify / Change.
    let mut buttons = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .flex_shrink_0();
    if !this.email_verified {
        buttons = buttons.child(
            Button::new("Verify")
                .size(ButtonSize::Small)
                .on_click(theme, cx.listener(|this, _event, _window, cx| {
                    this.verify_email(cx);
                }))
                .w(px(56.)),
        );
    }
    buttons = buttons.child(
        Button::new("Change")
            .size(ButtonSize::Small)
            .on_click(theme, cx.listener(|this, _event, _window, cx| {
                this.open_account_dialog(
                    "Change email",
                    "Enter a new email and your password to change the sign-in address.",
                    cx,
                );
            }))
            .w(px(56.)),
    );
    section = section.child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .mt(px(15.))
            .child(
                text::R12S
                    .style(div())
                    .text_color(theme.text_common)
                    .truncate()
                    .child(SharedString::from(format!(
                        "{identity} ({})",
                        verified_label(this.email_verified)
                    ))),
            )
            .child(buttons),
    );

    // Account actions (`Y`): Log out / Change username / Delete account.
    let username = username_of(account);
    section = section.child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .mt(px(10.))
            .mb(px(5.))
            .child(
                Button::new("Log out").on_click(theme, cx.listener(
                    |this, _event, _window, cx| {
                        this.request_logout(cx);
                    },
                )),
            )
            .child(
                Button::new(format!("Change {username}"))
                    .disabled(!pro)
                    .on_click(theme, cx.listener(|this, _event, _window, cx| {
                        this.open_account_dialog(
                            "Change username",
                            "Changing your username requires a paid plan and sync.",
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("Delete account").on_click(theme, cx.listener(
                    |this, _event, _window, cx| {
                        this.open_account_dialog(
                            "Confirm account deletion",
                            "Deleting your account starts a 30-day grace period. \
                             After that your data is permanently erased.",
                            cx,
                        );
                    },
                )),
            ),
    );

    section.element(theme)
}

/// The **Synchronization** section (`useNs`): the sync toggle, the last-sync
/// line and "Sync now".
fn sync_section(
    theme: TermiusTheme,
    account: &AccountInfo,
    this: &AccountScreen,
    cx: &mut Context<AccountScreen>,
) -> Div {
    let pro = is_pro(&account.plan);
    SettingsSection::new(SYNC_TITLE)
        .child(
            Switch::new("Sync keys and identities", this.sync_keys)
                .on_click(theme, cx.listener(|this, _event, _window, cx| {
                    this.toggle_sync_keys(cx);
                })),
        )
        .child(SettingsText::new(NO_SYNC_NOTE).element(theme))
        .child(
            div().mt(px(10.)).child(
                Button::new("Sync now")
                    .disabled(!pro)
                    .on_click(theme, cx.listener(|this, _event, _window, cx| {
                        this.open_account_dialog(
                            "Sync now",
                            "Cloud sync arrives with the termius-sync integration wave.",
                            cx,
                        );
                    })),
            ),
        )
        .element(theme)
}

// ---------------------------------------------------------------------------
// Signed-out: the sign-in card (`useZs` / `SignInScreen`)
// ---------------------------------------------------------------------------

/// The signed-out Account tab: the authorization card centred on the page.
fn signed_out_card(theme: TermiusTheme, cx: &mut Context<AccountScreen>) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .pt(px(60.))
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(SIGNIN_CARD_WIDTH))
                .p(px(SIGNIN_CARD_PAD))
                .rounded(px(SIGNIN_CARD_RADIUS))
                .bg(theme.card_a)
                .text_color(theme.title)
                .child(SettingsTitle::new("Sign in to Termius").element(theme))
                .child(
                    div().mt(px(20.)).child(
                        InputField::new("Email", "")
                            .placeholder(STUB_EMAIL)
                            .element(theme),
                    ),
                )
                .child(
                    div().mt(px(12.)).child(
                        InputField::new("Password", "")
                            .placeholder(PASSWORD_PLACEHOLDER)
                            .element(theme),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .mt(px(20.))
                        .child(
                            Button::new("Sign In")
                                .primary()
                                .on_click(theme, cx.listener(|this, _event, _window, cx| {
                                    this.sign_in(STUB_EMAIL, cx);
                                }))
                                .w_full(),
                        )
                        // Visual only: the real app opens termius.com sign-up.
                        .child(Button::new("Create Account").element(theme).w_full()),
                )
                .child(SettingsText::new(STUB_NOTE).element(theme)),
        )
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

/// The plan status badge (`gs` `ps`): `color: white`, `text-transform:
/// uppercase`, `font-weight: 700`, `font-size: 10px`, `padding: 6px 10px`,
/// `border-radius: 7px`, `margin-left: 15px`.
fn status_badge(theme: TermiusTheme, status: PlanStatus) -> Div {
    let fill = match status {
        PlanStatus::Active => theme.primary,
        PlanStatus::Expired => theme.danger,
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .ml(px(15.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(7.))
        .bg(fill)
        .text_color(on_accent())
        .child(
            text::R10S
                .style(div())
                .font_weight(FontWeight::BOLD)
                .child(SharedString::from(status.label())),
        )
}

/// An accent text link (`LinkButton color="accent"`), e.g. `Manage plan` /
/// `Manage`.
fn link_text(theme: TermiusTheme, label: &'static str) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("link-{}", label)))
        .flex()
        .items_center()
        .gap(px(6.))
        .flex_shrink_0()
        .cursor_pointer()
        .text_color(theme.primary)
        .child(
            text::R14P
                .style(div())
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(label)),
        )
        .hover(move |style| style.text_color(theme.primary_light))
}

/// White, for text/knobs on accent or danger fills.
fn on_accent() -> gpui::Rgba {
    gpui::rgb(0xff_ff_ff)
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

    #[test]
    fn pro_is_anything_but_free() {
        assert!(!is_pro("Free"));
        assert!(!is_pro("free"));
        assert!(is_pro("Pro"));
        assert!(is_pro("Team"));
        assert!(is_pro("Business"));
    }

    #[test]
    fn plan_status_and_action_follow_the_plan() {
        assert_eq!(plan_status("Free"), PlanStatus::Expired);
        assert_eq!(plan_status("Pro"), PlanStatus::Active);
        assert_eq!(PlanStatus::Expired.label(), "EXPIRED");
        assert_eq!(PlanStatus::Active.label(), "ACTIVE");

        assert_eq!(plan_action_label("Free"), "Upgrade");
        assert_eq!(plan_action_label("Pro"), "Manage");
    }

    #[test]
    fn username_falls_back_to_the_email_or_a_default() {
        let account = stub_account("dev@example.com");
        assert_eq!(username_of(&account), "dev");
        let mut bare = stub_account("dev");
        assert_eq!(username_of(&bare), "dev");
        bare.email = "  ".to_owned();
        assert_eq!(username_of(&bare), "username");
    }

    #[test]
    fn identity_and_verified_labels() {
        let account = stub_account("dev@example.com");
        assert_eq!(account_identity(&account), "dev@example.com");
        let mut blank = stub_account("");
        assert_eq!(account_identity(&blank), "username");
        assert_eq!(verified_label(true), "Verified");
        assert_eq!(verified_label(false), "Not verified");
    }
}
