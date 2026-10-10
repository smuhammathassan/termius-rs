//! primitives — small reusable GPUI components shared by every screen.
//!
//! Each primitive is a plain builder struct (no `Entity`, no listeners of its
//! own) with one `element(theme)` method producing the gpui element. Event
//! wiring stays in the owning view: interactive builders return
//! `Stateful<Div>` and expose an `on_click(theme, listener)` convenience, so
//! callers can chain `.on_click(cx.listener(|this, …, cx| …))` exactly like
//! `views::host_list` does today.
//!
//! ```ignore
//! // in a view's `render`:
//! let theme = theme_of(cx);
//! Button::new("Add host")
//!     .primary()
//!     .on_click(theme, cx.listener(|this, _event, _window, cx| this.add_host(cx)));
//! ```
//!
//! Styling comes exclusively from [`TermiusTheme`] so a theme switch repaints
//! every screen for free.
//!
//! The internals are a 1:1 port of the original Termius web components (the
//! rollup modules under `analysis/termius-extracted/ui-process/assets/`):
//!
//! * [`SettingsSection`] ← `SettingsSection-312062fd.js`
//! * [`SettingsTitle`] / [`SettingsText`] ← `SettingsTitle-46c13b9b.js`,
//!   `SettingsText-cb0a3f73.js` (Termius' custom `Typography`: `subtitle1`
//!   = 14/700, `body2` = 12/450, `Margin top:15`)
//! * [`Switch`] ← `SettingsSwitch-200210fd.js` + `Switch$1` in
//!   `reconnectSaga-f0db0c3c.js` (label 12px, `Margin top:20`, MUI switch
//!   track 40×14 with a 20px thumb, `--button-accent` when checked)
//! * [`ListItem`] ← `ListItem-38a69640.js` (MUI `MuiListItem` root: flex,
//!   dense `4/16` padding, hover wash, `selected` fill, `divider` rule)
//! * [`DialogFrame`] ← `DialogPanel-20312ab7.js` (`role=dialog`,
//!   `--foreground` card, `dialogContent` 40px padding, `dialogActions`
//!   20px + top rule)
//! * [`Button`] ← `Button-ab3b44b3.js` (MUI Button metrics) restyled with
//!   Termius' own `Button$1`/`ButtonImpl` sizes (36px tall, 16px inline
//!   padding, `--corner-radius-medium`, 14/500) and its hover "substrate"
//!   overlay.
//!
//! PORT-TODO: `InputField` renders its value as static text; real text editing
//! needs a focused content model (gpui `Content`/`InputEvent` on the tracked
//! [`FocusHandle`]) — wire it when the Add-Host form lands.
//!
//! PORT-TODO: gpui 0.2.2 has no ARIA/role support, so [`DialogFrame`] cannot
//! emit the original `role="dialog"` / `tabIndex=0` attributes.

use gpui::{
    div, px, App, ClickEvent, Div, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window,
};

use crate::theme::{over, with_alpha, TermiusTheme, ThemeMode};

/// Default button height — Termius' `large` button (`height: 36px`).
const BUTTON_HEIGHT: f32 = 36.0;
/// MUI dense list row padding: `paddingTop/Bottom: 4`, gutters `16`.
const ROW_PAD_Y: f32 = 4.0;
const ROW_PAD_X: f32 = 16.0;
/// Settings section header height.
const SECTION_HEADER_HEIGHT: f32 = 34.0;
/// Input box height.
const INPUT_HEIGHT: f32 = 36.0;
/// Dialog card width.
const DIALOG_WIDTH: f32 = 460.0;

/// White used for text/knobs on accent or danger fills, and for the button
/// hover "substrate" overlay (`--substrateColor: var(--white)`).
fn on_fill() -> gpui::Rgba {
    gpui::rgb(0xff_ff_ff)
}

/// `body2` / `subtitle1` line height (`lineHeight: 1.5` at 12 / 14px).
fn line_height_of(size: f32) -> f32 {
    size * 1.5
}

/// Termius' "regular weight" body face (`fontWeight: 450` — CircularXX Book).
fn body_weight() -> FontWeight {
    FontWeight(450.0)
}

/// `--button-disabled` (also the dialog action-row rule): `--card-c` in the
/// dark theme, `--card-b` in the light one.
fn dim_fill(theme: TermiusTheme) -> gpui::Rgba {
    match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    }
}

// ---------------------------------------------------------------------------
// EmptyState
// ---------------------------------------------------------------------------

/// A centered placeholder for sections/screens with no content yet.
pub struct EmptyState {
    title: SharedString,
    subtitle: SharedString,
}

impl EmptyState {
    pub fn new(title: impl Into<SharedString>, subtitle: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
        }
    }

    /// A full-size, centered placeholder (give it a sized parent).
    ///
    /// Typography matches Termius' `subtitle1` heading over a `body2` note:
    /// 14/700 `--text-primary` above 12/450 `--text-common`.
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .size_full()
            .text_color(theme.title)
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .line_height(px(line_height_of(14.)))
                    .child(self.title),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(body_weight())
                    .line_height(px(line_height_of(12.)))
                    .text_color(theme.text_common)
                    .child(self.subtitle),
            )
    }
}

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

/// Visual weight of a [`Button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Accent fill; the primary action of a form/dialog.
    Primary,
    /// Quiet fill; the default for list/sidebar affordances.
    Secondary,
    /// Danger fill; destructive actions (delete…).
    Danger,
}

/// A labeled button with a click listener slot.
pub struct Button {
    label: SharedString,
    variant: ButtonVariant,
    disabled: bool,
}

impl Button {
    /// A secondary (quiet) button.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            variant: ButtonVariant::Secondary,
            disabled: false,
        }
    }

    /// Set the variant explicitly.
    pub fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Shortcut for [`ButtonVariant::Primary`].
    pub fn primary(mut self) -> Self {
        self.variant = ButtonVariant::Primary;
        self
    }

    /// Shortcut for [`ButtonVariant::Secondary`].
    pub fn secondary(mut self) -> Self {
        self.variant = ButtonVariant::Secondary;
        self
    }

    /// Shortcut for [`ButtonVariant::Danger`].
    pub fn danger(mut self) -> Self {
        self.variant = ButtonVariant::Danger;
        self
    }

    /// Render in the disabled palette (`--button-disabled` fill,
    /// `--text-disabled` label) and drop the click listener.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The current variant.
    pub fn variant(&self) -> ButtonVariant {
        self.variant
    }

    /// The themed button element (stateful, so `.on_click` can be chained).
    ///
    /// MUI `MuiButton` box metrics (min inline padding, centered label)
    /// combined with Termius' `ButtonImpl` `large` sizing: `height: 36px`,
    /// `padding: 0 16px`, `fontSize: 14`, `fontWeight: 500`,
    /// `borderRadius: var(--corner-radius-medium)`.
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let white = on_fill();
        let disabled = self.disabled;
        let (background, foreground, border) = if disabled {
            (dim_fill(theme), theme.text_common, None)
        } else {
            match self.variant {
                ButtonVariant::Primary => (theme.primary, white, None),
                ButtonVariant::Secondary => (theme.card_c, theme.title, Some(theme.border_basic)),
                ButtonVariant::Danger => (theme.danger, white, None),
            }
        };
        // Termius' hover substrate: `--substrateColor: var(--white)` fading in
        // to `--substrateOpacity: .25` over the button colour.
        let hover_fill = over(background, with_alpha(white, 0.25));

        let mut button = div()
            .id(SharedString::from(format!("button-{}", self.label)))
            .flex()
            .items_center()
            .justify_center()
            .h(px(BUTTON_HEIGHT))
            .px(px(16.))
            .rounded(px(theme.corner_radius_medium))
            .bg(background)
            .text_color(foreground)
            .text_size(px(14.))
            .font_weight(FontWeight::MEDIUM)
            .line_height(px(line_height_of(14.)))
            .whitespace_nowrap()
            .child(self.label);

        if let Some(border) = border {
            button = button.border_1().border_color(border);
        }
        if disabled {
            button
        } else {
            button.hover(move |hover| hover.bg(hover_fill))
        }
    }

    /// The themed button element with a gpui click listener attached.
    ///
    /// `listener` is the shape `cx.listener(|this, event, window, cx| …)`
    /// produces, so views pass their own listener unchanged. A disabled button
    /// renders without the listener.
    pub fn on_click(
        self,
        theme: TermiusTheme,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        let disabled = self.disabled;
        let element = self.element(theme);
        if disabled {
            element
        } else {
            element.on_click(listener)
        }
    }
}

// ---------------------------------------------------------------------------
// InputField
// ---------------------------------------------------------------------------

/// A labeled value box for forms (Add Host, settings values…).
///
/// PORT-TODO: value display only — attach a [`FocusHandle`] with
/// [`InputField::focus_handle`] so the box paints its focused state, and wire
/// real editing (`Content` + `InputEvent`) when the forms land.
pub struct InputField {
    label: SharedString,
    value: SharedString,
    placeholder: SharedString,
    focus: Option<FocusHandle>,
}

impl InputField {
    pub fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            placeholder: SharedString::default(),
            focus: None,
        }
    }

    /// Shown (muted) while the value is empty.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Track a focus handle on the box (focused styling / future editing).
    pub fn focus_handle(mut self, focus: FocusHandle) -> Self {
        self.focus = Some(focus);
        self
    }

    /// The label currently displayed (value, else placeholder).
    pub fn shown_text(&self) -> SharedString {
        if self.value.is_empty() {
            self.placeholder.clone()
        } else {
            self.value.clone()
        }
    }

    /// The labeled field element.
    ///
    /// MUI `TextField variant="outlined"` ported: `MuiFormLabel` root
    /// (`fontSize: 14px`, `--light-grey-3` = `--text-common`) above an
    /// outlined box — `--border-basic` rule, `--corner-radius-small`,
    /// `--main-form-bg` (`--card-c`) fill, `r14p` value text and the
    /// `--form-input-color` placeholder.
    pub fn element(self, theme: TermiusTheme) -> Div {
        let empty = self.value.is_empty();
        let text_color = if empty {
            theme.text_common // --form-input-color
        } else {
            theme.title // --input-color / --main-color
        };
        // Read before `self.label` is moved out below.
        let shown = self.shown_text();

        let mut field = div().flex().flex_col().gap(px(6.));
        field = field.child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::NORMAL)
                .line_height(px(line_height_of(14.)))
                .text_color(theme.text_common)
                .child(self.label),
        );

        let mut box_el = div()
            .flex()
            .items_center()
            .h(px(INPUT_HEIGHT))
            .px(px(12.))
            .rounded(px(theme.corner_radius_small))
            .border_1()
            .border_color(theme.border_basic)
            .bg(theme.card_c)
            .text_size(px(14.))
            .font_weight(body_weight())
            .line_height(px(line_height_of(14.)))
            .text_color(text_color);
        box_el = box_el.child(shown);
        if let Some(focus) = self.focus.as_ref() {
            box_el = box_el.track_focus(focus);
        }
        field.child(box_el)
    }
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

/// A settings toggle row: label on the left, pill on the right, an optional
/// `body2` description underneath.
pub struct Switch {
    label: SharedString,
    description: SharedString,
    on: bool,
}

impl Switch {
    pub fn new(label: impl Into<SharedString>, on: bool) -> Self {
        Self {
            label: label.into(),
            description: SharedString::default(),
            on,
        }
    }

    /// The `body2` note rendered under the row (`--text-secondary`).
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = description.into();
        self
    }

    /// Whether the toggle is on.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The toggled state (the owning view persists the flip).
    pub fn flipped(mut self) -> Self {
        self.on = !self.on;
        self
    }

    /// The themed toggle row (stateful: chain `.on_click` to flip it).
    ///
    /// `SettingsSwitch-200210fd.js` renders `Margin(top: 20)` around the
    /// `Switch$1` row: a 12px label left, the MUI switch right, and the
    /// optional description below. The pill follows MUI's `MuiSwitch`
    /// geometry (40×14 track, `borderRadius: 7`, 20px thumb that overhangs by
    /// 3px top/bottom) with Termius' colours: checked = `--button-accent`
    /// (`--blue`), unchecked track = `--dark-grey-5`/`--light-grey-4`
    /// (`--border-strong`).
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        // Build the element id before `self.label` is moved into the row.
        let id = SharedString::from(format!("switch-{}", self.label));
        let track = if self.on { theme.primary } else { theme.border_strong };
        // Unchecked knob: `--dark-grey-6` in the dark theme (nearest token we
        // hold: `--c-text-common`) and `--white` (`--card-c`) in the light one.
        let thumb = match (self.on, theme.mode) {
            (true, _) => theme.primary,
            (false, ThemeMode::Dark) => theme.muted,
            (false, ThemeMode::Light) => theme.card_c,
        };
        let mut pill = div()
            .flex()
            .items_center()
            .w(px(40.))
            .h(px(14.))
            .rounded(px(7.))
            .bg(track)
            .child(div().w(px(20.)).h(px(20.)).rounded(px(10.)).bg(thumb));
        if self.on {
            pill = pill.justify_end();
        }

        let mut line = div().flex().items_center().justify_between().gap(px(8.));
        line = line.child(
            div()
                .min_w(px(0.))
                .text_size(px(12.))
                .line_height(px(line_height_of(12.)))
                .text_color(theme.primary)
                .truncate()
                .child(self.label),
        );
        line = line.child(pill);

        let mut row = div()
            .id(id)
            .flex()
            .flex_col()
            .mt(px(20.))
            .gap(px(4.))
            .text_color(theme.title)
            .child(line);

        if !self.description.is_empty() {
            row = row.child(
                div()
                    .text_size(px(12.))
                    .font_weight(body_weight())
                    .line_height(px(line_height_of(12.)))
                    .text_color(theme.text_common)
                    .child(self.description),
            );
        }
        row
    }

    /// The themed toggle row with a click listener attached (flip in the
    /// owning view: `cx.listener(|this, _, _, cx| this.toggle_setting(cx))`).
    pub fn on_click(
        self,
        theme: TermiusTheme,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        self.element(theme).on_click(listener)
    }
}

// ---------------------------------------------------------------------------
// ListItem
// ---------------------------------------------------------------------------

/// One selectable row (snippet, key, forwarding rule, log line…).
pub struct ListItem {
    label: SharedString,
    subtitle: SharedString,
    selected: bool,
    divider: bool,
    id: Option<SharedString>,
}

impl ListItem {
    pub fn new(label: impl Into<SharedString>, subtitle: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            subtitle: subtitle.into(),
            selected: false,
            divider: false,
            id: None,
        }
    }

    /// Highlight the row as selected.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Draw MUI's `divider` slot: a `1px` rule under the row
    /// (`--border-light`).
    pub fn divider(mut self, divider: bool) -> Self {
        self.divider = divider;
        self
    }

    /// Override the element id (defaults to the label; set it when labels may
    /// repeat so gpui's hit-testing stays unique).
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Whether the row renders as selected.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// The themed row (stateful: chain `.on_click` to select/open it).
    ///
    /// MUI `MuiListItem` root: `display: flex`, `justifyContent: flex-start`,
    /// `alignItems: center`, dense padding (`4px 16px`), full width. Termius
    /// adds the row fills: `selected` → `--list-select` (`--card-c`) inside a
    /// `--border-accent` rule, hover → `--blue-a10` (`--hover`), `divider` →
    /// `--border-light`.
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let id = self
            .id
            .unwrap_or_else(|| SharedString::from(format!("list-item-{}", self.label)));
        // The hover wash composites over the row's own fill (or straight onto
        // the panel when the row has none).
        let hover_fill = if self.selected {
            over(theme.card_c, theme.hover)
        } else {
            theme.hover
        };

        let mut row = div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(ROW_PAD_X))
            .py(px(ROW_PAD_Y))
            .rounded(px(theme.corner_radius_small))
            .text_color(theme.title);
        if self.selected {
            row = row
                .bg(theme.card_c)
                .border_1()
                .border_color(theme.border_accent);
        } else if self.divider {
            row = row.border_b_1().border_color(theme.border_light);
        }

        row = row.child(
            div()
                .text_size(px(14.))
                .font_weight(body_weight())
                .line_height(px(line_height_of(14.)))
                .child(self.label),
        );
        if !self.subtitle.is_empty() {
            row = row.child(
                div()
                    .text_size(px(12.))
                    .font_weight(body_weight())
                    .line_height(px(line_height_of(12.)))
                    .text_color(theme.text_common)
                    .child(self.subtitle),
            );
        }
        row.hover(move |hover| hover.bg(hover_fill))
    }

    /// The themed row with a click listener attached.
    pub fn on_click(
        self,
        theme: TermiusTheme,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        self.element(theme).on_click(listener)
    }
}

// ---------------------------------------------------------------------------
// Settings chrome
// ---------------------------------------------------------------------------

/// A muted group header (sidebar group, settings sub-section).
pub struct SectionHeader {
    title: SharedString,
}

impl SectionHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
        }
    }

    /// The header row element: Termius' `r12p`-style caption (12/450,
    /// `--text-common`) on a `SECTION_HEADER_HEIGHT` band.
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .flex()
            .items_center()
            .h(px(SECTION_HEADER_HEIGHT))
            .px(px(12.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .line_height(px(line_height_of(12.)))
            .whitespace_nowrap()
            .text_color(theme.text_common)
            .child(self.title)
    }
}

/// A settings page/screen title.
pub struct SettingsTitle {
    text: SharedString,
}

impl SettingsTitle {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
        }
    }

    /// The title element — Termius' `subtitle1` (`fontSize: 14px`,
    /// `fontWeight: 700`, `lineHeight: 1.5`, `color: --text-primary`).
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .text_size(px(14.))
            .font_weight(FontWeight::BOLD)
            .line_height(px(line_height_of(14.)))
            .text_color(theme.title)
            .child(self.text)
    }
}

/// A muted paragraph of settings help text.
pub struct SettingsText {
    text: SharedString,
}

impl SettingsText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
        }
    }

    /// The paragraph element — `Margin(top: 15)` around a `body2`
    /// (`fontSize: 12px`, `fontWeight: 450`, `color: --text-secondary`).
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .mt(px(15.))
            .text_size(px(12.))
            .font_weight(body_weight())
            .line_height(px(line_height_of(12.)))
            .whitespace_normal()
            .text_color(theme.text_common)
            .child(self.text)
    }
}

/// A titled card grouping settings rows (`Switch`, `InputField`…).
pub struct SettingsSection {
    title: SharedString,
    children: Vec<gpui::AnyElement>,
}

impl SettingsSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            children: Vec::new(),
        }
    }

    /// Append one row/element.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Append several rows/elements.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children
            .extend(children.into_iter().map(|child| child.into_any_element()));
        self
    }

    /// How many rows the section currently holds.
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// True when the section has no rows.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// The themed card element.
    ///
    /// `SettingsSection-312062fd.js` verbatim: `padding: 20px`,
    /// `background: var(--card-a)`, `borderRadius: var(--corner-radius-medium)`,
    /// `width: 100%`, `maxWidth: 700px`, `margin: 30px auto 0` (the
    /// `&:last-child { marginBottom: 60px }` rule needs sibling awareness a
    /// single element cannot have). The card's heading is the `subtitle1`
    /// title the port's callers pass to `::new`.
    pub fn element(self, theme: TermiusTheme) -> Div {
        let mut section = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(700.))
            .mx_auto()
            .mt(px(30.))
            .p(px(20.))
            .rounded(px(theme.corner_radius_medium))
            .bg(theme.card_a)
            .text_color(theme.title);
        section = section.child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::BOLD)
                .line_height(px(line_height_of(14.)))
                .mb(px(10.))
                .child(self.title),
        );
        let mut body = div().flex().flex_col().gap(px(8.));
        for child in self.children {
            body = body.child(child);
        }
        section.child(body)
    }
}

// ---------------------------------------------------------------------------
// DialogFrame
// ---------------------------------------------------------------------------

/// A centered modal card: title bar, body, action row.
///
/// [`DialogFrame::element`] builds the card only; the owning view hosts it in
/// a full-window scrim (see `views::app_shell::AppShell`).
pub struct DialogFrame {
    title: SharedString,
    children: Vec<gpui::AnyElement>,
    actions: Vec<gpui::AnyElement>,
}

impl DialogFrame {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            children: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// Append one body element.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Append several body elements.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children
            .extend(children.into_iter().map(|child| child.into_any_element()));
        self
    }

    /// Append one action button (bottom-right, in insertion order).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// Append several action buttons.
    pub fn actions(mut self, actions: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.actions
            .extend(actions.into_iter().map(|action| action.into_any_element()));
        self
    }

    /// How many body elements the frame holds.
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// True when the body is empty.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// The themed modal card element.
    ///
    /// `DialogPanel-20312ab7.js`: `role="dialog"` + `justifyContent:
    /// space-between` split into `dialogContent` / `dialogActions`. The card
    /// fill is `var(--foreground)` (which resolves to `--card-a` in both
    /// themes), the radius is `--corner-radius-large`, content padding is
    /// `40px` (`20px` bottom when an action row follows), and the action row
    /// is `20px` with a `1px` top rule in `--card-c` (dark) / `--card-b`
    /// (light). The `--backdrop` scrim belongs to the caller; gpui 0.2.2 has
    /// no ARIA attributes, so `role="dialog"` cannot be emitted.
    pub fn element(self, theme: TermiusTheme) -> Div {
        let has_actions = !self.actions.is_empty();

        let mut card = div()
            .flex()
            .flex_col()
            .w(px(DIALOG_WIDTH))
            .rounded(px(theme.corner_radius_large))
            .overflow_hidden()
            .bg(theme.card_a)
            .text_color(theme.title);

        card = card.child(
            div()
                .flex()
                .items_center()
                .px(px(40.))
                .pt(px(40.))
                .pb(px(10.))
                .text_size(px(14.))
                .font_weight(FontWeight::BOLD)
                .line_height(px(line_height_of(14.)))
                .child(self.title),
        );

        let mut content = div().flex().flex_col().gap(px(10.)).px(px(40.));
        content = if has_actions {
            content.pb(px(20.))
        } else {
            content.pb(px(40.))
        };
        for child in self.children {
            content = content.child(child);
        }
        // The original panel stretches `dialogContent` (`flexGrow: 1`) because
        // it is sized `100vh`; this card is content-sized, so the natural
        // height is what we want.
        card = card.child(content);

        if has_actions {
            let mut row = div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .p(px(20.))
                .border_t_1()
                .border_color(dim_fill(theme));
            for action in self.actions {
                row = row.child(action);
            }
            card = card.child(row);
        }
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_variants_round_trip() {
        assert_eq!(Button::new("Save").variant(), ButtonVariant::Secondary);
        assert_eq!(Button::new("Save").primary().variant(), ButtonVariant::Primary);
        assert_eq!(Button::new("Save").secondary().variant(), ButtonVariant::Secondary);
        assert_eq!(Button::new("Save").danger().variant(), ButtonVariant::Danger);
        assert_eq!(
            Button::new("Save")
                .with_variant(ButtonVariant::Primary)
                .variant(),
            ButtonVariant::Primary
        );
    }

    #[test]
    fn input_field_shows_value_else_placeholder() {
        let mut field = InputField::new("Host", "web-1.example.com").placeholder("hostname");
        assert_eq!(field.shown_text(), SharedString::from("web-1.example.com"));
        field.value = SharedString::default();
        assert_eq!(field.shown_text(), SharedString::from("hostname"));
    }

    #[test]
    fn switch_flips() {
        let switch = Switch::new("Cursor blink", true);
        assert!(switch.is_on());
        assert!(!switch.flipped().is_on());
        let described = Switch::new("Row", true).description("Note");
        assert_eq!(described.description.len(), 4);
    }

    #[test]
    fn list_item_selection_and_ids() {
        assert!(!ListItem::new("web-1", "10.0.0.1").is_selected());
        assert!(ListItem::new("web-1", "").selected(true).is_selected());
        // Additive builders keep their defaults.
        assert!(!ListItem::new("web-1", "").divider);
        assert!(!Button::new("Save").disabled);
    }

    #[test]
    fn settings_section_and_dialog_accumulate_children() {
        let section = SettingsSection::new("Terminal")
            .child(div())
            .children([div(), div()]);
        assert_eq!(section.len(), 3);
        assert!(!section.is_empty());

        let empty = DialogFrame::new("Add host");
        assert!(empty.is_empty());
        assert_eq!(empty.child(div()).action(div()).len(), 1);
    }

    #[test]
    fn elements_build_against_the_theme_without_a_window() {
        // Constructing (not painting) every primitive must not need a window:
        // this is what later screens call in their `render` impls.
        let theme = TermiusTheme::dark();
        let _ = EmptyState::new("Snippets", "Nothing yet").element(theme);
        let _ = Button::new("Add").primary().element(theme);
        let _ = Button::new("Delete").danger().element(theme);
        let _ = Button::new("Save").disabled(true).element(theme);
        let _ = InputField::new("Label", "value").placeholder("…").element(theme);
        let _ = Switch::new("Cursor blink", true).element(theme);
        let _ = Switch::new("Line numbers", false)
            .description("Show a gutter.")
            .element(theme);
        let _ = ListItem::new("row", "sub").selected(true).element(theme);
        let _ = ListItem::new("row", "sub").divider(true).element(theme);
        let _ = SectionHeader::new("Group").element(theme);
        let _ = SettingsTitle::new("Settings").element(theme);
        let _ = SettingsText::new("Help.").element(theme);
        let _ = SettingsSection::new("Terminal")
            .child(Switch::new("Cursor blink", true).element(theme))
            .element(theme);
        let _ = DialogFrame::new("Add host")
            .child(InputField::new("Hostname", "").element(theme))
            .action(Button::new("Cancel").element(theme))
            .element(theme);
    }
}
