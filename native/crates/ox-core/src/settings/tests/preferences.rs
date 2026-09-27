// SPDX-License-Identifier: AGPL-3.0-only
//! Preference persistence, validation and cross-window updates.

use super::*;

/// Ported from desktop/tests/test_v05.py::SettingsWindowsTests::test_default_context_menu_classic
#[test]
fn default_context_menu_classic() {
    let root = temp();
    assert_eq!(
        Settings::open(root.path()).snapshot().preferences.context_menu,
        "win10"
    );
}

/// Ported from desktop/tests/test_v05.py::SettingsWindowsTests::test_network_interval_whitelist
#[test]
fn network_interval_whitelist() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"networkInterval": 30})))
        .unwrap();
    store
        .update_preferences(&prefs(json!({"networkInterval": 1})))
        .unwrap();
    assert_eq!(store.snapshot().preferences.network_interval, 30);
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_layout_persists
#[test]
fn layout_persists() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(
            json!({"sidebarWidth": 333, "columnWidths": {"name": 460, "size": 100}}),
        ))
        .unwrap();
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reread.sidebar_width, Some(333));
    let columns = serde_json::to_value(reread.column_widths).unwrap();
    assert_eq!(columns, json!({"name": 460, "size": 100}));
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_sidebar_width_bounds
#[test]
fn sidebar_width_bounds() {
    let root = temp();
    let mut store = Settings::open(root.path());
    let rejected = [
        json!(-1),
        json!(139),
        json!(561),
        json!(null),
        json!(true),
        json!("350"),
    ];
    for value in rejected {
        store
            .update_preferences(&prefs(json!({"sidebarWidth": value})))
            .unwrap();
        assert_eq!(store.snapshot().preferences.sidebar_width, None, "{value}");
    }
    let nan = PreferencesUpdate {
        sidebar_width: Some(f64::NAN),
        ..PreferencesUpdate::default()
    };
    store.update_preferences(&nan).unwrap();
    assert_eq!(store.snapshot().preferences.sidebar_width, None);
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_width_rounding
#[test]
fn width_rounding() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"sidebarWidth": 280.4})))
        .unwrap();
    assert_eq!(store.snapshot().preferences.sidebar_width, Some(280));
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_columns_whitelist
#[test]
fn columns_whitelist() {
    let root = temp();
    let mut store = Settings::open(root.path());
    let update = json!({"columnWidths": {"name": 150, "css": "url(bad)", "size": 99999, "type": true}});
    store.update_preferences(&prefs(update)).unwrap();
    let columns = serde_json::to_value(store.snapshot().preferences.column_widths).unwrap();
    assert_eq!(columns, json!({"name": 150}));
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_column_reset
#[test]
fn column_reset() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"columnWidths": {"name": 700}})))
        .unwrap();
    store
        .update_preferences(&prefs(json!({"columnWidths": {}})))
        .unwrap();
    let columns = serde_json::to_value(store.snapshot().preferences.column_widths).unwrap();
    assert_eq!(columns, json!({}));
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_other_preferences_retained
#[test]
fn other_preferences_retained() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"theme": "dark", "contextMenu": "win11"})))
        .unwrap();
    store
        .update_preferences(&prefs(json!({"sidebarWidth": 300})))
        .unwrap();
    let current = store.snapshot().preferences;
    assert_eq!(
        (current.theme.as_str(), current.context_menu.as_str()),
        ("dark", "win11")
    );
}

/// Ported from desktop/tests/test_v06.py::PrefTests::test_partial_window_updates_do_not_remove_other_preferences
#[test]
fn partial_window_updates_do_not_remove_other_preferences() {
    let root = temp();
    let mut store = Settings::open(root.path());
    let mut other = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"sidebarWidth": 270})))
        .unwrap();
    other
        .update_preferences(&prefs(json!({"columnWidths": {"modified": 200}})))
        .unwrap();
    let merged = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(merged.sidebar_width, Some(270));
    assert_eq!(merged.column_widths.unwrap().get(Column::Modified), Some(200));
}

/// Ported from desktop/tests/test_zip_extract.py::TextPreferenceTests::test_default_round_trip
#[test]
fn text_size_default_round_trip() {
    let root = temp();
    let mut store = Settings::open(root.path());
    assert_eq!(store.snapshot().preferences.text_size, 100);
    store
        .update_preferences(&prefs(json!({"textSize": 150})))
        .unwrap();
    assert_eq!(Settings::open(root.path()).snapshot().preferences.text_size, 150);
}

/// Ported from desktop/tests/test_zip_extract.py::TextPreferenceTests::test_invalid_values_ignored
#[test]
fn text_size_invalid_values_ignored() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"textSize": 125})))
        .unwrap();
    let rejected = [
        json!(true),
        json!(false),
        json!("150"),
        json!(150.0),
        json!(0),
        json!(201),
        json!(-1),
        json!(10000),
        json!(101),
        json!(null),
        json!({}),
    ];
    for value in rejected {
        store
            .update_preferences(&prefs(json!({"textSize": value})))
            .unwrap();
        assert_eq!(store.snapshot().preferences.text_size, 125, "{value}");
    }
}

/// Ported from desktop/tests/test_zip_extract.py::TextPreferenceTests::test_all_sizes
#[test]
fn text_size_all_sizes() {
    let root = temp();
    let mut store = Settings::open(root.path());
    for size in TEXT_SIZES {
        let result = store
            .update_preferences(&prefs(json!({"textSize": size})))
            .unwrap();
        assert_eq!(result.text_size, size);
    }
}

/// Ported from desktop/tests/test_zip_extract.py::TextPreferenceTests::test_preserves_other_settings
#[test]
fn text_size_preserves_other_settings() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(
            json!({"theme": "dark", "sidebarWidth": 310, "showHidden": true}),
        ))
        .unwrap();
    store
        .update_preferences(&prefs(json!({"textSize": 150})))
        .unwrap();
    let current = store.snapshot().preferences;
    assert_eq!(current.theme, "dark");
    assert_eq!(current.sidebar_width, Some(310));
    assert!(current.show_hidden);
}

/// Ported from desktop/tests/test_zip_extract.py::TextPreferenceTests::test_multiple_instances_merge
#[test]
fn text_size_multiple_instances_merge() {
    let root = temp();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    first
        .update_preferences(&prefs(json!({"textSize": 175})))
        .unwrap();
    second
        .update_preferences(&prefs(json!({"theme": "dark"})))
        .unwrap();
    let merged = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(merged.text_size, 175);
    assert_eq!(merged.theme, "dark");
}
