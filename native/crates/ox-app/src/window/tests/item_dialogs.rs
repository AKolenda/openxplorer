// SPDX-License-Identifier: AGPL-3.0-only
//! Properties windows, previous versions and folder sizes in a real
//! window.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, these also save
//! `native-properties-general.png`, `native-properties-folder.png`,
//! `native-properties-security.png`, `native-properties-versions.png`,
//! `native-previous-version-tab.png` and `native-size-scan.png`.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{FileManagerMethod, FileManagerRequest};
use ox_core::settings::FolderTemplate;

use super::icons::{assert_same_colour, css_colour, painted_colour, TRANSITION_TIME};
use crate::dialog::{Dialog, DialogFrame};
use crate::integration::OpenWithDialog;
use crate::properties::{FolderSizeState, PropertiesWindow, RestoreRequest, SnapshotTarget, VersionsPanel};
use crate::test_support::harness::{
    capture, capture_dialog, descendants, settle, wait_for, wait_for_frames, wait_until, Fixture, TestWindow,
    ThemeGuard,
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

    /// Waits for a dialog on the window's dialog layer and returns it.
    pub(super) fn wait_for_dialog(&self, what: &str) -> DialogFrame {
        wait_until(what, || self.shown_dialog().is_some());
        self.shown_dialog().expect("a dialog is shown")
    }

    /// Waits for a Properties window titled `title` and returns it.
    pub(super) fn wait_for_properties(&self, title: &str) -> PropertiesWindow {
        let find = || {
            self.window
                .properties_windows()
                .into_iter()
                .find(|window| window.title_text() == title)
        };
        wait_until(title, || find().is_some());
        find().expect("the Properties window is open")
    }
}

/// The page of `window`'s tab named `name`, once the item is read.
pub(super) fn tab(window: &PropertiesWindow, name: &str) -> gtk::Widget {
    window.tab_page(name).unwrap_or_else(|| panic!("a {name} tab"))
}

/// The General tab of `window`, once its rows show.
pub(super) fn general(window: &PropertiesWindow) -> gtk::Widget {
    let page = tab(window, "general");
    wait_until("the properties to be read", || {
        value_after(&page, "Location:").is_some()
    });
    page
}

/// Every text shown in `widget`.
pub(super) fn texts(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    let labels = descendants::<gtk::Label>(widget);
    labels.iter().map(|label| label.text().to_string()).collect()
}

/// The value shown after the name `name` in `widget`'s rows.
pub(super) fn value_after(widget: &impl IsA<gtk::Widget>, name: &str) -> Option<String> {
    let shown = texts(widget);
    let index = shown.iter().position(|text| text == name)?;
    shown.get(index + 1).cloned()
}

/// The button labelled `label` in `widget`.
pub(super) fn button(widget: &impl IsA<gtk::Widget>, label: &str) -> gtk::Button {
    descendants::<gtk::Button>(widget)
        .into_iter()
        .find(|button| button_label(button).as_deref() == Some(label))
        .unwrap_or_else(|| panic!("a {label} button"))
}

/// Presses the button labelled `label` in `widget`.
pub(super) fn press(widget: &impl IsA<gtk::Widget>, label: &str) {
    button(widget, label).emit_clicked();
}

/// The check box labelled `label` in `widget`.
pub(super) fn check(widget: &impl IsA<gtk::Widget>, label: &str) -> gtk::CheckButton {
    descendants::<gtk::CheckButton>(widget)
        .into_iter()
        .find(|check| check.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("a {label} check box"))
}

/// A button's text: its label, or the label inside it.
fn button_label(button: &gtk::Button) -> Option<String> {
    if let Some(label) = button.label() {
        return Some(label.to_string());
    }
    let inner = descendants::<gtk::Label>(button).into_iter().next()?;
    Some(inner.text().to_string())
}

/// The name field of a Properties window.
fn name_field(window: &PropertiesWindow) -> gtk::Entry {
    let page = general(window);
    descendants::<gtk::Entry>(&page).remove(0)
}

/// The message box or dialog shown over `window`, if any.
fn dialog_over_properties(window: &PropertiesWindow) -> Option<Dialog> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|toplevel| toplevel.downcast::<Dialog>().ok())
        .find(|dialog| dialog.is_visible() && dialog.transient_for().as_ref() == Some(window.upcast_ref()))
}

/// Waits for the message box over `window` and returns it.
fn message_over(window: &PropertiesWindow) -> Dialog {
    wait_until("a message over Properties", || {
        dialog_over_properties(window).is_some()
    });
    dialog_over_properties(window).expect("a message shows")
}

/// The message box over `window` that is not the extension question,
/// once it shows.
fn refusal_over(window: &PropertiesWindow) -> Dialog {
    let find = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|toplevel| toplevel.downcast::<Dialog>().ok())
            .find(|dialog| {
                dialog.is_visible()
                    && dialog.transient_for().as_ref() == Some(window.upcast_ref())
                    && dialog.title_text() != "Change the extension?"
            })
    };
    wait_until("a message over Properties", || find().is_some());
    find().expect("a message shows")
}

/// The Properties dialog (Edit..., Advanced, Advanced Sharing) over
/// `window`, titled `title`.
fn sub_dialog(window: &PropertiesWindow, title: &str) -> PropertiesWindow {
    let find = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|toplevel| toplevel.downcast::<PropertiesWindow>().ok())
            .find(|dialog| {
                dialog.is_visible()
                    && dialog.transient_for().as_ref() == Some(window.upcast_ref())
                    && dialog.title_text() == title
            })
    };
    wait_until(title, || find().is_some());
    find().expect("the dialog is open")
}

/// The mode bits of `path`.
fn mode_of(path: &std::path::Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o7777
}

/// The Size on disk text of `files`: the blocks they take, as Properties
/// shows it.
fn on_disk_text(files: &[std::path::PathBuf]) -> String {
    use std::os::unix::fs::MetadataExt;
    let bytes: u64 = files
        .iter()
        .map(|file| fs::metadata(file).expect("a fixture file").blocks() * 512)
        .sum();
    ox_core::format::exact_size_text(bytes)
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
/// in a new tab with the item selected, and its Properties.
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

    test.wait_for_properties("report.txt Properties");
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents listing");
    wait_until("the selection", || test.selected_names() == ["report.txt"]);
}

/// The caller's activation token goes to the Properties window that
/// `ShowItemProperties` opens or brings back, so the desktop lets it come
/// to the front.
///
/// parity: INT-014, INT-023
#[gtk::test]
fn show_item_properties_gives_the_callers_token_to_the_properties_window() {
    let fixture = Fixture::standard();
    fixture.write("Documents/report.txt");
    let test = TestWindow::open(&fixture.uri());
    let item = [fixture.uri_of("Documents/report.txt")];
    let request =
        FileManagerRequest::new(FileManagerMethod::ShowItemProperties, &item).expect("a valid location");

    test.window.hand_startup_id_to_properties("caller-token-1");
    test.window.show_file_manager_request(&request);

    test.wait_for_properties("report.txt Properties");
    assert_eq!(
        test.window.properties_startup_ids(),
        ["caller-token-1"],
        "the Properties window took the caller's token"
    );

    // Asked again, the open window comes back with the new caller's token.
    test.window.hand_startup_id_to_properties("caller-token-2");
    test.window.show_file_manager_request(&request);
    wait_until("the second token", || {
        test.window.properties_startup_ids() == ["caller-token-1", "caller-token-2"]
    });
    assert_eq!(test.window.properties_windows().len(), 1, "the same window");
}

/// Alt+Enter opens a file's Properties as Windows 11 lays them out: a
/// window of its own with General, Security and Previous Versions, OK,
/// Cancel and a greyed Apply, every byte of the size and the long dates.
///
/// parity: PROP-001, PROP-003, PROP-006, PROP-033
#[gtk::test]
fn alt_enter_opens_a_files_properties_in_a_window_of_its_own() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");

    test.activate("properties", None);

    let window = test.wait_for_properties("Notes 2.txt Properties");
    assert!(!window.is_modal(), "Properties blocks nothing");
    assert_eq!(window.transient_for(), None, "a window of its own");
    assert_eq!(
        window.tab_labels(),
        ["General", "Security", "Details", "Previous Versions"]
    );
    assert_eq!(window.button_labels(), ["OK", "Cancel", "Apply"]);
    assert!(!window.apply_button().is_sensitive(), "nothing to apply yet");
    let general = general(&window);
    assert_eq!(
        value_after(&general, "Type of file:").as_deref(),
        Some("Text document (.txt)")
    );
    assert!(value_after(&general, "Opens with:").is_some());
    assert!(button(&general, "Change...").is_visible());
    assert_eq!(
        value_after(&general, "Size:").as_deref(),
        Some("20 bytes (20 bytes)")
    );
    assert_eq!(
        value_after(&general, "Size on disk:"),
        Some(on_disk_text(&[fixture.path("Notes 2.txt")])),
        "the blocks the file takes, as Windows Explorer's Size on disk"
    );
    let modified = value_after(&general, "Modified:").expect("a Modified row");
    assert!(modified.starts_with("Today, "), "{modified}");
    assert!(value_after(&general, "Accessed:").is_some());
    assert!(value_after(&general, "Created:").is_some());
    assert_eq!(value_after(&general, "Full path:"), None, "no Full path row");
    assert!(!check(&general, "Read-only").is_active());
    assert!(!check(&general, "Hidden").is_active());
    let shown = texts(&general);
    for removed in [
        "Copy full path",
        "Calculate folder size",
        "Change icon…",
        "Advanced...",
    ] {
        assert!(!shown.iter().any(|text| text == removed), "{removed} is gone");
    }
    assert_eq!(
        descendants::<gtk::Separator>(&general).len(),
        4,
        "divider lines between the groups, as Windows draws them"
    );
    capture_dialog(&window, "native-properties-general.png");
}

/// A folder's General tab counts Size, Size on disk and Contains by
/// itself, shows Created only, and its Read-only box starts half ticked.
///
/// parity: PROP-001, PROP-004, PROP-027
#[gtk::test]
fn a_folders_size_and_contents_count_by_themselves() {
    let fixture = Fixture::standard();
    fs::create_dir_all(fixture.path("Projects/Plans")).expect("folders");
    fs::write(fixture.path("Projects/a.txt"), b"0123456789").expect("a file");
    fs::write(fixture.path("Projects/Plans/b.txt"), b"01234").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Projects");

    test.activate("properties", None);

    let window = test.wait_for_properties("Projects Properties");
    assert_eq!(
        window.tab_labels(),
        ["General", "Sharing", "Security", "Previous Versions", "Customize"]
    );
    let general = general(&window);
    wait_until("the count", || {
        value_after(&general, "Contains:").as_deref() == Some("2 Files, 1 Folders")
    });
    assert_eq!(value_after(&general, "Type:").as_deref(), Some("File folder"));
    assert_eq!(
        value_after(&general, "Size:").as_deref(),
        Some("15 bytes (15 bytes)")
    );
    assert!(value_after(&general, "Size on disk:").is_some_and(|text| text.ends_with(" bytes)")));
    assert!(value_after(&general, "Created:").is_some());
    assert_eq!(
        value_after(&general, "Modified:"),
        None,
        "a folder shows Created only"
    );
    assert_eq!(value_after(&general, "Accessed:"), None);
    let read_only = check(&general, "Read-only (Only applies to files in folder)");
    assert!(read_only.is_inconsistent(), "half ticked, as Windows starts it");
    assert!(
        test.window
            .measured_folder_size(&fixture.uri_of("Projects"))
            .is_none(),
        "the count is the window's own, not a Calculate folder size run"
    );
    capture_dialog(&window, "native-properties-folder.png");
}

/// A count that left something out reads as at least its total, and a
/// folder that cannot be read reads Unavailable, never a false zero.
///
/// parity: PROP-027, PROP-028
#[gtk::test]
fn a_partial_or_failed_folder_count_is_never_shown_as_exact() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Linked")).expect("a folder");
    fs::write(fixture.path("Linked/a.txt"), b"0123456789").expect("a file");
    std::os::unix::fs::symlink(fixture.path("Notes 2.txt"), fixture.path("Linked/link.txt")).expect("a link");
    fs::create_dir(fixture.path("Locked")).expect("a folder");
    fs::write(fixture.path("Locked/secret.txt"), b"secret").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Linked");
    test.activate("properties", None);
    let window = test.wait_for_properties("Linked Properties");
    let page = general(&window);

    wait_until("the partial count", || {
        value_after(&page, "Contains:").is_some_and(|text| text.starts_with("≥ "))
    });
    assert_eq!(
        value_after(&page, "Size:").as_deref(),
        Some("≥ 10 bytes (10 bytes)")
    );
    window.close();

    // The superuser reads every folder, so only another user sees this.
    if ox_core::permissions::current_user_is_superuser() {
        return;
    }
    fs::set_permissions(fixture.path("Locked"), fs::Permissions::from_mode(0o000)).expect("mode");
    test.select_named("Locked");
    test.activate("properties", None);
    let window = test.wait_for_properties("Locked Properties");
    let page = general(&window);
    wait_until("the failed count", || {
        value_after(&page, "Contains:").as_deref() == Some("Unavailable")
    });
    let shown = (value_after(&page, "Size:"), value_after(&page, "Size on disk:"));
    fs::set_permissions(fixture.path("Locked"), fs::Permissions::from_mode(0o755)).expect("mode");
    assert_eq!(
        shown,
        (Some("Unavailable".to_owned()), Some("Unavailable".to_owned()))
    );
}

/// Edit... after Apply on General changes the item as it is now: Read-only
/// stays, and after Hidden renamed the file Edit changes the file under its
/// new name, never an unrelated file that took the old one.
///
/// parity: PROP-007
#[gtk::test]
fn edit_after_general_changes_the_item_as_it_is_now() {
    let fixture = Fixture::standard();
    let path = fixture.path("Notes 2.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).expect("a known mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let general_page = general(&window);

    check(&general_page, "Read-only").set_active(true);
    check(&general_page, "Hidden").set_active(true);
    press(&window, "Apply");
    let hidden = fixture.path(".Notes 2.txt");
    wait_until("the hidden name", || hidden.exists());
    wait_until("Read-only", || mode_of(&hidden) == 0o444);
    // Another file takes the old name.
    fs::write(&path, b"unrelated").expect("a file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("a known mode");
    window.select_tab("security");
    let security = tab(&window, "security");
    wait_until("the Security tab read again", || {
        texts(&security).iter().any(|text| text.ends_with(".Notes 2.txt"))
    });

    press(&security, "Edit...");
    let edit = sub_dialog(&window, "Permissions for .Notes 2.txt");
    let names = descendants::<gtk::ListBox>(&edit).remove(0);
    names.select_row(names.row_at_index(2).as_ref());
    let read = descendants::<gtk::CheckButton>(&edit)
        .into_iter()
        .filter(WidgetExt::is_sensitive)
        .nth(3)
        .expect("Allow Read");
    assert!(read.is_active(), "others may read");
    read.set_active(false);
    edit.ok_button().emit_clicked();

    wait_until("the change", || mode_of(&hidden) == 0o440);
    assert_eq!(mode_of(&path), 0o600, "the unrelated file is untouched");
}

/// Tabs taller than the screen, as at a large text size, scroll inside
/// the window, which stays within the screen's height, so OK, Cancel and
/// Apply stay on the screen; shorter ones show whole.
///
/// parity: PROP-001
#[gtk::test]
fn tabs_taller_than_the_screen_scroll_inside_the_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let filled = test.wait_for_properties("Notes 2.txt Properties");
    general(&filled);
    assert!(
        filled.scroller().parent().is_none(),
        "tabs that fit do not scroll"
    );
    filled.close();

    let window = PropertiesWindow::new("Tall Properties", None, Some(&test.window));
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for line in 0..60 {
        page.append(&gtk::Label::new(Some(&format!("Line {line}"))));
    }
    window.add_tab("general", "General", &page);

    // As if the screen were 300 high: after the window found the real
    // one's height as it is realized.
    window.connect_realize(|window| window.fit_to_height(300));
    window.present();
    wait_until("the window to fit", || {
        window.height() > 0 && window.height() <= 300
    });

    let scrolling = window.scroller().vadjustment();
    wait_until("the tabs to scroll", || scrolling.upper() > scrolling.page_size());
    assert!(window.ok_button().is_drawable(), "OK stays in the window");
    window.close();
}

/// Read-only on General follows a change Edit... made: taking the
/// owner's Write ticks it, and clearing it then gives Write back.
///
/// parity: PROP-003, PROP-007
#[gtk::test]
fn read_only_follows_a_change_made_in_edit() {
    let fixture = Fixture::standard();
    let path = fixture.path("Notes 2.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("a known mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let read_only = check(&general(&window), "Read-only");
    assert!(!read_only.is_active());

    window.select_tab("security");
    let security = tab(&window, "security");
    press(&security, "Edit...");
    let edit = sub_dialog(&window, "Permissions for Notes 2.txt");
    let names = descendants::<gtk::ListBox>(&edit).remove(0);
    names.select_row(names.row_at_index(0).as_ref());
    let write = descendants::<gtk::CheckButton>(&edit)
        .into_iter()
        .filter(WidgetExt::is_sensitive)
        .nth(4)
        .expect("Allow Write");
    assert!(write.is_active(), "the owner may write");
    write.set_active(false);
    edit.ok_button().emit_clicked();
    wait_until("the change", || mode_of(&path) == 0o444);

    wait_until("Read-only to follow", || read_only.is_active());
    assert!(!window.apply_button().is_sensitive(), "nothing is left to apply");
    read_only.set_active(false);
    assert!(window.apply_button().is_sensitive(), "clearing it is a change");
    press(&window, "Apply");
    wait_until("Write back", || mode_of(&path) & 0o200 != 0);
}

/// A rename on General reaches every tab: asking again for the renamed
/// item brings the same window back, and the other tabs work on the new
/// name.
///
/// parity: PROP-005
#[gtk::test]
fn a_rename_reaches_every_tab() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Album")).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Album");
    test.activate("properties", None);
    let window = test.wait_for_properties("Album Properties");

    name_field(&window).set_text("Papers");
    press(&window, "Apply");
    wait_until("the rename", || fixture.path("Papers").exists());
    wait_until("the new title", || window.title_text() == "Papers Properties");

    assert_eq!(versions_of(&window).item_uri(), fixture.uri_of("Papers"));
    test.wait_for_listing("the listing again");
    test.select_named("Papers");
    test.activate("properties", None);
    settle();
    assert_eq!(
        test.window.properties_windows(),
        std::slice::from_ref(&window),
        "the renamed item's Properties come back"
    );
}

/// Closing Properties while OK applies waits for the change: a refused
/// change keeps the window open with its message, and a made one closes it.
///
/// parity: PROP-001
#[gtk::test]
fn closing_during_ok_waits_for_the_change() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let name = name_field(&window);

    name.set_text("Notes 2.md");
    window.ok_button().emit_clicked();
    wait_until("the question", || extension_questions(&window) == 1);
    let question = message_over(&window);
    window.close();
    assert!(!window.is_closed(), "the window waits for the change");
    question.press("Cancel");
    refusal_over(&window).press("OK");
    assert!(!window.is_closed(), "a refused change keeps the window");

    wait_until("no message", || dialog_over_properties(&window).is_none());
    window.ok_button().emit_clicked();
    wait_until("the question again", || extension_questions(&window) == 1);
    let question = message_over(&window);
    window.close();
    question.press("Change extension");
    wait_until("the rename", || fixture.path("Notes 2.md").exists());
    wait_until("the window to close", || window.is_closed());
    assert!(
        dialog_over_properties(&window).is_none(),
        "no message over a closed window"
    );
}

/// The browser window's shortcuts do nothing in Properties, and F5 looks
/// for previous versions again there.
///
/// parity: PROP-001, PROP-019
#[gtk::test]
fn properties_take_no_browser_shortcut_and_f5_looks_again() {
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");
    test.activate("previous-versions", None);
    let window = test.wait_for_properties("Documents Properties");
    let versions = versions_of(&window);
    wait_until("the versions", || !versions.version_labels().is_empty());
    let first = versions.lookup_number();

    for action in [
        "win.refresh",
        "win.search",
        "win.location",
        "win.full-screen",
        "win.new-tab",
    ] {
        assert!(
            WidgetExt::activate_action(&window, action, None).is_err(),
            "{action} does not reach the browser window from Properties"
        );
    }
    let f5 = window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            controller
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .collect::<Vec<_>>()
        })
        .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
        .find(|shortcut| {
            shortcut
                .trigger()
                .is_some_and(|trigger| trigger.to_str().as_str() == "F5")
        })
        .expect("Properties has its own F5");
    f5.action()
        .expect("an action")
        .activate(gtk::ShortcutActionFlags::empty(), &window, None);

    assert!(versions.lookup_number() > first, "F5 looked again");
}

/// Moving the only tab to a new window keeps its window's Properties open,
/// with the change they hold, acting through the tab's new window.
///
/// parity: PROP-001, TAB-029
#[gtk::test]
fn properties_go_on_with_the_last_tab_moved_away() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    name_field(&window).set_text("Renamed.txt");
    let tab = test.active_tab().expect("a tab");

    let action = format!("win.{}", WindowAction::MoveTabToNewWindow.name());
    WidgetExt::activate_action(&test.window, &action, Some(&tab.to_raw().to_variant()))
        .expect("the action exists");

    wait_until("the old window to close", || !test.window.is_visible());
    assert!(!window.is_closed(), "Properties stays open");
    let opener = window.opener().expect("a new window took over");
    assert_ne!(opener, test.window);
    assert!(opener.properties_windows().contains(&window));
    window.ok_button().emit_clicked();
    wait_until("the rename", || fixture.path("Renamed.txt").exists());
    opener.close();
}

/// Properties are windows of their own: several stay open, the browser
/// window keeps its keys, the same item brings its window back, and they
/// close with the browser window.
///
/// parity: PROP-001
#[gtk::test]
fn properties_are_windows_of_their_own_that_block_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let file = test.wait_for_properties("Notes 2.txt Properties");
    test.select_named("Documents");
    test.activate("properties", None);
    let folder = test.wait_for_properties("Documents Properties");

    test.window.folder_pane().focus_view();
    assert!(test.window.file_keys_apply(), "the browser window keeps its keys");
    assert!(!test.window.dialog_in_front());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    assert_eq!(
        test.window.properties_windows().len(),
        2,
        "the same item brings its window back"
    );
    test.activate("new-tab", None);
    assert!(
        !file.is_closed() && !folder.is_closed(),
        "a tab switch hides nothing"
    );
    assert!(!tab_tooltips(&test)[0].contains("Properties"));

    test.window.close();

    assert!(
        file.is_closed() && folder.is_closed(),
        "they close with their window"
    );
}

/// Cancel and Escape close without applying; OK applies and closes.
///
/// parity: PROP-003
#[gtk::test]
fn read_only_waits_for_ok_and_cancel_discards_it() {
    let fixture = Fixture::standard();
    let path = fixture.path("Notes 2.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).expect("a known mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let read_only = check(&general(&window), "Read-only");

    read_only.set_active(true);
    assert!(window.apply_button().is_sensitive(), "a change turns Apply on");
    read_only.set_active(false);
    assert!(
        !window.apply_button().is_sensitive(),
        "undoing it greys Apply again"
    );
    read_only.set_active(true);
    press(&window, "Cancel");
    assert!(window.is_closed());
    assert_eq!(mode_of(&path), 0o664, "Cancel discards the change");

    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    check(&general(&window), "Read-only").set_active(true);
    window.ok_button().emit_clicked();
    wait_until("OK to close", || window.is_closed());
    assert_eq!(mode_of(&path), 0o444, "every write permission is taken away");
}

/// A folder's Read-only cycles through unchanged, ticked and cleared, and
/// reaches only the files inside it.
///
/// parity: PROP-003
#[gtk::test]
fn a_folders_read_only_reaches_the_files_inside_only() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Album")).expect("a folder");
    fs::write(fixture.path("Album/photo.png"), b"p").expect("a file");
    fs::set_permissions(fixture.path("Album/photo.png"), fs::Permissions::from_mode(0o644)).expect("mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Album");
    test.activate("properties", None);
    let window = test.wait_for_properties("Album Properties");
    let read_only = check(&general(&window), "Read-only (Only applies to files in folder)");

    read_only.set_active(true);
    assert!(!read_only.is_inconsistent() && read_only.is_active(), "ticked");
    assert!(window.apply_button().is_sensitive());
    press(&window, "Apply");

    wait_until("the files inside", || {
        mode_of(&fixture.path("Album/photo.png")) == 0o444
    });
    assert_eq!(
        mode_of(&fixture.path("Album")) & 0o200,
        0o200,
        "the folder stays writable"
    );
    wait_until("Apply to grey", || !window.apply_button().is_sensitive());
    assert!(!window.is_closed(), "Apply keeps the window open");
}

/// A folder's Read-only changed while Apply makes the change before it
/// stays to apply.
///
/// parity: PROP-003
#[gtk::test]
fn a_folders_read_only_changed_during_apply_stays_to_apply() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Album")).expect("a folder");
    fs::write(fixture.path("Album/photo.png"), b"p").expect("a file");
    fs::set_permissions(fixture.path("Album/photo.png"), fs::Permissions::from_mode(0o644)).expect("mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Album");
    test.activate("properties", None);
    let window = test.wait_for_properties("Album Properties");
    let read_only = check(&general(&window), "Read-only (Only applies to files in folder)");

    read_only.set_active(true);
    press(&window, "Apply");
    // Apply greys the buttons as it starts the change, which then runs off
    // the main thread until the main loop takes its result.
    while window.ok_button().is_sensitive() {
        glib::MainContext::default().iteration(true);
    }
    read_only.set_active(false);
    assert!(read_only.is_inconsistent() || !read_only.is_active(), "cleared");

    wait_until("the change", || window.ok_button().is_sensitive());
    assert_eq!(
        mode_of(&fixture.path("Album/photo.png")),
        0o444,
        "Read-only was made"
    );
    assert!(
        window.apply_button().is_sensitive(),
        "clearing it is left to apply"
    );
}

/// The name and Hidden wait while Apply renames the item, so an edit is
/// never lost behind the rename, and come back once it is made.
///
/// parity: PROP-005
#[gtk::test]
fn the_name_waits_while_apply_renames() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let name = name_field(&window);
    let hidden = check(&general(&window), "Hidden");

    name.set_text("Notes 3.txt");
    press(&window, "Apply");
    while window.ok_button().is_sensitive() {
        glib::MainContext::default().iteration(true);
    }
    let waiting = (!name.is_editable(), !hidden.is_sensitive());

    wait_until("the rename", || window.ok_button().is_sensitive());
    assert_eq!(waiting, (true, true), "the name and Hidden wait");
    assert!(fixture.path("Notes 3.txt").exists());
    assert!(name.is_editable() && hidden.is_sensitive(), "they come back");
    assert_eq!(name.text(), "Notes 3.txt");
}

/// Read-only is refused on a location registered as a previous version
/// after the window opened, as Security and renaming refuse it.
///
/// parity: PROP-024
#[gtk::test]
fn read_only_refuses_a_location_made_a_previous_version_meanwhile() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Backups")).expect("a backup folder");
    let path = fixture.path("Backups/plan.txt");
    fs::write(&path, b"plan").expect("a file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("a known mode");
    let test = TestWindow::open(&fixture.uri_of("Backups"));
    test.select_named("plan.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("plan.txt Properties");
    let read_only = check(&general(&window), "Read-only");

    test.context
        .previous_versions()
        .configure(
            &fixture.uri_of("Documents"),
            &ox_core::location::file_uri(&fixture.path("Backups")),
            ox_core::versions::SnapshotLayout::Direct,
        )
        .expect("a snapshot source");
    read_only.set_active(true);
    press(&window, "Apply");

    refusal_over(&window).press("OK");
    assert_eq!(mode_of(&path), 0o644, "the previous version is unchanged");
}

/// Hidden adds a leading dot to the name on Apply, and a taken name is
/// refused with the normal message.
///
/// parity: PROP-005
#[gtk::test]
fn hidden_renames_with_a_leading_dot() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Taken.txt"), b"keep").expect("a file");
    fs::write(fixture.path(".Taken.txt"), b"hidden").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");

    check(&general(&window), "Hidden").set_active(true);
    press(&window, "Apply");

    wait_until("the hidden name", || fixture.path(".Notes 2.txt").exists());
    assert!(!fixture.path("Notes 2.txt").exists());
    wait_until("the new title", || {
        window.title_text() == ".Notes 2.txt Properties"
    });
    assert_eq!(name_field(&window).text(), ".Notes 2.txt");

    test.select_named("Taken.txt");
    test.activate("properties", None);
    let taken = test.wait_for_properties("Taken.txt Properties");
    check(&general(&taken), "Hidden").set_active(true);
    taken.ok_button().emit_clicked();
    let refusal = message_over(&taken);
    assert!(!refusal.message_text().is_empty());
    refusal.press("OK");
    assert!(!taken.is_closed(), "a refused change keeps the window open");
    assert_eq!(fs::read(fixture.path(".Taken.txt")).expect("kept"), b"hidden");
    assert_eq!(fs::read(fixture.path("Taken.txt")).expect("kept"), b"keep");
}

/// The name renames the item on OK, refusing a taken name in a message
/// over the window, which stays open; Enter in the name presses OK.
///
/// parity: PROP-005
#[gtk::test]
fn the_name_in_properties_renames_the_item() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Taken.txt"), b"keep").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let name = name_field(&window);
    assert_eq!(name.text(), "Notes 2.txt");

    name.set_text("Taken.txt");
    assert!(window.apply_button().is_sensitive());
    window.ok_button().emit_clicked();
    message_over(&window).press("OK");
    assert_eq!(fs::read(fixture.path("Taken.txt")).expect("kept"), b"keep");
    assert!(!window.is_closed());
    name.set_text("Renamed.txt");
    assert!(name.activates_default(), "Enter presses OK");
    window.ok_button().emit_clicked();

    wait_until("the rename", || fixture.path("Renamed.txt").exists());
    assert!(!fixture.path("Notes 2.txt").exists());
    wait_until("the window to close", || window.is_closed());
}

/// An invalid name is refused before anything is asked, as renaming in
/// place and the Rename dialog refuse it.
///
/// parity: PROP-005, OPS-049
#[gtk::test]
fn an_invalid_name_in_properties_is_refused_before_the_extension_question() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");

    name_field(&window).set_text("");
    press(&window, "Apply");

    let refusal = message_over(&window);
    assert_eq!(
        refusal.message_text(),
        "Enter a non-empty file name, not “.” or “..”."
    );
    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "no extension question"
    );
    refusal.press("OK");
    assert!(fixture.path("Notes 2.txt").is_file());
}

/// The name asks before it changes a file's extension, as Windows
/// Explorer does, and renames only once the user confirms.
///
/// parity: PROP-005, OPS-049
#[gtk::test]
fn the_name_in_properties_asks_before_changing_the_extension() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let name = name_field(&window);

    name.set_text("Notes 2.md");
    press(&window, "Apply");
    let question = message_over(&window);
    assert_eq!(
        question.title_text(),
        "Change the extension?",
        "the question is over Properties"
    );
    question.press("Cancel");
    let refusal = refusal_over(&window);
    assert_eq!(refusal.message_text(), "The name was not changed.");
    refusal.press("OK");
    assert!(fixture.path("Notes 2.txt").is_file());

    press(&window, "Apply");
    wait_until("the question again", || extension_questions(&window) == 1);
    message_over(&window).press("Change extension");

    wait_until("the rename", || fixture.path("Notes 2.md").exists());
    assert!(!window.is_closed(), "Apply keeps the window open");
}

/// The extension questions open over the Properties window `window`.
fn extension_questions(window: &PropertiesWindow) -> usize {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|toplevel| toplevel.downcast::<Dialog>().ok())
        .filter(|dialog| {
            dialog.is_visible()
                && dialog.transient_for().as_ref() == Some(window.upcast_ref())
                && dialog.title_text() == "Change the extension?"
        })
        .count()
}

/// Pressing OK twice quickly asks about the extension once, not twice.
///
/// parity: PROP-005, OPS-049
#[gtk::test]
fn ok_twice_in_properties_asks_about_the_extension_once() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");

    name_field(&window).set_text("Notes 2.md");
    window.ok_button().emit_clicked();
    window.ok_button().emit_clicked();

    wait_until("the question", || extension_questions(&window) > 0);
    wait_for_frames(&test.window, 3);
    assert_eq!(extension_questions(&window), 1, "one question");
    message_over(&window).press("Cancel");
    wait_until("no question", || extension_questions(&window) == 0);
    refusal_over(&window).press("OK");
    assert!(fixture.path("Notes 2.txt").is_file());
}

/// A link to a folder is renamed without the extension question, as
/// renaming it in place does.
///
/// parity: PROP-005, OPS-049
#[gtk::test]
fn a_link_to_a_folder_is_renamed_in_properties_without_the_extension_question() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Target")).expect("the folder is made");
    std::os::unix::fs::symlink(fixture.path("Target"), fixture.path("Docs.old")).expect("the link is made");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Docs.old");
    test.activate("properties", None);
    let window = test.wait_for_properties("Docs.old Properties");

    name_field(&window).set_text("Docs.new");
    window.ok_button().emit_clicked();

    wait_until("the rename", || {
        fixture.path("Docs.new").is_symlink() || extension_questions(&window) > 0
    });
    assert_eq!(extension_questions(&window), 0, "no extension question");
    wait_until("the window to close", || window.is_closed());
}

/// The Security tab names the owner, the group and others, shows what the
/// one selected may do in Windows' words, and Edit... and Advanced change
/// the Linux permissions behind them.
///
/// parity: PROP-006, PROP-007
#[gtk::test]
fn the_security_tab_reads_and_changes_the_linux_permissions() {
    let fixture = Fixture::standard();
    let path = fixture.path("Notes 2.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("a known mode");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    window.select_tab("security");
    let security = tab(&window, "security");
    wait_until("the names", || {
        texts(&security).iter().any(|text| text == "Group or user names:")
    });
    let user = glib::user_name().to_string_lossy().into_owned();
    let shown = texts(&security);
    assert!(shown.contains(&format!("{user} (Owner)")), "{shown:?}");
    assert!(shown.contains(&"Everyone else (Others)".to_owned()));
    assert!(shown.contains(&format!("Permissions for {user} (Owner)")));
    for heading in ["Allow", "Deny", "To change permissions, click Edit."] {
        assert!(shown.iter().any(|text| text == heading), "{heading}");
    }
    let allowed = descendants::<gtk::Image>(&security)
        .into_iter()
        .filter(|image| image.has_css_class("properties-allowed") && image.opacity() > 0.5)
        .count();
    assert_eq!(allowed, 3, "Modify, Read and Write for the owner of a 644 file");
    capture_dialog(&window, "native-properties-security.png");

    let first_edit = button(&security, "Edit...");
    press(&security, "Edit...");
    let edit = sub_dialog(&window, "Permissions for Notes 2.txt");
    let names = descendants::<gtk::ListBox>(&edit).remove(0);
    names.select_row(names.row_at_index(1).as_ref());
    let full_control = descendants::<gtk::CheckButton>(&edit)
        .into_iter()
        .find(WidgetExt::is_sensitive)
        .expect("Allow Full control");
    full_control.set_active(true);
    assert!(edit.apply_button().is_sensitive());
    edit.ok_button().emit_clicked();
    wait_until("the group's full control", || mode_of(&path) == 0o674);
    wait_until("Edit to close", || edit.is_closed());

    let security = tab(&window, "security");
    wait_until("the tab read again", || {
        button(&security, "Edit...") != first_edit
    });
    press(&security, "Advanced");
    let advanced = sub_dialog(&window, "Advanced Security Settings for Notes 2.txt");
    assert!(texts(&advanced).iter().any(|text| text == "Permission entries:"));
    check(&advanced, "Set group ID").set_active(true);
    advanced.ok_button().emit_clicked();
    wait_until("the special bit", || mode_of(&path) == 0o2674);
}

/// With several items selected, Size on disk counts a link to a file as
/// the file it points to, as Size does.
///
/// parity: PROP-033
#[gtk::test]
fn size_on_disk_of_several_items_follows_links_as_size_does() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Big.bin"), vec![7_u8; 200_000]).expect("the file is written");
    std::os::unix::fs::symlink(fixture.path("Big.bin"), fixture.path("Big link.bin"))
        .expect("the link is made");
    let test = TestWindow::open(&fixture.uri());
    super::file_ops_support::select_names(&test, &["Big.bin", "Big link.bin"]);
    test.activate("properties", None);
    let window = test.wait_for_properties("Big link.bin, Big.bin Properties");
    let both = [fixture.path("Big.bin"), fixture.path("Big link.bin")];

    wait_until("the space on disk", || {
        value_after(&window, "Size on disk:") == Some(on_disk_text(&both))
    });
    assert_eq!(
        value_after(&window, "Size:").as_deref(),
        Some(ox_core::format::exact_size_text(400_000).as_str())
    );
}

/// Properties of several items total them, folders' content included,
/// and a permission change on the Security tab reaches every one while
/// every bit the user did not change stays as each item has it.
///
/// parity: PROP-002
#[gtk::test]
fn properties_of_several_items_total_them_and_change_them_together() {
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
    let window = test.wait_for_properties("Documents, Notes 2.txt, ... Properties");

    assert_eq!(window.tab_labels(), ["General", "Security"]);
    let read_only = check(&window, "Read-only (Only applies to files in folder)");
    wait_until("the items read", || read_only.is_sensitive());
    assert!(
        read_only.is_inconsistent(),
        "a folder's files keep what they have"
    );
    wait_until("the folders measured", || {
        value_after(&window, "Contains:").as_deref() == Some("3 Files, 1 Folders")
    });
    assert_eq!(
        value_after(&window, "Size:").as_deref(),
        Some("60 bytes (60 bytes)")
    );
    let files = ["Documents/inside.txt", "Notes 2.txt", "Notes 10.txt"].map(|name| fixture.path(name));
    assert_eq!(value_after(&window, "Size on disk:"), Some(on_disk_text(&files)));
    assert_eq!(
        value_after(&window, "Location:").as_deref(),
        Some(format!("All in {}", fixture.path("").display()).trim_end_matches('/'))
    );
    let security = tab(&window, "security");
    wait_until("the editor", || {
        !descendants::<gtk::DropDown>(&security).is_empty()
    });
    let choices = descendants::<gtk::DropDown>(&security);
    assert_eq!(choices[0].selected(), 2, "every owner may view and modify");
    assert_eq!(choices[2].selected(), 3, "Varying (No Change)");
    let apply = button(&security, "Apply permissions");
    assert!(!apply.is_sensitive(), "nothing to apply before a change");

    choices[2].set_selected(0);
    assert!(apply.is_sensitive());
    assert!(
        window.apply_button().is_sensitive(),
        "the window's Apply knows the change"
    );
    window.ok_button().emit_clicked();
    wait_until("OK to close", || window.is_closed());

    wait_until("every item changed", || {
        mode_of(&fixture.path("Notes 2.txt")) == 0o640
    });
    assert_eq!(
        mode_of(&fixture.path("Notes 10.txt")),
        0o700,
        "the private program stays private and runnable"
    );
    assert_eq!(mode_of(&fixture.path("Documents")), 0o750);
}

/// Several items' Read-only and Hidden are half ticked where the items
/// differ, and OK or Apply gives every item the state chosen: Read-only
/// takes write from every file, and Hidden adds a leading dot to each
/// name, as one item's do, and the title follows the new names.
///
/// parity: PROP-002
#[gtk::test]
fn several_items_attributes_show_mixed_and_apply_to_every_item() {
    let fixture = Fixture::standard();
    fs::set_permissions(fixture.path("Notes 2.txt"), fs::Permissions::from_mode(0o444))
        .expect("a read-only file");
    fs::set_permissions(fixture.path("Notes 10.txt"), fs::Permissions::from_mode(0o644))
        .expect("a writable file");
    let test = TestWindow::open(&fixture.uri());
    super::file_ops_support::select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt, Notes 10.txt Properties");
    let general = tab(&window, "general");
    assert_eq!(
        value_after(&general, "Type:").as_deref(),
        Some("All of type Text document")
    );
    let (read_only, hidden) = (check(&general, "Read-only"), check(&general, "Hidden"));
    wait_until("the items read", || {
        read_only.is_sensitive() && hidden.is_sensitive()
    });
    assert!(read_only.is_inconsistent(), "one file is read-only, one is not");
    assert!(
        !hidden.is_inconsistent() && !hidden.is_active(),
        "neither is hidden"
    );
    assert!(!window.apply_button().is_sensitive());

    read_only.set_active(!read_only.is_active());
    assert!(
        read_only.is_active() && !read_only.is_inconsistent(),
        "half ticked, then ticked"
    );
    hidden.set_active(true);
    press(&window, "Apply");

    wait_until("every name hidden", || {
        fixture.path(".Notes 2.txt").exists() && fixture.path(".Notes 10.txt").exists()
    });
    wait_until("the new title", || {
        window.title_text() == ".Notes 2.txt, .Notes 10.txt Properties"
    });
    for name in [".Notes 2.txt", ".Notes 10.txt"] {
        assert_eq!(mode_of(&fixture.path(name)) & 0o222, 0, "{name} is read-only");
    }
    assert!(!fixture.path("Notes 2.txt").exists() && !fixture.path("Notes 10.txt").exists());
    assert!(hidden.is_active() && read_only.is_active());
    assert!(!window.apply_button().is_sensitive(), "nothing is left to apply");
    window.close();
}

/// A link says where it points. A drive, the root among them, has
/// Windows' drive General tab: its type, file system, used and free
/// space and capacity in bytes and short, and the ring, with nothing on
/// it counted; where it is mounted and from what are on its Hardware tab.
///
/// parity: PROP-004
#[gtk::test]
fn properties_show_a_links_target_and_a_drives_space() {
    let fixture = Fixture::standard();
    std::os::unix::fs::symlink("Documents", fixture.path("Shortcut")).expect("a link");
    let test = TestWindow::open(&fixture.uri());

    test.activate("properties-of", Some(&fixture.uri_of("Shortcut")));
    let window = test.wait_for_properties("Shortcut Properties");
    let general = general(&window);
    assert_eq!(value_after(&general, "Target:").as_deref(), Some("Documents"));
    window.close();

    test.activate("properties-of", Some("file:///"));
    let window = test.wait_for_properties("Local Disk Properties");
    let tabs = window.tab_labels();
    assert_eq!(
        tabs.get(..2),
        Some(&["General".to_owned(), "Hardware".to_owned()][..]),
        "{tabs:?}"
    );
    let general = tab(&window, "general");
    wait_until("the drive's details", || value_after(&general, "Type:").is_some());
    assert_eq!(value_after(&general, "Type:").as_deref(), Some("Local Disk"));
    assert!(value_after(&general, "File system:").is_some_and(|name| !name.is_empty()));
    for row in ["Used space:", "Free space:", "Capacity:"] {
        assert!(
            value_after(&general, row).is_some_and(|text| text.ends_with(" bytes")),
            "{row} in bytes"
        );
    }
    for row in ["Location:", "Size:", "Contains:", "Mounted on:"] {
        assert_eq!(value_after(&general, row), None, "no {row} for a drive");
    }
    assert!(
        texts(&general).contains(&"Local Disk".to_owned()),
        "the name under the ring"
    );
    assert_eq!(
        descendants::<crate::properties::SpaceRing>(&general).len(),
        1,
        "the ring"
    );
    let hardware = tab(&window, "hardware");
    assert_eq!(value_after(&hardware, "Mounted on:").as_deref(), Some("/"));
    assert!(value_after(&hardware, "Mounted from:").is_some());
    capture_dialog(&window, "native-properties-drive.png");
}

/// Change... opens Open with over the Properties window, as its own
/// dialog transient for it, also when the browser window that opened
/// Properties is behind another window; Properties stays open, and Cancel
/// leaves its Opens with row as it was.
///
/// parity: PROP-003
#[gtk::test]
fn change_opens_open_with_over_the_properties_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let general = general(&window);
    let app_before = value_after(&general, "Opens with:");
    // Another window covers the browser window.
    let cover = gtk::Window::builder().title("Another app").build();
    cover.present();

    press(&general, "Change...");

    let open_with = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|toplevel| toplevel.downcast::<OpenWithDialog>().ok())
            .find(WidgetExt::is_visible)
    };
    wait_until("Open with", || open_with().is_some());
    let dialog = open_with().expect("Open with is open");
    assert_eq!(
        dialog.transient_for().as_ref(),
        Some(window.upcast_ref::<gtk::Window>()),
        "Open with belongs to the Properties window"
    );
    assert!(dialog.is_modal(), "a dialog of its own over Properties");
    assert!(!window.is_closed(), "Properties stays open");
    dialog.close();
    wait_until("Open with to close", || open_with().is_none());
    assert!(!window.is_closed(), "Cancel keeps Properties open");
    assert_eq!(
        value_after(&general, "Opens with:"),
        app_before,
        "Cancel changes nothing"
    );
    cover.destroy();
}

/// Escape closes Properties as Cancel does.
///
/// parity: PROP-001, ACC-004
#[gtk::test]
fn escape_closes_properties() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("properties", None);
    let window = test.wait_for_properties("Example projects Properties");
    let shortcut = window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            controller
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .collect::<Vec<_>>()
        })
        .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
        .find(|shortcut| {
            shortcut
                .trigger()
                .is_some_and(|trigger| trigger.to_str().as_str() == "Escape")
        })
        .expect("the window has the Escape shortcut");

    let action = shortcut.action().expect("the shortcut has an action");
    action.activate(gtk::ShortcutActionFlags::empty(), &window, None);

    assert!(window.is_closed(), "Escape closed the window");
    assert!(test.window.properties_windows().is_empty());
}

/// parity: PROP-026, PROP-027
#[gtk::test]
fn calculate_folder_size_shows_the_size_in_the_view_and_the_bar() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents").join("a.txt"), b"0123456789").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");

    test.activate("calculate-folder-size-of", Some(&fixture.uri_of("Documents")));

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

/// The Previous Versions tab of `window`.
pub(super) fn versions_of(window: &PropertiesWindow) -> VersionsPanel {
    tab(window, "versions")
        .downcast::<VersionsPanel>()
        .expect("the tab is the versions panel")
}

/// Previous Versions lists the snapshot under Name and Date modified,
/// grouped by date, and Open and Restore act on the one selected; a
/// click only selects it.
///
/// parity: PROP-019, PROP-020, PROP-032
#[gtk::test]
fn previous_versions_lists_the_snapshots_of_the_folder() {
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");

    test.activate("previous-versions", None);

    let window = test.wait_for_properties("Documents Properties");
    assert_eq!(window.selected_tab().as_deref(), Some("versions"));
    let versions = versions_of(&window);
    wait_until("the versions", || !versions.version_labels().is_empty());
    assert_eq!(versions.version_labels(), [SNAPSHOT_NAME]);
    let shown = versions.texts();
    for text in ["Folder versions:", "Name", "Date modified", "Documents"] {
        assert!(shown.iter().any(|shown| shown == text), "{text}: {shown:?}");
    }
    let headings = [
        "Earlier this year",
        "A long time ago",
        "Last month",
        "Earlier this month",
    ];
    assert!(
        shown.iter().any(|text| headings.contains(&text.as_str())),
        "a date heading: {shown:?}"
    );
    assert!(shown.iter().any(|text| text.ends_with(" 12:30")), "{shown:?}");
    assert!(!button(&versions, "Open").is_sensitive(), "nothing selected yet");
    let rows = descendants::<gtk::ListBox>(&versions).remove(0);
    assert!(
        !rows.activates_on_single_click(),
        "a click only selects a version, for Open or Restore"
    );
    capture_dialog(&window, "native-properties-versions.png");

    versions.select_version(0);
    assert!(button(&versions, "Open").is_sensitive());
    versions.press(false);
    let frame = test.wait_for_dialog("the Restore dialog");
    assert_eq!(frame.title(), "Restore a copy");
    frame.close();
    versions.press(true);
    test.wait_for_listing("the snapshot");
    assert!(test.window.snapshot_banner().is_visible());
}

/// parity: PROP-019
#[gtk::test]
fn a_file_without_snapshots_explains_that_none_were_found() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");

    test.activate("previous-versions", None);

    let window = test.wait_for_properties("Notes 2.txt Properties");
    let versions = versions_of(&window);
    wait_until("the lookup", || {
        versions
            .texts()
            .iter()
            .any(|text| text == "There are no previous versions available.")
    });
    assert!(versions.texts().iter().any(|text| text == "File versions:"));
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
/// item selected, and the item's Properties.
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

    test.wait_for_properties("Documents Properties");
    assert_eq!(test.window.tab_count(), 2);
}

/// parity: PROP-001
#[gtk::test]
fn properties_of_a_location_open_for_menus_outside_the_folder_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("properties-of", Some(&fixture.uri_of("Documents")));

    let window = test.wait_for_properties("Documents Properties");
    let general = general(&window);
    wait_until("the count", || {
        value_after(&general, "Size:").is_some_and(|text| text.ends_with(" bytes)"))
    });
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

/// Closing the window ends its size scan: the folder being measured
/// stops and the folders still waiting are not measured.
///
/// parity: PROP-029
#[gtk::test]
fn closing_the_window_ends_its_size_scan() {
    let fixture = Fixture::standard();
    for number in 0..5 {
        fs::create_dir(fixture.path(&format!("Folder {number}"))).expect("fixture folder");
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("calculate-folder-sizes", None);
    let cancel = test.window.size_run_cancellation().expect("a run");

    test.window.close();

    assert!(cancel.is_cancelled(), "the scan stops with its window");
    wait_until("the run to end", || test.window.size_run_cancellation().is_none());
    let measured = (0..5)
        .filter(|number| {
            let uri = fixture.uri_of(&format!("Folder {number}"));
            test.window.measured_folder_size(&uri).is_some()
        })
        .count();
    assert!(measured < 5, "the folders still waiting are not measured");
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
    let window = test.wait_for_properties("Documents Properties");
    let versions = versions_of(&window);
    wait_until("the versions", || !versions.version_labels().is_empty());

    press(&versions, "Snapshot source...");
    let fields = descendants::<gtk::Entry>(&versions);
    fields[1].set_text(&backups.display().to_string());
    press(&versions, "Save source");

    wait_until("the list again", || {
        versions.texts().iter().any(|text| text == "Folder versions:")
    });
    let sources = test.context.previous_versions().sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].live(), fixture.uri_of("Documents"));
    assert_eq!(sources[0].collection(), ox_core::location::file_uri(&backups));
}

/// A folder's Customize tab holds Change Icon...; the icon is stored as
/// the `GVfs` metadata Files reads, shows in the folder view, and Restore
/// Default with OK goes back to the default art. A file has no Customize
/// tab, as in Windows.
///
/// parity: PROP-016
#[gtk::test]
fn a_custom_icon_shows_in_the_view_until_the_default_is_restored() {
    use crate::folder_view::cells::FileCell;
    use crate::properties::set_custom_icon;

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Album")).expect("a folder");
    let pixels = glib::Bytes::from_owned(vec![0x80_u8; 2 * 2 * 4]);
    let picture = gtk::gdk::MemoryTexture::new(2, 2, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    picture
        .save_to_png(fixture.path("Star.png"))
        .expect("an icon picture");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let file = test.wait_for_properties("Notes 2.txt Properties");
    assert!(
        file.tab_page("customize").is_none(),
        "a file has no Customize tab"
    );
    file.close();
    let item = fixture.uri_of("Album");
    let shows_custom = |wanted: bool| {
        descendants::<FileCell>(&test.window)
            .iter()
            .any(|cell| cell.name() == "Album" && cell.shows_custom_icon() == wanted)
    };
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let finished = std::rc::Rc::clone(&done);
    let (target, icon) = (item.clone(), fixture.uri_of("Star.png"));
    glib::spawn_future_local(async move {
        set_custom_icon(&target, Some(icon))
            .await
            .expect("the metadata is stored");
        finished.set(true);
    });
    wait_until("the change", || done.get());
    test.window.refresh_item_icon(&item);
    wait_until("the custom icon", || shows_custom(true));

    test.select_named("Album");
    test.activate("properties", None);
    let window = test.wait_for_properties("Album Properties");
    let customize = group_box(&tab(&window, "customize"), "Folder icons");
    wait_until("the icon buttons", || {
        button(&customize, "Restore Default").is_visible()
    });
    assert!(button(&customize, "Change Icon...").is_sensitive());
    press(&customize, "Restore Default");
    assert!(window.apply_button().is_sensitive());
    window.ok_button().emit_clicked();

    wait_until("the default art", || shows_custom(false));
    let stored = gtk::gio::File::for_uri(&item)
        .query_info(
            "metadata::custom-icon",
            gtk::gio::FileQueryInfoFlags::NONE,
            gtk::gio::Cancellable::NONE,
        )
        .expect("metadata")
        .attribute_string("metadata::custom-icon");
    assert_eq!(stored, None);
}

/// The group box of the Customize tab titled `title`.
fn group_box(page: &gtk::Widget, title: &str) -> gtk::Widget {
    descendants::<gtk::Frame>(page)
        .into_iter()
        .find(|frame| frame.label().as_deref() == Some(title))
        .unwrap_or_else(|| panic!("a {title} group"))
        .upcast()
}

/// Stores `picture` as the folder picture of `folder`, as OK does.
fn put_folder_picture(folder: &str, picture: &str) {
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let finished = std::rc::Rc::clone(&done);
    let (folder, picture) = (folder.to_owned(), picture.to_owned());
    glib::spawn_future_local(async move {
        let info = gtk::gio::FileInfo::new();
        info.set_attribute_string(crate::folder_view::FOLDER_PICTURE, &picture);
        gtk::gio::File::for_uri(&folder)
            .set_attributes_future(&info, gtk::gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT)
            .await
            .expect("the metadata is stored");
        finished.set(true);
    });
    wait_until("the folder picture to be stored", || done.get());
}

/// The Customize tab has Windows 11's three groups in its order. A
/// template chosen under "What kind of folder do you want?" and applied
/// with "Also apply this template to all subfolders" shows the folder and
/// its sub-folders in the template's view (Pictures: Large icons) and is
/// there when the window opens again; nothing is written into the folder.
///
/// parity: PROP-036
#[gtk::test]
fn a_template_optimizes_a_folder_and_its_subfolders() {
    use crate::window::FolderView;

    let fixture = Fixture::standard();
    fs::create_dir_all(fixture.path("Holiday/Beach")).expect("folders");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Holiday");
    test.activate("properties", None);
    let window = test.wait_for_properties("Holiday Properties");
    let page = tab(&window, "customize");
    let titles: Vec<String> = descendants::<gtk::Frame>(&page)
        .iter()
        .filter_map(|frame| frame.label().map(|label| label.to_string()))
        .collect();
    assert_eq!(
        titles,
        [
            "What kind of folder do you want?",
            "Folder pictures",
            "Folder icons"
        ]
    );
    let kind = group_box(&page, "What kind of folder do you want?");
    let list = descendants::<gtk::DropDown>(&kind)
        .into_iter()
        .next()
        .expect("the template list");
    assert_eq!(list.selected(), 0, "General items");
    let names: Vec<String> = (0..5)
        .filter_map(|at| list.model()?.item(at)?.downcast::<gtk::StringObject>().ok())
        .map(|name| name.string().to_string())
        .collect();
    assert_eq!(
        names,
        ["General items", "Documents", "Pictures", "Music", "Videos"]
    );
    assert!(!window.apply_button().is_sensitive());
    list.set_selected(2);
    check(&kind, "Also apply this template to all subfolders").set_active(true);
    assert!(window.apply_button().is_sensitive());
    window.ok_button().emit_clicked();

    let holiday = fixture.uri_of("Holiday");
    let beach = format!("{holiday}/Beach");
    wait_until("the template to be saved", || {
        test.window.folder_template(&beach) == Some(FolderTemplate::Pictures)
    });
    test.show(&beach);
    test.wait_for_listing("Beach");
    wait_until("Large icons", || {
        test.window.folder_pane().view() == FolderView::Icons(crate::folder_view::grid::IconSize::LARGE)
    });
    let written: Vec<_> = fs::read_dir(fixture.path("Holiday"))
        .expect("the folder")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(written, ["Beach"], "nothing is written into the folder");

    test.show(&fixture.uri());
    test.wait_for_listing("the fixture");
    test.select_named("Holiday");
    test.activate("properties", None);
    let again = test.wait_for_properties("Holiday Properties");
    let kind = group_box(&tab(&again, "customize"), "What kind of folder do you want?");
    let list = descendants::<gtk::DropDown>(&kind)
        .into_iter()
        .next()
        .expect("the template list");
    assert_eq!(list.selected(), 2, "Pictures, as applied");
    assert!(!check(&kind, "Also apply this template to all subfolders").is_active());
    again.close();
}

/// Apply on the Customize tab restyles at once, without a reload, every
/// open window showing the folder and, with "Also apply this template to
/// all subfolders", every one showing a sub-folder, keeping its selection.
///
/// parity: PROP-036
#[gtk::test]
fn applying_a_template_restyles_open_windows_at_once() {
    use crate::folder_view::grid::IconSize;
    use crate::window::FolderView;

    let fixture = Fixture::standard();
    fs::create_dir_all(fixture.path("Holiday/Beach")).expect("folders");
    fs::write(fixture.path("Holiday/Notes.txt"), "fictional").expect("a file");
    let holiday = fixture.uri_of("Holiday");
    let beach = format!("{holiday}/Beach");
    let test = TestWindow::open(&fixture.uri());
    let on_holiday = test.open_beside(&holiday);
    let on_beach = test.open_beside(&beach);
    on_holiday.select_named("Notes.txt");
    let large = FolderView::Icons(IconSize::LARGE);
    assert_eq!(on_holiday.window.folder_pane().view(), FolderView::Details);

    test.select_named("Holiday");
    test.activate("properties", None);
    let window = test.wait_for_properties("Holiday Properties");
    let kind = group_box(&tab(&window, "customize"), "What kind of folder do you want?");
    let list = descendants::<gtk::DropDown>(&kind)
        .into_iter()
        .next()
        .expect("the template list");
    list.set_selected(2);
    window.apply_button().emit_clicked();
    wait_until("the folder's window in Large icons", || {
        on_holiday.window.folder_pane().view() == large
    });
    assert_eq!(on_holiday.selected_names(), ["Notes.txt"], "the selection stays");
    wait_for(std::time::Duration::from_millis(300));
    assert_eq!(
        on_beach.window.folder_pane().view(),
        FolderView::Details,
        "a sub-folder keeps its view without the box"
    );
    assert_eq!(
        test.window.folder_pane().view(),
        FolderView::Details,
        "the parent keeps its view"
    );

    list.set_selected(4);
    check(&kind, "Also apply this template to all subfolders").set_active(true);
    window.ok_button().emit_clicked();
    wait_until("the sub-folder's window in Large icons", || {
        on_beach.window.folder_pane().view() == large
    });
    assert_eq!(on_holiday.window.folder_pane().view(), large);
    assert_eq!(test.window.folder_pane().view(), FolderView::Details);
}

/// A folder's picture shows on its icon at Large icons and in the small
/// icon's place in Details, in every open window at once on OK, as in
/// Windows; the Customize tab previews it, and Restore Default with OK
/// takes it off again everywhere. The details pane's hook reads it.
///
/// parity: PROP-037
#[gtk::test]
fn a_folder_picture_shows_in_every_window_until_restored() {
    use crate::folder_view::cells::FileCell;

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Holiday")).expect("a folder");
    let pixels = glib::Bytes::from_owned(vec![0x60_u8; 4 * 3 * 4]);
    let picture = gtk::gdk::MemoryTexture::new(4, 3, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 16);
    picture
        .save_to_png(fixture.path("Harbour.png"))
        .expect("a picture");
    let holiday = fixture.uri_of("Holiday");
    put_folder_picture(&holiday, &fixture.uri_of("Harbour.png"));
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("large"));
    let in_details = test.open_beside(&fixture.uri());
    let holiday_cells = |window: &TestWindow| -> Vec<FileCell> {
        descendants::<FileCell>(&window.window)
            .into_iter()
            .filter(|cell| cell.name() == "Holiday")
            .collect()
    };
    let on_tile = |shown: bool| {
        let cells = holiday_cells(&test);
        !cells.is_empty() && cells.iter().all(|cell| cell.shows_folder_picture() == shown)
    };
    let in_row = |shown: bool| {
        let cells = holiday_cells(&in_details);
        !cells.is_empty()
            && cells
                .iter()
                .all(|cell| cell.shows_custom_icon() == shown && !cell.shows_folder_picture())
    };
    wait_until("the picture on the folder at Large icons", || on_tile(true));
    wait_until("the picture as the row's icon in Details", || in_row(true));

    let mut found = None;
    let hook = std::rc::Rc::new(std::cell::RefCell::new(None));
    let heard = std::rc::Rc::clone(&hook);
    let asked = holiday.clone();
    glib::spawn_future_local(async move {
        heard.replace(Some(crate::folder_view::cells::folder_picture_of(&asked).await));
    });
    wait_until("the details pane's hook", || hook.borrow().is_some());
    found.clone_from(&*hook.borrow());
    assert_eq!(found, Some(Some(fixture.uri_of("Harbour.png"))));

    test.select_named("Holiday");
    test.activate("properties", None);
    let window = test.wait_for_properties("Holiday Properties");
    let pictures = group_box(&tab(&window, "customize"), "Folder pictures");
    assert!(texts(&pictures).contains(&"Choose a file to show on this folder icon.".to_owned()));
    assert!(button(&pictures, "Choose File...").is_sensitive());
    wait_until("the folder's picture", || {
        button(&pictures, "Restore Default").is_sensitive()
    });
    press(&pictures, "Restore Default");
    assert!(!button(&pictures, "Restore Default").is_sensitive());
    assert!(window.apply_button().is_sensitive());
    wait_for(std::time::Duration::from_millis(200));
    assert!(on_tile(true), "nothing changes before OK or Apply");
    window.ok_button().emit_clicked();

    wait_until("the plain folder at Large icons", || on_tile(false));
    wait_until("the plain folder in the other window's row", || in_row(false));
    let stored = gtk::gio::File::for_uri(&holiday)
        .query_info(
            crate::folder_view::FOLDER_PICTURE,
            gtk::gio::FileQueryInfoFlags::NONE,
            gtk::gio::Cancellable::NONE,
        )
        .expect("metadata")
        .attribute_string(crate::folder_view::FOLDER_PICTURE);
    assert_eq!(stored, None);
}

/// Before and after pictures of an open window as OK on the Customize
/// tab applies the Pictures template and a folder picture, in both
/// themes, for review with `OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`):
/// `customize-apply-{before,after}-{light,dark}.png`. Without it, they
/// only prove the window follows OK at once in both themes.
///
/// parity: PROP-036, PROP-037
#[gtk::test]
fn applying_the_customize_tab_is_captured_before_and_after() {
    use crate::folder_view::grid::IconSize;
    use crate::window::FolderView;

    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        let fixture = Fixture::empty();
        let holiday = fixture.path("Holiday 2025");
        fs::create_dir_all(holiday.join("Beach day")).expect("folders");
        fs::create_dir_all(holiday.join("Old town")).expect("folders");
        for (name, shade) in [
            ("Harbour.png", 0x40_u8),
            ("Sunset.png", 0xb0),
            ("Market.png", 0x70),
        ] {
            let pixels = glib::Bytes::from_owned(
                (0..48 * 32)
                    .flat_map(|at: u32| {
                        let fade = u8::try_from(at / 48 * 4).unwrap_or(u8::MAX);
                        [shade, fade, 0xff - fade, 0xff]
                    })
                    .collect::<Vec<u8>>(),
            );
            gtk::gdk::MemoryTexture::new(48, 32, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 48 * 4)
                .save_to_png(holiday.join(name))
                .expect("a picture");
        }
        let uri = fixture.uri_of("Holiday 2025");
        let test = TestWindow::open(&uri);
        test.activate("theme", Some(theme));
        test.window.set_default_size(900, 560);
        wait_for_frames(&test.window, 4);
        capture(&test.window, &format!("customize-apply-before-{theme}.png"));
        test.activate("properties", None);
        let window = test.wait_for_properties("Holiday 2025 Properties");
        let page = tab(&window, "customize");
        let kind = group_box(&page, "What kind of folder do you want?");
        let list = descendants::<gtk::DropDown>(&kind)
            .into_iter()
            .next()
            .expect("the template list");
        list.set_selected(2);
        check(&kind, "Also apply this template to all subfolders").set_active(true);
        window.ok_button().emit_clicked();
        wait_until("Large icons at once", || {
            test.window.folder_pane().view() == FolderView::Icons(IconSize::LARGE)
        });
        put_folder_picture(&format!("{uri}/Beach%20day"), &format!("{uri}/Harbour.png"));
        test.window.announce_item_icon(&format!("{uri}/Beach%20day"));
        wait_until("the picture on Beach day", || {
            descendants::<crate::folder_view::cells::FileCell>(&test.window)
                .iter()
                .any(|cell| cell.name() == "Beach day" && cell.shows_folder_picture())
        });
        capture(&test.window, &format!("customize-apply-after-{theme}.png"));
    }
}

/// Change Icon... opens Windows' "Change Icon for <folder> Folder", which
/// lists the icon theme's folder icons, never its glyphs, with Browse...,
/// OK and Cancel; the icon chosen shows on the Customize tab on OK and
/// waits there for the tab's own OK or Apply.
///
/// parity: PROP-016
#[gtk::test]
fn change_icon_lists_the_themes_folder_icons() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Album")).expect("a folder");
    let theme = tempfile::tempdir().expect("an icon folder");
    let pixels = glib::Bytes::from_owned(vec![0x80_u8; 2 * 2 * 4]);
    let picture = gtk::gdk::MemoryTexture::new(2, 2, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    for name in ["folder-oxtest.png", "folder-oxtest-symbolic.png"] {
        picture
            .save_to_png(theme.path().join(name))
            .expect("an icon picture");
    }
    let test = TestWindow::open(&fixture.uri());
    gtk::IconTheme::for_display(&WidgetExt::display(&test.window)).add_search_path(theme.path());
    test.select_named("Album");
    test.activate("properties", None);
    let window = test.wait_for_properties("Album Properties");
    let customize = group_box(&tab(&window, "customize"), "Folder icons");
    wait_until("the icon buttons", || {
        texts(&customize).contains(&"Change Icon...".to_owned())
    });
    assert!(!button(&customize, "Restore Default").is_visible());

    press(&customize, "Change Icon...");
    let dialog = sub_dialog(&window, "Change Icon for Album Folder");
    assert_eq!(dialog.button_labels(), ["OK", "Cancel"]);
    assert!(button(&dialog, "Browse...").is_sensitive());
    let tiles = descendants::<gtk::FlowBoxChild>(&dialog);
    let names: Vec<String> = tiles
        .iter()
        .filter_map(|tile| tile.tooltip_text().map(String::from))
        .collect();
    assert!(names.iter().any(|name| name == "folder-oxtest"), "{names:?}");
    assert!(!names.iter().any(|name| name.ends_with("-symbolic")), "{names:?}");
    let tile = tiles
        .iter()
        .find(|tile| tile.tooltip_text().as_deref() == Some("folder-oxtest"))
        .expect("the theme's folder icon");
    descendants::<gtk::FlowBox>(&dialog)[0].select_child(tile);
    dialog.ok_button().emit_clicked();

    wait_until("Change Icon to close", || dialog.is_closed());
    assert!(
        button(&customize, "Restore Default").is_visible(),
        "the icon is chosen"
    );
    assert!(window.apply_button().is_sensitive(), "it waits for OK or Apply");
    window.close();
}

/// OK pressed while Apply's change is still being made closes the window
/// once the change is made, and does not make it twice.
///
/// parity: PROP-001
#[gtk::test]
fn ok_during_apply_closes_once_the_change_is_made() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");

    name_field(&window).set_text("Notes 2.md");
    press(&window, "Apply");
    wait_until("the question", || extension_questions(&window) == 1);
    window.ok_button().emit_clicked();
    assert!(!window.is_closed(), "the change is still being made");
    message_over(&window).press("Change extension");

    wait_until("the rename", || fixture.path("Notes 2.md").exists());
    wait_until("the window to close", || window.is_closed());
    assert_eq!(extension_questions(&window), 0, "asked once");
}
