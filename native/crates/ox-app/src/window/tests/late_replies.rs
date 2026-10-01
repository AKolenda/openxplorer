// SPDX-License-Identifier: AGPL-3.0-only
//! Properties lookups that answer late: a closed dialog or a newer lookup
//! is never overwritten (SAFE-013).

use ox_core::entry::EntryError;
use ox_core::versions::VersionList;

use super::item_dialogs::{fixture_with_snapshot, press, properties_view, texts, value_after, SNAPSHOT_NAME};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};

/// A properties read that answers after Close changes nothing in the
/// closed dialog.
///
/// parity: SAFE-013
#[gtk::test]
fn a_properties_read_after_close_changes_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    let view = properties_view(&frame);
    let general = view.general_panel();
    wait_until("the properties to be read", || {
        value_after(&general, "Type").is_some()
    });
    let shown = texts(&general);

    press(&frame, "Close");
    view.deliver_properties(Err(EntryError::NotFound("The item is gone.".into())));

    assert_eq!(texts(&general), shown);
}

/// A Previous versions lookup overtaken by Refresh is dropped when it
/// answers after the newer one.
///
/// parity: SAFE-013
#[gtk::test]
fn a_versions_lookup_overtaken_by_refresh_is_dropped() {
    let fixture = fixture_with_snapshot();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");
    test.activate("previous-versions", None);
    let frame = test.wait_for_dialog("the Previous versions tab");
    let versions = properties_view(&frame).versions();
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
