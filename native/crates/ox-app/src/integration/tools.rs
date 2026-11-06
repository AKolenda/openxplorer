// SPDX-License-Identifier: AGPL-3.0-only
//! Dolphin's tools that hand files to another application: Compare Files
//! (OPEN-023) and Open Preferred Search Tool (OPEN-024).
//!
//! Each tool is an installed application from a short list, found by its
//! desktop ID, as Dolphin looks for Kompare and `KFind`. It is launched
//! with the files through GIO, never through a command line built here.

use gtk::gio;
use gtk::prelude::*;

/// Applications that compare two files, in the order they are preferred.
const DIFF_TOOLS: [&str; 4] = [
    "org.gnome.Meld.desktop",
    "org.kde.kompare.desktop",
    "org.kde.kdiff3.desktop",
    "diffuse.desktop",
];

/// Applications that search a folder, in the order they are preferred.
const SEARCH_TOOLS: [&str; 3] = [
    "org.kde.kfind.desktop",
    "org.xfce.Catfish.desktop",
    "gnome-search-tool.desktop",
];

/// Why Compare Files did nothing.
pub(crate) const NO_DIFF_TOOL: &str =
    crate::i18n::message_id("No file comparison tool is installed. Install Meld to compare files.");

/// Why Open Preferred Search Tool did nothing.
pub(crate) const NO_SEARCH_TOOL: &str = crate::i18n::message_id("No search tool is installed.");

/// A tool Dolphin offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    /// Compare Files.
    Diff,
    /// Open Preferred Search Tool.
    Search,
}

impl Tool {
    /// The desktop IDs of the applications that can be the tool.
    const fn candidates(self) -> &'static [&'static str] {
        match self {
            Tool::Diff => &DIFF_TOOLS,
            Tool::Search => &SEARCH_TOOLS,
        }
    }

    /// Why the tool did nothing when none is installed.
    pub(crate) fn missing(self) -> &'static str {
        match self {
            Tool::Diff => ox_core::i18n::gettext_static(NO_DIFF_TOOL),
            Tool::Search => ox_core::i18n::gettext_static(NO_SEARCH_TOOL),
        }
    }

    /// The first installed application that can be the tool and takes
    /// files or addresses.
    pub(crate) fn installed(self) -> Option<gio::AppInfo> {
        self.candidates()
            .iter()
            .filter_map(|id| installed_application(id))
            .find(|app| app.supports_files() || app.supports_uris())
    }
}

/// The installed application whose desktop ID is `id`, such as
/// `org.gnome.Meld.desktop`.
pub(crate) fn installed_application(id: &str) -> Option<gio::AppInfo> {
    gio::AppInfo::all()
        .into_iter()
        .find(|application| application.id().as_deref() == Some(id))
}

/// KDE's Info Center, Plasma's system information app.
const KDE_INFO_CENTER: &str = "org.kde.kinfocenter.desktop";

/// GNOME Settings' About page (`gnome-control-center system about`).
const GNOME_ABOUT: &str = "gnome-about-panel.desktop";

/// GNOME Settings' System page, which holds About since GNOME 46.
const GNOME_SYSTEM: &str = "gnome-system-panel.desktop";

/// The desktop IDs of the system information apps, the current desktop's
/// own first: KDE's Info Center on Plasma, else GNOME Settings' About page,
/// then its System page. `current_desktop` is `XDG_CURRENT_DESKTOP`.
fn system_information_candidates(current_desktop: &str) -> [&'static str; 3] {
    let is_kde = current_desktop
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("KDE"));
    if is_kde {
        [KDE_INFO_CENTER, GNOME_ABOUT, GNOME_SYSTEM]
    } else {
        [GNOME_ABOUT, GNOME_SYSTEM, KDE_INFO_CENTER]
    }
}

/// The desktop's system information app, which This PC's "…" > Properties
/// opens, as Windows' opens the system's properties (CMD-007); `None`
/// where none is installed, and the item is then left out.
pub(crate) fn system_information() -> Option<gio::AppInfo> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    system_information_candidates(&desktop)
        .into_iter()
        .find_map(installed_application)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plasma opens KDE's Info Center first; GNOME, Zorin and the rest
    /// GNOME Settings' About page, then its System page.
    ///
    /// parity: CMD-007
    #[test]
    fn the_current_desktops_system_information_app_comes_first() {
        assert_eq!(system_information_candidates("KDE")[0], KDE_INFO_CENTER);
        assert_eq!(system_information_candidates("kde")[0], KDE_INFO_CENTER);
        assert_eq!(
            system_information_candidates("zorin:GNOME"),
            [GNOME_ABOUT, GNOME_SYSTEM, KDE_INFO_CENTER]
        );
        assert_eq!(system_information_candidates("")[0], GNOME_ABOUT);
    }
}
