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
//! PORT-TODO: `InputField` renders its value as static text; real text editing
//! needs a focused content model (gpui `Content`/`InputEvent` on the tracked
//! [`FocusHandle`]) — wire it when the Add-Host form lands.

use gpui::{
    div, px, App, ClickEvent, Div, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window,
};

use crate::theme::{over, TermiusTheme};

/// Corner radius used by cards, rows and controls.
const CORNER: f32 = 4.0;
/// Default button height.
const BUTTON_HEIGHT: f32 = 28.0;
/// List / settings row height.
const ROW_HEIGHT: f32 = 28.0;
/// Settings section header height.
const SECTION_HEADER_HEIGHT: f32 = 34.0;
/// Input box height.
const INPUT_HEIGHT: f32 = 28.0;
/// Dialog card width.
const DIALOG_WIDTH: f32 = 460.0;
/// White used for text/knobs on accent or danger fills.
fn on_fill() -> gpui::Rgba {
    gpui::rgb(0xff_ff_ff)
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
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .size_full()
            .text_color(theme.foreground)
            .child(div().text_lg().child(self.title))
            .child(div().text_sm().text_color(theme.muted).child(self.subtitle))
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
}

impl Button {
    /// A secondary (quiet) button.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            variant: ButtonVariant::Secondary,
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

    /// The current variant.
    pub fn variant(&self) -> ButtonVariant {
        self.variant
    }

    /// The themed button element (stateful, so `.on_click` can be chained).
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let (background, foreground) = match self.variant {
            ButtonVariant::Primary => (theme.accent, on_fill()),
            ButtonVariant::Secondary => (theme.tab_active, theme.foreground),
            ButtonVariant::Danger => (theme.danger, on_fill()),
        };
        div()
            .id(SharedString::from(format!("button-{}", self.label)))
            .flex()
            .items_center()
            .justify_center()
            .h(px(BUTTON_HEIGHT))
            .px(px(12.))
            .rounded(px(CORNER))
            .bg(background)
            .text_color(foreground)
            .text_sm()
            .child(self.label)
    }

    /// The themed button element with a gpui click listener attached.
    ///
    /// `listener` is the shape `cx.listener(|this, event, window, cx| …)`
    /// produces, so views pass their own listener unchanged.
    pub fn on_click(
        self,
        theme: TermiusTheme,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        self.element(theme).on_click(listener)
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
    pub fn element(self, theme: TermiusTheme) -> Div {
        let empty = self.value.is_empty();
        let text_color = if empty { theme.muted } else { theme.foreground };
        // Read before `self.label` is moved out below.
        let shown = self.shown_text();

        let mut field = div().flex().flex_col().gap(px(4.));
        field = field.child(div().text_xs().text_color(theme.muted).child(self.label));

        let mut box_el = div()
            .flex()
            .items_center()
            .h(px(INPUT_HEIGHT))
            .px(px(10.))
            .rounded(px(CORNER))
            .border_1()
            .border_color(theme.border)
            .bg(theme.tab_background);
        box_el = box_el.child(div().text_sm().text_color(text_color).child(shown));
        if let Some(focus) = self.focus.as_ref() {
            box_el = box_el.track_focus(focus);
        }
        field.child(box_el)
    }
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

/// A settings toggle row: label on the left, pill on the right.
pub struct Switch {
    label: SharedString,
    on: bool,
}

impl Switch {
    pub fn new(label: impl Into<SharedString>, on: bool) -> Self {
        Self {
            label: label.into(),
            on,
        }
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
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let pill_background = if self.on { theme.accent } else { theme.tab_active };
        let knob = div().w(px(14.)).h(px(14.)).rounded(px(7.)).bg(on_fill());
        let pill = div()
            .flex()
            .items_center()
            .px(px(2.))
            .w(px(34.))
            .h(px(18.))
            .rounded(px(9.))
            .bg(pill_background)
            .child(knob);
        let pill = if self.on { pill.justify_end() } else { pill.justify_start() };

        div()
            .id(SharedString::from(format!("switch-{}", self.label)))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .h(px(ROW_HEIGHT))
            .px(px(10.))
            .rounded(px(CORNER))
            .text_color(theme.foreground)
            .child(div().text_sm().child(self.label))
            .child(pill)
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
    id: Option<SharedString>,
}

impl ListItem {
    pub fn new(label: impl Into<SharedString>, subtitle: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            subtitle: subtitle.into(),
            selected: false,
            id: None,
        }
    }

    /// Highlight the row as selected.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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
    pub fn element(self, theme: TermiusTheme) -> Stateful<Div> {
        let id = self
            .id
            .unwrap_or_else(|| SharedString::from(format!("list-item-{}", self.label)));
        let mut row = div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .h(px(ROW_HEIGHT))
            .px(px(10.))
            .rounded(px(CORNER))
            .text_color(theme.foreground);
        if self.selected {
            row = row.bg(over(theme.tab_active, theme.selection));
        }
        row = row.child(div().text_sm().child(self.label));
        if !self.subtitle.is_empty() {
            row = row.child(div().text_xs().text_color(theme.muted).child(self.subtitle));
        }
        row
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

    /// The header row element.
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .flex()
            .items_center()
            .h(px(SECTION_HEADER_HEIGHT))
            .px(px(12.))
            .text_xs()
            .whitespace_nowrap()
            .text_color(theme.muted)
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

    /// The title element.
    pub fn element(self, theme: TermiusTheme) -> Div {
        div().text_lg().text_color(theme.foreground).child(self.text)
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

    /// The paragraph element.
    pub fn element(self, theme: TermiusTheme) -> Div {
        div()
            .text_sm()
            .whitespace_normal()
            .text_color(theme.muted)
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
    pub fn element(self, theme: TermiusTheme) -> Div {
        let mut section = div()
            .flex()
            .flex_col()
            .rounded(px(CORNER))
            .border_1()
            .border_color(theme.border)
            .bg(theme.sidebar_background);
        section = section.child(
            div()
                .flex()
                .items_center()
                .h(px(SECTION_HEADER_HEIGHT))
                .px(px(12.))
                .border_b_1()
                .border_color(theme.border)
                .text_sm()
                .text_color(theme.foreground)
                .child(self.title),
        );
        let mut body = div().flex().flex_col();
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
    pub fn element(self, theme: TermiusTheme) -> Div {
        let mut card = div()
            .flex()
            .flex_col()
            .w(px(DIALOG_WIDTH))
            .rounded(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(theme.border)
            .bg(theme.sidebar_background)
            .text_color(theme.foreground);

        card = card.child(
            div()
                .flex()
                .items_center()
                .h(px(SECTION_HEADER_HEIGHT + 8.))
                .px(px(16.))
                .border_b_1()
                .border_color(theme.border)
                .text_lg()
                .child(self.title),
        );

        let mut body = div().flex().flex_col().gap(px(10.)).p(px(16.));
        for child in self.children {
            body = body.child(child);
        }
        card = card.child(body);

        if !self.actions.is_empty() {
            let mut row = div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .px(px(16.))
                .py(px(12.))
                .border_t_1()
                .border_color(theme.border);
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
    }

    #[test]
    fn list_item_selection_and_ids() {
        assert!(!ListItem::new("web-1", "10.0.0.1").is_selected());
        assert!(ListItem::new("web-1", "").selected(true).is_selected());
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
        let _ = InputField::new("Label", "value").placeholder("…").element(theme);
        let _ = Switch::new("Cursor blink", true).element(theme);
        let _ = ListItem::new("row", "sub").selected(true).element(theme);
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
