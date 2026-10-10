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
//! every screen for free. Where the theme has no token for a reconstructed
//! CSS variable (e.g. `--dark-blue-solid`, `--light-grey-6`) the exact value
//! from the shipped stylesheet (`reconnectSaga-e37b5571.css`) is kept as a
//! documented local constant — `theme.rs` is intentionally left untouched.
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
//!   20px + top rule, `--corner-radius-large-increased`) + the close/back
//!   affordance of `CloseAndBackDialogSection-7cad7967.js`
//! * [`Button`] ← `Button-ab3b44b3.js` (MUI Button metrics) restyled with
//!   Termius' own `Button$1`/`ButtonImpl` sizes (tiny 20 / small 24 /
//!   medium 30 / large 36, `--corner-radius-*`, 14/500) and its hover
//!   "substrate" overlay.
//! * [`ShapedIcon`] ← `ShapedIcon` (`reconnectSaga:88420`): 40×40, radius 12,
//!   `--dark-blue-solid` fill, white 24px glyph.
//! * [`NavItem`] ← `aFe` (`_main.js:111361`, styles `sFe` `_main.js:111299`):
//!   the left-rail section row — 36px, `margin 8px 10px`, radius
//!   `--corner-radius-medium`, 14px label.
//! * [`EntityRow`] ← `GridItemPresenter` / `EntityItem` / `ItemLayout`
//!   (`reconnectSaga:131214` / `102710` / `102669`): `padding:10px`,
//!   radius 14, `2px solid var(--entity-item-background)`, hover
//!   `--list-hover-hover`, title 14px `--text-primary`, subtitle 11px
//!   `--text-secondary`, leading 40×40 [`ShapedIcon`], trailing ⋯ menu.
//! * [`FiltersHeader`] ← `FiltersHeader` (`reconnectSaga:124978`) /
//!   `HostsFiltersHeader` (`HostsFiltersHeader-fc79316f.js`): the shared
//!   45px search/filter band (`--surface-high`), primary action first.
//! * [`TabChip`] ← the top-strip tab `zN`/`F7` (`_main.js:106407` /
//!   `106568`): `min-height:30px`, `padding:0 10px 0 5px`, radius
//!   `--corner-radius-medium`, icon↔close swap.
//!
//! PORT-TODO: `InputField` renders its value as static text; real text editing
//! needs a focused content model (gpui `Content`/`InputEvent` on the tracked
//! [`FocusHandle`]) — wire it when the Add-Host form lands.
//!
//! PORT-TODO: gpui 0.2.2 has no ARIA/role support, so [`DialogFrame`] cannot
//! emit the original `role="dialog"` / `tabIndex=0` attributes.

use gpui::{
    div, px, AnyElement, App, ClickEvent, Div, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Rgba, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window,
};

use crate::assets::icon;
use crate::theme::{over, with_alpha, TermiusTheme, ThemeMode, UI_FONT};

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
/// `--corner-radius-large-increased` (`DialogPanel`).
const DIALOG_RADIUS: f32 = 20.0;
/// `sFe.root` — the left-rail section row.
const NAV_ITEM_HEIGHT: f32 = 36.0;
/// `ShapedIcon.shape` — the entity icon tile.
const SHAPED_ICON_SIZE: f32 = 40.0;
const SHAPED_ICON_RADIUS: f32 = 12.0;
const SHAPED_ICON_GLYPH: f32 = 24.0;
/// `FiltersHeader.header` (dark `min-height: 45px`).
const FILTERS_HEADER_HEIGHT: f32 = 45.0;
/// `zN.root` / `F7.root` — the top-strip tab.
const TAB_HEIGHT: f32 = 30.0;
/// `GridItemPresenter.entityItem` — the entity card radius.
const ENTITY_RADIUS: f32 = 14.0;

/// White used for text/knobs on accent or danger fills, and for the button
/// hover "substrate" overlay (`--substrateColor: var(--white)`).
fn on_fill() -> gpui::Rgba {
    gpui::rgb(0xff_ff_ff)
}

/// Fully transparent (an unselected row fill).
fn clear() -> Rgba {
    Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
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

// --- reconstructed tokens the theme does not model -------------------------
//
// Values verbatim from `analysis/termius-extracted/ui-process/assets/
// reconnectSaga-e37b5571.css`; kept local so `theme.rs` stays untouched.

/// `--dark-blue-solid` — the default [`ShapedIcon`] tile fill.
fn dark_blue_solid() -> Rgba {
    gpui::rgb(0x00_48_78)
}

/// `sFe.root` hover fill: `--light-grey-6` / `--dark-grey-4`.
fn nav_hover_fill(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => gpui::rgb(0xed_f1_f2),
        ThemeMode::Dark => gpui::rgb(0x32_36_4a),
    }
}

/// `sFe.root.selected` fill: `--light-grey-5` / `--dark-grey-5`.
fn nav_selected_fill(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => gpui::rgb(0xe6_eb_ed),
        ThemeMode::Dark => gpui::rgb(0x3e_42_57),
    }
}

/// `--entity-item-background`: `--white` (light) / `--dark-grey-3` (dark).
fn entity_item_bg(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => theme.card_c,
        ThemeMode::Dark => theme.card_a,
    }
}

/// `--list-hover-hover`: `--light-grey-5` (light) / `--dark-grey-4` (dark).
fn entity_hover_fill(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => theme.card_b,
        ThemeMode::Dark => theme.card_c,
    }
}

/// `--list-hover`: `--light-grey-5` (light) / `--dark-grey-5` (dark).
fn list_hover_fill(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => theme.card_b,
        ThemeMode::Dark => theme.border_strong,
    }
}

/// `--list-select`: `--light-grey-4` (light) / `--dark-grey-4` (dark).
fn list_select_fill(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => theme.border_strong,
        ThemeMode::Dark => theme.card_c,
    }
}

/// `--button-regular-light` (the `regular` button fill):
/// `--light-grey-a20` / `--dark-grey-7-a20`.
fn button_regular_light(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Light => with_alpha(gpui::rgb(0x79_8c_94), 0.20),
        ThemeMode::Dark => with_alpha(gpui::rgb(0x8d_91_a5), 0.20),
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
                    .font_family(UI_FONT).text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .line_height(px(line_height_of(14.)))
                    .child(self.title),
            )
            .child(
                div()
                    .font_family(UI_FONT).text_size(px(12.))
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

/// Visual weight of a [`Button`] (Termius `Button$1` `color` prop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// `color:"accent"` — `--button-accent` fill; the primary action.
    Primary,
    /// `color:"regular"` — `--button-regular-light` fill; the default.
    Secondary,
    /// `color:"destructive"` — `--button-destructive` fill.
    Danger,
}

/// The Termius `Button$1` size ladder (`reconnectSaga:70114`–`70141`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    /// `height: 20px`, `padding: 2px 5px`, 12px, radius `--corner-radius-small`.
    Tiny,
    /// `height: 24px`, `padding: 0 10px`, 14px, `--corner-radius-small-increased`.
    Small,
    /// `height: 30px`, `padding: 11px 10px`, 14px, `--corner-radius-small-medium`.
    Medium,
    /// `height: 36px`, `padding: 0 16px`, 14px, `--corner-radius-medium`.
    Large,
}

impl ButtonSize {
    /// `(height, pad_x, font_size, line_height, radius)`.
    fn metrics(self) -> (f32, f32, f32, f32, f32) {
        match self {
            ButtonSize::Tiny => (20.0, 5.0, 12.0, 16.0, 5.0),
            ButtonSize::Small => (24.0, 10.0, 14.0, line_height_of(14.), 6.0),
            ButtonSize::Medium => (30.0, 10.0, 14.0, line_height_of(14.), 8.0),
            ButtonSize::Large => (BUTTON_HEIGHT, 16.0, 14.0, line_height_of(14.), 10.0),
        }
    }
}

/// A labeled button with a click listener slot.
pub struct Button {
    label: SharedString,
    variant: ButtonVariant,
    size: ButtonSize,
    ghost: bool,
    disabled: bool,
}

impl Button {
    /// A `regular` (quiet) `large` button.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            variant: ButtonVariant::Secondary,
            size: ButtonSize::Large,
            ghost: false,
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

    /// Set the size explicitly (defaults to [`ButtonSize::Large`]).
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// The `ghost` variant: transparent fill, coloured text
    /// (`--substrateColor: var(--buttonColor)` at `0.15`).
    pub fn ghost(mut self) -> Self {
        self.ghost = true;
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

    /// The current size.
    pub fn button_size(&self) -> ButtonSize {
        self.size
    }

    /// The themed button element (stateful, so `.on_click` can be chained).
    ///
    /// `Button$1`/`ButtonImpl` (`reconnectSaga:69982`): `display:flex`,
    /// centred, `color: var(--white)` (or the variant text colour), and the
    /// per-size metrics from [`ButtonSize`]. Hover paints the "substrate"
    /// overlay — `--substrateColor: var(--white)` (regular/accent/
    /// destructive) fading to `--substrateOpacity: 0.25`.
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let white = on_fill();
        let disabled = self.disabled;
        let (height, pad_x, font_size, line_height, radius) = self.size.metrics();
        let (variant_fill, variant_fg) = match self.variant {
            ButtonVariant::Primary => (theme.primary, white),
            ButtonVariant::Secondary => (button_regular_light(theme), theme.title),
            ButtonVariant::Danger => (theme.danger, white),
        };
        let (background, foreground, substrate, substrate_opacity) = if disabled {
            (dim_fill(theme), theme.text_common, white, 0.0)
        } else if self.ghost {
            (clear(), variant_fill, variant_fill, 0.15)
        } else {
            (variant_fill, variant_fg, white, 0.25)
        };
        // Termius' hover substrate composites over the button colour.
        let hover_fill = over(background, with_alpha(substrate, substrate_opacity));

        let button = div()
            .id(SharedString::from(format!("button-{}", self.label)))
            .flex()
            .items_center()
            .justify_center()
            .h(px(height))
            .px(px(pad_x))
            .rounded(px(radius))
            .bg(background)
            .text_color(foreground)
            .font_family(UI_FONT).text_size(px(font_size))
            .font_weight(FontWeight::MEDIUM)
            .line_height(px(line_height))
            .whitespace_nowrap()
            .child(self.label);

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
                .font_family(UI_FONT).text_size(px(14.))
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
            .font_family(UI_FONT).text_size(px(14.))
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
    /// `Switch$1` row: a 12px `--text-primary` label left, the MUI switch
    /// right, and the optional description below. The pill follows MUI's
    /// `MuiSwitch` geometry (40×14 track, `borderRadius: 7`, 20px thumb that
    /// overhangs by 3px top/bottom) with Termius' colours: checked =
    /// `--button-accent` (`--blue`), unchecked track = `--light-grey-4`
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
                .font_family(UI_FONT).text_size(px(12.))
                .line_height(px(line_height_of(12.)))
                .text_color(theme.title) // `<Typography color="primary">`
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
                    .font_family(UI_FONT).text_size(px(12.))
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
    /// adds the row fills: `selected` → `--list-select`, hover → `--list-hover`,
    /// `divider` → `--border-light`.
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let id = self
            .id
            .unwrap_or_else(|| SharedString::from(format!("list-item-{}", self.label)));
        // `--list-select` stays put on hover; unselected rows wash to
        // `--list-hover`.
        let hover_fill = if self.selected {
            list_select_fill(theme)
        } else {
            list_hover_fill(theme)
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
            row = row.bg(list_select_fill(theme));
        } else if self.divider {
            row = row.border_b_1().border_color(theme.border_light);
        }

        row = row.child(
            div()
                .font_family(UI_FONT).text_size(px(14.))
                .font_weight(body_weight())
                .line_height(px(line_height_of(14.)))
                .child(self.label),
        );
        if !self.subtitle.is_empty() {
            row = row.child(
                div()
                    .font_family(UI_FONT).text_size(px(12.))
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
            .font_family(UI_FONT).text_size(px(12.))
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
            .font_family(UI_FONT).text_size(px(14.))
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
            .font_family(UI_FONT).text_size(px(12.))
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
                .font_family(UI_FONT).text_size(px(14.))
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
    on_close: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_back: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl DialogFrame {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            children: Vec::new(),
            actions: Vec::new(),
            on_close: None,
            on_back: None,
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

    /// Attach the `close` affordance (the original `Bar` end-adornment /
    /// `CloseAndBackDialogSection` close button).
    pub fn on_close(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Box::new(listener));
        self
    }

    /// Attach the `back` affordance (the original `CloseAndBackDialogSection`
    /// back arrow, shown top-left).
    pub fn on_back(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_back = Some(Box::new(listener));
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
    /// `DialogPanel-20312ab7.js`: `justifyContent: space-between` split into
    /// `dialogContent` / `dialogActions`. The card fill is `var(--foreground)`
    /// (`--card-a` in both themes), the radius is
    /// `--corner-radius-large-increased` (20px), content padding is `40px`
    /// (`20px` bottom when an action row follows), and the action row is
    /// `20px` with a `1px` top rule in `--card-c` (dark) / `--card-b` (light).
    /// The close/back affordances follow `CloseAndBackDialogSection`
    /// (`color: --light-grey-1`, hover `--dark-grey-1`/`--white`). The
    /// `--backdrop` scrim belongs to the caller; gpui 0.2.2 has no ARIA
    /// attributes, so `role="dialog"` cannot be emitted.
    pub fn element(self, theme: TermiusTheme) -> Div {
        let has_actions = !self.actions.is_empty();
        let id_base = SharedString::from(format!("dialog-{}", self.title));

        let mut card = div()
            .flex()
            .flex_col()
            .w(px(DIALOG_WIDTH))
            .rounded(px(DIALOG_RADIUS))
            .overflow_hidden()
            .bg(theme.card_a)
            .text_color(theme.title);

        // Title bar (the original `Bar`/`CloseAndBackDialogSection` header).
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(40.))
            .pt(px(40.))
            .pb(px(10.));
        if let Some(on_back) = self.on_back {
            header = header.child(dialog_icon_button(
                SharedString::from(format!("{id_base}-back")),
                "back.svg",
                16.,
                12.,
                theme,
                on_back,
            ));
        }
        header = header.child(
            div()
                .flex_grow()
                .min_w(px(0.))
                .truncate()
                .font_family(UI_FONT).text_size(px(14.))
                .font_weight(FontWeight::BOLD)
                .line_height(px(line_height_of(14.)))
                .child(self.title),
        );
        match self.on_close {
            Some(on_close) => {
                header = header.child(dialog_icon_button(
                    SharedString::from(format!("{id_base}-close")),
                    "close20x20.svg",
                    14.,
                    14.,
                    theme,
                    on_close,
                ));
            }
            // Always render the affordance for parity even when unwired.
            None => {
                header = header.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .text_color(theme.text_common)
                        .child(icon("close20x20.svg").w(px(14.)).h(px(14.))),
                );
            }
        }
        card = card.child(header);

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

/// A 24×24 ghost icon button used by the dialog chrome (close/back).
fn dialog_icon_button(
    id: SharedString,
    icon_name: &'static str,
    w: f32,
    h: f32,
    theme: TermiusTheme,
    listener: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.))
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.text_common)
        .child(icon(icon_name).w(px(w)).h(px(h)))
        .hover(move |style| style.bg(theme.border_light).text_color(theme.title))
        .on_click(listener)
}

// ---------------------------------------------------------------------------
// ShapedIcon
// ---------------------------------------------------------------------------

/// The 40×40 rounded entity icon tile (`ShapedIcon`, `reconnectSaga:88420`).
///
/// `.shape { width:40px; height:40px; border-radius:12px; display:flex;
/// justify-content:center; align-items:center; background-color:
/// var(--dark-blue-solid); color:white; font-size:24px; }`
pub struct ShapedIcon {
    icon_name: SharedString,
    size: f32,
    glyph_size: f32,
    radius: f32,
    background: Option<Rgba>,
}

impl ShapedIcon {
    pub fn new(icon_name: impl Into<SharedString>) -> Self {
        Self {
            icon_name: icon_name.into(),
            size: SHAPED_ICON_SIZE,
            glyph_size: SHAPED_ICON_GLYPH,
            radius: SHAPED_ICON_RADIUS,
            background: None,
        }
    }

    /// The tile's square size (default 40).
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// The glyph size inside the tile (default 24).
    pub fn glyph_size(mut self, glyph_size: f32) -> Self {
        self.glyph_size = glyph_size;
        self
    }

    /// The tile's corner radius (default 12).
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Override the tile fill (the icon registry's per-icon colour); defaults
    /// to `--dark-blue-solid`.
    pub fn background(mut self, color: Rgba) -> Self {
        self.background = Some(color);
        self
    }

    /// The themed tile element: rounded square, white centred glyph.
    pub fn element(self, theme: TermiusTheme) -> Div {
        let _ = theme; // fill is the reconstructed `--dark-blue-solid`
        let background = self.background.unwrap_or_else(dark_blue_solid);
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(self.size))
            .rounded(px(self.radius))
            .bg(background)
            .text_color(on_fill())
            .child(icon(self.icon_name.as_ref()).w(px(self.glyph_size)).h(px(self.glyph_size)))
    }
}

// ---------------------------------------------------------------------------
// NavItem
// ---------------------------------------------------------------------------

/// One left-panel section row (`aFe`, `_main.js:111361`; styles `sFe`).
///
/// `.root { width: calc(100% - 20px); padding: 0 10px 0 7px; height: 36px;
/// display:flex; align-items:center; border-radius: var(--corner-radius-medium);
/// margin: 8px 10px; }` with `--background-hover-color` /
/// `--background-selected-color` fills and a 14px label.
pub struct NavItem {
    label: SharedString,
    icon_name: SharedString,
    selected: bool,
}

impl NavItem {
    pub fn new(
        label: impl Into<SharedString>,
        icon_name: impl Into<SharedString>,
        selected: bool,
    ) -> Self {
        Self {
            label: label.into(),
            icon_name: icon_name.into(),
            selected,
        }
    }

    /// Highlight the row as the active section.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether the row renders as the active section.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// The themed row (stateful: chain `.on_click` to route).
    ///
    /// The row fills the rail width minus its `10px` margins (via flexbox
    /// stretch, i.e. the original `calc(100% - 20px)`); `selected` uses
    /// `--background-selected-color`, hover `--background-hover-color`.
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let id = SharedString::from(format!("nav-item-{}", self.label));
        let text = theme.title;
        let (fill, hover_fill) = if self.selected {
            (nav_selected_fill(theme), nav_selected_fill(theme))
        } else {
            (clear(), nav_hover_fill(theme))
        };

        div()
            .id(id)
            .flex()
            .items_center()
            .h(px(NAV_ITEM_HEIGHT))
            .my(px(8.))
            .mx(px(10.))
            .pl(px(7.))
            .pr(px(10.))
            .rounded(px(theme.corner_radius_medium))
            .bg(fill)
            .text_color(text)
            .child(icon(self.icon_name.as_ref()).w(px(18.)).h(px(18.)).text_color(text))
            .child(
                div()
                    .ml(px(10.))
                    .flex_grow()
                    .min_w(px(0.))
                    .truncate()
                    .font_family(UI_FONT).text_size(px(14.))
                    .font_weight(body_weight())
                    .line_height(px(line_height_of(14.)))
                    .child(self.label),
            )
            .hover(move |style| style.bg(hover_fill))
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
// EntityRow
// ---------------------------------------------------------------------------

/// One entity list row (`GridItemPresenter` / `EntityItem` / `ItemLayout`,
/// `reconnectSaga:131214` / `102710` / `102669`).
///
/// `.entityItem { padding:10px; border-radius:14px; border:2px solid
/// var(--entity-item-background); background: var(--entity-item-background); }`
/// `.entityItem:hover:not(.selected) { background/border:
/// var(--list-hover-hover); }` `.selected { border-color: var(--border-accent); }`
/// title 14px `--text-primary`, subtitle 11px `--text-secondary`, leading
/// 40×40 [`ShapedIcon`], trailing ⋯ menu (`dots.svg`).
pub struct EntityRow {
    title: SharedString,
    subtitle: SharedString,
    icon_name: SharedString,
    selected: bool,
}

impl EntityRow {
    pub fn new(
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        icon_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            icon_name: icon_name.into(),
            selected: false,
        }
    }

    /// Highlight the row as selected (`border-color: var(--border-accent)`).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether the row renders as selected.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// The themed row (stateful: chain `.on_click` to open/select).
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let id = SharedString::from(format!("entity-row-{}", self.title));
        let base = entity_item_bg(theme);
        let hover_fill = entity_hover_fill(theme);
        let border = if self.selected {
            theme.border_accent
        } else {
            base
        };

        let icon_el = ShapedIcon::new(self.icon_name).element(theme).mr(px(10.));

        let mut title_group = div()
            .flex()
            .flex_col()
            .flex_grow()
            .min_w(px(0.))
            .overflow_hidden()
            .child(
                div()
                    .truncate()
                    .font_family(UI_FONT).text_size(px(14.))
                    .font_weight(body_weight())
                    .line_height(px(line_height_of(14.)))
                    .text_color(theme.title)
                    .child(self.title),
            );
        if !self.subtitle.is_empty() {
            title_group = title_group.child(
                div()
                    .truncate()
                    .font_family(UI_FONT).text_size(px(11.))
                    .font_weight(body_weight())
                    .line_height(px(14.))
                    .text_color(theme.text_common)
                    .child(self.subtitle),
            );
        }

        let end_adornment = div()
            .flex()
            .items_center()
            .child(icon("dots.svg").w(px(16.)).h(px(4.)).text_color(theme.text_common));

        div()
            .id(id)
            .flex()
            .items_center()
            .mx(px(5.))
            .p(px(10.))
            .rounded(px(ENTITY_RADIUS))
            .border_2()
            .border_color(border)
            .bg(base)
            .text_color(theme.title)
            .child(icon_el)
            .child(title_group)
            .child(end_adornment)
            .hover(move |style| style.bg(hover_fill).border_color(hover_fill))
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
// FiltersHeader
// ---------------------------------------------------------------------------

/// The shared search/filter band every screen mounts (`FiltersHeader`,
/// `reconnectSaga:124978`; `HostsFiltersHeader-fc79316f.js`).
///
/// `.header { background: var(--surface-high); display:flex; padding: 5px 12px;
/// padding-top: 4px; height: 45px; align-items:center; gap:15px; }` with the
/// primary action as its **first child** (no separate title band) and a
/// search field + tags/sort buttons on the right.
pub struct FiltersHeader {
    title: SharedString,
    action: Option<AnyElement>,
}

impl FiltersHeader {
    /// `title` is the search field's placeholder text.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            action: None,
        }
    }

    /// The primary action, rendered as the band's first child (left).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    /// The themed band element.
    pub fn element(self, theme: TermiusTheme) -> Div {
        let id_base = SharedString::from(format!("filters-{}", self.title));
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(15.))
            .h(px(FILTERS_HEADER_HEIGHT))
            .px(px(12.))
            .pt(px(4.))
            .pb(px(5.))
            .bg(theme.card_a) // --surface-high (card_a in both themes)
            .text_color(theme.title);
        if let Some(action) = self.action {
            header = header.child(action);
        }

        let search = div()
            .id(SharedString::from(format!("{id_base}-search")))
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(30.))
            .px(px(10.))
            .min_w(px(140.))
            .rounded(px(theme.corner_radius_small))
            .bg(theme.card_b)
            .text_color(theme.text_common)
            .child(icon("search.svg").w(px(12.)).h(px(12.)))
            .child(
                div()
                    .truncate()
                    .font_family(UI_FONT).text_size(px(12.))
                    .font_weight(body_weight())
                    .line_height(px(line_height_of(12.)))
                    .child(self.title),
            );

        let tags = filter_button(SharedString::from(format!("{id_base}-tags")), "tags.svg", theme);
        let sort = filter_button(
            SharedString::from(format!("{id_base}-sort")),
            "sorting.svg",
            theme,
        );

        header.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(10.))
                .flex_grow()
                .min_w(px(0.))
                .child(search)
                .child(tags)
                .child(sort),
        )
    }
}

/// A 30×30 filter icon button on the [`FiltersHeader`] band.
fn filter_button(id: SharedString, icon_name: &'static str, theme: TermiusTheme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(30.))
        .rounded(px(6.))
        .text_color(theme.text_common)
        .child(icon(icon_name).w(px(15.)).h(px(15.)))
        .hover(move |style| style.bg(theme.border_light))
}

// ---------------------------------------------------------------------------
// TabChip
// ---------------------------------------------------------------------------

/// One top-strip tab (`zN` / `F7`, `_main.js:106407` / `106568`).
///
/// `.root { min-height:30px; padding:0 10px 0 5px; display:flex;
/// align-items:center; border-radius: var(--corner-radius-medium); }`; the
/// 20px leading icon is swapped for a 23×23 close button when the tab is
/// selected/closable. The shell wires the close through [`TabChip::on_close`].
pub struct TabChip {
    title: SharedString,
    icon_name: SharedString,
    selected: bool,
    closable: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_close: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl TabChip {
    pub fn new(
        title: impl Into<SharedString>,
        icon_name: impl Into<SharedString>,
        selected: bool,
        closable: bool,
    ) -> Self {
        Self {
            title: title.into(),
            icon_name: icon_name.into(),
            selected,
            closable,
            on_click: None,
            on_close: None,
        }
    }

    /// Attach the close-button listener (builder form; keep chaining to
    /// [`TabChip::element`]).
    pub fn on_close(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Box::new(listener));
        self
    }

    /// Attach the tab-select listener (builder form).
    pub fn on_click(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(listener));
        self
    }

    /// The themed tab element.
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let id = SharedString::from(format!("tab-chip-{}", self.title));
        let selected = self.selected;
        let closable = self.closable;
        let fill = if selected { theme.tab_active } else { clear() };
        let hover = theme.hover;

        let mut close_button = if closable {
            Some(tab_close_button(theme, self.on_close, self.title.clone()))
        } else {
            None
        };

        let mut chip = div()
            .id(id)
            .flex()
            .items_center()
            .h(px(TAB_HEIGHT))
            .min_h(px(TAB_HEIGHT))
            .pl(px(5.))
            .pr(px(10.))
            .rounded(px(theme.corner_radius_medium))
            .bg(fill)
            .text_color(theme.title)
            .whitespace_nowrap()
            .hover(move |style| style.bg(hover));

        // Leading slot: the close button replaces the icon when selected.
        if closable && selected {
            if let Some(button) = close_button.take() {
                chip = chip.child(button);
            }
        } else {
            chip = chip.child(
                icon(self.icon_name.as_ref())
                    .w(px(14.))
                    .h(px(14.))
                    .text_color(theme.title),
            );
        }
        chip = chip.child(
            div()
                .ml(px(5.))
                .flex_grow()
                .min_w(px(0.))
                .truncate()
                .font_family(UI_FONT).text_size(px(14.))
                .font_weight(body_weight())
                .line_height(px(line_height_of(14.)))
                .child(self.title.clone()),
        );
        // Trailing close for a closable-but-unselected tab.
        if closable && !selected {
            if let Some(button) = close_button.take() {
                chip = chip.child(button);
            }
        }
        if let Some(on_click) = self.on_click {
            chip = chip.on_click(on_click);
        }
        chip
    }
}

/// The 23×23 tab close button (`F7.tabActionButton`).
fn tab_close_button(
    theme: TermiusTheme,
    listener: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    title: SharedString,
) -> Stateful<Div> {
    let button = div()
        .id(SharedString::from(format!("tab-close-{title}")))
        .flex()
        .items_center()
        .justify_center()
        .size(px(23.))
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.title)
        .child(icon("close__10755b.svg").w(px(10.)).h(px(10.)))
        .hover(move |style| style.bg(theme.card_c));
    match listener {
        Some(listener) => button.on_click(listener),
        None => button,
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
    fn button_size_ladder_matches_the_original() {
        assert_eq!(Button::new("A").button_size(), ButtonSize::Large);
        assert_eq!(Button::new("A").size(ButtonSize::Tiny).button_size(), ButtonSize::Tiny);
        // large = 36px tall, 16px inline padding, 14px, radius 10.
        let (h, pad, font, _line, radius) = ButtonSize::Large.metrics();
        assert_eq!((h, pad, font, radius), (36.0, 16.0, 14.0, 10.0));
        let (h, pad, font, _line, radius) = ButtonSize::Tiny.metrics();
        assert_eq!((h, pad, font, radius), (20.0, 5.0, 12.0, 5.0));
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
    fn new_primitives_round_trip_their_state() {
        assert!(NavItem::new("Hosts", "Hosts.svg", true).is_selected());
        assert!(!NavItem::new("Logs", "session-log.svg", false).is_selected());
        assert!(NavItem::new("Logs", "session-log.svg", false).selected(true).is_selected());

        assert!(EntityRow::new("web-1", "10.0.0.1", "Apple.svg").selected(true).is_selected());
        assert!(!EntityRow::new("web-1", "", "Apple.svg").is_selected());

        let shaped = ShapedIcon::new("Apple.svg").size(26.).glyph_size(14.);
        assert_eq!(shaped.size, 26.);
        assert_eq!(shaped.glyph_size, 14.);
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
        let _ = Button::new("Ghost").ghost().size(ButtonSize::Small).element(theme);
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
        let _ = ShapedIcon::new("Apple.svg").element(theme);
        let _ = NavItem::new("Hosts", "Hosts.svg", true).element(theme);
        let _ = EntityRow::new("web-1", "10.0.0.1", "Apple.svg").element(theme);
        let _ = FiltersHeader::new("Search hosts")
            .action(Button::new("New host").primary().element(theme))
            .element(theme);
        let _ = TabChip::new("web-1", "Sftp.svg", true, true).element(theme);
    }
}
