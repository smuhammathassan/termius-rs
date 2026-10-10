//! host_list — the Hosts panel: Quick-Connect bar, New-Entity toolbar, the
//! two-section (Groups / Hosts) entity list, and the empty state.
//!
//! Rebuilt from `analysis/recon/10-hosts.md`. The routed screen is
//! `ConnectedHostsList` (`MD`, `_main.js:66368`); its list is
//! `FilteredVirtualizedHosts` → `VirtualizedHostsInternal`
//! (`reconnectSaga:134471`), which **flattens** groups and hosts into two
//! ordered `EntityLists` sections (`reconnectSaga:134645`) — it is *not* a
//! nested DOM tree:
//!
//! * **Quick-Connect bar** (`p6e`, `_main.js:66018`) — the `ssh user@hostname`
//!   input + Connect, sitting above the header (`div.quickConnectHotKeys`).
//! * **Header** (`HostsFiltersHeader-fc79316f.js:423`) — the shared 45px
//!   `--surface-high` band. Its **first child** is the New-Entity toolbar
//!   (`r6e`, `_main.js:65858`): the split **New host** button (+ chevron menu:
//!   New Group / Import / AWS / DigitalOcean / Azure), the **Terminal** and
//!   **Serial** ghost buttons, then the responsive search / tags / sort cluster.
//! * **List** — the group **breadcrumb** (`GroupPath`, `reconnectSaga:126512`)
//!   then the flattened `Groups` and `Hosts` sections. Rows use the shared
//!   [`EntityRow`] (40×40 `ShapedIcon` tile, 14px title over 11px subtitle,
//!   trailing ⋯). Group rows carry the `"{n} Host(s)"` count
//!   (`GroupPresenter.description`, `reconnectSaga:74206`) and **drill in** on
//!   click (breadcrumb navigation, *not* inline collapse). Host rows select on
//!   click and connect on double-click.
//! * **Empty state** — the centred "Create hosts" card with the 60×60 gradient
//!   icon wrapper (`EmptyScreenCard` / `IconWrapper`, `reconnectSaga:103728`).
//!
//! The legacy [`build_tree`]/[`TreeRow`] DFS flatten is kept intact for
//! compatibility (and its tests); the live list uses [`flatten_entities`] /
//! [`ListRow`]. Keyboard navigation (↑/↓/Enter) walks host rows only, now over
//! the flattened list.
//!
//! # PORT-TODOs
//!
//! * No text input exists in gpui 0.2.2, and `TermiusState` has no
//!   search / quick-connect field, so the header search box and the
//!   Quick-Connect field are display-only (the Connect button is therefore
//!   disabled, matching the original's empty-address state).
//! * The `Terminal` / `Serial` buttons connect to the first saved host of the
//!   matching [`HostType`]; spawning a bare local / serial terminal has no
//!   `TermiusState` API yet.
//! * Host icons key off [`HostType`] because `termius_core::Host` has no
//!   `os_name` field yet — see [`os_icon`].

use std::collections::HashSet;

use gpui::{
    div, linear_color_stop, linear_gradient, px, radians, ClickEvent, Context, Div, Entity,
    FocusHandle, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Transformation,
    Window,
};
use termius_core::host::HostType;
use termius_core::{Group, Host};

use crate::app_state::{Dialog, TermiusState};
use crate::primitives::{Button, ButtonSize, EntityRow, FiltersHeader};
use crate::theme::{over, text, theme_of, with_alpha, TermiusTheme, ThemeMode, UI_FONT};

// --- layout metrics (from the original CSS) --------------------------------

/// The header band height (`HostsFiltersHeader` `height: 45px`).
const HEADER_HEIGHT: f32 = 45.0;
/// Quick-Connect bar: `padding: 8px` around a `36px` content row.
const QUICK_CONNECT_HEIGHT: f32 = 52.0;
/// `ButtonImpl.large` — the toolbar button height.
const BUTTON_HEIGHT: f32 = 36.0;
/// `ButtonImpl.medium` — the Terminal / Serial ghost buttons.
const BUTTON_MEDIUM_HEIGHT: f32 = 30.0;
/// Cycle/depth guard for malformed group graphs.
const MAX_DEPTH: usize = 16;

/// White, for text/glyphs on accent fills and the gradient stops.
fn white() -> gpui::Rgba {
    gpui::rgb(0xff_ff_ff)
}

/// The click count of a gpui [`ClickEvent`] (keyboard activations count as 1).
fn click_count(event: &ClickEvent) -> usize {
    match event {
        ClickEvent::Mouse(mouse) => mouse.up.click_count,
        ClickEvent::Keyboard(_) => 1,
    }
}

// ---------------------------------------------------------------------------
// Legacy DFS flatten (kept for compatibility with `views`/tests)
// ---------------------------------------------------------------------------

/// One flattened sidebar row of the legacy DFS tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Group { id: String, title: String, depth: usize },
    Host { id: String, label: String, hostname: String, depth: usize },
}

impl TreeRow {
    /// The row's tree depth (indent level).
    fn depth(&self) -> usize {
        match self {
            Self::Group { depth, .. } | Self::Host { depth, .. } => *depth,
        }
    }
}

/// Depth-first flatten of groups + hosts: each group lists its member hosts
/// (first claim wins), then its child groups; hosts outside every group are
/// appended at the end.
///
/// Cycles are neutralized by a visit stack plus [`MAX_DEPTH`].
pub fn build_tree(groups: &[Group], hosts: &[Host]) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut visiting: HashSet<String> = HashSet::new();

    let mut roots: Vec<&Group> = groups.iter().filter(|group| group.parent_id.is_none()).collect();
    roots.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.title.cmp(&b.title)));

    for group in roots {
        emit_group(group, groups, hosts, 0, &mut rows, &mut claimed, &mut visiting);
    }

    // Hosts claimed by no group (or only by unreachable/cyclic groups).
    let mut orphans: Vec<&Host> = hosts
        .iter()
        .filter(|host| !claimed.contains(&host.id))
        .collect();
    orphans.sort_by(|a, b| a.label.cmp(&b.label));
    for host in orphans {
        rows.push(host_row(host, 0));
        claimed.insert(host.id.clone());
    }
    rows
}

fn emit_group(
    group: &Group,
    groups: &[Group],
    hosts: &[Host],
    depth: usize,
    rows: &mut Vec<TreeRow>,
    claimed: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
) {
    if depth > MAX_DEPTH || visiting.contains(&group.id) {
        return;
    }
    visiting.insert(group.id.clone());
    rows.push(TreeRow::Group {
        id: group.id.clone(),
        title: group.title.clone(),
        depth,
    });

    let mut members: Vec<&Host> = hosts
        .iter()
        .filter(|host| host.group_ids.contains(&group.id) && !claimed.contains(&host.id))
        .collect();
    members.sort_by(|a, b| a.label.cmp(&b.label));
    for host in members {
        rows.push(host_row(host, depth + 1));
        claimed.insert(host.id.clone());
    }

    let mut children: Vec<&Group> = groups
        .iter()
        .filter(|candidate| candidate.parent_id.as_deref() == Some(group.id.as_str()))
        .collect();
    children.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.title.cmp(&b.title)));
    for child in children {
        emit_group(child, groups, hosts, depth + 1, rows, claimed, visiting);
    }

    visiting.remove(&group.id);
}

fn host_row(host: &Host, depth: usize) -> TreeRow {
    TreeRow::Host {
        id: host.id.clone(),
        label: host.label.clone(),
        hostname: host.hostname.clone(),
        depth,
    }
}

/// Drop the descendants of every collapsed group (legacy collapse model).
///
/// Kept for compatibility with the tree tests; the live screen uses the
/// breadcrumb drill-in of [`flatten_entities`] instead.
#[allow(dead_code)]
fn visible_rows(rows: &[TreeRow], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut visible = Vec::with_capacity(rows.len());
    // Depth of the innermost collapsed group we are currently inside, if any.
    let mut hidden_below: Option<usize> = None;
    for row in rows {
        let depth = row.depth();
        if let Some(closed_at) = hidden_below {
            if depth > closed_at {
                continue;
            }
            hidden_below = None;
        }
        if let TreeRow::Group { id, .. } = row {
            if collapsed.contains(id) {
                hidden_below = Some(depth);
            }
        }
        visible.push(row.clone());
    }
    visible
}

/// The next host row `dir` steps away (`+1` down, `-1` up) from `from`,
/// skipping group headers; `None` at either end.
///
/// Kept for compatibility with the tree tests; [`next_host_in_list`] is the
/// live keyboard-navigation helper.
#[allow(dead_code)]
pub(crate) fn next_host_row(rows: &[TreeRow], from: Option<usize>, dir: i32) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let len = rows.len() as i32;
    let start = match from {
        Some(index) => index as i32,
        None if dir >= 0 => -1,
        None => len,
    };
    let mut index = start + dir;
    while index >= 0 && index < len {
        if matches!(rows[index as usize], TreeRow::Host { .. }) {
            return Some(index as usize);
        }
        index += dir;
    }
    None
}

// ---------------------------------------------------------------------------
// Flattened two-section model (the live list)
// ---------------------------------------------------------------------------

/// One row of the flattened Hosts list: the original's `Groups` / `Hosts`
/// `EntityLists` (`reconnectSaga:134645`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRow {
    /// A child group of the current group (drills in on click).
    Group { id: String, title: String, host_count: usize },
    /// A host in the current group (or ungrouped at the root).
    Host { id: String, label: String, subtitle: String },
}

/// Flatten the child groups of `current_group` (or the root groups when
/// `None`) followed by its member hosts (or the ungrouped hosts at the root).
///
/// Ordering mirrors the original: groups by `sort_order` then title, hosts by
/// label. Each group row carries its direct host count
/// (`GroupPresenter.description`, `reconnectSaga:74206`).
pub fn flatten_entities(
    groups: &[Group],
    hosts: &[Host],
    current_group: Option<&str>,
) -> Vec<ListRow> {
    let mut rows = Vec::new();

    let mut children: Vec<&Group> = groups
        .iter()
        .filter(|group| group.parent_id.as_deref() == current_group)
        .collect();
    children.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.title.cmp(&b.title)));
    for group in children {
        let host_count = hosts
            .iter()
            .filter(|host| host.group_ids.contains(&group.id))
            .count();
        rows.push(ListRow::Group {
            id: group.id.clone(),
            title: group.title.clone(),
            host_count,
        });
    }

    let mut members: Vec<&Host> = hosts
        .iter()
        .filter(|host| match current_group {
            Some(group_id) => host.group_ids.iter().any(|id| id.as_str() == group_id),
            None => host.group_ids.is_empty(),
        })
        .collect();
    members.sort_by(|a, b| a.label.cmp(&b.label));
    for host in members {
        rows.push(ListRow::Host {
            id: host.id.clone(),
            label: host.label.clone(),
            subtitle: host_subtitle(host),
        });
    }

    rows
}

/// The breadcrumb trail from "All hosts" down to `group_id`
/// (`GroupPath`, `reconnectSaga:126512`).
///
/// The first element is the root crumb (`None`, `"All hosts"`); each following
/// element is `(group id, title)`. A malformed (cyclic) parent chain is cut at
/// [`MAX_DEPTH`].
pub fn group_path(groups: &[Group], group_id: Option<&str>) -> Vec<(Option<String>, String)> {
    let mut path = vec![(None, "All hosts".to_owned())];
    let Some(mut current) = group_id.map(str::to_owned) else {
        return path;
    };

    let mut chain: Vec<(Option<String>, String)> = Vec::new();
    let mut guard = 0usize;
    while let Some(group) = groups.iter().find(|group| group.id == current) {
        chain.push((Some(group.id.clone()), group.title.clone()));
        guard += 1;
        if guard > MAX_DEPTH {
            break;
        }
        match group.parent_id.clone() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    chain.reverse();
    path.extend(chain);
    path
}

/// The next host row `dir` steps away (`+1` down, `-1` up) from `from` in the
/// flattened list, skipping group rows; `None` at either end.
pub fn next_host_in_list(rows: &[ListRow], from: Option<usize>, dir: i32) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let len = rows.len() as i32;
    let start = match from {
        Some(index) => index as i32,
        None if dir >= 0 => -1,
        None => len,
    };
    let mut index = start + dir;
    while index >= 0 && index < len {
        if matches!(rows[index as usize], ListRow::Host { .. }) {
            return Some(index as usize);
        }
        index += dir;
    }
    None
}

// ---------------------------------------------------------------------------
// Presenters (icon + text)
// ---------------------------------------------------------------------------

/// The bundled platform glyph for a Termius `os_name`
/// (`HostPresenter.icon` → `EntityIcon`, `reconnectSaga:63048`).
///
/// PORT-TODO: `termius_core::Host` carries no `os_name` field yet, so nothing
/// calls this at runtime; the mapping lives here so it is wired in one place
/// once the field lands (see [`host_icon`]).
pub fn os_icon(os_name: &str) -> &'static str {
    match os_name.trim().to_ascii_lowercase().as_str() {
        "apple" | "macos" | "mac" | "osx" | "darwin" => "Apple.svg",
        "windows" | "win" | "windows10" | "windows11" => "Windows.svg",
        "ubuntu" => "ubuntu.svg",
        "linux" | "debian" | "centos" | "fedora" | "arch" | "redhat" => "linux.svg",
        _ => "host.svg",
    }
}

/// The bundled icon for a host row.
///
/// PORT-TODO: the original keys this off the host's `os_name` (`HostPresenter
/// .icon` → `Apple` / `Windows` / `linux` / `ubuntu`…, `reconnectSaga:63048`),
/// which the local [`Host`] model does not carry yet; fall back to the
/// transport [`HostType`] so every row still renders a real SVG instead of a
/// placeholder. Replace with [`os_icon`] when `Host::os_name` lands.
fn host_icon(host: &Host) -> &'static str {
    match host.host_type {
        HostType::Local => "Apple.svg",
        HostType::Serial => "serial.svg",
        HostType::Mosh => "mosh.svg",
        HostType::Ssh | HostType::Telnet => "host.svg",
    }
}

/// The dim second line of a host row: `user@host` when a login user is set,
/// else the address alone (empty addresses fall back to the label).
fn host_subtitle(host: &Host) -> String {
    let address = if host.hostname.trim().is_empty() {
        host.label.as_str()
    } else {
        host.hostname.as_str()
    };
    if host.username.trim().is_empty() {
        address.to_owned()
    } else {
        format!("{}@{address}", host.username)
    }
}

/// `GroupPresenter.description`: `"{n} Host"` / `"{n} Hosts"`
/// (`reconnectSaga:74206`).
fn group_subtitle(host_count: usize) -> String {
    format!("{host_count} Host{}", if host_count == 1 { "" } else { "s" })
}

// ---------------------------------------------------------------------------
// Small chrome builders
// ---------------------------------------------------------------------------

/// The `Groups` / `Hosts` section title
/// (`reconnectSaga:134728`: `font-weight:700; padding:30px 30px 9px`).
fn section_title(theme: TermiusTheme, title: &'static str) -> Div {
    div()
        .px(px(30.))
        .pt(px(30.))
        .pb(px(9.))
        .font_family(UI_FONT).text_size(px(14.))
        .font_weight(FontWeight::BOLD)
        .line_height(px(18.))
        .whitespace_nowrap()
        .text_color(theme.title)
        .child(SharedString::from(title))
}

/// The accent "New host" split-button segment (plus glyph + label).
fn primary_segment(theme: TermiusTheme, id: &'static str) -> Stateful<Div> {
    let hover = over(theme.primary, with_alpha(white(), 0.25));
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(BUTTON_HEIGHT))
        .pl(px(12.))
        .pr(px(10.))
        .rounded_l(px(theme.corner_radius_medium))
        .bg(theme.primary)
        .text_color(white())
        .font_family(UI_FONT).text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(21.))
        .whitespace_nowrap()
        .child(crate::icon("plusThin.svg").w(px(14.)).h(px(14.)).text_color(white()))
        .child(SharedString::from("New host"))
        .hover(move |style| style.bg(hover))
}

/// The split button's chevron segment.
fn chevron_segment(theme: TermiusTheme, id: &'static str, open: bool) -> Stateful<Div> {
    let hover = over(theme.primary, with_alpha(white(), 0.25));
    let fill = if open { hover } else { theme.primary };
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .h(px(BUTTON_HEIGHT))
        .w(px(30.))
        .rounded_r(px(theme.corner_radius_medium))
        .bg(fill)
        .text_color(white())
        .child(crate::icon("chevron.svg").w(px(12.)).h(px(12.)).text_color(white()))
        .hover(move |style| style.bg(hover))
}

/// A `ButtonImpl.medium` ghost button with a leading glyph (Terminal / Serial).
fn ghost_icon_button(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    label: &'static str,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(BUTTON_MEDIUM_HEIGHT))
        .px(px(10.))
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.title)
        .font_family(UI_FONT).text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(21.))
        .whitespace_nowrap()
        .child(crate::icon(icon_name).w(px(14.)).h(px(14.)).text_color(theme.title))
        .child(SharedString::from(label))
        .hover(move |style| style.bg(theme.hover))
}

/// The small "Pro" tag the integration menu items carry (`upgrade:!isPro`).
fn pro_badge(theme: TermiusTheme) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(with_alpha(theme.primary, 0.15))
        .text_color(theme.primary)
        .font_family(UI_FONT).text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .line_height(px(13.))
        .child(SharedString::from("Pro"))
}

/// One row of the split button's dropdown menu.
fn menu_item(
    theme: TermiusTheme,
    id: &'static str,
    icon_name: &'static str,
    label: &'static str,
    upgrade: bool,
) -> Stateful<Div> {
    let mut row = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(34.))
        .px(px(12.))
        .rounded(px(theme.corner_radius_small))
        .text_color(theme.title)
        .child(crate::icon(icon_name).w(px(16.)).h(px(16.)).text_color(theme.muted))
        .child(
            text::R14P
                .style(div())
                .flex_grow()
                .min_w(px(0.))
                .truncate()
                .child(SharedString::from(label)),
        )
        .hover(move |style| style.bg(theme.card_c));
    if upgrade {
        row = row.child(pro_badge(theme));
    }
    row
}

/// The 60×60 gradient `IconWrapper` of the empty-screen card
/// (`reconnectSaga:103707`: `linear-gradient(135deg, --dark-grey-*-a15, --white-a15)`).
fn empty_icon(theme: TermiusTheme) -> Div {
    let from = match theme.mode {
        ThemeMode::Dark => theme.card_b,
        ThemeMode::Light => theme.card_c,
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(60.))
        .flex_shrink_0()
        .rounded(px(16.))
        .border_1()
        .border_color(with_alpha(white(), 0.05))
        .bg(linear_gradient(
            135.0,
            linear_color_stop(with_alpha(from, 0.15), 0.0),
            linear_color_stop(with_alpha(white(), 0.15), 1.0),
        ))
        .text_color(theme.title)
        .child(crate::icon("host.svg").w(px(20.)).h(px(20.)))
}

/// `DefaultEmptyHostsScreen` (`reconnectSaga:126536`): the centred "Create
/// hosts" card.
fn empty_hosts_card(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .text_color(theme.title)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(30.))
                .px(px(20.))
                .child(empty_icon(theme))
                .child(
                    div()
                        .font_family(UI_FONT).text_size(px(16.))
                        .font_weight(FontWeight(500.0))
                        .line_height(px(21.))
                        .child(SharedString::from("Create hosts")),
                )
                .child(
                    div()
                        .max_w(px(300.))
                        .text_center()
                        .font_family(UI_FONT).text_size(px(14.))
                        .font_weight(FontWeight(450.0))
                        .line_height(px(21.))
                        .text_color(theme.text_common)
                        .child(SharedString::from(
                            "Save your connection details as hosts to connect in one click.",
                        )),
                ),
        )
}

/// The Quick-Connect bar (`p6e`, `_main.js:66018`): a `ssh user@hostname`
/// field + Connect, above the header.
fn quick_connect_bar(theme: TermiusTheme) -> Div {
    div()
        .flex()
        .items_center()
        .w_full()
        .flex_shrink_0()
        .p(px(8.))
        .h(px(QUICK_CONNECT_HEIGHT))
        .bg(theme.card_a)
        .child(
            div()
                .flex()
                .items_center()
                .flex_1()
                .min_w(px(0.))
                .h(px(36.))
                .pr(px(7.))
                .rounded(px(theme.corner_radius_medium))
                .border_1()
                .border_color(theme.border_light)
                .bg(theme.card_b)
                // PORT-TODO: display-only — gpui 0.2.2 has no text input and
                // `TermiusState` has no quick-connect field.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .px(px(10.))
                        .truncate()
                        .text_color(theme.text_common)
                        .font_family(UI_FONT).text_size(px(14.))
                        .font_weight(FontWeight(450.0))
                        .line_height(px(21.))
                        .child(SharedString::from(
                            "Type \"ssh user@hostname -p port\" to connect...",
                        )),
                )
                .child(
                    Button::new("Connect")
                        .primary()
                        .size(ButtonSize::Small)
                        // The address is empty, so the original disables it.
                        .disabled(true)
                        .element(theme),
                ),
        )
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The Hosts panel view.
pub struct HostList {
    state: Entity<TermiusState>,
    focus: FocusHandle,
    /// Row cursor for keyboard navigation (index into the flattened rows).
    cursor: Option<usize>,
    /// Group currently drilled into (breadcrumb); `None` = "All hosts".
    current_group: Option<String>,
    /// Whether the split "New host" dropdown menu is open.
    new_menu_open: bool,
}

impl HostList {
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            focus: cx.focus_handle(),
            cursor: None,
            current_group: None,
            new_menu_open: false,
        }
    }

    /// Focus the list (e.g. on ⌘B sidebar toggle).
    ///
    /// gpui 0.2: [`gpui::FocusHandle::focus`] only needs the window; `cx` is
    /// kept so callers don't have to change.
    pub fn focus(&self, window: &mut Window, _cx: &mut Context<Self>) {
        self.focus.focus(window);
    }

    /// The rows currently painted for the drilled-in group.
    fn current_rows(&self, cx: &gpui::App) -> Vec<ListRow> {
        let state = self.state.read(cx);
        flatten_entities(
            &state.library.groups,
            &state.library.hosts,
            self.current_group.as_deref(),
        )
    }

    // ----- navigation -----------------------------------------------------

    /// Drill into `group_id` (or back to the root with `None`).
    fn navigate_to(&mut self, group_id: Option<String>, cx: &mut Context<Self>) {
        self.current_group = group_id;
        self.cursor = None;
        self.new_menu_open = false;
        cx.notify();
    }

    fn open_group(&mut self, group_id: &str, cx: &mut Context<Self>) {
        self.navigate_to(Some(group_id.to_owned()), cx);
    }

    fn toggle_new_menu(&mut self, cx: &mut Context<Self>) {
        self.new_menu_open = !self.new_menu_open;
        cx.notify();
    }

    fn close_new_menu(&mut self, cx: &mut Context<Self>) {
        if self.new_menu_open {
            self.new_menu_open = false;
            cx.notify();
        }
    }

    /// `New host` → the Add-Host dialog (`onAddHost`, `_main.js:65858`).
    fn open_new_host(&mut self, cx: &mut Context<Self>) {
        self.new_menu_open = false;
        self.state.update(cx, |state, cx| state.open_dialog(Dialog::AddHost, cx));
        cx.notify();
    }

    /// Terminal / Serial: connect to the first saved host of `host_type`.
    ///
    /// PORT-TODO: a bare local / serial terminal (not tied to a saved host) has
    /// no `TermiusState` API yet.
    fn open_first_host_of_type(&mut self, host_type: HostType, cx: &mut Context<Self>) {
        let target = self
            .state
            .read(cx)
            .library
            .hosts
            .iter()
            .find(|host| host.host_type == host_type)
            .map(|host| host.id.clone());
        if let Some(id) = target {
            self.state.update(cx, |state, cx| state.open_connection(&id, cx));
        }
    }

    fn move_cursor(&mut self, dir: i32, cx: &mut Context<Self>) {
        let rows = self.current_rows(cx);
        let Some(index) = next_host_in_list(&rows, self.cursor, dir) else { return };
        self.cursor = Some(index);
        let host_id = match rows.get(index) {
            Some(ListRow::Host { id, .. }) => Some(id.clone()),
            _ => None,
        };
        if host_id.is_some() {
            self.state.update(cx, |state, cx| state.select_host(host_id, cx));
        }
    }

    fn connect_selected(&mut self, cx: &mut Context<Self>) {
        let Some(host_id) = self.state.read(cx).selected_host.clone() else { return };
        self.state.update(cx, |state, cx| state.open_connection(&host_id, cx));
        // PORT-TODO(gpui 0.2): hand keyboard focus to the terminal pane once
        // the session opens (needs a Window; today the user clicks the pane).
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.platform || event.keystroke.modifiers.function {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" => self.move_cursor(1, cx),
            "up" => self.move_cursor(-1, cx),
            "enter" => self.connect_selected(cx),
            "home" => {
                self.cursor = None;
                self.move_cursor(1, cx);
            }
            "end" => {
                self.cursor = None;
                self.move_cursor(-1, cx);
            }
            _ => {}
        }
    }

    // ----- chrome ---------------------------------------------------------

    /// The New-Entity toolbar (`r6e`, `_main.js:65858`) — the header's first
    /// child: the split "New host" button, Terminal and Serial.
    fn toolbar(&self, theme: TermiusTheme, menu_open: bool, cx: &mut Context<Self>) -> Div {
        let split = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(
                primary_segment(theme, "hosts-new-host")
                    .on_click(cx.listener(|this, _event, _window, cx| this.open_new_host(cx))),
            )
            .child(
                div()
                    .w(px(1.))
                    .h(px(BUTTON_HEIGHT))
                    .bg(with_alpha(white(), 0.15)),
            )
            .child(chevron_segment(theme, "hosts-new-host-menu", menu_open).on_click(
                cx.listener(|this, _event, _window, cx| this.toggle_new_menu(cx)),
            ));

        let terminal = ghost_icon_button(theme, "hosts-terminal", "terminal.svg", "Terminal")
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.open_first_host_of_type(HostType::Local, cx)
            }));
        let serial = ghost_icon_button(theme, "hosts-serial", "serial.svg", "Serial")
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.open_first_host_of_type(HostType::Serial, cx)
            }));

        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .flex_shrink_0()
            .child(split)
            .child(terminal)
            .child(serial)
    }

    /// The header band with the toolbar as its first child
    /// (`HostsFiltersHeader-fc79316f.js:423`).
    ///
    /// PORT-TODO: in the 300px sidebar the responsive search / tags / sort
    /// cluster can clip — the original hides those controls by width
    /// (`width-81 >= 250/200/100`); the responsive rules are not modelled yet.
    fn filters_header(&self, theme: TermiusTheme, menu_open: bool, cx: &mut Context<Self>) -> Div {
        FiltersHeader::new("Find a host or ssh user@hostname...")
            .action(self.toolbar(theme, menu_open, cx))
            .element(theme)
    }

    /// The group breadcrumb (`GroupPath`, `reconnectSaga:126512`).
    fn breadcrumb(
        &self,
        theme: TermiusTheme,
        crumbs: &[(Option<String>, String)],
        cx: &mut Context<Self>,
    ) -> Div {
        let last = crumbs.len().saturating_sub(1);
        let mut row = div()
            .flex()
            .flex_row()
            .flex_nowrap()
            .items_center()
            .px(px(30.))
            .pt(px(20.))
            .text_color(theme.title)
            .font_family(UI_FONT).text_size(px(14.))
            .font_weight(FontWeight(450.0))
            .line_height(px(20.));

        for (index, (id, title)) in crumbs.iter().enumerate() {
            let is_last = index == last;
            let color = if is_last { theme.title } else { theme.accent };
            let target = id.clone();
            let item = div()
                .id(SharedString::from(format!("crumb-{index}")))
                .flex()
                .items_center()
                .flex_shrink_0()
                .pr(px(if is_last { 0. } else { 10. }))
                .text_color(color)
                .whitespace_nowrap()
                .child(SharedString::from(title.clone()));
            let item = if is_last {
                item
            } else {
                item.on_click(
                    cx.listener(move |this, _event, _window, cx| this.navigate_to(target.clone(), cx)),
                )
            };
            row = row.child(item);
            if !is_last {
                row = row.child(
                    crate::icon("chevron.svg")
                        .w(px(10.))
                        .h(px(16.))
                        .pr(px(10.))
                        .text_color(theme.accent)
                        .with_transformation(Transformation::rotate(radians(
                            -std::f32::consts::FRAC_PI_2,
                        ))),
                );
            }
        }
        row
    }

    /// The split button's dropdown (`_main.js:65858`): New Group / Import /
    /// AWS / DigitalOcean / Azure.
    fn new_host_menu(&self, theme: TermiusTheme, cx: &mut Context<Self>) -> Div {
        div()
            .absolute()
            .top(px(QUICK_CONNECT_HEIGHT + HEADER_HEIGHT))
            .left(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .w(px(220.))
            .rounded(px(theme.corner_radius_medium))
            .border_1()
            .border_color(theme.border_light)
            .bg(theme.card_a)
            .text_color(theme.title)
            // PORT-TODO: New Group / Import / integrations have no
            // `TermiusState` API yet; each item just dismisses the menu.
            .child(
                menu_item(theme, "hosts-menu-new-group", "createGroup.svg", "New Group", false)
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.close_new_menu(cx)
                    })),
            )
            .child(
                menu_item(theme, "hosts-menu-import", "import.svg", "Import", false).on_click(
                    cx.listener(|this, _event: &ClickEvent, _window, cx| this.close_new_menu(cx)),
                ),
            )
            .child(
                menu_item(theme, "hosts-menu-aws", "AWS.svg", "AWS Integration", true).on_click(
                    cx.listener(|this, _event: &ClickEvent, _window, cx| this.close_new_menu(cx)),
                ),
            )
            .child(
                menu_item(
                    theme,
                    "hosts-menu-do",
                    "DigitalOcean.svg",
                    "DigitalOcean Integration",
                    true,
                )
                .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.close_new_menu(cx)
                })),
            )
            .child(
                menu_item(theme, "hosts-menu-azure", "azure.svg", "Azure Integration", true)
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.close_new_menu(cx)
                    })),
            )
    }
}

impl Render for HostList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Resolve everything the rows need while the state borrow is held, then
        // drop it before any `cx.listener` below registers against `cx`.
        let (rows, selected, loading, load_error, crumbs, menu_open) = {
            let state = self.state.read(cx);
            let rows = flatten_entities(
                &state.library.groups,
                &state.library.hosts,
                self.current_group.as_deref(),
            );
            let crumbs = group_path(&state.library.groups, self.current_group.as_deref());
            (
                rows,
                state.selected_host.clone(),
                state.library_loading,
                state.library_error.clone(),
                crumbs,
                self.new_menu_open,
            )
        };
        let rows_empty = rows.is_empty();

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar_background)
            .text_color(theme.title);

        // ----- Quick-Connect bar (above the header) -------------------------
        root = root.child(quick_connect_bar(theme));

        // ----- header: New-Entity toolbar + search / tags / sort ------------
        root = root.child(self.filters_header(theme, menu_open, cx));

        // ----- list, or the empty screen ------------------------------------
        if rows_empty && !loading && load_error.is_none() {
            root = root.child(empty_hosts_card(theme));
        } else {
            let mut list = div()
                .id("host-list-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .pb(px(12.));

            list = list.child(self.breadcrumb(theme, &crumbs, cx));

            if loading {
                list = list.child(
                    div().px(px(30.)).py(px(8.)).child(
                        text::R12S.style(div()).text_color(theme.muted).child(SharedString::from(
                            if rows_empty { "Loading…" } else { "Refreshing…" },
                        )),
                    ),
                );
            }
            if let Some(error) = load_error {
                list = list.child(
                    div().px(px(30.)).py(px(8.)).child(
                        text::R12S
                            .style(div())
                            .text_color(theme.danger)
                            .child(SharedString::from(error)),
                    ),
                );
            }

            let mut shown_groups = false;
            let mut shown_hosts = false;
            for (index, row) in rows.iter().enumerate() {
                match row {
                    ListRow::Group { id, title, host_count } => {
                        if !shown_groups {
                            list = list.child(section_title(theme, "Groups"));
                            shown_groups = true;
                        }
                        let group_id = id.clone();
                        list = list.child(
                            EntityRow::new(title.clone(), group_subtitle(*host_count), "Group.svg")
                                .element(theme)
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.open_group(&group_id, cx);
                                })),
                        );
                    }
                    ListRow::Host { id, label, subtitle } => {
                        if !shown_hosts {
                            list = list.child(section_title(theme, "Hosts"));
                            shown_hosts = true;
                        }
                        let is_selected = selected.as_deref() == Some(id.as_str());
                        let icon_name = self
                            .state
                            .read(cx)
                            .library
                            .host(id)
                            .map(host_icon)
                            .unwrap_or("host.svg");
                        let host_id = id.clone();
                        list = list.child(
                            EntityRow::new(label.clone(), subtitle.clone(), icon_name)
                                .selected(is_selected)
                                .element(theme)
                                .on_click(cx.listener(
                                    move |this, event: &ClickEvent, _window, cx| {
                                        this.cursor = Some(index);
                                        if click_count(event) >= 2 {
                                            this.state.update(cx, |state, cx| {
                                                state.open_connection(&host_id, cx)
                                            });
                                        } else {
                                            let id = host_id.clone();
                                            this.state.update(cx, |state, cx| {
                                                state.select_host(Some(id), cx)
                                            });
                                        }
                                    },
                                )),
                        );
                    }
                }
            }

            root = root.child(list);
        }

        // ----- split-button dropdown (paints above the list) ----------------
        if menu_open {
            root = root.child(self.new_host_menu(theme, cx));
        }

        root.track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key(event, cx);
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: &str, title: &str, parent: Option<&str>) -> Group {
        Group {
            id: id.into(),
            title: title.into(),
            parent_id: parent.map(str::to_owned),
            ..Group::default()
        }
    }

    fn host(id: &str, label: &str, groups: &[&str]) -> Host {
        Host {
            id: id.into(),
            label: label.into(),
            hostname: format!("{label}.test"),
            group_ids: groups.iter().map(|g| (*g).to_owned()).collect(),
            ..Host::default()
        }
    }

    #[test]
    fn tree_nests_groups_and_hosts() {
        let groups = [
            group("g-root", "Production", None),
            group("g-child", "DB", Some("g-root")),
            group("g-other", "Staging", None),
        ];
        let hosts = [
            host("h1", "web-1", &["g-root"]),
            host("h2", "pg-1", &["g-child"]),
            host("h3", "orphan", &[]),
        ];
        let rows = build_tree(&groups, &hosts);
        let ids: Vec<&str> = rows
            .iter()
            .map(|row| match row {
                TreeRow::Group { id, .. } | TreeRow::Host { id, .. } => id.as_str(),
            })
            .collect();
        // Production (root) → its host → nested DB group → its host, then the
        // second root, then ungrouped hosts.
        assert_eq!(ids, ["g-root", "h1", "g-child", "h2", "g-other", "h3"]);
        // Indentation: member one level deeper than its group.
        assert!(matches!(&rows[1], TreeRow::Host { depth: 1, .. }));
        assert!(matches!(&rows[3], TreeRow::Host { depth: 2, .. }));
        assert!(matches!(&rows[5], TreeRow::Host { depth: 0, .. }));
    }

    #[test]
    fn tree_survives_group_cycles() {
        // a → b → a (no root) plus a healthy root.
        let groups = [
            group("a", "A", Some("b")),
            group("b", "B", Some("a")),
            group("ok", "Fine", None),
        ];
        let hosts = [host("h", "host", &["a"])];
        let rows = build_tree(&groups, &hosts);
        // The cycle is unreachable from the roots, so its host falls through
        // as ungrouped instead of hanging the builder.
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[0], TreeRow::Group { id, .. } if id == "ok"));
        assert!(matches!(&rows[1], TreeRow::Host { .. }));
    }

    #[test]
    fn tree_shared_host_claimed_once() {
        let groups = [group("g1", "One", None), group("g2", "Two", None)];
        let hosts = [host("h", "shared", &["g1", "g2"])];
        let rows = build_tree(&groups, &hosts);
        let host_rows = rows
            .iter()
            .filter(|row| matches!(row, TreeRow::Host { .. }))
            .count();
        assert_eq!(host_rows, 1);
    }

    #[test]
    fn cursor_walks_only_host_rows() {
        let rows = vec![
            TreeRow::Group { id: "g".into(), title: "G".into(), depth: 0 },
            TreeRow::Host { id: "h1".into(), label: "a".into(), hostname: "a".into(), depth: 1 },
            TreeRow::Host { id: "h2".into(), label: "b".into(), hostname: "b".into(), depth: 1 },
        ];
        // From nothing: down finds the first host (index 1), skipping the group.
        assert_eq!(next_host_row(&rows, None, 1), Some(1));
        assert_eq!(next_host_row(&rows, Some(1), 1), Some(2));
        // Past the end: None.
        assert_eq!(next_host_row(&rows, Some(2), 1), None);
        // Up from the first host skips the group header and stops.
        assert_eq!(next_host_row(&rows, Some(1), -1), None);
        assert_eq!(next_host_row(&rows, None, -1), Some(2));
        // Empty tree never yields a row.
        assert_eq!(next_host_row(&[], None, 1), None);
    }

    #[test]
    fn collapsing_a_group_hides_its_subtree() {
        let groups = [
            group("g-root", "Production", None),
            group("g-child", "DB", Some("g-root")),
            group("g-other", "Staging", None),
        ];
        let hosts = [
            host("h1", "web-1", &["g-root"]),
            host("h2", "pg-1", &["g-child"]),
            host("h3", "orphan", &[]),
        ];
        let rows = build_tree(&groups, &hosts);

        // Collapse the root: its own host, the nested group and its host vanish;
        // the second root and the orphan host remain.
        let mut collapsed = HashSet::new();
        collapsed.insert("g-root".to_owned());
        let visible = visible_rows(&rows, &collapsed);
        let ids: Vec<&str> = visible
            .iter()
            .map(|row| match row {
                TreeRow::Group { id, .. } | TreeRow::Host { id, .. } => id.as_str(),
            })
            .collect();
        assert_eq!(ids, ["g-root", "g-other", "h3"]);

        // Collapse the nested group only: the root keeps its own host.
        let mut collapsed = HashSet::new();
        collapsed.insert("g-child".to_owned());
        let visible = visible_rows(&rows, &collapsed);
        assert!(!visible.iter().any(|row| matches!(row, TreeRow::Host { id, .. } if id == "h2")));
        assert!(visible.iter().any(|row| matches!(row, TreeRow::Host { id, .. } if id == "h1")));

        // Nothing collapsed: identity.
        assert_eq!(visible_rows(&rows, &HashSet::new()), rows);
    }

    #[test]
    fn host_icon_and_subtitle_cover_the_model() {
        let mut host = host("h", "web", &[]);
        assert_eq!(host_icon(&host), "host.svg");
        assert_eq!(host_subtitle(&host), "web.test");

        host.username = "root".to_owned();
        assert_eq!(host_subtitle(&host), "root@web.test");

        host.host_type = HostType::Local;
        assert_eq!(host_icon(&host), "Apple.svg");

        host.hostname.clear();
        assert_eq!(host_subtitle(&host), "root@web");
    }

    #[test]
    fn flatten_splits_groups_and_hosts_at_the_root() {
        let groups = [
            group("g-b", "Beta", None),
            group("g-a", "Alpha", None),
            group("g-child", "Nested", Some("g-a")),
        ];
        let hosts = [
            host("h1", "web-2", &["g-a"]),
            host("h2", "web-1", &["g-a"]),
            host("h3", "loose", &[]),
        ];
        let rows = flatten_entities(&groups, &hosts, None);

        // Root children only (sorted), each with its direct host count, then
        // the ungrouped hosts (sorted).
        assert_eq!(
            rows,
            vec![
                ListRow::Group { id: "g-a".into(), title: "Alpha".into(), host_count: 2 },
                ListRow::Group { id: "g-b".into(), title: "Beta".into(), host_count: 0 },
                ListRow::Host { id: "h3".into(), label: "loose".into(), subtitle: "loose.test".into() },
            ]
        );
        // The nested group is not shown at the root.
        assert!(!rows.iter().any(|row| matches!(row, ListRow::Group { id, .. } if id == "g-child")));
    }

    #[test]
    fn flatten_drills_into_a_group() {
        let groups = [
            group("g-a", "Alpha", None),
            group("g-child", "Nested", Some("g-a")),
        ];
        let hosts = [
            host("h1", "web-2", &["g-a"]),
            host("h2", "web-1", &["g-a"]),
            host("h3", "elsewhere", &[]),
        ];
        let rows = flatten_entities(&groups, &hosts, Some("g-a"));
        // Child groups first, then the group's member hosts (sorted).
        assert_eq!(
            rows,
            vec![
                ListRow::Group { id: "g-child".into(), title: "Nested".into(), host_count: 0 },
                ListRow::Host { id: "h2".into(), label: "web-1".into(), subtitle: "web-1.test".into() },
                ListRow::Host { id: "h1".into(), label: "web-2".into(), subtitle: "web-2.test".into() },
            ]
        );
    }

    #[test]
    fn group_path_walks_to_the_root() {
        let groups = [
            group("root", "Production", None),
            group("mid", "Cluster", Some("root")),
            group("leaf", "DB", Some("mid")),
        ];
        let path = group_path(&groups, Some("leaf"));
        assert_eq!(
            path,
            vec![
                (None, "All hosts".to_owned()),
                (Some("root".to_owned()), "Production".to_owned()),
                (Some("mid".to_owned()), "Cluster".to_owned()),
                (Some("leaf".to_owned()), "DB".to_owned()),
            ]
        );
        // The root crumb alone when no group is drilled into.
        assert_eq!(group_path(&groups, None), vec![(None, "All hosts".to_owned())]);
    }

    #[test]
    fn group_path_survives_a_cycle() {
        let groups = [group("a", "A", Some("b")), group("b", "B", Some("a"))];
        // Must terminate rather than loop forever; the chain is cut at MAX_DEPTH.
        let path = group_path(&groups, Some("a"));
        assert_eq!(path[0], (None, "All hosts".to_owned()));
        assert!(path.len() <= MAX_DEPTH + 2);
    }

    #[test]
    fn os_icon_maps_platform_names() {
        assert_eq!(os_icon("Apple"), "Apple.svg");
        assert_eq!(os_icon("macOS"), "Apple.svg");
        assert_eq!(os_icon("Windows"), "Windows.svg");
        assert_eq!(os_icon("ubuntu"), "ubuntu.svg");
        assert_eq!(os_icon("Linux"), "linux.svg");
        assert_eq!(os_icon("unknown"), "host.svg");
        assert_eq!(os_icon(""), "host.svg");
    }

    #[test]
    fn group_subtitle_pluralizes() {
        assert_eq!(group_subtitle(0), "0 Hosts");
        assert_eq!(group_subtitle(1), "1 Host");
        assert_eq!(group_subtitle(3), "3 Hosts");
    }

    #[test]
    fn list_cursor_walks_only_host_rows() {
        let rows = vec![
            ListRow::Group { id: "g".into(), title: "G".into(), host_count: 2 },
            ListRow::Host { id: "h1".into(), label: "a".into(), subtitle: "a".into() },
            ListRow::Host { id: "h2".into(), label: "b".into(), subtitle: "b".into() },
        ];
        assert_eq!(next_host_in_list(&rows, None, 1), Some(1));
        assert_eq!(next_host_in_list(&rows, Some(1), 1), Some(2));
        assert_eq!(next_host_in_list(&rows, Some(2), 1), None);
        assert_eq!(next_host_in_list(&rows, Some(1), -1), None);
        assert_eq!(next_host_in_list(&rows, None, -1), Some(2));
        assert_eq!(next_host_in_list(&[], None, 1), None);
    }
}
