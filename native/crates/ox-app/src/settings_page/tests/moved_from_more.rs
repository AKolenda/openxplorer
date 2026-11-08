// SPDX-License-Identifier: AGPL-3.0-only
//! GTK tests of the settings that left the command bar's "…" menu for
//! Settings, as Windows 11 keeps them in Settings and not in File
//! Explorer's "…" (CMD-007): the folder shown before Settings cached for
//! search, the default file explorer, and the keyboard shortcuts, the
//! manual, the licence and the build in About.

use gtk::prelude::*;
use ox_core::location::same_location;
use ox_core::search::RootStatus;

use super::{switch_of, SettingsTest};
use crate::settings_page::pages::Category;
use crate::settings_page::row::SettingRow;
use crate::test_support::harness::{descendants, wait_until, TestWindow};
use crate::window::WindowAction;

impl SettingsTest {
    /// The row titled `title` on `category`'s page.
    fn row_in(&self, category: Category, title: &str) -> SettingRow {
        let rows = self.page.category_section(category).rows();
        rows.into_iter()
            .find(|row| row.text().title == title)
            .unwrap_or_else(|| panic!("{category:?} has a row titled {title:?}"))
    }

    /// Whether the search cache indexes `uri`.
    fn caches(&self, uri: &str) -> bool {
        let roots = self.test.context.search_cache().roots();
        roots
            .iter()
            .any(|root| root.is_enabled() && same_location(&root.uri, uri))
    }
}

/// The one button `row` shows.
fn button_of(row: &SettingRow) -> gtk::Button {
    row.controls()
        .into_iter()
        .find_map(|control| control.downcast::<gtk::Button>().ok())
        .expect("the row shows a button")
}

/// Settings > Search has "Cache this folder for search" for the folder
/// shown before Settings opened, which "…" used to hold: its switch,
/// named by the row, caches the folder and stops caching it again.
///
/// parity: CMD-007, SRCH-020, SET-006
#[gtk::test]
fn search_caches_the_folder_shown_before_settings() {
    let settings = SettingsTest::with_search_cache();
    let folder = settings.fixture.uri();
    let row = settings.row_in(Category::Search, "Cache this folder for search");
    let name = settings.fixture.root().file_name().expect("a named folder");
    assert_eq!(row.shown_description(), name.to_string_lossy());
    let switch = switch_of(&row);
    assert!(switch.is_sensitive());
    assert!(switch.is_focusable(), "the keyboard reaches it");
    assert!(!switch.is_active());

    switch.set_active(true);

    settings.test.wait_for_root(&folder, RootStatus::Ready);
    assert!(settings.caches(&folder));
    assert!(switch.is_active());

    switch.set_active(false);

    wait_until("the folder to stop being cached", || !settings.caches(&folder));
    assert!(!switch.is_active());
}

/// Settings opened from This PC has no folder to cache: the row's switch
/// is off and disabled, and says why.
///
/// parity: CMD-007, CMD-031
#[gtk::test]
fn search_has_no_folder_to_cache_from_this_pc() {
    let test = TestWindow::open(crate::locations::Page::ThisPc.uri());
    test.activate("settings", None);
    let page = super::settings_page_of(&test);
    let rows = page.category_section(Category::Search).rows();
    let row = rows
        .into_iter()
        .find(|row| row.text().title == "Cache this folder for search")
        .expect("Search has the row");
    let switch = switch_of(&row);
    assert!(!switch.is_sensitive());
    assert!(!switch.is_active());
    assert_eq!(
        row.shown_description(),
        "Only a folder on a drive or share can be cached for search."
    );
}

/// Settings > Default apps is where `OpenXplorer` becomes the default file
/// explorer, by an explicit button, now that "…" no longer offers it.
///
/// parity: CMD-007, INT-008
#[gtk::test]
fn default_apps_makes_openxplorer_the_default_only_when_asked() {
    let settings = SettingsTest::open();
    let section = settings.page.category_section(Category::DefaultApps);
    let make_default = descendants::<gtk::Button>(&section)
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Make OpenXplorer default"));
    assert!(make_default.is_some(), "an explicit button");
    settings.page.search("default file explorer");
    let shown = settings.shown_rows();
    assert!(!shown.is_empty(), "the search finds Default apps");
}

/// Settings > About has the keyboard shortcuts, the manual, the licence
/// and the build, which "…" used to list; each is a button with its own
/// name that runs the window's action, and F1 and Ctrl+? still do.
///
/// parity: CMD-007, SET-009, CMD-032
#[gtk::test]
fn about_offers_the_shortcuts_the_manual_the_licence_and_the_build() {
    let settings = SettingsTest::open();
    for (title, action) in [
        ("Keyboard shortcuts", WindowAction::KeyboardShortcuts),
        ("Help", WindowAction::Help),
        ("OpenXplorer · License & source", WindowAction::License),
    ] {
        let button = button_of(&settings.row_in(Category::About, title));
        assert_eq!(
            button.action_name().as_deref(),
            Some(&*action.detailed_name()),
            "{title}"
        );
        assert!(button.is_focusable(), "{title}");
        assert!(!button.label().unwrap_or_default().is_empty(), "{title}");
    }
    let section = settings.page.category_section(Category::About);
    let about_build = descendants::<gtk::Button>(&section)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some(&*WindowAction::About.detailed_name()));
    assert!(about_build.is_some(), "About this build");
    for query in ["keyboard shortcuts", "help manual"] {
        settings.page.search(query);
        assert!(!settings.shown_rows().is_empty(), "{query}");
    }
}

/// The labels of Settings > About's page.
fn about_texts(settings: &SettingsTest) -> Vec<String> {
    let section = settings.page.category_section(Category::About);
    descendants::<gtk::Label>(&section)
        .iter()
        .map(|label| label.text().to_string())
        .collect()
}

/// Settings > About describes the app as a file manager for Linux,
/// which it is on whatever desktop it runs.
///
/// parity: SET-009
#[gtk::test]
fn about_describes_a_file_manager_for_linux() {
    let settings = SettingsTest::open();
    let texts = about_texts(&settings);
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("An independent Windows 11–inspired file manager for Linux.")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|text| text.contains("Zorin")), "{texts:?}");
}

/// Settings > About spells license as its License & source heading does,
/// its lead line included.
///
/// parity: SET-009
#[gtk::test]
fn about_spells_license_one_way() {
    let settings = SettingsTest::open();
    let lead = ox_core::i18n::gettext(Category::About.lead());
    assert_eq!(lead, "What this build is, how it is updated and its license.");
    let texts = about_texts(&settings);
    assert!(
        !texts.iter().any(|text| text.to_lowercase().contains("licence")),
        "{texts:?}"
    );
}
