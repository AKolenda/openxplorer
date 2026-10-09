// SPDX-License-Identifier: AGPL-3.0-only
//! The settings layout of the 2026-10 mockup: eight pages, one line per
//! setting with its details in an ⓘ bubble, folded groups for what is
//! rarely changed, and a search that shows the matches of every page.
//! The layout moves the settings; it adds and removes none.

use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::super::group::SettingsGroup;
use super::super::pages::{Category, SettingsView};
use super::super::row::SettingRow;
use super::super::section::SettingsSection;
use super::SettingsTest;
use crate::test_support::harness::{descendants, wait_until};

/// Every setting row before the settings were rearranged, by its title
/// then, and the row that offers it now.
const MOVED_ROWS: [(&str, &str); 61] = [
    ("Theme", "Theme"),
    ("Text size", "Text size"),
    ("Use the desktop font", "Use the desktop font"),
    ("Right-click menu", "Right-click menu"),
    ("Show previews", "Show previews"),
    ("Show previews in network folders", "Also in network folders"),
    ("Skip previews of large files", "Skip very large files"),
    ("Preview pictures", "Pictures"),
    ("Preview videos", "Videos"),
    ("Preview documents and other files", "Documents and other files"),
    (
        "Show the number of items in folders",
        "Show the number of items in folders",
    ),
    ("Compact view", "Compact view"),
    ("Relative dates", "Relative dates (Today, Yesterday)"),
    ("Remember each folder's view", "Remember each folder's view"),
    ("Selection marker", "Item check boxes"),
    ("Expandable folders", "Expandable folders in Details"),
    ("Sidebar and column widths", "Sidebar and column widths"),
    ("Hide expand arrows", "Hide expand arrows in the sidebar"),
    ("Folders to index", "Indexed folders"),
    (
        "Index pinned folders automatically",
        "Index pinned folders automatically",
    ),
    ("Watch folders for live changes", "Watch folders for changes"),
    (
        "Network / fallback checks",
        "Check network and unwatched folders every",
    ),
    ("Calculate folder sizes", "Calculate folder sizes"),
    ("How sizes are counted", "How sizes are counted"),
    ("Include Show in folder", "Also handle “Show in folder”"),
    ("Also open ZIP files", "Also open ZIP files"),
    ("Folders", "Folders"),
    ("SMB links", "Network (SMB) links"),
    ("ZIP files", "Open ZIP files from other apps with OpenXplorer"),
    ("Brave and other apps", "“Show in folder” from browsers and apps"),
    ("Apps' Open and Save dialogs", "Other apps' Open and Save dialogs"),
    ("Troubleshooting", "Setup help for Zorin and Brave"),
    ("Restore previous", "Restore the previous file handlers"),
    ("Restore ZIP handler", "Give ZIP files back to the previous app"),
    ("Disable Show in folder", "Turn off Show in folder"),
    ("Super+E opens OpenXplorer", "Super+E opens OpenXplorer"),
    (
        "Restore Open and Save dialogs",
        "Turn off OpenXplorer's Open and Save dialogs",
    ),
    ("Open windows", "Open windows"),
    ("New window", "New window"),
    (
        "Open folders from other apps in a new window",
        "Folders from other apps open in a new window",
    ),
    (
        "Show full path in the title bar",
        "Show the full path in the title bar",
    ),
    ("Open new tabs", "New tabs open"),
    ("Open new windows at", "New windows open at"),
    (
        "Restore previous tabs at startup",
        "Reopen my tabs when OpenXplorer starts",
    ),
    ("Open new windows in split view", "Open new windows in split view"),
    (
        "Switch between split panes with Tab",
        "Tab key switches between split panes",
    ),
    ("Show full path in the address bar", "Show the full path"),
    (
        "Make the address bar editable in new windows",
        "Type addresses instead of breadcrumbs in new windows",
    ),
    ("Open archives as folders", "Open archives as folders"),
    ("Open ZIP files", "Double-clicking a ZIP"),
    (
        "Ask before moving items to the Recycle Bin",
        "Moving items to the Recycle Bin",
    ),
    ("Ask before deleting permanently", "Deleting permanently"),
    ("Ask before emptying the Recycle Bin", "Emptying the Recycle Bin"),
    (
        "Ask whether to run programs and scripts",
        "Running programs and scripts",
    ),
    (
        "Ask before closing a window with several tabs",
        "Closing a window with several tabs",
    ),
    ("Move tabs between windows", "Move tabs between windows"),
    ("Drag files into other apps", "Drag files into other apps"),
    ("Drop files on folders", "Drop files on folders"),
    (
        "Use Linux Downloads in Brave",
        "Brave saves to my Linux Downloads folder",
    ),
    ("Check for updates", "Check for updates"),
    ("OpenXplorer · License & source", "OpenXplorer · License & source"),
];

/// Every row of every page.
fn every_row(settings: &SettingsTest) -> Vec<SettingRow> {
    let sections = Category::ALL.map(|category| settings.page.category_section(category));
    sections.iter().flat_map(SettingsSection::rows).collect()
}

/// The folded group titled `title`.
fn folded_group(settings: &SettingsTest, title: &str) -> SettingsGroup {
    let groups = descendants::<SettingsGroup>(&settings.page);
    groups
        .into_iter()
        .filter(SettingsGroup::is_folded)
        .find(|group| {
            descendants::<gtk::Label>(group)
                .iter()
                .any(|label| label.text() == title)
        })
        .unwrap_or_else(|| panic!("Settings folds {title:?}"))
}

/// The title of a folded group, which opens and closes it.
fn fold_button(group: &SettingsGroup) -> gtk::Button {
    descendants::<gtk::Button>(group)
        .into_iter()
        .find(|button| button.has_css_class("group-fold"))
        .expect("a folded group has a title to click")
}

/// The pages are the mockup's, and every setting row of the earlier
/// layout is on one of them, found by its earlier name too: none was
/// added or left out.
///
/// parity: SET-019
#[gtk::test]
fn every_setting_of_the_earlier_layout_is_on_one_of_the_new_pages() {
    let settings = SettingsTest::open();
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
    let rows = every_row(&settings);
    assert_eq!(rows.len(), MOVED_ROWS.len(), "as many rows as before");
    for (earlier, now) in MOVED_ROWS {
        let row = settings.row(now);
        settings.page.search(earlier);
        assert!(row.get_visible(), "{earlier:?} still finds {now:?}");
    }
}

/// Each row is one line; what it used to say under its name is in the
/// bubble of the ⓘ after the name, which screen readers read as the ⓘ's
/// description.
///
/// parity: SET-019
#[gtk::test]
fn each_setting_is_one_line_with_its_details_in_the_info_bubble() {
    let settings = SettingsTest::open();
    for row in every_row(&settings) {
        let text = row.text();
        let info = row.info();
        assert_eq!(info.is_some(), !text.description.is_empty(), "{}", text.title);
        if let Some(info) = info {
            assert!(
                info.details().starts_with(text.description),
                "{}: {}",
                text.title,
                info.details()
            );
        }
        let lines = descendants::<gtk::Label>(&row);
        let description = lines
            .iter()
            .find(|label| label.has_css_class("setting-description"));
        assert!(
            description.is_none_or(|label| !label.get_visible() || label.text() != text.description),
            "{} does not repeat its details under its name",
            text.title
        );
    }
}

/// The ⓘ opens its bubble when the keyboard reaches it or it is clicked,
/// and Escape closes it. The bubble is a surface of its own, so the
/// page's scrolled area and the window's edge never cut it off.
///
/// parity: SET-019
#[gtk::test]
fn the_info_bubble_opens_on_hover_and_from_the_keyboard_on_its_own_surface() {
    let settings = SettingsTest::open();
    let row = settings.row("Tab key switches between split panes");
    let info = row.info().expect("the row has details");
    let popover = info.popover();
    assert!(
        popover
            .native()
            .is_some_and(|native| native.upcast::<gtk::Widget>() == *popover.upcast_ref::<gtk::Widget>()),
        "the bubble is a surface of its own"
    );
    assert!(!info.is_open());

    // Focus moved by the app, not the keyboard, opens nothing.
    settings.test.window.set_focus_visible(false);
    assert!(info.widget().grab_focus());
    assert!(!info.is_open(), "only the keyboard opens it");
    settings.page.focus_search();
    // Tab: the keyboard brings the focus here.
    settings.test.window.set_focus_visible(true);
    assert!(info.widget().grab_focus());
    wait_until("the bubble to open on focus", || info.is_open());
    let escape = gtk::gdk::Key::Escape;
    let controllers = info.widget().observe_controllers();
    let keys = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok());
    for keys in keys {
        keys.emit_by_name::<bool>(
            "key-pressed",
            &[&escape.into_glib(), &0_u32, &gtk::gdk::ModifierType::empty()],
        );
    }
    wait_until("Escape to close the bubble", || !info.is_open());

    // The pointer, with the keyboard elsewhere.
    settings.page.focus_search();
    info.widget().emit_clicked();
    assert!(!info.is_open(), "a click does not open the bubble");
    assert!(
        !info.widget().gets_focus_on_click(),
        "a click leaves the keyboard alone"
    );
    let motion = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerMotion>().ok())
        .expect("the ⓘ follows the pointer");
    motion.emit_by_name::<()>("enter", &[&5.0_f64, &5.0_f64]);
    wait_until("the bubble to open on hover", || info.is_open());
    motion.emit_by_name::<()>("leave", &[]);
    wait_until("the bubble to close when the pointer leaves", || !info.is_open());
    let text = descendants::<gtk::Label>(popover);
    assert!(text
        .iter()
        .any(|label| label.text().contains("Tab moves keyboard focus")));
}

/// Previews are folded away: the group shows its title until it is
/// clicked. A search that finds one of its rows opens it while the search
/// lasts, and ending the search folds it again; a group the user opened
/// stays open.
///
/// parity: SET-019
#[gtk::test]
fn rarely_changed_settings_are_folded_until_opened_or_found() {
    let settings = SettingsTest::open();
    let previews = folded_group(&settings, "Previews and thumbnails");
    let pictures = settings.row("Pictures");
    assert!(!previews.shows_rows(), "folded at first");

    settings.page.search("pictures");
    assert!(previews.shows_rows(), "the search opens it");
    assert!(pictures.is_visible());
    settings.page.search("");
    assert!(!previews.shows_rows(), "folded again after the search");

    fold_button(&previews).emit_clicked();
    assert!(previews.shows_rows(), "a click opens it");
    settings.page.search("zoom");
    settings.page.search("");
    assert!(previews.shows_rows(), "opened by the user, it stays open");
    let toggle = fold_button(&previews);
    assert_eq!(toggle.label(), None);
    for title in ["Dragging tabs and files", "Undo and troubleshooting"] {
        assert!(!folded_group(&settings, title).shows_rows(), "{title} is folded");
    }
}

/// A search shows the matches of every page on one page of results, each
/// under its page's name, so nobody has to go through the pages to find
/// them. The list of pages stays, with no page chosen; ending the search
/// chooses the page shown before and puts every page back.
///
/// parity: SET-019, SET-004
#[gtk::test]
fn the_search_shows_results_from_every_page() {
    let settings = SettingsTest::open();
    let imp = settings.page.imp();
    let pages = &imp.pages;
    settings
        .page
        .show_view(SettingsView::Category(Category::Appearance));

    settings.page.search("breadcrumbs");

    assert_eq!(pages.visible_child_name().as_deref(), Some("search-results"));
    assert!(imp.category_list.is_visible(), "the pages stay listed");
    assert_eq!(settings.listed_categories(), Category::ALL);
    assert_eq!(settings.chosen_category(), None, "no page is chosen");
    assert_eq!(settings.result_categories(), [Category::General]);
    settings.page.search("show");
    let found = settings.result_categories();
    assert!(found.len() > 1, "matches on several pages: {found:?}");
    let results = imp.results.get().expect("a results page").clone();
    for category in &found {
        let section = settings.page.category_section(*category);
        assert!(
            section.is_ancestor(&results),
            "{} is among the results",
            category.title()
        );
        let titles = descendants::<gtk::Label>(&section);
        assert!(titles.iter().any(|label| label.text() == category.title()));
    }
    assert!(imp.match_count().is_visible());
    assert!(
        imp.match_count().is_ancestor(&*imp.pages),
        "the count heads the results, not the left side"
    );
    for row in settings.page.category_rows() {
        let shown: Vec<String> = descendants::<gtk::Label>(&row)
            .into_iter()
            .filter(WidgetExt::is_visible)
            .map(|label| label.text().to_string())
            .collect();
        assert_eq!(shown, [row.category().title()], "no count beside a page's name");
    }

    settings.page.search("");

    assert_eq!(settings.chosen_category(), Some(Category::Appearance));
    assert_eq!(pages.visible_child_name().as_deref(), Some("appearance"));
    for category in Category::ALL {
        let section = settings.page.category_section(category);
        assert!(
            !section.is_ancestor(&results),
            "{} is on its own page",
            category.title()
        );
    }
}

/// A row is one line high however long its title, with room to spare:
/// a title too long for the row ends in "…" rather than taking a line a
/// word.
///
/// parity: SET-019
#[gtk::test]
fn a_row_is_one_line_high_whatever_its_title() {
    let settings = SettingsTest::open();
    let short = settings.row("New window");
    let long = settings.row("Folders from other apps open in a new window");
    let (short_height, ..) = short.measure(gtk::Orientation::Vertical, 760);
    let (long_height, ..) = long.measure(gtk::Orientation::Vertical, 760);
    assert!(
        long_height <= short_height,
        "a long title takes no more room: {long_height} > {short_height}"
    );
    for row in every_row(&settings) {
        let titles = descendants::<gtk::Label>(&row);
        let title = titles
            .iter()
            .find(|label| label.has_css_class("setting-title"))
            .expect("a row has a title");
        assert!(!title.wraps(), "{} stays on one line", row.text().title);
    }
}

/// Choosing a page in the list during a search ends the search and opens
/// that page, so nobody has to empty the search box first.
///
/// parity: SET-019
#[gtk::test]
fn choosing_a_page_during_a_search_ends_it_and_opens_the_page() {
    let settings = SettingsTest::open();
    let imp = settings.page.imp();
    settings
        .page
        .show_view(SettingsView::Category(Category::Appearance));
    settings.page.search("zoom");
    assert_eq!(settings.chosen_category(), None);

    let row = settings
        .page
        .category_list_row(Category::Confirmations)
        .expect("Confirmations is listed");
    imp.category_list.select_row(Some(&row));

    assert_eq!(imp.search_entry.text(), "", "the search is emptied");
    assert!(!imp.match_count().is_visible());
    assert_eq!(
        settings.page.view(),
        SettingsView::Category(Category::Confirmations)
    );
    assert_eq!(settings.chosen_category(), Some(Category::Confirmations));
    assert_eq!(
        imp.pages.visible_child_name().as_deref(),
        Some(Category::Confirmations.as_str())
    );
    let results = imp.results.get().expect("a results page");
    assert!(results.first_child().is_none(), "every page is back on its own");
}

/// A bubble open when its page is left, the pointer still on its ⓘ, is
/// closed then, so it does not come back on its own when the page shows
/// again, with no pointer on it to close it.
///
/// parity: SET-019
#[gtk::test]
fn a_bubble_closes_when_its_page_is_left() {
    let settings = SettingsTest::open();
    settings.page.show_view(SettingsView::Category(Category::General));
    let row = settings.row("Tab key switches between split panes");
    assert!(row.is_ancestor(&settings.page.category_section(Category::General)));
    let info = row.info().expect("the row has details");
    let controllers = info.widget().observe_controllers();
    let motion = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerMotion>().ok())
        .expect("the ⓘ follows the pointer");
    motion.emit_by_name::<()>("enter", &[&5.0_f64, &5.0_f64]);
    wait_until("the bubble to open on hover", || info.is_open());

    settings
        .page
        .show_view(SettingsView::Category(Category::Appearance));
    settings.page.show_view(SettingsView::Category(Category::General));
    crate::test_support::harness::wait_for_frames(&settings.test.window, 4);

    assert!(
        !info.is_open(),
        "the bubble stays closed when General shows again"
    );
}

/// A page chosen with a click in the list opens no ⓘ bubble. A click used
/// to activate the row too, which moved the keyboard onto the page's
/// first ⓘ (Appearance's Text size) and opened a bubble that stayed until
/// the window lost focus. Now a click only chooses the page, and even
/// Enter, which does move the keyboard into the page, opens a bubble only
/// when the keyboard is in use.
///
/// parity: SET-019
#[gtk::test]
fn choosing_a_page_with_a_click_opens_no_bubble() {
    let settings = SettingsTest::open();
    settings.page.show_view(SettingsView::Category(Category::General));
    let imp = settings.page.imp();
    assert!(
        !imp.category_list.activates_on_single_click(),
        "a click only chooses a page"
    );
    let open_bubbles = |category: Category| -> Vec<String> {
        settings
            .page
            .category_section(category)
            .rows()
            .iter()
            .filter(|row| {
                row.info()
                    .is_some_and(super::super::info_bubble::InfoBubble::is_open)
            })
            .map(|row| row.text().title.to_owned())
            .collect()
    };
    for category in [Category::Appearance, Category::FilesAndFolders, Category::General] {
        let row = settings.page.category_list_row(category).expect("listed");
        settings.test.window.set_focus_visible(false);
        // What a click does now: choose the row.
        row.grab_focus();
        imp.category_list.select_row(Some(&row));
        crate::test_support::harness::wait_for_frames(&settings.test.window, 4);
        assert_eq!(settings.page.view(), SettingsView::Category(category));
        let focus = GtkWindowExt::focus(&settings.test.window).expect("something has focus");
        assert!(
            focus.is_ancestor(&*imp.category_list) || focus == row.clone().upcast::<gtk::Widget>(),
            "{}: the keyboard stays in the list",
            category.title()
        );
        assert!(open_bubbles(category).is_empty(), "{}", category.title());

        // A double click, or focus moved in without the keyboard.
        row.emit_activate();
        crate::test_support::harness::wait_for_frames(&settings.test.window, 4);
        assert!(
            open_bubbles(category).is_empty(),
            "{}: {:?}",
            category.title(),
            open_bubbles(category)
        );
    }
}

/// The search finds settings by the words people use for them, not only
/// by the words the settings use, and those words find only what they
/// mean: "dark mode" finds Theme alone, the arrow words only the two
/// settings with arrows.
///
/// parity: SET-004
#[gtk::test]
fn the_search_finds_settings_by_the_words_people_use() {
    let settings = SettingsTest::open();
    let arrows = [
        "Hide expand arrows in the sidebar",
        "Expandable folders in Details",
    ];
    let cases: [(&str, &[&str]); 14] = [
        ("arrows", &arrows),
        ("chevron", &arrows),
        ("chevrons", &arrows),
        ("triangle", &arrows),
        ("sidebar chevron", &["Hide expand arrows in the sidebar"]),
        ("hiding arrows", &["Hide expand arrows in the sidebar"]),
        ("dark mode", &["Theme"]),
        ("night mode", &["Theme"]),
        ("light mode", &["Theme"]),
        ("magnify", &["Text size"]),
        (
            "wastebasket",
            &["Moving items to the Recycle Bin", "Emptying the Recycle Bin"],
        ),
        ("monitor", &["Watch folders for changes"]),
        (
            "dual pane",
            &[
                "Open new windows in split view",
                "Tab key switches between split panes",
            ],
        ),
        ("reopen", &["Reopen my tabs when OpenXplorer starts"]),
    ];
    for (typed, expected) in cases {
        settings.page.search(typed);
        assert_eq!(settings.shown_rows(), expected, "{typed:?}");
    }
    // "tree" finds the arrow settings among others that say it.
    settings.page.search("tree");
    let found = settings.shown_rows();
    assert!(arrows.iter().all(|row| found.contains(row)), "tree: {found:?}");
}
