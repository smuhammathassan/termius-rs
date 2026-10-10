//! snippets_screen — the Snippets section: the two-level **Packages /
//! Snippets** entity list plus the Add/Edit dialog body.
//!
//! Reconstruction of Termius v10's Snippets screen (`analysis/recon/20-snippets.md`):
//!
//! * **No title band.** The original mounts the shared [`FiltersHeader`] (the
//!   45px search/filter band) and puts the primary action in it as its *first
//!   child*; the screen title comes from the left-nav section tab
//!   (`_main.js:86446`). The port previously invented a 56px `B16P` "Snippets"
//!   band — removed.
//! * **Primary action = split button.** `New snippet` is the `useB0` split
//!   button: a contained main half carrying the thin plus glyph `SvgPlusThin`
//!   (14×14 — *not* `addCircle.svg`) plus a 30×30 chevron half whose menu holds
//!   `New snippet package` (`_main.js:86453`, `/tmp/reconnectSaga.js:107711`).
//!   A `Shell History` ghost button rides beside it (`_main.js:86469`).
//! * **Rows = [`EntityRow`].** The shared `GridItemPresenter` / `EntityItem`
//!   cell: a 40×40 [`ShapedIcon`](crate::primitives::ShapedIcon) tile, a 14px
//!   `--text-primary` title over an 11px `--text-secondary` subtitle,
//!   `padding:10px; border-radius:14px; border:2px solid
//!   var(--entity-item-background)`, hover `--list-hover-hover`
//!   (`/tmp/reconnectSaga.js:131264`).
//! * **Two lists.** `entitiesListsData()` builds a `Packages` list and a
//!   `Snippets` list (`_main.js:86165`). The port renders both; the presenter
//!   functions live below.
//! * **Form fields** (`cwe`, `_main.js:84371`): `Action description` (required,
//!   50 chars), `Package` (`Add a Package` select) and a multi-line monospace
//!   `Script` (`minRows:6`).
//!
//! # Honest gaps (PORT-TODO)
//!
//! * [`Snippet`] has no package field, so [`package_rows`] yields nothing yet —
//!   the `Packages` list renders its header and a note, and the presenter is
//!   kept so the two-level structure is real and ready.
//! * [`InputField`] is display-only (see `primitives`), so the dialog cannot
//!   capture typed text: the form is fully drawn and pre-filled, and Save
//!   persists what it shows. Swap the handler for real values once
//!   `InputField` grows a content model.
//! * gpui 0.2.2 has no anchored popup host, so the split-button menu renders as
//!   an inline panel under the [`FiltersHeader`] while open, and the row's hover
//!   ⋯ `ItemMenu` (Edit / Duplicate / Share / Remove) is not implemented — the
//!   Edit dialog carries a `Delete` action instead.
//!
//! # Contract with the shell (coordinator wiring)
//!
//! * [`snippets_screen`] renders the section content (give it a sized parent).
//! * [`snippet_dialog_body`] returns the complete [`DialogFrame`] **card** for
//!   [`Dialog::AddSnippet`] / [`Dialog::EditSnippet`] — the shell paints it
//!   centered inside its scrim (`None` = this dialog isn't a snippet dialog,
//!   fall back to the stub).
//!
//! # gpui 0.2 note (`Context<TermiusState>` + `Entity::read`)
//!
//! gpui hands out `Context<TermiusState>` only while the state entity is leased
//! for update (or being built), and reading a leased entity panics. So neither
//! factory may read `TermiusState` through the handle: [`snippet_dialog_body`]
//! takes `&TermiusState` directly (safe inside `state.update(..)`), and
//! [`snippets_screen`] only mints a [`SnippetsScreen`] view entity — the state
//! read happens in that view's `render`, long after the lease was released.

use gpui::{
    div, px, AnyElement, App, AppContext as _, ClickEvent, Context, Div, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Rgba, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use termius_core::Snippet;

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{
    Button, DialogFrame, EntityRow, FiltersHeader, InputField, SectionHeader, SettingsText,
};
use crate::theme::{over, theme_of, with_alpha, TermiusTheme, UI_FONT};

// --- icons (exact bundled asset names) -------------------------------------
/// The snippet entity icon (`entityIcons.snippet`).
const SNIPPET_ICON: &str = "snippet.svg";
/// The snippet-package entity icon (`entityIcons.snippets_package`).
const PACKAGE_ICON: &str = "snippets_package.svg";
/// The toolbar's thin plus (`SvgPlusThin`, 14×14 — *not* `addCircle.svg`).
const PLUS_ICON: &str = "plusThin.svg";
/// The `Shell History` ghost button glyph.
const HISTORY_ICON: &str = "history.svg";
/// The split button's menu chevron (`da_1` arrow).
const CHEVRON_ICON: &str = "chevron.svg";

// --- layout metrics (from the original CSS) --------------------------------
/// Termius' `medium` button height (the toolbar buttons live in the 45px band).
const BUTTON_HEIGHT: f32 = 30.0;
/// `--corner-radius-small-medium` (the medium button radius).
const BUTTON_RADIUS: f32 = 8.0;
/// The `Script` textarea's `minRows: 6` (6 × ~20px line + padding).
const SCRIPT_MIN_HEIGHT: f32 = 120.0;
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

/// `firstLineWithRegularCharacters(script)`: the first line that holds a real
/// command character — blank lines and `#`-comment lines fall through.
fn first_line_with_regular_characters(script: &str) -> Option<String> {
    script.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

/// `SnippetPresenter` (`gG`, `/tmp/reconnectSaga.js:74077`): label =
/// `entity.displayLabel`, description = the first regular script line (only
/// while a label exists), icon `snippet`.
fn snippet_presenter(snippet: &Snippet) -> RowPresenter {
    let label = snippet.title.trim().to_owned();
    let description = if label.is_empty() {
        String::new()
    } else {
        first_line_with_regular_characters(&snippet.body).unwrap_or_default()
    };
    RowPresenter { label, description, icon: SNIPPET_ICON }
}

/// One snippet-package row (see the module's PORT-TODO: [`Snippet`] has no
/// package field yet).
#[allow(dead_code)] // PORT-TODO: constructed once `Snippet` grows a package.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnippetPackage {
    label: String,
    snippet_count: usize,
}

/// `SnippetsPackagePresenter` (`mH`, `/tmp/reconnectSaga.js:74130`): label =
/// `entity.label`, description = `` `${count} snippet(s)` ``, icon
/// `snippets_package`.
#[allow(dead_code)] // PORT-TODO: driven by [`package_rows`] once packages exist.
fn package_presenter(package: &SnippetPackage) -> RowPresenter {
    RowPresenter {
        label: package.label.clone(),
        description: snippet_count_label(package.snippet_count),
        icon: PACKAGE_ICON,
    }
}

/// `` `${n} snippet(s)` ``.
fn snippet_count_label(count: usize) -> String {
    if count == 1 {
        "1 snippet".to_owned()
    } else {
        format!("{count} snippets")
    }
}

/// The `Packages` list for the two-level screen.
///
/// PORT-TODO: [`Snippet`] has no package field, so there is nothing to group
/// yet — this returns an empty list and the screen renders the `Packages`
/// header with an explanatory note. It becomes a real grouping the moment the
/// model grows a package id.
#[allow(dead_code)] // PORT-TODO: returns empty until packages are modeled.
fn package_rows(_snippets: &[Snippet]) -> Vec<SnippetPackage> {
    Vec::new()
}

/// One rendered snippet row (owned, so the list can be built after the state
/// borrow ends).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnippetRow {
    id: String,
    presenter: RowPresenter,
    /// True while the open dialog edits this snippet.
    selected: bool,
}

impl SnippetRow {
    fn of(snippet: &Snippet, active_dialog: Option<&Dialog>) -> Self {
        let selected =
            matches!(active_dialog, Some(Dialog::EditSnippet(id)) if *id == snippet.id);
        Self {
            id: snippet.id.clone(),
            presenter: snippet_presenter(snippet),
            selected,
        }
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
/// `size:"medium"`: transparent fill, coloured text, `0.15` substrate hover).
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

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The Snippets section view: the shared [`FiltersHeader`] (with the split
/// `New snippet` action) over a scrollable `Packages` + `Snippets` entity list,
/// or an [`EmptyState`](crate::primitives::EmptyState) when the library holds
/// none.
///
/// Clicking a row opens [`Dialog::EditSnippet`]; the split button's main half
/// opens [`Dialog::AddSnippet`] and its chevron toggles the package menu.
///
/// A separate entity from [`TermiusState`] (gpui leases one entity at a time):
/// the factory below only mints it, and [`Render::render`] reads the state from
/// a context that no longer holds the lease.
pub struct SnippetsScreen {
    state: Entity<TermiusState>,
    /// Whether the split button's menu is showing.
    menu_open: bool,
    /// Re-render whenever the state entity changes.
    _observe_state: Subscription,
    /// Re-render on theme switches.
    _observe_theme: Subscription,
}

/// Build the Snippets screen for `state`.
///
/// Safe to call while `TermiusState` is leased (inside `state.update(..)`): it
/// only mints a view entity, every state read happens later in
/// [`SnippetsScreen`]'s own render. Store the returned entity once (the same
/// way `AppShell` holds `HostList`) — calling this every render would mint a new
/// entity per frame.
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
        Self {
            state,
            menu_open: false,
            _observe_state: observe_state,
            _observe_theme: observe_theme,
        }
    }

    /// Raise a dialog (the shell hosts it above everything).
    fn open_dialog(&mut self, dialog: Dialog, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.state.update(cx, |state, cx| state.open_dialog(dialog, cx));
    }

    /// Toggle the split button's package menu.
    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        self.menu_open = !self.menu_open;
        cx.notify();
    }

    /// The split menu's `New snippet package` item.
    ///
    /// PORT-TODO: [`Snippet`] has no package field, so there is no package
    /// record to create yet; note the gap in the status bar.
    fn new_package(&mut self, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.state.update(cx, |state, cx| {
            state.status_text =
                "New snippet package — package storage arrives with the library wave.".to_owned();
            cx.notify();
        });
    }

    /// The `Shell History` ghost button (`showHistoryCommands`).
    fn shell_history(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.status_text =
                "Shell History — the command browser arrives with the terminal wave.".to_owned();
            cx.notify();
        });
    }

    /// The split `New snippet` button (main + chevron halves).
    fn new_snippet_button(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        let main = primary_icon_button(
            theme,
            "snippets-new",
            PLUS_ICON,
            "New snippet",
            cx.listener(|this, _event, _window, cx| {
                this.open_dialog(Dialog::AddSnippet, cx);
            }),
        );
        let chevron = chevron_button(
            theme,
            "snippets-new-menu",
            cx.listener(|this, _event, _window, cx| this.toggle_menu(cx)),
        );
        div().flex().items_center().gap(px(2.)).child(main).child(chevron)
    }

    /// One `Snippets` list row: a shared [`EntityRow`] wired to the Edit dialog.
    fn snippet_row(&self, row: &SnippetRow, theme: TermiusTheme, cx: &mut Context<Self>) -> Stateful<Div> {
        let open_id = row.id.clone();
        EntityRow::new(
            row.presenter.label.clone(),
            row.presenter.description.clone(),
            row.presenter.icon,
        )
        .selected(row.selected)
        .on_click(
            theme,
            cx.listener(move |this, _event, _window, cx| {
                this.open_dialog(Dialog::EditSnippet(open_id.clone()), cx);
            }),
        )
    }
}

impl Render for SnippetsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Read the slice once: rows (snippets in Termius order) plus which row
        // the open Edit dialog targets. Everything is cloned out so the borrow
        // ends before the listeners below register against `cx`.
        let (rows, packages, loading) = {
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
            let packages = package_rows(&state.library.snippets);
            (rows, packages, state.library_loading)
        };
        let empty = rows.is_empty() && packages.is_empty();

        // Header: the shared filter band, primary action first (no title band).
        let action = div()
            .flex()
            .items_center()
            .gap(px(3.))
            .child(self.new_snippet_button(theme, cx))
            .child(ghost_icon_button(
                theme,
                "snippets-shell-history",
                HISTORY_ICON,
                "Shell History",
                cx.listener(|this, _event, _window, cx| this.shell_history(cx)),
            ));
        let header = FiltersHeader::new("Search snippets").action(action).element(theme);

        // Scrollable two-level list (wheel scrolling comes free with
        // `overflow_y_scroll` in gpui 0.2).
        let mut body = div()
            .id("snippets-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .py(px(4.));

        if empty && loading {
            body = body.child(muted_note(theme, "Loading snippets…"));
        } else if empty {
            // No snippets and no packages → the original `XSe` empty card.
            body = body.child(
                crate::primitives::EmptyState::new(
                    "Create snippet",
                    "Save your most used commands as snippets to reuse them in one click.",
                )
                .element(theme),
            );
        } else {
            // Level 1 — Packages.
            body = body.child(SectionHeader::new("Packages").element(theme));
            if packages.is_empty() {
                body = body.child(muted_note(
                    theme,
                    "No packages yet. Group related snippets into a package.",
                ));
            } else {
                for package in &packages {
                    let presenter = package_presenter(package);
                    body = body.child(
                        EntityRow::new(presenter.label, presenter.description, presenter.icon)
                            .element(theme),
                    );
                }
            }

            // Level 2 — Snippets.
            body = body.child(SectionHeader::new("Snippets").element(theme));
            if rows.is_empty() {
                body = body.child(muted_note(
                    theme,
                    "No snippets in this package yet. Add one with New snippet.",
                ));
            } else {
                for row in &rows {
                    body = body.child(self.snippet_row(row, theme, cx));
                }
            }
        }

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
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
                            "snippets-new-package",
                            PACKAGE_ICON,
                            "New snippet package",
                            cx.listener(|this, _event, _window, cx| this.new_package(cx)),
                        )),
                ),
            );
        }

        root.child(body)
    }
}

// ---------------------------------------------------------------------------
// Dialog body (Add / Edit snippet)
// ---------------------------------------------------------------------------

/// The complete [`DialogFrame`] card for the snippet dialogs, or `None` when
/// `dialog` isn't a snippet dialog (or targets a snippet that no longer
/// exists).
///
/// The card is `DialogFrame`-shaped end to end: title bar from [`Dialog::title`]
/// (with the close affordance), the original `Action description` + `Package` +
/// `Script` fields, then Delete (edit only) / Cancel / Save. Save mutates the
/// state and closes the dialog.
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
) -> Div {
    let title = existing.map(|snippet| snippet.title.clone()).unwrap_or_default();
    let body = existing.map(|snippet| snippet.body.clone()).unwrap_or_default();
    let save_target = existing.map(|snippet| snippet.id.clone());

    let mut frame = DialogFrame::new(dialog.title())
        .on_close(cx.listener(|this, _event, _window, cx| this.close_dialog(cx)))
        // Original field `label` — "Action description" (`_main.js:84456`),
        // 50-char limit, placeholder "Example: check network load".
        .child(
            InputField::new("Action description", title)
                .placeholder("Example: check network load")
                .element(theme),
        )
        // Original field `package` — the `Add a Package` CreatableSelect
        // (`_main.js:84480`). Display-only here (PORT-TODO).
        .child(
            InputField::new("Package", "")
                .placeholder("Add a Package")
                .element(theme),
        )
        // Original field `script` — the `minRows:6` monospace textarea.
        .child(script_field(theme, &body))
        .child(
            SettingsText::new(
                "The snippet text is inserted as-is when you run it from the Snippets list.",
            )
            .element(theme),
        );

    // The edit form's overflow menu holds Remove; without an anchored menu the
    // card carries a Delete action instead (PORT-TODO).
    if let Some(id) = save_target.clone() {
        frame = frame.action(Button::new("Delete").danger().on_click(
            theme,
            cx.listener(move |this, _event, _window, cx| {
                this.delete_snippet(&id, cx);
                this.close_dialog(cx);
            }),
        ));
    }

    frame
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
                            sort_order: this.library.snippets.len() as i64,
                            ..Snippet::default()
                        };
                        this.add_snippet(fresh, cx);
                    }
                }
                this.close_dialog(cx);
            }),
        ))
        .element(theme)
}

/// The `Script` labeled multi-line body box: one line per body line, a muted
/// placeholder when the body is empty (the original `script` textarea,
/// `minRows: 6`, monospace).
fn script_field(theme: TermiusTheme, body: &str) -> Div {
    let mut field = div().flex().flex_col().gap(px(6.));
    field = field.child(
        div()
            .font_family(UI_FONT)
            .text_size(px(14.))
            .font_weight(FontWeight::NORMAL)
            .line_height(px(21.))
            .text_color(theme.text_common)
            .child(SharedString::from("Script")),
    );

    let mut box_el = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .min_h(px(SCRIPT_MIN_HEIGHT))
        .p(px(10.))
        .rounded(px(theme.corner_radius_small))
        .border_1()
        .border_color(theme.border_basic)
        .bg(theme.card_c);

    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        box_el = box_el.child(
            div()
                .font_family(UI_FONT)
                .text_size(px(14.))
                .text_color(theme.text_common)
                .child(SharedString::from("Enter a command or script…")),
        );
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
    fn first_regular_line_skips_blanks_and_comments() {
        assert_eq!(
            first_line_with_regular_characters("\n  # a comment\n  sudo uptime\nwhoami")
                .as_deref(),
            Some("sudo uptime")
        );
        assert_eq!(first_line_with_regular_characters("whoami").as_deref(), Some("whoami"));
        assert_eq!(first_line_with_regular_characters("  \n\t\n"), None);
        assert_eq!(first_line_with_regular_characters("# only a comment"), None);
    }

    #[test]
    fn snippet_presenter_uses_label_and_first_regular_line() {
        let presenter = snippet_presenter(&snippet("s", "Deploy", "# note\nsudo uptime", None));
        assert_eq!(presenter.label, "Deploy");
        assert_eq!(presenter.description, "sudo uptime");
        assert_eq!(presenter.icon, SNIPPET_ICON);
    }

    #[test]
    fn snippet_presenter_has_no_description_without_a_label() {
        // The original gates the description on `entity.label`.
        let presenter = snippet_presenter(&snippet("s", "   ", "uptime", None));
        assert_eq!(presenter.label, "");
        assert_eq!(presenter.description, "");
    }

    #[test]
    fn package_presenter_counts_snippets() {
        let one = package_presenter(&SnippetPackage { label: "Ops".into(), snippet_count: 1 });
        assert_eq!(one.label, "Ops");
        assert_eq!(one.description, "1 snippet");
        assert_eq!(one.icon, PACKAGE_ICON);

        let many = package_presenter(&SnippetPackage { label: "Net".into(), snippet_count: 4 });
        assert_eq!(many.description, "4 snippets");
    }

    #[test]
    fn snippet_count_label_pluralises() {
        assert_eq!(snippet_count_label(0), "0 snippets");
        assert_eq!(snippet_count_label(1), "1 snippet");
        assert_eq!(snippet_count_label(9), "9 snippets");
    }

    #[test]
    fn package_rows_is_empty_until_packages_are_modeled() {
        // PORT-TODO: `Snippet` has no package field yet.
        assert!(package_rows(&[snippet("s", "Deploy", "uptime", None)]).is_empty());
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
    fn row_carries_presenter_and_id() {
        let row = SnippetRow::of(&snippet("s1", "Deploy", "uptime", None), None);
        assert_eq!(row.id, "s1");
        assert_eq!(row.presenter.label, "Deploy");
        assert_eq!(row.presenter.description, "uptime");
    }

    #[test]
    fn screen_icons_are_bundled() {
        // The toolbar glyph is the thin plus, never `addCircle.svg`.
        assert!(crate::has_icon(PLUS_ICON));
        assert!(crate::has_icon(SNIPPET_ICON));
        assert!(crate::has_icon(PACKAGE_ICON));
        assert!(crate::has_icon(HISTORY_ICON));
        assert!(crate::has_icon(CHEVRON_ICON));
    }
}
