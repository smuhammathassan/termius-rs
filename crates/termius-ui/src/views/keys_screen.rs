//! keys_screen — the **Keychain › Keys** screen: MultiKeys / Keys / Identities
//! in one list plus the Add-Key dialog body.
//!
//! Reconstruction of Termius v10's Keychain section (`analysis/recon/21-keys.md`):
//!
//! * **One screen, three lists.** The original `BD` component renders a single
//!   screen whose `EntityLists` hold `MultiKeys`, `Keys` and `Identities`
//!   (`_main.js:72396`). The port previously showed a flat "Keys" list plus a
//!   fabricated "Security Keys" card — both replaced.
//! * **No title band.** The screen mounts the shared [`FiltersHeader`] (the 45px
//!   search/filter band); the primary action is its *first child* and the title
//!   comes from the nav (`_main.js:72461`). The port's 56px `B16P` "Keys" band is
//!   removed.
//! * **Rich toolbar** (`K6e`, `_main.js:67834`): a split `New key` button whose
//!   main half carries the thin plus `SvgPlusThin` (14×14 — *not*
//!   `addCircle.svg`) and whose chevron menu holds `Generate key` and
//!   `New Identity`, followed by `Certificate`, `Touch ID` / `Windows Hello` and
//!   `FIDO2` ghost buttons.
//! * **Rows = [`EntityRow`].** The shared `GridItemPresenter` cell: a 40×40
//!   `ShapedIcon` tile, a 14px title over an 11px `--text-secondary` subtitle
//!   (`/tmp/reconnectSaga.js:131264`). Presenters: `KeyPresenter` (`fI`),
//!   `IdentityPresenter` (`g6`), `MultiKeyPresenter` (`f_`).
//! * **New-Key form.** The original `h7e` slider is a `Paste / Import /
//!   Generate` selector over the `FD` key form (`_main.js:68933`, `:68418`); the
//!   dialog renders that structure.
//!
//! # Honest gaps (PORT-TODO)
//!
//! * [`Key`] carries no SEP marker and [`Library`] has no MultiKey records, so
//!   [`is_sep_key`] is always false and [`multikey_rows`] is empty — the
//!   `MultiKeys` list renders its header and a note, and the presenters are kept
//!   so the structure is real and ready.
//! * [`InputField`] is display-only (see `primitives`), so the form's text lives
//!   in the [`KeysUi`] draft global and only the chips can change it today;
//!   `Save` writes a *sample* [`Key`].
//! * gpui 0.2.2 has no anchored popup host, so the split-button menu renders as
//!   an inline panel under the [`FiltersHeader`], and the row's hover ⋯
//!   `ItemMenu` (Edit / Duplicate / Share / Remove) is not implemented.
//! * The `Certificate`, `Touch ID` and `New Identity` actions have no backing
//!   form yet (the `Dialog` enum has only `AddKey`); they note the gap in the
//!   status bar.
//!
//! # Wiring (owned by `views`, not this module)
//!
//! ```ignore
//! // Section::Keys center panel — store the entity once (see `AppShell`):
//! views::keys_screen::keys_screen(state.clone(), cx)  // -> Entity<KeysScreen>
//! // Add-Key dialog body (host it via `DialogFrame::child`, which supplies the
//! // frame; this body carries its own action row):
//! views::keys_screen::key_dialog_body(&dialog, state, cx)
//! ```
//!
//! The factory only mints the [`KeysScreen`] view entity, so it is safe to call
//! while `TermiusState` is leased (inside `state.update(..)`); the view reads the
//! library in its own `render` and drives [`TermiusState`] through
//! `Entity::update`, exactly like `views::host_list`. [`key_dialog_body`] still
//! takes a `&TermiusState` + its `Context`, so the shell calls it from inside
//! `state.update(..)`.

use gpui::{
    div, px, AnyElement, App, AppContext as _, BorrowAppContext as _, ClickEvent, Context, Div,
    Entity, FontWeight, Global, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Rgba, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};
use termius_core::{Identity, Key, KeyType};

use crate::app_state::{Dialog, Library, TermiusState};
use crate::primitives::{
    Button, EntityRow, FiltersHeader, InputField, SectionHeader, SettingsText,
};
use crate::theme::{over, theme_of, with_alpha, TermiusTheme, UI_FONT};

// --- icons (exact bundled asset names) -------------------------------------
/// The default SSH-key entity icon (`entityIcons.key`).
const KEY_ICON: &str = "key.svg";
/// The hardware (FIDO2/SK) key icon (`entityIcons.hardware_key`).
const HARDWARE_KEY_ICON: &str = "hardware_key.svg";
/// The secure-enclave key icon (`entityIcons.sep_key`).
const SEP_KEY_ICON: &str = "sep_key.svg";
/// The unsaved secure-enclave key icon (`entityIcons.unsaved_sep_key`).
const UNSAVED_SEP_KEY_ICON: &str = "unsaved_sep_key.svg";
/// The identity entity icon (`entityIcons.identity`).
const IDENTITY_ICON: &str = "Identity.svg";
/// The MultiKey entity icon (`entityIcons.multikey`).
const MULTIKEY_ICON: &str = "multiKey.svg";
/// The `Certificate` toolbar glyph.
const CERTIFICATE_ICON: &str = "certificate.svg";
/// The `FIDO2` toolbar glyph.
const FIDO_ICON: &str = "fido.svg";
/// The `Touch ID` toolbar glyph.
const TOUCH_ID_ICON: &str = "touch-icon.svg";
/// The toolbar's thin plus (`SvgPlusThin`, 14×14 — *not* `addCircle.svg`).
const PLUS_ICON: &str = "plusThin.svg";
/// The split button's menu chevron (`da_1` arrow).
const CHEVRON_ICON: &str = "chevron.svg";

// --- layout metrics (from the original CSS) --------------------------------
/// Termius' `medium` button height (the toolbar buttons live in the 45px band).
const BUTTON_HEIGHT: f32 = 30.0;
/// `--corner-radius-small-medium` (the medium button radius).
const BUTTON_RADIUS: f32 = 8.0;
/// The `Private key` textarea's minimum height (`minRows: 6`).
const PRIVATE_KEY_MIN_HEIGHT: f32 = 92.0;
/// White used for text/glyphs on the accent button fill.
const ON_ACCENT: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

// ---------------------------------------------------------------------------
// Presenters (pure; unit-tested below)
// ---------------------------------------------------------------------------

/// The `(label, description, icon)` triple a presenter yields for one row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RowPresenter {
    label: String,
    description: String,
    icon: &'static str,
}

/// Termius' lowercase key-type description (`entity.type`, e.g. `ed25519`).
fn key_type_description(key_type: KeyType) -> &'static str {
    match key_type {
        KeyType::Rsa => "rsa",
        KeyType::Dsa => "dsa",
        KeyType::Ecdsa => "ecdsa",
        KeyType::Ed25519 => "ed25519",
        KeyType::X509 => "x509",
    }
}

/// Termius' uppercase key-type label for the Generate chooser (`KEY_NAMES`).
fn key_type_label(key_type: KeyType) -> &'static str {
    match key_type {
        KeyType::Rsa => "RSA",
        KeyType::Dsa => "DSA",
        KeyType::Ecdsa => "ECDSA",
        KeyType::Ed25519 => "ED25519",
        KeyType::X509 => "X509",
    }
}

/// Whether the key material is a FIDO2 / hardware (SK) key.
///
/// Mirrors `isSKKey(private_key)`: OpenSSH marks security keys with the
/// `sk-ssh-…` / `sk-ecdsa-…` algorithm prefixes.
fn is_sk_key(private_key: Option<&str>) -> bool {
    private_key
        .map(|material| material.contains("sk-ssh-") || material.contains("sk-ecdsa-"))
        .unwrap_or(false)
}

/// Whether the key is a secure-enclave (SEP / Touch ID) key.
///
/// PORT-TODO: [`Key`] has no SEP marker, so this is always false today; it is
/// kept so [`key_presenter`] already carries the `sep_key` / `unsaved_sep_key`
/// branches from `KeyPresenter.icon(..)`.
fn is_sep_key(_key: &Key) -> bool {
    false
}

/// `KeyPresenter` (`fI`, `/tmp/reconnectSaga.js:74160`): label = `entity.label`,
/// description = `entity.type`, icon `sep_key` / `unsaved_sep_key` /
/// `hardware_key` / `key`.
fn key_presenter(key: &Key) -> RowPresenter {
    let icon = if is_sep_key(key) {
        if key.id.is_empty() {
            UNSAVED_SEP_KEY_ICON
        } else {
            SEP_KEY_ICON
        }
    } else if is_sk_key(key.private_key.as_deref()) {
        HARDWARE_KEY_ICON
    } else {
        KEY_ICON
    };
    RowPresenter {
        label: key.name.clone(),
        description: key_type_description(key.key_type).to_owned(),
        icon,
    }
}

/// `IdentityPresenter` (`g6`, `/tmp/reconnectSaga.js:74062`): label =
/// `label || username`, description = the authentication method, icon
/// `identity`.
fn identity_presenter(identity: &Identity) -> RowPresenter {
    let label = if identity.name.trim().is_empty() {
        identity.username.clone()
    } else {
        identity.name.clone()
    };
    RowPresenter {
        label,
        description: identity_auth_method(identity).to_owned(),
        icon: IDENTITY_ICON,
    }
}

/// The identity's authentication method (`"Password"` / `"Keyboard
/// Interactive"` / `"Public Key"`), derived from its bindings.
fn identity_auth_method(identity: &Identity) -> &'static str {
    if identity.key_id.is_some() {
        "Public Key"
    } else if identity.password.is_some() || identity.keychain_id.is_some() {
        "Password"
    } else {
        "Keyboard Interactive"
    }
}

/// One MultiKey row (see the module's PORT-TODO: [`Library`] has no MultiKey
/// records yet).
#[allow(dead_code)] // PORT-TODO: constructed once MultiKeys are modeled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MultiKeyRow {
    team_name: String,
}

/// `MultiKeyPresenter` (`f_`, `/tmp/reconnectSaga.js:74114`): label =
/// `` `${teamName} MultiKey` ``, description `ECDSA`, icon `multikey`.
#[allow(dead_code)] // PORT-TODO: driven by [`multikey_rows`] once modeled.
fn multi_key_presenter(row: &MultiKeyRow) -> RowPresenter {
    RowPresenter {
        label: format!("{} MultiKey", row.team_name),
        description: "ECDSA".to_owned(),
        icon: MULTIKEY_ICON,
    }
}

/// The `MultiKeys` list.
///
/// PORT-TODO: [`Library`] has no MultiKey records, so this is empty and the
/// screen renders the `MultiKeys` header with a note.
#[allow(dead_code)] // PORT-TODO: returns empty until MultiKeys are modeled.
fn multikey_rows(_library: &Library) -> Vec<MultiKeyRow> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Ephemeral screen state (gpui global; the dialog body is a free function)
// ---------------------------------------------------------------------------

/// The New-Key form's `Paste / Import / Generate` selector (`gx_1`,
/// `_main.js:68933`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum KeyFormMode {
    #[default]
    Paste,
    Import,
    Generate,
}

impl KeyFormMode {
    /// The selector label.
    fn label(self) -> &'static str {
        match self {
            Self::Paste => "Paste",
            Self::Import => "Import",
            Self::Generate => "Generate",
        }
    }

    /// The three selector items, in the original order.
    const ALL: [KeyFormMode; 3] = [KeyFormMode::Paste, KeyFormMode::Import, KeyFormMode::Generate];
}

/// The Keys screen's ephemeral Add-Key form draft.
///
/// Kept as a gpui [`Global`] because the dialog body is a free function with no
/// view entity of its own; listeners mutate it through
/// `update_default_global` + [`Context::notify`], which repaints the shell (it
/// observes the `TermiusState` entity the notify targets).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeysUi {
    /// Draft "Label" field.
    name: String,
    /// Draft key type — the one value the chooser can actually change today.
    key_type: KeyType,
    /// Draft private-key material (never persisted by the stub).
    private_key: String,
    /// Draft passphrase (never persisted here; keychain wiring is deferred).
    passphrase: String,
    /// The selected Paste / Import / Generate tab.
    mode: KeyFormMode,
}

impl Default for KeysUi {
    fn default() -> Self {
        Self {
            name: String::new(),
            // Matches `Key::default()` (not `KeyType::default()`, which is
            // `Rsa`): a fresh key starts out Ed25519.
            key_type: KeyType::Ed25519,
            private_key: String::new(),
            passphrase: String::new(),
            mode: KeyFormMode::Paste,
        }
    }
}

impl Global for KeysUi {}

impl KeysUi {
    /// Clear the form draft back to a fresh Paste tab.
    fn reset_draft(&mut self) {
        self.name.clear();
        self.key_type = KeyType::Ed25519;
        self.private_key.clear();
        self.passphrase.clear();
        self.mode = KeyFormMode::Paste;
    }
}

/// The current screen draft (a fresh default when the global was never set).
fn draft_of(cx: &App) -> KeysUi {
    match cx.try_global::<KeysUi>() {
        Some(ui) => ui.clone(),
        None => KeysUi::default(),
    }
}

/// The [`Key`] record `Save` writes.
///
/// PORT-TODO: `InputField` is display-only, so `name` / `private_key` /
/// `passphrase` only become reachable once the form grows a real content model;
/// until then an unnamed draft becomes `Key N` and the saved key has no
/// material or timestamps.
fn build_key(draft: KeysUi, existing: usize) -> Key {
    let name = if draft.name.trim().is_empty() {
        format!("Key {}", existing + 1)
    } else {
        draft.name
    };
    Key {
        name,
        key_type: draft.key_type,
        private_key: if draft.private_key.is_empty() {
            None
        } else {
            Some(draft.private_key)
        },
        ..Key::default()
    }
}

// ---------------------------------------------------------------------------
// Toolbar chrome (local; the primitives carry no icon slot)
// ---------------------------------------------------------------------------

/// The primary (accent) toolbar button with a leading icon — the `useB0` main
/// half / `Button$1` `color:"accent" size:"medium"`.
fn primary_icon_button(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    label: &'static str,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover_fill = over(theme.primary, with_alpha(ON_ACCENT, 0.25));
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .h(px(BUTTON_HEIGHT))
        .px(px(10.))
        .rounded(px(BUTTON_RADIUS))
        .bg(theme.primary)
        .text_color(ON_ACCENT)
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(21.))
        .whitespace_nowrap()
        .child(crate::icon(icon_name).w(px(14.)).h(px(14.)))
        .child(SharedString::from(label))
        .hover(move |hover| hover.bg(hover_fill))
        .on_click(listener)
}

/// A `ghost` toolbar button with a leading icon (`Button$1` `variant:"ghost"`,
/// `size:"medium"`).
fn ghost_icon_button(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    label: &'static str,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover_fill = with_alpha(theme.primary, 0.15);
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .h(px(BUTTON_HEIGHT))
        .px(px(10.))
        .rounded(px(BUTTON_RADIUS))
        .text_color(theme.primary)
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(21.))
        .whitespace_nowrap()
        .child(crate::icon(icon_name).w(px(14.)).h(px(14.)))
        .child(SharedString::from(label))
        .hover(move |hover| hover.bg(hover_fill))
        .on_click(listener)
}

/// The 30×30 chevron half of the split button (`useB0.menuButton`).
fn chevron_button(
    theme: TermiusTheme,
    id: &'static str,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover_fill = over(theme.primary, with_alpha(ON_ACCENT, 0.25));
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(BUTTON_HEIGHT))
        .h(px(BUTTON_HEIGHT))
        .flex_shrink_0()
        .rounded(px(BUTTON_RADIUS))
        .bg(theme.primary)
        .text_color(ON_ACCENT)
        .child(crate::icon(CHEVRON_ICON).w(px(10.)).h(px(6.)))
        .hover(move |hover| hover.bg(hover_fill))
        .on_click(listener)
}

/// One row of a toolbar split menu.
fn menu_item(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    label: &'static str,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(BUTTON_HEIGHT))
        .px(px(10.))
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.title)
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(21.))
        .whitespace_nowrap()
        .child(crate::icon(icon_name).w(px(14.)).h(px(14.)).text_color(theme.text_common))
        .child(SharedString::from(label))
        .hover(move |hover| hover.bg(theme.hover))
        .on_click(listener)
}

/// A muted single-line note under a list header.
fn muted_note(theme: TermiusTheme, note: &'static str) -> Div {
    div()
        .px(px(12.))
        .py(px(6.))
        .font_family(UI_FONT)
        .text_size(px(12.))
        .font_weight(FontWeight::NORMAL)
        .line_height(px(18.))
        .text_color(theme.text_common)
        .child(SharedString::from(note))
}

/// One selectable chip in the dialog (the Paste/Import/Generate and key-type
/// selectors, both `gx_1` `ButtonRowSelector`s).
fn selector_chip(
    theme: TermiusTheme,
    id: String,
    label: &'static str,
    selected: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut chip = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px(px(12.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(if selected { theme.primary } else { theme.border })
        .text_size(px(14.))
        .text_color(if selected { ON_ACCENT } else { theme.foreground });
    if selected {
        chip = chip.bg(theme.primary);
    }
    chip.child(SharedString::from(label)).on_click(listener)
}

/// A non-selectable chip — a key type the original `KEY_NAMES` lists but the
/// port's [`KeyType`] model does not cover yet (PORT-TODO).
fn disabled_chip(theme: TermiusTheme, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px(px(12.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(theme.border)
        .text_size(px(14.))
        .text_color(theme.text_common)
        .child(SharedString::from(label))
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Keys screen view: the shared [`FiltersHeader`] (with the rich toolbar)
/// over the `MultiKeys` / `Keys` / `Identities` entity lists, or an
/// [`EmptyState`](crate::primitives::EmptyState) when there is nothing yet.
///
/// A separate entity from [`TermiusState`] (gpui leases one entity at a time):
/// the factory below only mints it, [`Render::render`] reads the library
/// through the handle, and every click drives `TermiusState` through
/// `Entity::update` — the `views::host_list` pattern.
pub struct KeysScreen {
    state: Entity<TermiusState>,
    /// The highlighted row id (the click-to-select stand-in for the editor).
    selected: Option<String>,
    /// Whether the split button's menu is showing.
    menu_open: bool,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Keys screen for `state`.
///
/// Safe to call while `TermiusState` is leased (inside `state.update(..)`): it
/// only mints a view entity, so no `Entity::read` happens there. Store the
/// returned entity once (the same way `AppShell` holds `HostList`) — calling
/// this every render would mint a new entity per frame.
pub fn keys_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<KeysScreen> {
    cx.new(|cx| KeysScreen::new(state, cx))
}

impl KeysScreen {
    /// Create the screen and subscribe it to state + theme changes.
    fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let observe_theme = cx.observe_global::<TermiusTheme>(|_, cx| cx.notify());
        Self {
            state,
            selected: None,
            menu_open: false,
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// Raise the Add-Key dialog in `mode` (the shell hosts it above everything).
    fn open_add_key(&mut self, mode: KeyFormMode, cx: &mut Context<Self>) {
        self.menu_open = false;
        cx.update_default_global::<KeysUi, _>(|ui, _cx| ui.mode = mode);
        self.state.update(cx, |state, cx| state.open_dialog(Dialog::AddKey, cx));
    }

    /// Toggle the split button's menu.
    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        self.menu_open = !self.menu_open;
        cx.notify();
    }

    /// Select one row (the click-to-select stand-in for the editor).
    fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selected = Some(id.to_owned());
        cx.notify();
    }

    /// Note an unimplemented toolbar action in the status bar.
    fn note(&mut self, text: &'static str, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.state.update(cx, |state, cx| {
            state.status_text = text.to_owned();
            cx.notify();
        });
    }

    /// Surface the FIDO2 note in the status bar.
    fn show_fido_note(&mut self, cx: &mut Context<Self>) {
        self.note("FIDO2 keys are deferred to the termius-fido crate.", cx);
    }

    /// The split `New key` button (main + chevron halves).
    fn new_key_button(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        let main = primary_icon_button(
            theme,
            "keys-new",
            PLUS_ICON,
            "New key",
            cx.listener(|this, _event, _window, cx| {
                this.open_add_key(KeyFormMode::Paste, cx);
            }),
        );
        let chevron = chevron_button(
            theme,
            "keys-new-menu",
            cx.listener(|this, _event, _window, cx| this.toggle_menu(cx)),
        );
        div().flex().items_center().gap(px(2.)).child(main).child(chevron)
    }

    /// One entity row wired to select `id`.
    fn entity_row(
        presenter: &RowPresenter,
        id: &str,
        selected: bool,
        theme: TermiusTheme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let row_id = id.to_owned();
        EntityRow::new(
            presenter.label.clone(),
            presenter.description.clone(),
            presenter.icon,
        )
        .selected(selected)
        .on_click(
            theme,
            cx.listener(move |this, _event, _window, cx| {
                this.select(&row_id, cx);
            }),
        )
    }
}

impl Render for KeysScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Read the library once and clone out what the rows need.
        let (keys, identities, multikeys) = {
            let state = self.state.read(cx);
            (
                state.library.keys.clone(),
                state.library.identities.clone(),
                multikey_rows(&state.library),
            )
        };
        let empty = keys.is_empty() && identities.is_empty() && multikeys.is_empty();

        // Header: the shared filter band, primary action first (no title band).
        let toolbar = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.))
            .child(self.new_key_button(theme, cx))
            .child(ghost_icon_button(
                theme,
                "keys-certificate",
                CERTIFICATE_ICON,
                "Certificate",
                cx.listener(|this, _event, _window, cx| {
                    this.note(
                        "Certificate — the certificate form arrives with the SSH wave.",
                        cx,
                    );
                }),
            ))
            // The original swaps this for `Windows Hello` on Windows
            // (`_main.js:67892`); backend detection is a PORT-TODO.
            .child(ghost_icon_button(
                theme,
                "keys-touch-id",
                TOUCH_ID_ICON,
                "Touch ID",
                cx.listener(|this, _event, _window, cx| {
                    this.note(
                        "Touch ID — the secure-enclave form arrives with the biometric wave.",
                        cx,
                    );
                }),
            ))
            .child(ghost_icon_button(
                theme,
                "keys-fido2",
                FIDO_ICON,
                "FIDO2",
                cx.listener(|this, _event, _window, cx| this.show_fido_note(cx)),
            ));
        let header = FiltersHeader::new("Search keys").action(toolbar).element(theme);

        let mut body = div()
            .id("keys-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .py(px(4.));

        if empty {
            // The original `L6e` empty card.
            body = body.child(
                crate::primitives::EmptyState::new(
                    "Add credentials",
                    "Store your credentials to quick and securely access your servers.",
                )
                .element(theme),
            );
        } else {
            // Level 1 — MultiKeys.
            body = body.child(SectionHeader::new("MultiKeys").element(theme));
            if multikeys.is_empty() {
                body = body.child(muted_note(theme, "No MultiKeys yet."));
            } else {
                for row in &multikeys {
                    let presenter = multi_key_presenter(row);
                    body = body.child(Self::entity_row(
                        &presenter,
                        &row.team_name,
                        self.selected.as_deref() == Some(row.team_name.as_str()),
                        theme,
                        cx,
                    ));
                }
            }

            // Level 2 — Keys.
            body = body.child(SectionHeader::new("Keys").element(theme));
            if keys.is_empty() {
                body = body.child(muted_note(theme, "No keys yet. Add one with New key."));
            } else {
                for key in &keys {
                    let presenter = key_presenter(key);
                    body = body.child(Self::entity_row(
                        &presenter,
                        &key.id,
                        self.selected.as_deref() == Some(key.id.as_str()),
                        theme,
                        cx,
                    ));
                }
            }

            // Level 3 — Identities.
            body = body.child(SectionHeader::new("Identities").element(theme));
            if identities.is_empty() {
                body = body.child(muted_note(theme, "No identities yet."));
            } else {
                for identity in &identities {
                    let presenter = identity_presenter(identity);
                    body = body.child(Self::entity_row(
                        &presenter,
                        &identity.id,
                        self.selected.as_deref() == Some(identity.id.as_str()),
                        theme,
                        cx,
                    ));
                }
            }
        }

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .text_color(theme.foreground)
            .child(header);

        // The split menu (inline: gpui 0.2.2 has no anchored popup host).
        if self.menu_open {
            root = root.child(
                div().px(px(12.)).pb(px(4.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .p(px(4.))
                        .rounded(px(theme.corner_radius_small))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.card_a)
                        .child(menu_item(
                            theme,
                            "keys-generate",
                            KEY_ICON,
                            "Generate key",
                            cx.listener(|this, _event, _window, cx| {
                                this.open_add_key(KeyFormMode::Generate, cx);
                            }),
                        ))
                        .child(menu_item(
                            theme,
                            "keys-new-identity",
                            IDENTITY_ICON,
                            "New Identity",
                            cx.listener(|this, _event, _window, cx| {
                                this.note(
                                    "New Identity — the identity form arrives with the SSH wave.",
                                    cx,
                                );
                            }),
                        )),
                ),
            );
        }

        root.child(body)
    }
}

// ---------------------------------------------------------------------------
// The Add-Key dialog body
// ---------------------------------------------------------------------------

/// The `Add Key` dialog body; `None` for every other dialog (their screens
/// fill those in).
///
/// The returned element is meant for [`DialogFrame::child`](crate::primitives::DialogFrame)
/// (the frame supplies the card and title; this body carries its own action
/// row). The structure mirrors the original `h7e` slider: a `Paste / Import /
/// Generate` selector (`_main.js:68933`) over the `FD` key form's Label /
/// Passphrase / Private key fields (`_main.js:68481`) with the port's key-type
/// chooser.
pub fn key_dialog_body(
    dialog: &Dialog,
    state: &TermiusState,
    cx: &mut Context<TermiusState>,
) -> Option<AnyElement> {
    if *dialog != Dialog::AddKey {
        return None;
    }
    let theme = theme_of(cx);
    let draft = draft_of(cx);
    // The name `Save` assigns while the draft's Label field is untouched.
    let default_name = format!("Key {}", state.library.keys.len() + 1);

    // ----- Paste / Import / Generate selector -----
    let mut modes = div().flex().gap(px(6.));
    for mode in KeyFormMode::ALL {
        modes = modes.child(selector_chip(
            theme,
            format!("key-mode-{}", mode.label()),
            mode.label(),
            draft.mode == mode,
            cx.listener(move |_this, _event, _window, cx| {
                cx.update_default_global::<KeysUi, _>(|ui, cx| {
                    ui.mode = mode;
                    cx.notify();
                });
            }),
        ));
    }

    // ----- key-type chooser (ED25519 / ECDSA / RSA / MLDSA) -----
    let mut types = div().flex().gap(px(6.));
    for key_type in [KeyType::Ed25519, KeyType::Ecdsa, KeyType::Rsa] {
        types = types.child(selector_chip(
            theme,
            format!("key-type-{}", key_type_label(key_type)),
            key_type_label(key_type),
            draft.key_type == key_type,
            cx.listener(move |_this, _event, _window, cx| {
                cx.update_default_global::<KeysUi, _>(|ui, cx| {
                    ui.key_type = key_type;
                    cx.notify();
                });
            }),
        ));
    }
    // The original `KEY_NAMES` also lists MLDSA; the port's `KeyType` has no
    // such variant yet (PORT-TODO), so it renders as a disabled chip.
    types = types.child(disabled_chip(theme, "MLDSA"));

    let private_hint = if draft.private_key.is_empty() {
        "Paste an OpenSSH or PEM private key…".to_owned()
    } else {
        draft.private_key.clone()
    };

    let cancel = Button::new("Cancel").secondary().on_click(
        theme,
        cx.listener(|this, _event, _window, cx| {
            this.close_dialog(cx);
        }),
    );
    // Reads the draft from the global inside the listener, so it always saves
    // the latest chips/fields rather than the render-time copy.
    let save = Button::new("Save").primary().on_click(
        theme,
        cx.listener(|this, _event, _window, cx| {
            let key = build_key(draft_of(cx), this.library.keys.len());
            this.add_key(key, cx);
            this.close_dialog(cx);
            cx.update_default_global::<KeysUi, _>(|ui, cx| {
                ui.reset_draft();
                cx.notify();
            });
        }),
    );

    let body = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(field_label(theme, "Type"))
        .child(modes)
        .child(
            InputField::new("Label", draft.name.clone())
                .placeholder(default_name)
                .element(theme),
        )
        .child(field_label(theme, "Key type"))
        .child(types)
        .child(
            InputField::new("Passphrase", draft.passphrase.clone())
                .placeholder("Leave empty if unprotected")
                .element(theme),
        )
        .child(field_label(theme, "Private key"))
        .child(
            div()
                .flex()
                .flex_col()
                .min_h(px(PRIVATE_KEY_MIN_HEIGHT))
                .p(px(10.))
                .rounded(px(theme.corner_radius_small))
                .border_1()
                .border_color(theme.border_basic)
                .bg(theme.card_c)
                .child(SettingsText::new(private_hint).element(theme)),
        )
        .child(SettingsText::new("The passphrase is never stored in the key record.").element(theme))
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .child(cancel)
                .child(save),
        );
    Some(body.into_any_element())
}

/// A muted group label above a dialog control.
fn field_label(theme: TermiusTheme, label: &'static str) -> Div {
    div()
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::NORMAL)
        .line_height(px(21.))
        .text_color(theme.text_common)
        .child(SharedString::from(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_type_labels_match_termius() {
        assert_eq!(key_type_label(KeyType::Rsa), "RSA");
        assert_eq!(key_type_label(KeyType::Dsa), "DSA");
        assert_eq!(key_type_label(KeyType::Ecdsa), "ECDSA");
        assert_eq!(key_type_label(KeyType::Ed25519), "ED25519");
        assert_eq!(key_type_label(KeyType::X509), "X509");
    }

    #[test]
    fn key_type_descriptions_are_lowercase() {
        assert_eq!(key_type_description(KeyType::Rsa), "rsa");
        assert_eq!(key_type_description(KeyType::Ecdsa), "ecdsa");
        assert_eq!(key_type_description(KeyType::Ed25519), "ed25519");
    }

    #[test]
    fn key_presenter_uses_name_and_algorithm() {
        let key = Key { name: "deploy".into(), ..Key::default() };
        let presenter = key_presenter(&key);
        assert_eq!(presenter.label, "deploy");
        assert_eq!(presenter.description, "ed25519");
        assert_eq!(presenter.icon, KEY_ICON);

        let rsa = Key { key_type: KeyType::Rsa, ..Key::default() };
        assert_eq!(key_presenter(&rsa).description, "rsa");
    }

    #[test]
    fn key_presenter_detects_hardware_keys() {
        let sk = Key {
            name: "yubi".into(),
            private_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----\nsk-ssh-ed25519-...".into()),
            ..Key::default()
        };
        assert!(is_sk_key(sk.private_key.as_deref()));
        assert_eq!(key_presenter(&sk).icon, HARDWARE_KEY_ICON);

        let plain = Key { name: "plain".into(), ..Key::default() };
        assert!(!is_sk_key(plain.private_key.as_deref()));
        assert_eq!(key_presenter(&plain).icon, KEY_ICON);
    }

    #[test]
    fn identity_presenter_falls_back_to_username_and_reports_auth() {
        let mut identity = Identity {
            name: "prod".into(),
            username: "root".into(),
            ..Identity::default()
        };
        let presenter = identity_presenter(&identity);
        assert_eq!(presenter.label, "prod");
        assert_eq!(presenter.description, "Keyboard Interactive");
        assert_eq!(presenter.icon, IDENTITY_ICON);

        // A name-less identity shows the username.
        identity.name = "  ".into();
        assert_eq!(identity_presenter(&identity).label, "root");

        // Bindings pick the method.
        identity.key_id = Some("key-1".into());
        assert_eq!(identity_auth_method(&identity), "Public Key");
        let mut with_password = Identity { password: Some("x".into()), ..Identity::default() };
        assert_eq!(identity_auth_method(&with_password), "Password");
        with_password.keychain_id = Some("kc".into());
        assert_eq!(identity_auth_method(&with_password), "Password");
    }

    #[test]
    fn multi_key_presenter_names_the_team() {
        let presenter = multi_key_presenter(&MultiKeyRow { team_name: "Acme".into() });
        assert_eq!(presenter.label, "Acme MultiKey");
        assert_eq!(presenter.description, "ECDSA");
        assert_eq!(presenter.icon, MULTIKEY_ICON);
    }

    #[test]
    fn multikey_rows_is_empty_until_modeled() {
        assert!(multikey_rows(&Library::default()).is_empty());
    }

    #[test]
    fn save_builds_a_sample_key_from_the_draft() {
        let key = build_key(KeysUi::default(), 0);
        assert_eq!(key.name, "Key 1");
        assert_eq!(key.key_type, KeyType::Ed25519);
        assert!(key.private_key.is_none());
        assert!(key.public_key.is_none());
        assert!(key.keychain_id.is_none());
        assert!(key.passphrase_hint.is_none());

        let draft = KeysUi {
            name: "prod".into(),
            key_type: KeyType::Rsa,
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
            passphrase: "hunter2".into(),
            mode: KeyFormMode::Generate,
        };
        let key = build_key(draft, 3);
        assert_eq!(key.name, "prod");
        assert_eq!(key.key_type, KeyType::Rsa);
        assert_eq!(
            key.private_key.as_deref(),
            Some("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        // The passphrase itself never lands in the record (keychain wiring is
        // a later wave).
        assert!(key.passphrase_hint.is_none());
    }

    #[test]
    fn reset_draft_returns_to_a_fresh_paste_tab() {
        let mut ui = KeysUi {
            name: "n".into(),
            key_type: KeyType::Rsa,
            private_key: "p".into(),
            passphrase: "x".into(),
            mode: KeyFormMode::Generate,
        };
        ui.reset_draft();
        assert_eq!(ui.key_type, KeyType::Ed25519);
        assert!(ui.name.is_empty());
        assert!(ui.private_key.is_empty());
        assert!(ui.passphrase.is_empty());
        assert_eq!(ui.mode, KeyFormMode::Paste);
        assert_eq!(KeysUi::default().key_type, KeyType::Ed25519);
        assert_eq!(KeysUi::default().mode, KeyFormMode::Paste);
    }

    #[test]
    fn form_modes_match_the_original_selector() {
        assert_eq!(KeyFormMode::ALL, [KeyFormMode::Paste, KeyFormMode::Import, KeyFormMode::Generate]);
        assert_eq!(KeyFormMode::Paste.label(), "Paste");
        assert_eq!(KeyFormMode::Import.label(), "Import");
        assert_eq!(KeyFormMode::Generate.label(), "Generate");
    }

    #[test]
    fn add_key_dialog_is_owned_by_this_screen() {
        // The dialog body only claims `AddKey`; `section()` keeps them in
        // sync (Keys is where this screen routes).
        assert_eq!(Dialog::AddKey.title(), "Add Key");
        assert_eq!(Dialog::AddKey.section(), Some(crate::navigation::Section::Keys));
    }

    #[test]
    fn screen_icons_are_bundled() {
        // The toolbar glyph is the thin plus, never `addCircle.svg`.
        for icon in [
            PLUS_ICON,
            CHEVRON_ICON,
            KEY_ICON,
            HARDWARE_KEY_ICON,
            SEP_KEY_ICON,
            UNSAVED_SEP_KEY_ICON,
            IDENTITY_ICON,
            MULTIKEY_ICON,
            CERTIFICATE_ICON,
            FIDO_ICON,
            TOUCH_ID_ICON,
        ] {
            assert!(crate::has_icon(icon), "missing icon `{icon}`");
        }
    }
}
