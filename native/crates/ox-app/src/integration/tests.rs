// SPDX-License-Identifier: AGPL-3.0-only
//! The integration as the app uses it: the `FileManager1` service on the
//! test's private session bus, Show in folder requests in a window, and
//! Open with. Nothing here changes the associations of a real session or
//! starts a real application.
//!
//! Ports `handle_reveal`, `enable_reveal`/`disable_reveal` and
//! `revealTest` of `desktop/winspace.py`, `handleFileManagerRequest` and
//! `openWithDialog` of `desktop/ui/app.js`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{
    BraveIntegration, BravePaths, FileManagerMethod, FileManagerRequest, ProcessTable, Sandbox, BUS_NAME,
    OBJECT_PATH,
};

use super::{
    BraveDialog, DefaultChoice, DesktopIntegration, IntegrationFolders, LaunchTarget, MimeBackend,
    OpenWithDialog, OpenWithError, OpenWithSubject, PreparedLaunch,
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
        let root = tempfile::tempdir().expect("a temporary folder");
        let folders = IntegrationFolders::inside(root.path());
        let (backend, _) = MimeBackend::in_memory("org.kde.dolphin.desktop");
        let integration = DesktopIntegration::with_mime_backend(&folders, Sandbox::Host, backend);
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

/// The window's Open with launcher starts applications with a launch
/// context of the window's own display, which gives them startup
/// notification and focus.
///
/// parity: INT-023
#[gtk::test]
fn applications_start_with_the_windows_display() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let prepared = PreparedLaunch {
        target: LaunchTarget::Path(fixture.path("Notes 2.txt")),
        content_type: "text/plain".to_owned(),
        is_folder: false,
    };

    let launcher = test.window.launcher_with(record_start);
    let toast = launcher("demo-editor.desktop", &prepared, DefaultChoice::Keep);

    assert_eq!(toast.ok(), Some("Opened with the selected application."));
    let started = STARTED.with(RefCell::take);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].0, "demo-editor.desktop");
    let context = started[0]
        .1
        .downcast_ref::<gtk::gdk::AppLaunchContext>()
        .expect("a display's launch context");
    assert_eq!(
        gtk::gdk::prelude::GdkAppLaunchContextExt::display(context),
        WidgetExt::display(&test.window)
    );
}

thread_local! {
    /// What [`record_start`] was asked to start, with the launch context.
    static STARTED: RefCell<Vec<(String, gio::AppLaunchContext)>> = const { RefCell::new(Vec::new()) };
}

/// Records a launch instead of starting an application.
#[allow(clippy::unnecessary_wraps, reason = "it stands in for integration::launch")]
fn record_start(
    app_id: &str,
    _prepared: &PreparedLaunch,
    _default: DefaultChoice,
    launch_context: &gio::AppLaunchContext,
) -> Result<&'static str, OpenWithError> {
    STARTED.with(|started| {
        started
            .borrow_mut()
            .push((app_id.to_owned(), launch_context.clone()));
    });
    Ok("Opened with the selected application.")
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

/// Restore previous needs the consent and one profile, and says why it
/// changed nothing when no earlier setting was recorded.
///
/// parity: INT-021
#[gtk::test]
fn the_brave_dialogs_restore_needs_consent_and_a_record() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let root = tempfile::tempdir().expect("a temporary folder");
    let folders = IntegrationFolders::inside(root.path());
    let profile = folders.config_home.join("BraveSoftware/Brave-Browser/Default");
    std::fs::create_dir_all(&profile).expect("the temporary folder is writable");
    let preferences = r#"{"download": {"default_directory": "/tmp/old"}}"#;
    std::fs::write(profile.join("Preferences"), preferences).expect("the profile is writable");
    // An empty process table: Brave does not run, whatever the machine runs.
    let processes = root.path().join("proc");
    std::fs::create_dir(&processes).expect("the temporary folder is writable");
    let paths = BravePaths {
        settings: folders.settings.clone(),
        home: folders.home.clone(),
        config_home: folders.config_home.clone(),
    };
    let brave = BraveIntegration::with_activity(&paths, Sandbox::Host, ProcessTable::at(&processes));
    let dialog = BraveDialog::present_for(&test.window, brave, &fixture.uri(), |_| {});
    wait_until("the profiles", || !dialog.profile_labels().is_empty());

    dialog.click_restore();
    assert_eq!(
        dialog.status(),
        "Select one profile and confirm to restore its previous download setting."
    );

    dialog.set_consent(true);
    dialog.click_restore();
    wait_until("the answer", || {
        dialog.status() == "No previous download setting was recorded for this profile."
    });
    let unchanged = std::fs::read_to_string(profile.join("Preferences")).expect("the profile is readable");
    assert_eq!(unchanged, preferences);
    dialog.close();
}
