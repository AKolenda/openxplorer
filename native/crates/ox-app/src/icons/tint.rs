// SPDX-License-Identifier: AGPL-3.0-only
//! Place colours for glyphs: Home, This PC, Network and the standard
//! folders are drawn in colour, as the current app draws them.
//!
//! Ports the `color` options of `renderSidebar` in `desktop/ui/app.js`
//! (`im.style.color`), which are the same in both themes. The standard
//! folders' colours stay in ox-core ([`KnownFolder::glyph_color`]); the
//! others are the app's own. [`stylesheet`] turns every tint into a CSS
//! class, which the skin loads once for the display, so a glyph is tinted
//! by adding its classes ([`Tint::css_classes`]) and follows the CSS
//! cascade: the high-contrast rules draw every tinted glyph in the text
//! colour instead.

use std::fmt::Write as _;

use ox_core::places::KnownFolder;

/// The class every tinted glyph carries, for rules about all of them.
pub(crate) const TINTED_CLASS: &str = "tinted";

/// The colour of a place's glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Tint {
    /// Home: `#0078d4`.
    Home,
    /// This PC: `#347ba7`.
    ThisPc,
    /// Network: `#318db9`.
    Network,
    /// A standard folder in its Quick access colour.
    KnownFolder(KnownFolder),
}

impl Tint {
    /// The tint of a standard folder, `None` for a folder Quick access
    /// draws without a colour of its own.
    pub(crate) fn for_known_folder(folder: KnownFolder) -> Option<Tint> {
        folder.glyph_color().map(|_| Tint::KnownFolder(folder))
    }

    /// The colour, in CSS hex notation; `None` for a standard folder
    /// without one.
    fn color(self) -> Option<&'static str> {
        match self {
            Tint::Home => Some("#0078d4"),
            Tint::ThisPc => Some("#347ba7"),
            Tint::Network => Some("#318db9"),
            Tint::KnownFolder(folder) => folder.glyph_color(),
        }
    }

    /// The CSS class of this tint alone.
    fn css_class(self) -> &'static str {
        match self {
            Tint::Home => "tint-home",
            Tint::ThisPc => "tint-this-pc",
            Tint::Network => "tint-network",
            Tint::KnownFolder(KnownFolder::Desktop) => "tint-desktop",
            Tint::KnownFolder(KnownFolder::Downloads) => "tint-downloads",
            Tint::KnownFolder(KnownFolder::Documents) => "tint-documents",
            Tint::KnownFolder(KnownFolder::Pictures) => "tint-pictures",
            Tint::KnownFolder(KnownFolder::Music) => "tint-music",
            Tint::KnownFolder(KnownFolder::Videos) => "tint-videos",
            Tint::KnownFolder(KnownFolder::Templates) => "tint-templates",
            Tint::KnownFolder(KnownFolder::Public) => "tint-public",
        }
    }

    /// The classes a glyph in this tint carries: [`TINTED_CLASS`] and the
    /// tint's own.
    pub(crate) fn css_classes(self) -> [&'static str; 2] {
        [TINTED_CLASS, self.css_class()]
    }

    /// Every tint that has a colour.
    fn all() -> impl Iterator<Item = Tint> {
        let fixed = [Tint::Home, Tint::ThisPc, Tint::Network];
        let folders = KnownFolder::ALL.into_iter().filter_map(Tint::for_known_folder);
        fixed.into_iter().chain(folders)
    }
}

/// One CSS rule per tint, such as `.tinted.tint-home { color: #0078d4; }`.
/// The colours are the same in both themes, as in the current app.
pub(crate) fn stylesheet() -> String {
    let mut css = String::new();
    for tint in Tint::all() {
        let Some(color) = tint.color() else {
            continue;
        };
        let class = tint.css_class();
        writeln!(css, ".{TINTED_CLASS}.{class} {{ color: {color}; }}")
            .expect("writing to a String cannot fail");
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: LOOK-015
    #[test]
    fn every_quick_access_folder_is_tinted_with_its_ox_core_colour() {
        let css = stylesheet();
        for folder in KnownFolder::QUICK_ACCESS {
            let tint = Tint::for_known_folder(folder).expect("Quick access folders have a colour");
            let color = folder.glyph_color().expect("Quick access folders have a colour");
            let rule = format!(".tinted.{} {{ color: {color}; }}", tint.css_class());
            assert!(css.contains(&rule), "{folder:?}: {css}");
        }
    }

    #[test]
    fn home_this_pc_and_network_keep_the_current_apps_colours() {
        let css = stylesheet();
        assert!(css.contains(".tinted.tint-home { color: #0078d4; }"));
        assert!(css.contains(".tinted.tint-this-pc { color: #347ba7; }"));
        assert!(css.contains(".tinted.tint-network { color: #318db9; }"));
    }

    #[test]
    fn a_folder_without_a_colour_has_no_tint() {
        assert_eq!(Tint::for_known_folder(KnownFolder::Public), None);
        assert_eq!(Tint::Home.css_classes(), ["tinted", "tint-home"]);
    }

    #[gtk::test]
    fn the_tints_parse_as_css() {
        let provider = gtk::CssProvider::new();
        let errors = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = std::rc::Rc::clone(&errors);
        provider.connect_parsing_error(move |_, _, _| counted.set(counted.get() + 1));
        provider.load_from_string(&stylesheet());
        assert_eq!(errors.get(), 0);
    }
}
