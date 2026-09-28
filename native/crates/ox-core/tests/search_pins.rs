// SPDX-License-Identifier: AGPL-3.0-only
//! Pinned folders are indexed automatically (SRCH-040), through the index
//! service's public API.
//!
//! A new behaviour the product owner decided on 2026-09-28, so there is no
//! Python test to port; the cases are the ones the parity inventory lists.

mod search_support;

use std::time::Duration;

use ox_core::search::{Caching, HiddenItems, IndexSettings, PinIndexing, RootOrigin, RootStatus};
use ox_core::settings::Bookmark;
use search_support::{
    find_root, found_names, root_state, tick_for, wait_for_status, MemoryShare, TemporaryCache,
};

/// A pin of `uri` labelled `label`.
fn pin(uri: &str, label: &str) -> Bookmark {
    Bookmark {
        uri: uri.to_owned(),
        label: label.to_owned(),
    }
}

/// parity: SRCH-040
#[test]
fn pinning_a_folder_indexes_it_with_a_pinned_tag() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let team = share.add_folder(MemoryShare::URI, "Team");
    share.add_file(&team, "plan.pdf");
    let service = cache.start_service(share.clone());

    service
        .pin_added(&pin(&team, "Team files"), PinIndexing::Automatic)
        .unwrap();

    wait_for_status(&service, &team, RootStatus::Ready);
    let root = root_state(&cache.index, &team);
    assert_eq!(root.origin, RootOrigin::Pin);
    assert_eq!(root.label, "Team files");
    assert_eq!(found_names(&cache.index, "plan"), ["plan.pdf"]);
}

/// parity: SRCH-040
#[test]
fn unpinning_removes_only_roots_that_pinning_added() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let pinned = share.add_folder(MemoryShare::URI, "Pinned");
    let chosen = share.add_folder(MemoryShare::URI, "Chosen");
    share.add_file(&pinned, "pinned.pdf");
    let service = cache.start_service(share.clone());
    service
        .pin_added(&pin(&pinned, "Pinned"), PinIndexing::Automatic)
        .unwrap();
    service
        .configure(&chosen, Caching::Enabled, "", HiddenItems::Skip)
        .unwrap();
    wait_for_status(&service, &pinned, RootStatus::Ready);

    service.pin_removed(&pinned).unwrap();
    service.pin_removed(&chosen).unwrap();

    assert!(find_root(&cache.index, &pinned).is_none());
    assert!(found_names(&cache.index, "pinned").is_empty());
    assert_eq!(root_state(&cache.index, &chosen).origin, RootOrigin::User);
}

/// Safety rule "the user's choice wins": choosing to index a pinned folder
/// makes the root the user's, and a folder the user switched off stays
/// off when it is pinned.
///
/// parity: SRCH-040
#[test]
fn the_users_choice_outlasts_pinning() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let chosen = share.add_folder(MemoryShare::URI, "Chosen");
    let switched_off = share.add_folder(MemoryShare::URI, "Off");
    let service = cache.start_service(share.clone());
    service
        .pin_added(&pin(&chosen, "Chosen"), PinIndexing::Automatic)
        .unwrap();
    service
        .configure(&chosen, Caching::Enabled, "Chosen", HiddenItems::Skip)
        .unwrap();
    service
        .configure(&switched_off, Caching::Disabled, "", HiddenItems::Skip)
        .unwrap();

    service
        .pin_added(&pin(&switched_off, "Off"), PinIndexing::Automatic)
        .unwrap();
    service.pin_removed(&chosen).unwrap();

    assert_eq!(root_state(&cache.index, &chosen).origin, RootOrigin::User);
    let off = root_state(&cache.index, &switched_off);
    assert_eq!(off.status, RootStatus::Disabled);
    assert_eq!(off.origin, RootOrigin::User);
}

/// Folders pinned before pinned folders were indexed are indexed once;
/// a root the user removed afterwards is not added again.
///
/// parity: SRCH-040
#[test]
fn folders_pinned_earlier_are_indexed_once() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let first = share.add_folder(MemoryShare::URI, "First");
    let second = share.add_folder(MemoryShare::URI, "Second");
    share.add_file(&second, "second.pdf");
    let pins = [pin(&first, "First"), pin(&second, "Second")];
    let service = cache.start_service(share.clone());

    service
        .index_existing_pins(&pins, PinIndexing::Automatic)
        .unwrap();
    wait_for_status(&service, &second, RootStatus::Ready);
    service.remove(&first).unwrap();
    service
        .index_existing_pins(&pins, PinIndexing::Automatic)
        .unwrap();

    assert!(find_root(&cache.index, &first).is_none());
    assert_eq!(root_state(&cache.index, &second).origin, RootOrigin::Pin);
    assert_eq!(found_names(&cache.index, "second.pdf"), ["second.pdf"]);
}

/// Safety rule "a root the user removed stays removed": the start-up
/// indexing of earlier pins found the switch off, the user turned it on
/// and then removed the pinned root, and the next start-up does not add
/// the root back.
///
/// parity: SRCH-040
#[test]
fn a_removed_pinned_root_stays_removed_after_the_switch_was_turned_on() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let team = share.add_folder(MemoryShare::URI, "Team");
    let pins = [pin(&team, "Team")];
    let first_run = cache.start_service(share.clone());
    first_run.index_existing_pins(&pins, PinIndexing::Off).unwrap();
    first_run.set_pin_indexing(&pins, PinIndexing::Automatic).unwrap();
    wait_for_status(&first_run, &team, RootStatus::Ready);
    first_run.remove(&team).unwrap();
    first_run.shut_down();

    let next_run = cache.start_service(share.clone());
    next_run
        .index_existing_pins(&pins, PinIndexing::Automatic)
        .unwrap();

    assert!(find_root(&cache.index, &team).is_none());
}

/// Safety rule "a folder is indexed once": pinning a folder inside an
/// indexed share adds no second root, so its items are stored once.
///
/// parity: SRCH-040
#[test]
fn pinning_a_folder_inside_an_indexed_root_adds_no_root() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let team = share.add_folder(MemoryShare::URI, "Team");
    share.add_file(&team, "plan.pdf");
    let service = cache.index_share(&share);

    service
        .pin_added(&pin(&team, "Team"), PinIndexing::Automatic)
        .unwrap();

    assert!(find_root(&cache.index, &team).is_none());
    assert_eq!(cache.index.status().unwrap().entry_count, 2);
}

/// Safety rule "no scan while signing out": pinning a folder on a server
/// the user is signing out of does not resume it; the folder is indexed
/// once the user signs in again.
///
/// parity: SRCH-040, NET-022
#[test]
fn a_pin_on_a_server_being_signed_out_of_waits_for_sign_in() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let team = share.add_folder(MemoryShare::URI, "Team");
    share.add_file(&team, "plan.pdf");
    let service = cache.start_service(share.clone());
    service.pause_server("nas").unwrap();

    service
        .pin_added(&pin(&team, "Team"), PinIndexing::Automatic)
        .unwrap();
    tick_for(&service, &IndexSettings::default(), Duration::from_millis(600));
    assert_eq!(root_state(&cache.index, &team).status, RootStatus::NotIndexed);
    service.resume_server("nas").unwrap();

    wait_for_status(&service, &team, RootStatus::Ready);
    assert_eq!(found_names(&cache.index, "plan"), ["plan.pdf"]);
}

/// The switch off: pinning adds nothing and the roots pinning added go;
/// back on: every pinned folder is indexed again. Roots the user chose
/// stay throughout.
///
/// parity: SRCH-040
#[test]
fn the_switch_turns_indexing_of_pinned_folders_off_and_on() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let pinned = share.add_folder(MemoryShare::URI, "Pinned");
    let later = share.add_folder(MemoryShare::URI, "Later");
    let chosen = share.add_folder(MemoryShare::URI, "Chosen");
    let service = cache.start_service(share.clone());
    service
        .pin_added(&pin(&pinned, "Pinned"), PinIndexing::Automatic)
        .unwrap();
    service
        .configure(&chosen, Caching::Enabled, "", HiddenItems::Skip)
        .unwrap();
    let pins = [
        pin(&pinned, "Pinned"),
        pin(&later, "Later"),
        pin(&chosen, "Chosen"),
    ];

    service.set_pin_indexing(&pins, PinIndexing::Off).unwrap();
    service
        .pin_added(&pin(&later, "Later"), PinIndexing::Off)
        .unwrap();
    assert!(find_root(&cache.index, &pinned).is_none());
    assert!(find_root(&cache.index, &later).is_none());
    assert_eq!(root_state(&cache.index, &chosen).origin, RootOrigin::User);
    service.set_pin_indexing(&pins, PinIndexing::Automatic).unwrap();

    assert_eq!(root_state(&cache.index, &pinned).origin, RootOrigin::Pin);
    assert_eq!(root_state(&cache.index, &later).origin, RootOrigin::Pin);
    assert_eq!(root_state(&cache.index, &chosen).origin, RootOrigin::User);
}

/// A pinned share that needs sign-in is not mounted by the crawler; it
/// is indexed once the user signs in.
///
/// parity: SRCH-040
#[test]
fn a_pinned_share_that_needs_sign_in_is_indexed_after_sign_in() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::needing_sign_in();
    share.add_file(MemoryShare::URI, "bank.pdf");
    let service = cache.start_service(share.clone());
    service
        .pin_added(&pin(MemoryShare::URI, "share"), PinIndexing::Automatic)
        .unwrap();
    wait_for_status(&service, MemoryShare::URI, RootStatus::Incomplete);
    assert!(found_names(&cache.index, "bank").is_empty());

    share.sign_in();
    service.resume_server("nas").unwrap();

    wait_for_status(&service, MemoryShare::URI, RootStatus::Ready);
    assert_eq!(found_names(&cache.index, "bank"), ["bank.pdf"]);
}

/// Pinned phones, cameras and server share lists are not indexed.
///
/// parity: SRCH-021, SRCH-040
#[test]
fn pinned_devices_and_server_lists_are_not_indexed() {
    let cache = TemporaryCache::new();
    let service = cache.start_service(MemoryShare::new());

    service
        .pin_added(
            &pin("mtp://Pixel_7/Internal%20storage", "Phone"),
            PinIndexing::Automatic,
        )
        .unwrap();
    service
        .pin_added(&pin("smb://nas/", "nas"), PinIndexing::Automatic)
        .unwrap();

    assert!(cache.index.roots().unwrap().is_empty());
}
