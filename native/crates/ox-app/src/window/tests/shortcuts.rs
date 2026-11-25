// SPDX-License-Identifier: AGPL-3.0-only
//! Opening shortcuts as Windows Explorer opens them: a shortcut goes to
//! its target, under the target's own address and name, and a broken one
//! asks "Problem with Shortcut" and offers to delete it.

use std::fs;
use std::os::unix::fs::symlink;

use gtk::prelude::*;

use crate::test_support::harness::descendants;
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
use crate::window::tests::file_ops_support::{
    dialog_over, open_dialog, require_private_trash, wait_for_no_dialog,
};

/// Opening a shortcut to a folder goes to the folder itself, as Windows
/// does, not into the shortcut's own path: the address bar and Up then
/// lead from where the folder really is, Up selecting the folder.
///
/// parity: OPEN-027
#[gtk::test]
fn a_folder_shortcut_opens_the_folder_at_its_own_address() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Desktop")).expect("fixture folder");
    symlink(
        fixture.path("Documents"),
        fixture.path("Desktop/Documents - Shortcut"),
    )
    .expect("link");
    let test = TestWindow::open(&fixture.uri_of("Desktop"));

    test.window
        .activate_item(test.position_of("Documents - Shortcut"));

    wait_until("the folder the shortcut points to", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents")) && !test.window.is_loading()
    });

    // Up goes to the folder's own parent, with the folder selected, as in
    // Windows 11 (checked in a Windows 11 VM).
    test.activate("up", None);
    wait_until("the folder's parent", || {
        test.window.current_uri() == Some(fixture.uri()) && !test.window.is_loading()
    });
    wait_until("the folder to be selected", || {
        test.selected_names() == ["Documents"]
    });
}

/// Opening a shortcut to a file hands the application the file itself,
/// whose name tells its type, not the shortcut's path.
///
/// parity: OPEN-027
#[gtk::test]
fn a_file_shortcut_opens_the_file_itself() {
    let fixture = Fixture::standard();
    symlink("../Notes 2.txt", fixture.path("Documents/Notes 2.txt - Shortcut")).expect("link");
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.context.record_runs();

    test.window
        .activate_item(test.position_of("Notes 2.txt - Shortcut"));

    wait_until("the file to open", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 2.txt")]);
}

/// Presses the letter `key` alone in the message box `dialog`, through its
/// shortcuts, as Windows' message box takes Y and N: whether one took it.
fn press_letter(dialog: &crate::dialog::Dialog, key: gtk::gdk::Key) -> bool {
    let frame = descendants::<crate::dialog::DialogFrame>(dialog)
        .into_iter()
        .next()
        .expect("the dialog has its frame");
    frame
        .observe_controllers()
        .iter::<gtk::glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            (0..controller.n_items())
                .filter_map(|position| controller.item(position).and_downcast::<gtk::Shortcut>())
                .collect::<Vec<_>>()
        })
        .filter(|shortcut| {
            shortcut
                .trigger()
                .and_downcast::<gtk::KeyvalTrigger>()
                .is_some_and(|trigger| trigger.keyval() == key && trigger.modifiers().is_empty())
        })
        .any(|shortcut| {
            shortcut
                .action()
                .is_some_and(|action| action.activate(gtk::ShortcutActionFlags::empty(), &frame, None))
        })
}

/// Whether an item called `name` is in the test run's private Recycle Bin.
fn in_recycle_bin(name: &str) -> bool {
    gtk::glib::user_data_dir()
        .join("Trash/files")
        .join(name)
        .symlink_metadata()
        .is_ok()
}

/// A shortcut whose item was deleted asks "Problem with Shortcut", in
/// Windows' words, and offers to delete it: No keeps it; Yes, the default,
/// deletes the shortcut, and only the shortcut, permanently, as Windows 11
/// does (checked in a Windows 11 VM). A loop of links is broken the same
/// way.
///
/// parity: OPEN-028
#[gtk::test]
fn a_broken_shortcut_offers_to_delete_it() {
    require_private_trash();
    let fixture = Fixture::standard();
    symlink(
        fixture.path("Old plan.txt"),
        fixture.path("Old plan.txt - Shortcut"),
    )
    .expect("link");
    symlink("self loop", fixture.path("self loop")).expect("loop");
    let test = TestWindow::open(&fixture.uri());
    test.context.record_runs();

    test.window
        .activate_item(test.position_of("Old plan.txt - Shortcut"));
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Problem with Shortcut");
    assert_eq!(
        dialog.message_text(),
        "The item ‘Old plan.txt’ that this shortcut refers to has been changed or moved, so this \
         shortcut will no longer work properly.\n\nDo you want to delete this shortcut?"
    );
    assert_eq!(dialog.button_labels(), ["Yes", "No"]);
    dialog.press("No");
    wait_for(std::time::Duration::from_millis(200));
    assert!(
        fs::symlink_metadata(fixture.path("Old plan.txt - Shortcut")).is_ok(),
        "No keeps the shortcut"
    );

    test.window
        .activate_item(test.position_of("Old plan.txt - Shortcut"));
    open_dialog(&test).press_primary();
    wait_until("the shortcut to be deleted", || {
        fs::symlink_metadata(fixture.path("Old plan.txt - Shortcut")).is_err()
    });
    assert!(
        !in_recycle_bin("Old plan.txt - Shortcut"),
        "deleted, not moved to the Recycle Bin"
    );
    wait_until("the shortcut to leave the list", || {
        !test.names().contains(&"Old plan.txt - Shortcut".to_owned())
    });

    // Y alone answers Yes, as in Windows' message box (checked in a
    // Windows 11 VM); N alone answers No.
    test.window.activate_item(test.position_of("self loop"));
    let looped = open_dialog(&test);
    assert_eq!(looped.title_text(), "Problem with Shortcut");
    assert!(press_letter(&looped, gtk::gdk::Key::n), "N answers No");
    wait_for(std::time::Duration::from_millis(200));
    assert!(
        fs::symlink_metadata(fixture.path("self loop")).is_ok(),
        "No keeps it"
    );
    test.window.activate_item(test.position_of("self loop"));
    assert!(
        press_letter(&open_dialog(&test), gtk::gdk::Key::y),
        "Y answers Yes"
    );
    wait_until("the loop to be deleted", || {
        fs::symlink_metadata(fixture.path("self loop")).is_err()
    });
    assert!(test.context.recorded_launches().is_empty(), "nothing opened");
}

/// A shortcut onto a drive that is not plugged in says that the drive is
/// unavailable, as Windows says it, and offers nothing to delete: the
/// shortcut works again once the drive is back.
///
/// parity: OPEN-028
#[gtk::test]
fn a_shortcut_onto_a_drive_that_is_not_connected_says_so() {
    let fixture = Fixture::standard();
    symlink(
        "/media/fictional-user/BACKUP-USB/Photos",
        fixture.path("Photos - Shortcut"),
    )
    .expect("link");
    let test = TestWindow::open(&fixture.uri());

    test.window.activate_item(test.position_of("Photos - Shortcut"));
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Problem with Shortcut");
    assert_eq!(
        dialog.message_text(),
        "The drive or network connection that the shortcut ‘Photos - Shortcut’ refers to is \
         unavailable. Make sure that the disk is properly inserted or the network resource is \
         available, and then try again."
    );
    assert_eq!(dialog.button_labels(), ["OK"]);
    dialog.press("OK");
    assert!(fs::symlink_metadata(fixture.path("Photos - Shortcut")).is_ok());
}

/// The next page of the wizard: a dialog over the window other than
/// `page`.
fn next_page(test: &TestWindow, page: &crate::dialog::Dialog) -> crate::dialog::Dialog {
    wait_until("the next page", || {
        dialog_over(test).is_some_and(|dialog| &dialog != page)
    });
    dialog_over(test).expect("the next page shows")
}

/// New ▸ Shortcut is Windows' Create Shortcut wizard: the location, which
/// must exist, then the name, which Windows proposes and which must be
/// free; Back keeps what was typed; Finish makes the shortcut and selects
/// it.
///
/// parity: OPS-004, OPS-051
#[gtk::test]
fn new_shortcut_is_the_create_shortcut_wizard() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("new-link", None);
    let location = open_dialog(&test);
    assert_eq!(location.title_text(), "Create Shortcut");
    assert_eq!(
        location.message_text(),
        "What item would you like to create a shortcut for?\nThis wizard helps you to create shortcuts \
         to local or network programs, files, folders, computers, or Internet addresses."
    );
    assert_eq!(location.button_labels(), ["Cancel", "Next"]);
    let browse = descendants::<gtk::Button>(&location)
        .into_iter()
        .any(|button| button.label().as_deref() == Some("Browse…"));
    assert!(browse, "a Browse… button beside the location");
    let field = descendants::<gtk::Entry>(&location)[0].clone();
    field.set_text("/no/such/place");
    location.press("Next");
    wait_until("the refusal", || location.error_text().is_some());

    let notes = fixture.path("Notes 2.txt");
    field.set_text(&format!("\"{}\"", notes.display()));
    location.press("Next");
    let name = next_page(&test, &location);
    assert_eq!(name.title_text(), "Create Shortcut");
    assert!(
        name.message_text()
            .starts_with("What would you like to name the shortcut?"),
        "{}",
        name.message_text()
    );
    assert_eq!(name.button_labels(), ["Back", "Cancel", "Finish"]);
    let name_field = descendants::<gtk::Entry>(&name)[0].clone();
    assert_eq!(
        name_field.text(),
        "Notes 2.txt",
        "Windows proposes the item's name"
    );

    name.press("Back");
    let again = next_page(&test, &name);
    assert_eq!(
        descendants::<gtk::Entry>(&again)[0].text(),
        format!("\"{}\"", notes.display()),
        "Back keeps the location"
    );
    again.press("Next");
    let name = next_page(&test, &again);
    name.press("Finish");
    wait_until("the refusal of a taken name", || name.error_text().is_some());
    descendants::<gtk::Entry>(&name)[0].set_text("Notes shortcut");
    name.press("Finish");
    wait_for_no_dialog(&test);

    wait_until("the shortcut to be selected", || {
        test.selected_names() == ["Notes shortcut"]
    });
    assert_eq!(
        fs::read_link(fixture.path("Notes shortcut")).expect("a link"),
        notes
    );
    assert_eq!(
        fs::read(&notes).expect("the file"),
        fs::read(fixture.path("Notes shortcut")).expect("followed")
    );
}

/// Opening an installed app's launcher, or a shortcut to one, opens it in
/// its editor, as OPEN-007 keeps it, until Settings › "Run installed apps'
/// launchers on double-click" is on; then it starts the app, as Windows
/// runs a shortcut to a program. A launcher from elsewhere, such as one
/// downloaded, still asks the trust question, and Cancel runs nothing.
///
/// parity: OPEN-029
#[gtk::test]
fn installed_launchers_run_on_open_only_when_chosen() {
    use std::os::unix::fs::PermissionsExt;

    use ox_core::settings::{PreferencesUpdate, Settings};

    let fixture = Fixture::standard();
    let applications = gtk::glib::user_data_dir().join("applications");
    assert!(
        applications.starts_with(std::env::temp_dir()),
        "tests install launchers only in a private data folder"
    );
    fs::create_dir_all(&applications).expect("the data folder is writable");
    let launcher = applications.join("fictional-calculator.desktop");
    let ran = fixture.path("ran");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Fictional Calculator\nExec=touch \"{}\"\n",
        ran.display()
    );
    fs::write(&launcher, &entry).expect("the data folder is writable");
    symlink(&launcher, fixture.path("Calculator - Shortcut")).expect("link");
    let downloaded = fixture.path("Free Game.desktop");
    fs::write(&downloaded, entry.replace("Fictional Calculator", "Free Game")).expect("the fixture");
    fs::set_permissions(&downloaded, fs::Permissions::from_mode(0o755)).expect("executable");
    let test = TestWindow::open(&fixture.uri());
    test.context.record_runs();

    test.window
        .activate_item(test.position_of("Calculator - Shortcut"));
    wait_until("the launcher to open in its editor", || {
        !test.context.recorded_launches().is_empty()
    });
    assert!(test.context.recorded_runs().is_empty(), "off: nothing runs");

    let update = PreferencesUpdate {
        run_installed_launchers: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.run_installed_launchers
    });

    test.window
        .activate_item(test.position_of("Calculator - Shortcut"));
    wait_until("the installed app to start", || {
        !test.context.recorded_runs().is_empty()
    });
    assert_eq!(
        test.context.recorded_runs(),
        [ox_core::location::file_uri(&launcher)]
    );
    assert!(
        dialog_over(&test).is_none(),
        "an installed app is not asked about"
    );

    test.window.activate_item(test.position_of("Free Game.desktop"));
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Run this program?");
    assert_eq!(question.button_labels(), ["Cancel", "Trust and Launch"]);
    question.press("Cancel");
    wait_for(std::time::Duration::from_millis(300));
    assert_eq!(test.context.recorded_runs().len(), 1, "Cancel runs nothing");
    fs::remove_file(&launcher).expect("the private launcher is removed");
}
