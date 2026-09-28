// SPDX-License-Identifier: AGPL-3.0-only
//! The pages of Settings: its categories, and the pages their rows open.
//!
//! The Python app's Settings (`renderSettingsPage` in `desktop/ui/app.js`)
//! was one long page of sections. The native page shows one [`Category`]
//! at a time, chosen in a list on the left, as the `ChatGPT` and T3 Code
//! settings do (SET-019), and long lists open as a [`Subpage`] of their
//! own. The categories follow the Python sections, grouped by what the
//! user wants to change:
//!
//! | Category | Python sections |
//! |---|---|
//! | Appearance | Appearance & layout |
//! | Search & indexing | Search cache, Folder sizes |
//! | Default apps | Default file explorer |
//! | Windows & tabs | Windows & tabs |
//! | Brave & downloads | Brave & downloads |
//! | About | OpenXplorer · License & source |

use crate::icons::Icon;

/// One category of settings, as the list on the left names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Category {
    /// Theme, text size, the right-click menu and the pane widths.
    Appearance,
    /// The search index and folder sizes.
    SearchAndIndexing,
    /// Which app opens folders, SMB links and ZIP files, and Show in folder.
    DefaultApps,
    /// Open windows and moving tabs between them.
    WindowsAndTabs,
    /// Brave's download folder.
    BraveAndDownloads,
    /// The build, updates and the licence.
    About,
}

impl Category {
    /// Every category, in the order the list shows them.
    pub(crate) const ALL: [Category; 6] = [
        Category::Appearance,
        Category::SearchAndIndexing,
        Category::DefaultApps,
        Category::WindowsAndTabs,
        Category::BraveAndDownloads,
        Category::About,
    ];

    /// The name of the category's page in the page stack, and in
    /// `OPENXPLORER_SETTINGS`.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Category::Appearance => "appearance",
            Category::SearchAndIndexing => "search",
            Category::DefaultApps => "default-apps",
            Category::WindowsAndTabs => "windows",
            Category::BraveAndDownloads => "brave",
            Category::About => "about",
        }
    }

    /// The name in the list and the page title.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::SearchAndIndexing => "Search & indexing",
            Category::DefaultApps => "Default apps",
            Category::WindowsAndTabs => "Windows & tabs",
            Category::BraveAndDownloads => "Brave & downloads",
            Category::About => "About",
        }
    }

    /// The line under the page title.
    pub(crate) const fn lead(self) -> &'static str {
        match self {
            Category::Appearance => "How OpenXplorer looks in every window.",
            Category::SearchAndIndexing => {
                "Keep a private list of file names so searches in these folders are instant."
            }
            Category::DefaultApps => {
                "Choose what opens when you open a folder, a network link or a ZIP file."
            }
            Category::WindowsAndTabs => "Open more windows and move tabs and files between them.",
            Category::BraveAndDownloads => "Save Brave's downloads in your Linux Downloads folder.",
            Category::About => "What this build is, how it is updated and its licence.",
        }
    }

    /// The glyph before the name in the list.
    pub(crate) const fn icon(self) -> Icon {
        match self {
            Category::Appearance => Icon::PaintBrush,
            Category::SearchAndIndexing => Icon::Search,
            Category::DefaultApps => Icon::Apps,
            Category::WindowsAndTabs => Icon::WindowMultiple,
            Category::BraveAndDownloads => Icon::ArrowDownload,
            Category::About => Icon::Info,
        }
    }

    /// The CSS class that gives the glyph its colour
    /// (`resources/skin/settings.css`).
    pub(crate) const fn css_class(self) -> &'static str {
        match self {
            Category::Appearance => "category-appearance",
            Category::SearchAndIndexing => "category-search",
            Category::DefaultApps => "category-default-apps",
            Category::WindowsAndTabs => "category-windows",
            Category::BraveAndDownloads => "category-brave",
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
    /// The Zorin and Brave setup and troubleshooting steps.
    Troubleshooting,
}

impl Subpage {
    /// Every sub-page.
    pub(crate) const ALL: [Subpage; 2] = [Subpage::IndexedFolders, Subpage::Troubleshooting];

    /// The name of the page in the page stack, and in
    /// `OPENXPLORER_SETTINGS`.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Subpage::IndexedFolders => "indexed-folders",
            Subpage::Troubleshooting => "troubleshooting",
        }
    }

    /// The category whose row opens it, which the back arrow returns to.
    pub(crate) const fn category(self) -> Category {
        match self {
            Subpage::IndexedFolders => Category::SearchAndIndexing,
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
    /// Settings opens on Appearance, the first category.
    fn default() -> Self {
        SettingsView::Category(Category::Appearance)
    }
}

impl SettingsView {
    /// The name of the view's page in the page stack.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            SettingsView::Category(category) => category.key(),
            SettingsView::Subpage(subpage) => subpage.key(),
        }
    }

    /// The view whose page is called `key`, or `None` for another name.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        let categories = Category::ALL.into_iter().map(SettingsView::Category);
        let subpages = Subpage::ALL.into_iter().map(SettingsView::Subpage);
        categories.chain(subpages).find(|view| view.key() == key)
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
            assert_eq!(SettingsView::from_key(view.key()), Some(view));
        }
        assert_eq!(SettingsView::from_key("general"), None);
    }

    #[test]
    fn page_names_titles_and_colours_are_unique() {
        let keys: HashSet<_> = Category::ALL.map(Category::key).into();
        let titles: HashSet<_> = Category::ALL.map(Category::title).into();
        let classes: HashSet<_> = Category::ALL.map(Category::css_class).into();
        assert_eq!(keys.len(), Category::ALL.len());
        assert_eq!(titles.len(), Category::ALL.len());
        assert_eq!(classes.len(), Category::ALL.len());
    }

    #[test]
    fn a_subpage_highlights_the_category_that_opens_it() {
        let indexed = SettingsView::Subpage(Subpage::IndexedFolders);
        assert_eq!(indexed.category(), Category::SearchAndIndexing);
        let troubleshooting = SettingsView::Subpage(Subpage::Troubleshooting);
        assert_eq!(troubleshooting.category(), Category::DefaultApps);
    }
}
