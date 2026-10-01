// SPDX-License-Identifier: AGPL-3.0-only
//! Properties, previous versions and folder sizes in a real window.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, these also save
//! `native-properties-general.png`, `native-properties-versions.png`,
//! `native-previous-version-tab.png` and `native-size-scan.png`.

use std::fs;

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{FileManagerMethod, FileManagerRequest};

use super::icons::{assert_same_colour, css_colour, painted_colour, TRANSITION_TIME};
use crate::dialog_layer::DialogFrame;
use crate::integration::OpenWithDialog;
use crate::properties::{FolderSizeState, PropertiesView, RestoreRequest, SnapshotTarget};
use crate::test_support::harness::{
    capture, descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::tests::file_ops_support::open_dialog;
use crate::window::widget_tree::children;
use crate::window::WindowAction;

/// The name of the snapshot the fixtures create.
pub(super) const SNAPSHOT_NAME: &str = "daily-2026-09-05_1230";

impl TestWindow {
    /// Selects only the item called `name`.
    pub(crate) fn select_named(&self, name: &str) {
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
pub(super) fn properties_view(frame: &DialogFrame) -> PropertiesView {
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
pub(super) fn value_after(widget: &impl IsA<gtk::Widget>, name: &str) -> Option<String> {
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
pub(super) fn fixture_with_snapshot() -> Fixture {
    let fixture = Fixture::standard();
    let documents = fixture.path("Documents");
    fs::write(documents.join("plan.txt"), b"live plan").expect("fixture file");
    let snapshot = documents.join(".snapshot").join(SNAPSHOT_NAME);
    fs::create_dir_all(&snapshot).expect("snapshot folder");
    fs::write(snapshot.join("plan.txt"), b"earlier plan").expect("snapshot file");
    fixture
}

/// `ShowItemProperties` from another application opens the item's folder
/// in a new tab with the item selected, and its Properties over it.
///
/// parity: INT-014
#[gtk::test]
fn show_item_properties_opens_the_folder_and_the_properties() {
    let fixture = Fixture::standard();
    fixture.write("Documents/report.txt");
    let test = TestWindow::open(&fixture.uri());
    let item = [fixture.uri_of("Documents/report.txt")];
    let request =
        FileManagerRequest::new(FileManagerMethod::ShowItemProperties, &item).expect("a valid location");

    test.window.show_file_manager_request(&request);

    let frame = test.wait_for_dialog("the Properties dialog");
    assert_eq!(frame.title(), "report.txt Properties");
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents listing");
    wait_until("the selection", || test.selected_names() == ["report.txt"]);
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
    assert_eq!(
        view.tab_labels(),
        ["General", "Permissions", "Checksums", "Previous versions"]
    );
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

/// A file's Checksums tab checks a pasted checksum, computing its
/// algorithm first, and a folder has no such tab.
///
/// parity: PROP-014
#[gtk::test]
fn a_pasted_checksum_is_checked_against_the_file() {
    use ox_core::checksums::ChecksumKind;

    let fixture = Fixture::standard();
    let data = fs::read(fixture.path("Notes 2.txt")).expect("the fixture file");
    let sha256 = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &data).expect("a digest");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let view = properties_view(&test.wait_for_dialog("the Properties dialog"));
    let checksums = view.checksums().expect("a file has checksums");

    checksums.paste_expected(&sha256.to_uppercase());
    wait_until("the check", || checksums.verdict_text() == "Checksums match.");
    assert_eq!(checksums.value_text(ChecksumKind::Sha256), sha256.as_str());
    assert_eq!(checksums.value_text(ChecksumKind::Md5), "Not calculated");
    checksums.paste_expected(&"0".repeat(32));
    wait_until("the MD5 check", || {
        checksums.verdict_text() == "Checksums do not match."
    });
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
    assert!(properties_view(&frame).checksums().is_none());
    assert_eq!(value_after(&general, "Contains").as_deref(), Some("Not scanned"));
}

/// The name in Properties renames the item on Enter, refusing a taken
/// name inside the dialog, and the dialog closes once it is renamed.
///
/// parity: PROP-005
#[gtk::test]
fn the_name_in_properties_renames_the_item() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Taken.txt"), b"keep").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let general = properties_view(&frame).general_panel();
    wait_until("the name field", || {
        !descendants::<gtk::Entry>(&general).is_empty()
    });
    let name = descendants::<gtk::Entry>(&general).remove(0);
    assert_eq!(name.text(), "Notes 2.txt");

    name.set_text("Taken.txt");
    name.emit_activate();
    wait_until("the refusal", || !frame.error_text().is_empty());
    assert_eq!(fs::read(fixture.path("Taken.txt")).expect("kept"), b"keep");
    name.set_text("Renamed.txt");
    name.emit_activate();

    wait_until("the rename", || fixture.path("Renamed.txt").exists());
    assert!(!fixture.path("Notes 2.txt").exists());
    wait_until("the dialog to close", || frame.is_closed());
}

/// The owner changes who may view or modify a file on the Permissions
/// tab.
///
/// parity: PROP-007
#[gtk::test]
fn the_owner_changes_a_files_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::standard();
    let path = fixture.path("Notes 2.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("a known mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let permissions = properties_view(&frame).permissions_panel();
    wait_until("the editor", || {
        !descendants::<gtk::DropDown>(&permissions).is_empty()
    });
    let choices = descendants::<gtk::DropDown>(&permissions);
    assert_eq!(choices.len(), 4, "owner, group and others access, and the group");
    assert_eq!(choices[1].selected(), 1, "the group can only view");

    choices[1].set_selected(0);
    choices[2].set_selected(0);
    press(&permissions, "Apply permissions");

    let mode = || fs::metadata(&path).expect("metadata").permissions().mode() & 0o7777;
    wait_until("the new mode", || mode() == 0o600);
    assert_eq!(test.window.shown_message(), "Permissions changed.");

    // Advanced Permissions set single bits, such as Others Exec and
    // Set GID, which the three accesses cannot say.
    let expander = descendants::<gtk::Expander>(&permissions).remove(0);
    expander.set_expanded(true);
    assert!(
        !choices[0].is_sensitive(),
        "the advanced bits replace the accesses"
    );
    let set_gid = descendants::<gtk::CheckButton>(&permissions)
        .into_iter()
        .find(|check| check.label().as_deref() == Some("Set GID"))
        .expect("a Set GID check box");
    set_gid.set_active(true);
    press(&permissions, "Apply permissions");
    wait_until("the advanced mode", || mode() == 0o2600);
}

/// Properties of several items total them, folders' content included,
/// and a permission change reaches every one while every bit the user did
/// not change stays as each item has it.
///
/// parity: PROP-002
#[gtk::test]
fn properties_of_several_items_total_them_and_change_them_together() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::standard();
    fixture.write("Documents/inside.txt");
    for (name, mode) in [
        ("Documents", 0o755),
        ("Notes 2.txt", 0o644),
        ("Notes 10.txt", 0o700),
    ] {
        fs::set_permissions(fixture.path(name), fs::Permissions::from_mode(mode)).expect("a known mode");
    }
    let test = TestWindow::open(&fixture.uri());
    super::file_ops_support::select_names(&test, &["Documents", "Notes 2.txt", "Notes 10.txt"]);
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");

    assert_eq!(frame.title(), "3 items Properties");
    let panel_heights: Vec<i32> = descendants::<gtk::Widget>(&frame)
        .into_iter()
        .filter(|widget| widget.has_css_class("properties-panel"))
        .map(|panel| panel.size_request().1)
        .collect();
    assert_eq!(
        panel_heights,
        [290, 290],
        "General and Permissions keep the dialog's height, as one item's tabs do"
    );
    wait_until("the folders measured", || {
        value_after(&frame, "Contains").as_deref() == Some("3 files, 1 folder")
    });
    assert_eq!(value_after(&frame, "Size").as_deref(), Some("60 bytes"));
    assert_eq!(
        value_after(&frame, "Location").as_deref(),
        Some(format!("All in {}", fixture.path("").display()).trim_end_matches('/'))
    );
    wait_until("the editor", || !descendants::<gtk::DropDown>(&frame).is_empty());
    let choices = descendants::<gtk::DropDown>(&frame);
    assert_eq!(choices[0].selected(), 2, "every owner may view and modify");
    assert_eq!(choices[2].selected(), 3, "Varying (No Change)");
    let executable = descendants::<gtk::CheckButton>(&frame)
        .into_iter()
        .find(|check| check.label().as_deref() == Some("Is executable"))
        .expect("the executable check box");
    assert!(executable.is_inconsistent(), "one file is executable, one is not");
    let apply = descendants::<gtk::Button>(&frame)
        .into_iter()
        .find(|button| button_label(button).as_deref() == Some("Apply permissions"))
        .expect("the Apply button");
    assert!(!apply.is_sensitive(), "nothing to apply before a change");

    choices[2].set_selected(0);
    assert!(apply.is_sensitive());
    apply.emit_clicked();

    // Only the others lose their access; every other bit stays as each
    // item had it.
    let mode = |name: &str| {
        fs::metadata(fixture.path(name))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o7777
    };
    wait_until("every item changed", || mode("Notes 2.txt") == 0o640);
    assert_eq!(
        mode("Notes 10.txt"),
        0o700,
        "the private program stays private and runnable"
    );
    assert_eq!(mode("Documents"), 0o750);
}

/// A link says where it points, and a mount point what is mounted there
/// and how much space is free.
///
/// parity: PROP-004
#[gtk::test]
fn properties_show_a_links_target_and_a_mount_points_details() {
    let fixture = Fixture::standard();
    std::os::unix::fs::symlink("Documents", fixture.path("Shortcut")).expect("a link");
    let test = TestWindow::open(&fixture.uri());

    test.activate("properties-of", Some(&fixture.uri_of("Shortcut")));
    let frame = test.wait_for_dialog("the link's Properties");
    let general = properties_view(&frame).general_panel();
    wait_until("the link's target", || {
        value_after(&general, "Points to").is_some()
    });
    assert_eq!(value_after(&general, "Points to").as_deref(), Some("Documents"));
    frame.close();

    test.activate("properties-of", Some("file:///"));
    let frame = test.wait_for_dialog("the root's Properties");
    let general = properties_view(&frame).general_panel();
    wait_until("the mount's details", || {
        value_after(&general, "Mounted on").is_some()
    });
    assert_eq!(value_after(&general, "Mounted on").as_deref(), Some("/"));
    assert!(value_after(&general, "File system").is_some());
    assert!(value_after(&general, "Free space").is_some_and(|text| text.contains(" free of ")));
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

/// Escape dismisses Properties as Close does, and a tab with Properties
/// stays in its window until they close (`tabCanMove`).
///
/// parity: PROP-008
#[gtk::test]
fn escape_closes_properties_and_a_tab_with_properties_stays_in_its_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let owner = test.active_tab().expect("a tab");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let move_to_new_window = |test: &TestWindow| {
        let action = format!("win.{}", WindowAction::MoveTabToNewWindow.name());
        let target = owner.to_raw().to_variant();
        WidgetExt::activate_action(&test.window, &action, Some(&target)).expect("the action exists");
    };

    move_to_new_window(&test);

    assert_eq!(test.window.tab_count(), 1, "the tab stayed");
    assert_eq!(
        test.window.shown_message_text(),
        "Close this tab’s dialog and finish file operations before moving it."
    );
    press_escape(&test);
    assert!(frame.is_closed(), "Escape dismissed the dialog");
    assert!(!test.window.has_properties(owner));
}

/// Presses Escape on the window's dialog layer.
fn press_escape(test: &TestWindow) {
    let layer = test.window.dialog_layer();
    let controllers = layer.observe_controllers();
    let shortcut = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            controller
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .collect::<Vec<_>>()
        })
        .find_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
        .expect("the layer has the Escape shortcut");
    let action = shortcut.action().expect("the shortcut has an action");
    action.activate(gtk::ShortcutActionFlags::empty(), layer, None);
}

/// Change app… closes Properties and opens Open with for the file.
///
/// parity: PROP-003
#[gtk::test]
fn change_app_closes_properties_and_opens_open_with() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let general = properties_view(&frame).general_panel();
    wait_until("the properties to be read", || {
        value_after(&general, "Type").is_some()
    });

    press(&general, "Change app…");

    assert!(frame.is_closed(), "Properties closed");
    let open_with = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<OpenWithDialog>().ok())
        .find(|dialog| dialog.transient_for().as_ref() == Some(test.window.upcast_ref()))
        .expect("Open with opened for the file");
    open_with.close();
}

/// Ctrl+Tab and Ctrl+Shift+Tab switch tabs while a tab's Properties is
/// open, whatever has focus in it.
///
/// parity: CMD-017
#[gtk::test]
fn ctrl_tab_switches_tabs_from_inside_a_tabs_properties() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let owner = test.active_tab().expect("a tab");
    test.activate("new-tab", None);
    test.activate_tab(owner);
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let in_dialog = descendants::<gtk::Button>(&frame)
        .iter()
        .any(WidgetExt::grab_focus);
    assert!(in_dialog, "a control of the dialog takes focus");
    let keys_apply = test.window.tab_keys_apply();

    test.activate("next-tab", None);
    let after_next = (test.active_tab(), test.shown_dialog());
    test.activate("previous-tab", None);

    assert!(keys_apply, "the tab keys reach the window from the dialog");
    assert_ne!(after_next.0, Some(owner));
    assert!(after_next.1.is_none(), "the other tab hides the dialog");
    assert_eq!(test.active_tab(), Some(owner));
    assert_eq!(test.shown_dialog(), Some(frame));
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

/// A file inside a snapshot is never handed to an application that could
/// change it; opening it says, in "Could not open the item", to restore a
/// copy first.
///
/// parity: PROP-024
#[gtk::test]
fn a_file_in_a_snapshot_does_not_open_in_an_application() {
    let fixture = fixture_with_snapshot();
    let snapshot = fixture.path("Documents").join(".snapshot").join(SNAPSHOT_NAME);
    let test = TestWindow::open(&ox_core::location::file_uri(&snapshot));

    test.window.activate_item(test.position_of("plan.txt"));

    let dialog = open_dialog(&test);
    assert_eq!(
        dialog.message_text(),
        "Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first."
    );
    assert!(test.context.recorded_launches().is_empty());
    dialog.press("OK");
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

/// `ShowItemProperties` opens the item's folder in a new tab with the
/// item selected, and the item's Properties over it.
///
/// parity: PROP-001, INT-014
#[gtk::test]
fn show_item_properties_opens_the_properties_of_the_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let item = [fixture.uri_of("Documents")];
    let request =
        FileManagerRequest::new(FileManagerMethod::ShowItemProperties, &item).expect("a valid location");

    test.window.show_file_manager_request(&request);

    let frame = test.wait_for_dialog("the Properties dialog");
    assert_eq!(frame.title(), "Documents Properties");
    assert_eq!(test.window.tab_count(), 2);
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
    let fields = descendants::<gtk::Entry>(&versions);
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

/// A custom icon is stored as the `GVfs` metadata Files reads, shows in the
/// folder view, and can be restored to the default art.
///
/// parity: PROP-016
#[gtk::test]
fn a_custom_icon_shows_in_the_view_until_the_default_is_restored() {
    use crate::folder_view::cells::FileCell;
    use crate::properties::set_custom_icon;

    let fixture = Fixture::standard();
    let pixels = glib::Bytes::from_owned(vec![0x80_u8; 2 * 2 * 4]);
    let picture = gtk::gdk::MemoryTexture::new(2, 2, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    picture
        .save_to_png(fixture.path("Star.png"))
        .expect("an icon picture");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let general = properties_view(&frame).general_panel();
    wait_until("the icon button", || {
        texts(&general).contains(&"Change icon…".to_owned())
    });
    frame.close();
    let item = fixture.uri_of("Notes 2.txt");
    let shows_custom = |wanted: bool| {
        descendants::<FileCell>(&test.window)
            .iter()
            .any(|cell| cell.name() == "Notes 2.txt" && cell.shows_custom_icon() == wanted)
    };
    let change = |icon: Option<String>| {
        let (target, done) = (item.clone(), std::rc::Rc::new(std::cell::Cell::new(false)));
        let finished = std::rc::Rc::clone(&done);
        glib::spawn_future_local(async move {
            set_custom_icon(&target, icon)
                .await
                .expect("the metadata is stored");
            finished.set(true);
        });
        wait_until("the change", || done.get());
        test.window.refresh_item_icon(&item);
    };

    change(Some(fixture.uri_of("Star.png")));
    wait_until("the custom icon", || shows_custom(true));
    let stored = gtk::gio::File::for_uri(&item)
        .query_info(
            "metadata::custom-icon",
            gtk::gio::FileQueryInfoFlags::NONE,
            gtk::gio::Cancellable::NONE,
        )
        .expect("metadata")
        .attribute_string("metadata::custom-icon");
    assert_eq!(stored.as_deref(), Some(fixture.uri_of("Star.png").as_str()));
    change(None);
    wait_until("the default art", || shows_custom(false));
}
