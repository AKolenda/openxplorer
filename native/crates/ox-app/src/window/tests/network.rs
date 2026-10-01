// SPDX-License-Identifier: AGPL-3.0-only
//! Network shares and devices in the window: Discover servers on the
//! Network page, Map network location, Sign out of server, Keep in
//! Network and Remove saved location, This PC's network and drive cards,
//! and connecting and removing drives, against `renderNetwork`,
//! `connectDialog`, `signOut`, `networkLocationMenu`, `mountVolume` and
//! `unmount` in `desktop/ui/app.js`.
//!
//! No test mounts anything or reaches a network: discovery finds what the
//! test gives it, the keyring is in memory, and each mount or removal
//! fails on validation or for want of a mount before GIO is asked.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use ox_core::network::{
    begin_sign_out, DiscoveredServer, Discovery, ForgetScope, MountOutcome, NetworkError, SignOutRequest,
    WriteActivity, KEYRING_SAVE_NOTICE,
};

use crate::folder_view::item::FileItem;
use crate::locations::Page;
use crate::network::Discoverer;
use crate::test_support::file_entry;
use crate::test_support::harness::{
    capture, capture_dialog, descendants, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::session::TabPlacement;
use crate::window::BrowserWindow;
use crate::window::Dialog;

/// The labels `widget` shows, in order.
pub(super) fn texts_in(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    let labels = descendants::<gtk::Label>(widget);
    let shown = labels.iter().filter(|label| label.is_mapped());
    shown.map(|label| label.text().to_string()).collect()
}

/// Whether `widget` shows `text`.
pub(super) fn shows(widget: &impl IsA<gtk::Widget>, text: &str) -> bool {
    texts_in(widget).iter().any(|shown| shown == text)
}

/// The open network dialog, once it is shown.
pub(super) fn open_form_dialog() -> Dialog {
    let find = || {
        let toplevels = gtk::Window::list_toplevels().into_iter();
        let dialogs = toplevels.filter_map(|toplevel| toplevel.downcast::<Dialog>().ok());
        dialogs.filter(WidgetExt::is_visible).last()
    };
    wait_until("the dialog", || find().is_some());
    find().expect("the dialog is open")
}

/// The message box headed `title`, once it is shown, and its texts. OK
/// closes the box before this returns.
fn message_box_texts(title: &str) -> Vec<String> {
    let find = || {
        let toplevels = gtk::Window::list_toplevels().into_iter();
        let windows = toplevels.filter_map(|toplevel| toplevel.downcast::<gtk::Window>().ok());
        let messages = windows.filter(|window| !window.is::<BrowserWindow>() && window.is_visible());
        messages.into_iter().find(|window| shows(window, title))
    };
    wait_until(title, || find().is_some());
    let message = find().expect("the message box is open");
    let texts = texts_in(&message);
    let buttons = descendants::<gtk::Button>(&message);
    let ok = buttons
        .iter()
        .find(|button| button.label().as_deref() == Some("OK"));
    ok.expect("the message box has OK").emit_clicked();
    texts
}

/// The server `studio-nas` as discovery reports it.
fn studio_nas() -> DiscoveredServer {
    DiscoveredServer {
        uri: "smb://studio-nas/".into(),
        label: "studio-nas".into(),
        host: "studio-nas".into(),
    }
}

/// The replies a mount operation sent to `GVfs`, oldest first.
type Replies = Rc<RefCell<Vec<gtk::gio::MountOperationResult>>>;

/// `GVfs` asks the window's prompts for a password to mount
/// `smb://nas/share`; returns the operation, its recorded replies, and the
/// dialog the window shows.
fn ask_window_for_password(
    test: &TestWindow,
) -> (gtk::gio::MountOperation, Replies, crate::dialogs::SignInDialog) {
    let operation = test
        .window
        .network()
        .prompts()
        .create("smb://nas/share")
        .expect("an SMB share");
    let replies = Replies::default();
    let recorded = Rc::clone(&replies);
    operation.connect_reply(move |_, result| recorded.borrow_mut().push(result));
    let flags = gtk::gio::AskPasswordFlags::NEED_USERNAME
        | gtk::gio::AskPasswordFlags::NEED_PASSWORD
        | gtk::gio::AskPasswordFlags::SAVING_SUPPORTED;
    operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
    let sign_in = Rc::clone(test.window.network().sign_in());
    wait_until("the sign-in dialog", || sign_in.shown_dialog().is_some());
    let dialog = sign_in.shown_dialog().expect("the sign-in dialog");
    (operation, replies, dialog)
}

/// A credential that worked is kept for the window; when the keyring
/// cannot save it, the window says so.
///
/// parity: NET-015
#[gtk::test]
fn a_sign_in_the_keyring_cannot_keep_is_announced() {
    let test = TestWindow::open(Page::Network.uri());
    let (operation, _, dialog) = ask_window_for_password(&test);
    dialog.type_account("sam", "not-a-real-password");
    dialog.press_connect();

    test.window
        .network()
        .prompts()
        .finish(&operation, MountOutcome::Mounted);

    wait_until("the keyring notice", || {
        test.window.shown_message().as_str() == KEYRING_SAVE_NOTICE
    });
}

/// Closing a window cancels its open sign-ins, which aborts their mounts.
///
/// parity: NET-012, TAB-050
#[gtk::test]
fn closing_the_window_aborts_its_sign_ins() {
    let test = TestWindow::open(Page::Network.uri());
    let (operation, replies, _) = ask_window_for_password(&test);

    test.window.close();
    wait_until("the aborted mount", || !replies.borrow().is_empty());

    assert_eq!(*replies.borrow(), [gtk::gio::MountOperationResult::Aborted]);
    assert_eq!(operation.password(), None, "no password stays on the operation");
}

/// The first visit of Network discovers servers, shows them with their
/// count, and offers Stop while more passes follow.
///
/// Ported from `desktop/tests/ui_release.py::Network discovery populates sample servers`
///
/// parity: HOME-006, HOME-007, HOME-008, NET-024
#[gtk::test]
fn the_network_page_discovers_servers_on_its_first_visit() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let found = Discovery {
        servers: vec![studio_nas()],
        warnings: Vec::new(),
    };
    test.context.network().set_discoverer(Discoverer::finding(found));

    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    let landing = test.window.folder_pane().landing();
    wait_until("the discovered server", || shows(landing, "SMB · Discovered"));

    for expected in [
        "studio-nas",
        "\\\\studio-nas\\",
        "1",
        "Stop",
        "Listening for advertised SMB servers…",
    ] {
        assert!(shows(landing, expected), "{expected} in {:?}", texts_in(landing));
    }
    let card = descendants::<gtk::Button>(landing)
        .into_iter()
        .find(|button| button.has_css_class("discovered-server"))
        .expect("the server's card");
    let target = card
        .action_target_value()
        .and_then(|target| target.get::<String>());
    assert_eq!(
        target.as_deref(),
        Some("smb://studio-nas/"),
        "the card opens the server"
    );
    test.activate("stop-discovery", None);
    assert!(shows(landing, "Discover servers"));
    assert!(shows(landing, "Discover devices without scanning their files."));
    assert!(shows(landing, "studio-nas"), "Stop keeps what was found");
}

/// Map network location opens from every place that offers it; an
/// address that is not a share is refused inside the dialog, which stays
/// open for another try, and Cancel closes it.
///
/// parity: NET-001, HOME-009
#[gtk::test]
fn map_network_location_keeps_its_errors_inside_the_dialog() {
    let test = TestWindow::open(Page::Network.uri());
    assert!(test
        .window
        .lookup_action("map-network-location")
        .is_some_and(|action| action.is_enabled()));

    test.activate("map-network-location", None);
    let dialog = open_form_dialog();
    let entries = descendants::<gtk::Entry>(&dialog);
    entries[0].set_text("/home/demo");
    dialog.press_primary();
    wait_until("the error", || dialog.error_text().is_some());

    assert!(dialog.is_visible());
    assert!(dialog.can_press("Connect"), "Connect can be pressed again");
    dialog.press("Cancel");
    wait_until("Cancel to close the dialog", || !dialog.is_visible());
    assert_eq!(test.window.current_uri().as_deref(), Some(Page::Network.uri()));
}

/// A share that is not mounted is mounted once and listed again; a
/// second "not mounted" is reported, never mounted again, and a failed
/// mount is reported instead of the listing's error. The shares are on a
/// host that does not exist, which `GVfs` reports as not mounted without
/// reaching a network, and the mount is the test's.
///
/// parity: NET-004, NET-003, OPS-037
#[gtk::test]
fn an_unmounted_share_is_mounted_once_then_listed_again() {
    let test = TestWindow::open(Page::Network.uri());
    let mounts = Rc::new(Cell::new(0));
    let counted = Rc::clone(&mounts);
    test.window.network().answer_mounts_with(move || {
        counted.set(counted.get() + 1);
        Ok(())
    });

    test.window
        .navigate("smb://example.invalid/share")
        .expect("an SMB share");
    wait_until("the listing after the mount", || {
        mounts.get() == 1 && test.window.load_error().is_some()
    });
    test.wait_for_listing("the second listing");
    assert_eq!(mounts.get(), 1, "mounted once");

    test.window
        .network()
        .answer_mounts_with(|| Err(NetworkError::NotAFolder));
    test.window
        .navigate("smb://example.invalid/other")
        .expect("an SMB share");
    wait_until("the failed mount", || {
        test.window.load_error().as_deref() == Some("This location is not a folder.")
    });
}

/// Measuring a share that is not mounted mounts it once and measures it
/// again; a failed mount is what the folder's size reports.
///
/// parity: PROP-029, PROP-030
#[gtk::test]
fn an_unmounted_share_is_mounted_once_before_its_size_is_measured() {
    use crate::properties::FolderSizeState;

    let test = TestWindow::open(Page::Network.uri());
    let mounts = Rc::new(Cell::new(0));
    let counted = Rc::clone(&mounts);
    test.window.network().answer_mounts_with(move || {
        counted.set(counted.get() + 1);
        Err(NetworkError::NotAFolder)
    });
    let share = "smb://example.invalid/share";

    test.activate("calculate-folder-size-of", Some(share));

    wait_until("the measured share", || {
        matches!(
            test.window.measured_folder_size(share),
            Some(FolderSizeState::Unavailable(_))
        )
    });
    assert_eq!(mounts.get(), 1, "mounted once");
    assert_eq!(
        test.window.measured_folder_size(share),
        Some(FolderSizeState::Unavailable(
            "This location is not a folder.".to_owned()
        ))
    );
}

/// While a server is signed out, it is neither listed nor connected.
///
/// parity: NET-023
#[gtk::test]
fn a_server_being_signed_out_is_neither_listed_nor_mapped() {
    let test = TestWindow::open(Page::Network.uri());
    let registry = test.context.network().sign_out_registry();
    let request = SignOutRequest {
        uri: "smb://nas/share",
        forget: ForgetScope::AllScopes,
        writes: WriteActivity::Idle,
    };
    let signing_out =
        begin_sign_out(test.window.network().prompts(), &registry, request).expect("sign-out starts");

    test.window
        .navigate("smb://nas/share/Reports")
        .expect("an SMB folder");
    test.wait_for_listing("the refused listing");
    assert_eq!(
        test.window.load_error().as_deref(),
        Some("This server is being signed out. Reopen it after sign-out finishes.")
    );
    test.activate("map-network-location", None);
    let dialog = open_form_dialog();
    descendants::<gtk::Entry>(&dialog)[0].set_text("\\\\nas\\Projects");
    dialog.press_primary();
    wait_until("the error", || dialog.error_text().is_some());
    assert_eq!(
        dialog.error_text().as_deref(),
        Some("Sign-out is in progress. Reconnect after it finishes.")
    );
    dialog.press("Cancel");
    drop(signing_out);
}

/// Sign out keeping the saved credentials: the server leaves Network, its
/// tabs list again when shown, and the window goes to Network.
///
/// parity: NET-020, NET-021
#[gtk::test]
fn signing_out_forgets_the_server_and_its_tabs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.context().remember_network("smb://nas/share");
    test.window
        .open_tab("smb://nas/share", TabPlacement::Background)
        .expect("an SMB share");
    wait_until("the share in the sidebar", || {
        test.window.sidebar().labels().contains(&"share".to_owned())
    });

    test.activate("sign-out", Some("smb://nas/share"));
    let dialog = open_form_dialog();
    assert!(shows(&dialog, "Sign out of nas?"));
    descendants::<gtk::CheckButton>(&dialog)[0].set_active(false);
    dialog.press_primary();
    wait_until("the Network page", || {
        test.window.current_uri().as_deref() == Some(Page::Network.uri())
    });

    assert_eq!(
        test.window.shown_message().as_str(),
        "Disconnected. Saved credentials are retained."
    );
    assert!(!test.window.sidebar().labels().contains(&"share".to_owned()));
    let tabs = test.tab_listing_needs();
    assert!(tabs.contains(&("smb://nas/share".to_owned(), true)), "{tabs:?}");
}

/// Signing out of the server the window shows drops the rows on screen
/// without the window's own handlers finding the tabs still in use (the
/// release build aborted here: a `RefCell` borrowed twice).
///
/// parity: NET-020
#[gtk::test]
fn signing_out_of_the_server_on_screen_drops_its_rows() {
    let test = TestWindow::open(Page::Network.uri());
    test.window
        .network()
        .answer_mounts_with(|| Err(NetworkError::NotAFolder));
    test.window
        .navigate("smb://example.invalid/share")
        .expect("an SMB share");
    wait_until("the failed mount", || test.window.load_error().is_some());
    let tab = test.active_tab().expect("the share's tab");
    let rows = test.window.tab_store(tab).expect("the share's rows");
    rows.append(&FileItem::new(file_entry("Report.txt")));
    assert_eq!(
        test.window.folder_pane().model().n_items(),
        1,
        "the row is on screen"
    );

    test.activate("sign-out", Some("smb://example.invalid/share"));
    let dialog = open_form_dialog();
    descendants::<gtk::CheckButton>(&dialog)[0].set_active(false);
    dialog.press_primary();
    wait_until("the Network page", || {
        test.window.current_uri().as_deref() == Some(Page::Network.uri())
    });

    assert_eq!(rows.n_items(), 0);
    assert_eq!(
        test.window.shown_message().as_str(),
        "Disconnected. Saved credentials are retained."
    );
}

/// Forgetting saved credentials needs the keyring; without one the server
/// is disconnected and the user is told what remains.
///
/// parity: NET-021
#[gtk::test]
fn signing_out_without_a_keyring_says_the_credentials_remain() {
    let test = TestWindow::open(Page::Network.uri());

    test.activate("sign-out", Some("smb://nas/share"));
    open_form_dialog().press_primary();

    let texts = message_box_texts("Sign-out did not fully finish");
    let detail =
        "Disconnected, but saved credentials could not be removed. Make sure the system keyring is running \
                  and try Sign out again.";
    assert!(texts.iter().any(|text| text == detail), "{texts:?}");
}

/// Keep in Network saves a browsed share with its label and no
/// credentials; Remove saved location deletes only that entry.
///
/// Ported from `desktop/tests/ui_v07.py::Keep in Network explicitly persists location`
///
/// parity: NET-016, NET-017, SIDE-009, SIDE-019, SIDE-020
#[gtk::test]
fn keep_in_network_saves_a_browsed_share_and_remove_deletes_it() {
    let test = TestWindow::open(Page::Network.uri());
    test.window.context().remember_network("smb://nas/media");
    wait_until("the visited share under Network", || {
        test.window.sidebar().labels().contains(&"media".to_owned())
    });
    assert!(
        test.context.settings_data().shares.is_empty(),
        "browsing alone saves nothing"
    );

    test.activate("keep-in-network", Some("smb://nas/media"));
    wait_until("the saved share", || {
        !test.context.settings_data().shares.is_empty()
    });
    let saved = test.context.settings_data().shares;
    assert_eq!(
        (saved[0].uri.as_str(), saved[0].label.as_str()),
        ("smb://nas/media", "media")
    );

    test.activate("remove-saved-location", Some("smb://nas/media"));
    wait_until("the removal", || test.context.settings_data().shares.is_empty());
    assert_eq!(test.window.shown_message().as_str(), "Saved location removed.");
}

/// parity: SIDE-017, SIDE-020
#[gtk::test]
fn open_in_new_window_shows_the_place_in_another_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(Page::Network.uri());
    let before = gtk::Window::list_toplevels().len();

    test.activate("open-window", Some(&fixture.uri()));

    let others = || {
        let windows = gtk::Window::list_toplevels().into_iter();
        let browsers = windows.filter_map(|window| window.downcast::<BrowserWindow>().ok());
        browsers
            .filter(|window| window != &test.window)
            .collect::<Vec<_>>()
    };
    wait_until("the new window", || gtk::Window::list_toplevels().len() > before);
    let opened = others();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].current_uri(), Some(fixture.uri()));
    opened[0].close();
}

/// This PC lists saved locations with their state, or says how to add
/// one.
///
/// parity: HOME-005
#[gtk::test]
fn this_pc_lists_saved_locations_or_says_how_to_map_one() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = test.window.folder_pane().landing();
    let empty =
        "No network locations saved. Use “Map network location” to connect to a share and add it to the \
                 sidebar.";
    assert!(shows(landing, empty));
    assert!(shows(landing, "Map network location"));

    test.save_share("smb://nas/media", "Media (M:)");
    wait_until("the saved share's card", || shows(landing, "Media (M:)"));

    for expected in ["\\\\nas\\media", "Connect on open"] {
        assert!(shows(landing, expected), "{expected} in {:?}", texts_in(landing));
    }
    assert!(!shows(landing, empty));
}

/// Local Disk's card shows how full it is once GIO has measured it.
///
/// parity: HOME-003, HOME-004
#[gtk::test]
fn the_local_disk_card_shows_its_capacity() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = test.window.folder_pane().landing();

    wait_until("the capacity bar", || {
        descendants::<gtk::ProgressBar>(landing)
            .iter()
            .any(|bar| bar.has_css_class("capacity"))
    });

    assert!(shows(landing, "Local Disk"));
    let texts = texts_in(landing);
    assert!(texts.iter().any(|text| text.contains(" free of ")), "{texts:?}");
}

/// A volume that went away shows "Could not mount device".
///
/// parity: DEV-003, SIDE-016
#[gtk::test]
fn mounting_a_volume_that_went_away_says_so() {
    let test = TestWindow::open(Page::ThisPc.uri());

    test.activate("mount-volume", Some("no-such-volume"));

    let texts = message_box_texts("Could not mount device");
    assert!(
        texts
            .iter()
            .any(|text| text == "This volume is no longer available."),
        "{texts:?}"
    );
}

/// Disconnect asks first, naming the location; a location outside every
/// mount then shows "Could not disconnect".
///
/// parity: DEV-006
#[gtk::test]
fn disconnect_asks_first_and_reports_a_location_without_a_mount() {
    let test = TestWindow::open(Page::ThisPc.uri());

    test.activate("disconnect", Some("file:///media/demo/USB"));
    let dialog = open_form_dialog();
    let question = "/media/demo/USB\n\nClose files using this mount first. This disconnects the session mount for other \
                    applications too.";
    assert!(shows(&dialog, "Disconnect this mount?"));
    assert!(shows(&dialog, question), "{:?}", dialog.texts());
    dialog.press_primary();

    let texts = message_box_texts("Could not disconnect");
    assert!(
        texts
            .iter()
            .any(|text| text == "This location has no active user-session mount."),
        "{texts:?}"
    );
}

/// parity: DEV-007, DEV-008
#[gtk::test]
fn eject_and_safely_remove_report_a_location_without_a_mount() {
    let test = TestWindow::open(Page::ThisPc.uri());

    for (action, title) in [
        ("eject", "Could not eject"),
        ("safely-remove", "Could not safely remove"),
    ] {
        test.activate(action, Some("file:///media/demo/USB"));
        let texts = message_box_texts(title);
        assert!(
            texts
                .iter()
                .any(|text| text == "This location has no active user-session mount."),
            "{texts:?}"
        );
    }
}

/// Before a drive is taken away, the tab in front and the tabs behind it
/// that show a folder on it move to Home; other tabs stay.
///
/// parity: DEV-009
#[gtk::test]
fn the_tabs_on_a_drive_move_home_before_it_is_ejected() {
    let fixture = Fixture::standard();
    let home = ox_core::location::file_uri(&gtk::glib::home_dir());
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.window
        .open_tab(&fixture.uri(), TabPlacement::Background)
        .expect("a tab on the drive");
    test.window
        .open_tab(Page::ThisPc.uri(), TabPlacement::Background)
        .expect("a tab elsewhere");

    test.activate("eject", Some(&fixture.uri()));

    message_box_texts("Could not eject");
    let tab_uris: Vec<String> = {
        use gtk::subclass::prelude::*;
        let session = test.window.imp().session.borrow();
        session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
    };
    assert_eq!(test.window.current_uri(), Some(home.clone()));
    assert_eq!(tab_uris, [home.clone(), home, Page::ThisPc.uri().to_owned()]);
}

/// With `OX_NATIVE_CAPTURE_DIR` set, saves the network surfaces for visual
/// review in both themes: `native-network-discovered-*.png`,
/// `native-this-pc-saved-share-*.png`, `native-sign-in-*.png`,
/// `native-map-network-location-*.png`, `native-sign-out-*.png` and
/// `native-disconnect-*.png`. Without it, they only prove each surface
/// opens in both themes.
#[gtk::test]
fn the_network_surfaces_are_captured() {
    let _theme = ThemeGuard::keep();
    let test = TestWindow::open(Page::ThisPc.uri());
    test.save_share("smb://studio-nas/projects", "Studio NAS (Z:)");
    let found = Discovery {
        servers: vec![studio_nas()],
        warnings: Vec::new(),
    };
    test.context.network().set_discoverer(Discoverer::finding(found));
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        test.window.navigate(Page::ThisPc.uri()).expect("This PC");
        let landing = test.window.folder_pane().landing();
        wait_until("the saved share", || shows(landing, "Studio NAS (Z:)"));
        capture(&test.window, &format!("native-this-pc-saved-share-{theme}.png"));
        test.window
            .navigate(Page::Network.uri())
            .expect("the Network page");
        wait_until("the discovered server", || shows(landing, "SMB · Discovered"));
        capture(&test.window, &format!("native-network-discovered-{theme}.png"));
        capture_sign_in(&test, theme);
        for (action, target, name) in [
            ("map-network-location", None, "map-network-location"),
            ("sign-out", Some("smb://studio-nas/projects"), "sign-out"),
            ("disconnect", Some("file:///media/demo/USB"), "disconnect"),
        ] {
            test.activate(action, target);
            let dialog = open_form_dialog();
            capture_dialog(
                dialog.upcast_ref::<gtk::Window>(),
                &format!("native-{name}-{theme}.png"),
            );
            dialog.press("Cancel");
        }
    }
}

/// Saves the sign-in dialog `GVfs` would open for `smb://studio-nas/`.
fn capture_sign_in(test: &TestWindow, theme: &str) {
    let prompts = test.window.network().prompts().clone();
    let operation = prompts.create("smb://studio-nas/projects").expect("an SMB share");
    let flags = gtk::gio::AskPasswordFlags::NEED_USERNAME
        | gtk::gio::AskPasswordFlags::NEED_PASSWORD
        | gtk::gio::AskPasswordFlags::SAVING_SUPPORTED;
    operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
    let sign_in = test.window.network().sign_in().clone();
    wait_until("the sign-in dialog", || sign_in.shown_dialog().is_some());
    let dialog = sign_in.shown_dialog().expect("the sign-in dialog");
    capture_dialog(
        dialog.upcast_ref::<gtk::Window>(),
        &format!("native-sign-in-{theme}.png"),
    );
    prompts.finish(&operation, ox_core::network::MountOutcome::Failed);
}

/// Sign out and Disconnect wait for the window's file operation, so a
/// mount never goes away under a write.
///
/// parity: OPS-024, NET-020, DEV-006
#[gtk::test]
fn sign_out_and_disconnect_wait_for_a_running_file_operation() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");
    assert!(operation.is_some(), "no other operation runs");

    test.activate("sign-out", Some("smb://studio-nas/projects"));
    let sign_out_refusal = test.window.shown_message().to_string();
    test.activate("disconnect", Some("file:///media/demo/USB"));
    let disconnect_refusal = test.window.shown_message().to_string();
    test.window.end_operation();

    assert_eq!(
        sign_out_refusal,
        "Finish the current file operation before signing out."
    );
    assert_eq!(
        disconnect_refusal,
        "Finish the current operation before disconnecting."
    );
}
