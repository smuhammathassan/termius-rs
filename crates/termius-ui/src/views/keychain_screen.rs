//! keychain_screen — the **Vaults** management screen (the original's
//! "Keychain" surfaces, part b).
//!
//! Reconstructed from `analysis/recon/11-vaults.md` and
//! `analysis/recon/23-keychain.md`: in the shipped Termius v10, the word
//! "Keychain" maps onto **three** distinct surfaces — (a) the Keys / Identities
//! *section* (see [`crate::views::keys_screen`]), (b) this **Vaults management
//! screen** (`assets/Vaults-ef2e0d94.js`), and (c) the **ChangeVault wizard**
//! ("Choose where to store credentials", `assets/ChangeVault-4b860408.js`).
//!
//! The earlier port modelled a nonexistent flat list of "keychains of secrets"
//! (title over `"{n} secret(s)"` with expandable masked entries). No such screen
//! exists in the original, so this module now renders the vault list:
//!
//! * **No title band** — the primary action is the **first child of the shared
//!   [`FiltersHeader`]**, exactly like every other section.
//! * **Vault rows** ([`EntityRow`]): the vault glyph, the vault name
//!   (`getVaultName`: `"{name} vault"` for the default vault), and — revealed
//!   once the row is selected, mirroring the original's hover/selected stats
//!   reveal — the per-type counts (hosts · snippets · keys · identities ·
//!   port-forwarding rules).
//! * **Vault editor panel** (the original `VaultEditorSlider`): the selected
//!   vault's name, its per-type stats with their icons, and the
//!   **Change vault…** entry point into the wizard.
//! * **ChangeVault wizard** (the original `Le` step, `ChangeVault-4b860408.js`):
//!   the "Choose where to store credentials" dialog with the three credential
//!   modes (personal / shared / multikey).
//!
//! # PORT-TODOs (model gaps)
//!
//! * `Keychain` is the port's vault record: it has no `isDefault` flag and the
//!   library records carry no `vault_id`, so vault membership cannot be
//!   computed — the per-type counts are **library-wide** and the wizard's
//!   "Continue" is a stub.
//! * There is no vault CRUD in [`TermiusState`], so "New vault" / "Create
//!   vault" surface a status note (the original opens the create form).
//! * `InputField` is display-only; the vault-name field shows a placeholder.
//!
//! # Wire contract
//!
//! The function hands back a cached [`Entity`] so the shell can drop it
//! straight into its routed column. It deliberately does **not** read
//! `TermiusState` here: a `&mut Context<TermiusState>` only exists inside a
//! `TermiusState` update, and gpui leases one entity at a time — reading the
//! state while it is leased would panic. The view reads the library in its
//! own `render` (by then no lease is held), and every click notifies the
//! screen so it repaints.

use gpui::{
    div, px, AppContext as _, Context, Div, Entity, FontWeight, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use termius_core::Keychain;

use crate::app_state::TermiusState;
use crate::assets::{icon, UI_FONT};
use crate::primitives::{Button, ButtonSize, EntityRow, FiltersHeader, InputField};
use crate::theme::{text, theme_of, with_alpha, TermiusTheme, ThemeMode};

/// `GridItemPresenter.entityItem` — radius of the row card (used by the editor
/// panel too).
const ENTITY_RADIUS: f32 = 14.0;
/// White used for text/glyphs on the accent fill.
const ON_ACCENT: gpui::Rgba = gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
/// The ChangeVault dialog shell (`ChangeVault-4b860408.js`: `720×540`).
const WIZARD_WIDTH: f32 = 720.0;
/// The vault editor panel width (the original `RightSlider` is `552px`; the
/// port keeps a narrower details column).
const EDITOR_WIDTH: f32 = 320.0;

// ---------------------------------------------------------------------------
// Vault iconography
// ---------------------------------------------------------------------------

/// The vault row glyph (`vault.react-93164eff.svg` / `vault.highlighted`).
const VAULT_ICON: &str = "vault__93164e.svg";
const VAULT_ICON_SELECTED: &str = "vault.highlighted.svg";

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// `getVaultName` (`/tmp/reconnectSaga.js:72955`): the default / id-less vault
/// gets a `" vault"` suffix; an unnamed named vault is `"Unnamed"`.
fn vault_name(keychain: &Keychain) -> String {
    let title = keychain.title.trim();
    if keychain.id.is_empty() {
        if title.is_empty() {
            "Personal vault".to_owned()
        } else {
            format!("{title} vault")
        }
    } else if title.is_empty() {
        "Unnamed".to_owned()
    } else {
        title.to_owned()
    }
}

/// Per-type counts shown on a vault row / editor (`VaultStats`).
///
/// PORT-TODO: the model has no vault membership, so these are library-wide.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VaultCounts {
    hosts: usize,
    snippets: usize,
    keys: usize,
    identities: usize,
    forwards: usize,
}

impl VaultCounts {
    /// Whether every count is zero.
    fn is_empty(&self) -> bool {
        self.hosts == 0
            && self.snippets == 0
            && self.keys == 0
            && self.identities == 0
            && self.forwards == 0
    }
}

/// Append `"{n} {label}"` (pluralised) when `count > 0`.
fn push_count(parts: &mut Vec<String>, count: usize, singular: &str, plural: &str) {
    if count == 0 {
        return;
    }
    if count == 1 {
        parts.push(format!("1 {singular}"));
    } else {
        parts.push(format!("{count} {plural}"));
    }
}

/// The vault row subtitle: the per-type counts (`VaultStats`), or `"Empty"`.
fn vault_stats_summary(counts: &VaultCounts) -> String {
    let mut parts = Vec::new();
    push_count(&mut parts, counts.hosts, "host", "hosts");
    push_count(&mut parts, counts.snippets, "snippet", "snippets");
    push_count(&mut parts, counts.keys, "key", "keys");
    push_count(&mut parts, counts.identities, "identity", "identities");
    push_count(&mut parts, counts.forwards, "rule", "rules");
    if parts.is_empty() {
        "Empty".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// The credential modes of the ChangeVault wizard
/// (`ChangeVault-4b860408.js:540`, `Le`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeVaultMode {
    /// `no_credentials_sharing` — credentials stay in personal vaults.
    NoCredentialsSharing,
    /// `credentials_sharing` — credentials move to the target vault.
    CredentialsSharing,
    /// `multikey` — personal unextractable passkeys.
    Multikey,
}

impl ChangeVaultMode {
    /// All options, in the original order.
    const ALL: [Self; 3] = [
        Self::NoCredentialsSharing,
        Self::CredentialsSharing,
        Self::Multikey,
    ];

    /// The option body verbatim from `ChangeVault-4b860408.js:540`.
    fn body(self) -> &'static str {
        match self {
            Self::NoCredentialsSharing => {
                "Your credentials are not shared. Vault members connect with credentials from their personal vaults."
            }
            Self::CredentialsSharing => {
                "Your credentials are shared. Vault members connect with credentials you move to this vault."
            }
            Self::Multikey => {
                "Vault members connect with personal unextractable passkeys generated on each device."
            }
        }
    }

    /// The option label: the personal / target vault name, or `"Multikey"`.
    fn label(self, target_vault: &str) -> String {
        match self {
            Self::NoCredentialsSharing => "Personal vault".to_owned(),
            Self::CredentialsSharing => target_vault.to_owned(),
            Self::Multikey => "Multikey".to_owned(),
        }
    }
}

/// The ChangeVault operation verb (`$e`, `:534`): moving / copying / …
fn change_vault_operation() -> &'static str {
    "moving to"
}

// ---------------------------------------------------------------------------
// Row / button chrome
// ---------------------------------------------------------------------------

/// The primary header action: `plusThin.svg` + label on the accent fill (the
/// original `New …` split button; the thin plus, not `addCircle.svg`).
fn new_vault_button(
    theme: TermiusTheme,
    listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let hover_fill = with_alpha(ON_ACCENT, 0.12);
    div()
        .id("vault-new")
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(36.))
        .px(px(14.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.primary)
        .text_color(ON_ACCENT)
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(icon("plusThin.svg").w(px(14.)).h(px(14.)))
        .child(SharedString::from("New vault"))
        .hover(move |style| style.bg(hover_fill))
        .on_click(listener)
}

/// A small ghost action button with a caller-supplied id (so the header and
/// the editor panel can both offer "Change vault…" without colliding).
fn ghost_button(
    theme: TermiusTheme,
    id: &str,
    label: &'static str,
    listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id.to_owned()))
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.))
        .px(px(10.))
        .rounded(px(6.))
        .text_color(theme.primary)
        .font_family(UI_FONT)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(SharedString::from(label))
        .hover(move |style| style.bg(theme.hover))
        .on_click(listener)
}

/// `--entity-item-background`: `--white` (light) / `--dark-grey-3` (dark).
fn entity_bg(theme: TermiusTheme) -> gpui::Rgba {
    match theme.mode {
        ThemeMode::Light => theme.card_c,
        ThemeMode::Dark => theme.card_a,
    }
}

/// `--list-hover-hover`: `--light-grey-5` (light) / `--dark-grey-4` (dark).
fn entity_hover(theme: TermiusTheme) -> gpui::Rgba {
    match theme.mode {
        ThemeMode::Light => theme.card_b,
        ThemeMode::Dark => theme.card_c,
    }
}

/// One per-type stat row in the editor panel: icon · count · label.
fn stat_row(theme: TermiusTheme, icon_name: &'static str, count: usize, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(icon(icon_name).w(px(14.)).h(px(14.)).text_color(theme.text_common))
        .child(
            text::R12P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from(count.to_string())),
        )
        .child(
            text::R12S
                .style(div())
                .text_color(theme.text_common)
                .child(SharedString::from(label)),
        )
}

/// One ChangeVault radio option (`RadioOptionsList`): a radio glyph, the
/// option label over its body copy.
fn radio_option(
    theme: TermiusTheme,
    mode: ChangeVaultMode,
    target_vault: &str,
    selected: bool,
    cx: &mut Context<KeychainScreen>,
) -> Stateful<Div> {
    let base = entity_bg(theme);
    let hover = entity_hover(theme);
    let border = if selected { theme.border_accent } else { base };
    let glyph = if selected { "checked.svg" } else { "unchecked.svg" };
    let label = mode.label(target_vault);
    div()
        .id(SharedString::from(format!("vault-mode-{label}")))
        .flex()
        .items_center()
        .gap(px(10.))
        .p(px(10.))
        .rounded(px(ENTITY_RADIUS))
        .border_2()
        .border_color(border)
        .bg(base)
        .text_color(theme.title)
        .hover(move |style| style.bg(hover).border_color(hover))
        .child(
            icon(glyph)
                .w(px(18.))
                .h(px(18.))
                .flex_shrink_0()
                .text_color(if selected { theme.primary } else { theme.text_common }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    text::R14P
                        .style(div())
                        .text_color(theme.title)
                        .truncate()
                        .child(SharedString::from(label)),
                )
                .child(
                    text::R12S
                        .style(div())
                        .text_color(theme.text_common)
                        .child(SharedString::from(mode.body())),
                ),
        )
        .on_click(cx.listener(move |this, _event, _window, cx| {
            this.wizard_mode = mode;
            cx.notify();
        }))
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The process-wide Vaults screen instance (keeps the selection + wizard state
/// across the shell's re-renders).
#[derive(Clone, Default)]
struct KeychainScreenHost(Option<Entity<KeychainScreen>>);

impl Global for KeychainScreenHost {}

/// The Vaults management screen (the original's "Keychain" vault surface).
pub struct KeychainScreen {
    state: Entity<TermiusState>,
    /// The vault whose editor panel is open.
    selected: Option<String>,
    /// The ChangeVault wizard is showing.
    wizard_open: bool,
    /// The wizard's chosen credential mode.
    wizard_mode: ChangeVaultMode,
}

impl KeychainScreen {
    fn new(state: Entity<TermiusState>) -> Self {
        Self {
            state,
            selected: None,
            wizard_open: false,
            wizard_mode: ChangeVaultMode::CredentialsSharing,
        }
    }

    /// Select a vault row (opens its editor panel).
    fn select_vault(&mut self, vault_id: &str, cx: &mut Context<Self>) {
        self.selected = if self.selected.as_deref() == Some(vault_id) {
            None
        } else {
            Some(vault_id.to_owned())
        };
        cx.notify();
    }

    /// Open the ChangeVault wizard for the selected vault.
    fn open_wizard(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            return;
        }
        self.wizard_open = true;
        cx.notify();
    }

    /// Close the ChangeVault wizard.
    fn close_wizard(&mut self, cx: &mut Context<Self>) {
        self.wizard_open = false;
        cx.notify();
    }

    /// PORT-TODO: vault CRUD needs a `TermiusState::add_keychain`; until then
    /// the create actions surface a status note.
    fn new_vault_note(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.status_text =
                "New vault — vault creation arrives with the storage/sync wave.".to_owned();
            cx.notify();
        });
    }

    /// PORT-TODO: moving credentials between vaults needs vault membership on
    /// the records; until then "Continue" surfaces a status note.
    fn submit_wizard(&mut self, cx: &mut Context<Self>) {
        let mode = self.wizard_mode;
        self.wizard_open = false;
        self.state.update(cx, |state, cx| {
            state.status_text = match mode {
                ChangeVaultMode::NoCredentialsSharing => {
                    "Credentials stay in personal vaults.".to_owned()
                }
                ChangeVaultMode::CredentialsSharing => {
                    "Credentials move to the selected vault.".to_owned()
                }
                ChangeVaultMode::Multikey => {
                    "Vault members use personal multikey passkeys.".to_owned()
                }
            };
            cx.notify();
        });
    }

    /// The selected vault's record, if it still exists.
    fn selected_vault(&self, cx: &Context<Self>) -> Option<Keychain> {
        let id = self.selected.as_deref()?;
        self.state
            .read(cx)
            .library
            .keychains
            .iter()
            .find(|vault| vault.id == id)
            .cloned()
    }

    /// Per-type counts (PORT-TODO: library-wide — the model has no vault
    /// membership).
    fn vault_counts(&self, cx: &Context<Self>) -> VaultCounts {
        let state = self.state.read(cx);
        VaultCounts {
            hosts: state.library.hosts.len(),
            snippets: state.library.snippets.len(),
            keys: state.library.keys.len(),
            identities: state.library.identities.len(),
            forwards: state.port_forwardings.len(),
        }
    }

    /// The vault list (left pane): one [`EntityRow`] per vault.
    fn vault_list(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        let keychains = self.state.read(cx).library.keychains.clone();
        let counts = self.vault_counts(cx);
        let summary = vault_stats_summary(&counts);

        if keychains.is_empty() {
            return div()
                .flex()
                .flex_1()
                .min_w(px(0.))
                .items_center()
                .justify_center()
                .child(self.create_vault_form(theme, cx));
        }

        // PORT-TODO: no scroll handle yet (`overflow_y_scroll` needs one in
        // some gpui layouts); long vault lists clip.
        let mut list = div().flex().flex_col().flex_1().min_w(px(0.)).gap(px(2.)).p(px(8.));
        for vault in &keychains {
            let vault_id = vault.id.clone();
            let is_selected = self.selected.as_deref() == Some(vault.id.as_str());
            let icon_name = if is_selected { VAULT_ICON_SELECTED } else { VAULT_ICON };
            // The stats reveal on selection (the original reveals on
            // hover/selected via `--stats-opacity`).
            let subtitle = if is_selected { summary.clone() } else { String::new() };
            list = list.child(
                EntityRow::new(vault_name(vault), subtitle, icon_name)
                    .selected(is_selected)
                    .on_click(
                        theme,
                        cx.listener(move |this, _event, _window, cx| {
                            this.select_vault(&vault_id, cx);
                        }),
                    ),
            );
        }
        list
    }

    /// The create-vault form shown when the vault set is empty
    /// (`Vaults-ef2e0d94.js`: the Vaults screen renders the create form).
    fn create_vault_form(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .w(px(420.))
            .p(px(20.))
            .rounded(px(theme.corner_radius_medium))
            .bg(theme.card_a)
            .child(
                text::B14P
                    .style(div())
                    .text_color(theme.title)
                    .child(SharedString::from("Create vault")),
            )
            .child(
                InputField::new("Vault name", "")
                    .placeholder("Enter vault name, e.g. Production...")
                    .element(theme),
            )
            .child(
                div().flex().justify_end().child(
                    Button::new("Create vault").primary().on_click(
                        theme,
                        cx.listener(|this, _event, _window, cx| this.new_vault_note(cx)),
                    ),
                ),
            )
    }

    /// The vault editor panel (right pane): name + per-type stats + the
    /// Change-vault entry point.
    fn vault_editor(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        let vault = self.selected_vault(cx);
        let counts = self.vault_counts(cx);
        let name = vault
            .as_ref()
            .map(vault_name)
            .unwrap_or_else(|| "Vault".to_owned());

        let mut panel = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(EDITOR_WIDTH))
            .gap(px(12.))
            .p(px(16.))
            .border_l_1()
            .border_color(theme.border)
            .child(
                text::B16P
                    .style(div())
                    .text_color(theme.title)
                    .child(SharedString::from("Vault details")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(VAULT_ICON).w(px(16.)).h(px(16.)).text_color(theme.title))
                    .child(
                        text::R14P
                            .style(div())
                            .text_color(theme.title)
                            .truncate()
                            .child(SharedString::from(name)),
                    ),
            );

        if counts.is_empty() {
            panel = panel.child(
                text::R12S
                    .style(div())
                    .text_color(theme.text_common)
                    .child(SharedString::from("No items in this vault yet.")),
            );
        } else {
            let mut stats = div().flex().flex_col().gap(px(6.));
            stats = stats.child(stat_row(theme, "host.svg", counts.hosts, "hosts"));
            stats = stats.child(stat_row(theme, "snippet.svg", counts.snippets, "snippets"));
            stats = stats.child(stat_row(theme, "key.svg", counts.keys, "keys"));
            stats = stats.child(stat_row(
                theme,
                "Identity.svg",
                counts.identities,
                "identities",
            ));
            stats = stats.child(stat_row(
                theme,
                "PortForwarding.svg",
                counts.forwards,
                "port forwarding rules",
            ));
            panel = panel.child(stats);
        }

        panel = panel.child(div().pt(px(4.)).child(ghost_button(
            theme,
            "vault-editor-change",
            "Change vault…",
            cx.listener(|this, _event, _window, cx| this.open_wizard(cx)),
        )));
        panel
    }

    /// The ChangeVault wizard overlay ("Choose where to store credentials",
    /// `ChangeVault-4b860408.js:540`).
    fn wizard_overlay(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        let target = self
            .selected_vault(cx)
            .as_ref()
            .map(vault_name)
            .unwrap_or_else(|| "this vault".to_owned());
        let scrim = gpui::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.55 };

        let mut options = div().flex().flex_col().gap(px(10.)).w_full();
        for mode in ChangeVaultMode::ALL {
            let selected = self.wizard_mode == mode;
            options = options.child(radio_option(theme, mode, &target, selected, cx));
        }

        let subtitle = format!(
            "Select how your teammates will access the items {} {}.",
            change_vault_operation(),
            target
        );

        let card = div()
            .id("vault-wizard-card")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.))
            .w(px(WIZARD_WIDTH))
            .p(px(40.))
            .rounded(px(theme.corner_radius_large))
            .bg(theme.card_a)
            .text_color(theme.title)
            .child(
                div()
                    .font_family(UI_FONT)
                    .text_size(px(24.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.title)
                    .child(SharedString::from("Choose where to store credentials")),
            )
            .child(
                text::R14S
                    .style(div())
                    .text_color(theme.text_common)
                    .child(SharedString::from(subtitle)),
            )
            .child(options)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .w_full()
                    .pt(px(4.))
                    .child(Button::new("Cancel").secondary().on_click(
                        theme,
                        cx.listener(|this, _event, _window, cx| this.close_wizard(cx)),
                    ))
                    .child(Button::new("Continue").primary().on_click(
                        theme,
                        cx.listener(|this, _event, _window, cx| this.submit_wizard(cx)),
                    )),
            )
            // Swallow clicks on the card background so they never reach the
            // scrim below (which dismisses the wizard).
            .on_click(cx.listener(|_this, _event, _window, _cx| {}));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(scrim)
                    .id("vault-wizard-scrim")
                    .on_click(cx.listener(|this, _event, _window, cx| this.close_wizard(cx))),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(card),
            )
    }
}

impl Render for KeychainScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // The primary action is the filter band's first child — no title band.
        let actions = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(new_vault_button(
                theme,
                cx.listener(|this, _event, _window, cx| this.new_vault_note(cx)),
            ))
            .child(
                Button::new("Change vault…")
                    .ghost()
                    .size(ButtonSize::Small)
                    .disabled(self.selected.is_none())
                    .on_click(
                        theme,
                        cx.listener(|this, _event, _window, cx| this.open_wizard(cx)),
                    ),
            );
        let header = FiltersHeader::new("Search vaults").action(actions).element(theme);

        let list = self.vault_list(theme, cx);
        let mut body = div().flex().flex_1().min_h(px(0.)).child(list);
        if self.selected.is_some() {
            body = body.child(self.vault_editor(theme, cx));
        }

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(header)
            .child(body);

        if self.wizard_open {
            root = root.child(self.wizard_overlay(theme, cx));
        }
        root
    }
}

/// The Vaults management screen, ready to drop into the shell's routed column
/// (see the module docs for the call contract).
pub fn keychain_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<KeychainScreen> {
    let cached = cx
        .try_global::<KeychainScreenHost>()
        .and_then(|host| host.0.clone());
    if let Some(screen) = cached {
        // Reuse the live view (it owns the selection/wizard state) as long as
        // it watches the same state entity. Reading it here is safe: only
        // `TermiusState` is leased at this point, never the screen itself.
        if screen.read(cx).state == state {
            return screen;
        }
    }
    let screen = cx.new(|_cx| KeychainScreen::new(state));
    cx.set_global(KeychainScreenHost(Some(screen.clone())));
    screen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(id: &str, title: &str) -> Keychain {
        Keychain {
            id: id.into(),
            title: title.into(),
            ..Keychain::default()
        }
    }

    #[test]
    fn vault_names_follow_get_vault_name() {
        // The default / id-less vault gets the " vault" suffix.
        assert_eq!(vault_name(&vault("", "Team")), "Team vault");
        assert_eq!(vault_name(&vault("", "")), "Personal vault");
        // A named vault keeps its name; unnamed named vaults are "Unnamed".
        assert_eq!(vault_name(&vault("v1", "Production")), "Production");
        assert_eq!(vault_name(&vault("v1", "  ")), "Unnamed");
    }

    #[test]
    fn stats_summary_lists_per_type_counts() {
        let counts = VaultCounts {
            hosts: 3,
            snippets: 1,
            keys: 2,
            identities: 0,
            forwards: 4,
        };
        assert_eq!(
            vault_stats_summary(&counts),
            "3 hosts · 1 snippet · 2 keys · 4 rules"
        );
        assert_eq!(vault_stats_summary(&VaultCounts::default()), "Empty");
        assert!(VaultCounts::default().is_empty());
        assert!(!counts.is_empty());
    }

    #[test]
    fn push_count_pluralises() {
        let mut parts = Vec::new();
        push_count(&mut parts, 0, "host", "hosts");
        push_count(&mut parts, 1, "host", "hosts");
        push_count(&mut parts, 2, "host", "hosts");
        assert_eq!(parts, vec!["1 host".to_owned(), "2 hosts".to_owned()]);
    }

    #[test]
    fn change_vault_modes_match_the_wizard() {
        assert_eq!(ChangeVaultMode::ALL.len(), 3);
        assert_eq!(
            ChangeVaultMode::NoCredentialsSharing.label("Team vault"),
            "Personal vault"
        );
        assert_eq!(
            ChangeVaultMode::CredentialsSharing.label("Team vault"),
            "Team vault"
        );
        assert_eq!(ChangeVaultMode::Multikey.label("Team vault"), "Multikey");
        assert!(ChangeVaultMode::CredentialsSharing.body().contains("shared"));
        assert_eq!(change_vault_operation(), "moving to");
    }

    #[test]
    fn vault_and_stat_icons_are_bundled() {
        assert!(crate::has_icon(VAULT_ICON));
        assert!(crate::has_icon(VAULT_ICON_SELECTED));
        assert!(crate::has_icon("host.svg"));
        assert!(crate::has_icon("snippet.svg"));
        assert!(crate::has_icon("key.svg"));
        assert!(crate::has_icon("Identity.svg"));
        assert!(crate::has_icon("PortForwarding.svg"));
        assert!(crate::has_icon("plusThin.svg"));
        assert!(crate::has_icon("checked.svg"));
        assert!(crate::has_icon("unchecked.svg"));
    }
}
