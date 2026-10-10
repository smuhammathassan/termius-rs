//! navigation — the section model behind the shell's **left-panel** nav.
//!
//! Reconstructed from `analysis/recon/00-shell.md`: the original's section
//! navigation is a **vertical list inside the 185px left panel** (`leftPanelTabs
//! = baseTabs minus SFTP`, `_main.js:112079`), *not* a horizontal top bar. The
//! routable sections are, in order, Hosts, Keychain, Port Forwarding, Snippets,
//! Known Hosts, Logs; this port drops the unimplemented Known Hosts row and keeps
//! [`Section::ALL`] in that nav order.
//!
//! Screens that are **not** left-panel rows keep their [`Section`] variants so
//! every screen stays reachable:
//!
//! * [`Section::Sftp`] — a fixed top-strip shortcut tab (`RDe`'s SFTP tab);
//! * [`Section::Keys`] — reached from the top-strip right cluster;
//! * [`Section::Settings`] / [`Section::Account`] / [`Section::Team`] — the
//!   top-strip right cluster (gear / person / team).
//!
//! Every variant owns a display [`label`](Section::label) and an
//! [`icon`](Section::icon) (a real bundled SVG file name resolved through
//! [`crate::assets::icon`]).
//!
//! The settings screen additionally has sub-tabs ([`SETTINGS_TABS`]).

/// One screen the shell can route to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    // --- left-panel rows (in `Section::ALL` order) ---
    Hosts,
    Keychain,
    PortForwarding,
    Snippets,
    Logs,
    // --- routable, but reached from the top strip, not the nav rail ---
    Keys,
    Sftp,
    Settings,
    Account,
    Team,
}

impl Section {
    /// The left-panel section rows, in nav order. The single source of truth for
    /// the rail (see [`sidebar_items`]) and for completeness tests.
    pub const ALL: &'static [Section] = &[
        Section::Hosts,
        Section::Keychain,
        Section::PortForwarding,
        Section::Snippets,
        Section::Logs,
    ];

    /// The label Termius shows in the left-panel rail ("Port Forwarding"…).
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Hosts => "Hosts",
            Self::Keychain => "Keychain",
            Self::PortForwarding => "Port Forwarding",
            Self::Snippets => "Snippets",
            Self::Logs => "Logs",
            Self::Keys => "Keys",
            Self::Sftp => "SFTP",
            Self::Settings => "Settings",
            Self::Account => "Account",
            Self::Team => "Team",
        }
    }

    /// The bundled SVG icon file name rendered before the label.
    ///
    /// The name is a bare file name from `crates/termius-ui/assets/icons`
    /// (e.g. `"host.svg"`); pass it straight to [`crate::assets::icon`]. Each
    /// section maps to the glyph the original Termius icon set shipped for that
    /// screen (host rack, keychain, port-forward rule, snippet sheet, session
    /// log, key, SFTP folder, gear, person, team).
    pub const fn icon(&self) -> &'static str {
        match self {
            Self::Hosts => "host.svg",
            Self::Keychain => "keys.svg",
            Self::PortForwarding => "PortForwarding.svg",
            Self::Snippets => "snippet.svg",
            Self::Logs => "session-log.svg",
            Self::Keys => "key.svg",
            Self::Sftp => "Sftp.svg",
            Self::Settings => "gear.svg",
            Self::Account => "person.svg",
            Self::Team => "team.svg",
        }
    }

    /// The nav row for this section (label + icon resolved from the section).
    pub const fn sidebar_item(&self) -> SidebarItem {
        SidebarItem::of(*self)
    }
}

/// Settings sub-tabs (rendered as a header row inside [`Section::Settings`]).
pub const SETTINGS_TABS: &[&str] =
    &["Terminal", "SFTP", "Logs", "Advanced", "Keyboard", "Team"];

/// One row of the left-panel nav rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarItem {
    pub section: Section,
    pub label: &'static str,
    pub icon: &'static str,
}

impl SidebarItem {
    /// The nav row for `section` (label + icon resolved from the section).
    pub const fn of(section: Section) -> Self {
        Self {
            section,
            label: section.label(),
            icon: section.icon(),
        }
    }
}

/// The left-panel sections, in nav order.
///
/// Returns [`Section::ALL`] so views iterating the rail stay identical to the
/// router; use [`Section::sidebar_item`] when the label/icon is needed.
pub fn sidebar_items() -> &'static [Section] {
    Section::ALL
}

/// The settings sub-tab labels.
pub fn settings_tabs() -> &'static [&'static str] {
    SETTINGS_TABS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_the_left_panel_rows_in_nav_order() {
        let expected = [
            Section::Hosts,
            Section::Keychain,
            Section::PortForwarding,
            Section::Snippets,
            Section::Logs,
        ];
        assert_eq!(Section::ALL.len(), expected.len());
        assert_eq!(Section::ALL, expected.as_slice());
        // The rail renders exactly what the router switches on.
        assert_eq!(sidebar_items(), Section::ALL);
    }

    #[test]
    fn labels_are_exact_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for section in Section::ALL {
            let label = section.label();
            assert!(!label.is_empty());
            assert!(seen.insert(label), "duplicate nav label `{label}`");
        }
        assert_eq!(Section::PortForwarding.label(), "Port Forwarding");
        assert_eq!(Section::Keychain.label(), "Keychain");
        assert_eq!(Section::Sftp.label(), "SFTP");
    }

    #[test]
    fn sidebar_items_resolve_from_sections() {
        for section in Section::ALL {
            let item = section.sidebar_item();
            assert_eq!(item.section, *section);
            assert_eq!(item.label, section.label());
            assert_eq!(item.icon, section.icon());
            // Real bundled SVG assets, never emoji/unicode glyphs.
            assert!(item.icon.ends_with(".svg"), "`{}` is not an SVG name", item.icon);
            assert_eq!(SidebarItem::of(*section), item);
        }
    }

    #[test]
    fn section_icons_are_bundled_assets() {
        for section in Section::ALL {
            assert!(
                crate::assets::has_icon(section.icon()),
                "missing icon asset `{}` for {:?}",
                section.icon(),
                section
            );
        }
    }

    #[test]
    fn section_icons_are_unique_and_ascii() {
        let mut seen = std::collections::HashSet::new();
        for section in Section::ALL {
            let icon = section.icon();
            assert!(icon.is_ascii(), "icon `{icon}` is not ASCII");
            assert!(seen.insert(icon), "duplicate section icon `{icon}`");
        }
    }

    #[test]
    fn off_nav_screens_stay_routable() {
        // These are reached from the top strip, so they must not be in the rail
        // but must keep a label + bundled icon.
        for section in [
            Section::Keys,
            Section::Sftp,
            Section::Settings,
            Section::Account,
            Section::Team,
        ] {
            assert!(!Section::ALL.contains(&section));
            assert!(!section.label().is_empty());
            assert!(
                crate::assets::has_icon(section.icon()),
                "missing icon asset `{}` for {:?}",
                section.icon(),
                section
            );
        }
    }

    #[test]
    fn settings_tabs_match_termius() {
        assert_eq!(settings_tabs(), SETTINGS_TABS);
        assert_eq!(
            settings_tabs(),
            ["Terminal", "SFTP", "Logs", "Advanced", "Keyboard", "Team"].as_slice()
        );
    }
}
