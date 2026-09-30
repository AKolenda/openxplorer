// SPDX-License-Identifier: AGPL-3.0-only
//! The integration as the app uses it: the `FileManager1` service on the
//! test's private session bus, Show in folder requests in a window, and
//! Open with. Nothing here changes the associations of a real session or
//! starts a real application.
//!
//! Ports `handle_reveal`, `enable_reveal`/`disable_reveal` and
//! `revealTest` of `desktop/winspace.py`, `handleFileManagerRequest` and
//! `openWithDialog` of `desktop/ui/app.js`. Show in folder inside Flatpak
//! is in `flatpak`.

mod flatpak;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{FileManagerMethod, FileManagerRequest, Sandbox, BUS_NAME, OBJECT_PATH};

use super::{
    BraveDialog, DesktopIntegration, IntegrationFolders, MimeBackend, OpenWithDialog, OpenWithSubject,
};
use crate::test_support::harness::{application, capture_dialog, wait_until, Fixture, TestWindow};

/// Requests the service handed to the application.
type Received = Rc<RefCell<Vec<FileManagerRequest>>>;

/// An integration attached to the test application whose requests are
/// recorded, and which disables Show in folder when dropped, so a failed
/// test leaves no session files or bus name to the next one.
struct AttachedIntegration {
    integration: DesktopIntegration,
    received: Received,
    folders: IntegrationFolders,
    _root: tempfile::TempDir,
}

impl AttachedIntegration {
    fn new() -> Self {
        Self::attach(|folders, backend| {
            DesktopIntegration::with_mime_backend(folders, Sandbox::Host, backend)
        })
    }

    /// An integration made by `make` in a temporary folder, attached.
    fn attach(make: impl FnOnce(&IntegrationFolders, MimeBackend) -> DesktopIntegration) -> Self {
        let root = tempfile::tempdir().expect("a temporary folder");
        let folders = IntegrationFolders::inside(root.path());
        let (backend, _) = MimeBackend::in_memory("org.kde.dolphin.desktop");
        let integration = make(&folders, backend);
        let received: Received = Rc::default();
        let recorder = Rc::clone(&received);
        integration.attach(&application(), move |request, _startup_id| {
            recorder.borrow_mut().push(request);
            Ok(())
        });
        Self {
            integration,
            received,
            folders,
            _root: root,
        }
    }
}

/// Runs `future` on the main loop until it finishes.
fn wait_for<T: 'static>(what: &str, future: impl std::future::Future<Output = T> + 'static) -> T {
    let result: Rc<RefCell<Option<T>>> = Rc::default();
    let slot = Rc::clone(&result);
    glib::spawn_future_local(async move {
        let value = future.await;
        slot.replace(Some(value));
    });
    wait_until(what, || result.borrow().is_some());
    result.take().expect("the future finished")
}

impl Drop for AttachedIntegration {
    fn drop(&mut self) {
        let integration = self.integration.clone();
        let _ = wait_for("Show in folder to be disabled", async move {
            integration.disable_show_in_folder().await.is_ok()
        });
    }
}

/// Enabling Show in folder writes the session files and owns
/// `FileManager1` on the session bus; a `ShowFolders` call, as the test
/// sends it, reaches the application; disabling gives the name up.
///
/// parity: INT-013, INT-015, INT-016, INT-017
#[gtk::test]
fn show_in_folder_owns_file_manager1_until_disabled() {
    let attached = AttachedIntegration::new();
    let integration = attached.integration.clone();

    let enabled = integration.clone();
    wait_for("enabling", async move { enabled.enable_show_in_folder().await })
        .expect("Show in folder can be enabled in the test session");
    wait_until("the bus name", || integration.owns_file_manager());
    let service_file = attached
        .folders
        .data_home
        .join("dbus-1/services/org.freedesktop.FileManager1.service");
    assert!(service_file.is_file(), "the session's service file is written");
    let reading = integration.clone();
    let status = wait_for("the status", async move { reading.status().await });
    assert_eq!(
        status.show_in_folder.text(),
        "Show in folder: OpenXplorer owns FileManager1. Browser portal routing is a separate check."
    );

    let testing = integration.clone();
    let outcome = wait_for("the test", async move { testing.test_show_in_folder().await });
    assert_eq!(
        outcome.expect("the test reaches the service").message(),
        "Test request sent through FileManager1."
    );
    let received = attached.received.borrow().clone();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].method(), FileManagerMethod::ShowFolders);

    let disabling = integration.clone();
    wait_for(
        "disabling",
        async move { disabling.disable_show_in_folder().await },
    )
    .expect("Show in folder can be disabled");
    assert!(!integration.owns_file_manager());
    assert!(!service_file.exists(), "the service file is removed");
    let reading = integration.clone();
    let status = wait_for("the status", async move { reading.status().await });
    assert!(!status.show_in_folder.is_enabled);
}

/// A request with a file that is not a location is refused on the bus as
/// invalid arguments and never reaches the application.
///
/// parity: INT-013
#[gtk::test]
fn the_service_refuses_invalid_requests_before_the_app_sees_them() {
    let attached = AttachedIntegration::new();
    let integration = attached.integration.clone();
    let enabled = integration.clone();
    wait_for("enabling", async move { enabled.enable_show_in_folder().await })
        .expect("Show in folder can be enabled in the test session");
    wait_until("the bus name", || integration.owns_file_manager());
    let connection = application()
        .dbus_connection()
        .expect("the test application is on the bus");

    let arguments = (vec!["javascript:alert(1)".to_owned()], String::new()).to_variant();
    let call = connection.call_future(
        Some(BUS_NAME),
        OBJECT_PATH,
        BUS_NAME,
        "ShowItems",
        Some(&arguments),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    );
    let answer = wait_for("the answer", call);

    let error = answer.expect_err("the location is refused");
    assert_eq!(
        gio::DBusError::remote_error(&error).as_deref(),
        Some("org.freedesktop.DBus.Error.InvalidArgs")
    );
    assert!(attached.received.borrow().is_empty());
}

/// `ShowItems` selects exactly the requested file in the tab that already
/// shows its folder, and opens another folder's file in a new tab.
///
/// parity: INT-007, INT-014
#[gtk::test]
fn show_items_selects_the_files_in_their_folders() {
    let fixture = Fixture::standard();
    fixture.write("Documents/report.txt");
    let test = TestWindow::open(&fixture.uri());
    let items = [
        fixture.uri_of("Notes 2.txt"),
        fixture.uri_of("Documents/report.txt"),
    ];
    let request = FileManagerRequest::new(FileManagerMethod::ShowItems, &items).expect("valid locations");

    test.window.show_file_manager_request(&request);

    wait_until("the second folder's tab", || test.window.tab_count() == 2);
    test.wait_for_listing("the Documents listing");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.selected_names(), ["report.txt"]);
    gtk::prelude::WidgetExt::activate_action(&test.window, "win.previous-tab", None)
        .expect("tab actions exist");
    test.wait_for_listing("the first tab");
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// `ShowFolders` opens each folder in a new tab and never a file as a
/// folder.
///
/// parity: INT-014
#[gtk::test]
fn show_folders_opens_each_folder_in_a_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let folders = [fixture.uri_of("Documents")];
    let request =
        FileManagerRequest::new(FileManagerMethod::ShowFolders, &folders).expect("a valid location");

    test.window.show_file_manager_request(&request);

    wait_until("the new tab", || test.window.tab_count() == 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// A launcher that records what it would start.
type Launches = Rc<RefCell<Vec<String>>>;

fn recording_launcher(launches: &Launches) -> super::Launcher {
    let launches = Rc::clone(launches);
    Box::new(move |app_id, _prepared, _default| {
        launches.borrow_mut().push(app_id.to_owned());
        Ok("Opened with the selected application.")
    })
}

/// Open with lists the installed applications for a file, chooses the
/// default or the first available one, filters by name, and launches the
/// chosen one after checking it again. "Always use this app" is offered
/// for files.
///
/// parity: OPEN-011, OPEN-012
#[gtk::test]
fn open_with_lists_filters_and_launches_the_chosen_application() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let launches: Launches = Rc::default();
    let subject = OpenWithSubject {
        uri: fixture.uri_of("Notes 2.txt"),
        name: "Notes 2.txt".to_owned(),
        is_folder: false,
    };
    let reports: Rc<RefCell<Vec<String>>> = Rc::default();
    let report = Rc::clone(&reports);
    let dialog = OpenWithDialog::present_for(
        &test.window,
        subject,
        recording_launcher(&launches),
        move |message| {
            report.borrow_mut().push(message.to_owned());
        },
    );

    wait_until("the list", || {
        dialog.status() == "Choose an installed application."
    });
    let (offers_default, _) = dialog.controls_state();
    assert!(offers_default, "Always use this app is offered for a file");
    capture_dialog(&dialog, "native-open-with.png");
    let names = dialog.shown_names();
    if let Some(chosen) = dialog.chosen() {
        dialog.click_open();
        wait_until("the launch", || !launches.borrow().is_empty());
        assert_eq!(*launches.borrow(), [chosen]);
        assert_eq!(*reports.borrow(), ["Opened with the selected application."]);
    } else {
        assert!(
            names.is_empty(),
            "an application is chosen when one can open the file"
        );
        dialog.type_filter("no such application");
        assert!(dialog.shown_names().is_empty());
        dialog.close();
    }
}

/// For a folder every installed application is listed, and the
/// file-manager default is never offered for change.
///
/// parity: OPEN-011
#[gtk::test]
fn open_folder_with_never_offers_to_change_the_default() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let launches: Launches = Rc::default();
    let subject = OpenWithSubject {
        uri: fixture.uri_of("Documents"),
        name: "Documents".to_owned(),
        is_folder: true,
    };
    let dialog = OpenWithDialog::present_for(&test.window, subject, recording_launcher(&launches), |_| {});

    wait_until("the list", || {
        dialog.status() == "Choose an installed application."
    });
    let (offers_default, _) = dialog.controls_state();
    assert!(!offers_default, "a folder's default is changed only in Settings");
    dialog.type_filter("no such application");
    assert!(dialog.shown_names().is_empty());
    dialog.close();
    assert!(launches.borrow().is_empty());
}

/// The Brave dialog lists each native profile, ticked, with its folder,
/// and changes nothing without the consent check box.
///
/// parity: INT-019, INT-020
#[gtk::test]
fn the_brave_dialog_lists_profiles_and_needs_consent() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let root = tempfile::tempdir().expect("a temporary folder");
    let folders = IntegrationFolders::inside(root.path());
    let profile = folders.config_home.join("BraveSoftware/Brave-Browser/Default");
    std::fs::create_dir_all(&profile).expect("the temporary folder is writable");
    let preferences = r#"{"profile": {"name": "Personal"}, "download": {"default_directory": "/tmp/old"}}"#;
    std::fs::write(profile.join("Preferences"), preferences).expect("the profile is writable");
    let (backend, _) = MimeBackend::in_memory("org.kde.dolphin.desktop");
    let integration = DesktopIntegration::with_mime_backend(&folders, Sandbox::Host, backend);

    let dialog = BraveDialog::present_for(&test.window, integration.brave(), &fixture.uri(), |_| {});
    wait_until("the profiles", || !dialog.profile_labels().is_empty());

    assert_eq!(dialog.profile_labels(), ["Personal · Brave-Browser\n/tmp/old"]);
    capture_dialog(&dialog, "native-brave-dialog.png");
    dialog.click_apply();
    assert_eq!(dialog.status(), "Confirm the change using the checkbox.");
    let unchanged = std::fs::read_to_string(profile.join("Preferences")).expect("the profile is readable");
    assert_eq!(unchanged, preferences);
    dialog.close();
}
