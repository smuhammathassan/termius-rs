//! keys_screen — the **Keys** section: stored SSH key pairs (SSHid) plus a
//! FIDO2 security-key stub.
//!
//! Port of Termius' Keychain → Keys screen. The entity row presenter is `Cw`
//! (`analysis/readable/_main.js:69838`) — a 14px `primary` title over a 12px
//! `deprecatedSecondary` description beside the key icon — and the toolbar's
//! `New Key` action (the `K6e` key chevron cluster, `_main.js:67834`). The
//! "Security Keys" card mirrors the FIDO2 block whose `Add FIDO2 Key` button is
//! `AddFIDO2KeyButton-1f6200c9.js` (an accent button: `addCircle` icon +
//! "Add FIDO2 Key").
//!
//! The screen matches the entity-list chrome: a **screen header** (the `Keys`
//! title + the primary `New Key` action with `addCircle.svg`), a
//! **search/filter row** (`search.svg` + tag-filter / sort glyphs), and the
//! **list** — one row per [`Key`] with a leading `key.svg` tile, the name
//! (`R14P`) over the algorithm / passphrase hint (`R12S`), and a trailing
//! `dots.svg` overflow affordance (wired to Delete).
//!
//! # Wiring (owned by `views`, not this module)
//!
//! ```ignore
//! // Section::Keys center panel — store the entity once (see `AppShell`):
//! views::keys_screen::keys_screen(state.clone(), cx)  // -> Entity<KeysScreen>
//! // Add-Key dialog body (host it via `DialogFrame::child`, which supplies
//! // the frame; this body carries its own action row):
//! views::keys_screen::key_dialog_body(&dialog, state, cx)
//! ```
//!
//! The factory only mints the [`KeysScreen`] view entity, so it is safe to
//! call while `TermiusState` is leased (inside `state.update(..)`); the view
//! reads the library in its own `render` and drives [`TermiusState`] through
//! `Entity::update`, exactly like `views::host_list`. [`key_dialog_body`]
//! still takes a `&TermiusState` + its `Context`, so the shell calls it from
//! inside `state.update(..)`.
//!
//! # PORT-TODOs
//!
//! * [`InputField`] is display-only (see the note in `primitives`), so the
//!   form's text values live in the [`KeysUi`] draft global and only the
//!   key-type chips can change them today; `Save` therefore writes a *sample*
//!   [`Key`] (`Key N`, the chosen algorithm, no key material, no timestamps).
//! * No edit flow yet: clicking a row only highlights it, and the ⋯ removes the
//!   key immediately (Termius confirms first).

use gpui::{
    div, px, AnyElement, App, AppContext as _, BorrowAppContext as _, ClickEvent, Context, Div,
    Entity, FontWeight, Global, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window,
};
use termius_core::{Key, KeyType};

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{Button, EmptyState, InputField, SettingsSection, SettingsText};
use crate::theme::{over, text, theme_of, with_alpha, TermiusTheme, ThemeMode, UI_FONT};

// --- layout metrics (from the original CSS) --------------------------------
/// Primary `New …` button height (Termius' `large` button: `height: 36px`).
const BUTTON_HEIGHT: f32 = 36.0;
/// Screen-title band height.
const TITLE_HEIGHT: f32 = 56.0;
/// Search/filter band (`HostsFiltersHeader` `height: 45px`).
const HEADER_HEIGHT: f32 = 45.0;
/// Entity row height (24px icon tile + two text lines), matching `host_list`.
const ROW_HEIGHT: f32 = 40.0;
/// Leading `entityIcon` tile (`width/height: 24px`).
const ICON_TILE: f32 = 24.0;
/// Glyph inside the tile.
const ICON_GLYPH: f32 = 16.0;
/// The SSH key entity icon.
const KEY_ICON: &str = "key.svg";
/// Corner radius for the ⋯ affordance.
const CORNER: f32 = 4.0;

// ---------------------------------------------------------------------------
// Ephemeral screen state (gpui global; screens are plain functions)
// ---------------------------------------------------------------------------

/// The Keys screen's ephemeral state: the highlighted row plus the Add-Key
/// form draft.
///
/// Kept as a gpui [`Global`] because this screen is a plain function with no
/// view entity of its own; listeners mutate it through
/// `update_default_global` + [`Context::notify`], which repaints the shell
/// (it observes the `TermiusState` entity the notify targets).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeysUi {
    /// Row highlighted in the key list (click-to-select stand-in for the
    /// editor Termius opens on click).
    selected_key: Option<String>,
    /// Draft "Name" field.
    name: String,
    /// Draft key type — the one field the chooser can actually change today.
    key_type: KeyType,
    /// Draft private-key material (never persisted by the stub).
    private_key: String,
    /// Draft passphrase (never persisted here; keychain wiring is deferred).
    passphrase: String,
}

impl Default for KeysUi {
    fn default() -> Self {
        Self {
            selected_key: None,
            name: String::new(),
            // Matches `Key::default()` (not `KeyType::default()`, which is
            // `Rsa`): a fresh key starts out Ed25519.
            key_type: KeyType::Ed25519,
            private_key: String::new(),
            passphrase: String::new(),
        }
    }
}

impl Global for KeysUi {}

impl KeysUi {
    /// Clear the form draft, keeping the row selection.
    fn reset_draft(&mut self) {
        self.name.clear();
        self.key_type = KeyType::Ed25519;
        self.private_key.clear();
        self.passphrase.clear();
    }
}

/// The current screen draft (a fresh default when the global was never set).
fn draft_of(cx: &App) -> KeysUi {
    match cx.try_global::<KeysUi>() {
        Some(ui) => ui.clone(),
        None => KeysUi::default(),
    }
}

// ---------------------------------------------------------------------------
// Shared list chrome (local; the primitives have no icon row)
// ---------------------------------------------------------------------------

/// The primary header action: `addCircle.svg` + label on the accent fill.
fn new_button(
    label: &'static str,
    theme: TermiusTheme,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover_fill = over(theme.primary, with_alpha(gpui::rgb(0xff_ff_ff), 0.25));
    div()
        .id(SharedString::from(format!("new-{label}")))
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(BUTTON_HEIGHT))
        .px(px(16.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.primary)
        .text_color(gpui::rgb(0xff_ff_ff))
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(21.))
        .whitespace_nowrap()
        .child(crate::icon("addCircle.svg").w(px(16.)).h(px(16.)))
        .child(SharedString::from(label))
        .hover(move |hover| hover.bg(hover_fill))
        .on_click(listener)
}

/// The display-only search field (`search.svg` + muted placeholder).
fn search_field(theme: TermiusTheme, placeholder: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .flex_1()
        .min_w(px(0.))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(theme.border_basic)
        .bg(theme.card_c)
        .text_color(theme.muted)
        .child(crate::icon("search.svg").w(px(12.)).h(px(12.)))
        .child(text::R12P.style(div()).child(SharedString::from(placeholder)))
}

/// A 24×24 glyph button (the header's filter / sort affordances).
fn icon_button(theme: TermiusTheme, icon_name: &'static str, id: &'static str) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .flex_shrink_0()
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.muted)
        .hover(move |hover| hover.bg(theme.hover))
        .child(crate::icon(icon_name).w(px(14.)).h(px(14.)))
}

/// The search + tag-filter + sort band (the original `HostsFiltersHeader`).
fn filter_row(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(HEADER_HEIGHT))
        .px(px(12.))
        .pt(px(4.))
        .pb(px(5.))
        .bg(theme.card_a)
        .child(search_field(theme, "Search keys"))
        .child(icon_button(theme, "tags.svg", "keys-filter-tags"))
        .child(icon_button(theme, "sorting.svg", "keys-sort"))
}

/// The leading 24×24 `entityIcon` tile for a row.
fn icon_tile(theme: TermiusTheme, icon_name: &'static str) -> Div {
    let tile_bg = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(ICON_TILE))
        .h(px(ICON_TILE))
        .flex_shrink_0()
        .rounded(px(theme.corner_radius_small))
        .bg(tile_bg)
        .text_color(theme.muted)
        .child(crate::icon(icon_name).w(px(ICON_GLYPH)).h(px(ICON_GLYPH)))
}

/// The title (`R14P`) over the subtitle (`R12S`) column of a row.
fn row_column(theme: TermiusTheme, title: String, subtitle: String) -> Div {
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
                .child(SharedString::from(title)),
        )
        .child(
            text::R12S
                .style(div())
                .text_color(theme.muted)
                .truncate()
                .child(SharedString::from(subtitle)),
        )
}

/// The trailing `dots.svg` overflow affordance (the row's context menu).
///
/// PORT-TODO(menu): gpui 0.2.2 has no anchored menu host, so the ⋯ stands in
/// for the context menu whose only wired item is Delete.
fn dots_button(
    id: String,
    theme: TermiusTheme,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .flex_shrink_0()
        .rounded(px(CORNER))
        .text_color(theme.muted)
        .hover(move |hover| hover.bg(theme.hover))
        .child(crate::icon("dots.svg").w(px(12.)).h(px(4.)))
        .on_click(listener)
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// The Termius label for one key algorithm.
fn key_type_label(key_type: KeyType) -> &'static str {
    match key_type {
        KeyType::Rsa => "RSA",
        KeyType::Dsa => "DSA",
        KeyType::Ecdsa => "ECDSA",
        KeyType::Ed25519 => "Ed25519",
        KeyType::X509 => "X509",
    }
}

/// The row label: the key's name, falling back like Termius' list does.
fn key_label(key: &Key) -> String {
    if key.name.is_empty() {
        "Untitled key".to_owned()
    } else {
        key.name.clone()
    }
}

/// The row subtitle: the algorithm, plus a passphrase hint when the key is
/// bound to a keychain entry (Termius shows a lock in that case).
fn key_subtitle(key: &Key) -> String {
    let label = key_type_label(key.key_type);
    if key.keychain_id.is_some() {
        format!("{label} · passphrase protected")
    } else {
        label.to_owned()
    }
}

/// The [`Key`] record `Save` writes.
///
/// PORT-TODO: `InputField` is display-only, so `name` / `private_key` /
/// `passphrase` only become reachable once the form grows a real content
/// model; until then an unnamed draft becomes `Key N` and the saved key has
/// no material or timestamps (`created_at`/`updated_at` stay empty — a
/// timestamp helper enters with the storage/sync wave).
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
// Chips (interactive pieces shared by the screen and the dialog)
// ---------------------------------------------------------------------------

/// One algorithm chip in the Add-Key form's type chooser.
fn type_chip(
    key_type: KeyType,
    selected: bool,
    theme: TermiusTheme,
    cx: &mut Context<TermiusState>,
) -> Stateful<Div> {
    let label = SharedString::from(key_type_label(key_type));
    let mut chip = div()
        .id(SharedString::from(format!("key-type-{label}")))
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px(px(12.))
        .rounded(px(CORNER))
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .text_sm()
        .text_color(if selected {
            // White on the accent fill, matching `Button::primary`.
            gpui::rgb(0xff_ff_ff)
        } else {
            theme.foreground
        });
    if selected {
        chip = chip.bg(theme.accent);
    }
    chip.child(label).on_click(cx.listener(
        // `move`: listeners are `'static`, so the Copy `key_type` rides along
        // by value (a non-move closure would borrow the local).
        move |_this, _event, _window, cx| {
            cx.update_default_global::<KeysUi, _>(|ui, cx| {
                ui.key_type = key_type;
                cx.notify();
            });
        },
    ))
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Keys screen view: header + search/filter row + key list + the Security
/// Keys stub.
///
/// A separate entity from [`TermiusState`] (gpui leases one entity at a time):
/// the factory below only mints it, [`Render::render`] reads the library
/// through the handle, and every click drives `TermiusState` through
/// `Entity::update` — the `views::host_list` pattern.
pub struct KeysScreen {
    state: Entity<TermiusState>,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Keys screen for `state`.
///
/// Safe to call while `TermiusState` is leased (inside `state.update(..)`):
/// it only mints a view entity, so no `Entity::read` happens there. Store the
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
        Self { state, _observe_state: observe_state, _observe_theme: observe_theme }
    }

    /// Raise the Add-Key dialog (the shell hosts it above everything).
    fn open_add_key(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.open_dialog(Dialog::AddKey, cx));
    }

    /// Delete one key (the row's ⋯).
    fn delete_key(&mut self, key_id: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.delete_key(key_id, cx));
    }

    /// Select one key row (the click-to-select stand-in for the editor).
    fn select_key(&mut self, key_id: &str, cx: &mut Context<Self>) {
        cx.update_default_global::<KeysUi, _>(|ui, cx| {
            ui.selected_key = Some(key_id.to_owned());
            cx.notify();
        });
    }

    /// Surface the FIDO2 stub note in the status bar.
    fn show_fido_note(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.status_text = "FIDO2 keys are deferred to the termius-fido crate.".to_owned();
            cx.notify();
        });
    }
}

impl Render for KeysScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let keys = self.state.read(cx).library.keys.clone();
        let selected = cx.try_global::<KeysUi>().and_then(|ui| ui.selected_key.clone());

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(TITLE_HEIGHT))
            .px(px(12.))
            .child(
                text::B16P
                    .style(div())
                    .text_color(theme.title)
                    .child(SharedString::from("Keys")),
            )
            .child(new_button(
                "New Key",
                theme,
                cx.listener(|this, _event, _window, cx| this.open_add_key(cx)),
            ));

        let mut list = div()
            .id("keys-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .gap(px(2.))
            .px(px(8.))
            .py(px(4.));

        if keys.is_empty() {
            list = list.child(
                EmptyState::new("No keys", "Add an SSH key to authenticate to your hosts.")
                    .element(theme),
            );
        } else {
            for key in &keys {
                let id = key.id.clone();
                let open_id = id.clone();
                let delete_id = id.clone();
                let is_selected = selected.as_deref() == Some(key.id.as_str());

                // The whole row carries the selected/hover wash; the content
                // and the ⋯ are siblings so the ⋯ click never also selects.
                let wrapper = div()
                    .id(SharedString::from(format!("key-row-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(ROW_HEIGHT))
                    .px(px(12.))
                    .rounded(px(theme.corner_radius_small));
                let wrapper = if is_selected {
                    wrapper
                        .bg(theme.card_c)
                        .hover(move |hover| hover.bg(over(theme.card_c, theme.hover)))
                } else {
                    wrapper.hover(move |hover| hover.bg(theme.hover))
                };

                let content = div()
                    .id(SharedString::from(format!("key-open-{open_id}")))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .flex_1()
                    .min_w(px(0.))
                    .child(icon_tile(theme, KEY_ICON))
                    .child(row_column(theme, key_label(key), key_subtitle(key)))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.select_key(&open_id, cx);
                    }));

                let dots = dots_button(
                    format!("key-menu-{delete_id}"),
                    theme,
                    cx.listener(move |this, _event, _window, cx| {
                        this.delete_key(&delete_id, cx);
                    }),
                );

                list = list.child(wrapper.child(content).child(dots));
            }
        }

        // PORT-TODO(termius-fido): the real flow registers/uses a FIDO2
        // (WebAuthn) credential; the button only surfaces that today.
        let security_keys = div().p(px(10.)).child(
            SettingsSection::new("Security Keys")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            crate::icon("hardware_key.svg")
                                .w(px(16.))
                                .h(px(16.))
                                .text_color(theme.muted),
                        )
                        .child(
                            SettingsText::new(
                                "Authenticate to a host with a FIDO2 / hardware security key.",
                            )
                            .element(theme),
                        ),
                )
                .child(
                    div().pt(px(8.)).child(
                        Button::new("Add FIDO2 Key").primary().on_click(
                            theme,
                            cx.listener(|this, _event, _window, cx| this.show_fido_note(cx)),
                        ),
                    ),
                )
                .element(theme),
        );

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .text_color(theme.foreground)
            .child(header)
            .child(filter_row(theme))
            .child(list)
            .child(security_keys)
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
/// row). Field order mirrors the original `FD` key form
/// (`_main.js:68481`): Label → Passphrase → Private key, with the port's
/// algorithm chooser inserted after the label.
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

    let mut types = div().flex().gap(px(6.));
    for key_type in [KeyType::Ed25519, KeyType::Rsa, KeyType::Ecdsa] {
        let is_selected = draft.key_type == key_type;
        types = types.child(type_chip(key_type, is_selected, theme, cx));
    }

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
    // Reads the draft from the global inside the listener, so it always
    // saves the latest chips/fields rather than the render-time copy.
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
        .child(
            InputField::new("Label", draft.name.clone())
                .placeholder(default_name)
                .element(theme),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .font_family(UI_FONT)
                        .text_size(px(14.))
                        .text_color(theme.text_common)
                        .child(SharedString::from("Type")),
                )
                .child(types),
        )
        .child(
            InputField::new("Passphrase", draft.passphrase.clone())
                .placeholder("Leave empty if unprotected")
                .element(theme),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .font_family(UI_FONT)
                        .text_size(px(14.))
                        .text_color(theme.text_common)
                        .child(SharedString::from("Private key")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .h(px(92.))
                        .p(px(8.))
                        .rounded(px(theme.corner_radius_small))
                        .border_1()
                        .border_color(theme.border_basic)
                        .bg(theme.card_c)
                        .child(SettingsText::new(private_hint).element(theme)),
                ),
        )
        .child(
            SettingsText::new("The passphrase is never stored in the key record.").element(theme),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_type_labels_match_termius() {
        assert_eq!(key_type_label(KeyType::Rsa), "RSA");
        assert_eq!(key_type_label(KeyType::Dsa), "DSA");
        assert_eq!(key_type_label(KeyType::Ecdsa), "ECDSA");
        assert_eq!(key_type_label(KeyType::Ed25519), "Ed25519");
        assert_eq!(key_type_label(KeyType::X509), "X509");
    }

    #[test]
    fn row_text_describes_the_key() {
        let mut key = Key { name: "deploy".into(), ..Key::default() };
        assert_eq!(key_label(&key), "deploy");
        assert_eq!(key_subtitle(&key), "Ed25519");

        // A keychain-bound key advertises its passphrase protection.
        key.keychain_id = Some("kc-1".into());
        assert_eq!(key_subtitle(&key), "Ed25519 · passphrase protected");

        key.key_type = KeyType::Rsa;
        assert_eq!(key_subtitle(&key), "RSA · passphrase protected");

        let unnamed = Key::default();
        assert_eq!(key_label(&unnamed), "Untitled key");
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
            selected_key: Some("k".into()),
            name: "prod".into(),
            key_type: KeyType::Rsa,
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
            passphrase: "hunter2".into(),
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
    fn reset_draft_keeps_the_selection() {
        let mut ui = KeysUi {
            selected_key: Some("k".into()),
            name: "n".into(),
            key_type: KeyType::Rsa,
            private_key: "p".into(),
            passphrase: "x".into(),
        };
        ui.reset_draft();
        assert_eq!(ui.selected_key.as_deref(), Some("k"));
        assert_eq!(ui.key_type, KeyType::Ed25519);
        assert!(ui.name.is_empty());
        assert!(ui.private_key.is_empty());
        assert!(ui.passphrase.is_empty());
        assert_eq!(KeysUi::default().key_type, KeyType::Ed25519);
    }

    #[test]
    fn add_key_dialog_is_owned_by_this_screen() {
        // The dialog body only claims `AddKey`; `section()` keeps them in
        // sync (Keys is where this screen routes).
        assert_eq!(Dialog::AddKey.title(), "Add Key");
        assert_eq!(Dialog::AddKey.section(), Some(crate::navigation::Section::Keys));
    }

    #[test]
    fn key_row_icon_is_bundled() {
        assert!(crate::has_icon(KEY_ICON));
        assert!(crate::has_icon("hardware_key.svg"));
        assert!(crate::has_icon("addCircle.svg"));
        assert!(crate::has_icon("dots.svg"));
    }
}
