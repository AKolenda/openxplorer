// SPDX-License-Identifier: AGPL-3.0-only
//! Properties lookups that answer late: a closed window or a newer lookup
//! is never overwritten (SAFE-013).

use gtk::prelude::*;
use ox_core::versions::VersionList;

use super::item_dialogs::{fixture_with_snapshot, tab, texts, versions_of, SNAPSHOT_NAME};
use crate::test_support::harness::{settle, wait_for, wait_until, Fixture, TestWindow};

/// A properties read that answers after the window closed fills nothing
/// in it.
///
/// parity: SAFE-013
#[gtk::test]
fn a_properties_read_after_close_changes_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let window = test.wait_for_properties("Notes 2.txt Properties");
    let general = tab(&window, "general");
    let shown = texts(&general);

    window.close();
    wait_for(std::time::Duration::from_millis(300));
    settle();

    assert_eq!(texts(&general), shown, "the late read filled nothing");
}

/// A Previous Versions lookup overtaken by F5 is dropped when it
/// answers after the newer one.
///
/// parity: SAFE-013
#[gtk::test]
fn a_versions_lookup_overtaken_by_refresh_is_dropped() {
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");
    test.activate("previous-versions", None);
    let window = test.wait_for_properties("Documents Properties");
    let versions = versions_of(&window);
    wait_until("the versions", || !versions.version_labels().is_empty());
    let first = versions.lookup_number();

    versions.refresh();
    wait_until("the refreshed versions", || !versions.version_labels().is_empty());
    let nothing_found = VersionList {
        versions: Vec::new(),
        collections: Vec::new(),
        warnings: Vec::new(),
        is_truncated: false,
        configured: Vec::new(),
    };
    versions.deliver_lookup(first, Ok(nothing_found));

    assert_eq!(versions.version_labels(), [SNAPSHOT_NAME]);
}
