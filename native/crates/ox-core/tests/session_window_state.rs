// SPDX-License-Identifier: AGPL-3.0-only
//! A tab's navigation state and `FileManager1` request arguments. Ports
//! `HandoffTests` of `desktop/tests/test_v07.py`; `session_python.rs`
//! runs the same functions through both apps.

use ox_core::location::{HOME_URI, NETWORK_URI, SETTINGS_URI};
use ox_core::session::{
    FileManagerMethod, FileManagerRequest, SettingsSection, SortDirection, SortField, TabSnapshot, TabView,
    WindowStateError, MAX_SCROLL,
};
use serde_json::json;

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_tab_roundtrip`
/// parity: TAB-038
#[test]
fn tab_roundtrip() {
    let state = json!({
        "uri": "smb://nas/work",
        "history": ["file:///home/demo", "smb://nas/work"],
        "index": 1,
        "selection": ["smb://nas/work/report.pdf"],
        "view": "grid",
        "scroll": 1600,
        "sort": "size",
        "descending": true,
    });

    let snapshot = TabSnapshot::from_json(&state).unwrap();

    assert_eq!(snapshot.index, 1);
    assert_eq!(snapshot.history, ["file:///home/demo", "smb://nas/work"]);
    assert_eq!(snapshot.selection, ["smb://nas/work/report.pdf"]);
    assert_eq!(snapshot.view, TabView::Grid);
    assert_eq!(snapshot.sort, SortField::Size);
    assert_eq!(snapshot.direction, SortDirection::Descending);
    assert!((snapshot.scroll - 1600.0).abs() < f64::EPSILON);
    assert_eq!(TabSnapshot::from_json(&snapshot.to_json()).unwrap(), snapshot);
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_no_password_fields_forwarded`
/// parity: TAB-038
#[test]
fn no_password_fields_forwarded() {
    let state = json!({"uri": "home:", "password": "not a real password"});

    let snapshot = TabSnapshot::from_json(&state).unwrap();

    let forwarded = snapshot.to_json();
    assert!(forwarded.get("password").is_none());
    assert!(!forwarded.to_string().contains("not a real password"));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_unknown_scheme_rejected`
/// parity: TAB-038
#[test]
fn unknown_scheme_rejected() {
    let result = TabSnapshot::from_json(&json!({"uri": "javascript:alert(1)"}));

    assert!(matches!(result, Err(WindowStateError::Location(_))));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_bad_history_position_rejected`
/// parity: TAB-038
#[test]
fn bad_history_position_rejected() {
    for index in [json!(true), json!(-1), json!(1), json!(0.0), json!("0")] {
        let result = TabSnapshot::from_json(&json!({"uri": "home:", "index": index}));

        assert_eq!(result, Err(WindowStateError::HistoryPosition), "{index}");
    }
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_mismatching_history_resets_safely`
/// parity: TAB-038
#[test]
fn mismatching_history_resets_safely() {
    let state = json!({"uri": "home:", "history": ["network:"], "index": 0});

    let snapshot = TabSnapshot::from_json(&state).unwrap();

    assert_eq!(snapshot.history, [HOME_URI]);
    assert_eq!(snapshot.index, 0);
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_history_size_limit`
/// parity: TAB-038, NAV-005
#[test]
fn history_size_limit() {
    let longest = TabSnapshot::from_json(&json!({"history": vec!["home:"; 200]}));
    let too_long = TabSnapshot::from_json(&json!({"history": vec!["home:"; 201]}));
    let empty = TabSnapshot::from_json(&json!({"history": []}));

    assert_eq!(longest.unwrap().index, 199);
    assert_eq!(too_long, Err(WindowStateError::HistoryLength));
    assert_eq!(empty, Err(WindowStateError::HistoryLength));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_infinite_scroll_rejected`
///
/// JSON has no infinity, so Python's `float('inf')` arrives as text.
/// parity: TAB-038
#[test]
fn infinite_scroll_rejected() {
    for scroll in [json!("inf"), json!("-Infinity"), json!("nan")] {
        let result = TabSnapshot::from_json(&json!({"scroll": scroll}));

        assert_eq!(result, Err(WindowStateError::ScrollPosition), "{scroll}");
    }
    let clamped = TabSnapshot::from_json(&json!({"scroll": 5e9})).unwrap();
    assert!((clamped.scroll - MAX_SCROLL).abs() < f64::EPSILON);
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_selection_size_limit`
/// parity: TAB-038
#[test]
fn selection_size_limit() {
    let too_many = TabSnapshot::from_json(&json!({"selection": vec!["/tmp/a"; 10_001]}));
    let not_a_list = TabSnapshot::from_json(&json!({"selection": "/tmp/a"}));

    assert_eq!(too_many, Err(WindowStateError::Selection));
    assert_eq!(not_a_list, Err(WindowStateError::Selection));
}

/// A selected item is a file location, never one of the app's pages.
/// parity: TAB-038
#[test]
fn selected_items_are_file_locations() {
    let selected = TabSnapshot::from_json(&json!({"selection": ["/tmp/a b.txt"]})).unwrap();
    let page = TabSnapshot::from_json(&json!({"selection": ["settings:"]}));
    let number = TabSnapshot::from_json(&json!({"selection": [5]}));

    assert_eq!(selected.selection, ["file:///tmp/a%20b.txt"]);
    assert!(matches!(page, Err(WindowStateError::Location(_))));
    let message = number.unwrap_err().to_string();
    assert_eq!(message, "Enter a local folder path or an SMB address.");
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_settings_supported`
/// parity: TAB-038
#[test]
fn settings_supported() {
    let state = json!({"uri": "settings:", "settingsSection": "brave"});

    let snapshot = TabSnapshot::from_json(&state).unwrap();

    assert_eq!(snapshot.uri, SETTINGS_URI);
    assert_eq!(snapshot.settings_section, Some(SettingsSection::Brave));
    let unknown = TabSnapshot::from_json(&json!({"settingsSection": "danger"})).unwrap();
    assert_eq!(unknown.settings_section, None);
}

/// Unknown or missing view, sort and direction fall back to details, name
/// and ascending; only a JSON `true` sorts descending.
/// parity: TAB-038
#[test]
fn unknown_view_and_sort_fall_back_to_the_defaults() {
    let state = json!({"view": "tiles", "sort": "owner", "descending": "yes"});

    let snapshot = TabSnapshot::from_json(&state).unwrap();

    assert_eq!(snapshot.uri, HOME_URI);
    assert_eq!(snapshot.view, TabView::Details);
    assert_eq!(snapshot.sort, SortField::Name);
    assert_eq!(snapshot.direction, SortDirection::Ascending);
    assert_eq!(
        TabSnapshot::from_json(&json!([])),
        Err(WindowStateError::InvalidTab)
    );
    let not_text = TabSnapshot::from_json(&json!({"uri": 7}));
    assert_eq!(not_text, Err(WindowStateError::LocationNotText));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_showitems_keeps_file_path`
/// parity: INT-014, SAFE-017
#[test]
fn showitems_keeps_file_path() {
    let request = FileManagerRequest::new("ShowItems", &["file:///tmp/movie.mp4"]).unwrap();

    assert_eq!(request.method, FileManagerMethod::ShowItems);
    assert_eq!(request.uris, ["file:///tmp/movie.mp4"]);
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_showfolders_and_properties`
/// parity: INT-014
#[test]
fn showfolders_and_properties() {
    for name in ["ShowFolders", "ShowItemProperties"] {
        let request = FileManagerRequest::new(name, &["smb://nas/work"]).unwrap();

        assert_eq!(request.method.dbus_name(), name);
        assert_eq!(request.uris, ["smb://nas/work"]);
    }
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_unsupported_method_rejected`
/// parity: SAFE-017
#[test]
fn unsupported_method_rejected() {
    let result = FileManagerRequest::new("Execute", &["/tmp/script"]);

    assert_eq!(result, Err(WindowStateError::UnsupportedMethod));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_empty_request_rejected`
/// parity: SAFE-017
#[test]
fn empty_request_rejected() {
    let result = FileManagerRequest::new("ShowItems", &[] as &[&str]);

    assert_eq!(result, Err(WindowStateError::RequestLength));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_reveal_limit`
/// parity: SAFE-017
#[test]
fn reveal_limit() {
    let most = FileManagerRequest::new("ShowItems", &vec!["/tmp/x"; 100]);
    let too_many = FileManagerRequest::new("ShowItems", &vec!["/tmp/x"; 101]);

    assert_eq!(most.unwrap().uris.len(), 100);
    assert_eq!(too_many, Err(WindowStateError::RequestLength));
}

/// Ported from `desktop/tests/test_v07.py::HandoffTests::test_no_virtual_locations_in_external_requests`
/// parity: SAFE-017
#[test]
fn no_virtual_locations_in_external_requests() {
    for uri in [
        "settings:",
        "home:",
        NETWORK_URI,
        "https://example.invalid/",
        "smb://user:secret@nas/work",
    ] {
        let result = FileManagerRequest::new("ShowFolders", &[uri]);

        assert!(matches!(result, Err(WindowStateError::Location(_))), "{uri}");
    }
}

/// Shell text in a location stays literal data.
/// parity: SAFE-017
#[test]
fn shell_text_in_a_location_stays_literal() {
    let request = FileManagerRequest::new("ShowItems", &["/tmp/a;touch b"]).unwrap();

    assert_eq!(request.uris, ["file:///tmp/a%3Btouch%20b"]);
}
