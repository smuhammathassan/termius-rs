//! snippets_screen — the Snippets section: the reusable-command list plus the
//! Add/Edit dialog bodies.
//!
//! Port of Termius' Snippets screen: a header with the "New Snippet" action,
//! one row per [`Snippet`] (title + first body line / bound command), click a
//! row to open the edit dialog, the ✕ affordance on the right of a row deletes
//! it, and an [`EmptyState`] replaces the list while the library has no
//! snippets yet.
//!
//! # Contract with the shell (coordinator wiring)
//!
//! * [`snippets_screen`] renders the section content (give it a sized parent).
//! * [`snippet_dialog_body`] returns the complete [`DialogFrame`] **card** for
//!   [`Dialog::AddSnippet`] / [`Dialog::EditSnippet`] — the shell should paint
//!   it centered inside its scrim instead of its stub frame (`None` = this
//!   dialog isn't a snippet dialog, fall back to the stub).
//!
//! # PORT-TODO (text capture)
//!
//! [`InputField`] is display-only (see `primitives`), so the dialog cannot
//! capture typed text yet. The form is fully drawn and pre-filled, and every
//! action is wired to a concrete state change: Delete/Cancel are exact, Save
//! persists what the form shows — for `AddSnippet` a placeholder snippet
//! titled "Untitled snippet" (appended last), for `EditSnippet` the existing
//! record as displayed. Swap the Save handler for real field values once
//! `InputField` grows a focusable content model.
//!
//! # gpui 0.2 note (`Context<TermiusState>` + `Entity::read`)
//!
//! gpui hands out `Context<TermiusState>` only while the state entity is
//! leased for update (or being built), and reading a leased entity panics
//! ("cannot read … while it is already being updated" —
//! `EntityMap::read` → `double_lease_panic`). So neither factory may read
//! `TermiusState` through the handle:
//! [`snippet_dialog_body`] is safe to call inside
//! `state.update(cx, |state, cx| …)` (it takes `&TermiusState` directly),
//! and [`snippets_screen`] only mints a [`SnippetsScreen`] view entity —
//! the state read happens in that view's `render`, long after the lease was
//! released (the `views::host_list` / `views::settings_screen` pattern).

use gpui::{
    div, px, AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};
use termius_core::Snippet;

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{
    Button, DialogFrame, EmptyState, InputField, ListItem, SectionHeader, SettingsText,
};
use crate::theme::{theme_of, TermiusTheme};

/// Corner radius for the delete affordance (matches the primitives).
const CORNER: f32 = 4.0;
/// Delete "✕" hit target.
const DELETE_SIZE: f32 = 24.0;
/// Preview-box minimum height in the dialog (a few command lines).
const BODY_MIN_HEIGHT: f32 = 84.0;
/// Title used for snippets (and the Add dialog placeholder) with no name.
const UNTITLED: &str = "Untitled snippet";

// ---------------------------------------------------------------------------
// Pure row helpers (unit-tested below)
// ---------------------------------------------------------------------------

/// The list subtitle: the snippet's first non-empty body line, falling back to
/// its bound command name ([`Snippet::command`]) when the body has none.
fn snippet_subtitle(snippet: &Snippet) -> String {
    if let Some(line) = snippet
        .body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    snippet
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// The list label: the snippet's title, or [`UNTITLED`] when it has none.
fn snippet_title(snippet: &Snippet) -> String {
    let title = snippet.title.trim();
    if title.is_empty() {
        UNTITLED.to_owned()
    } else {
        title.to_owned()
    }
}

/// One rendered snippet row (owned, so the list can be built after the state
/// borrow ends).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnippetRow {
    id: String,
    title: String,
    subtitle: String,
    /// True while the open dialog edits this snippet.
    selected: bool,
}

impl SnippetRow {
    fn of(snippet: &Snippet, active_dialog: Option<&Dialog>) -> Self {
        let selected =
            matches!(active_dialog, Some(Dialog::EditSnippet(id)) if *id == snippet.id);
        Self {
            id: snippet.id.clone(),
            title: snippet_title(snippet),
            subtitle: snippet_subtitle(snippet),
            selected,
        }
    }
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Snippets section view: header ("Snippets" + "New Snippet") over a
/// scrollable column of [`ListItem`]s, one per snippet, or an [`EmptyState`]
/// when the library holds none.
///
/// Clicking a row opens [`Dialog::EditSnippet`]; the ✕ on the right of a row
/// deletes it immediately; "New Snippet" opens [`Dialog::AddSnippet`].
///
/// A separate entity from [`TermiusState`] (gpui leases one entity at a
/// time): the factory below only mints it, and [`Render::render`] reads the
/// state from a context that no longer holds the lease.
pub struct SnippetsScreen {
    state: Entity<TermiusState>,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Snippets screen for `state`.
///
/// Safe to call while `TermiusState` is leased (inside `state.update(..)`):
/// it only mints a view entity, every state read happens later in
/// [`SnippetsScreen`]'s own render. Store the returned entity once (the same
/// way `AppShell` holds `HostList`) — calling this every render would mint a
/// new entity per frame.
pub fn snippets_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<SnippetsScreen> {
    cx.new(|cx| SnippetsScreen::new(state, cx))
}

impl SnippetsScreen {
    /// Create the screen and subscribe it to state + theme changes.
    fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let observe_theme = cx.observe_global::<TermiusTheme>(|_, cx| cx.notify());
        Self { state, _observe_state: observe_state, _observe_theme: observe_theme }
    }

    /// Raise a dialog (the shell hosts it above everything).
    fn open_dialog(&mut self, dialog: Dialog, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.open_dialog(dialog, cx));
    }

    /// Delete one snippet (the row's ✕).
    fn delete_snippet(&mut self, snippet_id: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.delete_snippet(snippet_id, cx));
    }
}

impl Render for SnippetsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Read the slice once: rows (snippets in Termius order) plus which row the
        // open Edit dialog targets. Everything is cloned out so the borrow ends
        // before the listeners below register against `cx`.
        let (rows, loading) = {
            let state = self.state.read(cx);
            let mut snippets: Vec<&Snippet> = state.library.snippets.iter().collect();
            snippets.sort_by(|a, b| {
                a.sort_order
                    .cmp(&b.sort_order)
                    .then_with(|| a.title.cmp(&b.title))
            });
            let rows: Vec<SnippetRow> = snippets
                .into_iter()
                .map(|snippet| SnippetRow::of(snippet, state.active_dialog.as_ref()))
                .collect();
            (rows, state.library_loading)
        };
        let empty = rows.is_empty();

        // Header: section title + the primary action (opens the Add dialog).
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .pr(px(8.))
            .child(SectionHeader::new("Snippets").element(theme).flex_1())
            .child(Button::new("New Snippet").primary().on_click(
                theme,
                cx.listener(|this, _event, _window, cx| {
                    this.open_dialog(Dialog::AddSnippet, cx);
                }),
            ));

        // Scrollable list column (wheel scrolling comes free with
        // `overflow_y_scroll` in gpui 0.2; a ScrollHandle is only needed for
        // programmatic scrolling).
        let mut list = div()
            .id("snippets-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .gap(px(2.))
            .px(px(8.))
            .py(px(4.));

        if empty && loading {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_color(theme.muted)
                    .child(SharedString::from("Loading snippets…")),
            );
        } else if empty {
            list = list.child(
                EmptyState::new("No snippets", "Create reusable commands…").element(theme),
            );
        } else {
            for row in rows {
                let open_id = row.id.clone();
                let delete_id = row.id.clone();

                // The row itself: label + subtitle, click → edit dialog. `flex_1`
                // keeps it clear of the delete button beside it.
                let item = ListItem::new(row.title.clone(), row.subtitle.clone())
                    .selected(row.selected)
                    .id(SharedString::from(format!("snippet-row-{}", row.id)))
                    .element(theme)
                    .flex_1()
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.open_dialog(Dialog::EditSnippet(open_id.clone()), cx);
                    }));

                // Small delete affordance beside the row (a sibling, not a child,
                // so its click never also opens the edit dialog).
                let delete = div()
                    .id(SharedString::from(format!("snippet-delete-{delete_id}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(DELETE_SIZE))
                    .h(px(DELETE_SIZE))
                    .rounded(px(CORNER))
                    .text_color(theme.danger)
                    .child(SharedString::from("✕"))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.delete_snippet(&delete_id, cx);
                    }));

                list = list.child(div().flex().items_center().gap(px(4.)).child(item).child(delete));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.foreground)
            .child(header)
            .child(list)
    }
}

// ---------------------------------------------------------------------------
// Dialog body (Add / Edit snippet)
// ---------------------------------------------------------------------------

/// The complete [`DialogFrame`] card for the snippet dialogs, or `None` when
/// `dialog` isn't a snippet dialog (or targets a snippet that no longer
/// exists).
///
/// The card is `DialogFrame`-shaped end to end: title bar from
/// [`Dialog::title`], body fields ([`InputField`] title + command, a
/// multi-line display of the snippet body, help text) and the Delete / Cancel /
/// Save actions. Save and Delete mutate the state and close the dialog.
pub fn snippet_dialog_body(
    dialog: &Dialog,
    state: &TermiusState,
    cx: &mut Context<TermiusState>,
) -> Option<AnyElement> {
    let theme = theme_of(cx);
    match dialog {
        Dialog::AddSnippet => Some(snippet_form(dialog, None, theme, cx).into_any_element()),
        Dialog::EditSnippet(id) => {
            // A vanished snippet (deleted while the dialog was open) has
            // nothing to edit: `None` lets the shell fall back to its stub.
            let existing = state
                .library
                .snippets
                .iter()
                .find(|snippet| snippet.id == *id)?;
            Some(snippet_form(dialog, Some(existing), theme, cx).into_any_element())
        }
        _ => None,
    }
}

/// The Add/Edit form card. `existing` pre-fills the fields (edit mode also
/// gains the Delete action).
fn snippet_form(
    dialog: &Dialog,
    existing: Option<&Snippet>,
    theme: TermiusTheme,
    cx: &mut Context<TermiusState>,
) -> gpui::Div {
    let title = existing.map(|snippet| snippet.title.clone()).unwrap_or_default();
    let command = existing
        .and_then(|snippet| snippet.command.clone())
        .unwrap_or_default();
    let body = existing.map(|snippet| snippet.body.clone()).unwrap_or_default();
    let delete_target = existing.map(|snippet| snippet.id.clone());
    let save_target = delete_target.clone();

    let mut frame = DialogFrame::new(dialog.title())
        .child(InputField::new("Title", title).placeholder("Snippet name").element(theme))
        .child(
            InputField::new("Command", command)
                .placeholder("Optional command name")
                .element(theme),
        )
        .child(body_field(theme, &body))
        .child(
            SettingsText::new(
                "The snippet text is inserted as-is when you run it from the Snippets list.",
            )
            .element(theme),
        );

    frame = frame.action(Button::new("Cancel").secondary().on_click(
        theme,
        cx.listener(|this, _event, _window, cx| {
            this.close_dialog(cx);
        }),
    ));

    if let Some(delete_target) = delete_target {
        frame = frame.action(Button::new("Delete").danger().on_click(
            theme,
            cx.listener(move |this, _event, _window, cx| {
                this.delete_snippet(&delete_target, cx);
                this.close_dialog(cx);
            }),
        ));
    }

    // PORT-TODO: persist the real field values once `InputField` can capture
    // typed text; today Save round-trips what the form displays.
    frame = frame.action(Button::new("Save").primary().on_click(
        theme,
        cx.listener(move |this, _event, _window, cx| {
            match save_target.as_deref() {
                Some(id) => {
                    if let Some(existing) = this
                        .library
                        .snippets
                        .iter()
                        .find(|snippet| snippet.id == id)
                        .cloned()
                    {
                        this.update_snippet(existing, cx);
                    }
                }
                None => {
                    let fresh = Snippet {
                        title: UNTITLED.to_owned(),
                        sort_order: this.library.snippets.len() as i64,
                        ..Snippet::default()
                    };
                    this.add_snippet(fresh, cx);
                }
            }
            this.close_dialog(cx);
        }),
    ));

    frame.element(theme)
}

/// The "Snippet" labeled multi-line body box: one muted line per body line, a
/// muted placeholder when the body is empty.
fn body_field(theme: TermiusTheme, body: &str) -> gpui::Div {
    let mut field = div().flex().flex_col().gap(px(4.));
    field = field.child(div().text_xs().text_color(theme.muted).child(SharedString::from(
        "Snippet",
    )));

    let mut box_el = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .min_h(px(BODY_MIN_HEIGHT))
        .p(px(8.))
        .rounded(px(CORNER))
        .border_1()
        .border_color(theme.border)
        .bg(theme.tab_background);

    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        box_el = box_el.child(
            SettingsText::new("Enter a command or script…").element(theme),
        );
    } else {
        for line in lines {
            box_el = box_el.child(
                div()
                    .text_sm()
                    .whitespace_normal()
                    .text_color(theme.foreground)
                    .child(SharedString::from(line.to_owned())),
            );
        }
    }
    field.child(box_el)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(id: &str, title: &str, body: &str, command: Option<&str>) -> Snippet {
        Snippet {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            command: command.map(str::to_owned),
            ..Snippet::default()
        }
    }

    #[test]
    fn subtitle_is_the_first_non_empty_body_line() {
        assert_eq!(
            snippet_subtitle(&snippet("s", "Deploy", "\n  sudo uptime\nwhoami", None)),
            "sudo uptime"
        );
        assert_eq!(snippet_subtitle(&snippet("s", "D", "whoami", None)), "whoami");
    }

    #[test]
    fn subtitle_falls_back_to_the_command_name() {
        assert_eq!(
            snippet_subtitle(&snippet("s", "Deploy", "  \n\t", Some(" status "))),
            "status"
        );
        assert_eq!(snippet_subtitle(&snippet("s", "Deploy", "", Some(""))), "");
        assert_eq!(snippet_subtitle(&snippet("s", "Deploy", "", None)), "");
    }

    #[test]
    fn title_falls_back_to_untitled() {
        assert_eq!(snippet_title(&snippet("s", "  ", "uptime", None)), UNTITLED);
        assert_eq!(snippet_title(&snippet("s", "Deploy", "uptime", None)), "Deploy");
    }

    #[test]
    fn row_marks_the_snippet_the_dialog_edits() {
        let snippet = snippet("s1", "Deploy", "uptime", None);
        let editing = Some(Dialog::EditSnippet("s1".into()));
        assert!(SnippetRow::of(&snippet, editing.as_ref()).selected);
        let other = Some(Dialog::EditSnippet("s2".into()));
        assert!(!SnippetRow::of(&snippet, other.as_ref()).selected);
        assert!(!SnippetRow::of(&snippet, None).selected);
        // Add dialog selects nothing.
        assert!(!SnippetRow::of(&snippet, Some(&Dialog::AddSnippet)).selected);
    }

    #[test]
    fn row_carries_title_and_subtitle() {
        let row = SnippetRow::of(&snippet("s1", "Deploy", "uptime", None), None);
        assert_eq!(row.id, "s1");
        assert_eq!(row.title, "Deploy");
        assert_eq!(row.subtitle, "uptime");
    }
}
