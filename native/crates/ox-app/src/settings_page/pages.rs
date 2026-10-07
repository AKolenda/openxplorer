// SPDX-License-Identifier: AGPL-3.0-only
//! The pages of Settings: its categories, and the pages their rows open.
//!
//! The Python app's Settings (`renderSettingsPage` in `v2.0.0:desktop/ui/app.js`)
//! was one long page of sections. The native page shows one [`Category`]
//! at a time, chosen in a list on the left, as the `ChatGPT` and T3 Code
//! settings do (SET-019), and long lists open as a [`Subpage`] of their
//! own. The categories follow the owner's settings mockup, grouped by what
//! the user wants to change; every setting of the Python sections is on
//! one of them:
//!
//! | Category | Python sections |
//! |---|---|
//! | General | Windows & tabs |
//! | Appearance | Appearance & layout |
//! | Files & folders | Appearance & layout, Folder sizes |
//! | ZIP & archives | Default file explorer (ZIP files) |
//! | Confirmations | (Dolphin's Confirmations) |
//! | Search | Search cache |
//! | Default apps | Default file explorer, Brave & downloads |
//! | About | OpenXplorer · License & source |

use crate::icons::Icon;

/// One category of settings, as the list on the left names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Category {
    /// Start up, windows and tabs, the address bar and dragging.
    General,
    /// Theme, text size, the right-click menu and the pane widths.
    Appearance,
    /// Folder views, the details view, previews and folder sizes.
    FilesAndFolders,
    /// How ZIP and TAR archives open.
    Archives,
    /// The questions asked before deleting, running and closing.
    Confirmations,
    /// The search index.
    Search,
    /// Which app opens folders, SMB links and ZIP files, Show in folder,
    /// the file dialogs and Brave's downloads.
    DefaultApps,
    /// The build, updates and the licence.
    About,
}

impl Category {
    /// Every category, in the order the list shows them.
    pub(crate) const ALL: [Category; 8] = [
        Category::General,
        Category::Appearance,
        Category::FilesAndFolders,
        Category::Archives,
        Category::Confirmations,
        Category::Search,
        Category::DefaultApps,
        Category::About,
    ];

    /// The name of the category's page in the page stack, and in
    /// `OPENXPLORER_SETTINGS`.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Category::General => "general",
            Category::Appearance => "appearance",
            Category::FilesAndFolders => "files",
            Category::Archives => "archives",
            Category::Confirmations => "confirmations",
            Category::Search => "search",
            Category::DefaultApps => "default-apps",
            Category::About => "about",
        }
    }

    /// The name in the list and the page title.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Category::General => crate::i18n::message_id("General"),
            Category::Appearance => crate::i18n::message_id("Appearance"),
            Category::FilesAndFolders => crate::i18n::message_id("Files & folders"),
            Category::Archives => crate::i18n::message_id("ZIP & archives"),
            Category::Confirmations => crate::i18n::message_id("Confirmations"),
            Category::Search => crate::i18n::message_id("Search"),
            Category::DefaultApps => crate::i18n::message_id("Default apps"),
            Category::About => crate::i18n::message_id("About"),
        }
    }

    /// The line under the page title.
    pub(crate) const fn lead(self) -> &'static str {
        match self {
            Category::General => {
                crate::i18n::message_id("How windows and tabs open, and what the address bar shows.")
            }
            Category::Appearance => crate::i18n::message_id("How OpenXplorer looks in every window."),
            Category::FilesAndFolders => {
                crate::i18n::message_id("How folders, their items and previews are shown.")
            }
            Category::Archives => crate::i18n::message_id("How ZIP and TAR archives open."),
            Category::Confirmations => {
                crate::i18n::message_id("What OpenXplorer asks before it deletes, runs or closes.")
            }
            Category::Search => crate::i18n::message_id(
                "Keep a private list of file names so searches in these folders are instant.",
            ),
            Category::DefaultApps => crate::i18n::message_id(
                "Choose what opens when you open a folder, a network link or a ZIP file.",
            ),
            Category::About => {
                crate::i18n::message_id("What this build is, how it is updated and its licence.")
            }
        }
    }

    /// The glyph before the name in the list.
    pub(crate) const fn icon(self) -> Icon {
        match self {
            Category::General => Icon::WindowMultiple,
            Category::Appearance => Icon::PaintBrush,
            Category::FilesAndFolders => Icon::Folder,
            Category::Archives => Icon::FolderZip,
            Category::Confirmations => Icon::ShieldLock,
            Category::Search => Icon::Search,
            Category::DefaultApps => Icon::Apps,
            Category::About => Icon::Info,
        }
    }

    /// The CSS class that gives the glyph its colour
    /// (`resources/skin/settings.css`).
    pub(crate) const fn css_class(self) -> &'static str {
        match self {
            Category::General => "category-general",
            Category::Appearance => "category-appearance",
            Category::FilesAndFolders => "category-files",
            Category::Archives => "category-archives",
            Category::Confirmations => "category-confirmations",
            Category::Search => "category-search",
            Category::DefaultApps => "category-default-apps",
            Category::About => "category-about",
        }
    }
}

/// A page a row of a category opens, with a back arrow to its category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Subpage {
    /// The folders to index for search (the Python app's inline cache
    /// list).
    IndexedFolders,
    /// How folder sizes are counted (the Python "Folder sizes" help).
    FolderSizes,
    /// The Zorin and Brave setup and troubleshooting steps.
    Troubleshooting,
}

impl Subpage {
    /// Every sub-page.
    pub(crate) const ALL: [Subpage; 3] = [
        Subpage::IndexedFolders,
        Subpage::FolderSizes,
        Subpage::Troubleshooting,
    ];

    /// The name of the page in the page stack, and in
    /// `OPENXPLORER_SETTINGS`.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Subpage::IndexedFolders => "indexed-folders",
            Subpage::FolderSizes => "folder-sizes",
            Subpage::Troubleshooting => "troubleshooting",
        }
    }

    /// The category whose row opens it, which the back arrow returns to.
    pub(crate) const fn category(self) -> Category {
        match self {
            Subpage::IndexedFolders => Category::Search,
            Subpage::FolderSizes => Category::FilesAndFolders,
            Subpage::Troubleshooting => Category::DefaultApps,
        }
    }
}

/// What the right side of Settings shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SettingsView {
    /// A category's settings.
    Category(Category),
    /// A page one of its rows opened.
    Subpage(Subpage),
}

impl Default for SettingsView {
    /// Settings opens on General, the first category.
    fn default() -> Self {
        SettingsView::Category(Category::General)
    }
}

impl SettingsView {
    /// The name of the view's page in the page stack.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            SettingsView::Category(category) => category.as_str(),
            SettingsView::Subpage(subpage) => subpage.as_str(),
        }
    }

    /// The view whose page is called `key`, or `None` for another name.
    /// The names of the categories before the settings were rearranged
    /// still lead to where their settings are now.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        let moved = match key {
            "windows" => Some(Category::General),
            "brave" => Some(Category::DefaultApps),
            _ => None,
        };
        if let Some(category) = moved {
            return Some(SettingsView::Category(category));
        }
        let categories = Category::ALL.into_iter().map(SettingsView::Category);
        let subpages = Subpage::ALL.into_iter().map(SettingsView::Subpage);
        categories.chain(subpages).find(|view| view.as_str() == key)
    }

    /// The category the list highlights for this view.
    pub(crate) const fn category(self) -> Category {
        match self {
            SettingsView::Category(category) => category,
            SettingsView::Subpage(subpage) => subpage.category(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_view_is_found_by_its_page_name() {
        let categories = Category::ALL.map(SettingsView::Category);
        let subpages = Subpage::ALL.map(SettingsView::Subpage);
        for view in categories.into_iter().chain(subpages) {
            assert_eq!(SettingsView::from_key(view.as_str()), Some(view));
        }
        assert_eq!(SettingsView::from_key("windows-and-more"), None);
        assert_eq!(
            SettingsView::from_key("windows"),
            Some(SettingsView::Category(Category::General)),
            "an earlier name still leads to its settings"
        );
    }

    #[test]
    fn page_names_titles_and_colours_are_unique() {
        let keys: HashSet<_> = Category::ALL.map(Category::as_str).into();
        let titles: HashSet<_> = Category::ALL.map(Category::title).into();
        let classes: HashSet<_> = Category::ALL.map(Category::css_class).into();
        assert_eq!(keys.len(), Category::ALL.len());
        assert_eq!(titles.len(), Category::ALL.len());
        assert_eq!(classes.len(), Category::ALL.len());
    }

    #[test]
    fn a_subpage_highlights_the_category_that_opens_it() {
        let indexed = SettingsView::Subpage(Subpage::IndexedFolders);
        assert_eq!(indexed.category(), Category::Search);
        let folder_sizes = SettingsView::Subpage(Subpage::FolderSizes);
        assert_eq!(folder_sizes.category(), Category::FilesAndFolders);
        let troubleshooting = SettingsView::Subpage(Subpage::Troubleshooting);
        assert_eq!(troubleshooting.category(), Category::DefaultApps);
    }
}
