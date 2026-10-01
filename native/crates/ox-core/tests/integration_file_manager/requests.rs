// SPDX-License-Identifier: AGPL-3.0-only
//! The checks every `FileManager1` request passes. Ports the
//! `filemanager_request` cases of `HandoffTests`.

use ox_core::integration::{FileManagerMethod, FileManagerRequest, FileManagerRequestError};

/// Ported from `v2.0.0:desktop/tests/test_v07.py::HandoffTests::test_showitems_keeps_file_path`
/// parity: INT-013, INT-014
#[test]
fn show_items_keeps_the_file_itself() {
    let request =
        FileManagerRequest::from_method_name("ShowItems", &["file:///tmp/movie.mp4"]).expect("valid");

    assert_eq!(request.uris(), ["file:///tmp/movie.mp4"]);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::HandoffTests::test_showfolders_and_properties`
/// parity: INT-014
#[test]
fn show_folders_and_show_item_properties_are_accepted() {
    for method in ["ShowFolders", "ShowItemProperties"] {
        let request = FileManagerRequest::from_method_name(method, &["smb://nas/work"]).expect("valid");

        assert_eq!(request.method().as_str(), method);
    }
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::HandoffTests::test_unsupported_method_rejected`
/// parity: INT-013, SAFE-017
#[test]
fn an_unsupported_method_is_rejected() {
    let refused = FileManagerRequest::from_method_name("Execute", &["/tmp/script"]);

    assert_eq!(refused, Err(FileManagerRequestError::UnsupportedMethod));
    assert_eq!(refused.expect_err("refused").to_string(), "Unsupported method.");
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::HandoffTests::test_empty_request_rejected`
/// parity: INT-013, SAFE-017
#[test]
fn a_request_without_locations_is_rejected() {
    let no_locations: [&str; 0] = [];

    let refused = FileManagerRequest::new(FileManagerMethod::ShowItems, &no_locations);

    assert_eq!(refused, Err(FileManagerRequestError::WrongLocationCount));
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "Expected 1–100 file locations."
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::HandoffTests::test_reveal_limit`
/// parity: INT-013, SAFE-017
#[test]
fn more_than_100_locations_are_rejected() {
    let at_limit = vec!["/tmp/x"; 100];
    let over_limit = vec!["/tmp/x"; 101];

    assert!(FileManagerRequest::new(FileManagerMethod::ShowItems, &at_limit).is_ok());
    assert_eq!(
        FileManagerRequest::new(FileManagerMethod::ShowItems, &over_limit),
        Err(FileManagerRequestError::WrongLocationCount)
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::HandoffTests::test_no_virtual_locations_in_external_requests`
/// parity: INT-013, SAFE-017
#[test]
fn app_pages_cannot_be_requested_from_outside() {
    for page in ["settings:", "ox:settings", "home:", "trash:///"] {
        let refused = FileManagerRequest::new(FileManagerMethod::ShowFolders, &[page]);

        assert!(
            matches!(refused, Err(FileManagerRequestError::Location(_))),
            "{page}: {refused:?}"
        );
    }
}
