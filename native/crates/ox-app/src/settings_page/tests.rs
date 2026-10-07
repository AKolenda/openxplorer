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
use super::pages::{Category, SettingsView, Subpage};
use super::row::SettingRow;
use super::section::SettingsSection;
use super::status_card::StatusCard;
use super::SettingsPage;
use crate::test_support::harness::{descendants, skin, wait_until, Fixture, TestWindow, ThemeGuard};
use crate::test_support::python::{python_preference, python_saves_preferences};
use crate::text_size::TextSize;
use crate::window::WindowAction;

mod indexing;
mod redesign;

/// A window on the standard fixture with Settings open in front.
struct SettingsTest {
    /// The window and its settings directory.
    test: TestWindow,
    /// The window's Settings page.
    page: SettingsPage,
    /// The folder the window showed before Settings.
    fixture: Fixture,
}

/// The Settings page of `test`'s window.
fn settings_page_of(test: &TestWindow) -> SettingsPage {
    descendants::<SettingsPage>(&test.window)
        .into_iter()
        .next()
        .expect("the window has a Settings page")
}

impl SettingsTest {
    fn open() -> Self {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.activate("settings", None);
        let page = settings_page_of(&test);
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

    /// The titles of the rows the category page shown now shows, or
    /// would show with its folded groups open: the rows the search keeps.
    fn shown_rows(&self) -> Vec<&'static str> {
        let sections = self.shown_sections().into_iter();
        let rows = sections.flat_map(|section| section.rows());
        let shown = rows.filter(WidgetExt::get_visible);
        shown.map(|row| row.text().title).collect()
    }

    /// The category pages shown now: every one with matches among the
    /// search results, else the chosen one.
    fn shown_sections(&self) -> Vec<SettingsSection> {
        if self.page.imp().search_entry.text().is_empty() {
            let category = self.page.view().category();
            return vec![self.page.category_section(category)];
        }
        let results = self.result_categories().into_iter();
        results
            .map(|category| self.page.category_section(category))
            .collect()
    }

    /// The radio button of the theme card `name`, such as "Dark".
    fn theme_radio(&self, name: &str) -> gtk::CheckButton {
        let radios = descendants::<gtk::CheckButton>(&self.page).into_iter();
        let mut theme_radios = radios.filter(|radio| radio.has_css_class("theme-radio"));
        theme_radios
            .find(|radio| radio.label().as_deref() == Some(name))
            .unwrap_or_else(|| panic!("Appearance has a {name} card"))
    }

    /// Whether the category page shown now shows its status card.
    fn shows_status_card(&self) -> bool {
        let sections = self.shown_sections().into_iter();
        let cards = sections.flat_map(|section| descendants::<StatusCard>(&section));
        cards.into_iter().any(|card| card.is_visible())
    }

    /// The categories the list shows now: none while it is hidden.
    fn listed_categories(&self) -> Vec<Category> {
        if !self.page.imp().category_list.is_visible() {
            return Vec::new();
        }
        let rows = self.page.category_rows().into_iter();
        let listed = rows.filter(WidgetExt::is_child_visible);
        listed.map(|row| row.category()).collect()
    }

    /// The categories whose matches the search results show, in order.
    fn result_categories(&self) -> Vec<Category> {
        let shown = Category::ALL.into_iter();
        shown
            .filter(|category| self.page.category_section(*category).get_visible())
            .collect()
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

/// Most windows never show Settings, so a window builds the page's rows
/// the first time Settings is shown, not when the window is created.
#[gtk::test]
fn a_window_builds_its_settings_rows_when_settings_is_first_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let page = settings_page_of(&test);
    assert!(
        descendants::<SettingRow>(&page).is_empty(),
        "no rows before Settings opens"
    );

    test.activate("settings", None);

    assert!(!descendants::<SettingRow>(&page).is_empty());
    assert_eq!(page.view(), SettingsView::Category(Category::General));
}

/// A window whose first tab is Settings, as `OPENXPLORER_START=ox:settings`
/// opens one, shows the page without Settings being opened by a command.
#[gtk::test]
fn a_window_started_at_settings_shows_its_rows() {
    let test = TestWindow::open(crate::locations::Page::Settings.uri());

    let page = settings_page_of(&test);
    wait_until("the Settings page to be built", || {
        !descendants::<SettingRow>(&page).is_empty()
    });
    assert!(page.is_mapped());
}

/// The navigation column of `settingsDialog`: the heading, its subtitle,
/// the search box, the categories and "Back to files".
///
/// parity: SET-003
#[gtk::test]
fn the_navigation_column_names_the_page_its_search_and_categories() {
    let settings = SettingsTest::open();
    let imp = settings.page.imp();
    let heading = descendants::<gtk::Label>(&settings.page)
        .into_iter()
        .find(|label| label.has_css_class("settings-heading"))
        .expect("the column has a heading");
    assert_eq!(heading.text(), "Settings");
    assert_eq!(imp.subtitle.text(), "Your explorer, your way.");
    assert_eq!(
        imp.search_entry.placeholder_text().as_deref(),
        Some("Search settings")
    );
    assert_eq!(settings.listed_categories(), Category::ALL);
    let titles: Vec<&str> = Category::ALL.iter().map(|category| category.title()).collect();
    assert_eq!(
        titles,
        [
            "General",
            "Appearance",
            "Files & folders",
            "ZIP & archives",
            "Confirmations",
            "Search",
            "Default apps",
            "About"
        ]
    );
    let back = descendants::<gtk::Label>(&imp.back_button.get());
    assert!(back.iter().any(|label| label.text() == "Back to files"));
}

/// parity: SET-019
#[gtk::test]
fn choosing_a_category_shows_only_that_category() {
    let settings = SettingsTest::open();
    let pages = &settings.page.imp().pages;
    assert_eq!(pages.visible_child_name().as_deref(), Some("general"));

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
    assert_eq!(settings.page.view(), SettingsView::Category(Category::Appearance));
    list.emit_move_cursor(gtk::MovementStep::DisplayLines, 1, false, false);
    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::FilesAndFolders)
    );
}

/// A search and the category and rows it shows.
struct SearchCase {
    typed: &'static str,
    categories: &'static [Category],
    shown: &'static [&'static str],
}

/// Ported from the keywords of `appendV07Settings` in `v2.0.0:desktop/ui/app.js`,
/// which make "zoom", "watch live" and "dolphin" find their settings.
///
/// parity: SET-019, SET-004
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
            categories: &[Category::Search],
            shown: &["Watch folders for changes"],
        },
        SearchCase {
            typed: "dolphin",
            categories: &[Category::DefaultApps],
            shown: &["Folders"],
        },
        SearchCase {
            typed: "brave",
            categories: &[Category::DefaultApps],
            shown: &[
                "Also handle “Show in folder”",
                "“Show in folder” from browsers and apps",
                "Brave saves to my Linux Downloads folder",
                "Turn off Show in folder",
                "Setup help for Zorin and Brave",
            ],
        },
    ];
    for case in cases {
        settings.page.search(case.typed);
        assert_eq!(settings.listed_categories(), Category::ALL, "{}", case.typed);
        assert_eq!(settings.chosen_category(), None, "{}", case.typed);
        assert_eq!(settings.result_categories(), case.categories, "{}", case.typed);
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

/// Ported from `settingsSearch` in `v2.0.0:desktop/ui/app.js`, which matched an
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
            category: Category::Search,
            shows_status_card: true,
            // "all" is in the privacy note ("locally"), in the row's ⓘ.
            shown: &["Indexed folders", "Check network and unwatched folders every"],
        },
        ShownTextCase {
            typed: "make openxplorer default",
            category: Category::DefaultApps,
            shows_status_card: true,
            shown: &["Also handle “Show in folder”", "Also open ZIP files"],
        },
        ShownTextCase {
            typed: "refresh status",
            category: Category::DefaultApps,
            shows_status_card: false,
            shown: &[
                "Folders",
                "Network (SMB) links",
                "“Show in folder” from browsers and apps",
                "Other apps' Open and Save dialogs",
            ],
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
        let results = settings.result_categories();
        assert_eq!(results, [case.category], "{}", case.typed);
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

    let section = settings.page.category_section(Category::Search);
    let card = descendants::<StatusCard>(&section)
        .into_iter()
        .next()
        .expect("Search has a status card");
    assert!(card.has_css_class("jump-target"));
    assert_eq!(settings.page.imp().match_count.text(), "3 matching settings");
}

/// parity: SET-019
#[gtk::test]
fn a_search_that_matches_nothing_says_so() {
    let settings = SettingsTest::open();
    settings.page.search("no such setting");
    let imp = settings.page.imp();
    assert_eq!(imp.match_count.text(), "No matching settings");
    assert_eq!(imp.pages.visible_child_name().as_deref(), Some("no-matches"));
    assert_eq!(settings.listed_categories(), Category::ALL);
    assert_eq!(settings.chosen_category(), None);
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

/// parity: SET-019, SET-004
#[gtk::test]
fn enter_in_the_search_jumps_to_the_first_match() {
    let settings = SettingsTest::open();
    settings.page.show_view(SettingsView::Category(Category::About));
    settings.page.search("network interval");

    settings.page.imp().search_entry.emit_activate();

    assert_eq!(settings.page.view(), SettingsView::Category(Category::Search));
    assert_eq!(settings.chosen_category(), None, "the search goes on");
    let row = settings.row("Check network and unwatched folders every");
    assert!(row.has_css_class("jump-target"));
    let focus = GtkWindowExt::focus(&settings.test.window).expect("a control has focus");
    assert!(focus.is_ancestor(&row), "the row's drop-down has keyboard focus");
}

/// parity: SET-019, SET-004
#[gtk::test]
fn escape_leaves_the_search_and_shows_every_row_again() {
    let settings = SettingsTest::open();
    settings
        .page
        .show_view(SettingsView::Category(Category::Appearance));
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
            "Use the desktop font",
            "Compact view",
            "Hide expand arrows in the sidebar",
            "Right-click menu",
            "Sidebar and column widths"
        ]
    );
}

/// A setting of the Python page, by the words that named it there, and
/// the native row that offers it.
struct PythonSetting {
    python: &'static str,
    row: &'static str,
}

const fn offered_by(python: &'static str, row: &'static str) -> PythonSetting {
    PythonSetting { python, row }
}

/// "Every setting the current app offers stays available" (SET-019): each
/// setting, action and piece of advice of `renderSettingsPage` and
/// `appendV07Settings` has a row, which a search for its Python wording
/// finds. Some Python controls share a row now, such as "Use `OpenXplorer`
/// for ZIPs", which is the ZIP files row's button.
///
/// parity: SET-019
#[gtk::test]
fn every_setting_of_the_python_page_has_a_row() {
    let settings = SettingsTest::open();
    let python_settings = [
        offered_by("Theme", "Theme"),
        offered_by("Text size", "Text size"),
        offered_by("Right-click menu", "Right-click menu"),
        offered_by("Reset sidebar and column widths", "Sidebar and column widths"),
        offered_by("Folders to index", "Indexed folders"),
        offered_by("Watch folders for live changes", "Watch folders for changes"),
        offered_by(
            "Network / fallback checks",
            "Check network and unwatched folders every",
        ),
        offered_by("Calculate folder sizes", "Calculate folder sizes"),
        offered_by("Folders", "Folders"),
        offered_by("SMB links", "Network (SMB) links"),
        offered_by("ZIP files", "Open ZIP files from other apps with OpenXplorer"),
        offered_by("Include Show in folder", "Also handle “Show in folder”"),
        offered_by("Also open ZIP files in OpenXplorer", "Also open ZIP files"),
        offered_by(
            "Use OpenXplorer for ZIPs",
            "Open ZIP files from other apps with OpenXplorer",
        ),
        offered_by("Test Show in folder", "“Show in folder” from browsers and apps"),
        offered_by("Enable Show in folder", "“Show in folder” from browsers and apps"),
        offered_by(
            "Zorin + Brave setup and troubleshooting",
            "Setup help for Zorin and Brave",
        ),
        offered_by("Restore previous", "Restore the previous file handlers"),
        offered_by("Restore ZIP handler", "Give ZIP files back to the previous app"),
        offered_by("Disable Show in folder", "Turn off Show in folder"),
        offered_by("Open windows", "Open windows"),
        offered_by("New window", "New window"),
        offered_by("Move tabs between windows", "Move tabs between windows"),
        offered_by(
            "Use Linux Downloads in Brave",
            "Brave saves to my Linux Downloads folder",
        ),
        offered_by("OpenXplorer · License & source", "OpenXplorer · License & source"),
    ];
    for setting in python_settings {
        let row = settings.row(setting.row);
        settings.page.search(setting.python);
        assert!(row.is_visible(), "{:?} finds {:?}", setting.python, setting.row);
    }
}

/// Rows that work but disable their button while there is nothing for it
/// to do, as Restore previous with no handler recorded (INT-030).
const ROWS_FOLLOWING_THEIR_STATE: [&str; 3] = [
    "Restore the previous file handlers",
    "Give ZIP files back to the previous app",
    "Open ZIP files from other apps with OpenXplorer",
];

/// Every row of every category works: its controls take input, except
/// those that wait for something to do.
///
/// parity: SET-019
#[gtk::test]
fn every_row_can_be_used() {
    let settings = SettingsTest::open();
    let rows = Category::ALL
        .into_iter()
        .flat_map(|category| settings.page.category_section(category).groups())
        .flat_map(|group| group.rows());
    for row in rows {
        let title = row.text().title;
        let enabled = row.controls().iter().all(WidgetExt::is_sensitive);
        assert!(
            enabled || ROWS_FOLLOWING_THEIR_STATE.contains(&title),
            "{title} works"
        );
    }
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
    let dark = settings.theme_radio("Dark");

    dark.activate();

    assert_eq!(skin().theme(), Theme::Dark);
    assert!(dark.is_active());
    let dark_card = dark.parent().expect("the radio is in its card");
    assert!(dark_card.has_css_class("chosen"), "the chosen card is outlined");
    wait_until("the theme to be saved", || {
        settings.saved_preferences().theme == Theme::Dark
    });
    assert_eq!(
        python_preference(settings.test.settings_directory(), "theme"),
        "dark"
    );
}

/// The theme cards are one choice of three: screen readers hear radio
/// buttons, and an arrow key moves to the next card and chooses it.
///
/// parity: SET-019
#[gtk::test]
fn the_arrow_keys_move_between_the_theme_cards_and_choose_them() {
    let _theme = ThemeGuard::keep();
    let settings = SettingsTest::open();
    settings
        .page
        .show_view(SettingsView::Category(Category::Appearance));
    let light = settings.theme_radio("Light");
    let dark = settings.theme_radio("Dark");
    for radio in [&settings.theme_radio("System"), &light, &dark] {
        assert!(gtk::test_accessible_has_role(radio, gtk::AccessibleRole::Radio));
    }
    light.activate();
    wait_until("the cards to be laid out", || dark.width() > 0);
    assert!(light.grab_focus());

    // What the window does with the Right arrow key.
    settings.test.window.child_focus(gtk::DirectionType::Right);

    assert!(dark.has_focus(), "the next card has keyboard focus");
    assert!(dark.is_active());
    assert_eq!(skin().theme(), Theme::Dark);
}

/// The choices of the Python "Appearance & layout" section: the theme,
/// the text sizes with "100% (default)", the two menus, and the reset of
/// the pane widths.
///
/// parity: SET-005
#[gtk::test]
fn appearance_offers_the_python_choices() {
    let settings = SettingsTest::open();
    for theme in ["System", "Light", "Dark"] {
        settings.theme_radio(theme);
    }
    assert_eq!(
        choices_of(&settings.row("Text size")).options(),
        [
            "80%",
            "90%",
            "100% (default)",
            "110%",
            "125%",
            "150%",
            "175%",
            "200%"
        ]
    );
    assert_eq!(
        choices_of(&settings.row("Right-click menu")).options(),
        ["Windows 10 · Classic (default)", "Windows 11 · Compact actions"]
    );
    let controls = settings.row("Sidebar and column widths").controls();
    let reset = controls
        .first()
        .and_then(|control| control.downcast_ref::<gtk::Button>());
    let reset = reset.expect("a Reset button");
    assert_eq!(
        reset.action_name().as_deref(),
        Some(WindowAction::ResetLayout.detailed_name().as_str())
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

/// "Hide expand arrows" is off by default and saves the choice, which
/// the open window then takes up (SIDE-032).
///
/// parity: SIDE-032
#[gtk::test]
fn the_expand_arrows_switch_saves_the_choice() {
    let settings = SettingsTest::open();
    let switch = switch_of(&settings.row("Hide expand arrows in the sidebar"));
    assert!(!switch.is_active(), "the arrows are shown by default");

    switch.set_active(true);
    wait_until("the choice to be saved", || {
        settings.saved_preferences().hide_expand_arrows
    });
    wait_until("the window to follow", || {
        settings.test.window.hides_expand_arrows()
    });
}

/// The Compact view switch is off by default and saves the choice
/// (VIEW-067), which the open window then takes up.
///
/// parity: VIEW-067
#[gtk::test]
fn the_compact_view_switch_saves_the_choice() {
    let settings = SettingsTest::open();
    let switch = switch_of(&settings.row("Compact view"));
    assert!(!switch.is_active(), "off by default");

    switch.set_active(true);
    wait_until("Compact view to be saved", || {
        settings.saved_preferences().compact_density
    });
    wait_until("the window to follow", || {
        settings.test.window.shows_compact_density()
    });
}

/// The rows of preferences the Python app follows save its keys and
/// values, so a change made here reaches it.
///
/// parity: SET-019, SET-016
#[gtk::test]
fn the_menu_watch_and_interval_rows_save_the_python_keys() {
    let settings = SettingsTest::open();
    choices_of(&settings.row("Right-click menu")).choose_labelled("Windows 11 · Compact actions");
    switch_of(&settings.row("Watch folders for changes")).set_active(false);
    choices_of(&settings.row("Check network and unwatched folders every")).choose_labelled("5 minutes");

    wait_until("the three preferences to be saved", || {
        let saved = settings.saved_preferences();
        saved.context_menu == ContextMenu::Win11 && !saved.auto_index && saved.network_interval == 300
    });
    let directory = settings.test.settings_directory();
    assert_eq!(python_preference(directory, "contextMenu"), "win11");
    assert_eq!(python_preference(directory, "autoIndex"), "False");
    assert_eq!(python_preference(directory, "networkInterval"), "300");
}

/// "Open ZIP files" says how a browsed ZIP opens, so it is off while
/// "Open archives as folders" is off.
///
/// parity: ARC-026
#[gtk::test]
fn open_zip_files_waits_for_open_archives_as_folders() {
    let settings = SettingsTest::open();
    let opening = settings.row("Double-clicking a ZIP");
    let is_usable = || opening.controls().iter().all(WidgetExt::is_sensitive);
    assert!(is_usable());

    switch_of(&settings.row("Open archives as folders")).set_active(false);
    wait_until("the row to be off", || !is_usable());
    switch_of(&settings.row("Open archives as folders")).set_active(true);
    wait_until("the row to be on again", is_usable);
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

    let watch = switch_of(&settings.row("Watch folders for changes"));
    wait_until("the rows to show the Python app's values", || !watch.is_active());
    assert_eq!(
        choices_of(&settings.row("Right-click menu")).chosen_label(),
        "Windows 11 · Compact actions"
    );
    assert_eq!(
        choices_of(&settings.row("Check network and unwatched folders every")).chosen_label(),
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
    let manage = settings.row("Indexed folders").controls()[0]
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
    let labels = folders.shown_labels();
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
    assert_eq!(settings.page.view(), SettingsView::Category(Category::Search));
}

/// The Python "Folder sizes" help moves off the Files & folders page to
/// a page of its own, whole.
///
/// parity: SET-019
#[gtk::test]
fn how_sizes_are_counted_opens_the_folder_sizes_help() {
    let settings = SettingsTest::open();
    let open = settings.row("How sizes are counted").controls()[0]
        .clone()
        .downcast::<gtk::Button>()
        .expect("the row opens its page with a button");

    open.emit_clicked();

    assert_eq!(settings.page.view(), SettingsView::Subpage(Subpage::FolderSizes));
    let subpage = settings.page.imp().subpages.borrow()[&Subpage::FolderSizes].clone();
    let text = crate::settings_page::search::shown_text(&subpage);
    assert!(text.contains("Results are logical file bytes"), "{text}");
    assert!(text.contains("Cancel stops the active scan"), "{text}");
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
    if std::env::var_os("OX_DISTRO_CI").is_some() {
        eprintln!("skipped under OX_DISTRO_CI: GIO's application lookup needs a desktop session bus");
        return;
    }
    let settings = SettingsTest::open();
    let values: Vec<gtk::Label> = ["Folders", "Network (SMB) links"]
        .into_iter()
        .map(|title| {
            let control = settings.row(title).controls().into_iter().next();
            control
                .and_downcast::<gtk::Label>()
                .expect("the route shows its app")
        })
        .collect();
    let zip_files = settings.row("Open ZIP files from other apps with OpenXplorer");
    let section = settings.page.category_section(Category::DefaultApps);
    let card = descendants::<StatusCard>(&section)
        .into_iter()
        .next()
        .expect("Default apps has a status card");
    wait_until("GIO to answer", || {
        let routes_read = values
            .iter()
            .all(|value| value.text() != "Checking the current default…");
        let zip_read = zip_files.shown_description() != "Checking the current default…";
        routes_read && zip_read
    });
    assert!(
        card.title().starts_with("OpenXplorer is"),
        "the card states the result: {}",
        card.title()
    );
}
