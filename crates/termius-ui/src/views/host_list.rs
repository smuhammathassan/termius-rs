//! host_list — the **Home / "New Tab"** screen: the centred search field, the
//! "Recent connections" card, and its rows.
//!
//! Rebuilt from `analysis/recon/05-home.md`. The routed New-Tab body is `r_e`
//! (`_main.js:87394`, styles `i_e` `_main.js:87444`): a `--entity-grid-background`
//! root with `padding: 40px 20px` holding a `max-width: 720px`, `margin: 0 auto`
//! column (`gap: 20px`) of
//!
//! * **the search field** (`Nwe`, `_main.js:86591`) — a 36px `InputField`
//!   (`--light-grey-4` / dark `--dark-grey-1`, 10px radius, placeholder
//!   `"Search hosts or tabs"`, `--text-secondary`) with the ⌘K
//!   command-palette hint right-aligned (`Hwe.shortcut`);
//! * **the "Recent connections" card** (`Qwe`, `_main.js:87109`; shell `PF`
//!   `_main.js:87012` / `$we` `86946`, styles `Uwe` `86980`) — a `--foreground`
//!   card (15px padding, 20px radius, 10px gap) whose header is the 14/700
//!   title plus the two `Button$1` `size="small"` actions **"Create a
//!   workspace"** (`color:"regular"`) and **"Restore"** (`color:"accent"`),
//!   and whose list is the `RF` rows (`_main.js:86712`, styles `j0` `86763`):
//!   a 20×20 OS-coloured [`ShapedIcon`] tile (radius 6, white glyph), the 14px
//!   label left (`kF`, `_main.js:86684`) and the 12px vault path right (`EF`,
//!   `_main.js:86698`; `qwe`, `_main.js:87043`);
//! * **the empty screen** (`Bwe`, `_main.js:86663`) when nothing is recent.
//!
//! The OS tile keys off `HostPresenter.icon(host)` = `host.os_name || "unknown"`
//! (`reconnectSaga:63048`) resolved through `systemsIcons` (`reconnectSaga:88730`)
//! to a colour (`--system-ubuntu` orange, `--system-macos` blue, …) and a
//! `ShapedIcon` (`reconnectSaga:88420`).
//!
//! # Legacy helpers (kept for compatibility)
//!
//! The file still carries the original Hosts-panel reconstruction: the DFS
//! [`build_tree`] / [`TreeRow`] flatten, the two-section [`flatten_entities`] /
//! [`ListRow`] model, the [`group_path`] breadcrumb and the keyboard helpers.
//! They are no longer painted by [`HostList`] (the Hosts panel moved out of this
//! screen) but stay exported and unit-tested.
//!
//! # PORT-TODOs
//!
//! * The Home screen is the **full** `pane2` body in the original; `AppShell`
//!   still mounts this view as the 300px Hosts sidebar column
//!   (`app_shell.rs:386`), so the 720px centred column is squeezed until the
//!   shell routes it full-width.
//! * `termius_core::Host` has no `os_name`, so the tile keys off [`HostType`]
//!   (Local → Apple, Serial → serial, else host); [`os_icon`] / [`os_color`] are
//!   wired and ready for the field.
//! * gpui 0.2.2 has no text input and `TermiusState` has no command palette, so
//!   the search field is display-only (clicking it does nothing).
//! * There is no connection-history or workspace-template API: "Recent
//!   connections" is fed from `library.hosts` and "Create a workspace" only
//!   clears the selection.
//! * Selection is single-click (the frozen `TermiusState::select_host` is one
//!   id); the original's ⌘-toggle / shift-range multi-select (`jwe`,
//!   `_main.js:86890`) is not modelled.

use std::collections::HashSet;

use gpui::{
    div, px, ClickEvent, Context, Div, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use termius_core::host::HostType;
use termius_core::{Group, Host};

use crate::app_state::TermiusState;
use crate::primitives::{Button, ButtonSize, EmptyState, ShapedIcon};
use crate::theme::{text, theme_of, with_alpha, TermiusTheme, ThemeMode};

// --- layout metrics (from the original CSS) --------------------------------

/// `Hwe` + `InputField.medium` — the search field height (`36px`).
const SEARCH_HEIGHT: f32 = 36.0;
/// `i_e.contentWrapper` — the centred column width (`max-width: 720px`).
const CONTENT_MAX_WIDTH: f32 = 720.0;
/// `Uwe.card` — the section-card radius (`--corner-radius-large-increased`).
const CARD_RADIUS: f32 = 20.0;
/// `j0.icon` — the row tile size (`20px`).
const ROW_TILE_SIZE: f32 = 20.0;
/// `j0.icon` — the row tile radius (`--corner-radius-small-increased`).
const ROW_TILE_RADIUS: f32 = 6.0;
/// `j0.icon svg` — the glyph inside the tile (`max-width: 10px`, rounded up for
/// legibility at the port's DPI).
const ROW_TILE_GLYPH: f32 = 12.0;
/// `j0.label` / `j0.descriptionText` — the row text column cap (`max-width:
/// 310px`).
const ROW_TEXT_MAX_WIDTH: f32 = 310.0;
/// Cycle/depth guard for malformed group graphs.
const MAX_DEPTH: usize = 16;
/// The `commandPalette` shortcut shown at the right of the search field
/// (`Nwe.shortcut`; `_main.js:86664`).
const COMMAND_PALETTE_HINT: &str = "⌘+K";

/// Fully transparent (an unselected, odd row fill).
fn clear() -> Rgba {
    Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
}

/// The click count of a gpui [`ClickEvent`] (keyboard activations count as 1).
fn click_count(event: &ClickEvent) -> usize {
    match event {
        ClickEvent::Mouse(mouse) => mouse.up.click_count,
        ClickEvent::Keyboard(_) => 1,
    }
}

// --- resolved Home colours the theme has no token for ----------------------

/// `--entity-grid-background`: `--main-bg` `#1d2033` (dark) / `#edf1f2` (light).
fn entity_grid_bg(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Dark => theme.background,
        ThemeMode::Light => gpui::rgb(0xed_f1_f2),
    }
}

/// `j0.item:nth-of-type(even)`: `--dark-grey-4` `#32364a` (dark) /
/// `--entity-grid-background` `#edf1f2` (light).
fn even_row_bg(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => entity_grid_bg(theme),
    }
}

/// `j0.item:hover`: `--list-hover` `#3e4257` (dark) / `#e6ebed` (light).
fn list_hover_fill(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Dark => theme.border_strong,
        ThemeMode::Light => theme.card_b,
    }
}

/// `Hwe.inputField` fill: `--light-grey-4` `#d5dde0` (light) /
/// `--dark-grey-1` `#141729` (dark).
fn search_field_bg(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Dark => theme.sidebar_background,
        ThemeMode::Light => gpui::rgb(0xd5_dd_e0),
    }
}

/// `Hwe.inputField` border: `--border-basic` (light) / `--dark-grey-5`
/// `#3e4257` (dark).
fn search_field_border(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Dark => theme.border_strong,
        ThemeMode::Light => theme.border_basic,
    }
}

/// `--text-secondary`: `#8d91a5` (dark) / `#798c94` (light).
fn text_secondary(theme: TermiusTheme) -> Rgba {
    match theme.mode {
        ThemeMode::Dark => gpui::rgb(0x8d_91_a5),
        ThemeMode::Light => gpui::rgb(0x79_8c_94),
    }
}

/// `--dark-blue-solid` `#004878` — the default [`ShapedIcon`] fill (used when a
/// host has no OS colour).
fn dark_blue_solid() -> Rgba {
    gpui::rgb(0x00_48_78)
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
// Flattened two-section model (kept for compatibility with tests)
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
/// once the field lands (see [`host_tile`]).
pub fn os_icon(os_name: &str) -> &'static str {
    match os_name.trim().to_ascii_lowercase().as_str() {
        "apple" | "macos" | "mac" | "osx" | "darwin" => "Apple.svg",
        "windows" | "win" | "windows10" | "windows11" => "Windows.svg",
        "ubuntu" => "ubuntu.svg",
        "linux" | "debian" | "centos" | "fedora" | "arch" | "redhat" => "linux.svg",
        _ => "host.svg",
    }
}

/// The tile colour for a Termius `os_name` (`systemsIcons`, `reconnectSaga:88730`).
///
/// Values are the `--system-*` vars from `analysis/termius-theme.json`; `None`
/// for unknown platforms (the caller falls back to [`dark_blue_solid`]).
///
/// PORT-TODO: unused until `termius_core::Host::os_name` lands (see [`host_tile`]).
pub fn os_color(os_name: &str) -> Option<Rgba> {
    let color = match os_name.trim().to_ascii_lowercase().as_str() {
        "apple" | "mac" | "osx" | "darwin" => gpui::rgb(0x17_17_19), // --system-apple
        "macos" => gpui::rgb(0x49_a3_f2),                            // --system-macos
        "windows" | "win" | "windows10" | "windows11" => gpui::rgb(0x00_a1_f1), // --system-windows
        "ubuntu" => gpui::rgb(0xe9_54_20),                           // --system-ubuntu
        "debian" => gpui::rgb(0xce_00_56),
        "centos" => gpui::rgb(0xef_a7_20),
        "fedora" => gpui::rgb(0x3c_6e_b4),
        "arch" => gpui::rgb(0x17_93_d1),
        "redhat" => gpui::rgb(0xee_00_00),
        "linux" => gpui::rgb(0xff_cc_33),                            // --system-linux
        "localhost" | "local" => gpui::rgb(0x21_b5_68),              // --green
        _ => return None,
    };
    Some(color)
}

/// The bundled icon for a host row (legacy Hosts-panel presenter).
///
/// PORT-TODO: the original keys this off the host's `os_name` (`HostPresenter
/// .icon`, `reconnectSaga:63048`), which the local [`Host`] model does not
/// carry yet; fall back to the transport [`HostType`]. Replaced on the Home
/// screen by [`host_tile`].
#[allow(dead_code)]
fn host_icon(host: &Host) -> &'static str {
    match host.host_type {
        HostType::Local => "Apple.svg",
        HostType::Serial => "serial.svg",
        HostType::Mosh => "mosh.svg",
        HostType::Ssh | HostType::Telnet => "host.svg",
    }
}

/// The `(glyph, tile colour)` for a Home row (`EntityIcon`, `reconnectSaga:88914`).
///
/// PORT-TODO: `termius_core::Host` has no `os_name`, so the OS split is not
/// reproducible; key off [`HostType`] instead (Local → Apple, Serial → serial,
/// else host) with the OS colours [`os_color`] resolves.
fn host_tile(host: &Host) -> (&'static str, Rgba) {
    match host.host_type {
        HostType::Local => ("Apple.svg", os_color("osx").unwrap_or_else(dark_blue_solid)),
        HostType::Serial => ("serial.svg", dark_blue_solid()),
        HostType::Mosh => ("mosh.svg", dark_blue_solid()),
        HostType::Ssh | HostType::Telnet => ("host.svg", dark_blue_solid()),
    }
}

/// `HostPresenter.label(host)` = `host.label || host.address || ""`
/// (`reconnectSaga:63039`).
fn host_label(host: &Host) -> String {
    if !host.label.trim().is_empty() {
        host.label.clone()
    } else if !host.hostname.trim().is_empty() {
        host.hostname.clone()
    } else {
        String::new()
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

/// The right-aligned vault path of a Home row (`qwe`, `_main.js:87043`):
/// `"Personal"` (+ the first group's title) for a personal host.
///
/// PORT-TODO: `termius_core::Host` has no vault / `is_shared`, and groups are
/// referenced by id (not a single nested `host.group`), so the shared-vault
/// prefix and the full group breadcrumb are approximated by the first group.
fn vault_label(host: &Host, groups: &[Group]) -> String {
    let mut label = String::from("Personal");
    if let Some(group) = host
        .group_ids
        .iter()
        .find_map(|id| groups.iter().find(|group| group.id == *id))
    {
        label.push_str(" / ");
        label.push_str(&group.title);
    }
    label
}

/// `GroupPresenter.description`: `"{n} Host"` / `"{n} Hosts"`
/// (`reconnectSaga:74206`).
#[allow(dead_code)]
fn group_subtitle(host_count: usize) -> String {
    format!("{host_count} Host{}", if host_count == 1 { "" } else { "s" })
}

// ---------------------------------------------------------------------------
// The Home list model
// ---------------------------------------------------------------------------

/// One "Recent connections" row (`RF`, `_main.js:86712`).
struct RecentRow {
    id: String,
    label: String,
    vault: String,
    icon: &'static str,
    color: Rgba,
}

/// The Home rows: every saved host (the original uses the recent-connections
/// history, `Xwe` `_main.js:87153`), sorted by label.
///
/// PORT-TODO: no connection-history API — feed from `library.hosts`.
fn recent_rows(hosts: &[Host], groups: &[Group]) -> Vec<RecentRow> {
    let mut rows: Vec<RecentRow> = hosts
        .iter()
        .map(|host| {
            let (icon, color) = host_tile(host);
            RecentRow {
                id: host.id.clone(),
                label: host_label(host),
                vault: vault_label(host, groups),
                icon,
                color,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.label.cmp(&b.label));
    rows
}

// ---------------------------------------------------------------------------
// Small chrome builders
// ---------------------------------------------------------------------------

/// The centred search field (`Nwe`, `_main.js:86591`; styles `Hwe` `86635`).
///
/// PORT-TODO: display-only — gpui 0.2.2 has no text input and `TermiusState`
/// has no command palette, so clicking it cannot open the palette
/// (`commandPaletteOpenedVia: "New Tab Empty State Search Field"`).
fn search_bar(theme: TermiusTheme) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .h(px(SEARCH_HEIGHT))
        .px(px(10.))
        .rounded(px(theme.corner_radius_medium))
        .border_1()
        .border_color(search_field_border(theme))
        .bg(search_field_bg(theme))
        .child(
            text::R14S
                .style(div())
                .flex_grow()
                .min_w(px(0.))
                .truncate()
                .text_color(text_secondary(theme))
                .child(SharedString::from("Search hosts or tabs")),
        )
        .child(
            text::R14S
                .style(div())
                .flex_shrink_0()
                .min_w(px(31.))
                .mx(px(5.))
                .text_color(text_secondary(theme))
                .child(SharedString::from(COMMAND_PALETTE_HINT)),
        )
}

/// One "Recent connections" row (`RF`, `_main.js:86712`; styles `j0` `86763`).
///
/// `even` toggles the `:nth-of-type(even)` fill; `selected` paints the
/// `--border-accent` rule + `--button-hover-overlay-accent` fill.
fn recent_row(theme: TermiusTheme, row: &RecentRow, selected: bool, even: bool) -> Stateful<Div> {
    let base = if even { even_row_bg(theme) } else { clear() };
    let fill = if selected { with_alpha(theme.primary, 0.25) } else { base };
    let border = if selected { theme.border_accent } else { clear() };
    let hover = list_hover_fill(theme);

    let tile = ShapedIcon::new(row.icon)
        .size(ROW_TILE_SIZE)
        .glyph_size(ROW_TILE_GLYPH)
        .corner_radius(ROW_TILE_RADIUS)
        .background(row.color)
        .element(theme)
        .mr(px(10.));

    div()
        .id(SharedString::from(format!("recent-{}", row.id)))
        .flex()
        .items_center()
        .gap(px(20.))
        .justify_between()
        .w_full()
        .p(px(10.))
        .rounded(px(theme.corner_radius_medium))
        .border_1()
        .border_color(border)
        .bg(fill)
        .overflow_hidden()
        .text_color(theme.title)
        .child(
            div()
                .flex()
                .items_center()
                .flex_grow()
                .min_w(px(0.))
                .child(tile)
                .child(
                    text::R14P
                        .style(div())
                        .truncate()
                        .max_w(px(ROW_TEXT_MAX_WIDTH))
                        .text_color(theme.title)
                        .child(SharedString::from(row.label.clone())),
                ),
        )
        .child(
            text::R12S
                .style(div())
                .flex_shrink_0()
                .truncate()
                .max_w(px(ROW_TEXT_MAX_WIDTH))
                .text_right()
                .text_color(text_secondary(theme))
                .child(SharedString::from(row.vault.clone())),
        )
        .hover(move |style| style.bg(hover))
}

/// A muted status line (loading / library error).
fn status_line(theme: TermiusTheme, message: &str, danger: bool) -> Div {
    let color = if danger { theme.danger } else { text_secondary(theme) };
    text::R12S
        .style(div())
        .px(px(4.))
        .text_color(color)
        .child(SharedString::from(message.to_owned()))
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The Home / "New Tab" screen view.
pub struct HostList {
    state: Entity<TermiusState>,
    focus: FocusHandle,
    /// Ids of the recent-connection rows currently selected; the two card
    /// buttons act on this set. The original's ⌘-toggle / shift-range
    /// multi-select (`jwe`, `_main.js:86890`) is PORT-TODO.
    selection: HashSet<String>,
}

impl HostList {
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            focus: cx.focus_handle(),
            selection: HashSet::new(),
        }
    }

    /// Focus the list (e.g. on ⌘B sidebar toggle).
    ///
    /// gpui 0.2: [`gpui::FocusHandle::focus`] only needs the window; `cx` is
    /// kept so callers don't have to change.
    pub fn focus(&self, window: &mut Window, _cx: &mut Context<Self>) {
        self.focus.focus(window);
    }

    // ----- selection ------------------------------------------------------

    /// Select one recent row (single-click; the frozen `select_host` is one id).
    fn select_row(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selection.clear();
        self.selection.insert(id.to_owned());
        self.state
            .update(cx, |state, cx| state.select_host(Some(id.to_owned()), cx));
        cx.notify();
    }

    /// Restore one connection (double-click / Enter).
    fn open_host(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selection.clear();
        self.selection.insert(id.to_owned());
        self.state
            .update(cx, |state, cx| state.open_connection(id, cx));
        cx.notify();
    }

    /// Restore every selected connection (`restoreConnections`, `_main.js:87076`).
    fn restore_selected(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        let ids: Vec<String> = self.selection.iter().cloned().collect();
        self.state.update(cx, |state, cx| {
            for id in &ids {
                state.open_connection(id, cx);
            }
        });
    }

    /// "Create a workspace" (`createWorkspace`, `_main.js:87086`).
    ///
    /// PORT-TODO: `TermiusState` has no workspace-template API; the original
    /// builds a `WorkspaceTemplate` from the selected connections and opens the
    /// create-workspace flow. For now this just resets the selection.
    fn create_workspace(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        cx.notify();
    }

    // ----- keyboard -------------------------------------------------------

    /// Move the selection `dir` rows (`+1` down, `-1` up) through the recent
    /// list.
    fn move_selection(&mut self, dir: i32, cx: &mut Context<Self>) {
        let ids: Vec<String> = {
            let state = self.state.read(cx);
            recent_rows(&state.library.hosts, &state.library.groups)
                .into_iter()
                .map(|row| row.id)
                .collect()
        };
        if ids.is_empty() {
            return;
        }
        let next = match ids.iter().position(|id| self.selection.contains(id)) {
            Some(index) => {
                let candidate = index as i32 + dir;
                if candidate < 0 || candidate as usize >= ids.len() {
                    return;
                }
                candidate as usize
            }
            None if dir >= 0 => 0,
            None => ids.len() - 1,
        };
        let id = ids[next].clone();
        self.select_row(&id, cx);
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.platform || event.keystroke.modifiers.function {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" => self.move_selection(1, cx),
            "up" => self.move_selection(-1, cx),
            "enter" => self.restore_selected(cx),
            "home" => {
                self.selection.clear();
                self.move_selection(1, cx);
            }
            "end" => {
                self.selection.clear();
                self.move_selection(-1, cx);
            }
            _ => {}
        }
    }

    // ----- chrome ---------------------------------------------------------

    /// The "Recent connections" card (`PF` / `$we`, `_main.js:86946`).
    fn recent_card(
        &self,
        theme: TermiusTheme,
        rows: &[RecentRow],
        has_selection: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().flex().flex_col().gap(px(5.)).w_full();
        for (index, row) in rows.iter().enumerate() {
            let selected = self.selection.contains(&row.id);
            let host_id = row.id.clone();
            list = list.child(recent_row(theme, row, selected, index % 2 == 1).on_click(
                cx.listener(move |this, event: &ClickEvent, _window, cx| {
                    if click_count(event) >= 2 {
                        this.open_host(&host_id, cx);
                    } else {
                        this.select_row(&host_id, cx);
                    }
                }),
            ));
        }

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .w_full()
            .child(
                text::B14P
                    .style(div())
                    .flex_grow()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(theme.title)
                    .child(SharedString::from("Recent connections")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .flex_shrink_0()
                    .child(
                        Button::new("Create a workspace")
                            .secondary()
                            .size(ButtonSize::Small)
                            .disabled(!has_selection)
                            .on_click(
                                theme,
                                cx.listener(|this, _event, _window, cx| this.create_workspace(cx)),
                            ),
                    )
                    .child(
                        Button::new("Restore")
                            .primary()
                            .size(ButtonSize::Small)
                            .disabled(!has_selection)
                            .on_click(
                                theme,
                                cx.listener(|this, _event, _window, cx| {
                                    this.restore_selected(cx)
                                }),
                            ),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(10.))
            .w_full()
            .p(px(15.))
            .rounded(px(CARD_RADIUS))
            .bg(theme.card_a) // --foreground
            .text_color(theme.title)
            .child(header)
            .child(list)
    }
}

impl Render for HostList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Resolve everything the rows need while the state borrow is held, then
        // drop it before any `cx.listener` below registers against `cx`.
        let (rows, loading, load_error) = {
            let state = self.state.read(cx);
            (
                recent_rows(&state.library.hosts, &state.library.groups),
                state.library_loading,
                state.library_error.clone(),
            )
        };
        let rows_empty = rows.is_empty();
        let has_selection = !self.selection.is_empty();

        // `i_e.contentWrapper`: max-width 720px, `margin: 0 auto`, gap 20px.
        let mut content = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(20.))
            .w_full()
            .max_w(px(CONTENT_MAX_WIDTH))
            .mx_auto();

        // `Nwe`: the search field.
        content = content.child(search_bar(theme));

        if rows_empty && !loading && load_error.is_none() {
            // `Bwe`: the "Recent sessions" empty screen.
            content = content.child(
                div().w_full().h(px(260.)).child(
                    EmptyState::new(
                        "Recent sessions",
                        "Your recent sessions will be visible here.",
                    )
                    .element(theme),
                ),
            );
        } else {
            if loading {
                content = content.child(status_line(theme, "Loading…", false));
            }
            if let Some(error) = load_error {
                content = content.child(status_line(theme, &error, true));
            }
            content = content.child(self.recent_card(theme, &rows, has_selection, cx));
        }

        // `i_e.root`: --entity-grid-background, padding 40px 20px, scrollable.
        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .bg(entity_grid_bg(theme))
            .text_color(theme.title)
            .pt(px(40.))
            .pb(px(40.))
            .px(px(20.))
            .child(content)
            .track_focus(&self.focus)
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

    #[test]
    fn os_color_maps_system_palette() {
        assert_eq!(os_color("ubuntu"), Some(gpui::rgb(0xe9_54_20)));
        assert_eq!(os_color("macOS"), Some(gpui::rgb(0x49_a3_f2)));
        assert_eq!(os_color("Windows"), Some(gpui::rgb(0x00_a1_f1)));
        assert_eq!(os_color("unknown"), None);
        assert_eq!(os_color(""), None);
    }

    #[test]
    fn host_label_falls_back_to_address() {
        let mut host = host("h", "web-1", &[]);
        assert_eq!(host_label(&host), "web-1");
        host.label.clear();
        assert_eq!(host_label(&host), "web-1.test");
        host.hostname.clear();
        assert_eq!(host_label(&host), "");
    }

    #[test]
    fn vault_label_is_personal_plus_first_group() {
        let groups = [group("g1", "Production", None)];
        let mut host = host("h", "web", &[]);
        assert_eq!(vault_label(&host, &groups), "Personal");
        host.group_ids = vec!["g1".to_owned()];
        assert_eq!(vault_label(&host, &groups), "Personal / Production");
        // A dangling group id degrades to "Personal".
        host.group_ids = vec!["missing".to_owned()];
        assert_eq!(vault_label(&host, &groups), "Personal");
    }

    #[test]
    fn recent_rows_sort_by_label_and_tile_local_hosts() {
        let hosts = [
            host("h2", "zulu", &[]),
            host("h1", "alpha", &[]),
        ];
        let mut local = host("h3", "mid", &[]);
        local.host_type = HostType::Local;
        let mut all = hosts.to_vec();
        all.push(local);
        let rows = recent_rows(&all, &[]);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["alpha", "mid", "zulu"]);
        // Local hosts get the Apple tile on the apple system colour.
        let mid = rows.iter().find(|row| row.label == "mid");
        assert!(matches!(mid, Some(row) if row.icon == "Apple.svg"));
    }
}
