// SPDX-License-Identifier: AGPL-3.0-only
//! Explorer's Group by in a real window (VIEW-022): Downloads grouped by
//! date out of the box, the Sort menu's choices, the headings of the
//! details view, and each folder remembering its choice.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use gtk::prelude::*;
use ox_core::grouping::GroupBy;
use ox_core::location::file_uri;
use ox_core::places::FolderLocations;

use crate::test_support::harness::{capture, descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::FolderView;

/// The headings the details view draws, top to bottom.
fn headings(test: &TestWindow) -> Vec<String> {
    wait_for_frames(&test.window, 3);
    let column_view = test.window.folder_pane().details().column_view().clone();
    let mut labels: Vec<(f64, String)> = descendants::<gtk::Label>(&column_view)
        .into_iter()
        .filter(|label| label.has_css_class("group-heading-label") && label.is_mapped())
        .filter_map(|label| {
            let point = label.compute_point(&column_view, &gtk::graphene::Point::new(0.0, 0.0))?;
            Some((f64::from(point.y()), label.text().to_string()))
        })
        .collect();
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    labels.into_iter().map(|(_, text)| text).collect()
}

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
        let long_ago = SystemTime::now() - Duration::from_secs(3 * 366 * 86_400);
        fs::File::options()
            .write(true)
            .open(&old)
            .and_then(|file| file.set_modified(long_ago))
            .expect("the date is set");
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

/// Downloads opens grouped by date with Explorer's headings; another
/// folder does not; choosing No grouping in Downloads is remembered.
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
    assert_eq!(test.window.folder_model().group_by(), GroupBy::Modified);
    capture(&test.window, "native-group-by-date.png");

    test.window
        .navigate(&file_uri(&home.home))
        .expect("the home folder");
    test.wait_for_listing("the home folder");
    assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
    assert!(headings(&test).is_empty(), "an ungrouped list has no heading");
    assert!(!test.window.folder_pane().details().shows_group_headings());

    test.window.navigate(&downloads).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert_eq!(test.action_state("group-by").as_deref(), Some("modified"));

    test.activate("group-by", Some("none"));
    assert!(headings(&test).is_empty());
    assert_eq!(test.names(), ["Installers", "old manual.pdf", "today.txt"]);
    wait_until("the choice to be saved", || {
        test.context
            .settings_data()
            .preferences
            .folder_group_by
            .get(&downloads)
            .is_some_and(|key| key == "none")
    });
    test.window
        .navigate(&file_uri(&home.home))
        .expect("the home folder");
    test.wait_for_listing("the home folder");
    test.window.navigate(&downloads).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
}

/// A folder grouped from the Sort menu shows its groups in both views,
/// headings in the details view only, and opens grouped in another
/// window too.
///
/// parity: VIEW-022
#[gtk::test]
fn a_folder_remembers_the_group_chosen_in_the_sort_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert_eq!(test.action_state("group-by").as_deref(), Some("none"));
    assert!(test.window.select_named("Notes 2.txt"));

    test.activate("group-by", Some("name"));
    assert_eq!(test.action_state("group-by").as_deref(), Some("name"));
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
    assert_eq!(headings(&test), ["Tiny (0 – 16 KB) (3)", "Unspecified (1)"]);
    capture(&test.window, "native-group-by-size.png");

    test.window
        .folder_pane()
        .show_view(FolderView::Icons(crate::folder_view::grid::IconSize::Large));
    assert_eq!(
        test.names().last().map(String::as_str),
        Some("Documents"),
        "the icon view lists the groups in order too"
    );
    test.window.folder_pane().show_view(FolderView::Details);

    let uri = fixture.uri();
    wait_until("the choice to be saved", || {
        test.context
            .settings_data()
            .preferences
            .folder_group_by
            .get(&uri)
            .is_some_and(|key| key == "size")
    });
    let beside = test.open_beside(&fixture.uri());
    assert_eq!(beside.action_state("group-by").as_deref(), Some("size"));
    assert_eq!(headings(&beside).len(), 2);
}

/// The first group's heading stays in sight: a folder opened from another
/// one, and a folder just regrouped, both show their list from the top,
/// where GTK's scroll anchor would keep the first row at the top edge
/// with the heading above it, or follow an item to the end.
///
/// parity: VIEW-022
#[gtk::test]
fn the_first_heading_is_shown_when_a_folder_opens_or_is_regrouped() {
    let home = TestHome::new();
    for number in 0..40 {
        fs::write(
            home.home.join("Downloads").join(format!("file {number}.txt")),
            "x",
        )
        .expect("a file");
    }
    let scroll = |test: &TestWindow| {
        wait_for_frames(&test.window, 5);
        test.window.folder_pane().details().vadjustment().value()
    };
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), home.locations(), |_| {});
    test.window.navigate(&home.downloads()).expect("Downloads");
    test.wait_for_listing("Downloads");
    assert!(scroll(&test) < 0.5, "Downloads opens at its first heading");
    assert_eq!(headings(&test).first().map(String::as_str), Some("Today (42)"));

    test.activate("group-by", Some("none"));
    test.activate("group-by", Some("name"));
    assert!(scroll(&test) < 0.5, "a regrouped folder is shown from the top");
    assert_eq!(headings(&test).first().map(String::as_str), Some("A – H (40)"));
}
