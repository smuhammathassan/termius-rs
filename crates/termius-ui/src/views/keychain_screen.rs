//! keychain_screen — the Keychain section: named vaults of stored secrets.
//!
//! Port of Termius' **Vault › Keychain** screen. The original
//! (`ConnectedVaultKeychain` in `analysis/readable/_main.js`, seeded from
//! `analysis/termius-extracted/ui-process/assets/Vaults-ef2e0d94.js`) renders
//! each vault as an `EntityReceipt`: a leading real icon, a title over a dim
//! subtitle, and a trailing `dots.svg` context-menu anchor, with hover
//! (`--blue-a10`) / selected (`--list-select`) fills.
//!
//! This port keeps that anatomy for each keychain and, under an expanded one,
//! lists its entries indented beneath it:
//!
//! * **keychain rows** — `keys.svg` glyph, the title (`R14P`) over
//!   `"{n} secret(s)"` (`R12S`), trailing `dots.svg`; clicking toggles the
//!   expansion (the original opens the entry).
//! * **entry rows** — `Lock.svg` glyph, the owning record id (`R12P`) over a
//!   fixed mask (`R12S`); indented under the parent.
//!
//! # Secrets never render
//!
//! Rows show `entry.owner_id` and a fixed mask ([`SECRET_MASK`]);
//! `KeychainEntry::secret` is not read anywhere in this module —
//! `entry_summaries_never_expose_secrets` guards that contract.
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
    div, px, AppContext as _, ClickEvent, Context, Div, Entity, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use termius_core::Keychain;

use crate::app_state::TermiusState;
use crate::primitives::{EmptyState, SectionHeader, SettingsText};
use crate::theme::{over, text, theme_of, with_alpha, TermiusTheme, ThemeMode};

/// Fixed mask shown in place of a stored secret.
const SECRET_MASK: &str = "••••••••";

/// Row height for a two-line receipt (title 14px over meta 12px).
const ROW_HEIGHT: f32 = 44.0;
/// Entry row height (single dense line).
const ENTRY_HEIGHT: f32 = 34.0;
/// Entity icon tile size (original `entityIcon`).
const ICON_TILE: f32 = 28.0;
/// Glyph inside the icon tile.
const ICON_GLYPH: f32 = 16.0;
/// Indent applied to an expanded keychain's entry rows.
const ENTRY_INDENT: f32 = 40.0;
/// White used for text/glyphs on the accent button fill.
const ON_ACCENT: gpui::Rgba = gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

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
// Row / button chrome (mirrors the Port Forwarding screen)
// ---------------------------------------------------------------------------

/// The primary toolbar action: an accent button with a leading `addCircle.svg`
/// glyph (the original `New key` toolbar button metrics).
fn new_action_button(
    theme: TermiusTheme,
    id: &str,
    label: &'static str,
    listener: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let hover_fill = over(theme.primary, with_alpha(ON_ACCENT, 0.25));
    div()
        .id(SharedString::from(id.to_owned()))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(36.))
        .px(px(16.))
        .rounded(px(theme.corner_radius_medium))
        .bg(theme.primary)
        .text_color(ON_ACCENT)
        .font_family(crate::assets::UI_FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(crate::icon("addCircle.svg").w(px(14.)).h(px(14.)))
        .child(label)
        .hover(move |hover| hover.bg(hover_fill))
        .on_click(listener)
}

/// The square entity-icon tile shared by every row (card-tinted square, muted
/// glyph).
fn icon_tile(theme: TermiusTheme, icon_name: &str) -> Div {
    let tile_bg = match theme.mode {
        ThemeMode::Dark => theme.card_c,
        ThemeMode::Light => theme.card_b,
    };
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
        .child(crate::icon(icon_name).w(px(ICON_GLYPH)).h(px(ICON_GLYPH)))
}

/// The trailing `dots.svg` overflow affordance (the original row's context-menu
/// anchor).
///
/// PORT-TODO: gpui 0.2.2 has no anchored popup menu here, so the affordance is
/// decorative for now (the same PORT-TODO `views::host_list` carries).
fn row_dots(theme: TermiusTheme, id: &str) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("keychain-menu-{id}")))
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .flex_shrink_0()
        .text_color(theme.muted)
        .child(crate::icon("dots.svg").w(px(12.)).h(px(4.)))
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

/// The Keychain section body (Termius' Vault › Keychain screen).
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

    /// One keychain receipt: `keys.svg` tile · title/meta · trailing dots.
    fn keychain_row(
        &self,
        theme: TermiusTheme,
        keychain: &Keychain,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let is_expanded = self.expanded.as_deref() == Some(keychain.id.as_str());
        let keychain_id = keychain.id.clone();
        let subtitle = secret_count_label(keychain.entries.len());

        let mut row = div()
            .id(SharedString::from(format!("keychain-{}", keychain.id)))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_HEIGHT))
            .px(px(12.))
            .rounded(px(theme.corner_radius_small))
            .text_color(theme.title);
        if is_expanded {
            // `--list-select` fill inside a `--border-accent` rule.
            row = row
                .bg(theme.card_c)
                .border_1()
                .border_color(theme.border_accent)
                .hover(move |hover| hover.bg(over(theme.card_c, theme.hover)));
        } else {
            row = row.hover(move |hover| hover.bg(theme.hover));
        }
        row = row
            .child(icon_tile(theme, "keys.svg"))
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
                            .child(SharedString::from(keychain_label(keychain))),
                    )
                    .child(
                        text::R12S
                            .style(div())
                            .text_color(theme.muted)
                            .truncate()
                            .child(SharedString::from(subtitle)),
                    ),
            )
            .child(row_dots(theme, &keychain.id))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_expanded(&keychain_id);
                // An expansion change repaints through the state observer the
                // shell attaches to `TermiusState`.
                this.state.update(cx, |_state, cx| cx.notify());
            }));
        row
    }

    /// One secret entry under an expanded keychain: `Lock.svg` tile · owner id
    /// over the mask. Indented; never reveals the secret.
    fn entry_row(
        theme: TermiusTheme,
        keychain_id: &str,
        index: usize,
        owner_id: String,
        mask: String,
    ) -> Stateful<Div> {
        div()
            .id(SharedString::from(format!(
                "keychain-{keychain_id}-entry-{index}"
            )))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(ENTRY_HEIGHT))
            .pl(px(ENTRY_INDENT))
            .pr(px(12.))
            .rounded(px(theme.corner_radius_small))
            .child(icon_tile(theme, "Lock.svg"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        text::R12P
                            .style(div())
                            .text_color(theme.title)
                            .truncate()
                            .child(SharedString::from(owner_id)),
                    )
                    .child(
                        text::R12S
                            .style(div())
                            .text_color(theme.muted)
                            .child(SharedString::from(mask)),
                    ),
            )
    }
}

impl Render for KeychainScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme_of(cx);
        let keychains = self.state.read(cx).library.keychains.clone();

        // Toolbar: section title + the primary "New Keychain" action (with
        // `addCircle.svg`), ported from the original keychain toolbar's
        // "+ New key" button.
        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.))
            .border_b_1()
            .border_color(theme.border)
            .child(SectionHeader::new("Keychain").element(theme))
            .child(div().px(px(12.)).child(new_action_button(
                theme,
                "keychain-new",
                "New Keychain",
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
            )));

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
        let mut list = div().flex().flex_col().gap(px(2.)).px(px(8.)).py(px(8.));
        for keychain in &keychains {
            let expanded = self.expanded.as_deref() == Some(keychain.id.as_str());
            list = list.child(self.keychain_row(theme, keychain, cx));

            if expanded {
                let entries = entry_labels(keychain);
                if entries.is_empty() {
                    list = list.child(
                        div().pl(px(ENTRY_INDENT)).child(
                            SettingsText::new("No secrets stored in this keychain yet.")
                                .element(theme),
                        ),
                    );
                } else {
                    for (index, (owner_id, mask)) in entries.into_iter().enumerate() {
                        list = list.child(Self::entry_row(
                            theme,
                            &keychain.id,
                            index,
                            owner_id,
                            mask,
                        ));
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

    #[test]
    fn row_icons_are_bundled() {
        assert!(crate::has_icon("keys.svg"));
        assert!(crate::has_icon("Lock.svg"));
        assert!(crate::has_icon("addCircle.svg"));
        assert!(crate::has_icon("dots.svg"));
    }
}
