//! keychain_screen — the Keychain section: named vaults of stored secrets.
//!
//! Port of Termius' Keychain screen: one row per keychain (`title` +
//! `"{n} secrets"`), click a row to expand a summary of that vault's
//! entries by `KeychainEntry::owner_id`, a New Keychain button in the
//! toolbar, and an empty state when the library has no vaults yet.
//!
//! **Secrets never render.** Rows show `entry.owner_id` and a fixed mask
//! ([`SECRET_MASK`]); `KeychainEntry::secret` is not read anywhere in this
//! module — `entry_summaries_never_expose_secrets` guards that contract.
//!
//! # Wire contract
//!
//! The function hands back a cached [`Entity`] so the shell can drop it
//! straight into its routed column. It deliberately does **not** read
//! `TermiusState` here: a `&mut Context<TermiusState>` only exists inside a
//! `TermiusState` update, and gpui leases one entity at a time — reading the
//! state while it is leased would panic. The view reads the library in its
//! own `render` (by then no lease is held), and every click notifies the
//! state so the shell's observer repaints:
//!
//! ```ignore
//! // inside AppShell::render:
//! let screen = self
//!     .state
//!     .update(cx, |_, cx| keychain_screen(self.state.clone(), cx));
//! ```

use gpui::{
    div, px, AppContext as _, Context, Entity, Global, IntoElement, ParentElement as _, Render,
    Styled as _, Window,
};
use termius_core::Keychain;

use crate::app_state::TermiusState;
use crate::primitives::{Button, EmptyState, ListItem, SectionHeader, SettingsText};
use crate::theme::theme_of;

/// Fixed mask shown in place of a stored secret.
const SECRET_MASK: &str = "••••••••";

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// The row subtitle: `"{n} secret(s)"` for a keychain's entry count.
fn secret_count_label(count: usize) -> String {
    if count == 1 {
        "1 secret".to_owned()
    } else {
        format!("{count} secrets")
    }
}

/// The row label: the keychain's title, or a placeholder when untitled.
fn keychain_label(keychain: &Keychain) -> String {
    if keychain.title.trim().is_empty() {
        "Untitled Keychain".to_owned()
    } else {
        keychain.title.clone()
    }
}

/// Safe summary rows for one keychain's entries: `(owner_id, mask)`.
///
/// The only entry data this screen renders — `KeychainEntry::secret` never
/// leaves the record.
fn entry_labels(keychain: &Keychain) -> Vec<(String, String)> {
    keychain
        .entries
        .iter()
        .map(|entry| (entry.owner_id.clone(), SECRET_MASK.to_owned()))
        .collect()
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// The process-wide Keychain screen instance.
///
/// The shell re-runs [`keychain_screen`] on every repaint, so the view (and
/// with it the expansion state) lives in a gpui [`Global`] instead of being
/// rebuilt — and losing its selection — each frame.
#[derive(Clone, Default)]
struct KeychainScreenHost(Option<Entity<KeychainScreen>>);

impl Global for KeychainScreenHost {}

/// The Keychain section body (Termius' Keychain screen).
pub struct KeychainScreen {
    state: Entity<TermiusState>,
    /// The expanded keychain (one at a time); `None` = all collapsed.
    expanded: Option<String>,
}

impl KeychainScreen {
    fn new(state: Entity<TermiusState>) -> Self {
        Self { state, expanded: None }
    }

    /// Expand `keychain_id`, or collapse it when it is already open.
    fn toggle_expanded(&mut self, keychain_id: &str) {
        self.expanded = if self.expanded.as_deref() == Some(keychain_id) {
            None
        } else {
            Some(keychain_id.to_owned())
        };
    }
}

impl Render for KeychainScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let keychains = self.state.read(cx).library.keychains.clone();

        // Toolbar: section header + New Keychain.
        let new_keychain = Button::new("New Keychain").primary().on_click(
            theme,
            cx.listener(|this, _event, _window, cx| {
                // PORT-TODO: a naming dialog + keychain CRUD once
                // TermiusState grows `add_keychain`; note it meanwhile.
                this.state.update(cx, |state, cx| {
                    state.status_text =
                        "New Keychain — naming and secret storage arrive with the next wave."
                            .to_owned();
                    cx.notify();
                });
            }),
        );
        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .pr(px(8.))
            .child(SectionHeader::new("Keychain").element(theme))
            .child(new_keychain);

        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(toolbar);

        if keychains.is_empty() {
            return root.child(
                div().flex_1().min_h(px(0.)).overflow_hidden().child(
                    EmptyState::new("No keychains", "Store passphrases securely…").element(theme),
                ),
            );
        }

        // PORT-TODO: the list does not scroll yet (same deal as host_list:
        // `overflow_y_scroll` needs a scroll handle + scrollbar).
        let mut list = div().flex().flex_col().gap(px(2.)).px(px(8.)).pb(px(8.));
        for keychain in &keychains {
            let expanded = self.expanded.as_deref() == Some(keychain.id.as_str());
            let subtitle = secret_count_label(keychain.entries.len());
            let row = ListItem::new(keychain_label(keychain), subtitle)
                .id(format!("keychain-{}", keychain.id))
                .selected(expanded)
                .on_click(theme, cx.listener({
                    let keychain_id = keychain.id.clone();
                    move |this, _event, _window, cx| {
                        this.toggle_expanded(&keychain_id);
                        // Repaint through the shell's state observer.
                        this.state.update(cx, |_state, cx| cx.notify());
                    }
                }));
            list = list.child(row);

            if expanded {
                let entries = entry_labels(keychain);
                if entries.is_empty() {
                    list = list.child(
                        SettingsText::new("No secrets stored in this keychain yet.").element(theme),
                    );
                } else {
                    for (index, (owner_id, mask)) in entries.into_iter().enumerate() {
                        list = list.child(
                            ListItem::new(owner_id, mask)
                                .id(format!("keychain-{}-entry-{index}", keychain.id))
                                .element(theme),
                        );
                    }
                }
            }
        }
        root.child(div().flex_1().min_h(px(0.)).overflow_hidden().child(list))
    }
}

/// The Keychain section screen, ready to drop into the shell's routed
/// column (see the module docs for the call contract).
pub fn keychain_screen(
    state: Entity<TermiusState>,
    cx: &mut Context<TermiusState>,
) -> Entity<KeychainScreen> {
    let cached = cx
        .try_global::<KeychainScreenHost>()
        .and_then(|host| host.0.clone());
    if let Some(screen) = cached {
        // Reuse the live view (it owns the expansion state) as long as it
        // watches the same state entity. Reading it here is safe: only
        // `TermiusState` is leased at this point, never the screen itself.
        if screen.read(cx).state == state {
            return screen;
        }
    }
    let screen = cx.new(|_cx| KeychainScreen::new(state));
    cx.set_global(KeychainScreenHost(Some(screen.clone())));
    screen
}

#[cfg(test)]
mod tests {
    use super::*;
    use termius_core::keychain::KeychainEntry;

    #[test]
    fn row_labels_and_counts() {
        assert_eq!(keychain_label(&Keychain::default()), "Untitled Keychain");
        let mut titled = Keychain::default();
        titled.title = " prod vault ".into();
        assert_eq!(keychain_label(&titled), " prod vault ");

        assert_eq!(secret_count_label(0), "0 secrets");
        assert_eq!(secret_count_label(1), "1 secret");
        assert_eq!(secret_count_label(7), "7 secrets");
    }

    #[test]
    fn entry_summaries_never_expose_secrets() {
        let keychain = Keychain {
            entries: vec![
                KeychainEntry {
                    owner_id: "key-1".into(),
                    secret: "hunter2".into(),
                },
                KeychainEntry {
                    owner_id: "host-9".into(),
                    secret: "hunter2".into(),
                },
            ],
            ..Keychain::default()
        };
        let labels = entry_labels(&keychain);
        assert_eq!(
            labels,
            vec![
                ("key-1".to_owned(), SECRET_MASK.to_owned()),
                ("host-9".to_owned(), SECRET_MASK.to_owned()),
            ]
        );
        for (_, mask) in &labels {
            assert_eq!(mask, SECRET_MASK);
        }
    }
}
