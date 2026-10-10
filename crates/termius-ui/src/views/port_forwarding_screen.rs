//! port_forwarding_screen — the Port Forwarding section: the rule list plus
//! the "Add Port Forward" dialog body.
//!
//! Termius' Port Forwarding screen lists every tunnel bound to a host —
//! `Type :listen → destination` with the bound host and listen interface
//! underneath — behind a "New forwarding" action that opens the add form
//! (type, ports, destination, bind address, host).
//!
//! # Row anatomy (ported from the original)
//!
//! Each row mirrors the recovered `EntityReceipt` / `PortForwardingRulePresenter`
//! (`analysis/readable/PortForwardingRulePresenter-4942c31a.js`, the
//! `ConnectedPortForwardings` screen in `analysis/readable/_main.js`):
//!
//! * a leading **real icon tile** — `local-port-forwarding-rule.svg`,
//!   `remote-port-forwarding.svg` or `dynamic-port-forwarding.svg`, chosen by
//!   the tunnel direction (the presenter's `local_pf` / `remote_pf` /
//!   `dynamic_pf`);
//! * the rule summary (`R14P`) over the bound host + listen interface (`R12S`);
//! * a trailing `dots.svg` overflow affordance;
//! * hover wash (`--blue-a10`) — the original has no persisted selection here.
//!
//! The header is the toolbar from the original: the `Port Forwarding` title and
//! a primary `New forwarding` button carrying the `addCircle.svg` glyph.
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
//! That is the same shape [`crate::views::host_list`] already uses — read a
//! different entity than the leased one from `Render`, wire events through
//! `cx.listener(..)` + `this.state.update(..)`.
//!
//! [`AppContext::update_entity`]: gpui::AppContext::update_entity
//! [`Entity::read`]: gpui::Entity::read

use gpui::{
    div, px, AnyElement, AppContext as _, ClickEvent, Context, Div, Entity, EntityId, FontWeight,
    Global, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
};
use termius_core::{ForwardType, Host, PortForwardingConfig};

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{Button, EmptyState, InputField, SectionHeader, SettingsText, Switch};
use crate::theme::{over, text, theme_of, with_alpha, TermiusTheme, ThemeMode};

/// Rule row height: two text lines beside the 28px icon tile.
const ROW_HEIGHT: f32 = 44.0;
/// Entity icon tile (`entityIcon`: `24px`, a touch larger to seat the richer
/// port-forwarding glyph).
const ICON_TILE: f32 = 28.0;
/// Glyph inside the icon tile.
const ICON_GLYPH: f32 = 16.0;
/// White used for text/glyphs on the accent button fill.
const ON_ACCENT: gpui::Rgba = gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// Termius' label for one tunnel direction.
fn forward_type_label(forward_type: ForwardType) -> &'static str {
    match forward_type {
        ForwardType::Local => "Local",
        ForwardType::Remote => "Remote",
        ForwardType::Dynamic => "Dynamic",
        ForwardType::LocalAuto => "Local (auto)",
        ForwardType::RemoteAuto => "Remote (auto)",
        ForwardType::LocalSerial => "Local Serial",
        ForwardType::LocalSerialAuto => "Local Serial (auto)",
    }
}

/// The leading icon for a rule, ported from
/// `PortForwardingRulePresenter.icon(..)` (the original `local_pf` / `remote_pf`
/// / `dynamic_pf` keys) onto the bundled SVG set.
fn forward_icon(forward_type: ForwardType) -> &'static str {
    match forward_type {
        ForwardType::Remote | ForwardType::RemoteAuto => "remote-port-forwarding.svg",
        ForwardType::Dynamic => "dynamic-port-forwarding.svg",
        ForwardType::Local
        | ForwardType::LocalAuto
        | ForwardType::LocalSerial
        | ForwardType::LocalSerialAuto => "local-port-forwarding-rule.svg",
    }
}

/// The row headline: `Local :8080 → db.internal:5432`.
///
/// A SOCKS ([`ForwardType::Dynamic`]) rule has no destination, so it falls
/// back to `Dynamic :1080` instead of showing an empty arrow.
fn rule_summary(rule: &PortForwardingConfig) -> String {
    let kind = forward_type_label(rule.forward_type);
    let listen = format!(":{}", rule.listen_port);
    match (&rule.destination_host, rule.destination_port) {
        (Some(host), Some(port)) => format!("{kind} {listen} → {host}:{port}"),
        (Some(host), None) => format!("{kind} {listen} → {host}"),
        (None, Some(port)) => format!("{kind} {listen} → :{port}"),
        // Dynamic / serial forwards listen without a destination.
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

/// The row subtitle: bound host label + listen interface (`web-1 · 127.0.0.1`).
fn rule_subtitle(rule: &PortForwardingConfig, hosts: &[Host]) -> String {
    let host = rule
        .host_id
        .as_deref()
        .and_then(|id| hosts.iter().find(|host| host.id == id))
        .map(|host| host.label.clone())
        .unwrap_or_else(|| "No host".to_owned());
    format!("{host} · {}", listen_label(rule))
}

// ---------------------------------------------------------------------------
// The add-form draft
// ---------------------------------------------------------------------------

/// The add-form selection that outlives one paint.
///
/// gpui gives a free function no local view state, so the chosen type, host
/// and bind-all flag ride in a [`Global`] until Save consumes them; the dialog
/// body reads it and the controls write it back (repainting through
/// [`Context::notify`]).
///
/// PORT-TODO: `primitives::InputField` is display-only, so listen/destination
/// are *samples* derived from the type rather than user input — the form
/// shows exactly what Save will persist. Swap them for real fields once
/// `InputField` grows a content model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PortForwardDraft {
    forward_type: ForwardType,
    host_id: Option<String>,
    /// Bind the listener to `0.0.0.0` (the "Bind to all interfaces" switch).
    bind_all: bool,
}

impl Global for PortForwardDraft {}

impl PortForwardDraft {
    /// A fresh Local draft bound to `host_id` (the first host, on open).
    fn with_host(host_id: Option<String>) -> Self {
        Self {
            forward_type: ForwardType::Local,
            host_id,
            bind_all: false,
        }
    }

    /// The sample listen port shown and saved (1080 is the SOCKS default).
    fn listen_port(&self) -> u16 {
        match self.forward_type {
            ForwardType::Dynamic => 1080,
            _ => 8080,
        }
    }

    /// The sample destination shown and saved (a SOCKS proxy has none).
    fn destination(&self) -> (Option<String>, Option<u16>) {
        match self.forward_type {
            ForwardType::Dynamic => (None, None),
            _ => (Some("localhost".to_owned()), Some(80)),
        }
    }

    /// The rule Save persists (`add_port_forwarding` mints the id).
    fn to_config(&self) -> PortForwardingConfig {
        let (destination_host, destination_port) = self.destination();
        PortForwardingConfig {
            id: String::new(),
            forward_type: self.forward_type,
            listen_interface: "127.0.0.1".to_owned(),
            listen_port: self.listen_port(),
            destination_host,
            destination_port,
            host_id: self.host_id.clone(),
            bind_all: self.bind_all,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

/// The current draft (a default when the dialog was raised elsewhere).
fn current_draft(cx: &Context<TermiusState>) -> PortForwardDraft {
    cx.try_global::<PortForwardDraft>().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Row / button chrome
// ---------------------------------------------------------------------------

/// The primary toolbar action: an accent button with a leading real icon
/// (`addCircle.svg`), mirroring the original `New forwarding` / `New key`
/// toolbar button (`Button`'s "substrate" hover, 36px tall, 16px inline
/// padding, `--corner-radius-medium`).
fn new_action_button(
    theme: TermiusTheme,
    id: &str,
    label: &'static str,
    listener: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let hover_fill = over(theme.primary, with_alpha(ON_ACCENT, 0.25));
    div()
        .id(SharedString::from(id.to_owned()))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(36.))
        .px(px(16.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.primary)
        .text_color(ON_ACCENT)
        .font_family(crate::assets::UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(crate::icon("addCircle.svg").w(px(14.)).h(px(14.)))
        .child(label)
        .hover(move |hover| hover.bg(hover_fill))
        .on_click(listener)
}

/// The square entity-icon tile shared by every row (`entityIcon`: card-tinted
/// square, muted glyph).
fn icon_tile(theme: TermiusTheme, icon_name: &str) -> Div {
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

/// The trailing `dots.svg` overflow affordance (the original row's context-menu
/// anchor).
///
/// PORT-TODO: gpui 0.2.2 has no anchored popup menu here, so the button stands
/// in for the context menu's `Remove` item and deletes the rule directly.
fn row_dots(
    id: String,
    theme: TermiusTheme,
    cx: &mut Context<PortForwardList>,
) -> Stateful<Div> {
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
        .text_color(theme.muted)
        .hover(move |hover| hover.bg(theme.hover))
        .child(crate::icon("dots.svg").w(px(12.)).h(px(4.)))
        .on_click(cx.listener(move |this, _event, _window, cx| {
            this.delete_rule(&rule_id, cx);
        }))
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// One painted rule row (the strings + icon [`PortForwardList`] hands to the
/// row chrome; built from a single `TermiusState` read).
struct RuleRow {
    id: String,
    label: String,
    subtitle: String,
    icon: &'static str,
}

/// The memoized list view for one `TermiusState`, so repeated renders reuse
/// the same entity instead of minting one per frame (the state notifies on
/// every session byte, and the shell re-renders with it).
///
/// Held weakly: once the shell stops rendering the screen the view — and with
/// it the `TermiusState` it keeps alive — is released.
struct PortForwardScreenMemo {
    state: EntityId,
    view: WeakEntity<PortForwardList>,
}

impl Global for PortForwardScreenMemo {}

/// The Port Forwarding rule list (see the module docs for why this is a view
/// entity rather than a plain element tree).
pub struct PortForwardList {
    state: Entity<TermiusState>,
}

impl PortForwardList {
    fn new(state: Entity<TermiusState>, _cx: &mut Context<Self>) -> Self {
        Self { state }
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
            .map(|rule| RuleRow {
                id: rule.id.clone(),
                label: rule_summary(rule),
                subtitle: rule_subtitle(rule, &state.library.hosts),
                icon: forward_icon(rule.forward_type),
            })
            .collect()
    }

    /// Seed the draft with the first host and raise the add dialog.
    fn open_new_rule(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            let host_id = state.library.hosts.first().map(|host| host.id.clone());
            cx.set_global(PortForwardDraft::with_host(host_id));
            state.open_dialog(Dialog::AddPortForward, cx);
        });
    }

    /// Drop one rule (the row's `dots.svg` menu anchor).
    fn delete_rule(&mut self, rule_id: &str, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.delete_port_forwarding(rule_id, cx));
    }

    /// One painted rule row: icon tile · title/meta · trailing dots.
    fn rule_row(&self, row: &RuleRow, theme: TermiusTheme, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(SharedString::from(format!("pf-row-{}", row.id)))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_HEIGHT))
            .px(px(12.))
            .rounded(px(theme.corner_radius_small))
            .text_color(theme.title)
            .hover(move |hover| hover.bg(theme.hover))
            .child(icon_tile(theme, row.icon))
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
                            .child(SharedString::from(row.label.clone())),
                    )
                    .child(
                        text::R12S
                            .style(div())
                            .text_color(theme.muted)
                            .truncate()
                            .child(SharedString::from(row.subtitle.clone())),
                    ),
            )
            .child(row_dots(row.id.clone(), theme, cx))
    }
}

impl Render for PortForwardList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let rows = self.rule_rows(cx);

        // Header: section title + the primary "New forwarding" action
        // (`addCircle.svg`), ported from the original toolbar.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.))
            .border_b_1()
            .border_color(theme.border)
            .child(SectionHeader::new("Port Forwarding").element(theme))
            .child(div().px(px(12.)).child(new_action_button(
                theme,
                "pf-new-forwarding",
                "New forwarding",
                cx.listener(|this, _event, _window, cx| this.open_new_rule(cx)),
            )));

        // Body: the rules, or Termius' empty state (`V8e` in the original).
        //
        // PORT-TODO: gpui 0.2.2's `Styled` has no scrollable overflow helper
        // (only `overflow_hidden`/`overflow_x/y_hidden`), so long rule lists
        // clip instead of scrolling — wire a scrollable list element (a
        // uniform list + scroll handle) when the rules outgrow the window.
        let mut body = div().flex().flex_col().flex_1().min_h(px(0.)).overflow_hidden();
        if rows.is_empty() {
            body = body.child(
                EmptyState::new(
                    "Set up port forwarding",
                    "Save port forwarding to access databases, web apps, and other services.",
                )
                .element(theme),
            );
        } else {
            let mut list = div().flex().flex_col().gap(px(2.)).px(px(8.)).py(px(8.));
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

/// The Port Forwarding screen: a column of rules with a "New forwarding"
/// action.
///
/// Call it while holding a `Context<TermiusState>` (i.e. from inside
/// `state.update(..)`): it returns a [`PortForwardList`] view, whose own
/// render reads `state` safely — see the module docs.
///
/// Safe to call on every render: the view is memoized per `TermiusState`
/// (one weak entry in [`PortForwardScreenMemo`]), so no entity is minted per
/// frame the way a bare `cx.new(..)` would be.
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
/// shell hosts inside a [`DialogFrame`](crate::primitives::DialogFrame)
/// card; the frame supplies the title bar and card fill.
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
    div().text_xs().text_color(theme.muted).child(SharedString::from(text))
}

/// The "Add Port Forward" form: type chooser → ports → bind switch → host
/// picker → actions.
fn add_port_forward_form(state: &TermiusState, cx: &mut Context<TermiusState>) -> AnyElement {
    let theme = theme_of(cx);
    let draft = current_draft(cx);
    let dynamic = matches!(draft.forward_type, ForwardType::Dynamic);

    let mut form = div().flex().flex_col().gap(px(12.));

    // ----- type chooser (Local / Remote / Dynamic) -----
    form = form.child(field_label(theme, "Type"));
    let mut chips = div().flex().gap(px(8.));
    for kind in [ForwardType::Local, ForwardType::Remote, ForwardType::Dynamic] {
        let label = forward_type_label(kind);
        let selected = draft.forward_type == kind;
        let chip = div()
            .id(SharedString::from(format!("pf-type-{label}")))
            .flex()
            .items_center()
            .h(px(26.))
            .px(px(12.))
            .rounded(px(4.))
            .border_1()
            .border_color(if selected { theme.primary } else { theme.border })
            .bg(if selected { theme.primary } else { theme.tab_background })
            .text_sm()
            .text_color(if selected { ON_ACCENT } else { theme.muted })
            .child(SharedString::from(label))
            .on_click(cx.listener(move |_this, _event, _window, cx| {
                let mut draft = current_draft(cx);
                draft.forward_type = kind;
                cx.set_global(draft);
                cx.notify();
            }));
        chips = chips.child(chip);
    }
    form = form.child(chips);

    // ----- ports + destination (display-only sample values) -----
    form = form.child(
        InputField::new("Listen port", draft.listen_port().to_string())
            .placeholder("8080")
            .element(theme),
    );
    if dynamic {
        form = form.child(
            SettingsText::new("A dynamic (SOCKS) rule has no destination — it proxies every connection.")
                .element(theme),
        );
    } else {
        let (destination_host, destination_port) = draft.destination();
        form = form.child(
            InputField::new("Destination host", destination_host.unwrap_or_default())
                .placeholder("hostname or IP")
                .element(theme),
        );
        form = form.child(
            InputField::new(
                "Destination port",
                destination_port.map(|port| port.to_string()).unwrap_or_default(),
            )
            .placeholder("80")
            .element(theme),
        );
    }

    // ----- bind address -----
    form = form.child(
        Switch::new("Bind to all interfaces", draft.bind_all)
            .description("Listen on 0.0.0.0 instead of 127.0.0.1.")
            .on_click(
                theme,
                cx.listener(|_this, _event, _window, cx| {
                    let mut draft = current_draft(cx);
                    draft.bind_all = !draft.bind_all;
                    cx.set_global(draft);
                    cx.notify();
                }),
            ),
    );

    // ----- host picker -----
    form = form.child(field_label(theme, "Host"));
    if state.library.hosts.is_empty() {
        form = form.child(
            SettingsText::new("No hosts yet — add one on the Hosts screen.").element(theme),
        );
    } else {
        let mut picker = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(4.))
            .rounded(px(4.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.tab_background);
        for host in &state.library.hosts {
            let host_id = host.id.clone();
            let selected = draft.host_id.as_deref() == Some(host.id.as_str());
            let item = host_pick_row(theme, host, selected, cx);
            picker = picker.child(item);
        }
        form = form.child(picker);
    }

    // ----- actions -----
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
    form = form.child(
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .pt(px(4.))
            .child(cancel)
            .child(save),
    );

    form.into_any_element()
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
                .text_color(theme.muted)
                .truncate()
                .child(SharedString::from(host.hostname.clone())),
        )
        .on_click(cx.listener(move |_this, _event, _window, cx| {
            let mut draft = current_draft(cx);
            draft.host_id = Some(host_id.clone());
            cx.set_global(draft);
            cx.notify();
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
    fn summaries_match_termius_shape() {
        let mut local = rule();
        local.forward_type = ForwardType::Local;
        local.destination_host = Some("db.internal".into());
        local.destination_port = Some(5432);
        assert_eq!(rule_summary(&local), "Local :8080 → db.internal:5432");

        // A SOCKS proxy listens without a destination.
        let mut dynamic = rule();
        dynamic.forward_type = ForwardType::Dynamic;
        dynamic.listen_port = 1080;
        dynamic.destination_host = None;
        dynamic.destination_port = None;
        assert_eq!(rule_summary(&dynamic), "Dynamic :1080");

        // Half-filled destinations degrade instead of rendering ":0".
        let mut half = rule();
        half.destination_host = Some("cache".into());
        half.destination_port = None;
        assert_eq!(rule_summary(&half), "Local :8080 → cache");
        let mut port_only = rule();
        port_only.destination_host = None;
        port_only.destination_port = Some(80);
        assert_eq!(rule_summary(&port_only), "Local :8080 → :80");
    }

    #[test]
    fn subtitles_bind_host_and_interface() {
        let hosts = [host("h1", "web-1")];

        let mut bound = rule();
        bound.host_id = Some("h1".into());
        assert_eq!(rule_subtitle(&bound, &hosts), "web-1 · 127.0.0.1");

        // An unknown (or missing) host still renders a readable subtitle.
        let mut orphan = rule();
        orphan.host_id = Some("gone".into());
        assert_eq!(rule_subtitle(&orphan, &hosts), "No host · 127.0.0.1");
        assert_eq!(rule_subtitle(&rule(), &hosts), "No host · 127.0.0.1");

        // `bind_all` and an explicit interface both win over the default.
        let mut all = rule();
        all.bind_all = true;
        assert_eq!(rule_subtitle(&all, &hosts), "No host · 0.0.0.0");
        let mut eth = rule();
        eth.listen_interface = "10.0.0.5".into();
        assert_eq!(rule_subtitle(&eth, &hosts), "No host · 10.0.0.5");
    }

    #[test]
    fn forward_type_labels_are_termius_labels() {
        assert_eq!(forward_type_label(ForwardType::Local), "Local");
        assert_eq!(forward_type_label(ForwardType::Remote), "Remote");
        assert_eq!(forward_type_label(ForwardType::Dynamic), "Dynamic");
    }

    #[test]
    fn forward_icons_track_the_presenter() {
        assert_eq!(forward_icon(ForwardType::Local), "local-port-forwarding-rule.svg");
        assert_eq!(forward_icon(ForwardType::LocalAuto), "local-port-forwarding-rule.svg");
        assert_eq!(forward_icon(ForwardType::LocalSerial), "local-port-forwarding-rule.svg");
        assert_eq!(forward_icon(ForwardType::Remote), "remote-port-forwarding.svg");
        assert_eq!(forward_icon(ForwardType::RemoteAuto), "remote-port-forwarding.svg");
        assert_eq!(forward_icon(ForwardType::Dynamic), "dynamic-port-forwarding.svg");
        // Every icon the row chrome asks for is bundled.
        for kind in [
            ForwardType::Local,
            ForwardType::Remote,
            ForwardType::Dynamic,
            ForwardType::LocalAuto,
            ForwardType::LocalSerialAuto,
            ForwardType::RemoteAuto,
            ForwardType::LocalSerial,
        ] {
            assert!(crate::has_icon(forward_icon(kind)));
        }
    }

    #[test]
    fn draft_samples_drive_what_save_persists() {
        let local = PortForwardDraft::with_host(Some("h1".into()));
        assert_eq!(local.forward_type, ForwardType::Local);
        assert_eq!(local.listen_port(), 8080);

        let config = local.to_config();
        assert!(config.id.is_empty()); // minted by `add_port_forwarding`
        assert_eq!(config.host_id.as_deref(), Some("h1"));
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.destination_host.as_deref(), Some("localhost"));
        assert_eq!(config.destination_port, Some(80));
        assert_eq!(config.listen_interface, "127.0.0.1");
        assert!(!config.bind_all);

        // Dynamic becomes a SOCKS rule: no destination, SOCKS default port.
        let dynamic = PortForwardDraft {
            forward_type: ForwardType::Dynamic,
            host_id: None,
            bind_all: true,
        };
        let config = dynamic.to_config();
        assert_eq!(config.listen_port, 1080);
        assert!(config.destination_host.is_none());
        assert!(config.destination_port.is_none());
        assert!(config.host_id.is_none());
        assert!(config.bind_all);
    }

    #[test]
    fn rule_row_icons_are_bundled() {
        assert!(crate::has_icon("local-port-forwarding-rule.svg"));
        assert!(crate::has_icon("remote-port-forwarding.svg"));
        assert!(crate::has_icon("dynamic-port-forwarding.svg"));
        assert!(crate::has_icon("addCircle.svg"));
        assert!(crate::has_icon("dots.svg"));
    }
}
