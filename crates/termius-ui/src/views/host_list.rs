//! host_list — the left sidebar: group tree + host rows.
//!
//! Port of Termius' host-tree sidebar: nested groups, host rows indented by
//! depth, click/arrow selection, Enter (or the footer button) to connect.

use std::collections::HashSet;

use gpui::{
    div, px, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use termius_core::{Group, Host};

use crate::app_state::TermiusState;
use crate::theme::{theme_of, TermiusTheme};

/// Indent per tree level.
const INDENT: f32 = 14.0;
/// Row height; keeps keyboard navigation aligned with the painted rows.
const ROW_HEIGHT: f32 = 26.0;
/// Cycle/depth guard for malformed group graphs.
const MAX_DEPTH: usize = 16;

/// One flattened sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Group { id: String, title: String, depth: usize },
    Host { id: String, label: String, hostname: String, depth: usize },
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

/// The sidebar view.
pub struct HostList {
    state: Entity<TermiusState>,
    focus: FocusHandle,
    /// Row cursor for keyboard navigation (index into [`build_tree`] output).
    cursor: Option<usize>,
}

impl HostList {
    pub fn new(state: Entity<TermiusState>, cx: &mut Context<Self>) -> Self {
        Self { state, focus: cx.focus_handle(), cursor: None }
    }

    /// Focus the list (e.g. on ⌘B sidebar toggle).
    ///
    /// gpui 0.2: [`gpui::FocusHandle::focus`] only needs the window; `cx` is
    /// kept so callers don't have to change.
    pub fn focus(&self, window: &mut Window, _cx: &mut Context<Self>) {
        self.focus.focus(window);
    }

    fn move_cursor(&mut self, dir: i32, cx: &mut Context<Self>) {
        let rows = {
            let state = self.state.read(cx);
            build_tree(&state.library.groups, &state.library.hosts)
        };
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
        let rows = {
            let state = self.state.read(cx);
            build_tree(&state.library.groups, &state.library.hosts)
        };
        let selected = self.state.read(cx).selected_host.clone();
        let loading = self.state.read(cx).library_loading;
        let load_error = self.state.read(cx).library_error.clone();

        let mut list = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar_background)
            .text_color(theme.foreground);

        list = list.child(
            div()
                .flex()
                .items_center()
                .h(px(36.))
                .px(px(12.))
                .border_b_1()
                .border_color(theme.border)
                .child(SharedString::from("Hosts"))
                .child(
                    div()
                        .text_color(theme.muted)
                        .child(SharedString::from(" · Termius")),
                ),
        );

        if loading {
            list = list.child(
                div().px(px(12.)).py(px(8.)).text_color(theme.muted).child(SharedString::from(
                    if rows.is_empty() { "Loading…" } else { "Refreshing…" },
                )),
            );
        }
        if let Some(error) = load_error {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_color(theme.danger)
                    .child(SharedString::from(error)),
            );
        }

        for (index, row) in rows.iter().enumerate() {
            let (indent, label, sub, is_group, row_id) = match row {
                TreeRow::Group { id, title, depth } => {
                    (*depth as f32 * INDENT, title.clone(), String::new(), true, id.clone())
                }
                TreeRow::Host { id, label, hostname, depth } => (
                    *depth as f32 * INDENT,
                    label.clone(),
                    hostname.clone(),
                    false,
                    id.clone(),
                ),
            };
            let is_selected = !is_group && selected.as_deref() == Some(row_id.as_str());
            let is_cursor = self.cursor == Some(index);
            let color = if is_group { theme.muted } else { theme.foreground };

            let mut row_el = div()
                .id(SharedString::from(format!("row-{row_id}")))
                .flex()
                .items_center()
                .justify_between()
                .h(px(ROW_HEIGHT))
                .px(px(12. + indent))
                .rounded(px(4.))
                .text_color(color);
            if is_selected {
                row_el = row_el.bg(with_selection(theme));
            } else if is_group {
                row_el = row_el.bg(theme.tab_background);
            } else if is_cursor {
                row_el = row_el.bg(theme.hover);
            }
            row_el = row_el.child(SharedString::from(label));
            if !sub.is_empty() {
                row_el = row_el.child(
                    div().text_color(theme.muted).child(SharedString::from(sub)),
                );
            }

            if is_group {
                list = list.child(row_el);
            } else {
                let host_id = row_id.clone();
                list = list.child(
                    row_el.on_click(cx.listener(move |this, _event, _window, cx| {
                        this.cursor = Some(index);
                        this.state
                            .update(cx, |state, cx| state.select_host(Some(host_id.clone()), cx));
                    })),
                );
            }
        }

        // Footer: explicit connect affordance (Enter does the same).
        //
        // PORT-TODO: the row list does not scroll yet (`overflow_y_scroll`
        // needs a scroll handle + scrollbar; wire one when the tree outgrows
        // the window).
        let connect = div()
            .id("host-list-connect")
            .flex()
            .items_center()
            .justify_center()
            .h(px(30.))
            .m(px(8.))
            .px(px(4.))
            .rounded(px(4.))
            .bg(theme.accent)
            .text_color(gpui::rgb(0xff_ffff))
            .child(SharedString::from("Connect ⏎"))
            .on_click(cx.listener(|this, _event, _window, cx| this.connect_selected(cx)));

        let mut root = div().flex().flex_col().size_full();
        root = root.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(list),
        );
        root.child(connect)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key(event, cx);
            }))
    }
}

/// Selection wash for the active row (accent at panel alpha).
fn with_selection(theme: TermiusTheme) -> gpui::Rgba {
    crate::theme::over(theme.tab_active, theme.selection)
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
}
