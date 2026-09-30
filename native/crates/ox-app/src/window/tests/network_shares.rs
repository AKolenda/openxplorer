// SPDX-License-Identifier: AGPL-3.0-only
//! SMB shares in the window: a share in a server listing, a mapped share
//! after it connected, tabs under a kernel CIFS mount, what browsing a
//! share remembers, and refreshing a folder the server does not watch.
//!
//! Ports the share rows of `fileIcon` and `activate`, `connectDialog`'s
//! success path, `networkLocation` and `renderTabs` in
//! `desktop/ui/app.js`, and `watch` and `remember_network` in
//! `desktop/winspace.py`. No test reaches a server: shares are mounted
//! only by the test's own answer, and a listing on a host that does not
//! exist fails before any network is used.

use std::path::PathBuf;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::entry_from_info;
use ox_core::network::{ConnectedShare, MountOutcome};
use ox_core::places::StableMount;
use ox_core::search::{Caching, RootStatus};

use super::network::open_form_dialog;
use crate::dialogs::ShareKeeping;
use crate::folder_view::watch::watch_folder;
use crate::icons::Art;
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow};
use crate::window::activation::{activation_for, Activation};

/// The tooltips of the tabs in the strip, in order.
fn tab_tooltips(test: &TestWindow) -> Vec<String> {
    let tab_list = test.window.tab_strip().tab_list();
    let tabs = descendants::<gtk::Box>(&tab_list)
        .into_iter()
        .filter(|tab| tab.accessible_role() == gtk::AccessibleRole::Tab);
    tabs.filter_map(|tab| tab.tooltip_text())
        .map(|text| text.to_string())
        .collect()
}

/// A share as `gvfsd-smb-browse` lists it in `smb://nas/`: a mountable
/// folder whose target is the share itself. The share shows the share art
/// on the network bar, opens its target (on activation and middle-click
/// alike), and as a share root is not an item the edit commands act on.
///
/// Ported from `desktop/tests/ui_rc4.py::Share opens its target URI, not a fabricated local folder`
///
/// parity: NET-003
#[test]
fn a_share_in_a_server_listing_is_a_network_folder_that_opens_its_target() {
    let info = gio::FileInfo::new();
    info.set_file_type(gio::FileType::Mountable);
    info.set_display_name("media");
    info.set_content_type("inode/directory");
    info.set_attribute_string("standard::target-uri", "smb://nas/media");

    let share = entry_from_info(&gio::File::for_uri("smb://nas/media"), &info);

    assert_eq!(Art::for_entry(&share), Art::SHARE);
    assert_eq!(
        activation_for(&share),
        Activation::Folder("smb://nas/media".to_owned())
    );
    assert!(share.is_virtual, "the edit commands skip virtual items");
    assert!(
        !share.can_operate,
        "a share root is never cut, copied, renamed or deleted"
    );
}

/// After Map network location connected, the share is saved with its
/// label, the dialog closes, the sidebar lists it and the window opens it.
/// Connecting itself needs a server, so the test starts where the mount
/// succeeded.
///
/// Ported from `desktop/tests/test_v05.py::AuthTests::test_success_saved_after_finish`
///
/// parity: NET-001
#[gtk::test]
fn a_connected_share_is_saved_listed_and_opened() {
    let test = TestWindow::open(Page::Network.uri());
    test.activate("map-network-location", None);
    let dialog = open_form_dialog();
    let share = ConnectedShare {
        uri: "smb://example.invalid/projects".to_owned(),
        label: "Projects (Z:)".to_owned(),
    };

    test.window
        .keep_mapped_share(&dialog, share, ShareKeeping::SaveInSidebar);

    wait_until("the opened share", || {
        test.window.current_uri().as_deref() == Some("smb://example.invalid/projects")
    });
    let saved = test.context.settings_data().shares;
    assert_eq!(saved.len(), 1);
    assert_eq!(
        (saved[0].uri.as_str(), saved[0].label.as_str()),
        ("smb://example.invalid/projects", "Projects (Z:)")
    );
    assert!(!dialog.is_visible(), "the dialog closed");
    let labels = test.window.sidebar().labels();
    assert!(labels.contains(&"Projects (Z:)".to_owned()), "{labels:?}");
}

/// A tab inside a kernel CIFS/SMB3 mount is a network location: its
/// address says so and its icon stands on the network bar, as soon as the
/// mount is known.
///
/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_stable_cifs_mount`
///
/// parity: NET-006
#[gtk::test]
fn a_tab_inside_a_cifs_mount_is_marked_as_a_network_location() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(!tab_tooltips(&test)[0].ends_with(" · Network location"));

    let mount = StableMount {
        path: PathBuf::from(fixture.root()),
        label: String::new(),
        filesystem: "cifs".to_owned(),
    };
    test.context.network().replace_stable_mounts(vec![mount]);
    // Any change of the places redraws the tabs; the kernel mounts are
    // read with them.
    test.context.remember_network("smb://example.invalid/other");

    wait_until("the network tab", || {
        tab_tooltips(&test)[0].ends_with(" · Network location")
    });
}

/// Browsing remembers a share for the session only after it was listed:
/// a share that could not be listed is not added to Network, and nothing
/// is saved to the settings.
///
/// parity: NET-016
#[gtk::test]
fn a_share_that_could_not_be_listed_is_not_remembered() {
    let test = TestWindow::open(Page::Network.uri());

    test.window
        .navigate("smb://example.invalid/unlisted/Reports")
        .expect("an SMB folder");
    wait_until("the failed listing", || test.window.load_error().is_some());

    let labels = test.window.sidebar().labels();
    assert!(!labels.contains(&"unlisted".to_owned()), "{labels:?}");
    assert!(test.context.settings_data().shares.is_empty());
}

/// A folder the server cannot watch for changes still lists, shows no
/// error, and Refresh lists it again.
///
/// parity: NET-005
#[gtk::test]
fn a_folder_without_change_notifications_refreshes_on_request() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let id = test.active_tab().expect("the fixture's tab");
    // A location GIO cannot monitor stands in for an SMB server without
    // change notifications: its watch ends at once.
    let unwatched = watch_folder("smb://example.invalid/share", || {});
    test.window
        .imp()
        .session
        .borrow_mut()
        .tab_mut(id)
        .expect("the tab")
        .watch = Some(unwatched);

    fixture.write("Added on the server.txt");
    wait_for(Duration::from_millis(800));
    assert!(!test.names().contains(&"Added on the server.txt".to_owned()));

    test.activate("refresh", None);
    wait_until("the refreshed folder", || {
        test.names().contains(&"Added on the server.txt".to_owned())
    });
    assert_eq!(test.window.load_error(), None);
}

/// Whether the search cache pauses indexing of the server `host`.
fn is_indexing_paused(test: &TestWindow, host: &str) -> bool {
    let paused = test.context.search_cache().is_server_paused(host);
    glib::MainContext::default()
        .block_on(paused)
        .expect("a started search cache")
}

/// Signing out pauses indexing of the server, and clears its cached
/// names when chosen, until the next successful mount of it through any
/// window's prompts, which indexes a pinned share that needed sign-in.
///
/// parity: NET-022, SRCH-040
#[gtk::test]
fn signing_out_pauses_indexing_the_server_until_it_is_mounted_again() {
    let test = TestWindow::open(Page::Network.uri());
    test.start_search_cache();
    let share = "smb://example.invalid/share";
    let cache = test.context.search_cache().clone();
    glib::spawn_future_local(async move {
        cache
            .set_caching(share, Caching::Enabled, "share")
            .await
            .expect("a share can be indexed");
    });
    wait_until("the share's first scan", || {
        test.find_root(share).is_some_and(|root| {
            !matches!(
                root.status,
                RootStatus::NotIndexed | RootStatus::Queued | RootStatus::Indexing
            )
        })
    });

    test.activate("sign-out", Some(share));
    let dialog = open_form_dialog();
    let choices = descendants::<gtk::CheckButton>(&dialog);
    choices[0].set_active(false);
    choices[1].set_active(true);
    dialog.press_primary();
    wait_until("the Network page", || {
        test.window.current_uri().as_deref() == Some(Page::Network.uri())
    });
    wait_until("the paused server", || {
        is_indexing_paused(&test, "example.invalid")
    });
    wait_until("the cleared share", || {
        test.find_root(share)
            .is_some_and(|root| root.status == RootStatus::NotIndexed)
    });

    let prompts = test.window.network().prompts();
    let operation = prompts.create(share).expect("a share address");
    prompts.finish(&operation, MountOutcome::Mounted);

    wait_until("indexing to resume", || {
        !is_indexing_paused(&test, "example.invalid")
    });
}
