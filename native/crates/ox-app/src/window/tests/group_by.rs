// SPDX-License-Identifier: AGPL-3.0-only
//! Explorer's Group by in a real window (VIEW-022): grouping apart from
//! the sort, Explorer's name ranges and date periods, Downloads grouped by
//! date out of the box, and the Sort menu's More and Group by submenus.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, this also saves
//! `native-group-by-date.png`, `native-sort-menu.png` and
//! `native-group-by-menu.png`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gtk::glib;
use gtk::prelude::*;
use ox_core::grouping::GroupBy;
use ox_core::location::file_uri;
use ox_core::places::FolderLocations;
use ox_core::settings::PreferencesUpdate;

use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{
    capture, capture_popover, descendants, wait_for_frames, wait_until, Fixture, TestWindow,
};
use crate::window::command_bar::menus::sort_menu;
use crate::window::menu_popover::MenuEntry;

/// The headings the details view draws, top to bottom.
fn headings(test: &TestWindow) -> Vec<String> {
    wait_for_frames(&test.window, 3);
    let column_view = test.window.folder_pane().details().column_view().clone();
    let mut labels: Vec<(f64, String)> = descendants::<gtk::Label>(&column_view)
        .into_iter()
        .filter(|label| label.has_css_class("group-title") && label.is_mapped())
        .filter_map(|label| {
            let point = label.compute_point(&column_view, &gtk::graphene::Point::new(0.0, 0.0))?;
            Some((f64::from(point.y()), label.text().to_string()))
        })
        .collect();
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    labels.into_iter().map(|(_, text)| text).collect()
}

/// Sets the modification time of `path` to `age` ago.
fn age(path: &Path, age: Duration) {
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now() - age))
        .expect("the date is set");
}

/// Three years, a date that is "A long time ago" on any day.
const LONG_AGO: Duration = Duration::from_secs(3 * 366 * 86_400);

/// A home with a Downloads folder holding a folder and a file from today
/// and a file from long ago.
struct TestHome {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
}

impl TestHome {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room");
        let base = fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        let downloads = home.join("Downloads");
        fs::create_dir_all(downloads.join("Installers")).expect("a folder");
        fs::create_dir_all(&config).expect("a folder");
        fs::write(downloads.join("today.txt"), "x").expect("a file");
        let old = downloads.join("old manual.pdf");
        fs::write(&old, "x").expect("a file");
        age(&old, LONG_AGO);
        Self {
            _root: root,
            home,
            config,
        }
    }

    fn locations(&self) -> FolderLocations {
        FolderLocations::new(self.home.clone(), &self.config)
    }

    fn downloads(&self) -> String {
        file_uri(&self.home.join("Downloads"))
    }
}

/// Grouped by date modified, each period is sorted by name, as Explorer
/// does: the groups no longer follow the sort.
///
/// parity: VIEW-022
#[gtk::test]
fn grouping_by_date_keeps_the_name_sort_within_each_group() {
    let fixture = Fixture::empty();
    for name in ["zeta.txt", "alpha.txt", "beta.txt"] {
        fixture.write(name);
    }
    age(&fixture.path("beta.txt"), LONG_AGO);
    let test = TestWindow::open(&fixture.uri());
    assert_eq!(test.action_state("sort").as_deref(), Some("name"));

    test.activate("group-by", Some("modified"));

    assert_eq!(test.action_state("group-by").as_deref(), Some("modified"));
    assert_eq!(
        test.action_state("sort").as_deref(),
        Some("name"),
        "the sort stays"
    );
    assert_eq!(
        test.names(),
        ["alpha.txt", "zeta.txt", "beta.txt"],
        "today's by name, then long ago's"
    );
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);

    test.activate("direction", Some("descending"));
    assert_eq!(
        test.names(),
        ["zeta.txt", "alpha.txt", "beta.txt"],
        "the sort turns within the groups, which keep their order"
    );
}

/// Grouped by name, the names fall in Explorer's letter ranges, and the
/// selection survives regrouping; by size, folders come first.
///
/// parity: VIEW-022
#[gtk::test]
fn names_fall_in_explorers_letter_ranges() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(test.window.select_named("Notes 2.txt"));

    test.activate("group-by", Some("name"));

    assert_eq!(headings(&test), ["A – H (1)", "I – P (2)", "Q – Z (1)"]);
    assert_eq!(
        test.names(),
        ["Documents", "Notes 2.txt", "Notes 10.txt", "Résumé.txt"]
    );
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt"],
        "regrouping keeps the selection"
    );
    test.activate("group-by", Some("size"));
    assert_eq!(headings(&test), ["Folders (1)", "Tiny (0 – 16 KB) (3)"]);
    assert_eq!(test.selected_names(), ["Notes 2.txt"], "and regrouping again");
}

/// A group's count follows the files: one added or removed shows in its
/// heading at the next listing (F5), without the folder being opened
/// again. Before, "Today (2)" kept its first count.
///
/// parity: VIEW-022
#[gtk::test]
fn group_counts_follow_files_added_and_removed() {
    let home = TestHome::new();
    let downloads = home.downloads();
    let test = TestWindow::open_with_standard_folders(&downloads, home.locations(), |_| {});
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);
    let new_file = home.home.join("Downloads").join("receipt.pdf");

    fs::write(&new_file, "x").expect("a new file");
    test.activate("refresh", None);
    wait_until("the new file", || {
        test.names().contains(&"receipt.pdf".to_owned())
    });
    assert_eq!(headings(&test), ["Today (3)", "A long time ago (1)"]);

    fs::remove_file(&new_file).expect("the file goes");
    test.activate("refresh", None);
    wait_until("the file to go", || {
        !test.names().contains(&"receipt.pdf".to_owned())
    });
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);
}

/// Date groups follow the calendar past midnight: when the day changes,
/// today's files move to Yesterday without the folder being opened again.
///
/// parity: VIEW-022
#[gtk::test]
fn date_groups_move_on_when_the_day_changes() {
    let home = TestHome::new();
    let downloads = home.downloads();
    let test = TestWindow::open_with_standard_folders(&downloads, home.locations(), |_| {});
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);

    let tomorrow = glib::DateTime::now_local()
        .and_then(|now| now.add_days(1))
        .expect("tomorrow");
    crate::folder_view::groups::set_clock_for_tests(Some(tomorrow));
    test.window.follow_the_day();
    let shown = headings(&test);
    crate::folder_view::groups::set_clock_for_tests(None);
    assert_eq!(shown, ["Yesterday (2)", "A long time ago (1)"]);
}

/// Columns shown, hidden or moved while the groups are headed: Downloads
/// grouped, then the Recycle Bin (its own columns) and Back, then a column
/// chosen and taken away again. GTK 4.22 crashed on Back after it had
/// finalized group headers whose cells were still in them.
///
/// parity: VIEW-022
#[gtk::test]
fn columns_change_safely_while_the_groups_are_headed() {
    super::file_ops_support::require_private_trash();
    let home = TestHome::new();
    let downloads = home.downloads();
    let test = TestWindow::open_with_standard_folders(&downloads, home.locations(), |_| {});
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);

    test.window.navigate("trash:///").expect("the Recycle Bin");
    test.wait_for_listing("the Recycle Bin");
    wait_for_frames(&test.window, 4);
    test.activate("back", None);
    test.wait_for_listing("Downloads");
    wait_for_frames(&test.window, 4);
    assert_eq!(
        headings(&test),
        ["Today (2)", "A long time ago (1)"],
        "the groups are headed again"
    );

    let details = test.window.folder_pane().details();
    let before = details.chosen_columns();
    let mut more = before.clone();
    more.push(SortColumn::Created);
    details.show_chosen_columns(more);
    wait_for_frames(&test.window, 4);
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);
    details.show_chosen_columns(before);
    wait_for_frames(&test.window, 4);
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);
}

/// Downloads opens grouped by date, as in Explorer, while other folders
/// share the style without groups; choosing (None) in Downloads is
/// remembered for Downloads only.
///
/// parity: VIEW-022
#[gtk::test]
fn downloads_is_grouped_by_date_until_the_user_chooses_otherwise() {
    let home = TestHome::new();
    let downloads = home.downloads();
    let test = TestWindow::open_with_standard_folders(&downloads, home.locations(), |_| {});
    assert_eq!(test.action_state("group-by").as_deref(), Some("modified"));
    assert_eq!(
        test.names(),
        ["Installers", "today.txt", "old manual.pdf"],
        "newest group first, folders first within it"
    );
    assert_eq!(headings(&test), ["Today (2)", "A long time ago (1)"]);
    capture(&test.window, "native-group-by-date.png");

    test.window
        .navigate(&file_uri(&home.home))
        .expect("the home folder");
    test.wait_for_listing("the home folder");
    assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
    assert!(headings(&test).is_empty(), "an ungrouped list has no heading");

    test.window.navigate(&downloads).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert_eq!(test.action_state("group-by").as_deref(), Some("modified"));

    test.activate("group-by", Some("none"));
    assert!(headings(&test).is_empty());
    wait_until("the choice to be saved", || {
        test.context.settings_data().preferences.downloads_group_by == Some(GroupBy::None)
    });
    let shared = test.context.settings_data().preferences.view_defaults;
    assert!(
        shared.is_none_or(|style| style.grouping() == GroupBy::None),
        "the other folders' style is unchanged"
    );
    test.window
        .navigate(&file_uri(&home.home))
        .expect("the home folder");
    test.wait_for_listing("the home folder");
    test.window.navigate(&downloads).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
}

/// With each folder keeping its own style, Downloads is grouped by date
/// until it has a style of its own, which then keeps its Group by.
///
/// parity: VIEW-022, VIEW-020
#[gtk::test]
fn downloads_keeps_its_own_group_by_when_each_folder_keeps_a_style() {
    let home = TestHome::new();
    let downloads = home.downloads();
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), home.locations(), |_| {});
    let per_folder = PreferencesUpdate {
        per_folder_views: Some(true),
        ..PreferencesUpdate::default()
    };
    test.context
        .update_preferences(per_folder, |result| result.expect("saved"));
    wait_until("the preference", || {
        test.context.settings_data().preferences.per_folder_views
    });
    test.window.navigate(&downloads).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert_eq!(test.action_state("group-by").as_deref(), Some("modified"));

    test.activate("group-by", Some("type"));
    wait_until("Downloads' own style", || {
        let preferences = test.context.settings_data().preferences;
        preferences.view_in(&downloads, Some(&downloads)).grouping() == GroupBy::Type
    });
    test.window
        .navigate(&file_uri(&home.home))
        .expect("the home folder");
    test.wait_for_listing("the home folder");
    assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
    test.window.navigate(&downloads).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert_eq!(test.action_state("group-by").as_deref(), Some("type"));
}

/// The first group's heading is in sight when a grouped folder opens: no
/// row of the list hides it.
///
/// parity: VIEW-022
#[gtk::test]
fn the_first_heading_is_shown_when_a_grouped_folder_opens() {
    let home = TestHome::new();
    for number in 0..40 {
        fs::write(
            home.home.join("Downloads").join(format!("file {number}.txt")),
            "x",
        )
        .expect("a file");
    }
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), home.locations(), |_| {});
    test.window.navigate(&home.downloads()).expect("Downloads");
    test.wait_for_listing("Downloads");
    wait_for_frames(&test.window, 5);
    let scrolled = test.window.folder_pane().details().vadjustment().value();
    assert!(scrolled < 0.5, "Downloads opens at its first heading: {scrolled}");
    assert_eq!(headings(&test).first().map(String::as_str), Some("Today (42)"));
}

/// Back to a grouped folder puts the view where it was: showing a list
/// from its top must not override a restored position.
///
/// parity: VIEW-022, NAV-008
#[gtk::test]
fn back_to_a_grouped_folder_keeps_its_scroll_position() {
    let home = TestHome::new();
    for number in 0..80 {
        fs::write(
            home.home.join("Downloads").join(format!("file {number}.txt")),
            "x",
        )
        .expect("a file");
    }
    let test = TestWindow::open_with_standard_folders(&home.downloads(), home.locations(), |_| {});
    let adjustment = test.window.folder_pane().details().vadjustment();
    wait_for_frames(&test.window, 5);
    adjustment.set_value(400.0);
    wait_for_frames(&test.window, 3);
    let scrolled = adjustment.value();
    assert!(scrolled > 300.0, "the list scrolls: {scrolled}");

    test.window
        .navigate(&file_uri(&home.home))
        .expect("the home folder");
    test.wait_for_listing("the home folder");
    test.activate("back", None);
    test.wait_for_listing("Downloads");
    wait_for_frames(&test.window, 8);

    let restored = test.window.folder_pane().details().vadjustment().value();
    assert!(
        (restored - scrolled).abs() < 1.0,
        "back to {scrolled}, not {restored}"
    );
}

/// The Sort menu's submenus: More holds Size and the further keys, and
/// Group by Explorer's choices, "Same as sort" and (None); choosing one
/// from the open menu groups the folder.
///
/// parity: VIEW-022
#[gtk::test]
fn the_sort_menu_has_more_and_group_by_submenus() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.right_click(None);
    let menu = test.window.context_menu();
    menu.row("Sort by").emit_activate();
    wait_until("the Sort menu", || {
        menu.is_visible() && menu.row_labels().contains(&"Group by".to_owned())
    });
    capture_popover(&test.window, menu.upcast_ref(), "native-sort-menu.png");
    // Clicking Group by opens it beside the Sort menu, which stays open.
    menu.row("Group by").emit_activate();
    let group_by = menu.open_submenu_menu().expect("Group by opens beside the menu");
    wait_until("the Group by submenu", || group_by.is_visible());
    assert!(menu.is_visible(), "the Sort menu stays open");
    assert_eq!(
        group_by.row_labels(),
        [
            "Name",
            "Date modified",
            "Type",
            "Size",
            "Date created",
            "Same as sort",
            "(None)"
        ]
    );
    capture_popover(&test.window, group_by.upcast_ref(), "native-group-by-menu.png");

    // Left closes only the submenu; Right on Group by opens it again.
    assert!(group_by.press_in_list(gtk::gdk::Key::Left));
    assert!(menu.open_submenu_menu().is_none());
    assert!(menu.is_visible());
    menu.row("Group by").grab_focus();
    assert!(menu.press_in_list(gtk::gdk::Key::Right));
    let group_by = menu.open_submenu_menu().expect("Right opens it");

    // Choosing in the submenu closes both menus, then groups.
    group_by.row("Date modified").emit_activate();
    assert_eq!(test.action_state("group-by").as_deref(), Some("modified"));
    wait_until("both menus to close", || {
        !menu.is_visible() && !group_by.is_visible()
    });
}

/// With Group by open, the submenu grabs the pointer, so its motion over
/// the Sort menu reaches the Sort menu through the submenu, as a position
/// on the Sort menu's surface: resting there on More still opens More's
/// submenu in Group by's place. Before, More stayed shut on KDE Plasma.
///
/// parity: VIEW-022
#[gtk::test]
fn the_pointer_back_from_group_by_still_opens_more() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.right_click(None);
    let menu = test.window.context_menu();
    menu.row("Sort by").emit_activate();
    wait_until("the Sort menu", || {
        menu.is_visible() && menu.row_labels().contains(&"Group by".to_owned())
    });
    menu.hover_row(Some("Group by"));
    wait_until("the Group by submenu", || {
        menu.open_submenu_menu()
            .is_some_and(|submenu| submenu.is_visible())
    });
    // The pointer goes into the submenu, then back over the Sort menu.
    menu.hover_row(None);
    menu.hover_row_through_submenu("Descending");
    menu.hover_row_through_submenu("More");
    wait_until("More's submenu in Group by's place", || {
        menu.open_submenu_menu()
            .is_some_and(|submenu| submenu.row_labels().first().map(String::as_str) == Some("Size"))
    });
    menu.hover_row_through_submenu("Folders first");
    wait_until("the submenu to close", || menu.open_submenu_menu().is_none());
    assert!(menu.is_visible(), "the Sort menu stays open");
}

/// Resting the pointer on Group by opens its submenu after a moment, and
/// resting it on another row closes it again, as Windows' menus do.
///
/// parity: VIEW-022
#[gtk::test]
fn hovering_group_by_opens_its_submenu_beside_the_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.right_click(None);
    let menu = test.window.context_menu();
    menu.row("Sort by").emit_activate();
    wait_until("the Sort menu", || {
        menu.is_visible() && menu.row_labels().contains(&"Group by".to_owned())
    });
    menu.hover_row(Some("Group by"));
    assert!(menu.open_submenu_menu().is_none(), "not at once");
    wait_until("the submenu after a moment", || {
        menu.open_submenu_menu()
            .is_some_and(|submenu| submenu.is_visible())
    });
    let submenu = menu.open_submenu_menu().expect("open");
    assert!(
        !submenu.is_ancestor(&menu.row_list()),
        "the pointer over the submenu does not hover the menu's rows"
    );
    menu.hover_row(Some("More"));
    wait_until("More's submenu in its place", || {
        menu.open_submenu_menu()
            .is_some_and(|submenu| submenu.row_labels().first().map(String::as_str) == Some("Size"))
    });
    menu.hover_row(Some("Ascending"));
    wait_until("the submenu to close", || menu.open_submenu_menu().is_none());
    assert!(menu.is_visible(), "the Sort menu stays open");

    let submenu = |label: &str| -> Vec<String> {
        sort_menu()
            .into_iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == label => Some(item.submenu),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the Sort menu has {label}"))
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label,
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    };
    assert_eq!(
        submenu("More"),
        [
            "Size",
            "Date created",
            "Date accessed",
            "File extension",
            "Permissions",
            "Owner",
            "User group",
            "Link destination"
        ]
    );
    assert_eq!(
        submenu("Group by"),
        [
            "Name",
            "Date modified",
            "Type",
            "Size",
            "Date created",
            "Same as sort",
            "(None)"
        ]
    );
}

/// Turning groups on and off again in Details, as often as the user
/// likes, keeps the list and saves the choice. In 2.0.2, turning "Show in
/// groups" off aborted in `FolderModel::set_grouping`, so the choice was
/// never saved and the groups could not be turned off.
///
/// parity: VIEW-022
#[gtk::test]
fn groups_turn_on_and_off_again_in_details() {
    let fixture = Fixture::standard();
    // Shown and laid out: GTK builds group headings only for a list on screen.
    let test = super::geometry::laid_out(&fixture.uri());
    test.activate("view", Some("details"));
    test.wait_for_listing("the folder");
    let listed = test.names();

    for round in 0..3 {
        for by in ["sort", "modified", "name"] {
            test.activate("group-by", Some(by));
            assert!(!headings(&test).is_empty(), "round {round}: grouped by {by}");

            test.activate("group-by", Some("none"));
            assert!(headings(&test).is_empty(), "round {round}: {by} turned off");
            assert_eq!(test.names(), listed, "round {round}: the list is whole again");
            assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
            wait_until("groups off to be saved", || {
                let saved = test.context.settings_data().preferences.view_defaults;
                saved.is_some_and(|style| style.grouping() == GroupBy::None)
            });
        }
    }
}
