// SPDX-License-Identifier: AGPL-3.0-only
//! Preference persistence, validation and cross-window updates.
//!
//! Ports the preference cases of `desktop/tests/test_v05.py`,
//! `test_v06.py` and `test_zip_extract.py`.

use super::*;

/// Ported from `desktop/tests/test_v05.py::SettingsWindowsTests::test_default_context_menu_classic`
/// parity: SET-016
#[test]
fn the_default_context_menu_is_the_classic_one() {
    let root = temporary_folder();
    let preferences = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(preferences.context_menu, ContextMenu::Win10);
}

/// Ported from `desktop/tests/test_v05.py::SettingsWindowsTests::test_network_interval_whitelist`
/// parity: SET-016
#[test]
fn a_network_interval_outside_the_whitelist_is_not_saved() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"networkInterval": 30}));
    save_preferences(&mut store, &json!({"networkInterval": 1}));
    assert_eq!(store.snapshot().preferences.network_interval, 30);
}

/// The view starts as details, is saved, and ignores unknown values.
///
/// parity: VIEW-007
#[test]
fn the_view_is_details_grid_or_left_as_it_was() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    assert_eq!(store.snapshot().preferences.view, View::Details);
    save_preferences(&mut store, &json!({"view": "grid"}));
    save_preferences(&mut store, &json!({"view": "tiles"}));
    assert_eq!(store.snapshot().preferences.view, View::Grid);
    let reopened = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reopened.view, View::Grid);
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_layout_persists`
/// parity: SIDE-023, VIEW-028
#[test]
fn sidebar_and_column_widths_persist() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(
        &mut store,
        &json!({"sidebarWidth": 333, "columnWidths": {"name": 460, "size": 100}}),
    );
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reread.sidebar_width, Some(333));
    let columns = serde_json::to_value(reread.column_widths).unwrap();
    assert_eq!(columns, json!({"name": 460, "size": 100}));
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_sidebar_width_bounds`
/// parity: SIDE-023
#[test]
fn sidebar_widths_out_of_bounds_are_ignored() {
    let root = temporary_folder();
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
        save_preferences(&mut store, &json!({"sidebarWidth": value}));
        assert_eq!(store.snapshot().preferences.sidebar_width, None, "{value}");
    }
    let not_a_number = PreferencesUpdate {
        sidebar_width: Some(f64::NAN),
        ..PreferencesUpdate::default()
    };
    store.update_preferences(&not_a_number).unwrap();
    assert_eq!(store.snapshot().preferences.sidebar_width, None);
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_width_rounding`
/// parity: SIDE-023, VIEW-028
#[test]
fn sidebar_widths_are_rounded_when_saved() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"sidebarWidth": 280.4}));
    assert_eq!(store.snapshot().preferences.sidebar_width, Some(280));
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_columns_whitelist`
/// parity: VIEW-028, SAFE-018
#[test]
fn only_known_in_range_column_widths_are_saved() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    let update = json!({"columnWidths": {"name": 150, "css": "url(bad)", "size": 99999, "type": true}});
    save_preferences(&mut store, &update);
    let columns = serde_json::to_value(store.snapshot().preferences.column_widths).unwrap();
    assert_eq!(columns, json!({"name": 150}));
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_column_reset`
/// parity: VIEW-028
#[test]
fn an_empty_column_widths_object_resets_every_column() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"columnWidths": {"name": 700}}));
    save_preferences(&mut store, &json!({"columnWidths": {}}));
    let columns = serde_json::to_value(store.snapshot().preferences.column_widths).unwrap();
    assert_eq!(columns, json!({}));
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_other_preferences_retained`
/// parity: SET-014, SIDE-023
#[test]
fn changing_the_sidebar_width_keeps_other_preferences() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"theme": "dark", "contextMenu": "win11"}));
    save_preferences(&mut store, &json!({"sidebarWidth": 300}));
    let current = store.snapshot().preferences;
    assert_eq!(
        (current.theme, current.context_menu),
        (Theme::Dark, ContextMenu::Win11)
    );
}

/// Ported from `desktop/tests/test_v06.py::PrefTests::test_partial_window_updates_do_not_remove_other_preferences`
/// parity: SET-014, VIEW-028
#[test]
fn partial_window_updates_do_not_remove_other_preferences() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    let mut other = Settings::open(root.path());
    save_preferences(&mut store, &json!({"sidebarWidth": 270}));
    save_preferences(&mut other, &json!({"columnWidths": {"modified": 200}}));
    let merged = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(merged.sidebar_width, Some(270));
    assert_eq!(merged.column_widths.unwrap().get(Column::Modified), Some(200));
}

/// Ported from `desktop/tests/test_zip_extract.py::TextPreferenceTests::test_default_round_trip`
/// parity: VIEW-045
#[test]
fn text_size_defaults_to_100_and_persists() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    assert_eq!(store.snapshot().preferences.text_size, 100);
    save_preferences(&mut store, &json!({"textSize": 150}));
    assert_eq!(Settings::open(root.path()).snapshot().preferences.text_size, 150);
}

/// Ported from `desktop/tests/test_zip_extract.py::TextPreferenceTests::test_invalid_values_ignored`
/// parity: VIEW-045
#[test]
fn invalid_text_sizes_are_ignored() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"textSize": 125}));
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
        save_preferences(&mut store, &json!({"textSize": value}));
        assert_eq!(store.snapshot().preferences.text_size, 125, "{value}");
    }
}

/// Ported from `desktop/tests/test_zip_extract.py::TextPreferenceTests::test_all_sizes`
/// parity: VIEW-045
#[test]
fn every_offered_text_size_is_saved() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    for size in TEXT_SIZES {
        let saved = save_preferences(&mut store, &json!({"textSize": size}));
        assert_eq!(saved.text_size, size);
    }
}

/// Ported from `desktop/tests/test_zip_extract.py::TextPreferenceTests::test_preserves_other_settings`
/// parity: VIEW-045
#[test]
fn changing_the_text_size_keeps_other_preferences() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(
        &mut store,
        &json!({"theme": "dark", "sidebarWidth": 310, "showHidden": true}),
    );
    save_preferences(&mut store, &json!({"textSize": 150}));
    let current = store.snapshot().preferences;
    assert_eq!(current.theme, Theme::Dark);
    assert_eq!(current.sidebar_width, Some(310));
    assert!(current.show_hidden);
}

/// Ported from `desktop/tests/test_zip_extract.py::TextPreferenceTests::test_multiple_instances_merge`
/// parity: VIEW-045, SET-014
#[test]
fn text_size_changes_from_two_windows_merge() {
    let root = temporary_folder();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    save_preferences(&mut first, &json!({"textSize": 175}));
    save_preferences(&mut second, &json!({"theme": "dark"}));
    let merged = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(merged.text_size, 175);
    assert_eq!(merged.theme, Theme::Dark);
}
