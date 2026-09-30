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
pub(crate) const NO_DIFF_TOOL: &str = "No file comparison tool is installed. Install Meld to compare files.";

/// Why Open Preferred Search Tool did nothing.
pub(crate) const NO_SEARCH_TOOL: &str = "No search tool is installed.";

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
    pub(crate) const fn missing(self) -> &'static str {
        match self {
            Tool::Diff => NO_DIFF_TOOL,
            Tool::Search => NO_SEARCH_TOOL,
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
