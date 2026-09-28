// SPDX-License-Identifier: AGPL-3.0-only
//! What activating an item does, and which application opens a file.
//! Ports `OpeningTests` of `desktop/tests/test_v05.py`.

use ox_core::entry::{Entry, EntryKind};
use ox_core::integration::{choose_application, Activation, OpenError};

use super::{app, OWN_ID};
use crate::integration_support::item;

fn activation(entry: &Entry) -> Result<Activation, OpenError> {
    Activation::for_entry(entry)
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_regular_pdf`
/// parity: OPEN-001, OPEN-005
#[test]
fn a_pdf_opens_as_a_file() {
    assert_eq!(
        activation(&item(EntryKind::File, "a.pdf")),
        Ok(Activation::OpenFile)
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_regular_mp4`
/// parity: OPEN-001, OPEN-005
#[test]
fn a_video_opens_as_a_file() {
    assert_eq!(
        activation(&item(EntryKind::File, "a.mp4")),
        Ok(Activation::OpenFile)
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_directory_with_extension`
/// parity: OPEN-001, OPEN-005, NAV-040
#[test]
fn a_folder_named_like_a_video_opens_as_a_folder() {
    assert_eq!(
        activation(&item(EntryKind::Directory, "a.mp4")),
        Ok(Activation::OpenFolder)
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_directory_named_zip`
/// parity: OPEN-001, OPEN-005, ARC-002, NAV-040
#[test]
fn a_folder_named_like_a_zip_opens_as_a_folder() {
    assert_eq!(
        activation(&item(EntryKind::Directory, "a.zip")),
        Ok(Activation::OpenFolder)
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_zip_by_mime`
/// parity: OPEN-001, OPEN-005, ARC-002
#[test]
fn a_zip_by_content_type_is_browsed() {
    let archive = Entry {
        content_type: Some("application/zip".to_owned()),
        ..item(EntryKind::File, "a")
    };

    assert_eq!(activation(&archive), Ok(Activation::BrowseArchive));
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_zip_by_extension`
/// parity: OPEN-001, OPEN-005, ARC-002
#[test]
fn a_zip_by_name_in_any_case_is_browsed() {
    assert_eq!(
        activation(&item(EntryKind::File, "a.ZIP")),
        Ok(Activation::BrowseArchive)
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_unknown_not_directory`
/// parity: OPEN-001, OPEN-005
#[test]
fn an_item_of_unknown_type_is_not_opened() {
    let refused = activation(&item(EntryKind::Unknown, "a"));

    assert_eq!(refused, Err(OpenError::NotRegularOrFolder));
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "This item is not a regular file or a readable folder."
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_mountable`
/// parity: OPEN-001, OPEN-005
#[test]
fn a_share_that_navigates_opens_as_a_folder() {
    let share = Entry {
        is_dir: true,
        ..item(EntryKind::Mountable, "Projects")
    };

    assert_eq!(activation(&share), Ok(Activation::OpenFolder));
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_regular_overrides_stale_bool`
/// parity: OPEN-001, OPEN-005, NAV-040
#[test]
fn a_regular_file_is_opened_even_if_flagged_as_a_folder() {
    let stale = Entry {
        is_dir: true,
        ..item(EntryKind::File, "a.pdf")
    };

    assert_eq!(activation(&stale), Ok(Activation::OpenFile));
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_excludes_smb_self_handler`
/// parity: OPEN-001, OPEN-005
#[test]
fn openxplorer_is_skipped_for_the_next_application() {
    let own = app(OWN_ID, "OpenXplorer");
    let external = app("evince.desktop", "Document Viewer");

    let chosen = choose_application([own.clone(), external.clone()], Some(own));

    assert_eq!(chosen, Ok(external));
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_no_recursion_without_external_app`
/// parity: OPEN-005
#[test]
fn without_another_application_nothing_opens() {
    let own = app(OWN_ID, "OpenXplorer");

    let refused = choose_application([own.clone()], Some(own));

    assert_eq!(refused, Err(OpenError::NoApplication));
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "No application is installed for this file type. Use Open with… to choose one."
    );
}

/// Ported from `desktop/tests/test_v05.py::OpeningTests::test_respects_external_default`
/// parity: OPEN-001, OPEN-005
#[test]
fn another_default_application_is_used() {
    let player = app("vlc.desktop", "VLC media player");

    let chosen = choose_application(Vec::new(), Some(player.clone()));

    assert_eq!(chosen, Ok(player));
}
