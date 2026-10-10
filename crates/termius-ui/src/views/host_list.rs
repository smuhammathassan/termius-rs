//! host_list — the left sidebar: search/filter header, group tree + host rows.
//!
//! Port of Termius' hosts sidebar. The header is the
//! `HostsFiltersHeader-fc79316f.js` band (45px tall, `--surface-high` fill, a
//! search field on the left and the tag-filter / sort icon buttons on the
//! right). Below it the list renders, per `HostsScreenSidebar-a8fb8852.js`
//! (Termius' host entity list) and the shared MUI `ListItem-38a69640.js`:
//!
//! * **group rows** — disclosure chevron + folder glyph + title (`B12P`);
//!   clicking the row collapses/expands the group's subtree.
//! * **host rows** — an OS/platform icon tile (24×24, `--card-c` dark), the
//!   label (`R14P`) over `user@host` (`R12S`), and a trailing ⋯ (`dots.svg`)
//!   affordance. Selected rows fill `--list-select`; hovered rows wash
//!   `--blue-a10`.
//!
//! Keyboard navigation (↑/↓/Enter) and the pure tree builder
//! ([`build_tree`]/[`TreeRow`]) are unchanged.

use std::collections::HashSet;

use gpui::{
    div, px, radians, App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Transformation, Window,
};
use termius_core::host::HostType;
use termius_core::{Group, Host};

use crate::app_state::TermiusState;
use crate::theme::{over, text, theme_of, TermiusTheme, ThemeMode};

/// Indent per tree level.
const INDENT: f32 = 14.0;
/// Host row height; two text lines (14 + 12) beside the 24px icon tile. Keeps
/// keyboard navigation aligned with the painted rows.
const ROW_HEIGHT: f32 = 40.0;
/// The header band height (`HostsFiltersHeader` `height: 45px`).
const HEADER_HEIGHT: f32 = 45.0;
/// Entity icon tile size (original `entityIcon`: `width/height: 24px`).
const ICON_TILE: f32 = 24.0;
/// Glyph inside the icon tile (`& svg { max-width/height }`).
const ICON_GLYPH: f32 = 14.0;
/// Cycle/depth guard for malformed group graphs.
const MAX_DEPTH: usize = 16;

/// One flattened sidebar row.
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

/// Drop the descendants of every collapsed group. Rows keep their
/// [`build_tree`] order and depth, so indentation and the child-tree layout are
/// untouched — collapse only hides a group's subtree.
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

/// The bundled OS/platform icon for a host row.
///
/// PORT-TODO: the original keys this off the host's `os_name`
/// (`HostPresenter.icon` → `Apple` / `Windows` / `linux`…), which the local
/// [`Host`] model does not carry yet; fall back to the transport [`HostType`]
/// so every row still renders a real SVG instead of a placeholder.
fn host_icon(host: &Host) -> &'static str {
    match host.host_type {
        HostType::Local => "Apple.svg",
        HostType::Serial => "terminal.svg",
        HostType::Ssh | HostType::Mosh | HostType::Telnet => "host.svg",
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

/// A small square icon button (header filter/sort affordances).
fn icon_button(theme: TermiusTheme, icon_name: &str, id: &str) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_owned()))
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

/// The sidebar view.
pub struct HostList {
    state: Entity<TermiusState>,
    focus: FocusHandle,
    /// Row cursor for keyboard navigation (index into the visible rows).
    cursor: Option<usize>,
    /// Ids of groups whose subtree is currently collapsed.
    collapsed: HashSet<String>,
}

impl HostList {
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            focus: cx.focus_handle(),
            cursor: None,
            collapsed: HashSet::new(),
        }
    }

    /// Focus the list (e.g. on ⌘B sidebar toggle).
    ///
    /// gpui 0.2: [`gpui::FocusHandle::focus`] only needs the window; `cx` is
    /// kept so callers don't have to change.
    pub fn focus(&self, window: &mut Window, _cx: &mut Context<Self>) {
        self.focus.focus(window);
    }

    /// The rows currently painted (tree minus collapsed subtrees).
    fn current_rows(&self, cx: &App) -> Vec<TreeRow> {
        let state = self.state.read(cx);
        visible_rows(
            &build_tree(&state.library.groups, &state.library.hosts),
            &self.collapsed,
        )
    }

    fn move_cursor(&mut self, dir: i32, cx: &mut Context<Self>) {
        let rows = self.current_rows(cx);
        let Some(index) = next_host_row(&rows, self.cursor, dir) else { return };
        self.cursor = Some(index);
        let host_id = match rows.get(index) {
            Some(TreeRow::Host { id, .. }) => Some(id.clone()),
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

    /// Flip a group's collapsed state (disclosure chevron click).
    fn toggle_group(&mut self, group_id: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(group_id) {
            self.collapsed.insert(group_id.to_owned());
        }
        cx.notify();
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
}

impl Render for HostList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);

        // Resolve everything the rows need while the state borrow is held, then
        // drop it before any `cx.listener` below registers against `cx`.
        let (rows, host_meta, selected, loading, load_error) = {
            let state = self.state.read(cx);
            let rows = visible_rows(
                &build_tree(&state.library.groups, &state.library.hosts),
                &self.collapsed,
            );
            let meta: Vec<Option<(&'static str, String)>> = rows
                .iter()
                .map(|row| match row {
                    TreeRow::Host { id, .. } => state
                        .library
                        .host(id)
                        .map(|host| (host_icon(host), host_subtitle(host))),
                    TreeRow::Group { .. } => None,
                })
                .collect();
            (
                rows,
                meta,
                state.selected_host.clone(),
                state.library_loading,
                state.library_error.clone(),
            )
        };
        let rows_empty = rows.is_empty();

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar_background)
            .text_color(theme.title);

        // ----- header: search field + tag-filter / sort buttons -------------
        //
        // `HostsFiltersHeader-fc79316f.js`: `height: 45px`, `padding: 4px 12px 5px`,
        // `background: var(--surface-high)` (dark-grey-3 = `--card-a`), a `15px`
        // gap, and the responsive right cluster ending in the tag + sort glyphs.
        //
        // PORT-TODO: the search box is display-only — `TermiusState` has no
        // `search` field and gpui 0.2 text editing needs a `Content`/`InputEvent`
        // model on a tracked focus handle. Wire both when the filter lands.
        let header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(HEADER_HEIGHT))
            .px(px(12.))
            .pt(px(4.))
            .pb(px(5.))
            .bg(theme.card_a)
            .child(
                div()
                    .id("host-search")
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
                    .child(text::R12P.style(div()).child(SharedString::from("Search hosts"))),
            )
            .child(icon_button(theme, "tags.svg", "host-filter-tags"))
            .child(icon_button(theme, "sorting.svg", "host-sort"));
        root = root.child(header);

        // ----- scrollable list ---------------------------------------------
        let mut list = div()
            .id("host-list-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .py(px(4.));

        if loading {
            list = list.child(
                div().px(px(12.)).py(px(8.)).child(
                    text::R12S.style(div()).text_color(theme.muted).child(SharedString::from(
                        if rows_empty { "Loading…" } else { "Refreshing…" },
                    )),
                ),
            );
        }
        if let Some(error) = load_error {
            list = list.child(
                div().px(px(12.)).py(px(8.)).child(
                    text::R12S.style(div()).text_color(theme.danger).child(SharedString::from(error)),
                ),
            );
        }

        for (index, row) in rows.iter().enumerate() {
            let indent = row.depth() as f32 * INDENT;
            match row {
                TreeRow::Group { id, title, .. } => {
                    let is_collapsed = self.collapsed.contains(id);
                    // Chevron points down when expanded, right when collapsed.
                    let chevron = crate::icon("allSettingsChevron.svg")
                        .w(px(12.))
                        .h(px(12.))
                        .text_color(theme.muted);
                    let chevron = if is_collapsed {
                        chevron.with_transformation(Transformation::rotate(radians(
                            -std::f32::consts::FRAC_PI_2,
                        )))
                    } else {
                        chevron
                    };

                    let row_el = div()
                        .id(SharedString::from(format!("group-{id}")))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(ROW_HEIGHT))
                        .px(px(12. + indent))
                        .rounded(px(4.))
                        .text_color(theme.title)
                        .hover(move |hover| hover.bg(theme.hover))
                        .child(chevron)
                        .child(crate::icon("Group.svg").w(px(16.)).h(px(16.)).text_color(theme.muted))
                        .child(text::B12P.style(div()).child(SharedString::from(title.clone())));

                    let group_id = id.clone();
                    list = list.child(row_el.on_click(cx.listener(
                        move |this, _event, _window, cx| {
                            this.toggle_group(&group_id, cx);
                        },
                    )));
                }
                TreeRow::Host { id, label, hostname, .. } => {
                    let is_selected = selected.as_deref() == Some(id.as_str());
                    let is_cursor = self.cursor == Some(index);
                    let (icon_name, subtitle) = host_meta
                        .get(index)
                        .and_then(|meta| meta.clone())
                        .unwrap_or(("host.svg", hostname.clone()));
                    let display = if label.trim().is_empty() {
                        hostname.clone()
                    } else {
                        label.clone()
                    };

                    // `entityIcon` tile: 24×24, `--card-b` light / `--card-c`
                    // dark, `--text-secondary` glyph.
                    let tile_bg = match theme.mode {
                        ThemeMode::Dark => theme.card_c,
                        ThemeMode::Light => theme.card_b,
                    };

                    let mut row_el = div()
                        .id(SharedString::from(format!("host-{id}")))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(ROW_HEIGHT))
                        .px(px(12. + indent))
                        .rounded(px(4.))
                        .text_color(theme.title);
                    if is_selected {
                        // `--list-select` (dark-grey-4 = `--card-c`).
                        row_el = row_el.bg(theme.card_c);
                        row_el = row_el.hover(move |hover| hover.bg(over(theme.card_c, theme.hover)));
                    } else if is_cursor {
                        row_el = row_el.bg(theme.hover);
                    } else {
                        row_el = row_el.hover(move |hover| hover.bg(theme.hover));
                    }
                    row_el = row_el
                        .child(
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
                                .child(
                                    crate::icon(icon_name).w(px(ICON_GLYPH)).h(px(ICON_GLYPH)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.))
                                .child(
                                    text::R14P
                                        .style(div())
                                        .truncate()
                                        .child(SharedString::from(display)),
                                )
                                .child(
                                    text::R12S
                                        .style(div())
                                        .text_color(theme.muted)
                                        .truncate()
                                        .child(SharedString::from(subtitle)),
                                ),
                        )
                        // Trailing ⋯ overflow affordance.
                        //
                        // PORT-TODO: wire the host context menu (edit / duplicate
                        // / delete) once the shell exposes an anchor menu.
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(20.))
                                .h(px(20.))
                                .flex_shrink_0()
                                .text_color(theme.muted)
                                .child(crate::icon("dots.svg").w(px(12.)).h(px(4.))),
                        );

                    let host_id = id.clone();
                    list = list.child(row_el.on_click(cx.listener(
                        move |this, _event, _window, cx| {
                            this.cursor = Some(index);
                            this.state.update(cx, |state, cx| {
                                state.select_host(Some(host_id.clone()), cx);
                            });
                        },
                    )));
                }
            }
        }

        root.child(list)
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
        assert!(matches!(
            &rows[1],
            TreeRow::Host { depth: 1, .. }
        ));
        assert!(matches!(
            &rows[3],
            TreeRow::Host { depth: 2, .. }
        ));
        assert!(matches!(
            &rows[5],
            TreeRow::Host { depth: 0, .. }
        ));
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
}
