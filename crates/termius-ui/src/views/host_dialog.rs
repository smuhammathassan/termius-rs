//! host_dialog — the Add Host / Edit Host **slider form** body.
//!
//! In v10 the entity editors are **right-side sliders**, not modals
//! (`analysis/recon/32-dialogs.md` §G). `dhe` (`readable/_main.js:47152-47348`)
//! renders a `FormHeader` ("New Host" / "Host Details" + saving status + a
//! three-dots menu) over a scrollable `FormContent` of `FormCard`s, with a
//! full-width **Connect** button pinned to the bottom — there is **no
//! Cancel/Save row**; the form autosaves.
//!
//! This module rebuilds that body for [`Dialog::AddHost`] / [`Dialog::EditHost`]
//! and is called by the shell from inside `TermiusState::update(..)`:
//!
//! ```ignore
//! let body = self
//!     .state
//!     .update(cx, |state, cx| host_dialog_body(dialog, state, cx));
//! ```
//!
//! Every other dialog returns `None`, so the shell keeps its generic body.
//!
//! The shell currently wraps the returned element in a
//! [`DialogFrame`](crate::primitives::DialogFrame) (`complete_card = false`),
//! which supplies the outer card + title; the **content** here is the slider
//! form (header + field cards + Connect). That outer frame is a known stand-in
//! for the real slider chrome.
//!
//! # Field set (`HostForm`, `reconnectSaga-f0db0c3c.js`)
//!
//! * **Address** (`placeholder "IP or Hostname"`, **required**) → `host.hostname`
//! * **IP Version** select
//! * **General**: `Label`, a **Group** autocomplete (`"Parent Group"`), a
//!   **Tags** creatable multi-select, and a **Backspace** selector
//! * the **SSH/Telnet config** group: the bound **Identity**, **Port**, **Username**
//!
//! # PORT-TODOs
//!
//! * [`InputField`](crate::primitives::InputField) is display-only, so the
//!   text fields show the seeded values and **Connect / Duplicate rebuild the
//!   record from exactly those displayed values**. Real editing (`Content` +
//!   `InputEvent` on a tracked `FocusHandle`) lands with the forms wave; until
//!   then Add is gated by [`Host::validate`] and Connect surfaces the error
//!   instead of minting an unlabeled host.
//! * Fields the model lacks — **IP Version**, **Tags**, **Backspace** — are
//!   view-local draft state (dropped on save). **Proxy** forwarding config is
//!   not modelled at all and is omitted.
//! * The Group / Identity pickers and the IP-Version / Backspace selects cycle
//!   their options on click because gpui 0.2.2 has no popup primitive.
//!
//! The picker *is* interactive: clicks stash the pending pick in the
//! [`HostDraft`] gpui global (re-seeded whenever a different dialog opens), and
//! save folds it into the record — an untouched pick never rewrites
//! memberships, a changed one moves the host the way Termius' single-group
//! picker does.

use gpui::{
    div, px, AnyElement, App, ClickEvent, Context, Div, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use termius_core::host::HostType;
use termius_core::Host;

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{Button, InputField, ShapedIcon};
use crate::theme::{text, theme_of, TermiusTheme};

/// Port shown (and used) when a host has none — Termius' default SSH port.
const DEFAULT_PORT: u16 = 22;
/// The `IP Version` select options (`HostForm`).
const IP_VERSIONS: [&str; 3] = ["Auto", "IPv4", "IPv6"];
/// The `Backspace` selector options (`ButtonMenuSelector`, `:250`).
const BACKSPACE_OPTIONS: [&str; 2] = ["Default", "Ctrl+H"];
/// The slider header title for a new host (`dD_1`, `_main.js:47220`).
const NEW_HOST_TITLE: &str = "New Host";
/// The slider header title when editing (`dD_1`).
const HOST_DETAILS_TITLE: &str = "Host Details";
/// The saving-status label (`useVs(savingStatus)`; always clean today).
const SAVED_LABEL: &str = "Saved";
/// The entity icon on the Address card (`HostPresenter.icon`).
const HOST_ICON: &str = "host.svg";

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// What the open dialog is editing, resolved against the current library.
enum HostForm<'a> {
    /// A brand-new host.
    Add,
    /// The existing host being edited.
    Edit(&'a Host),
}

/// Resolve `dialog` against `hosts`: `None` when it is not a host dialog
/// (or the edit target no longer exists) and the shell should keep its
/// fallback body.
fn host_form<'a>(dialog: &Dialog, hosts: &'a [Host]) -> Option<HostForm<'a>> {
    match dialog {
        Dialog::AddHost => Some(HostForm::Add),
        Dialog::EditHost(id) => hosts.iter().find(|host| host.id == *id).map(HostForm::Edit),
        _ => None,
    }
}

/// The text fields as displayed (Edit: pre-filled; Add: empty/"22").
#[derive(Debug, Clone, PartialEq, Eq)]
struct FormValues {
    label: String,
    hostname: String,
    port: String,
    username: String,
}

/// Prefill the fields from the host being edited, or the Add defaults.
fn form_values(host: Option<&Host>) -> FormValues {
    match host {
        Some(host) => FormValues {
            label: host.label.clone(),
            hostname: host.hostname.clone(),
            port: host.port.to_string(),
            username: host.username.clone(),
        },
        None => FormValues {
            label: String::new(),
            hostname: String::new(),
            port: DEFAULT_PORT.to_string(),
            username: String::new(),
        },
    }
}

/// Parse the displayed port, falling back to [`DEFAULT_PORT`] (the field is
/// display-only today, so this only guards against corrupt records).
fn parse_port(text: &str) -> u16 {
    match text.trim().parse::<u16>() {
        Ok(port) if port > 0 => port,
        _ => DEFAULT_PORT,
    }
}

/// The config-card title for a host type (`SSH` / `Telnet` / …).
fn host_type_label(host_type: HostType) -> &'static str {
    match host_type {
        HostType::Ssh => "SSH",
        HostType::Telnet => "Telnet",
        HostType::Mosh => "Mosh",
        HostType::Local => "Local",
        HostType::Serial => "Serial",
    }
}

/// The pending group/identity/type selection and local-only UI state for the
/// open host dialog.
///
/// A gpui [`Global`] because the shell rebuilds the dialog body on every
/// repaint: the draft outlives each build, and [`host_dialog_body`] re-seeds
/// it whenever a *different* dialog opens (closed dialogs reset it to
/// [`HostDraft::default`]).
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostDraft {
    /// The dialog this draft belongs to; a mismatch forces a re-seed.
    dialog: Dialog,
    /// Group the dialog opened with (an untouched draft never moves the host).
    initial_group: Option<String>,
    group_id: Option<String>,
    initial_identity: Option<String>,
    identity_id: Option<String>,
    /// The transport protocol (`Add SSH` / `Add Telnet`; model `host_type`).
    host_type: HostType,
    /// Whether the three-dots menu is expanded.
    menu_open: bool,
    /// Index into [`IP_VERSIONS`] (model has no `ip_version`).
    ip_version: usize,
    /// Index into [`BACKSPACE_OPTIONS`] (model has no backspace field).
    backspace: usize,
}

impl Global for HostDraft {}

impl Default for HostDraft {
    fn default() -> Self {
        Self {
            dialog: Dialog::AddHost,
            initial_group: None,
            group_id: None,
            initial_identity: None,
            identity_id: None,
            host_type: HostType::default(),
            menu_open: false,
            ip_version: 0,
            backspace: 0,
        }
    }
}

impl HostDraft {
    /// Seed from the host being edited (`None` for Add).
    fn new(dialog: &Dialog, host: Option<&Host>) -> Self {
        let initial_group = host.and_then(|host| host.group_ids.first().cloned());
        let initial_identity = host.and_then(|host| host.identity_id.clone());
        Self {
            dialog: dialog.clone(),
            initial_group: initial_group.clone(),
            group_id: initial_group,
            initial_identity: initial_identity.clone(),
            identity_id: initial_identity,
            host_type: host.map(|host| host.host_type).unwrap_or_default(),
            menu_open: false,
            ip_version: 0,
            backspace: 0,
        }
    }

    /// Pick `group_id`, or clear the pick when it is already selected.
    fn toggle_group(&mut self, group_id: &str) {
        self.group_id = if self.group_id.as_deref() == Some(group_id) {
            None
        } else {
            Some(group_id.to_owned())
        };
    }

    /// Pick `identity_id`, or clear the pick when it is already selected.
    fn toggle_identity(&mut self, identity_id: &str) {
        self.identity_id = if self.identity_id.as_deref() == Some(identity_id) {
            None
        } else {
            Some(identity_id.to_owned())
        };
    }

    /// Fold the pending picks into `host` (Edit keeps its id and untouched
    /// fields; Add starts from `Host::default()`).
    fn apply(&self, host: &mut Host) {
        // An untouched pick never rewrites memberships (multi-group hosts
        // survive a plain save); a changed one moves the host outright.
        if self.group_id != self.initial_group {
            host.group_ids = match self.group_id.clone() {
                Some(group_id) => vec![group_id],
                None => Vec::new(),
            };
        }
        host.identity_id = self.identity_id.clone();
        host.host_type = self.host_type;
    }
}

/// Build the record save writes: start from the live host (Edit) or a
/// default (Add), overlay the displayed fields, apply the picker draft.
fn build_host(
    existing: Option<&Host>,
    values: &FormValues,
    port: u16,
    draft: &HostDraft,
) -> Host {
    let mut host = existing.cloned().unwrap_or_default();
    host.label = values.label.clone();
    host.hostname = values.hostname.clone();
    host.port = port;
    host.username = values.username.clone();
    draft.apply(&mut host);
    host
}

/// Read the current dialog draft (falls back to the default when unseeded).
fn draft_of(cx: &Context<TermiusState>) -> HostDraft {
    cx.try_global::<HostDraft>()
        .cloned()
        .unwrap_or_default()
}

/// The existing host an edit dialog targets, if any.
fn existing_host<'a>(dialog: &Dialog, state: &'a TermiusState) -> Option<&'a Host> {
    match dialog {
        Dialog::EditHost(id) => state.library.host(id.as_str()),
        _ => None,
    }
}

/// Persist the form and open a session (the Connect button + menu item).
fn connect_host(
    this: &mut TermiusState,
    cx: &mut Context<TermiusState>,
    dialog: &Dialog,
    values: &FormValues,
) {
    let draft = draft_of(cx);
    // The edit target vanished while the dialog was open.
    if let Dialog::EditHost(id) = dialog {
        if this.library.host(id.as_str()).is_none() {
            this.status_text = format!("Edit Host: `{id}` no longer exists");
            cx.set_global(HostDraft::default());
            this.close_dialog(cx);
            return;
        }
    }
    let existing = existing_host(dialog, this).cloned();
    let host = build_host(existing.as_ref(), values, parse_port(&values.port), &draft);
    match host.validate() {
        Ok(()) => {
            let target = match dialog {
                Dialog::EditHost(id) => {
                    this.update_host(host, cx);
                    Some(id.clone())
                }
                _ => {
                    this.add_host(host, cx);
                    this.library.hosts.last().map(|host| host.id.clone())
                }
            };
            cx.set_global(HostDraft::default());
            if let Some(id) = target {
                this.open_connection(&id, cx);
            }
            this.close_dialog(cx);
        }
        Err(err) => {
            // Display-only fields: surface why Connect was refused and keep
            // the slider open (Termius blocks save until Address is filled).
            this.status_text = format!("{}: {err}", dialog.title());
            cx.notify();
        }
    }
}

/// Switch the host's transport protocol (`Add SSH` / `Add Telnet`) and
/// autosave it when the record is already valid.
fn set_host_type(
    this: &mut TermiusState,
    cx: &mut Context<TermiusState>,
    dialog: &Dialog,
    values: &FormValues,
    host_type: HostType,
) {
    let mut draft = draft_of(cx);
    draft.host_type = host_type;
    draft.menu_open = false;
    cx.set_global(draft.clone());
    if let Dialog::EditHost(_) = dialog {
        let existing = existing_host(dialog, this).cloned();
        let host = build_host(existing.as_ref(), values, parse_port(&values.port), &draft);
        if host.validate().is_ok() {
            this.update_host(host, cx);
        }
    }
    cx.notify();
}

/// Duplicate the current host (`Duplicate`), minting a fresh id.
fn duplicate_host(
    this: &mut TermiusState,
    cx: &mut Context<TermiusState>,
    dialog: &Dialog,
    values: &FormValues,
) {
    let draft = draft_of(cx);
    let existing = existing_host(dialog, this).cloned();
    let mut host = build_host(existing.as_ref(), values, parse_port(&values.port), &draft);
    match host.validate() {
        Ok(()) => {
            host.id = String::new();
            host.label = if host.label.trim().is_empty() {
                NEW_HOST_TITLE.to_owned()
            } else {
                format!("{} copy", host.label)
            };
            this.add_host(host, cx);
            cx.set_global(HostDraft::default());
            this.close_dialog(cx);
        }
        Err(err) => {
            this.status_text = format!("Duplicate: {err}");
            cx.notify();
        }
    }
}

/// Remove the host (`Remove`) — a no-op save-wise for an unsaved Add.
fn remove_host(this: &mut TermiusState, cx: &mut Context<TermiusState>, dialog: &Dialog) {
    if let Dialog::EditHost(id) = dialog {
        this.delete_host(id.as_str(), cx);
    }
    cx.set_global(HostDraft::default());
    this.close_dialog(cx);
}

// ---------------------------------------------------------------------------
// The dialog body
// ---------------------------------------------------------------------------

/// The Add/Edit Host slider-form body for [`Dialog::AddHost`] /
/// [`Dialog::EditHost`].
///
/// Returns `None` for every other dialog — and for an edit whose host has
/// vanished — so the shell falls back to its generic body.
pub fn host_dialog_body(
    dialog: &Dialog,
    state: &TermiusState,
    cx: &mut Context<TermiusState>,
) -> Option<AnyElement> {
    let form = host_form(dialog, &state.library.hosts)?;
    let existing = match &form {
        HostForm::Add => None,
        HostForm::Edit(host) => Some(*host),
    };
    let is_edit = matches!(&form, HostForm::Edit(_));
    let theme = theme_of(cx);
    let values = form_values(existing);

    // Re-seed the draft whenever a different dialog opened; keep the user's
    // pending picks / menu state across re-renders of the same dialog.
    let draft_stale = cx
        .try_global::<HostDraft>()
        .map(|draft| draft.dialog != *dialog)
        .unwrap_or(true);
    if draft_stale {
        cx.set_global(HostDraft::new(dialog, existing));
    }
    let draft = draft_of(cx);

    // ----- header (FormHeader) --------------------------------------------
    let title = if is_edit { HOST_DETAILS_TITLE } else { NEW_HOST_TITLE };
    let mut header = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .min_w(px(0.))
                .child(
                    text::B16P
                        .style(div())
                        .text_color(theme.title)
                        .child(SharedString::from(title)),
                )
                .child(saving_status(theme)),
        )
        .child(dots_button(theme, draft.menu_open, cx));
    let mut body = div().flex().flex_col().gap(px(15.)).w_full().child(header);

    // ----- three-dots menu -------------------------------------------------
    if draft.menu_open {
        body = body.child(host_menu(dialog, &values, theme, cx));
    }

    // ----- Address card ----------------------------------------------------
    let ip_label = IP_VERSIONS[draft.ip_version.min(IP_VERSIONS.len() - 1)];
    let mut address = form_card(theme, "Address");
    address = address.child(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                ShapedIcon::new(HOST_ICON)
                    .size(36.)
                    .glyph_size(20.)
                    .corner_radius(8.)
                    .element(theme),
            )
            .child(
                div().flex_1().min_w(px(0.)).child(
                    InputField::new("Address", values.hostname.clone())
                        .placeholder("IP or Hostname")
                        .element(theme),
                ),
            ),
    );
    address = address.child(dropdown_row(
        theme,
        "host-ip-version",
        "IP Version",
        SharedString::from(ip_label),
        cx.listener(|_this, _event, _window, cx| {
            let mut draft = draft_of(cx);
            draft.ip_version = (draft.ip_version + 1) % IP_VERSIONS.len();
            cx.set_global(draft);
            cx.notify();
        }),
    ));
    body = body.child(address);

    // ----- General card ----------------------------------------------------
    let mut general = form_card(theme, "General");
    general = general.child(
        InputField::new("Label", values.label.clone())
            .placeholder("Label")
            .element(theme),
    );
    general = general.child(group_picker(theme, state, &draft, cx));
    // PORT-TODO: the model has no tags; the creatable multi-select is a stub.
    general = general.child(
        InputField::new("Tags", "")
            .placeholder("Tags")
            .element(theme),
    );
    let backspace_label = BACKSPACE_OPTIONS[draft.backspace.min(BACKSPACE_OPTIONS.len() - 1)];
    general = general.child(dropdown_row(
        theme,
        "host-backspace",
        "Backspace",
        SharedString::from(backspace_label),
        cx.listener(|_this, _event, _window, cx| {
            let mut draft = draft_of(cx);
            draft.backspace = (draft.backspace + 1) % BACKSPACE_OPTIONS.len();
            cx.set_global(draft);
            cx.notify();
        }),
    ));
    body = body.child(general);

    // ----- SSH / Telnet config card ---------------------------------------
    let mut config = form_card(theme, host_type_label(draft.host_type));
    config = config.child(identity_picker(theme, state, &draft, cx));
    config = config.child(
        InputField::new("Port", values.port.clone())
            .placeholder("22")
            .element(theme),
    );
    config = config.child(
        InputField::new("Username", values.username.clone())
            .placeholder("root")
            .element(theme),
    );
    body = body.child(config);

    // ----- Connect button (connectButton) ---------------------------------
    body = body.child(connect_button(dialog, &values, theme, cx));

    Some(body.into_any_element())
}

// ---------------------------------------------------------------------------
// Header pieces
// ---------------------------------------------------------------------------

/// The saving-status indicator (`useVs(savingStatus)`): a check + "Saved".
fn saving_status(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .flex_shrink_0()
        .text_color(theme.success)
        .child(crate::icon("check.svg").w(px(12.)).h(px(12.)))
        .child(
            text::R12S
                .style(div())
                .text_color(theme.text_common)
                .child(SharedString::from(SAVED_LABEL)),
        )
}

/// The three-dots menu toggle (`eq ghost "Three dots button"`).
fn dots_button(
    theme: TermiusTheme,
    open: bool,
    cx: &mut Context<TermiusState>,
) -> Stateful<Div> {
    let hover = theme.hover;
    let mut button = div()
        .id("host-dots")
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.))
        .flex_shrink_0()
        .rounded(px(theme.corner_radius_small))
        .cursor_pointer()
        .text_color(theme.title)
        .child(crate::icon("dots.svg").w(px(16.)).h(px(4.)));
    if open {
        button = button.bg(hover);
    }
    button
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(|_this, _event, _window, cx| {
            let mut draft = draft_of(cx);
            draft.menu_open = !draft.menu_open;
            cx.set_global(draft);
            cx.notify();
        }))
}

/// The three-dots menu (`eN_1`): Connect / Add SSH / Add Telnet / Duplicate /
/// Remove.
fn host_menu(
    dialog: &Dialog,
    values: &FormValues,
    theme: TermiusTheme,
    cx: &mut Context<TermiusState>,
) -> Div {
    let mut menu = div()
        .flex()
        .flex_col()
        .w_full()
        .p(px(6.))
        .rounded(px(theme.corner_radius_medium))
        .border_1()
        .border_color(theme.border_basic)
        .bg(theme.card_c);

    menu = menu.child(menu_item(
        theme,
        "host-menu-connect",
        "connect.svg",
        "Connect",
        false,
        cx.listener({
            let dialog = dialog.clone();
            let values = values.clone();
            move |this, _event, _window, cx| {
                connect_host(this, cx, &dialog, &values);
            }
        }),
    ));
    menu = menu.child(menu_item(
        theme,
        "host-menu-ssh",
        "ssh.svg",
        "Add SSH",
        false,
        cx.listener({
            let dialog = dialog.clone();
            let values = values.clone();
            move |this, _event, _window, cx| {
                set_host_type(this, cx, &dialog, &values, HostType::Ssh);
            }
        }),
    ));
    menu = menu.child(menu_item(
        theme,
        "host-menu-telnet",
        "terminal.svg",
        "Add Telnet",
        false,
        cx.listener({
            let dialog = dialog.clone();
            let values = values.clone();
            move |this, _event, _window, cx| {
                set_host_type(this, cx, &dialog, &values, HostType::Telnet);
            }
        }),
    ));
    menu = menu.child(menu_item(
        theme,
        "host-menu-duplicate",
        "duplicate.svg",
        "Duplicate",
        false,
        cx.listener({
            let dialog = dialog.clone();
            let values = values.clone();
            move |this, _event, _window, cx| {
                duplicate_host(this, cx, &dialog, &values);
            }
        }),
    ));
    menu.child(menu_item(
        theme,
        "host-menu-remove",
        "remove.svg",
        "Remove",
        true,
        cx.listener({
            let dialog = dialog.clone();
            move |this, _event, _window, cx| {
                remove_host(this, cx, &dialog);
            }
        }),
    ))
}

/// One row of the three-dots menu: an icon + label, `danger` for Remove.
fn menu_item(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    label: &'static str,
    danger: bool,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let color = if danger { theme.danger } else { theme.title };
    let hover = theme.hover;
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .px(px(10.))
        .py(px(8.))
        .rounded(px(theme.corner_radius_small))
        .cursor_pointer()
        .text_color(color)
        .child(crate::icon(icon_name).w(px(14.)).h(px(14.)).text_color(color))
        .child(
            text::R14P
                .style(div())
                .text_color(color)
                .child(SharedString::from(label)),
        )
        .hover(move |style| style.bg(hover))
        .on_click(listener)
}

// ---------------------------------------------------------------------------
// Field cards
// ---------------------------------------------------------------------------

/// A `FormCard` (`padding: 15px 20px 20px`) with its `titleSection` heading.
fn form_card(theme: TermiusTheme, title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .w_full()
        .pt(px(15.))
        .px(px(20.))
        .pb(px(20.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.card_a)
        .text_color(theme.title)
        .child(
            div()
                .font_family(crate::assets::UI_FONT)
                .text_size(px(14.))
                .font_weight(FontWeight(500.0))
                .line_height(px(21.))
                .child(SharedString::from(title)),
        )
}

/// A label-left / value-right select row (`fN` / `FiltersButton`). The original
/// opens a menu; gpui 0.2.2 has no popup primitive, so the row cycles its
/// options on click while still showing the live value.
fn dropdown_row(
    theme: TermiusTheme,
    id: &'static str,
    label: &'static str,
    value: SharedString,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .w_full()
        .cursor_pointer()
        .child(
            div()
                .font_family(crate::assets::UI_FONT)
                .text_size(px(14.))
                .line_height(px(21.))
                .text_color(theme.text_common)
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    text::R14P
                        .style(div())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.title)
                        .child(value),
                )
                .child(
                    crate::icon("allSettingsChevron.svg")
                        .w(px(15.))
                        .h(px(6.))
                        .text_color(theme.muted),
                ),
        )
        .on_click(listener)
}

/// The Group picker (`EntityAutocomplete`, `placeholder "Parent Group"`):
/// cycles `None` + the library's groups. PORT-TODO: no create-new-group.
fn group_picker(
    theme: TermiusTheme,
    state: &TermiusState,
    draft: &HostDraft,
    cx: &mut Context<TermiusState>,
) -> Stateful<Div> {
    let mut options: Vec<Option<String>> = vec![None];
    let mut groups: Vec<_> = state.library.groups.iter().collect();
    groups.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.title.cmp(&b.title))
    });
    for group in &groups {
        options.push(Some(group.id.clone()));
    }
    let label = match draft.group_id.as_deref() {
        Some(id) => groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| group.title.clone())
            .unwrap_or_else(|| "None".to_owned()),
        None => "None".to_owned(),
    };
    let count = options.len();

    dropdown_row(
        theme,
        "host-group",
        "Parent Group",
        SharedString::from(label),
        cx.listener(move |_this, _event, _window, cx| {
            let mut draft = draft_of(cx);
            let current = options
                .iter()
                .position(|option| option.as_deref() == draft.group_id.as_deref())
                .unwrap_or(0);
            let next = (current + 1) % count;
            draft.group_id = options.get(next).cloned().flatten();
            cx.set_global(draft);
            cx.notify();
        }),
    )
}

/// The Identity picker (the SSH/Telnet config group's bound login user).
fn identity_picker(
    theme: TermiusTheme,
    state: &TermiusState,
    draft: &HostDraft,
    cx: &mut Context<TermiusState>,
) -> Stateful<Div> {
    let mut options: Vec<Option<String>> = vec![None];
    let mut identities: Vec<_> = state.library.identities.iter().collect();
    identities.sort_by(|a, b| a.name.cmp(&b.name));
    for identity in &identities {
        options.push(Some(identity.id.clone()));
    }
    let label = match draft.identity_id.as_deref() {
        Some(id) => identities
            .iter()
            .find(|identity| identity.id == id)
            .map(|identity| identity.name.clone())
            .unwrap_or_else(|| "None".to_owned()),
        None => "None".to_owned(),
    };
    let count = options.len();

    dropdown_row(
        theme,
        "host-identity",
        "Identity",
        SharedString::from(label),
        cx.listener(move |_this, _event, _window, cx| {
            let mut draft = draft_of(cx);
            let current = options
                .iter()
                .position(|option| option.as_deref() == draft.identity_id.as_deref())
                .unwrap_or(0);
            let next = (current + 1) % count;
            draft.identity_id = options.get(next).cloned().flatten();
            cx.set_global(draft);
            cx.notify();
        }),
    )
}

/// The full-width **Connect** button pinned under the form (`connectButton`).
fn connect_button(
    dialog: &Dialog,
    values: &FormValues,
    theme: TermiusTheme,
    cx: &mut Context<TermiusState>,
) -> Div {
    div()
        .flex()
        .items_center()
        .w_full()
        .p(px(20.))
        .bg(theme.card_b)
        .child(
            Button::new("Connect").primary().on_click(
                theme,
                cx.listener({
                    let dialog = dialog.clone();
                    let values = values.clone();
                    move |this, _event, _window, cx| {
                        connect_host(this, cx, &dialog, &values);
                    }
                }),
            )
            .w_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str, groups: &[&str]) -> Host {
        Host {
            id: id.into(),
            label: format!("host-{id}"),
            hostname: format!("{id}.test"),
            port: 2222,
            username: "deploy".into(),
            group_ids: groups.iter().map(|group| (*group).to_owned()).collect(),
            ..Host::default()
        }
    }

    #[test]
    fn resolves_only_host_dialogs() {
        let hosts = vec![host("h1", &["g1"])];
        assert!(matches!(
            host_form(&Dialog::AddHost, &hosts),
            Some(HostForm::Add)
        ));
        assert!(matches!(
            host_form(&Dialog::EditHost("h1".into()), &hosts),
            Some(HostForm::Edit(_))
        ));
        // A host that no longer exists falls back to the shell's body.
        assert!(host_form(&Dialog::EditHost("gone".into()), &hosts).is_none());
        assert!(host_form(&Dialog::AddSnippet, &hosts).is_none());
        assert!(host_form(
            &Dialog::Confirm {
                title: "Delete".into(),
                message: "Sure?".into()
            },
            &hosts
        )
        .is_none());
    }

    #[test]
    fn form_values_prefill_or_default_to_22() {
        assert_eq!(
            form_values(None),
            FormValues {
                label: String::new(),
                hostname: String::new(),
                port: "22".into(),
                username: String::new(),
            }
        );
        let values = form_values(Some(&host("h1", &[])));
        assert_eq!(values.label, "host-h1");
        assert_eq!(values.hostname, "h1.test");
        assert_eq!(values.port, "2222");
        assert_eq!(values.username, "deploy");
    }

    #[test]
    fn ports_fall_back_to_22() {
        assert_eq!(parse_port("22"), 22);
        assert_eq!(parse_port(" 2200 "), 2200);
        assert_eq!(parse_port("65535"), 65535);
        assert_eq!(parse_port("0"), 22);
        assert_eq!(parse_port("65536"), 22);
        assert_eq!(parse_port("ssh"), 22);
    }

    #[test]
    fn host_type_labels_match_the_config_card() {
        assert_eq!(host_type_label(HostType::Ssh), "SSH");
        assert_eq!(host_type_label(HostType::Telnet), "Telnet");
        assert_eq!(host_type_label(HostType::Local), "Local");
    }

    #[test]
    fn draft_toggles_group_and_identity_picks() {
        let mut draft = HostDraft::new(&Dialog::AddHost, None);
        assert_eq!(draft, HostDraft::default());

        draft.toggle_group("g1");
        assert_eq!(draft.group_id.as_deref(), Some("g1"));
        draft.toggle_group("g1");
        assert_eq!(draft.group_id, None);

        draft.toggle_identity("i1");
        assert_eq!(draft.identity_id.as_deref(), Some("i1"));
        draft.toggle_identity("i1");
        assert_eq!(draft.identity_id, None);
    }

    #[test]
    fn untouched_picks_preserve_memberships_changed_ones_move() {
        let mut base = host("h1", &["g1", "g2"]);
        base.identity_id = Some("i1".into());
        let draft = HostDraft::new(&Dialog::EditHost("h1".into()), Some(&base));

        let mut saved = base.clone();
        draft.apply(&mut saved);
        assert_eq!(saved.group_ids, vec!["g1".to_owned(), "g2".to_owned()]);
        assert_eq!(saved.identity_id.as_deref(), Some("i1"));

        let mut moved = draft.clone();
        moved.toggle_group("g3");
        let mut saved = base.clone();
        moved.apply(&mut saved);
        assert_eq!(saved.group_ids, vec!["g3".to_owned()]);

        let mut cleared = draft.clone();
        cleared.toggle_identity("i1");
        let mut saved = base.clone();
        cleared.apply(&mut saved);
        assert_eq!(saved.identity_id, None);
    }

    #[test]
    fn apply_writes_the_chosen_host_type() {
        let mut draft = HostDraft::new(&Dialog::AddHost, None);
        draft.host_type = HostType::Telnet;
        let mut target = Host::default();
        draft.apply(&mut target);
        assert_eq!(target.host_type, HostType::Telnet);
    }

    #[test]
    fn build_host_updates_in_place_or_creates_validatable_default() {
        let existing = host("h1", &["g1"]);
        let values = FormValues {
            label: "web".into(),
            hostname: "web.test".into(),
            port: "2200".into(),
            username: "root".into(),
        };
        let draft = HostDraft::new(&Dialog::EditHost("h1".into()), Some(&existing));
        let built = build_host(Some(&existing), &values, 2200, &draft);
        assert_eq!(built.id, "h1");
        assert_eq!(built.label, "web");
        assert_eq!(built.port, 2200);
        assert_eq!(built.username, "root");

        // A blank Add fails validation instead of minting a junk record.
        let draft = HostDraft::new(&Dialog::AddHost, None);
        let blank = build_host(None, &form_values(None), parse_port("22"), &draft);
        assert_eq!(blank.port, 22);
        assert!(blank.validate().is_err());
    }
}
