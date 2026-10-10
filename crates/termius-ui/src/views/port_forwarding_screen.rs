//! port_forwarding_screen — the Port Forwarding section: the live-tunnel rule
//! list plus the type-dependent add/edit form (with its 5-step wizard).
//!
//! Reconstructed from `analysis/recon/22-port-forwarding.md` (the
//! `ConnectedPortForwardings` screen in `analysis/readable/_main.js` and the
//! rule presenter `assets/PortForwardingRulePresenter-4942c31a.js`).
//!
//! # What changed from the earlier port
//!
//! * **No title band.** The primary action is the **first child of the shared
//!   [`FiltersHeader`]** (search band), exactly like the original — there is no
//!   separate `"Port Forwarding"` `SectionHeader` strip.
//! * **Split toolbar button.** `" New forwarding"` carries the thin
//!   `plusThin.svg` glyph (the original inline `SvgPlusThinreact`), not
//!   `addCircle.svg`, and is a split button whose chevron opens the
//!   Local / Remote / Dynamic menu (`_main.js:75510-75528`).
//! * **Rule glyph is a ShapedIcon letter tile** — `L` / `R` / `D`, `40×40`,
//!   radius 12, `--dark-grey-6` (accent `--button-accent` while connected) — per
//!   the presenter's `pficons` registry (`/tmp/reconnectSaga.js:88857`). The
//!   standalone `*-port-forwarding.svg` files are **not** the runtime path.
//! * **Live tunnels.** Each row carries a status (connected / disconnected) and
//!   a Connect / Disconnect action; the row title is the presenter's label (the
//!   rule string, since the model has no `label` field) and the subtitle names
//!   the bound host, bind interface and status.
//! * **Type-dependent form.** Local = Local port number · Bind address ·
//!   Intermediate host · Destination address · Destination port number; Remote =
//!   Remote host · Remote port number · Bind address · Destination address ·
//!   Destination port number; Dynamic = Local port number · Bind address ·
//!   Intermediate host (`_main.js:73702-73825`). The **5-step wizard**
//!   (`_main.js:74259`) drives the same fields step by step.
//!
//! Rows reuse the shared entity idiom (`padding:10px`, radius 14,
//! `2px solid var(--entity-item-background)`, hover `--list-hover-hover`,
//! title 14 / subtitle 11). [`EntityRow`] itself only paints a bundled SVG
//! glyph, so the letter tile + Connect/Disconnect action are composed locally
//! around the same geometry.
//!
//! # Why the list is a view entity
//!
//! [`port_forwarding_screen`] takes a `&mut Context<TermiusState>`, and in
//! gpui 0.2.2 a `Context<T>` only exists while `T` is *leased out of the
//! entity map*: [`AppContext::update_entity`] removes the entity for the
//! duration of the update, and [`Entity::read`] then panics with "cannot
//! read TermiusState while it is already being updated". The only way to
//! obtain that context is therefore `state.update(..)`, i.e. exactly the
//! window in which `state.read(cx)` would panic.
//!
//! The factory works around that by handing back a small view entity
//! ([`PortForwardList`]) instead of a ready-built element tree: its [`Render`]
//! runs during layout, *after* the enclosing `state.update(..)` returned and
//! `TermiusState` is back in the map, so reading `self.state` there is safe.
//! That is the same shape [`crate::views::host_list`] already uses.
//!
//! # PORT-TODOs (model gaps)
//!
//! * `PortForwardingConfig` has no `label`, `intermediate_host` or runtime
//!   `status`; the label falls back to the rule string, "Intermediate host" is
//!   collected into the draft but dropped on save, and connection status lives
//!   in [`PortForwardList`] only.
//! * `InputField` is display-only, so ports/addresses are sample values; the
//!   type chooser and wizard steps are the only editable inputs.

use std::collections::HashMap;

use gpui::{
    div, px, AnyElement, AppContext as _, Context, Div, Entity, EntityId, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
};
use termius_core::{ForwardType, Host, PortForwardingConfig};

use crate::app_state::{Dialog, TermiusState};
use crate::assets::{icon, UI_FONT};
use crate::primitives::{Button, ButtonSize, EmptyState, FiltersHeader, InputField, SettingsText};
use crate::theme::{text, theme_of, with_alpha, TermiusTheme, ThemeMode};

/// Entity icon tile (`ShapedIcon.shape`: `40×40`, radius `12`).
const TILE: f32 = 40.0;
const TILE_RADIUS: f32 = 12.0;
const TILE_GLYPH: f32 = 24.0;
/// `GridItemPresenter.entityItem` — padding / radius of the row card.
const ROW_PAD: f32 = 10.0;
const ENTITY_RADIUS: f32 = 14.0;
/// White used for text/glyphs on the accent fill.
const ON_ACCENT: gpui::Rgba = gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
/// `--dark-grey-6` (`#5a5e73`): the unconnected letter-tile fill.
const DARK_GREY_6: gpui::Rgba =
    gpui::Rgba { r: 90.0 / 255.0, g: 94.0 / 255.0, b: 115.0 / 255.0, a: 1.0 };

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// Termius' label for one tunnel direction (`uSe` / `pSe`).
fn forward_type_label(forward_type: ForwardType) -> &'static str {
    match forward_type {
        ForwardType::Local => "Local",
        ForwardType::Remote => "Remote",
        ForwardType::Dynamic => "Dynamic",
        ForwardType::LocalAuto => "Local",
        ForwardType::RemoteAuto => "Remote",
        ForwardType::LocalSerial => "Local",
        ForwardType::LocalSerialAuto => "Local",
    }
}

/// The rule glyph, from the presenter's `pficons` registry: `L` / `R` / `D`
/// (`/tmp/reconnectSaga.js:88857`) — a ShapedIcon **letter**, not an SVG.
fn rule_letter(forward_type: ForwardType) -> &'static str {
    match forward_type {
        ForwardType::Remote | ForwardType::RemoteAuto => "R",
        ForwardType::Dynamic => "D",
        _ => "L",
    }
}

/// The presenter's `description(item)` / rule string, e.g.
/// `Local :8080 → db.internal:5432`.
///
/// A SOCKS ([`ForwardType::Dynamic`]) rule has no destination, so it falls back
/// to `Dynamic :1080`.
fn rule_string(rule: &PortForwardingConfig) -> String {
    let kind = forward_type_label(rule.forward_type);
    let listen = format!(":{}", rule.listen_port);
    match (&rule.destination_host, rule.destination_port) {
        (Some(host), Some(port)) => format!("{kind} {listen} → {host}:{port}"),
        (Some(host), None) => format!("{kind} {listen} → {host}"),
        (None, Some(port)) => format!("{kind} {listen} → :{port}"),
        (None, None) => format!("{kind} {listen}"),
    }
}

/// What the listener binds to: `bind_all` wins, an empty interface means
/// loopback (the default in [`PortForwardingConfig`]).
fn listen_label(rule: &PortForwardingConfig) -> String {
    if rule.bind_all {
        "0.0.0.0".to_owned()
    } else if rule.listen_interface.trim().is_empty() {
        "127.0.0.1".to_owned()
    } else {
        rule.listen_interface.clone()
    }
}

/// The row subtitle: bound host + bind interface + live status.
fn rule_subtitle(rule: &PortForwardingConfig, hosts: &[Host], status: PfConnState) -> String {
    let host = rule
        .host_id
        .as_deref()
        .and_then(|id| hosts.iter().find(|host| host.id == id))
        .map(|host| host.label.clone())
        .unwrap_or_else(|| "No host".to_owned());
    format!("{host} · {} · {}", listen_label(rule), status.label())
}

/// Runtime status of one tunnel (the original `ruleStatus`; the port models
/// only the connected/disconnected endpoints — PORT-TODO: `connecting` /
/// `in-queue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PfConnState {
    Disconnected,
    Connected,
}

impl PfConnState {
    fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }

    fn is_connected(self) -> bool {
        matches!(self, Self::Connected)
    }

    /// The original's double-click toggle: connected ⇄ disconnected.
    fn toggled(self) -> Self {
        match self {
            Self::Connected => Self::Disconnected,
            Self::Disconnected => Self::Connected,
        }
    }
}

/// One step of the add/edit wizard (`PSe`, `_main.js:74259`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PfStep {
    Type,
    LocalPort,
    ServiceHost,
    RemotePort,
    Destination,
    Label,
}

impl PfStep {
    fn label(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::LocalPort => "Local port number",
            Self::ServiceHost => "Service host",
            Self::RemotePort => "Remote port number",
            Self::Destination => "Destination",
            Self::Label => "Label",
        }
    }
}

/// The wizard step sequence for a rule type (`PSe`, `_main.js:74259`):
/// * `local`   — pf_type → local_port → service_host → destination → label
/// * `remote`  — pf_type → service_host → remote_port → destination → label
/// * `dynamic` — pf_type → local_port → service_host → label
fn wizard_steps(forward_type: ForwardType) -> &'static [PfStep] {
    const LOCAL: &[PfStep] = &[
        PfStep::Type,
        PfStep::LocalPort,
        PfStep::ServiceHost,
        PfStep::Destination,
        PfStep::Label,
    ];
    const REMOTE: &[PfStep] = &[
        PfStep::Type,
        PfStep::ServiceHost,
        PfStep::RemotePort,
        PfStep::Destination,
        PfStep::Label,
    ];
    const DYNAMIC: &[PfStep] = &[PfStep::Type, PfStep::LocalPort, PfStep::ServiceHost, PfStep::Label];
    match forward_type {
        ForwardType::Remote | ForwardType::RemoteAuto => REMOTE,
        ForwardType::Dynamic => DYNAMIC,
        _ => LOCAL,
    }
}

/// A numeric field's shown value (`0` renders as an empty box).
fn port_value(port: u16) -> String {
    if port == 0 {
        String::new()
    } else {
        port.to_string()
    }
}

/// An optional port (`0` = unset).
fn port_opt(port: u16) -> Option<u16> {
    if port == 0 {
        None
    } else {
        Some(port)
    }
}

/// A trimmed string as `Some`, or `None` when empty.
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

// ---------------------------------------------------------------------------
// The add-form draft
// ---------------------------------------------------------------------------

/// The add/edit form state that outlives one paint.
///
/// gpui gives a free function no local view state, so the draft rides in a
/// [`Global`] until Save consumes it; the dialog body reads it and the controls
/// write it back (repainting through [`Context::notify`]).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PortForwardDraft {
    forward_type: ForwardType,
    host_id: Option<String>,
    /// Bind the listener to `0.0.0.0` (derived from the Bind address field).
    bind_all: bool,
    /// Bind address (maps to `listen_interface`).
    bind_address: String,
    local_port: u16,
    remote_port: u16,
    destination_host: String,
    destination_port: u16,
    /// PORT-TODO: the model has no `intermediate_host`; collected then dropped.
    intermediate_host: String,
    label: String,
    /// The wizard is off until the form's "Open wizard" affordance is used.
    wizard: bool,
    step: usize,
}

impl Default for PortForwardDraft {
    fn default() -> Self {
        Self {
            forward_type: ForwardType::Local,
            host_id: None,
            bind_all: false,
            bind_address: String::new(),
            local_port: 8080,
            remote_port: 22,
            destination_host: "localhost".to_owned(),
            destination_port: 80,
            intermediate_host: String::new(),
            label: String::new(),
            wizard: false,
            step: 0,
        }
    }
}

impl Global for PortForwardDraft {}

impl PortForwardDraft {
    /// A fresh draft of `forward_type` bound to `host_id` (the first host, on
    /// open). Mirrors `UD.addRule(type, formType)`.
    fn with_host(forward_type: ForwardType, host_id: Option<String>) -> Self {
        Self {
            forward_type,
            host_id,
            ..Self::default()
        }
    }

    /// The listener port: Dynamic/Local listen on `local_port`, Remote on
    /// `remote_port`.
    fn listen_port(&self) -> u16 {
        match self.forward_type {
            ForwardType::Remote | ForwardType::RemoteAuto => self.remote_port,
            _ => self.local_port,
        }
    }

    /// The effective bind address (`127.0.0.1` unless overridden / bind-all).
    fn bind_address(&self) -> String {
        if !self.bind_address.trim().is_empty() {
            self.bind_address.clone()
        } else if self.bind_all {
            "0.0.0.0".to_owned()
        } else {
            "127.0.0.1".to_owned()
        }
    }

    /// The rule Save persists (`add_port_forwarding` mints the id).
    fn to_config(&self) -> PortForwardingConfig {
        let dynamic = matches!(self.forward_type, ForwardType::Dynamic);
        let bind = self.bind_address();
        PortForwardingConfig {
            id: String::new(),
            forward_type: self.forward_type,
            listen_interface: bind.clone(),
            listen_port: self.listen_port(),
            destination_host: if dynamic { None } else { non_empty(&self.destination_host) },
            destination_port: if dynamic { None } else { port_opt(self.destination_port) },
            host_id: self.host_id.clone(),
            bind_all: bind == "0.0.0.0",
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

/// The current draft (a default when the dialog was raised elsewhere).
fn current_draft(cx: &Context<TermiusState>) -> PortForwardDraft {
    cx.try_global::<PortForwardDraft>().cloned().unwrap_or_default()
}

/// Store a draft and repaint the dialog.
fn write_draft(cx: &mut Context<TermiusState>, draft: PortForwardDraft) {
    cx.set_global(draft);
    cx.notify();
}

// ---------------------------------------------------------------------------
// Row / tile / button chrome
// ---------------------------------------------------------------------------

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

/// The rule glyph tile: a `ShapedIcon`-shaped square (`40×40`, radius 12) whose
/// glyph is the letter `L` / `R` / `D`, white 24px, on `--dark-grey-6` (or the
/// accent `--button-accent` while the tunnel is connected).
fn letter_tile(theme: TermiusTheme, letter: &'static str, connected: bool) -> Div {
    let background = if connected { theme.primary } else { DARK_GREY_6 };
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(TILE))
        .rounded(px(TILE_RADIUS))
        .bg(background)
        .text_color(ON_ACCENT)
        .font_family(UI_FONT)
        .text_size(px(TILE_GLYPH))
        .font_weight(FontWeight::BOLD)
        .child(SharedString::from(letter))
}

/// A smaller letter tile for the type menu (20×20, radius 6).
fn letter_tile_small(letter: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(20.))
        .rounded(px(6.))
        .bg(DARK_GREY_6)
        .text_color(ON_ACCENT)
        .font_family(UI_FONT)
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .child(SharedString::from(letter))
}

/// The split toolbar action: `" New forwarding"` (thin plus) + a chevron that
/// opens the Local / Remote / Dynamic menu (`useB0`, `_main.js:75530-75558`).
fn split_new_button(theme: TermiusTheme, cx: &mut Context<PortForwardList>) -> Div {
    let segment_hover = with_alpha(ON_ACCENT, 0.12);
    let main = div()
        .id("pf-new-forwarding")
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(36.))
        .pl(px(14.))
        .pr(px(12.))
        .text_color(ON_ACCENT)
        .font_family(UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(icon("plusThin.svg").w(px(14.)).h(px(14.)))
        .child(SharedString::from("New forwarding"))
        .hover(move |style| style.bg(segment_hover))
        .on_click(
            cx.listener(|this, _event, _window, cx| {
                this.open_new_rule(ForwardType::Local, cx);
            }),
        );
    let chevron = div()
        .id("pf-new-forwarding-menu")
        .flex()
        .items_center()
        .justify_center()
        .h(px(36.))
        .w(px(30.))
        .text_color(ON_ACCENT)
        .hover(move |style| style.bg(segment_hover))
        .child(icon("chevron.svg").w(px(12.)).h(px(12.)))
        .on_click(cx.listener(|this, _event, _window, cx| this.toggle_menu(cx)));
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.primary)
        .overflow_hidden()
        .child(main)
        .child(div().w(px(1.)).h(px(18.)).bg(with_alpha(ON_ACCENT, 0.25)))
        .child(chevron)
}

/// The chevron menu: Local / Remote / Dynamic forwarding (each opens the add
/// flow with that type preset, `_main.js:75510-75528`).
fn type_menu(theme: TermiusTheme, cx: &mut Context<PortForwardList>) -> Div {
    let items = [
        (ForwardType::Local, "Local Forwarding"),
        (ForwardType::Remote, "Remote Forwarding"),
        (ForwardType::Dynamic, "Dynamic Forwarding"),
    ];
    let mut menu = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(4.))
        .mx(px(12.))
        .mt(px(4.))
        .rounded(px(theme.corner_radius_medium))
        .border_1()
        .border_color(theme.border_light)
        .bg(theme.card_a);
    for (forward_type, label) in items {
        let item = div()
            .id(SharedString::from(format!("pf-typemenu-{label}")))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(theme.corner_radius_small))
            .text_color(theme.title)
            .hover(move |style| style.bg(theme.hover))
            .child(letter_tile_small(rule_letter(forward_type)))
            .child(text::R14P.style(div()).child(SharedString::from(label)))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.menu_open = false;
                this.open_new_rule(forward_type, cx);
            }));
        menu = menu.child(item);
    }
    menu
}

/// A small ghost Connect / Disconnect button (unique id per rule).
fn connect_button(
    theme: TermiusTheme,
    id: &str,
    connected: bool,
    cx: &mut Context<PortForwardList>,
) -> Stateful<Div> {
    let label = if connected { "Disconnect" } else { "Connect" };
    let rule_id = id.to_owned();
    div()
        .id(SharedString::from(format!("pf-connect-{id}")))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .h(px(24.))
        .px(px(10.))
        .rounded(px(6.))
        .text_color(if connected { theme.primary } else { theme.text_common })
        .font_family(UI_FONT)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(SharedString::from(label))
        .hover(move |style| style.bg(theme.hover))
        .on_click(cx.listener(move |this, _event, _window, cx| {
            this.toggle_status(&rule_id, cx);
        }))
}

/// The trailing `dots.svg` overflow affordance (the row's context menu anchor).
///
/// PORT-TODO(menu): gpui 0.2.2 has no anchored popup menu here, so the button
/// stands in for the context menu's `Remove` item and deletes the rule.
fn row_dots(id: String, theme: TermiusTheme, cx: &mut Context<PortForwardList>) -> Stateful<Div> {
    let rule_id = id.clone();
    div()
        .id(SharedString::from(format!("pf-menu-{id}")))
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .flex_shrink_0()
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.text_common)
        .hover(move |style| style.bg(theme.hover))
        .child(icon("dots.svg").w(px(12.)).h(px(4.)))
        .on_click(cx.listener(move |this, _event, _window, cx| {
            this.delete_rule(&rule_id, cx);
        }))
}

/// The title (`R14P`) over the subtitle (`R12S`, 11px) column of a row.
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
            div()
                .truncate()
                .font_family(UI_FONT)
                .text_size(px(11.))
                .font_weight(FontWeight(450.0))
                .line_height(px(14.))
                .text_color(theme.text_common)
                .child(SharedString::from(subtitle)),
        )
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// One painted rule row (the strings + glyph [`PortForwardList`] hands to the
/// row chrome; built from a single `TermiusState` read).
struct RuleRow {
    id: String,
    label: String,
    subtitle: String,
    letter: &'static str,
    connected: bool,
}

/// The memoized list view for one `TermiusState`, so repeated renders reuse
/// the same entity instead of minting one per frame.
struct PortForwardScreenMemo {
    state: EntityId,
    view: WeakEntity<PortForwardList>,
}

impl Global for PortForwardScreenMemo {}

/// The Port Forwarding rule list (see the module docs for why this is a view
/// entity rather than a plain element tree).
pub struct PortForwardList {
    state: Entity<TermiusState>,
    /// The chevron type menu is open.
    menu_open: bool,
    /// Runtime tunnel status per rule id (the model carries none).
    status: HashMap<String, PfConnState>,
}

impl PortForwardList {
    fn new(state: Entity<TermiusState>, _cx: &mut Context<Self>) -> Self {
        Self {
            state,
            menu_open: false,
            status: HashMap::new(),
        }
    }

    /// Rows to paint, cloned out of `TermiusState` in one read.
    ///
    /// Safe here: `self.state` is a *different* entity than the leased
    /// [`PortForwardList`], exactly as `views::host_list` reads its state.
    fn rule_rows(&self, cx: &Context<Self>) -> Vec<RuleRow> {
        let state = self.state.read(cx);
        state
            .port_forwardings
            .iter()
            .map(|rule| {
                let conn = self
                    .status
                    .get(&rule.id)
                    .copied()
                    .unwrap_or(PfConnState::Disconnected);
                RuleRow {
                    id: rule.id.clone(),
                    label: rule_string(rule),
                    subtitle: rule_subtitle(rule, &state.library.hosts, conn),
                    letter: rule_letter(rule.forward_type),
                    connected: conn.is_connected(),
                }
            })
            .collect()
    }

    /// Seed the draft with the chosen type + first host and raise the add dialog.
    fn open_new_rule(&mut self, forward_type: ForwardType, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            let host_id = state.library.hosts.first().map(|host| host.id.clone());
            cx.set_global(PortForwardDraft::with_host(forward_type, host_id));
            state.open_dialog(Dialog::AddPortForward, cx);
        });
    }

    /// Toggle the chevron type menu.
    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        self.menu_open = !self.menu_open;
        cx.notify();
    }

    /// Flip a tunnel between connected and disconnected.
    fn toggle_status(&mut self, rule_id: &str, cx: &mut Context<Self>) {
        let current = self
            .status
            .get(rule_id)
            .copied()
            .unwrap_or(PfConnState::Disconnected);
        self.status.insert(rule_id.to_owned(), current.toggled());
        cx.notify();
    }

    /// Drop one rule (the row's `dots.svg` menu anchor).
    fn delete_rule(&mut self, rule_id: &str, cx: &mut Context<Self>) {
        self.status.remove(rule_id);
        self.state
            .update(cx, |state, cx| state.delete_port_forwarding(rule_id, cx));
    }

    /// One painted rule row: letter tile · title/meta · Connect/Disconnect · ⋯.
    fn rule_row(&self, row: &RuleRow, theme: TermiusTheme, cx: &mut Context<Self>) -> Stateful<Div> {
        let base = entity_bg(theme);
        let hover = entity_hover(theme);
        div()
            .id(SharedString::from(format!("pf-row-{}", row.id)))
            .flex()
            .items_center()
            .gap(px(10.))
            .mx(px(5.))
            .p(px(ROW_PAD))
            .rounded(px(ENTITY_RADIUS))
            .border_2()
            .border_color(base)
            .bg(base)
            .text_color(theme.title)
            .hover(move |style| style.bg(hover).border_color(hover))
            .child(letter_tile(theme, row.letter, row.connected))
            .child(row_column(theme, row.label.clone(), row.subtitle.clone()))
            .child(connect_button(theme, &row.id, row.connected, cx))
            .child(row_dots(row.id.clone(), theme, cx))
    }
}

impl Render for PortForwardList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let rows = self.rule_rows(cx);

        // Header: the shared search/filter band with the primary action as its
        // first child — no separate title band (the original `z0`).
        let header = FiltersHeader::new("Search port forwarding")
            .action(split_new_button(theme, cx))
            .element(theme);

        let mut body = div().flex().flex_col().flex_1().min_h(px(0.)).overflow_hidden();
        if self.menu_open {
            body = body.child(type_menu(theme, cx));
        }

        if rows.is_empty() {
            body = body.child(
                EmptyState::new(
                    "Set up port forwarding",
                    "Save port forwarding to access databases, web apps, and other services.",
                )
                .element(theme),
            );
        } else {
            // PORT-TODO: gpui 0.2.2's `Styled` has no scrollable overflow helper
            // wired here, so long rule lists clip (see `keys_screen` for the
            // `overflow_y_scroll` variant).
            let mut list = div().flex().flex_col().gap(px(2.)).p(px(8.));
            for row in &rows {
                list = list.child(self.rule_row(row, theme, cx));
            }
            body = body.child(list);
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.foreground)
            .child(header)
            .child(body)
    }
}

/// The Port Forwarding screen: a column of live tunnels behind the shared
/// filter band's `" New forwarding"` split action.
///
/// Call it while holding a `Context<TermiusState>` (i.e. from inside
/// `state.update(..)`): it returns a [`PortForwardList`] view, whose own
/// render reads `state` safely — see the module docs.
pub fn port_forwarding_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<PortForwardList> {
    let cached = cx
        .try_global::<PortForwardScreenMemo>()
        .filter(|memo| memo.state == state.entity_id())
        .and_then(|memo| memo.view.upgrade());
    match cached {
        Some(view) => view,
        None => {
            let view = cx.new(|cx| PortForwardList::new(state.clone(), cx));
            cx.set_global(PortForwardScreenMemo {
                state: state.entity_id(),
                view: view.downgrade(),
            });
            view
        }
    }
}

// ---------------------------------------------------------------------------
// The dialog body
// ---------------------------------------------------------------------------

/// The body for `dialog`, or `None` when another screen owns it.
///
/// Takes `&TermiusState` because the caller is inside `state.update(..)` —
/// the leased value is handed over directly, so no `Entity::read` happens.
///
/// The returned element is a bare body (with its own action row) that the
/// shell hosts inside a [`DialogFrame`](crate::primitives::DialogFrame) card.
pub fn port_forward_dialog_body(
    dialog: &Dialog,
    state: &TermiusState,
    cx: &mut Context<TermiusState>,
) -> Option<AnyElement> {
    match dialog {
        Dialog::AddPortForward => Some(add_port_forward_form(state, cx)),
        _ => None,
    }
}

/// A muted group label above a form control.
fn field_label(theme: TermiusTheme, text: &'static str) -> Div {
    div()
        .font_family(UI_FONT)
        .text_size(px(14.))
        .text_color(theme.text_common)
        .child(SharedString::from(text))
}

/// The "Add Port Forward" form. When the wizard is off this is the full
/// type-dependent form; when on it is the 5-step wizard (`_main.js:74259`).
fn add_port_forward_form(state: &TermiusState, cx: &mut Context<TermiusState>) -> AnyElement {
    let theme = theme_of(cx);
    let draft = current_draft(cx);

    let mut form = div().flex().flex_col().gap(px(12.));
    if draft.wizard {
        form = form.child(wizard_header(theme, &draft));
        form = form.child(wizard_step_body(theme, &draft, cx));
        form = form.child(wizard_actions(theme, &draft, cx));
    } else {
        // Label first (the original `fN_1 { label:"Label", autoFocus }`).
        form = form.child(
            InputField::new("Label", draft.label.clone())
                .placeholder("Label")
                .element(theme),
        );
        form = form.child(field_label(theme, "Type"));
        form = form.child(type_chips(theme, draft.forward_type, cx));
        form = form.child(type_fields(theme, &draft));
        form = form.child(host_picker(state, theme, &draft, cx));
        form = form.child(
            div().child(
                Button::new("Open wizard")
                    .secondary()
                    .size(ButtonSize::Small)
                    .on_click(
                        theme,
                        cx.listener(|_this, _event, _window, cx| {
                            let mut draft = current_draft(cx);
                            draft.wizard = true;
                            draft.step = 0;
                            write_draft(cx, draft);
                        }),
                    ),
            ),
        );
        form = form.child(form_actions(theme, cx));
    }
    form.into_any_element()
}

/// The Local / Remote / Dynamic type chooser (`uSe`, `_main.js:73575`).
fn type_chips(theme: TermiusTheme, selected: ForwardType, cx: &mut Context<TermiusState>) -> Div {
    let mut chips = div().flex().gap(px(8.));
    for kind in [ForwardType::Local, ForwardType::Remote, ForwardType::Dynamic] {
        let label = forward_type_label(kind);
        let is_selected = selected == kind;
        let chip = div()
            .id(SharedString::from(format!("pf-type-{label}")))
            .flex()
            .items_center()
            .h(px(26.))
            .px(px(12.))
            .rounded(px(4.))
            .border_1()
            .border_color(if is_selected { theme.primary } else { theme.border })
            .bg(if is_selected { theme.primary } else { theme.tab_background })
            .font_family(UI_FONT)
            .text_size(px(14.))
            .text_color(if is_selected { ON_ACCENT } else { theme.text_common })
            .child(SharedString::from(label))
            .on_click(cx.listener(move |_this, _event, _window, cx| {
                let mut draft = current_draft(cx);
                draft.forward_type = kind;
                write_draft(cx, draft);
            }));
        chips = chips.child(chip);
    }
    chips
}

/// The Bind address field.
fn bind_address_field(theme: TermiusTheme, draft: &PortForwardDraft) -> Div {
    InputField::new("Bind address", draft.bind_address.clone())
        .placeholder("127.0.0.1")
        .element(theme)
}

/// The type-dependent field set (`_main.js:73702-73825`):
/// * Local   — Local port number · Bind address · Intermediate host ·
///             Destination address · Destination port number
/// * Remote  — Remote host · Remote port number · Bind address ·
///             Destination address · Destination port number
/// * Dynamic — Local port number · Bind address · Intermediate host
fn type_fields(theme: TermiusTheme, draft: &PortForwardDraft) -> Div {
    let mut col = div().flex().flex_col().gap(px(12.));
    match draft.forward_type {
        ForwardType::Dynamic => {
            col = col.child(
                InputField::new("Local port number", port_value(draft.local_port))
                    .placeholder("1080")
                    .element(theme),
            );
            col = col.child(bind_address_field(theme, draft));
            col = col.child(
                InputField::new("Intermediate host", draft.intermediate_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            );
        }
        ForwardType::Remote | ForwardType::RemoteAuto => {
            col = col.child(
                InputField::new("Remote host", draft.intermediate_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            );
            col = col.child(
                InputField::new("Remote port number", port_value(draft.remote_port))
                    .placeholder("22")
                    .element(theme),
            );
            col = col.child(bind_address_field(theme, draft));
            col = col.child(
                InputField::new("Destination address", draft.destination_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            );
            col = col.child(
                InputField::new("Destination port number", port_value(draft.destination_port))
                    .placeholder("80")
                    .element(theme),
            );
        }
        _ => {
            col = col.child(
                InputField::new("Local port number", port_value(draft.local_port))
                    .placeholder("8080")
                    .element(theme),
            );
            col = col.child(bind_address_field(theme, draft));
            col = col.child(
                InputField::new("Intermediate host", draft.intermediate_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            );
            col = col.child(
                InputField::new("Destination address", draft.destination_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            );
            col = col.child(
                InputField::new("Destination port number", port_value(draft.destination_port))
                    .placeholder("80")
                    .element(theme),
            );
        }
    }
    col
}

/// The wizard header: `Step {n} of {m} · {step}`.
fn wizard_header(theme: TermiusTheme, draft: &PortForwardDraft) -> Div {
    let steps = wizard_steps(draft.forward_type);
    let total = steps.len();
    let index = draft.step.min(total.saturating_sub(1));
    let step = steps.get(index).copied().unwrap_or(PfStep::Type);
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            text::R14P
                .style(div())
                .text_color(theme.title)
                .child(SharedString::from(format!(
                    "Step {} of {} · {}",
                    index + 1,
                    total,
                    step.label()
                ))),
        )
        .child(SettingsText::new("Create a port forwarding rule step by step.").element(theme))
}

/// The current wizard step's controls.
fn wizard_step_body(
    theme: TermiusTheme,
    draft: &PortForwardDraft,
    cx: &mut Context<TermiusState>,
) -> Div {
    let steps = wizard_steps(draft.forward_type);
    let index = draft.step.min(steps.len().saturating_sub(1));
    match steps.get(index).copied().unwrap_or(PfStep::Type) {
        PfStep::Type => div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(field_label(theme, "Type"))
            .child(type_chips(theme, draft.forward_type, cx)),
        PfStep::LocalPort => div().child(
            InputField::new("Local port number", port_value(draft.local_port))
                .placeholder("8080")
                .element(theme),
        ),
        PfStep::RemotePort => div().child(
            InputField::new("Remote port number", port_value(draft.remote_port))
                .placeholder("22")
                .element(theme),
        ),
        PfStep::ServiceHost => {
            let remote = matches!(
                draft.forward_type,
                ForwardType::Remote | ForwardType::RemoteAuto
            );
            let label = if remote { "Remote host" } else { "Intermediate host" };
            div().child(
                InputField::new(label, draft.intermediate_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            )
        }
        PfStep::Destination => div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                InputField::new("Destination address", draft.destination_host.clone())
                    .placeholder("hostname or IP")
                    .element(theme),
            )
            .child(
                InputField::new("Destination port number", port_value(draft.destination_port))
                    .placeholder("80")
                    .element(theme),
            ),
        PfStep::Label => div().child(
            InputField::new("Label", draft.label.clone())
                .placeholder("Label")
                .element(theme),
        ),
    }
}

/// The wizard's Back / Next / Finish + Cancel row.
fn wizard_actions(
    theme: TermiusTheme,
    draft: &PortForwardDraft,
    cx: &mut Context<TermiusState>,
) -> Div {
    let steps = wizard_steps(draft.forward_type);
    let last = draft.step + 1 >= steps.len();
    let mut row = div().flex().items_center().justify_end().gap(px(8.)).pt(px(4.));
    if draft.step > 0 {
        row = row.child(Button::new("Back").secondary().on_click(
            theme,
            cx.listener(|_this, _event, _window, cx| {
                let mut draft = current_draft(cx);
                draft.step = draft.step.saturating_sub(1);
                write_draft(cx, draft);
            }),
        ));
    } else {
        row = row.child(Button::new("Cancel").secondary().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| this.close_dialog(cx)),
        ));
    }
    if last {
        row = row.child(Button::new("Finish").primary().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| {
                let config = current_draft(cx).to_config();
                this.add_port_forwarding(config, cx);
                this.close_dialog(cx);
            }),
        ));
    } else {
        row = row.child(Button::new("Next").primary().on_click(
            theme,
            cx.listener(|_this, _event, _window, cx| {
                let mut draft = current_draft(cx);
                draft.step += 1;
                write_draft(cx, draft);
            }),
        ));
    }
    row
}

/// The full form's Cancel / Save row.
fn form_actions(theme: TermiusTheme, cx: &mut Context<TermiusState>) -> Div {
    let cancel = Button::new("Cancel").secondary().on_click(
        theme,
        cx.listener(|this, _event, _window, cx| this.close_dialog(cx)),
    );
    let save = Button::new("Save").primary().on_click(
        theme,
        cx.listener(|this, _event, _window, cx| {
            let config = current_draft(cx).to_config();
            this.add_port_forwarding(config, cx);
            this.close_dialog(cx);
        }),
    );
    div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .pt(px(4.))
        .child(cancel)
        .child(save)
}

/// The host picker (the SSH host the tunnel is bound to).
fn host_picker(
    state: &TermiusState,
    theme: TermiusTheme,
    draft: &PortForwardDraft,
    cx: &mut Context<TermiusState>,
) -> Div {
    let mut col = div().flex().flex_col().gap(px(6.)).child(field_label(theme, "Host"));
    if state.library.hosts.is_empty() {
        col = col.child(
            SettingsText::new("No hosts yet — add one on the Hosts screen.").element(theme),
        );
        return col;
    }
    let mut picker = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(4.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(theme.border)
        .bg(theme.tab_background);
    for host in &state.library.hosts {
        let selected = draft.host_id.as_deref() == Some(host.id.as_str());
        picker = picker.child(host_pick_row(theme, host, selected, cx));
    }
    col.child(picker)
}

/// One selectable host row inside the dialog's host picker.
fn host_pick_row(
    theme: TermiusTheme,
    host: &Host,
    selected: bool,
    cx: &mut Context<TermiusState>,
) -> Stateful<Div> {
    let host_id = host.id.clone();
    let label = if host.label.trim().is_empty() {
        host.hostname.clone()
    } else {
        host.label.clone()
    };
    let mut row = div()
        .id(SharedString::from(format!("pf-pick-{}", host.id)))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .px(px(10.))
        .py(px(4.))
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.title)
        .child(text::R14P.style(div()).truncate().child(SharedString::from(label)))
        .child(
            text::R12S
                .style(div())
                .text_color(theme.text_common)
                .truncate()
                .child(SharedString::from(host.hostname.clone())),
        )
        .on_click(cx.listener(move |_this, _event, _window, cx| {
            let mut draft = current_draft(cx);
            draft.host_id = Some(host_id.clone());
            write_draft(cx, draft);
        }));
    if selected {
        row = row.bg(theme.card_c).border_1().border_color(theme.border_accent);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> PortForwardingConfig {
        PortForwardingConfig {
            listen_port: 8080,
            ..PortForwardingConfig::default()
        }
    }

    fn host(id: &str, label: &str) -> Host {
        Host {
            id: id.into(),
            label: label.into(),
            hostname: format!("{label}.test"),
            ..Host::default()
        }
    }

    #[test]
    fn rule_strings_match_termius_shape() {
        let mut local = rule();
        local.forward_type = ForwardType::Local;
        local.destination_host = Some("db.internal".into());
        local.destination_port = Some(5432);
        assert_eq!(rule_string(&local), "Local :8080 → db.internal:5432");

        // A SOCKS proxy listens without a destination.
        let mut dynamic = rule();
        dynamic.forward_type = ForwardType::Dynamic;
        dynamic.listen_port = 1080;
        dynamic.destination_host = None;
        dynamic.destination_port = None;
        assert_eq!(rule_string(&dynamic), "Dynamic :1080");

        // Half-filled destinations degrade instead of rendering ":0".
        let mut half = rule();
        half.destination_host = Some("cache".into());
        half.destination_port = None;
        assert_eq!(rule_string(&half), "Local :8080 → cache");
        let mut port_only = rule();
        port_only.destination_host = None;
        port_only.destination_port = Some(80);
        assert_eq!(rule_string(&port_only), "Local :8080 → :80");
    }

    #[test]
    fn subtitles_bind_host_interface_and_status() {
        let hosts = [host("h1", "web-1")];

        let mut bound = rule();
        bound.host_id = Some("h1".into());
        assert_eq!(
            rule_subtitle(&bound, &hosts, PfConnState::Disconnected),
            "web-1 · 127.0.0.1 · disconnected"
        );
        assert_eq!(
            rule_subtitle(&bound, &hosts, PfConnState::Connected),
            "web-1 · 127.0.0.1 · connected"
        );

        // An unknown (or missing) host still renders a readable subtitle.
        let mut orphan = rule();
        orphan.host_id = Some("gone".into());
        assert_eq!(
            rule_subtitle(&orphan, &hosts, PfConnState::Disconnected),
            "No host · 127.0.0.1 · disconnected"
        );

        // `bind_all` and an explicit interface both win over the default.
        let mut all = rule();
        all.bind_all = true;
        assert_eq!(
            rule_subtitle(&all, &hosts, PfConnState::Disconnected),
            "No host · 0.0.0.0 · disconnected"
        );
        let mut eth = rule();
        eth.listen_interface = "10.0.0.5".into();
        assert_eq!(
            rule_subtitle(&eth, &hosts, PfConnState::Disconnected),
            "No host · 10.0.0.5 · disconnected"
        );
    }

    #[test]
    fn forward_type_labels_are_termius_labels() {
        assert_eq!(forward_type_label(ForwardType::Local), "Local");
        assert_eq!(forward_type_label(ForwardType::Remote), "Remote");
        assert_eq!(forward_type_label(ForwardType::Dynamic), "Dynamic");
        // Auto/serial variants collapse onto their base label.
        assert_eq!(forward_type_label(ForwardType::LocalAuto), "Local");
        assert_eq!(forward_type_label(ForwardType::RemoteAuto), "Remote");
        assert_eq!(forward_type_label(ForwardType::LocalSerial), "Local");
    }

    #[test]
    fn rule_letters_track_the_presenter_registry() {
        // `pficons`: local_pf → "L", remote_pf → "R", dynamic_pf → "D".
        assert_eq!(rule_letter(ForwardType::Local), "L");
        assert_eq!(rule_letter(ForwardType::LocalAuto), "L");
        assert_eq!(rule_letter(ForwardType::LocalSerial), "L");
        assert_eq!(rule_letter(ForwardType::Remote), "R");
        assert_eq!(rule_letter(ForwardType::RemoteAuto), "R");
        assert_eq!(rule_letter(ForwardType::Dynamic), "D");
    }

    #[test]
    fn status_toggles_between_endpoints() {
        assert!(!PfConnState::Disconnected.is_connected());
        assert!(PfConnState::Connected.is_connected());
        assert_eq!(PfConnState::Disconnected.toggled(), PfConnState::Connected);
        assert_eq!(PfConnState::Connected.toggled(), PfConnState::Disconnected);
        assert_eq!(PfConnState::Connected.label(), "connected");
        assert_eq!(PfConnState::Disconnected.label(), "disconnected");
    }

    #[test]
    fn wizard_steps_match_termius() {
        assert_eq!(
            wizard_steps(ForwardType::Local),
            &[
                PfStep::Type,
                PfStep::LocalPort,
                PfStep::ServiceHost,
                PfStep::Destination,
                PfStep::Label
            ]
        );
        assert_eq!(
            wizard_steps(ForwardType::Remote),
            &[
                PfStep::Type,
                PfStep::ServiceHost,
                PfStep::RemotePort,
                PfStep::Destination,
                PfStep::Label
            ]
        );
        assert_eq!(
            wizard_steps(ForwardType::Dynamic),
            &[PfStep::Type, PfStep::LocalPort, PfStep::ServiceHost, PfStep::Label]
        );
    }

    #[test]
    fn draft_to_config_persists_type_dependent_fields() {
        let local = PortForwardDraft::with_host(ForwardType::Local, Some("h1".into()));
        assert_eq!(local.listen_port(), 8080);
        let config = local.to_config();
        assert!(config.id.is_empty()); // minted by `add_port_forwarding`
        assert_eq!(config.host_id.as_deref(), Some("h1"));
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.destination_host.as_deref(), Some("localhost"));
        assert_eq!(config.destination_port, Some(80));
        assert_eq!(config.listen_interface, "127.0.0.1");
        assert!(!config.bind_all);

        // Remote listens on the remote port.
        let remote = PortForwardDraft::with_host(ForwardType::Remote, None);
        assert_eq!(remote.listen_port(), 22);
        assert_eq!(remote.to_config().forward_type, ForwardType::Remote);

        // Dynamic becomes a SOCKS rule: no destination.
        let mut dynamic = PortForwardDraft::with_host(ForwardType::Dynamic, None);
        dynamic.local_port = 1080;
        let config = dynamic.to_config();
        assert_eq!(config.listen_port, 1080);
        assert!(config.destination_host.is_none());
        assert!(config.destination_port.is_none());

        // A bind-all address flips the flag.
        let mut all = PortForwardDraft::default();
        all.bind_address = "0.0.0.0".into();
        assert!(all.to_config().bind_all);
    }

    #[test]
    fn port_helpers_round_trip() {
        assert_eq!(port_value(0), "");
        assert_eq!(port_value(8080), "8080");
        assert_eq!(port_opt(0), None);
        assert_eq!(port_opt(80), Some(80));
        assert_eq!(non_empty("  "), None);
        assert_eq!(non_empty(" db "), Some("db".to_owned()));
    }

    #[test]
    fn toolbar_and_row_icons_are_bundled() {
        // Thin plus (not `addCircle.svg`), chevron, dots.
        assert!(crate::has_icon("plusThin.svg"));
        assert!(crate::has_icon("chevron.svg"));
        assert!(crate::has_icon("dots.svg"));
    }
}
