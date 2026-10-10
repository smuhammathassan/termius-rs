//! host_dialog — the Add Host / Edit Host form body.
//!
//! Termius' host editor is a modal (Label, Hostname, Port, Username, group
//! and identity pickers + Save/Cancel) hosted in a
//! [`DialogFrame`](crate::primitives::DialogFrame) by the app shell. This
//! module builds that body for [`Dialog::AddHost`] / [`Dialog::EditHost`];
//! every other dialog returns `None`, so the shell keeps its generic
//! fallback body.
//!
//! # Wire contract
//!
//! A `&mut Context<TermiusState>` only exists inside a `TermiusState`
//! update (gpui leases one entity at a time), so the shell calls this from
//! there — the same borrow also provides the `&TermiusState`:
//!
//! ```ignore
//! let body = self
//!     .state
//!     .update(cx, |state, cx| host_dialog_body(dialog, state, cx));
//! ```
//!
//! Click listeners created here run later (no lease is held by then) and
//! mutate the `TermiusState` they receive — [`TermiusState::add_host`],
//! [`TermiusState::update_host`], [`TermiusState::close_dialog`] — exactly
//! like `views::host_list` does.
//!
//! # PORT-TODO: text editing
//!
//! [`InputField`] is display-only, so the form shows the pre-filled values
//! (Edit) or the empty defaults (Add; Port is `"22"`) and **Save re-builds
//! a `Host` from exactly those displayed values** — label/hostname cannot
//! be typed yet. Until real editing lands (`Content` + `InputEvent` on a
//! tracked `FocusHandle`), Add Host is gated by [`Host::validate`]: Save
//! surfaces the validation error in the status bar and keeps the dialog
//! open rather than minting an unlabeled host.
//!
//! The group/identity pickers *are* interactive: clicks stash the pending
//! pick in the [`HostDraft`] gpui global (re-seeded whenever a different
//! dialog opens), and Save folds it into the record — an untouched pick
//! never rewrites memberships, a changed one moves the host the way
//! Termius' single-group picker does.

use gpui::{div, px, AnyElement, Context, Global, IntoElement, ParentElement as _, Styled as _};
use termius_core::Host;

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{Button, InputField, ListItem, SettingsSection, SettingsText};
use crate::theme::theme_of;

/// Port shown (and used) when a host has none — Termius' default SSH port.
const DEFAULT_PORT: u16 = 22;

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

/// The four text fields as displayed (Edit: pre-filled; Add: empty/"22").
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

/// The pending group/identity selection for the open host dialog.
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
        // survive a plain Save); a changed one moves the host outright.
        if self.group_id != self.initial_group {
            host.group_ids = match self.group_id.clone() {
                Some(group_id) => vec![group_id],
                None => Vec::new(),
            };
        }
        host.identity_id = self.identity_id.clone();
    }
}

/// Build the record Save writes: start from the live host (Edit) or a
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

// ---------------------------------------------------------------------------
// The dialog body
// ---------------------------------------------------------------------------

/// The Add/Edit Host form body for [`Dialog::AddHost`] / [`Dialog::EditHost`].
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
    // pending picks across re-renders of the same dialog.
    let draft_stale = cx
        .try_global::<HostDraft>()
        .map(|draft| draft.dialog != *dialog)
        .unwrap_or(true);
    if draft_stale {
        cx.set_global(HostDraft::new(dialog, existing));
    }
    let draft = cx
        .try_global::<HostDraft>()
        .cloned()
        .unwrap_or_else(|| HostDraft::new(dialog, existing));

    let mut form_column = div().flex().flex_col().gap(px(10.));
    form_column = form_column.child(
        InputField::new("Label", values.label.clone())
            .placeholder("e.g. web-1")
            .element(theme),
    );
    form_column = form_column.child(
        InputField::new("Hostname", values.hostname.clone())
            .placeholder("example.com or 10.0.0.1")
            .element(theme),
    );
    form_column = form_column.child(InputField::new("Port", values.port.clone()).element(theme));
    form_column = form_column.child(
        InputField::new("Username", values.username.clone())
            .placeholder("root")
            .element(theme),
    );

    // ----- group picker ----------------------------------------------------
    let mut groups: Vec<_> = state.library.groups.iter().collect();
    groups.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.title.cmp(&b.title))
    });
    let mut group_section = SettingsSection::new("Group");
    if groups.is_empty() {
        group_section = group_section.child(
            SettingsText::new("No groups yet — create one from the host tree.").element(theme),
        );
    } else {
        for group in groups {
            let selected = draft.group_id.as_deref() == Some(group.id.as_str());
            let group_id = group.id.clone();
            group_section = group_section.child(
                ListItem::new(group.title.clone(), String::new())
                    .id(format!("host-dialog-group-{}", group.id))
                    .selected(selected)
                    .on_click(theme, cx.listener(move |_this, _event, _window, cx| {
                        let mut draft = cx
                            .try_global::<HostDraft>()
                            .cloned()
                            .unwrap_or_else(HostDraft::default);
                        draft.toggle_group(&group_id);
                        cx.set_global(draft);
                        cx.notify();
                    })),
            );
        }
    }
    form_column = form_column.child(group_section.element(theme));

    // ----- identity picker -------------------------------------------------
    let mut identities: Vec<_> = state.library.identities.iter().collect();
    identities.sort_by(|a, b| a.name.cmp(&b.name));
    let mut identity_section = SettingsSection::new("Identity");
    if identities.is_empty() {
        identity_section = identity_section.child(
            SettingsText::new("No identities yet — bind one to pre-fill its credentials.")
                .element(theme),
        );
    } else {
        for identity in identities {
            let selected = draft.identity_id.as_deref() == Some(identity.id.as_str());
            let identity_id = identity.id.clone();
            let subtitle = if identity.username.is_empty() {
                String::new()
            } else {
                identity.username.clone()
            };
            identity_section = identity_section.child(
                ListItem::new(identity.name.clone(), subtitle)
                    .id(format!("host-dialog-identity-{}", identity.id))
                    .selected(selected)
                    .on_click(theme, cx.listener(move |_this, _event, _window, cx| {
                        let mut draft = cx
                            .try_global::<HostDraft>()
                            .cloned()
                            .unwrap_or_else(HostDraft::default);
                        draft.toggle_identity(&identity_id);
                        cx.set_global(draft);
                        cx.notify();
                    })),
            );
        }
    }
    form_column = form_column.child(identity_section.element(theme));

    // ----- actions ---------------------------------------------------------
    let cancel = Button::new("Cancel").secondary().on_click(
        theme,
        cx.listener(|this, _event, _window, cx| {
            cx.set_global(HostDraft::default());
            this.close_dialog(cx);
        }),
    );
    let save = Button::new("Save").primary().on_click(
        theme,
        cx.listener({
            let dialog = dialog.clone();
            let values = values.clone();
            move |this, _event, _window, cx| {
                let draft = cx
                    .try_global::<HostDraft>()
                    .cloned()
                    .unwrap_or_else(|| HostDraft::new(&dialog, None));
                // The edit target vanished while the dialog was open.
                if let Dialog::EditHost(id) = &dialog {
                    if this.library.host(id.as_str()).is_none() {
                        this.status_text = format!("Edit Host: `{id}` no longer exists");
                        cx.set_global(HostDraft::default());
                        this.close_dialog(cx);
                        return;
                    }
                }
                let existing = match &dialog {
                    Dialog::EditHost(id) => this.library.host(id.as_str()).cloned(),
                    _ => None,
                };
                let host = build_host(existing.as_ref(), &values, parse_port(&values.port), &draft);
                match host.validate() {
                    Ok(()) => {
                        if is_edit {
                            this.update_host(host, cx);
                        } else {
                            this.add_host(host, cx);
                        }
                        cx.set_global(HostDraft::default());
                        this.close_dialog(cx);
                    }
                    Err(err) => {
                        // Display-only fields: surface why Save was refused
                        // and keep the dialog open (Termius disables Save
                        // until the required fields are filled).
                        this.status_text = format!("{}: {err}", dialog.title());
                        cx.notify();
                    }
                }
            }
        }),
    );
    let mut actions = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .pt(px(4.));
    actions = actions.child(cancel).child(save);

    Some(form_column.child(actions).into_any_element())
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
