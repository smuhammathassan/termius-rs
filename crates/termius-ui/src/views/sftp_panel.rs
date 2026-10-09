//! sftp_panel — a minimal remote file browser.
//!
//! Lists the active session's remote directory over the bridge's SFTP
//! command, navigates into subdirectories, and climbs with `..`. File
//! download/preview is a later wave (PORT-TODO below).

use gpui::{
    div, px, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::app_state::TermiusState;
use crate::theme::theme_of;

/// Panel width (consumed by the terminal pane's geometry math).
pub const SFTP_WIDTH: f32 = super::app_shell::SFTP_WIDTH;

/// Render a byte count the way a file manager would.
pub(crate) fn format_size(size: Option<u64>) -> String {
    match size {
        None => String::new(),
        Some(k) if k < 1_000 => format!("{k} B"),
        Some(k) if k < 1024 * 100 => format!("{:.1} KB", k as f64 / 1024.0),
        Some(m) if m < 1024 * 1024 * 100 => format!("{:.1} MB", m as f64 / (1024.0 * 1024.0)),
        Some(g) => format!("{:.1} GB", g as f64 / (1024.0 * 1024.0 * 1024.0)),
    }
}

/// The SFTP side panel.
pub struct SftpPanel {
    state: Entity<TermiusState>,
}

impl SftpPanel {
    pub fn new(state: Entity<TermiusState>) -> Self {
        Self { state }
    }

    fn navigate(&mut self, name: String, cx: &mut Context<Self>) {
        // `sftp_open_entry` targets a specific session; the panel navigates
        // whatever session is currently active.
        let Some(session_id) = self.active_session_id(cx) else { return };
        self.state
            .update(cx, |state, cx| state.sftp_open_entry(&session_id, &name, cx));
    }
}

impl Render for SftpPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let (active_id, path, entries, loading, error) = {
            let state = self.state.read(cx);
            match state.active_session() {
                None => (None, String::new(), Vec::new(), false, None),
                Some(session) => (
                    Some(session.id.clone()),
                    session.sftp.path.clone(),
                    session.sftp.entries.clone(),
                    session.sftp.loading,
                    session.sftp.error.clone(),
                ),
            }
        };
        let entries_empty = entries.is_empty();
        let has_error = error.is_some();

        let mut panel = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar_background)
            .text_color(theme.foreground)
            .border_l_1()
            .border_color(theme.border);

        // Header: path + navigation.
        let path_label =
            if path.is_empty() { "SFTP".to_owned() } else { path.clone() };
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.))
            .px(px(10.))
            .border_b_1()
            .border_color(theme.border);
        header = header.child(
            div().text_color(theme.muted).child(SharedString::from(path_label)),
        );

        let nav_active = active_id.is_some();
        let up = div()
            .id("sftp-up")
            .px(px(6.))
            .rounded(px(3.))
            .text_color(if nav_active { theme.foreground } else { theme.muted })
            .child(SharedString::from("↑"))
            .on_click(cx.listener(|this, _event, _window, cx| {
                if let Some(id) = this.active_session_id(cx) {
                    this.state.update(cx, |state, cx| state.sftp_up(&id, cx));
                }
            }));
        let refresh = div()
            .id("sftp-refresh")
            .px(px(6.))
            .rounded(px(3.))
            .text_color(if nav_active { theme.foreground } else { theme.muted })
            .child(SharedString::from("⟳"))
            .on_click(cx.listener(|this, _event, _window, cx| {
                if let Some(id) = this.active_session_id(cx) {
                    this.state.update(cx, |state, cx| state.sftp_refresh(&id, cx));
                }
            }));
        header = header.child(div().flex().flex_row().gap(px(4.)).child(up).child(refresh));
        panel = panel.child(header);

        // Body.
        if active_id.is_none() {
            panel = panel.child(
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .text_color(theme.muted)
                    .child(SharedString::from("Connect a host to browse files")),
            );
        } else {
            if let Some(error) = error {
                panel = panel.child(
                    div()
                        .px(px(10.))
                        .py(px(8.))
                        .text_color(theme.danger)
                        .child(SharedString::from(error)),
                );
            }
            if loading {
                panel = panel.child(
                    div()
                        .px(px(10.))
                        .py(px(8.))
                        .text_color(theme.muted)
                        .child(SharedString::from("Loading…")),
                );
            }
            for entry in entries {
                let is_dir = entry.is_dir.unwrap_or(false);
                let label = if is_dir {
                    format!("📁 {}", entry.name)
                } else {
                    format!("📄 {}", entry.name)
                };
                let size = format_size(entry.size);
                let name = entry.name.clone();

                let mut row = div()
                    .id(SharedString::from(format!("sftp-{}", entry.name)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(px(3.));
                row = row.child(SharedString::from(label));
                if !size.is_empty() {
                    row = row.child(
                        div().text_color(theme.muted).child(SharedString::from(size)),
                    );
                }
                if is_dir {
                    // Double duty: single click enters (Termius uses double;
                    // PORT-TODO once raw gpui's double-click is confirmed).
                    row = row.on_click(cx.listener(move |this, _event, _window, cx| {
                        this.navigate(name.clone(), cx);
                    }));
                }
                panel = panel.child(row);
            }
            if !loading && entries_empty && !has_error {
                panel = panel.child(
                    div()
                        .px(px(10.))
                        .py(px(8.))
                        .text_color(theme.muted)
                        .child(SharedString::from("Empty directory")),
                );
            }
        }

        panel
    }
}

impl SftpPanel {
    fn active_session_id(&self, cx: &Context<Self>) -> Option<String> {
        self.state.read(cx).active_session.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_like_a_file_manager() {
        assert_eq!(format_size(None), "");
        assert_eq!(format_size(Some(0)), "0 B");
        assert_eq!(format_size(Some(999)), "999 B");
        assert_eq!(format_size(Some(1024)), "1.0 KB");
        assert_eq!(format_size(Some(1024 * 1024)), "1.0 MB");
        assert_eq!(format_size(Some(1024 * 1024 * 3 + 512 * 1024)), "3.5 MB");
        assert_eq!(format_size(Some(1024 * 1024 * 1024 * 2)), "2.0 GB");
    }
}
