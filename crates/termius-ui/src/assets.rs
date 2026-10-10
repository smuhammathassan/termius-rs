//! assets — the bundled Termius UI resources served to GPUI.
//!
//! The original Electron renderer shipped its icon set as 400+ standalone
//! SVGs (`*.react-<hash>.svg`) and its UI type as **CircularXX**
//! (`CircularXXWeb-*.woff2`). Those files are reproduced here faithfully
//! (the woff2 faces were converted to ttf so CoreText/GPUI can load them).
//!
//! Registration happens in `termius-app`:
//! ```ignore
//! gpui::Application::new().with_assets(termius_ui::assets::TermiusAssets)…
//! ```
//! and [`load_fonts`] is called from [`crate::views::init`].
//!
//! The compile-time table is produced by `build.rs` (`$OUT_DIR/assets_gen.rs`).

use std::borrow::Cow;

use gpui::{svg, App, AssetSource, Result, SharedString, Svg};

include!(concat!(env!("OUT_DIR"), "/assets_gen.rs"));

/// The UI font family embedded by [`load_fonts`] (Lineto Circular, shipped by
/// Termius as `CircularXX`).
pub const UI_FONT: &str = "CircularXX";

/// GPUI asset source for the bundled SVG icons.
///
/// The original renderer referenced icons by hashed filename; this serves the
/// same glyphs from the embedded table under `icons/<name>.svg`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TermiusAssets;

impl AssetSource for TermiusAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICON_NAMES
            .iter()
            .map(|name| SharedString::from(format!("icons/{name}")))
            .collect())
    }
}

/// Register the embedded CircularXX faces with the text system.
///
/// Call once during app init, before the first window opens.
pub fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        tracing::error!(error = %err, "failed to load the CircularXX font faces");
    }
}

/// An SVG icon element for the bundled icon `name` (e.g. `"addCircle.svg"`).
///
/// The icon inherits its colour from the parent's `text_color` (GPUI renders
/// SVGs as a tinted alpha mask), so callers style it like text:
/// `icon("Lock.svg").w(px(16.)).h(px(16.)).text_color(theme.muted)`.
///
/// Unknown names render nothing (GPUI logs a lookup miss); use
/// [`has_icon`] in tests / debug assertions.
pub fn icon(name: &str) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}")))
}

/// Whether an icon with this bare file name is bundled.
pub fn has_icon(name: &str) -> bool {
    ICON_NAMES.iter().any(|candidate| *candidate == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_bundled() {
        assert!(ICON_NAMES.len() > 300, "expected the full Termius icon set");
        assert!(has_icon("addCircle.svg"));
        assert!(!has_icon("does-not-exist.svg"));
        assert!(ICONS.iter().all(|(name, bytes)| !bytes.is_empty() && name.starts_with("icons/")));
    }

    #[test]
    fn fonts_are_bundled() {
        assert_eq!(FONTS.len(), 4, "CircularXX Book/Medium/Bold/Black");
        assert!(FONTS.iter().all(|bytes| !bytes.is_empty()));
    }
}
