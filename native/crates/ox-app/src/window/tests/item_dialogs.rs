// SPDX-License-Identifier: AGPL-3.0-only
//! Properties, previous versions and folder sizes in a real window.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, these also save
//! `native-properties-general.png`, `native-properties-versions.png`,
//! `native-previous-version-tab.png` and `native-size-scan.png`.

use std::fs;

use gtk::{gdk, glib};
use gtk::prelude::*;

use crate::dialog_layer::DialogFrame;
use crate::properties::{FolderSizeState, PropertiesView, RestoreRequest, SnapshotTarget};
use super::icons::{assert_same_colour, css_colour, painted, pixel_rows, unpremultiplied_rgb, TRANSITION_TIME};
use crate::test_support::harness::{
    capture, descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::widget_tree::children;

/// The name of the snapshot the fixtures create.
const SNAPSHOT_NAME: &str = "daily-2026-09-05_1230";

impl TestWindow {
    /// Selects only the item called `name`.
    pub(super) fn select_named(&self, name: &str) {
        let position = self
            .names()
            .iter()
            .position(|shown| shown == name)
            .unwrap_or_else(|| panic!("{name} is listed"));
        let position = u32::try_from(position).expect("a short listing");
        self.window.folder_model().select_only(position);
    }

    /// The dialog shown on the window's dialog layer.
    pub(super) fn shown_dialog(&self) -> Option<DialogFrame> {
        self.window.dialog_layer().shown()
    }

    /// Waits for a dialog and returns it.
    pub(super) fn wait_for_dialog(&self, what: &str) -> DialogFrame {
        wait_until(what, || self.shown_dialog().is_some());
        self.shown_dialog().expect("a dialog is shown")
    }
}

/// The Properties view inside `frame`.
fn properties_view(frame: &DialogFrame) -> PropertiesView {
    descendants::<PropertiesView>(frame)
        .into_iter()
        .next()
        .expect("a Properties dialog holds its view")
}

/// Every text shown in `widget`.
pub(super) fn texts(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    let labels = descendants::<gtk::Label>(widget);
    labels.iter().map(|label| label.text().to_string()).collect()
}

/// The value shown after the name `name` in `widget`'s name-value grids.
fn value_after(widget: &impl IsA<gtk::Widget>, name: &str) -> Option<String> {
    let shown = texts(widget);
    let index = shown.iter().position(|text| text == name)?;
    shown.get(index + 1).cloned()
}

/// Presses the button labelled `label` in `widget`.
pub(super) fn press(widget: &impl IsA<gtk::Widget>, label: &str) {
    let button = descendants::<gtk::Button>(widget)
        .into_iter()
        .find(|button| button_label(button).as_deref() == Some(label))
        .unwrap_or_else(|| panic!("a {label} button"));
    button.emit_clicked();
}

/// A button's text: its label, or the label inside it.
fn button_label(button: &gtk::Button) -> Option<String> {
    if let Some(label) = button.label() {
        return Some(label.to_string());
    }
    let inner = descendants::<gtk::Label>(button).into_iter().next()?;
    Some(inner.text().to_string())
}

/// The tooltips of the window's tabs, left to right.
fn tab_tooltips(test: &TestWindow) -> Vec<String> {
    let tabs = children(&test.window.tab_strip().tab_list());
    tabs.filter_map(|tab| tab.tooltip_text())
        .map(String::from)
        .collect()
}

/// A standard fixture whose Documents folder holds a file and a snapshot
/// collection with one snapshot of the folder.
fn fixture_with_snapshot() -> Fixture {
    let fixture = Fixture::standard();
    let documents = fixture.path("Documents");
    fs::write(documents.join("plan.txt"), b"live plan").expect("fixture file");
    let snapshot = documents.join(".snapshot").join(SNAPSHOT_NAME);
    fs::create_dir_all(&snapshot).expect("snapshot folder");
    fs::write(snapshot.join("plan.txt"), b"earlier plan").expect("snapshot file");
    fixture
}

/// parity: PROP-001, PROP-003, PROP-006
#[gtk::test]
fn alt_enter_opens_the_properties_of_the_selected_file() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");

    test.activate("properties", None);

    let frame = test.wait_for_dialog("the Properties dialog");
    assert_eq!(frame.title(), "Notes 2.txt Properties");
    let view = properties_view(&frame);
    assert_eq!(view.tab_labels(), ["General", "Permissions", "Previous versions"]);
    let general = view.general_panel();
    wait_until("the properties to be read", || {
        value_after(&general, "Type").is_some()
    });
    assert_eq!(value_after(&general, "Type").as_deref(), Some("Text document"));
    assert_eq!(value_after(&general, "Size").as_deref(), Some("20 bytes"));
    assert_eq!(
        value_after(&general, "Full path"),
        Some(fixture.path("Notes 2.txt").display().to_string())
    );
    assert!(value_after(&general, "Opens with").is_some());
    assert!(texts(&general).iter().any(|text| text == "Copy full path"));
    let permissions = view.permissions_panel();
    assert_eq!(
        value_after(&permissions, "Owner"),
        Some(glib::user_name().to_string_lossy().into_owned())
    );
    assert_eq!(value_after(&permissions, "Readable").as_deref(), Some("Yes"));
    assert!(value_after(&permissions, "POSIX mode").is_some_and(|mode| mode.starts_with("0o")));
    assert_eq!(frame.button_labels(), ["Close"]);
    capture(&test.window, "native-properties-general.png");
}

/// parity: PROP-001
#[gtk::test]
fn properties_without_a_selection_describe_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("properties", None);

    let frame = test.wait_for_dialog("the folder's Properties");
    assert_eq!(frame.title(), "Example projects Properties");
    let general = properties_view(&frame).general_panel();
    wait_until("the properties to be read", || {
        value_after(&general, "Size").is_some()
    });
    assert_eq!(value_after(&general, "Size").as_deref(), Some("Not scanned"));
    assert!(texts(&general).iter().any(|text| text == "Calculate folder size"));
}

/// parity: PROP-008
#[gtk::test]
fn properties_belong_to_the_tab_that_opened_them() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let owner = test.active_tab().expect("a tab");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    assert!(tab_tooltips(&test)[0].ends_with(" · Properties open"));

    test.activate("new-tab", None);

    assert!(test.shown_dialog().is_none(), "another tab hides the dialog");
    test.activate_tab(owner);
    assert_eq!(
        test.shown_dialog(),
        Some(frame.clone()),
        "its tab shows the same dialog"
    );
    press(&frame, "Close");
    assert!(test.shown_dialog().is_none());
    assert!(!tab_tooltips(&test)[0].contains("Properties open"));
}

/// parity: PROP-008
#[gtk::test]
fn closing_a_tab_discards_its_properties() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let owner = test.active_tab().expect("a tab");
    test.activate("new-tab", None);

    test.activate_tab_close(owner);

    assert!(frame.is_closed(), "the dialog was discarded with its tab");
    assert!(test.shown_dialog().is_none());
}

/// parity: PROP-026, PROP-027
#[gtk::test]
fn calculate_folder_size_shows_the_size_everywhere_the_folder_is() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents").join("a.txt"), b"0123456789").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let view = properties_view(&frame);
    wait_until("the properties to be read", || view.size_text().is_some());
    assert_eq!(view.size_text().as_deref(), Some("Not scanned"));

    test.activate("calculate-folder-size-of", Some(&fixture.uri_of("Documents")));

    wait_until("the scan to finish", || {
        view.size_text().as_deref() == Some("10 bytes")
    });
    let strip = test.window.size_strip();
    wait_until("the bar to show the end", || strip.button_label() == "Dismiss");
    assert_eq!(
        strip.text(),
        "Size scan finished · 1 complete · Logical bytes; recalculate after changes"
    );
    let item = test.window.folder_model().selected_items()[0].clone();
    assert!(item.folder_size().is_some_and(|size| size.is_complete()));
    assert_eq!(
        item.folder_size().map(|size| size.size_text()).as_deref(),
        Some("10 bytes")
    );
    capture(&test.window, "native-size-scan.png");
    press(&frame, "Close");
    test.activate("cancel-size-scan", None);
    assert!(!strip.is_visible(), "Dismiss hides the finished bar");
}

/// parity: PROP-026, PROP-029
#[gtk::test]
fn calculate_folder_sizes_measures_every_folder_shown_and_one_scan_runs_at_a_time() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Music")).expect("fixture folder");
    let test = TestWindow::open(&fixture.uri());

    test.activate("calculate-folder-sizes", None);
    test.activate("calculate-folder-size-of", Some(&fixture.uri_of("Documents")));

    assert_eq!(
        test.window.shown_message(),
        "Cancel or finish the current folder-size scan first."
    );
    let strip = test.window.size_strip();
    wait_until("the run to finish", || strip.button_label() == "Dismiss");
    assert!(
        strip.text().starts_with("Size scan finished · 2 complete"),
        "{}",
        strip.text()
    );
    let measured: Vec<Option<FolderSizeState>> = ["Documents", "Music"]
        .iter()
        .map(|name| test.window.measured_folder_size(&fixture.uri_of(name)))
        .collect();
    assert!(measured
        .iter()
        .all(|size| size.as_ref().is_some_and(FolderSizeState::is_complete)));
}

/// parity: PROP-026
#[gtk::test]
fn a_request_without_folders_says_what_to_select() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("calculate-folder-size-of", Some("mtp://phone/DCIM"));

    assert_eq!(
        test.window.shown_message(),
        "Select a folder or share to calculate its size."
    );
}

/// parity: PROP-019, PROP-020, PROP-032
#[gtk::test]
fn previous_versions_lists_the_snapshots_of_the_folder() {
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");

    test.activate("previous-versions", None);

    let frame = test.wait_for_dialog("the Previous versions tab");
    let versions = properties_view(&frame).versions();
    wait_until("the versions", || !versions.version_labels().is_empty());
    assert_eq!(versions.version_labels(), [SNAPSHOT_NAME]);
    let shown = versions.texts();
    assert!(shown.iter().any(|text| text == "12:30"), "{shown:?}");
    assert!(shown.iter().any(|text| text.contains("2026")), "{shown:?}");
    assert!(shown
        .iter()
        .any(|text| text.starts_with("Dates are read from snapshot names.")));
    assert!(texts(&frame).iter().any(|text| text == "Browse"));
    assert!(texts(&frame).iter().any(|text| text == "Restore a copy…"));
    capture(&test.window, "native-properties-versions.png");
}

/// parity: PROP-019
#[gtk::test]
fn a_file_without_snapshots_explains_that_none_were_found() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");

    test.activate("previous-versions", None);

    let frame = test.wait_for_dialog("the Previous versions tab");
    let versions = properties_view(&frame).versions();
    wait_until("the lookup", || {
        versions
            .texts()
            .iter()
            .any(|text| text == "No accessible previous versions")
    });
    assert!(versions
        .texts()
        .iter()
        .any(|text| text.starts_with("No matching previous versions were found")));
}

/// parity: PROP-021, PROP-022, PROP-024, LOOK-023
#[gtk::test]
fn browse_opens_the_snapshot_in_a_marked_tab_with_its_banner() {
    let _theme = ThemeGuard::keep();
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    let first_tab = test.active_tab().expect("a tab");
    let root = format!("{}/.snapshot/{SNAPSHOT_NAME}", fixture.uri_of("Documents"));
    let target = SnapshotTarget {
        uri: root.clone(),
        root: root.clone(),
        label: SNAPSHOT_NAME.to_owned(),
    };

    WidgetExt::activate_action(&test.window, "win.browse-snapshot", Some(&target.to_variant()))
        .expect("the window browses snapshots");
    test.wait_for_listing("the snapshot");

    assert_eq!(test.window.current_uri(), Some(root));
    assert_eq!(test.names(), ["plan.txt"]);
    let banner = test.window.snapshot_banner();
    assert!(banner.is_visible());
    assert_eq!(banner.date_text(), "5 Sep 2026 · 12:30");
    let tooltips = tab_tooltips(&test);
    assert!(
        tooltips[1].ends_with(&format!(" · Previous version · {SNAPSHOT_NAME}")),
        "{tooltips:?}"
    );
    capture(&test.window, "native-previous-version-tab.png");
    assert_amber_marking(&test, banner);
    test.activate_tab(first_tab);
    assert!(!banner.is_visible(), "a live folder has no banner");
}

/// The colour `widget` paints at (`x`, `y`) of its own area.
fn painted_colour(widget: &impl IsA<gtk::Widget>, x: i32, y: i32) -> gdk::RGBA {
    let rows = pixel_rows(&painted(widget));
    let row = usize::try_from(y).expect("a point inside the widget");
    let column = usize::try_from(x).expect("a point inside the widget");
    let [red, green, blue] = unpremultiplied_rgb(rows[row][column]);
    gdk::RGBA::new(red, green, blue, 1.0)
}

/// Asserts the amber of a previous version in both appearances: the
/// banner, the badge and the top edge of the snapshot's tab.
fn assert_amber_marking(test: &TestWindow, banner: &impl IsA<gtk::Widget>) {
    let cases = [
        ("light", "#fff7e8", "#775314", "#fff2d6"),
        ("dark", "#302a20", "#ecc993", "#443721"),
    ];
    for (theme, banner_bg, banner_text, badge_bg) in cases {
        test.activate("theme", Some(theme));
        wait_for(TRANSITION_TIME);
        wait_for_frames(&test.window, 2);
        // The tab strip draws its tabs again for a new appearance.
        let tab = descendants::<gtk::Widget>(test.window.tab_strip())
            .into_iter()
            .find(|widget| widget.has_css_class("snapshot-tab"))
            .expect("the snapshot's tab is marked");
        let badge = descendants::<gtk::Widget>(&tab)
            .into_iter()
            .find(|widget| widget.has_css_class("snapshot-tab-badge"))
            .expect("the tab has the badge");
        let banner_colour = painted_colour(banner, banner.width() - 3, 3);
        assert_same_colour(banner_colour, css_colour(banner_bg), theme);
        assert_same_colour(banner.color(), css_colour(banner_text), theme);
        let badge_colour = painted_colour(&badge, 1, badge.height() / 2);
        assert_same_colour(badge_colour, css_colour(badge_bg), theme);
        let edge = painted_colour(&tab, tab.width() / 2, 0);
        assert_same_colour(edge, css_colour("#c48c2f"), theme);
    }
}

/// parity: PROP-025
#[gtk::test]
fn restore_a_copy_copies_the_version_into_a_live_folder_only() {
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    let version = fixture
        .path("Documents")
        .join(".snapshot")
        .join(SNAPSHOT_NAME)
        .join("plan.txt");
    let request = RestoreRequest {
        version_uri: ox_core::location::file_uri(&version),
        label: SNAPSHOT_NAME.to_owned(),
        name: "plan.txt".to_owned(),
    };
    WidgetExt::activate_action(&test.window, "win.restore-version", Some(&request.to_variant()))
        .expect("the window restores versions");
    let frame = test.wait_for_dialog("the Restore dialog");
    assert_eq!(frame.title(), "Restore a copy");
    let entry = descendants::<gtk::Entry>(&frame)
        .into_iter()
        .next()
        .expect("a destination field");

    entry.set_text(&fixture.path("Documents/.snapshot").display().to_string());
    press(&frame, "Copy version");
    assert_eq!(
        frame.error_text(),
        "Choose a folder outside the snapshot collection."
    );
    entry.set_text(&fixture.root().display().to_string());
    press(&frame, "Copy version");

    let restored = fixture.path("plan.txt");
    wait_until("the restored copy", || restored.exists());
    assert_eq!(fs::read(&restored).expect("the copy"), b"earlier plan");
    assert_eq!(
        fs::read(fixture.path("Documents/plan.txt")).expect("the live file"),
        b"live plan"
    );
}

/// parity: PROP-001
#[gtk::test]
fn properties_of_a_location_open_for_menus_outside_the_folder_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("properties-of", Some(&fixture.uri_of("Documents")));

    let frame = test.wait_for_dialog("the Properties dialog");
    assert_eq!(frame.title(), "Documents Properties");
    let general = properties_view(&frame).general_panel();
    wait_until("the properties to be read", || {
        value_after(&general, "Size").is_some()
    });
    assert_eq!(value_after(&general, "Size").as_deref(), Some("Not scanned"));
}

/// parity: PROP-026
#[gtk::test]
fn cancel_scan_stops_the_run_and_says_so() {
    let fixture = Fixture::standard();
    for number in 0..5 {
        fs::create_dir(fixture.path(&format!("Folder {number}"))).expect("fixture folder");
    }
    let test = TestWindow::open(&fixture.uri());

    test.activate("calculate-folder-sizes", None);
    test.activate("cancel-size-scan", None);

    let strip = test.window.size_strip();
    wait_until("the run to end", || strip.button_label() == "Dismiss");
    assert_eq!(
        strip.text(),
        "Size scan cancelled · Logical bytes; recalculate after changes"
    );
}

/// parity: PROP-023
#[gtk::test]
fn the_snapshot_source_form_saves_a_mapping_and_lists_again() {
    let fixture = fixture_with_snapshot();
    let backups = fixture.path("Backups");
    fs::create_dir(&backups).expect("a backup collection");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");
    test.activate("previous-versions", None);
    let frame = test.wait_for_dialog("the Previous versions tab");
    let versions = properties_view(&frame).versions();
    wait_until("the versions", || !versions.version_labels().is_empty());

    press(&frame, "Snapshot source…");
    let fields = descendants::<gtk::Entry>(&frame);
    fields[1].set_text(&backups.display().to_string());
    press(&frame, "Save source");

    wait_until("the list again", || {
        versions.texts().iter().any(|text| text == "Refresh")
    });
    let sources = test.context.previous_versions().sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].live(), fixture.uri_of("Documents"));
    assert_eq!(sources[0].collection(), ox_core::location::file_uri(&backups));
}
