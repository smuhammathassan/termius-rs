//! navigation — the sidebar section model (the Electron renderer's left nav).
//!
//! [`Section`] is the routing enum [`TermiusState`](crate::app_state::TermiusState)
//! stores as `current_section` and the shell switches on. Every variant owns a
//! display [`label`](Section::label) and an [`icon`](Section::icon), and the
//! full sidebar order lives in [`Section::ALL`] (returned by
//! [`sidebar_items`]) so the nav cannot drift out of sync with the router.
//!
//! The settings screen additionally has sub-tabs
//! ([`SETTINGS_TABS`]) — the Settings section renders them as a header row.

/// One screen the left sidebar can route to.
///
/// Mirrors the Termius Electron sidebar: Hosts, Snippets, Keys, Port
/// Forwarding, SFTP, Keychain, Team, Logs, Settings, Account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Hosts,
    Snippets,
    Keys,
    PortForwarding,
    Sftp,
    Keychain,
    Team,
    Logs,
    Settings,
    Account,
}

impl Section {
    /// Every section, in sidebar order. The single source of truth for the
    /// nav (see [`sidebar_items`]) and for completeness tests.
    pub const ALL: &'static [Section] = &[
        Section::Hosts,
        Section::Snippets,
        Section::Keys,
        Section::PortForwarding,
        Section::Sftp,
        Section::Keychain,
        Section::Team,
        Section::Logs,
        Section::Settings,
        Section::Account,
    ];

    /// The label Termius shows in the sidebar ("Port Forwarding", "SFTP"…).
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Hosts => "Hosts",
            Self::Snippets => "Snippets",
            Self::Keys => "Keys",
            Self::PortForwarding => "Port Forwarding",
            Self::Sftp => "SFTP",
            Self::Keychain => "Keychain",
            Self::Team => "Team",
            Self::Logs => "Logs",
            Self::Settings => "Settings",
            Self::Account => "Account",
        }
    }

    /// A single-glyph icon rendered before the label.
    ///
    /// PORT-TODO: swap for the Termius SVG assets once gpui's image/svg
    /// pipeline is wired (`cx.load_asset`); emoji keeps the nav readable with
    /// zero font dependencies.
    pub const fn icon(&self) -> &'static str {
        match self {
            Self::Hosts => "🖥",
            Self::Snippets => "📝",
            Self::Keys => "🔑",
            Self::PortForwarding => "🔗",
            Self::Sftp => "📁",
            Self::Keychain => "🗝",
            Self::Team => "👥",
            Self::Logs => "📋",
            Self::Settings => "⚙",
            Self::Account => "👤",
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

/// One row of the left sidebar nav.
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

/// The sidebar sections, in nav order.
///
/// Returns [`Section::ALL`] so views iterating the nav stay identical to the
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
    fn all_lists_every_section_in_sidebar_order() {
        let expected = [
            Section::Hosts,
            Section::Snippets,
            Section::Keys,
            Section::PortForwarding,
            Section::Sftp,
            Section::Keychain,
            Section::Team,
            Section::Logs,
            Section::Settings,
            Section::Account,
        ];
        assert_eq!(Section::ALL.len(), expected.len());
        assert_eq!(Section::ALL, expected.as_slice());
        // The nav renders exactly what the router switches on.
        assert_eq!(sidebar_items(), Section::ALL);
    }

    #[test]
    fn labels_are_exact_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for section in Section::ALL {
            let label = section.label();
            assert!(!label.is_empty());
            assert!(seen.insert(label), "duplicate sidebar label `{label}`");
        }
        assert_eq!(Section::PortForwarding.label(), "Port Forwarding");
        assert_eq!(Section::Sftp.label(), "SFTP");
    }

    #[test]
    fn sidebar_items_resolve_from_sections() {
        for section in Section::ALL {
            let item = section.sidebar_item();
            assert_eq!(item.section, *section);
            assert_eq!(item.label, section.label());
            assert_eq!(item.icon, section.icon());
            assert!(!item.icon.is_empty());
            assert_eq!(SidebarItem::of(*section), item);
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
