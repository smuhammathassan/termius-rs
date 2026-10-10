//! team_screen — the Team section: the vault list ("clusters" in Termius),
//! the New Vault affordance and the sharing note.
//!
//! Port of the Termius desktop Team screen: every signed-in account has a
//! personal Default vault plus (for teams) shared vaults; this screen lists
//! them, raises the new-vault confirmation, and explains sharing.
//! [`team_screen`] builds a small self-contained view (GPUI renders the
//! returned entity as a child, exactly like `HostList` / `SettingsScreen`).
//!
//! PORT-TODO: `TermiusState` has no `vaults: Vec<Vault>` slice yet — the
//! `vaults` storage table (`termius_storage`) and [`Vault`] domain type
//! already exist, but the state field has not landed, so the list renders
//! empty and the screen shows its [`EmptyState`]. [`team_vaults`] is the
//! single read site to flip once the slice arrives.

use termius_core::Vault;

use gpui::{
    div, px, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window,
};

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{
    Button, EmptyState, ListItem, SectionHeader, SettingsSection, SettingsText,
};
use crate::theme::{theme_of, TermiusTheme};

/// Height of the empty-vaults placeholder card.
const EMPTY_HEIGHT: f32 = 160.0;
/// Column width for this settings-style page.
const COLUMN_WIDTH: f32 = 560.0;
/// The Sharing explainer under the vault list.
const SHARING_NOTE: &str = "Vaults you share with teammates appear here. Teammates \
                            get every host, key, and snippet stored in a shared \
                            vault — end-to-end encrypted like the rest of your library.";
/// What the New Vault confirmation says (creation syncs with the cloud).
const NEW_VAULT_NOTE: &str = "Vault creation syncs with the Termius cloud; it \
                               arrives with the termius-sync integration. This \
                               dialog confirms the action for now.";

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
        if self.is_default { "Default" } else { "Team" }
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
        let rows = vault_rows(&team_vaults(&self.state));

        let mut card = div()
            .flex()
            .flex_col()
            .max_w(px(COLUMN_WIDTH))
            .rounded(px(4.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.sidebar_background)
            .text_color(theme.foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(SectionHeader::new("Team").element(theme)),
            );

        if rows.is_empty() {
            card = card.child(
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
                    ListItem::new(title, subtitle).id(SharedString::from(id)).element(theme),
                );
            }
            card = card.child(list);
        }

        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .border_t_1()
                .border_color(theme.border)
                // Opens the shell-hosted confirm dialog (a real state change:
                // `TermiusState::active_dialog`); creation itself is a stub.
                .child(
                    Button::new("New Vault").on_click(theme, cx.listener(|this, _event, _window, cx| {
                        this.confirm_new_vault(cx);
                    })),
                ),
        );

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap(px(16.))
            .p(px(16.))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(card)
            .child(
                div().max_w(px(COLUMN_WIDTH)).child(
                    SettingsSection::new("Sharing")
                        .child(
                            div()
                                .px(px(12.))
                                .py(px(12.))
                                .child(SettingsText::new(SHARING_NOTE).element(theme)),
                        )
                        .element(theme),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(id: &str, title: &str, is_default: bool) -> Vault {
        Vault { id: id.into(), title: title.into(), is_default, ..Vault::default() }
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
}
