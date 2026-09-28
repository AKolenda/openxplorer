// SPDX-License-Identifier: AGPL-3.0-only
//! GTK tests of the Settings page: the categories, the settings search,
//! the sub-pages, and rows that read and write the settings file the Python
//! app shares.
//!
//! Each test opens a window on the standard fixture with Settings in front
//! and drives the page as the user would: choosing categories, typing a
//! search, and clicking, switching and choosing options. The window-level
//! behaviour (the tab, "Back to files", Ctrl+,) is tested in
//! `window::tests::settings`.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{ContextMenu, Settings, Theme};

use super::category_row::CategoryRow;
use super::choice_list::ChoiceList;
use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::row::{Availability, SettingRow};
use super::status_card::StatusCard;
use super::SettingsPage;
use crate::test_support::harness::{descendants, skin, wait_until, Fixture, TestWindow, ThemeGuard};
use crate::test_support::python::{python_preference, python_saves_preferences};
use crate::text_size::TextSize;
use crate::theme::ThemePreference;

/// A window on the standard fixture with Settings open in front.
struct SettingsTest {
    /// The window and its settings directory.
    test: TestWindow,
    /// The window's Settings page.
    page: SettingsPage,
    /// The folder the window showed before Settings.
    fixture: Fixture,
}

impl SettingsTest {
    fn open() -> Self {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.activate("settings", None);
        let page = descendants::<SettingsPage>(&test.window)
            .into_iter()
            .next()
            .expect("the window has a Settings page");
        Self { test, page, fixture }
    }

    /// The row titled `title`, in whichever category it is.
    fn row(&self, title: &str) -> SettingRow {
        let rows = Category::ALL
            .into_iter()
            .flat_map(|category| self.page.category_section(category).rows());
        rows.into_iter()
            .find(|row| row.text().title == title)
            .unwrap_or_else(|| panic!("Settings has a row titled {title:?}"))
    }

    /// The preferences in the settings file, as another process reads them.
    fn saved_preferences(&self) -> ox_core::settings::Preferences {
        Settings::open(self.test.settings_directory())
            .data()
            .preferences
            .clone()
    }

    /// The titles of the rows the category page shown now shows.
    fn shown_rows(&self) -> Vec<&'static str> {
        let category = self.page.view().category();
        let rows = self.page.category_section(category).rows();
        let shown = rows.into_iter().filter(WidgetExt::is_visible);
        shown.map(|row| row.text().title).collect()
    }

    /// Whether the category page shown now shows its status card.
    fn shows_status_card(&self) -> bool {
        let category = self.page.view().category();
        let section = self.page.category_section(category);
        let cards = descendants::<StatusCard>(&section);
        cards.iter().any(WidgetExt::is_visible)
    }

    /// The categories the list shows now.
    fn listed_categories(&self) -> Vec<Category> {
        let rows = self.page.category_rows().into_iter();
        let listed = rows.filter(WidgetExt::is_child_visible);
        listed.map(|row| row.category()).collect()
    }

    /// The category the list shows as chosen.
    fn chosen_category(&self) -> Option<Category> {
        let chosen = self.page.imp().category_list.selected_row();
        chosen.and_downcast::<CategoryRow>().map(|row| row.category())
    }
}

/// The drop-down a row shows.
fn choices_of(row: &SettingRow) -> ChoiceList {
    let button = row
        .controls()
        .into_iter()
        .next()
        .and_downcast::<gtk::MenuButton>();
    button
        .and_then(|button| button.popover())
        .and_downcast::<ChoiceList>()
        .expect("the row shows a drop-down")
}

/// The switch a row shows.
fn switch_of(row: &SettingRow) -> gtk::Switch {
    row.controls()
        .into_iter()
        .find_map(|control| control.downcast::<gtk::Switch>().ok())
        .expect("the row shows a switch")
}

/// Keeps the shared skin's text size for the length of a test that
/// changes it.
struct TextSizeGuard(TextSize);

impl TextSizeGuard {
    fn keep() -> Self {
        Self(skin().text_size())
    }
}

impl Drop for TextSizeGuard {
    fn drop(&mut self) {
        skin().set_text_size(self.0);
    }
}

/// parity: SET-019
#[gtk::test]
fn choosing_a_category_shows_only_that_category() {
    let settings = SettingsTest::open();
    let pages = &settings.page.imp().pages;
    assert_eq!(pages.visible_child_name().as_deref(), Some("appearance"));

    let default_apps = settings
        .page
        .category_list_row(Category::DefaultApps)
        .expect("the list has Default apps");
    settings.page.imp().category_list.select_row(Some(&default_apps));

    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::DefaultApps)
    );
    assert_eq!(pages.visible_child_name().as_deref(), Some("default-apps"));
    let shown_pages = descendants::<gtk::ScrolledWindow>(&pages.get())
        .into_iter()
        .filter(|page| page.is_child_visible() && page.is_visible() && page.is_mapped());
    assert_eq!(shown_pages.count(), 1, "one category at a time");
}

/// parity: SET-019
#[gtk::test]
fn arrow_keys_move_through_the_categories() {
    let settings = SettingsTest::open();
    let list = &settings.page.imp().category_list;
    // Tab moves keyboard focus onto the chosen category first.
    let chosen = list.selected_row().expect("a category is chosen");
    assert!(chosen.grab_focus());
    list.emit_move_cursor(gtk::MovementStep::DisplayLines, 1, false, false);
    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::SearchAndIndexing)
    );
    list.emit_move_cursor(gtk::MovementStep::DisplayLines, 1, false, false);
    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::DefaultApps)
    );
}

/// A search and the category and rows it shows.
struct SearchCase {
    typed: &'static str,
    categories: &'static [Category],
    shown: &'static [&'static str],
}

/// Ported from the keywords of `appendV07Settings` in `desktop/ui/app.js`,
/// which make "zoom", "watch live" and "dolphin" find their settings.
///
/// parity: SET-019
#[gtk::test]
fn the_search_filters_rows_across_every_category() {
    let settings = SettingsTest::open();
    let cases = [
        SearchCase {
            typed: "zoom",
            categories: &[Category::Appearance],
            shown: &["Text size"],
        },
        SearchCase {
            typed: "watch live",
            categories: &[Category::SearchAndIndexing],
            shown: &["Watch folders for live changes"],
        },
        SearchCase {
            typed: "dolphin",
            categories: &[Category::DefaultApps],
            shown: &["Folders"],
        },
        SearchCase {
            typed: "brave",
            categories: &[Category::DefaultApps, Category::BraveAndDownloads],
            shown: &[
                "Include Show in folder",
                "Show in folder",
                "Zorin + Brave setup and troubleshooting",
                "Disable Show in folder",
            ],
        },
    ];
    for case in cases {
        settings.page.search(case.typed);
        assert_eq!(settings.listed_categories(), case.categories, "{}", case.typed);
        assert_eq!(settings.shown_rows(), case.shown, "{}", case.typed);
    }
    let count = &settings.page.imp().match_count;
    assert!(count.is_visible());
    assert_eq!(count.text(), "5 matching settings");
}

/// A search for what a setting shows, and the category, status card and
/// rows it finds.
struct ShownTextCase {
    typed: &'static str,
    category: Category,
    shows_status_card: bool,
    shown: &'static [&'static str],
}

/// Ported from `settingsSearch` in `desktop/ui/app.js`, which matched an
/// element's visible text too: buttons, drop-down options and headings
/// find their settings.
///
/// parity: SET-019
#[gtk::test]
fn the_search_finds_what_buttons_options_and_headings_show() {
    let settings = SettingsTest::open();
    let cases = [
        ShownTextCase {
            typed: "refresh all",
            category: Category::SearchAndIndexing,
            shows_status_card: true,
            shown: &["Network / fallback checks"],
        },
        ShownTextCase {
            typed: "make openxplorer default",
            category: Category::DefaultApps,
            shows_status_card: true,
            shown: &["Include Show in folder", "Also open ZIP files in OpenXplorer"],
        },
        ShownTextCase {
            typed: "refresh status",
            category: Category::DefaultApps,
            shows_status_card: false,
            shown: &["Folders", "SMB links", "ZIP files"],
        },
        ShownTextCase {
            typed: "compact actions",
            category: Category::Appearance,
            shows_status_card: false,
            shown: &["Right-click menu"],
        },
    ];
    for case in cases {
        settings.page.search(case.typed);
        let view = settings.page.view();
        assert_eq!(view, SettingsView::Category(case.category), "{}", case.typed);
        assert_eq!(
            settings.shows_status_card(),
            case.shows_status_card,
            "{}",
            case.typed
        );
        assert_eq!(settings.shown_rows(), case.shown, "{}", case.typed);
    }
}

/// Enter on a search that finds a status card jumps to the card.
///
/// parity: SET-019
#[gtk::test]
fn enter_on_a_status_card_match_jumps_to_the_card() {
    let settings = SettingsTest::open();
    settings.page.search("refresh all");

    settings.page.imp().search_entry.emit_activate();

    let section = settings.page.category_section(Category::SearchAndIndexing);
    let card = descendants::<StatusCard>(&section)
        .into_iter()
        .next()
        .expect("Search & indexing has a status card");
    assert!(card.has_css_class("jump-target"));
    assert_eq!(settings.page.imp().match_count.text(), "2 matching settings");
}

/// parity: SET-019
#[gtk::test]
fn a_search_that_matches_nothing_says_so() {
    let settings = SettingsTest::open();
    settings.page.search("no such setting");
    let imp = settings.page.imp();
    assert_eq!(imp.match_count.text(), "No matching settings");
    assert_eq!(imp.pages.visible_child_name().as_deref(), Some("no-matches"));
    assert!(settings.listed_categories().is_empty());
}

/// More > Default file explorer… names its category, so it shows all of
/// it rather than the rows an earlier search left, which may be none.
///
/// parity: SET-019
#[gtk::test]
fn default_file_explorer_during_a_search_shows_all_of_default_apps() {
    let settings = SettingsTest::open();
    settings.page.search("zoom");

    settings.test.activate("default-file-explorer", None);

    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::DefaultApps)
    );
    let imp = settings.page.imp();
    assert_eq!(imp.search_entry.text(), "", "the search is emptied");
    assert_eq!(imp.pages.visible_child_name().as_deref(), Some("default-apps"));
    assert_eq!(settings.listed_categories(), Category::ALL);
    let every_row = settings.page.category_section(Category::DefaultApps).rows();
    assert_eq!(settings.shown_rows().len(), every_row.len());
}

/// Typing on the page outside a text field goes to the settings search.
///
/// parity: SET-019
#[gtk::test]
fn typing_on_the_page_starts_a_settings_search() {
    let settings = SettingsTest::open();
    let capture = settings.page.imp().search_entry.key_capture_widget();
    assert_eq!(capture, Some(settings.page.clone().upcast()));
}

/// parity: SET-019
#[gtk::test]
fn enter_in_the_search_jumps_to_the_first_match() {
    let settings = SettingsTest::open();
    settings.page.show_view(SettingsView::Category(Category::About));
    settings.page.search("network interval");

    settings.page.imp().search_entry.emit_activate();

    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::SearchAndIndexing)
    );
    let row = settings.row("Network / fallback checks");
    assert!(row.has_css_class("jump-target"));
    let focus = GtkWindowExt::focus(&settings.test.window).expect("a control has focus");
    assert!(focus.is_ancestor(&row), "the row's drop-down has keyboard focus");
}

/// parity: SET-019
#[gtk::test]
fn escape_leaves_the_search_and_shows_every_row_again() {
    let settings = SettingsTest::open();
    settings.page.search("zoom");

    settings.page.imp().search_entry.emit_stop_search();

    assert_eq!(settings.page.imp().search_entry.text(), "");
    assert!(!settings.page.imp().match_count.is_visible());
    assert_eq!(settings.listed_categories(), Category::ALL);
    assert_eq!(
        settings.shown_rows(),
        [
            "Theme",
            "Text size",
            "Right-click menu",
            "Sidebar and column widths"
        ]
    );
}

/// "Every setting the current app offers stays available" (SET-019): each
/// setting, action and piece of advice of `renderSettingsPage` and
/// `appendV07Settings` has a row.
///
/// parity: SET-019
#[gtk::test]
fn every_setting_of_the_python_page_has_a_row() {
    let settings = SettingsTest::open();
    let python_settings = [
        "Theme",
        "Text size",
        "Right-click menu",
        "Sidebar and column widths",
        "Folders to index",
        "Watch folders for live changes",
        "Network / fallback checks",
        "Calculate folder sizes",
        "Folders",
        "SMB links",
        "ZIP files",
        "Include Show in folder",
        "Also open ZIP files in OpenXplorer",
        "Use OpenXplorer for ZIPs",
        "Show in folder",
        "Zorin + Brave setup and troubleshooting",
        "Restore previous",
        "Restore ZIP handler",
        "Disable Show in folder",
        "Open windows",
        "New window",
        "Move tabs between windows",
        "Use Linux Downloads in Brave",
        "OpenXplorer · License & source",
    ];
    for title in python_settings {
        settings.row(title);
    }
}

/// Checks that `row` of `group` is enabled or disabled as its availability
/// says, and names its milestone, on itself or in the group's heading.
fn assert_row_follows_its_availability(row: &SettingRow, group: &SettingsGroup) {
    let title = row.text().title;
    let notice = row.shown_notice().or_else(|| group.shown_notice());
    let controls = row.controls();
    let enabled = controls.iter().all(WidgetExt::is_sensitive);
    let disabled = controls.iter().all(|control| !control.is_sensitive());
    let milestone = match row.availability() {
        Availability::Ready => {
            assert!(enabled, "{title} works");
            assert_eq!(notice, None, "{title} needs no milestone");
            return;
        }
        Availability::Unported(milestone) => {
            assert!(disabled, "{title} waits for its milestone");
            milestone
        }
        Availability::SavedForLater(milestone) => {
            assert!(enabled, "{title} saves its preference now");
            milestone
        }
    };
    let notice = notice.unwrap_or_default();
    assert!(notice.contains(milestone.description()), "{title}: {notice}");
}

/// parity: SET-019
#[gtk::test]
fn rows_the_preview_cannot_run_yet_are_disabled_and_name_their_milestone() {
    let settings = SettingsTest::open();
    let groups = Category::ALL
        .into_iter()
        .flat_map(|category| settings.page.category_section(category).groups());
    for group in groups {
        for row in group.rows() {
            assert_row_follows_its_availability(&row, &group);
        }
    }
    let restore_previous = settings.row("Restore previous");
    let tooltip = restore_previous.tooltip_text().unwrap_or_default();
    assert!(
        tooltip.ends_with("arrives with desktop integration."),
        "{tooltip}"
    );
}

/// Choosing a theme card runs `win.theme`, which the Appearance menu runs
/// too: the skin changes at once and the choice is saved where the Python
/// app reads it.
///
/// parity: SET-019, SET-015
#[gtk::test]
fn a_theme_card_applies_the_theme_and_saves_it_for_both_apps() {
    let _theme = ThemeGuard::keep();
    let settings = SettingsTest::open();
    let dark_card = descendants::<gtk::ToggleButton>(&settings.page)
        .into_iter()
        .find(|card| card.has_css_class("dark"))
        .expect("Appearance has a Dark card");

    dark_card.emit_clicked();

    assert_eq!(skin().preference(), ThemePreference::Dark);
    assert!(dark_card.is_active());
    wait_until("the theme to be saved", || {
        settings.saved_preferences().theme == Theme::Dark
    });
    assert_eq!(
        python_preference(settings.test.settings_directory(), "theme"),
        "dark"
    );
}

/// parity: SET-019, VIEW-045
#[gtk::test]
fn the_text_size_row_draws_and_saves_the_chosen_size() {
    let _text_size = TextSizeGuard::keep();
    let settings = SettingsTest::open();
    let choices = choices_of(&settings.row("Text size"));
    assert_eq!(choices.chosen_label(), "100% (default)");

    choices.choose_labelled("125%");

    assert_eq!(skin().text_size().percent(), 125);
    wait_until("the text size to be saved", || {
        settings.saved_preferences().text_size == 125
    });
    assert_eq!(
        python_preference(settings.test.settings_directory(), "textSize"),
        "125"
    );
}

/// The rows of preferences the Python app follows save its keys and
/// values, so a change made here reaches it.
///
/// parity: SET-019, SET-016
#[gtk::test]
fn the_menu_watch_and_interval_rows_save_the_python_keys() {
    let settings = SettingsTest::open();
    choices_of(&settings.row("Right-click menu")).choose_labelled("Windows 11 · Compact actions");
    switch_of(&settings.row("Watch folders for live changes")).set_active(false);
    choices_of(&settings.row("Network / fallback checks")).choose_labelled("5 minutes");

    wait_until("the three preferences to be saved", || {
        let saved = settings.saved_preferences();
        saved.context_menu == ContextMenu::Win11 && !saved.auto_index && saved.network_interval == 300
    });
    let directory = settings.test.settings_directory();
    assert_eq!(python_preference(directory, "contextMenu"), "win11");
    assert_eq!(python_preference(directory, "autoIndex"), "False");
    assert_eq!(python_preference(directory, "networkInterval"), "300");
}

/// parity: SET-019, SET-015
#[gtk::test]
fn the_rows_show_what_the_python_app_saved() {
    let settings = SettingsTest::open();
    let directory = settings.test.settings_directory();
    python_saves_preferences(
        directory,
        "{'contextMenu': 'win11', 'autoIndex': False, 'networkInterval': 30}",
    );

    // F5 reads the settings file again, as the window does when a volume
    // changes.
    settings.test.activate("refresh", None);

    let watch = switch_of(&settings.row("Watch folders for live changes"));
    wait_until("the rows to show the Python app's values", || !watch.is_active());
    assert_eq!(
        choices_of(&settings.row("Right-click menu")).chosen_label(),
        "Windows 11 · Compact actions"
    );
    assert_eq!(
        choices_of(&settings.row("Network / fallback checks")).chosen_label(),
        "30 seconds"
    );
    assert!(
        !settings.saved_preferences().auto_index,
        "showing a value saves nothing over it"
    );
}

/// parity: SET-019
#[gtk::test]
fn manage_opens_the_indexed_folders_page_and_back_returns() {
    let settings = SettingsTest::open();
    let manage = settings.row("Folders to index").controls()[0]
        .clone()
        .downcast::<gtk::Button>()
        .expect("Manage… is a button");

    manage.emit_clicked();

    assert_eq!(
        settings.page.view(),
        SettingsView::Subpage(Subpage::IndexedFolders)
    );
    let folders = settings
        .page
        .imp()
        .indexed_folders
        .get()
        .expect("the page is built");
    let labels: Vec<String> = folders.shown().into_iter().map(|folder| folder.label).collect();
    let origin = settings
        .fixture
        .root()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    assert_eq!(
        labels.first(),
        origin.as_ref(),
        "the folder shown before comes first"
    );
    assert!(labels.iter().any(|label| label == "Home"));
    assert!(labels.iter().any(|label| label == "Local Disk"));

    let subpage = settings.page.imp().subpages.borrow()[&Subpage::IndexedFolders].clone();
    subpage
        .back_button()
        .expect("a sub-page has a back arrow")
        .emit_clicked();
    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::SearchAndIndexing)
    );
}

/// parity: SET-019
#[gtk::test]
fn escape_on_a_subpage_returns_to_its_category() {
    let settings = SettingsTest::open();
    settings
        .page
        .show_view(SettingsView::Subpage(Subpage::Troubleshooting));
    assert_eq!(
        settings.chosen_category(),
        Some(Category::DefaultApps),
        "Default apps stays chosen in the list"
    );

    let stepped = settings.page.step_back();

    assert_eq!(stepped, gtk::glib::Propagation::Stop);
    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::DefaultApps)
    );
}

#[gtk::test]
fn default_apps_reads_which_app_opens_each_route() {
    let settings = SettingsTest::open();
    let values: Vec<gtk::Label> = ["Folders", "SMB links", "ZIP files"]
        .into_iter()
        .map(|title| {
            let control = settings.row(title).controls().into_iter().next();
            control
                .and_downcast::<gtk::Label>()
                .expect("the route shows its app")
        })
        .collect();
    wait_until("GIO to answer", || {
        values
            .iter()
            .all(|value| value.text() != "Checking the current default…")
    });
}
