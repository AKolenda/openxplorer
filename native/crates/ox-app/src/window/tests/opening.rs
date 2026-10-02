// SPDX-License-Identifier: AGPL-3.0-only
//! Opening typed addresses, command-line locations and files. Files are
//! recorded instead of started (see `AppContext::record_launches`).

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use ox_core::settings::{PreferencesUpdate, Settings};

use crate::integration::{installed_application, Tool};
use crate::locations::Page;
use crate::test_support::harness::{application, wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::{Tab, TabPlacement};
use crate::window::tests::file_ops_support::{open_dialog, select_names};

fn can_go_back(test: &TestWindow) -> bool {
    let session = test.window.imp().session.borrow();
    session.active().is_some_and(|tab| tab.history.can_go_back())
}

/// A command that fails says why in the toast instead of failing
/// silently: here Enter on an address that does not exist, which leaves
/// the tab where it was.
///
/// parity: CMD-018
#[gtk::test]
fn a_failing_command_says_why_in_the_toast() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let missing = fixture.path("Missing folder");

    test.window
        .submit_address(missing.to_str().expect("fixture paths are UTF-8"));

    wait_until("the toast", || !test.window.shown_message().is_empty());
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(test.context.recorded_launches().is_empty());
}

/// A typed address whose lookup answers after the tab navigated elsewhere
/// is dropped: the file does not open and the tab stays where the user
/// went.
///
/// Ported from `v2.0.0:desktop/tests/ui_regressions.cjs::Navigation supersedes a delayed activation`
///
/// parity: SAFE-013
#[gtk::test]
fn navigating_drops_a_typed_address_that_answers_late() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let file = fixture.path("Notes 2.txt");
    let typed = file.to_str().expect("fixture paths are UTF-8");

    // The lookup answers on a later main-loop turn, after this navigation.
    test.window.submit_address(typed);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("the fixture folder");
    wait_until("the lookup to answer", || test.window.answered_activations() == 1);

    assert!(test.context.recorded_launches().is_empty());
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.window.submit_address(typed);
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
}

/// Lookups belong to their own tab: a background tab that finds its
/// location is a file does not cancel the address the user typed in the
/// front tab, while switching away from a tab and back drops its lookup.
///
/// parity: SAFE-013
#[gtk::test]
fn a_lookup_belongs_to_its_tab_and_a_tab_switch_drops_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let typed = fixture.path("Notes 2.txt");
    let typed = typed.to_str().expect("fixture paths are UTF-8");
    let front = test.window.imp().session.borrow().active_id().expect("a tab");
    test.window
        .open_tab(&fixture.uri_of("Documents"), TabPlacement::Background)
        .expect("the fixture folder");
    let background = test.window.imp().session.borrow().tabs()[1].id;

    test.window.submit_address(typed);
    test.window
        .open_file_location(background, &fixture.uri_of("Notes 10.txt"));
    wait_until("both lookups to answer", || {
        test.window.answered_activations() == 2
    });
    // The opener reads each file again before it records the launch.
    wait_until("both files to be opened", || {
        test.context.recorded_launches().len() == 2
    });
    let mut launches = test.context.recorded_launches();
    launches.sort();
    assert_eq!(
        launches,
        [fixture.uri_of("Notes 10.txt"), fixture.uri_of("Notes 2.txt")]
    );

    test.window.submit_address(typed);
    test.window.switch_tab(background);
    test.window.switch_tab(front);
    wait_until("the lookup to answer", || test.window.answered_activations() == 3);
    // An opener that did start would record its launch shortly after.
    wait_for(std::time::Duration::from_millis(300));
    assert_eq!(
        test.context.recorded_launches().len(),
        2,
        "the dropped lookup opens nothing"
    );
}

/// parity: NAV-033, NAV-040
#[gtk::test]
fn a_typed_file_path_opens_the_file_and_leaves_the_folder_and_history() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let file = fixture.path("Notes 2.txt");
    test.window
        .submit_address(file.to_str().expect("fixture paths are UTF-8"));
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 2.txt")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(!can_go_back(&test), "opening a file adds no history entry");
    assert_eq!(test.window.load_error(), None);
}

/// parity: NAV-040, NAV-037
#[gtk::test]
fn a_tab_opened_on_a_file_shows_its_folder_and_opens_the_file() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Notes 10.txt"));
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.window.load_error(), None);
    assert_eq!(test.names(), STANDARD_NAMES);
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 10.txt")]);
}

#[gtk::test]
fn a_folder_replaced_by_a_file_is_never_opened_by_a_reload() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Swap")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri_of("Swap"));
    fs::remove_dir(fixture.path("Swap")).expect("the subfolder is empty");
    fixture.write("Swap");
    test.window.refresh();
    wait_until("the tab to show the parent folder", || {
        test.window.current_uri() == Some(fixture.uri()) && !test.window.is_loading()
    });
    wait_for(std::time::Duration::from_millis(100));
    assert!(
        test.context.recorded_launches().is_empty(),
        "a reload never starts an application"
    );
}

/// parity: NAV-034
#[gtk::test]
fn a_typed_page_title_prefers_a_folder_of_that_name() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Network")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri());
    test.window.submit_address("Network");
    wait_until("the folder called Network", || {
        test.window.current_uri() == Some(fixture.uri_of("Network"))
    });
    test.window.submit_address("This PC");
    wait_until("the This PC page", || {
        test.window.current_uri().as_deref() == Some(Page::ThisPc.uri())
    });
}

/// Locations another program passes (`openxplorer %U`, as GNOME does for
/// the mount root of inserted media when `OpenXplorer` is the folder
/// handler) open in the running instance's active window.
///
/// parity: NAV-041, INT-024
#[gtk::test]
fn command_line_locations_open_in_the_current_tab_then_in_new_tabs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let locations = vec![
        fixture.uri_of("Documents"),
        fixture.uri(),
        fixture.uri_of("Notes 2.txt"),
    ];
    test.window.open_locations(locations);
    wait_until("every location to open", || {
        test.window.tab_count() == 2 && !test.context.recorded_launches().is_empty()
    });
    test.wait_for_listing("the new tab");
    let session = test.window.imp().session.borrow();
    let uris: Vec<&str> = session.tabs().iter().map(Tab::uri).collect();
    assert_eq!(uris, [fixture.uri_of("Documents"), fixture.uri()]);
    assert!(
        session.tabs()[0].history.can_go_back(),
        "the first location keeps the tab's history"
    );
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 2.txt")]);
}

/// The URIs of `test`'s tabs, left to right.
fn tab_uris(test: &TestWindow) -> Vec<String> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
}

/// An item is read again when it is opened: one deleted since the
/// folder was listed says why in "Could not open the item".
///
/// parity: OPEN-001
#[gtk::test]
fn an_item_is_read_again_when_opened_and_a_failure_is_a_dialog() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let position = test.position_of("Notes 2.txt");
    fs::remove_file(fixture.path("Notes 2.txt")).expect("the fixture file can be removed");

    test.window.activate_item(position);

    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Could not open the item");
    assert!(test.context.recorded_launches().is_empty(), "nothing opened");
    dialog.press("OK");
}

/// A file inside a snapshot is never handed to an application, which
/// could change it in place: "Could not open the item" says why.
///
/// parity: OPEN-005, OPEN-007
#[gtk::test]
fn a_file_in_a_previous_version_is_refused_and_not_opened() {
    let fixture = Fixture::standard();
    let snapshot = fixture.path(".snapshot/Monday");
    fs::create_dir_all(&snapshot).expect("a snapshot folder in the fixture");
    fs::write(snapshot.join("Plan.txt"), b"Synthetic test data\n").expect("a file in the snapshot");
    let test = TestWindow::open(&fixture.uri_of(".snapshot/Monday"));

    test.window.activate_item(test.position_of("Plan.txt"));

    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Could not open the item");
    assert!(
        dialog.message_text().contains("read-only"),
        "{}",
        dialog.message_text()
    );
    assert!(test.context.recorded_launches().is_empty(), "nothing opened");
    dialog.press("OK");
}

/// A folder opened just before another tab came to the front opens in
/// the tab it was opened from; one whose tab moved on is dropped.
///
/// parity: OPEN-004
#[gtk::test]
fn an_opened_folder_stays_with_the_tab_that_asked_for_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.activate_item(test.position_of("Documents"));
    test.window
        .add_tab(&fixture.uri())
        .expect("the fixture is a folder");
    wait_until("the first tab to open Documents", || {
        tab_uris(&test)[0] == fixture.uri_of("Documents")
    });
    assert_eq!(tab_uris(&test)[1], fixture.uri(), "the tab in front stays");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));

    test.wait_for_listing("the second tab");
    test.window.activate_item(test.position_of("Documents"));
    test.window.navigate_or_report(&fixture.uri_of("Documents"));
    test.window.navigate_or_report(&fixture.uri());
    wait_for(std::time::Duration::from_millis(300));
    assert_eq!(
        tab_uris(&test)[1],
        fixture.uri(),
        "a tab that moved on drops the result"
    );
}

/// Open in Terminal says in the message line why a location has no
/// terminal, such as a server listing, and starts nothing.
///
/// parity: OPEN-017
#[gtk::test]
fn open_in_terminal_explains_a_refusal_in_the_message_line() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("open-in-terminal-of", Some("smb://studio-nas/"));

    wait_until("the refusal", || {
        test.window.shown_message()
            == "Open a network share first. A server listing is not a terminal directory."
    });
}

/// "Open in new tab" of a folder, a pin, a network entry or a share card
/// opens that folder in a new tab in front.
///
/// parity: OPEN-016
#[gtk::test]
fn open_in_new_tab_opens_the_folder_in_a_tab_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("open-tab", Some(&fixture.uri_of("Documents")));

    assert_eq!(tab_uris(&test), [fixture.uri(), fixture.uri_of("Documents")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// Enter on several items opens each: folders in background tabs and
/// files in their applications; more than five are asked about first.
///
/// parity: OPEN-003
#[gtk::test]
fn enter_on_several_items_opens_each_and_asks_for_many() {
    let fixture = Fixture::standard();
    for name in ["A", "B", "C"] {
        fs::write(fixture.path(name), "").expect("fixture file");
    }
    let test = TestWindow::open(&fixture.uri());

    select_names(&test, &["Documents", "Notes 2.txt", "Résumé.txt"]);
    // Enter on the focused row is the column view's activate signal.
    let column_view = test.window.folder_pane().details().column_view();
    column_view.emit_by_name::<()>("activate", &[&test.position_of("Notes 2.txt")]);
    wait_until("both files to open", || {
        test.context.recorded_launches().len() == 2
    });

    assert_eq!(tab_uris(&test), [fixture.uri(), fixture.uri_of("Documents")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()), "the tab stays");
    select_names(
        &test,
        &["Notes 2.txt", "Notes 10.txt", "Résumé.txt", "A", "B", "C"],
    );
    test.activate("open", None);
    let dialog = open_dialog(&test);
    assert_eq!(dialog.message_text(), "Are you sure you want to open 6 items?");
    dialog.press("Cancel");
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(test.context.recorded_launches().len(), 2, "nothing more opened");
}

/// An executable script opens in its application; with "Ask whether to
/// run programs and scripts" on, opening it asks first: Open opens it
/// without running it, Cancel does nothing and Run runs it. An executable
/// text file is not a program and opens without asking.
///
/// parity: OPEN-008
#[gtk::test]
fn opening_a_script_asks_to_run_it_only_when_the_settings_say_so() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::standard();
    let script = fixture.path("tidy.sh");
    fs::write(&script, "#!/bin/sh\ntouch ran\n").expect("fixture script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
    let test = TestWindow::open(&fixture.uri());
    test.context.record_runs();

    test.window.activate_item(test.position_of("tidy.sh"));
    wait_until("the script to open", || {
        test.context.recorded_launches().len() == 1
    });

    let update = PreferencesUpdate {
        ask_to_run_programs: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.ask_to_run_programs
    });
    test.window.activate_item(test.position_of("tidy.sh"));
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Run this program?");
    assert_eq!(dialog.button_labels(), ["Cancel", "Open", "Run"]);
    dialog.press("Open");
    wait_until("the script to open again", || {
        test.context.recorded_launches().len() == 2
    });
    assert!(!fixture.path("ran").exists(), "nothing ran");

    test.window.activate_item(test.position_of("tidy.sh"));
    open_dialog(&test).press("Cancel");
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(
        test.context.recorded_launches().len(),
        2,
        "Cancel opens and runs nothing"
    );
    assert!(test.context.recorded_runs().is_empty());

    test.window.activate_item(test.position_of("tidy.sh"));
    open_dialog(&test).press("Run");
    wait_until("the script to run", || !test.context.recorded_runs().is_empty());
    assert_eq!(test.context.recorded_runs(), [fixture.uri_of("tidy.sh")]);
    assert_eq!(test.context.recorded_launches().len(), 2, "Run does not open it");

    // On FAT, NTFS and SMB mounts every file may be executed; a text
    // file there opens without the question.
    let notes = fixture.path("Notes 2.txt");
    fs::set_permissions(&notes, fs::Permissions::from_mode(0o755)).expect("executable");
    test.window.activate_item(test.position_of("Notes 2.txt"));
    wait_until("the text file to open", || {
        test.context.recorded_launches().len() == 3
    });
    assert_eq!(test.context.recorded_launches()[2], fixture.uri_of("Notes 2.txt"));
}

/// Shift+F4 opens a terminal in the folder shown and Shift+Alt+F4 one per
/// folder of the selection, a file standing for its folder; more than
/// five are asked about first, and Cancel opens none.
///
/// parity: OPEN-021
#[gtk::test]
fn open_terminal_here_opens_one_per_folder_and_asks_for_many() {
    let fixture = Fixture::standard();
    let names = ["A", "B", "C", "D", "E", "F"];
    for name in names {
        fs::create_dir(fixture.path(name)).expect("fixture subfolder");
    }
    let test = TestWindow::open(&fixture.uri());
    let keys = |action: &str| application().accels_for_action(&format!("win.{action}"));
    assert_eq!(keys("open-terminal"), ["<Shift>F4", "<Shift><Control>F4"]);
    assert_eq!(keys("open-terminal-here"), ["<Shift><Alt>F4"]);

    select_names(&test, &["Notes 2.txt", "Résumé.txt", "Documents"]);
    assert_eq!(
        test.window.terminal_folders(),
        [fixture.uri_of("Documents"), fixture.uri()]
    );

    select_names(&test, &names);
    test.activate("open-terminal-here", None);
    let dialog = open_dialog(&test);
    assert_eq!(
        dialog.message_text(),
        "Are you sure you want to open 6 terminals?"
    );
    dialog.press("Cancel");
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(test.window.shown_message(), "", "no terminal was started");
}

/// Opening a `.desktop` link to a folder browses that folder.
///
/// parity: OPEN-009
#[gtk::test]
fn a_desktop_link_to_a_folder_opens_the_folder() {
    let fixture = Fixture::standard();
    let link = format!(
        "[Desktop Entry]\nType=Link\nName=Documents\nURL={}\n",
        fixture.uri_of("Documents")
    );
    fs::write(fixture.path("Documents link.desktop"), link).expect("the fixture is writable");
    let test = TestWindow::open(&fixture.uri());

    test.window
        .activate_item(test.position_of("Documents link.desktop"));

    wait_until("the linked folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    assert!(
        test.context.recorded_launches().is_empty(),
        "the link file is not opened"
    );
}

/// Compare files hands the two selected files to the installed comparison
/// tool, or says that none is installed; the item menu offers it for two
/// files when a tool is there.
///
/// parity: OPEN-023
#[gtk::test]
fn compare_files_hands_two_files_to_the_comparison_tool() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    install_test_application("org.gnome.Meld.desktop");

    test.activate("compare-files", None);

    assert!(Tool::Diff.installed().is_some());
    let pair = format!(
        "{} {}",
        fixture.uri_of("Notes 2.txt"),
        fixture.uri_of("Notes 10.txt")
    );
    assert_eq!(test.context.recorded_launches(), [pair]);
}

/// Ctrl+Shift+F opens the installed search tool at the folder shown, or
/// says that none is installed.
///
/// parity: OPEN-024
#[gtk::test]
fn ctrl_shift_f_opens_the_search_tool_at_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let keys = application().accels_for_action("win.search-tool");
    assert_eq!(keys, ["<Shift><Control>f"]);
    install_test_application("org.kde.kfind.desktop");

    test.activate("search-tool", None);

    assert_eq!(test.context.recorded_launches(), [fixture.uri()]);
}

/// When no application opens a file's type, "Could not open the item"
/// offers to find one in Software, while Software is installed.
///
/// parity: OPEN-010
#[gtk::test]
fn a_file_without_an_application_offers_a_search_in_software() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let mut entry = crate::test_support::file_entry("scene.blend");
    entry.content_type = Some("application/x-blender".to_owned());
    let reason = ox_core::integration::OpenError::NoApplication.to_string();
    install_test_application("org.gnome.Software.desktop");
    let software = FakeSoftware::start();

    test.window.report_open_failure(&reason, &entry);

    let dialog = open_dialog(&test);
    assert_eq!(dialog.message_text(), reason);
    dialog.press("Find an app in Software");
    wait_until("Software to be asked", || !software.calls.borrow().is_empty());
    let search = ("search".to_owned(), vec!["application/x-blender".to_owned()]);
    assert_eq!(*software.calls.borrow(), [search]);
}

/// Adds a desktop entry `desktop_id` to the private data folder the tests
/// run with (native/tools/check.py), so a tool is installed whatever the
/// host has, and waits until GIO lists it.
fn install_test_application(desktop_id: &str) {
    let data = glib::user_data_dir();
    assert!(
        data.starts_with(std::env::temp_dir()),
        "tests add applications only to a private data folder"
    );
    let folder = data.join("applications");
    fs::create_dir_all(&folder).expect("the data folder is writable");
    let entry = "[Desktop Entry]\nType=Application\nName=Test tool\nExec=true %U\nNoDisplay=true\n";
    fs::write(folder.join(desktop_id), entry).expect("the data folder is writable");
    wait_until("GIO to list the application", || {
        installed_application(desktop_id).is_some()
    });
}

/// An action and its string parameters.
type ActionCall = (String, Vec<String>);

/// GNOME Software's `org.freedesktop.Application` interface on the
/// private session bus, recording each `ActivateAction` call.
struct FakeSoftware {
    /// Each call, in order.
    calls: Rc<RefCell<Vec<ActionCall>>>,
    owner: Option<gio::OwnerId>,
    registration: Option<gio::RegistrationId>,
    connection: gio::DBusConnection,
}

const APPLICATION_XML: &str = "<node><interface name='org.freedesktop.Application'>\
    <method name='ActivateAction'><arg type='s' direction='in'/><arg type='av' direction='in'/>\
    <arg type='a{sv}' direction='in'/></method></interface></node>";

impl FakeSoftware {
    fn start() -> Self {
        let connection =
            gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).expect("a private session bus");
        let node = gio::DBusNodeInfo::for_xml(APPLICATION_XML).expect("the interface XML is valid");
        let interface = node
            .lookup_interface("org.freedesktop.Application")
            .expect("the XML declares the interface");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&calls);
        let registration = connection
            .register_object("/org/gnome/Software", &interface)
            .method_call(move |_, _, _, _, _, parameters, invocation| {
                let action = parameters.child_value(0).str().unwrap_or_default().to_owned();
                let arguments = parameters.child_value(1);
                let strings = arguments
                    .iter()
                    .filter_map(|argument| argument.as_variant()?.str().map(str::to_owned))
                    .collect();
                recorded.borrow_mut().push((action, strings));
                invocation.return_value(None);
            })
            .build()
            .expect("the object path is free");
        let owned = Rc::new(std::cell::Cell::new(false));
        let is_owned = Rc::clone(&owned);
        let owner = gio::bus_own_name_on_connection(
            &connection,
            "org.gnome.Software",
            gio::BusNameOwnerFlags::REPLACE,
            move |_, _| is_owned.set(true),
            |_, _| {},
        );
        wait_until("the bus name", || owned.get());
        Self {
            calls,
            owner: Some(owner),
            registration: Some(registration),
            connection,
        }
    }
}

impl Drop for FakeSoftware {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            let _ = self.connection.unregister_object(registration);
        }
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
    }
}
