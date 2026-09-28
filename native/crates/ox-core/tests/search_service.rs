// SPDX-License-Identifier: AGPL-3.0-only
//! The index service's scans, checks and commands through its public API.
//!
//! Covers what `desktop/index_service.py` and the cache operations of
//! `desktop/winspace.py` do beyond live local events, with an SMB share
//! held in memory: the start-up rescan, network checks, the scan limits,
//! re-reading folders the app changed, and the Search settings' commands.

mod search_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ox_core::search::{Caching, HiddenItems, IndexService, IndexSettings, RootStatus, UpdateMode};
use search_support::{
    find_root, found_names, root_state, tick_until, wait_for_status, MemoryShare, TemporaryCache,
};

/// Ticks with a short network check interval, so a test sees a check.
fn frequent_network_checks() -> IndexSettings {
    IndexSettings {
        network_interval: Duration::from_millis(100),
        ..IndexSettings::default()
    }
}

/// Each enabled root is scanned once after start-up, to catch changes
/// made while no `OpenXplorer` ran.
///
/// parity: SRCH-026
#[test]
fn each_root_is_rescanned_once_at_start_up() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let first = cache.index_share(&share);
    let generation = root_state(&cache.index, MemoryShare::URI).generation;
    first.shut_down();
    share.add_file(MemoryShare::URI, "while-closed.pdf");

    let second = cache.start_service(share.clone());

    tick_until(
        &second,
        &IndexSettings::default(),
        "the start-up scan finds the file",
        || found_names(&cache.index, "while-closed").len() == 1,
    );
    assert_ne!(root_state(&cache.index, MemoryShare::URI).generation, generation);
}

/// Network roots are never push-watched; a timed check re-reads them.
///
/// parity: SRCH-030
#[test]
fn a_network_root_is_checked_on_a_timer() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let service = cache.index_share_ticking(&share, &frequent_network_checks());
    let root = root_state(&cache.index, MemoryShare::URI);
    assert_eq!(root.update_mode, UpdateMode::IncrementalNetworkChecks);
    assert_eq!(root.watch_count, 0);

    share.add_file(MemoryShare::URI, "new-on-the-nas.pdf");

    tick_until(
        &service,
        &frequent_network_checks(),
        "a check finds the file",
        || found_names(&cache.index, "new-on-the-nas").len() == 1,
    );
}

/// Safety rule "a failed check keeps the last good data".
///
/// parity: SRCH-030
#[test]
fn a_failed_network_check_keeps_the_last_results() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    share.add_file(MemoryShare::URI, "bank.pdf");
    let service = cache.index_share_ticking(&share, &frequent_network_checks());

    share.sign_out();

    tick_until(&service, &frequent_network_checks(), "the check fails", || {
        root_state(&cache.index, MemoryShare::URI).update_mode == UpdateMode::OfflineChecks
    });
    let root = root_state(&cache.index, MemoryShare::URI);
    assert_eq!(root.watch_error.as_deref(), Some("Location is not mounted"));
    assert_eq!(found_names(&cache.index, "bank"), ["bank.pdf"]);
}

/// parity: SRCH-032
#[test]
fn folders_deeper_than_128_levels_are_reported() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let mut folder = MemoryShare::URI.to_owned();
    for level in 1..=130 {
        folder = share.add_folder(&folder, &format!("level{level}"));
    }
    let service = cache.start_service(share.clone());

    service
        .configure(MemoryShare::URI, Caching::Enabled, "", HiddenItems::Skip)
        .unwrap();

    wait_for_status(&service, MemoryShare::URI, RootStatus::Incomplete);
    let root = root_state(&cache.index, MemoryShare::URI);
    let message = "Some folders could not be read. Some directories exceed the 128-level traversal limit.";
    assert_eq!(root.error.as_deref(), Some(message));
    assert_eq!(found_names(&cache.index, "level129"), ["level129"]);
    assert!(found_names(&cache.index, "level130").is_empty());
}

/// parity: SRCH-033
#[test]
fn a_folder_the_app_changed_is_read_again() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let reports = share.add_folder(MemoryShare::URI, "Reports");
    let service = cache.index_share(&share);
    share.add_file(&reports, "pasted.pdf");

    service.folder_changed(&reports).unwrap();

    tick_until(
        &service,
        &IndexSettings::default(),
        "the pasted file is found",
        || found_names(&cache.index, "pasted").len() == 1,
    );
}

/// parity: SRCH-019
#[test]
fn enabling_a_root_scans_it_and_disabling_removes_its_names() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    share.add_file(MemoryShare::URI, "bank.pdf");

    let service = cache.index_share(&share);
    assert_eq!(found_names(&cache.index, "bank"), ["bank.pdf"]);
    service
        .configure(MemoryShare::URI, Caching::Disabled, "", HiddenItems::Skip)
        .unwrap();

    assert!(found_names(&cache.index, "bank").is_empty());
    assert_eq!(
        root_state(&cache.index, MemoryShare::URI).status,
        RootStatus::Disabled
    );
}

/// Signing out of a server pauses its indexing and stops its running
/// scan, and can clear its cached names; the next sign-in resumes
/// indexing.
///
/// parity: NET-022
#[test]
fn signing_out_pauses_a_server_and_can_clear_its_names() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    share.add_file(MemoryShare::URI, "bank.pdf");
    let service = cache.index_share(&share);
    share.stall();
    service.refresh(MemoryShare::URI).unwrap();
    wait_for_status(&service, MemoryShare::URI, RootStatus::Indexing);

    service.pause_server("NAS").unwrap();
    wait_for_status(&service, MemoryShare::URI, RootStatus::Incomplete);
    service.clear_server("nas").unwrap();
    assert!(found_names(&cache.index, "bank").is_empty());
    assert_eq!(
        root_state(&cache.index, MemoryShare::URI).status,
        RootStatus::NotIndexed
    );
    share.sign_in();
    service.resume_server("nas").unwrap();

    wait_for_status(&service, MemoryShare::URI, RootStatus::Ready);
    assert_eq!(found_names(&cache.index, "bank"), ["bank.pdf"]);
}

/// Stop cancels a running scan; the scan counts as incomplete and the
/// earlier results stay.
///
/// parity: SRCH-023, SRCH-024
#[test]
fn stopping_a_scan_keeps_the_earlier_results() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    share.add_file(MemoryShare::URI, "bank.pdf");
    let service = cache.index_share(&share);
    share.stall();
    service.refresh(MemoryShare::URI).unwrap();
    wait_for_status(&service, MemoryShare::URI, RootStatus::Indexing);

    service.stop(MemoryShare::URI).unwrap();

    wait_for_status(&service, MemoryShare::URI, RootStatus::Incomplete);
    let root = root_state(&cache.index, MemoryShare::URI);
    assert_eq!(root.error.as_deref(), Some("Operation cancelled."));
    assert_eq!(found_names(&cache.index, "bank"), ["bank.pdf"]);
}

/// parity: SRCH-023
#[test]
fn clearing_and_removing_a_root_through_the_service() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    share.add_file(MemoryShare::URI, "bank.pdf");
    let service = cache.index_share(&share);

    service.clear(MemoryShare::URI).unwrap();
    assert!(found_names(&cache.index, "bank").is_empty());
    assert_eq!(
        root_state(&cache.index, MemoryShare::URI).status,
        RootStatus::NotIndexed
    );
    service.remove(MemoryShare::URI).unwrap();

    assert!(find_root(&cache.index, MemoryShare::URI).is_none());
}

/// The app hears about every change of the cache status, so the search
/// settings and an active search can refresh (`cacheChanged`).
///
/// parity: SRCH-018
#[test]
fn the_app_is_told_when_the_cache_changed() {
    let cache = TemporaryCache::new();
    let share = MemoryShare::new();
    let changes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&changes);
    let listener = move || {
        counter.fetch_add(1, Ordering::Relaxed);
    };
    let service = IndexService::start(cache.index.clone(), share.clone(), listener).unwrap();

    service
        .configure(MemoryShare::URI, Caching::Enabled, "", HiddenItems::Skip)
        .unwrap();

    wait_for_status(&service, MemoryShare::URI, RootStatus::Ready);
    tick_until(&service, &IndexSettings::default(), "the app is told", || {
        changes.load(Ordering::Relaxed) >= 2
    });
}
