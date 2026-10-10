//! snippets_screen — the Snippets section: the reusable-command list plus the
//! Add/Edit dialog bodies.
//!
//! Port of Termius' Snippets screen (`ui-process-a9c01aa6.js` — the
//! `isSnippetGenerationError` / `New snippet` toolbar around
//! `analysis/readable/_main.js:86191`, and the entity row presenter
//! `Cw` at `_main.js:69838`, which renders the 14px title over a 12px
//! `deprecatedSecondary` description beside a real entity icon).
//!
//! The screen matches the original's entity-list chrome:
//!
//! * a **screen header** — the `Snippets` title beside the primary
//!   `New snippet` action (`addCircle.svg` + label on the accent fill, the
//!   original `useB0` toolbar button at `_main.js:86453`).
//! * a **search/filter row** — the display-only search field plus the
//!   tag-filter / sort glyph buttons (`HostsFiltersHeader-fc79316f.js`:
//!   `height: 45px`, `background: var(--surface-high)`).
//! * the **list** — one row per [`Snippet`] with a leading entity icon
//!   (`snippet.svg`), the title (`R14P`) over the first body line (`R12S`),
//!   and a trailing `dots.svg` overflow affordance.
//!
//! Clicking a row opens the edit dialog; the trailing ⋯ is the row's context
//! menu in the original, wired here to Delete (the only item the port has).
//!
//! # Contract with the shell (coordinator wiring)
//!
//! * [`snippets_screen`] renders the section content (give it a sized parent).
//! * [`snippet_dialog_body`] returns the complete [`DialogFrame`] **card** for
//!   [`Dialog::AddSnippet`] / [`Dialog::EditSnippet`] — the shell paints it
//!   centered inside its scrim (`None` = this dialog isn't a snippet dialog,
//!   fall back to the stub).
//!
//! # PORT-TODO (text capture)
//!
//! [`InputField`] is display-only (see `primitives`), so the dialog cannot
//! capture typed text yet. The form is fully drawn and pre-filled, and every
//! action is wired to a concrete state change: Cancel is exact, Save persists
//! what the form shows — for `AddSnippet` a placeholder snippet titled
//! "Untitled snippet" (appended last), for `EditSnippet` the existing record
//! as displayed. Swap the Save handler for real field values once
//! `InputField` grows a focusable content model. The port has no snippet
//! *packages* (the original's `Add a Package` selector) nor the AI shell-assist
//! affordance, so the form shows Label + Script only.
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
    div, px, AnyElement, App, AppContext as _, ClickEvent, Context, Div, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use termius_core::Snippet;

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{Button, DialogFrame, EmptyState, InputField, SettingsText};
use crate::theme::{over, text, theme_of, with_alpha, TermiusTheme, ThemeMode, UI_FONT};

/// Corner radius for the delete affordance (matches the primitives).
const CORNER: f32 = 4.0;
/// Preview-box minimum height in the dialog (a few command lines).
const BODY_MIN_HEIGHT: f32 = 84.0;
/// Title used for snippets (and the Add dialog placeholder) with no name.
const UNTITLED: &str = "Untitled snippet";

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
/// The snippet entity icon (`Sb`/`snippet` presenter icon).
const SNIPPET_ICON: &str = "snippet.svg";

// ---------------------------------------------------------------------------
// Shared list chrome (local; the primitives have no icon row)
// ---------------------------------------------------------------------------

/// The primary header action: `addCircle.svg` + label on the accent fill.
///
/// `Button` (the primitive) carries no icon slot, so this mirrors its metrics
/// (`height: 36px`, `padding: 0 16px`, `--corner-radius-medium`, 14/500, white
/// label) around an [`crate::icon`] instead.
fn new_button(
    label: &'static str,
    theme: TermiusTheme,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    // Termius' button hover "substrate": `--white` faded to `.25` over the fill.
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
///
/// PORT-TODO: `TermiusState` has no search field and gpui 0.2.2 text editing
/// needs a `Content`/`InputEvent` model on a tracked focus handle (the same gap
/// `views::host_list` notes).
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
        .child(search_field(theme, "Search snippets"))
        .child(icon_button(theme, "tags.svg", "snippet-filter-tags"))
        .child(icon_button(theme, "sorting.svg", "snippet-sort"))
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

/// The Snippets section view: header ("Snippets" + "New snippet") over a
/// search/filter row and a scrollable column of entity rows, or an
/// [`EmptyState`] when the library holds none.
///
/// Clicking a row opens [`Dialog::EditSnippet`]; the trailing ⋯ deletes it;
/// "New snippet" opens [`Dialog::AddSnippet`].
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

    /// Delete one snippet (the row's ⋯).
    fn delete_snippet(&mut self, snippet_id: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.delete_snippet(snippet_id, cx));
    }
}

impl Render for SnippetsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Read the slice once: rows (snippets in Termius order) plus which row
        // the open Edit dialog targets. Everything is cloned out so the borrow
        // ends before the listeners below register against `cx`.
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

        // Header: screen title + the primary action (opens the Add dialog).
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
                    .child(SharedString::from("Snippets")),
            )
            .child(new_button(
                "New snippet",
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
                text::R12S
                    .style(div())
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

                // The whole row carries the selected/hover wash; the content
                // and the ⋯ are siblings so the ⋯ click never also opens the
                // dialog (gpui bubbles clicks through ancestors).
                let wrapper = div()
                    .id(SharedString::from(format!("snippet-row-{}", row.id)))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(ROW_HEIGHT))
                    .px(px(12.))
                    .rounded(px(theme.corner_radius_small));
                let wrapper = if row.selected {
                    wrapper
                        .bg(theme.card_c)
                        .hover(move |hover| hover.bg(over(theme.card_c, theme.hover)))
                } else {
                    wrapper.hover(move |hover| hover.bg(theme.hover))
                };

                let content = div()
                    .id(SharedString::from(format!("snippet-open-{open_id}")))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .flex_1()
                    .min_w(px(0.))
                    .child(icon_tile(theme, SNIPPET_ICON))
                    .child(row_column(theme, row.title, row.subtitle))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.open_dialog(Dialog::EditSnippet(open_id.clone()), cx);
                    }));

                let dots = dots_button(
                    format!("snippet-menu-{delete_id}"),
                    theme,
                    cx.listener(move |this, _event, _window, cx| {
                        this.delete_snippet(&delete_id, cx);
                    }),
                );

                list = list.child(wrapper.child(content).child(dots));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.foreground)
            .child(header)
            .child(filter_row(theme))
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
/// [`Dialog::title`], the original `Action description` + `Script` fields and
/// help text, then Cancel / Save. Save mutates the state and closes the dialog.
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

/// The Add/Edit form card. `existing` pre-fills the fields.
fn snippet_form(
    dialog: &Dialog,
    existing: Option<&Snippet>,
    theme: TermiusTheme,
    cx: &mut Context<TermiusState>,
) -> gpui::Div {
    let title = existing.map(|snippet| snippet.title.clone()).unwrap_or_default();
    let body = existing.map(|snippet| snippet.body.clone()).unwrap_or_default();
    let save_target = existing.map(|snippet| snippet.id.clone());

    let frame = DialogFrame::new(dialog.title())
        // Original field: `label` — "Action description"
        // (`_main.js:84456`), 50-char limit, placeholder "Example: check
        // network load".
        .child(
            InputField::new("Action description", title)
                .placeholder("Example: check network load")
                .element(theme),
        )
        .child(body_field(theme, &body))
        .child(
            SettingsText::new(
                "The snippet text is inserted as-is when you run it from the Snippets list.",
            )
            .element(theme),
        )
        .action(Button::new("Cancel").secondary().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| {
                this.close_dialog(cx);
            }),
        ))
        // PORT-TODO: persist the real field values once `InputField` can
        // capture typed text; today Save round-trips what the form displays.
        .action(Button::new("Save").primary().on_click(
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

/// The "Script" labeled multi-line body box: one line per body line, a muted
/// placeholder when the body is empty (the original `script` textarea,
/// `minRows: 6`).
fn body_field(theme: TermiusTheme, body: &str) -> gpui::Div {
    let mut field = div().flex().flex_col().gap(px(4.));
    field = field.child(
        div()
            .font_family(UI_FONT)
            .text_size(px(14.))
            .text_color(theme.text_common)
            .child(SharedString::from("Script")),
    );

    let mut box_el = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .min_h(px(BODY_MIN_HEIGHT))
        .p(px(8.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(theme.border_basic)
        .bg(theme.card_c);

    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        box_el = box_el.child(SettingsText::new("Enter a command or script…").element(theme));
    } else {
        for line in lines {
            box_el = box_el.child(
                div()
                    .font_family(UI_FONT)
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .whitespace_normal()
                    .text_color(theme.title)
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

    #[test]
    fn snippet_row_icon_is_bundled() {
        assert!(crate::has_icon(SNIPPET_ICON));
        assert!(crate::has_icon("addCircle.svg"));
        assert!(crate::has_icon("dots.svg"));
    }
}
