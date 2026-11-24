// SPDX-License-Identifier: AGPL-3.0-only
//! Network shares and devices in the window: Network's discovered
//! computers and info bar, Map network drive, Sign out of server, Keep in
//! Network and Remove saved location, This PC's network and drive cards,
//! and connecting and removing drives, against `renderNetwork`,
//! `connectDialog`, `signOut`, `networkLocationMenu`, `mountVolume` and
//! `unmount` in `v2.0.0:desktop/ui/app.js`.
//!
//! No test mounts anything or reaches a network: discovery finds what the
//! test gives it, the keyring is in memory, and each mount or removal
//! fails on validation or for want of a mount before GIO is asked.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::integration::{FileManagerMethod, FileManagerRequest};
use ox_core::network::{
    begin_sign_out, DiscoveredServer, Discovery, ForgetScope, MountOutcome, NetworkError, SignOutRequest,
    WriteActivity, KEYRING_SAVE_NOTICE,
};

use crate::folder_view::item::FileItem;
use crate::icons::Art;
use crate::locations::Page;
use crate::network::Discoverer;
use crate::test_support::file_entry;
use crate::test_support::harness::{
    capture, capture_dialog, descendants, wait_for, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::activation::{activation_for, Activation};
use crate::window::folder_pane::PanePage;
use crate::window::landing;
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
pub(super) fn message_box_texts(title: &str) -> Vec<String> {
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
        name: "studio-nas".into(),
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

/// Connect as guest looks like the link it is, in the accent colour, and
/// stands apart from the keyring note above it, not like more of the
/// note's text.
///
/// parity: NET-011
#[gtk::test]
fn connect_as_guest_is_a_link_set_apart_from_the_note() {
    let test = TestWindow::open(Page::Network.uri());
    let operation = test
        .window
        .network()
        .prompts()
        .create("smb://nas/share")
        .expect("an SMB share");
    let flags = gtk::gio::AskPasswordFlags::NEED_USERNAME
        | gtk::gio::AskPasswordFlags::NEED_PASSWORD
        | gtk::gio::AskPasswordFlags::SAVING_SUPPORTED
        | gtk::gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;
    operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
    let sign_in = Rc::clone(test.window.network().sign_in());
    wait_until("the sign-in dialog", || sign_in.shown_dialog().is_some());
    let dialog = sign_in.shown_dialog().expect("the sign-in dialog");
    wait_until("the dialog drawn", || dialog.is_mapped() && dialog.width() > 0);
    wait_for(Duration::from_millis(200));

    let guest = descendants::<gtk::Button>(&dialog)
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Connect as guest"))
        .expect("Connect as guest");
    assert!(guest.is_visible());
    let note = descendants::<gtk::Box>(&dialog)
        .into_iter()
        .find(|widget| widget.has_css_class("sign-in-note"))
        .expect("the keyring note");
    let text = descendants::<gtk::Label>(&note)
        .into_iter()
        .next()
        .expect("the note's text");
    let link = descendants::<gtk::Label>(&guest)
        .into_iter()
        .next()
        .expect("the button's text");
    let heading = descendants::<gtk::Label>(&dialog)
        .into_iter()
        .find(|label| label.has_css_class("dialog-title"))
        .expect("the dialog's heading");
    assert_ne!(link.color(), text.color(), "the link is not drawn as the note");
    assert_ne!(link.color(), heading.color(), "the link is not drawn as text");
    let note_bottom = note
        .compute_bounds(&dialog)
        .map(|bounds| bounds.y() + bounds.height())
        .expect("the note is laid out");
    let guest_top = guest
        .compute_bounds(&dialog)
        .map(|bounds| bounds.y())
        .expect("the button is laid out");
    assert!(
        guest_top - note_bottom >= 12.0,
        "Connect as guest is {} px under the note",
        guest_top - note_bottom
    );
    dialog.close();
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

/// Network is listed like a folder, as in Windows 11: going there
/// discovers computers, each an item of the type "Computer" with the
/// server picture, which opens the server's shared folders; no info bar
/// shows while discovery runs.
///
/// Ported from `v2.0.0:desktop/tests/ui_release.py::Network discovery populates sample servers`
///
/// parity: HOME-007, HOME-008, NET-024
#[gtk::test]
fn network_lists_the_discovered_computers_as_items() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let found = Discovery {
        servers: vec![studio_nas()],
        warnings: Vec::new(),
    };
    test.context.network().set_discoverer(Discoverer::finding(found));

    test.window.navigate(Page::Network.uri()).expect("Network");
    test.wait_for_listing("Network");

    assert_eq!(test.window.folder_pane().page(), Some(PanePage::Listing));
    assert_eq!(test.names(), ["studio-nas"]);
    let watches = test
        .window
        .imp()
        .session
        .borrow()
        .active()
        .map(|tab| tab.watch.is_some());
    assert_eq!(
        watches,
        Some(false),
        "the folder left behind is no longer watched"
    );
    let computer = test.window.folder_model().item(0).expect("the computer");
    assert_eq!(computer.entry().type_label, "Computer");
    assert_eq!(Art::for_entry(computer.entry()), Art::COMPUTER);
    assert!(!test.window.network_info_bar().bar.is_visible());
    assert_eq!(
        activation_for(computer.entry()),
        Activation::Folder("smb://studio-nas/".into()),
        "the computer opens its shared folders"
    );
}

/// The services one NAS advertises: its SFTP and WebDAV servers, without
/// SMB, so listing the computer reads no server in a test.
fn harbor_files() -> Discovery {
    let service = |uri: &str, label: &str| DiscoveredServer {
        uri: uri.into(),
        label: label.into(),
        name: "Harbor Files".into(),
        host: "harbornas.local".into(),
    };
    Discovery {
        servers: vec![
            service("sftp://harbornas.local:2222/", "Harbor Files (Remote Login)"),
            service("dav://harbornas.local:8080/dav", "Harbor Files (dav+sd)"),
        ],
        warnings: Vec::new(),
    }
}

/// Network lists one computer for every service of a host, in Tiles
/// grouped by type; opening it lists its services as rows, and the tab,
/// the crumbs and the search box name it as Windows does (`\\\\PC`,
/// "Network > PC", "Search PC"). Its menu has no Properties, and
/// Properties does nothing on it.
///
/// parity: NET-036, HOME-008
#[gtk::test]
fn a_computer_lists_its_services_and_is_named_like_windows() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.context
        .network()
        .set_discoverer(Discoverer::finding(harbor_files()));

    test.window.navigate(Page::Network.uri()).expect("Network");
    test.wait_for_listing("Network");

    assert_eq!(test.names(), ["harbornas"]);
    let pane = test.window.folder_pane();
    assert_eq!(pane.view().as_str(), "tiles", "Network starts in Tiles");
    assert_eq!(
        pane.model().grouping().map(|grouping| grouping.by),
        Some(ox_core::grouping::GroupBy::Type)
    );
    let computer = test.window.folder_model().item(0).expect("the computer");
    let uri = computer.entry().navigation_uri().to_owned();
    assert_eq!(uri, "smb://harbornas.local/");
    // The menu a right-click on it opens: Windows 11's for a computer.
    test.window.right_click(Some(0));
    let menu = test.window.context_menu();
    assert_eq!(
        menu.row_labels(),
        [
            "Open",
            "Open in new tab",
            "Open in new window",
            "Pin to Quick access",
            "Create shortcut",
            "-",
            "Show more options"
        ]
    );
    menu.popdown();

    test.window.navigate(&uri).expect("the computer");
    test.wait_for_listing("the computer");

    assert_eq!(test.names(), ["Harbor Files (SFTP)", "Harbor Files (WebDAV)"]);
    let locations = test.window.imp().locations.borrow().clone();
    assert_eq!(locations.title_for(&uri), "\\\\harbornas");
    let crumbs: Vec<String> = locations
        .breadcrumbs(&uri)
        .into_iter()
        .map(|crumb| crumb.label)
        .collect();
    assert_eq!(crumbs, ["Network", "harbornas"]);
    assert_eq!(test.window.search_box().accessible_name(), "Search harbornas");
    // The tabs are back on Network: no item there opens Properties.
    test.window.navigate(Page::Network.uri()).expect("Network");
    test.wait_for_listing("Network again");
    test.window.folder_pane().model().select_all();
    test.window
        .open_properties(crate::properties::PropertiesTab::General);
    let titles: Vec<String> = gtk::Window::list_toplevels()
        .iter()
        .filter_map(|window| window.downcast_ref::<gtk::Window>().and_then(gtk::Window::title))
        .map(|title| title.to_string())
        .collect();
    assert!(
        titles.iter().all(|title| !title.contains("Properties")),
        "a computer has no Properties: {titles:?}"
    );
}

/// Typing a mapped drive letter, alone or with a path, opens the share
/// mapped to it, as in Windows; the mapped share is listed under This PC.
///
/// parity: NET-039, SIDE-035
#[gtk::test]
fn a_mapped_drive_letter_opens_in_the_address_bar() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.context
        .remember_mapped_share("smb://harbornas.local/Projects", "Projects (Z:)");

    assert_eq!(
        test.window.resolve_address("Z:").as_deref(),
        Ok("smb://harbornas.local/Projects")
    );
    assert_eq!(
        test.window.resolve_address("z:\\Plans\\Q3").as_deref(),
        Ok("smb://harbornas.local/Projects/Plans/Q3")
    );
    assert!(
        test.window.resolve_address("Q:\\Plans").is_err(),
        "no share is mapped to Q"
    );
    let labels = test.window.sidebar().labels_shown();
    let this_pc = labels
        .iter()
        .position(|label| label == "This PC")
        .expect("This PC");
    let network = labels
        .iter()
        .position(|label| label == "Network")
        .expect("Network");
    let mapped = labels
        .iter()
        .position(|label| label == "Projects (Z:)")
        .expect("the mapped share");
    assert!(this_pc < mapped && mapped < network, "{labels:?}");
}

/// Network in the navigation pane starts collapsed; expanding it looks for
/// computers, and each computer's chevron expands it to what it shares.
///
/// parity: SIDE-035
#[gtk::test]
fn the_navigation_pane_expands_network_to_computers_and_shares() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.context
        .network()
        .set_discoverer(Discoverer::finding(harbor_files()));
    let sidebar = test.window.sidebar();
    assert!(sidebar.section_is_collapsed("network"));

    sidebar.toggle_section("network");
    wait_until("the computer under Network", || {
        sidebar.labels_shown().contains(&"harbornas".to_owned())
    });
    assert!(!sidebar
        .labels_shown()
        .iter()
        .any(|label| label.starts_with("Harbor Files")));

    test.window.toggle_computer("smb://harbornas.local/");
    wait_until("its services", || {
        sidebar.labels_shown().contains(&"Harbor Files (SFTP)".to_owned())
    });
    assert!(test.window.is_computer_expanded("smb://harbornas.local/"));
    let labels = sidebar.labels_shown();
    let computer = labels
        .iter()
        .position(|label| label == "harbornas")
        .expect("the computer");
    assert_eq!(
        labels[computer + 1..computer + 3],
        ["Harbor Files (SFTP)", "Harbor Files (WebDAV)"]
    );

    test.window.toggle_computer("smb://harbornas.local/");
    assert!(!sidebar
        .labels_shown()
        .iter()
        .any(|label| label.starts_with("Harbor Files")));
}

/// Where the network browser cannot be read, Network says, as Windows
/// does, that discovery is off, with GIO's reason as the tooltip, and
/// lists nothing ("This folder is empty."); the bar's text opens a menu
/// whose "How to turn on network discovery" opens the manual's steps,
/// its × hides it until discovery runs again, and it leaves with Network.
///
/// parity: NET-024, NET-038
#[gtk::test]
fn network_says_when_discovery_is_turned_off() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.context.network().set_discoverer(Discoverer::answering(|| {
        let missing = gtk::glib::Error::new(gtk::gio::IOErrorEnum::NotSupported, "Location is not supported");
        Err(NetworkError::Gio(missing))
    }));

    test.window.navigate(Page::Network.uri()).expect("Network");
    test.wait_for_listing("Network");

    let parts = test.window.network_info_bar();
    assert!(parts.bar.is_visible());
    assert_eq!(
        texts_in(&parts.text),
        ["Network discovery is turned off. Network computers and devices are not visible. Click to change..."]
    );
    assert_eq!(
        parts.text.tooltip_text().as_deref(),
        Some("The request is not supported.")
    );
    assert!(test.names().is_empty());
    capture(&test.window, "native-network-discovery-off.png");

    parts.text.emit_clicked();
    let menu =
        crate::window::widget_tree::descendants::<crate::window::menu_popover::MenuPopover>(&parts.bar)
            .into_iter()
            .next()
            .expect("the bar's menu");
    let labels: Vec<String> = menu.items().iter().map(|item| item.label.clone()).collect();
    assert_eq!(labels, ["How to turn on network discovery", "Open Help"]);
    menu.popdown();

    test.window.dismiss_network_info_bar();
    assert!(!parts.bar.is_visible(), "× hides the bar");
    test.window.refresh();
    test.wait_for_listing("Network again");
    assert!(parts.bar.is_visible(), "discovery ran again and is still off");

    test.window.navigate(&fixture.uri()).expect("the folder");
    test.wait_for_listing("the folder");
    assert!(!parts.bar.is_visible(), "the bar belongs to Network");
}

/// With the network browser there but neither Avahi nor wsdd, and
/// nothing found, discovery is off too, and the tooltip says what is
/// missing; with them there, an empty network shows no bar.
///
/// parity: NET-038
#[gtk::test]
fn network_says_discovery_is_off_without_avahi_or_wsdd() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let off = ox_core::network::DiscoverySources {
        browser: true,
        dns_sd: false,
        ws_discovery: false,
    };
    test.context
        .network()
        .set_discoverer(Discoverer::finding(Discovery::default()).with_sources(off));

    test.window.navigate(Page::Network.uri()).expect("Network");
    test.wait_for_listing("Network");

    let parts = test.window.network_info_bar();
    assert!(parts.bar.is_visible());
    assert_eq!(
        parts.text.tooltip_text().as_deref(),
        Some("Avahi (DNS-SD) is not running and wsdd (WS-Discovery) is not installed.")
    );

    test.context
        .network()
        .set_discoverer(Discoverer::finding(Discovery::default()));
    test.window.refresh();
    test.wait_for_listing("Network again");
    assert!(!parts.bar.is_visible());
}

/// Map network drive opens from every place that offers it; an
/// address that is not a share is refused inside the dialog, which stays
/// open for another try, and Cancel closes it.
///
/// parity: NET-001
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
    assert!(dialog.can_press("Finish"), "Finish can be pressed again");
    dialog.press("Cancel");
    wait_until("Cancel to close the dialog", || !dialog.is_visible());
    assert_eq!(test.window.current_uri().as_deref(), Some(Page::Network.uri()));
}

/// "…" > Add a network location opens the connect dialog under Windows'
/// title, at a web address type (Windows' wizard adds websites and FTP
/// sites), and saves what it connects under Network locations; Map
/// network drive opens Windows' dialog at drive Z:, saving the share.
///
/// parity: NET-001, NET-002, CMD-007
#[gtk::test]
fn add_a_network_location_opens_the_connect_dialog_at_a_web_address() {
    let test = TestWindow::open(Page::ThisPc.uri());
    for (action, title, first, keep) in [
        (
            "add-network-location",
            "Add a network location",
            "Secure WebDAV (HTTPS)",
            "Save in the sidebar · reconnect when opened",
        ),
        (
            "map-network-location",
            "Map network drive",
            "Z:",
            "Save in the sidebar · reconnect when opened",
        ),
    ] {
        test.activate(action, None);
        let dialog = open_form_dialog();
        assert_eq!(dialog.title_text(), title);
        let chosen = descendants::<gtk::DropDown>(&dialog)
            .first()
            .and_then(gtk::DropDown::selected_item)
            .and_downcast::<gtk::StringObject>()
            .map(|chosen| chosen.string().to_string());
        assert_eq!(chosen.as_deref(), Some(first), "{action}");
        let save = descendants::<gtk::CheckButton>(&dialog)
            .into_iter()
            .find(|check| check.label().is_some_and(|label| label == keep))
            .expect("the dialog saves the location");
        assert!(save.is_active(), "{action} saves it under Network locations");
        dialog.press("Cancel");
        wait_until("Cancel to close the dialog", || !dialog.is_visible());
    }
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

/// A share the server refuses, or that dropped off the network, opened in
/// a tab with no folder to stay in, says so in plain words that name it,
/// as Windows Explorer does, with the
/// system's error code and words, in Windows', only under Details. Nothing on the page is
/// selected when it shows or takes the keyboard, and Try again stays.
/// The share is on a host that does not exist, and the mount is the
/// test's.
///
/// parity: NET-003, OPS-037
#[gtk::test]
fn an_offline_share_says_so_plainly_and_names_it() {
    const RAW: &str = "Failed to mount Windows share: Connection refused";
    let test = TestWindow::open(Page::Network.uri());
    test.window.network().answer_mounts_with(|| {
        Err(NetworkError::Gio(gtk::glib::Error::new(
            gtk::gio::IOErrorEnum::Failed,
            RAW,
        )))
    });

    // A new tab has no folder to stay in, so the share's page shows; a
    // tab that came from a folder stays there and says "Network Error"
    // (a_share_that_cannot_be_reached_says_network_error_and_stays).
    test.window
        .open_tab("smb://example.invalid/Studio/Photos", TabPlacement::Foreground)
        .expect("an SMB share");
    wait_until("the failed mount", || test.window.load_error().is_some());
    let error = test.window.load_error().unwrap_or_default();
    assert!(!error.contains(RAW), "Windows' words, not Samba's: {error}");
    test.wait_for_listing("the failed listing");

    let pane = test.window.folder_pane();
    assert_eq!(pane.page(), Some(PanePage::Empty));
    let page = pane.empty_page();
    assert_eq!(page.title(), "This location is unavailable");
    let message = page.message();
    assert!(
        message.contains("\\\\example.invalid\\Studio\\Photos"),
        "the message names the share: {message:?}"
    );
    assert!(
        !message.contains(RAW),
        "the raw error is not the message: {message:?}"
    );
    assert!(page.offers_try_again(), "Try again stays");
    let details = descendants::<gtk::Expander>(&page.root)
        .into_iter()
        .find(|expander| expander.is_visible() && expander.label().as_deref() == Some("Details"))
        .expect("the system's error is under Details");
    let detail = details.child().and_downcast::<gtk::Label>();
    assert_eq!(
        detail.map(|label| label.text().to_string()).as_deref(),
        Some("Error code: 0x80070043\nThe network name cannot be found."),
        "Windows' error code and words under Details"
    );
    details.set_expanded(true);
    let labels = descendants::<gtk::Label>(&page.root);
    // The message keeps its place in Tab's order, but taking the keyboard
    // selects none of its text.
    for label in labels.iter().filter(|label| label.is_focusable()) {
        label.child_focus(gtk::DirectionType::TabForward);
        wait_for(Duration::from_millis(50));
    }
    for label in &labels {
        assert!(
            label.selection_bounds().is_none(),
            "{:?} is not selected",
            label.text()
        );
    }
}

/// Any program on the session bus may ask to show a location
/// (`FileManager1`), so a share it names is not mounted for it: the tab
/// says the share is not mounted, and the share is mounted, signing in
/// with a saved password if there is one, only when the user chooses Try
/// again. The share is on a host that does not exist, which `GVfs` reports
/// as not mounted, and the mount is the test's.
///
/// parity: NET-004, INT-014
#[gtk::test]
fn a_share_another_application_shows_is_mounted_only_when_the_user_asks() {
    let test = TestWindow::open(Page::Network.uri());
    let mounts = Rc::new(Cell::new(0));
    let counted = Rc::clone(&mounts);
    test.window.network().answer_mounts_with(move || {
        counted.set(counted.get() + 1);
        Ok(())
    });
    let share = "smb://example.invalid/share";
    let folders = FileManagerRequest::new(FileManagerMethod::ShowFolders, &[share]).expect("an SMB share");
    let items = ["smb://example.invalid/other/report.pdf"];
    let items = FileManagerRequest::new(FileManagerMethod::ShowItems, &items).expect("an SMB file");

    test.window.show_file_manager_request(&folders);
    test.wait_for_listing("the shown share's listing");
    assert_eq!(test.window.current_uri().as_deref(), Some(share));
    let page = test.window.folder_pane().empty_page();
    assert!(
        page.offers_try_again(),
        "the share is unavailable, with Try again"
    );
    test.window.show_file_manager_request(&items);
    test.wait_for_listing("the shown item's folder");
    assert_eq!(
        test.window.current_uri().as_deref(),
        Some("smb://example.invalid/other")
    );
    assert!(test.window.load_error().is_some(), "the share is unavailable");
    let item = ["smb://example.invalid/third/notes.txt"];
    let properties =
        FileManagerRequest::new(FileManagerMethod::ShowItemProperties, &item).expect("an SMB file");
    test.window.show_file_manager_request(&properties);
    test.wait_for_listing("the Properties item's folder");
    wait_for(Duration::from_millis(500));
    assert!(test.window.properties_windows().is_empty(), "no Properties");
    assert_eq!(mounts.get(), 0, "nothing was mounted for the requests");
    // A refresh the folder watch starts is not the user asking.
    let active = test.window.imp().session.borrow().active_id();
    test.window.folder_changed(active.expect("a tab in front"));
    test.wait_for_listing("the watch's refresh");
    assert_eq!(mounts.get(), 0, "nothing was mounted for the watch's refresh");

    let selection = || {
        let session = test.window.imp().session.borrow();
        let tab = session.active().expect("a tab in front");
        (tab.selected.clone(), tab.reveals_selection)
    };
    let requested = (vec![item[0].to_owned()], true);
    assert_eq!(selection(), requested, "the item waits to be selected");

    test.activate("refresh", None);
    assert_eq!(selection(), requested, "Try again keeps the item to select");
    wait_until("the user's connect", || mounts.get() == 1);
    test.wait_for_listing("the listing after the mount");
    assert_eq!(mounts.get(), 1, "mounted once");
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
    // Its server is a computer under Network (SIDE-035).
    wait_until("the share's computer in the sidebar", || {
        test.window.sidebar().labels().contains(&"nas".to_owned())
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
        "",
        "signing out says nothing"
    );
    assert!(!test.window.sidebar().labels().contains(&"nas".to_owned()));
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
        "",
        "signing out says nothing"
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
/// Ported from `v2.0.0:desktop/tests/ui_v07.py::Keep in Network explicitly persists location`
///
/// parity: NET-016, NET-017, SIDE-009, SIDE-019, SIDE-020
#[gtk::test]
fn keep_in_network_saves_a_browsed_share_and_remove_deletes_it() {
    let test = TestWindow::open(Page::Network.uri());
    test.window.context().remember_network("smb://nas/media");
    // The visited share's server is a computer under Network (SIDE-035).
    wait_until("the visited share's computer under Network", || {
        test.window.sidebar().labels().contains(&"nas".to_owned())
    });
    assert!(
        test.context.settings_data().shares.is_empty(),
        "browsing alone saves nothing"
    );

    test.activate("keep-in-network", Some("smb://nas/media"));
    wait_until("the saved share", || {
        !test.context.settings_data().shares.is_empty()
    });
    // A saved location is under This PC, as Windows lists them.
    wait_until("the saved share under This PC", || {
        test.window.sidebar().labels().contains(&"media".to_owned())
    });
    let saved = test.context.settings_data().shares;
    assert_eq!(
        (saved[0].uri.as_str(), saved[0].label.as_str()),
        ("smb://nas/media", "media")
    );

    test.activate("remove-saved-location", Some("smb://nas/media"));
    wait_until("the removal", || test.context.settings_data().shares.is_empty());
    assert_eq!(test.window.shown_message().as_str(), "", "removing says nothing");
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

/// This PC lists saved locations as tiles in their own group, which it
/// leaves out while none is saved, as Windows 11 leaves out an empty
/// group; Map network drive is in "..." (CMD-007).
///
/// parity: HOME-005
#[gtk::test]
fn this_pc_lists_saved_locations_in_their_own_group() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = test.window.folder_pane().landing();
    assert_eq!(landing::section_titles(landing), ["Devices and drives"]);

    test.save_share("smb://fileserver/backup", "Backup (S:)");
    // Named as Windows 11 names a mapped drive: its label, its server and
    // its letter; the full address is the tile's tooltip.
    let name = "Backup (\\\\fileserver) (S:)";
    wait_until("the saved share's tile", || shows(landing, name));
    assert_eq!(
        landing::section_titles(landing),
        ["Devices and drives", "Network locations"]
    );
    assert!(shows(landing, "Connect on open"), "{:?}", texts_in(landing));
    let tile = descendants::<gtk::FlowBoxChild>(landing)
        .into_iter()
        .find(|tile| shows(tile, name))
        .expect("the share's tile");
    assert_eq!(tile.tooltip_text().as_deref(), Some("\\\\fileserver\\backup"));
}

/// A share's card name, as Windows 11 writes a mapped drive: "Backup
/// (\\fileserver) (S:)", the server put before a drive letter the label
/// ends with, and only once.
///
/// parity: HOME-005
#[test]
fn a_share_card_is_named_with_its_server_and_letter_as_windows_does() {
    use crate::window::landing::share_card_name;
    assert_eq!(
        share_card_name("Backup (S:)", "smb://fileserver/backup"),
        "Backup (\\\\fileserver) (S:)"
    );
    assert_eq!(
        share_card_name("Backup", "smb://fileserver/backup"),
        "Backup (\\\\fileserver)"
    );
    assert_eq!(
        share_card_name("Backup (\\\\fileserver) (S:)", "smb://fileserver/backup"),
        "Backup (\\\\fileserver) (S:)"
    );
    assert_eq!(
        share_card_name("Photos", "sftp://demo@archive.example/srv"),
        "Photos (archive.example)"
    );
}

/// Local Disk's tile shows how full it is once GIO has measured it.
///
/// parity: HOME-003, HOME-004
#[gtk::test]
fn the_local_disk_tile_shows_its_capacity() {
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
/// that show a folder on it move to Home; other tabs stay. When the drive
/// then cannot be removed, they go back to the folders they showed.
///
/// parity: DEV-009
#[gtk::test]
fn the_tabs_on_a_drive_move_home_before_it_is_ejected() {
    use gtk::subclass::prelude::*;
    let fixture = Fixture::standard();
    let home = ox_core::location::file_uri(&gtk::glib::home_dir());
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.window
        .open_tab(&fixture.uri(), TabPlacement::Background)
        .expect("a tab on the drive");
    test.window
        .open_tab(Page::ThisPc.uri(), TabPlacement::Background)
        .expect("a tab elsewhere");
    let tab_uris = || -> Vec<String> {
        let session = test.window.imp().session.borrow();
        session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
    };

    test.activate("eject", Some(&fixture.uri()));
    let moved_front = test.window.current_uri();
    let moved_tabs = tab_uris();

    message_box_texts("Could not eject");
    assert_eq!(moved_front, Some(home.clone()));
    assert_eq!(moved_tabs, [home.clone(), home, Page::ThisPc.uri().to_owned()]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(
        tab_uris(),
        [
            fixture.uri_of("Documents"),
            fixture.uri(),
            Page::ThisPc.uri().to_owned()
        ]
    );
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
        wait_until("the saved share", || {
            shows(landing, "Studio NAS (\\\\studio-nas) (Z:)")
        });
        capture(&test.window, &format!("native-this-pc-saved-share-{theme}.png"));
        test.window.navigate(Page::Network.uri()).expect("Network");
        test.wait_for_listing("Network");
        wait_until("the discovered computer", || test.names() == ["studio-nas"]);
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

/// A share that cannot be reached, gone to from a folder, says so as
/// Windows 11 does (checked on a Windows 11 VM with
/// `\\192.168.1.250\Projects`): "Network Error", "`OpenXplorer` cannot
/// access \\…", what to check, "See details" ("Hide details" when open)
/// with "Error code: 0x80070043" and "The network name cannot be found.",
/// Diagnose when a network settings app is installed, and Cancel, after
/// which the tab is still on the folder it showed.
///
/// parity: NET-003
#[gtk::test]
fn a_share_that_cannot_be_reached_says_network_error_and_stays() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.network().answer_mounts_with(|| {
        Err(NetworkError::Gio(gtk::glib::Error::new(
            gtk::gio::IOErrorEnum::Failed,
            "Failed to mount Windows share: Connection timed out",
        )))
    });

    test.window
        .navigate("smb://192.0.2.250/Projects")
        .expect("an SMB share");

    let dialog = super::file_ops_support::open_dialog(&test);
    assert_eq!(dialog.title().as_deref(), Some("Network Error"));
    assert!(
        dialog.has_css_class("main-instruction"),
        "the heading is blue, as Windows' is"
    );
    assert_eq!(
        dialog.title_text(),
        "OpenXplorer cannot access \\\\192.0.2.250\\Projects"
    );
    assert!(
        dialog.message_text().starts_with(
            "Check the spelling of the name. Otherwise, there might be a problem with your network."
        ),
        "{}",
        dialog.message_text()
    );
    let details = descendants::<gtk::Expander>(&dialog)
        .into_iter()
        .next()
        .expect("See details");
    assert!(!details.is_expanded());
    assert_eq!(details.label().as_deref(), Some("See details"));
    details.set_expanded(true);
    assert_eq!(details.label().as_deref(), Some("Hide details"));
    let detail = details.child().and_downcast::<gtk::Label>().expect("the details");
    assert_eq!(
        detail.text(),
        "Error code: 0x80070043\nThe network name cannot be found."
    );
    let buttons = dialog.button_labels();
    assert_eq!(buttons.last().map(String::as_str), Some("Cancel"));
    assert!(buttons.len() == 1 || buttons[0] == "Diagnose", "{buttons:?}");
    dialog.press("Cancel");
    super::file_ops_support::wait_for_no_dialog(&test);
    wait_until("the tab back on its folder", || {
        test.window.current_uri() == Some(fixture.uri())
    });
    let session = test.window.imp().session.borrow();
    let tab = session.active().expect("a tab");
    assert!(tab.error.is_none(), "no unavailable page");
    assert!(!tab.history.can_go_forward(), "the share is not in the history");
}
