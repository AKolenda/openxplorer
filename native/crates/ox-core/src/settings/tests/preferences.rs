// SPDX-License-Identifier: AGPL-3.0-only
//! Preference persistence, validation and cross-window updates.
//!
//! Ports the preference cases of `v2.0.0:desktop/tests/test_v05.py`,
//! `test_v06.py` and `test_zip_extract.py`.

use super::*;
use crate::grouping::GroupBy;

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_default_context_menu_classic`
/// parity: SET-016
#[test]
fn the_default_context_menu_is_the_classic_one() {
    let root = temporary_folder();
    let preferences = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(preferences.context_menu, ContextMenu::Win10);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_network_interval_whitelist`
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

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_layout_persists`
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

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_sidebar_width_bounds`
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

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_width_rounding`
/// parity: SIDE-023, VIEW-028
#[test]
fn sidebar_widths_are_rounded_when_saved() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"sidebarWidth": 280.4}));
    assert_eq!(store.snapshot().preferences.sidebar_width, Some(280));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_columns_whitelist`
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

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_column_reset`
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

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_other_preferences_retained`
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

/// Ported from `v2.0.0:desktop/tests/test_v06.py::PrefTests::test_partial_window_updates_do_not_remove_other_preferences`
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

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::TextPreferenceTests::test_default_round_trip`
/// parity: VIEW-045
#[test]
fn text_size_defaults_to_100_and_persists() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    assert_eq!(store.snapshot().preferences.text_size, 100);
    save_preferences(&mut store, &json!({"textSize": 150}));
    assert_eq!(Settings::open(root.path()).snapshot().preferences.text_size, 150);
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::TextPreferenceTests::test_invalid_values_ignored`
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

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::TextPreferenceTests::test_all_sizes`
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

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::TextPreferenceTests::test_preserves_other_settings`
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

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::TextPreferenceTests::test_multiple_instances_merge`
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

/// parity: SIDE-022
#[test]
fn sections_and_places_hidden_from_two_windows_merge() {
    let root = temporary_folder();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    first.set_section_hidden("network", true).expect("hidden");
    second.set_section_hidden("drives", true).expect("hidden");
    first.set_section_hidden("drives", false).expect("shown");
    first.set_place_hidden("trash:///", true).expect("hidden");
    second.set_place_hidden("recent:///", true).expect("hidden");
    first.set_place_hidden("trash:///", false).expect("shown");
    let merged = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(merged.hidden_sidebar_sections, ["network"]);
    assert_eq!(merged.hidden_sidebar_places, ["recent:///"]);
}

/// ZIPs open in their window until the user chooses the folder, which is
/// then stored; an unknown value changes nothing.
///
/// parity: ARC-026
#[test]
fn zip_opening_is_the_window_until_the_folder_is_chosen() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    assert_eq!(store.snapshot().preferences.zip_opening, ZipOpening::Window);
    save_preferences(&mut store, &json!({"zipOpening": "folder"}));
    save_preferences(&mut store, &json!({"zipOpening": "popup"}));
    let reopened = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reopened.zip_opening, ZipOpening::Folder);
    save_preferences(&mut store, &json!({"zipOpening": "window"}));
    let file = std::fs::read_to_string(root.path().join("settings.json")).expect("saved");
    assert!(!file.contains("zipOpening"), "the window is not stored");
}

/// Apply to all folders gives Downloads the applied Group by too: the
/// Group by kept for Downloads apart from the shared style would
/// otherwise outlive the change, so Downloads stayed grouped by date.
/// parity: VIEW-020, VIEW-022
#[test]
fn apply_to_all_folders_gives_downloads_the_applied_group_by() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&PreferencesUpdate {
            downloads_group_by: Some(GroupBy::Type),
            ..PreferencesUpdate::default()
        })
        .unwrap();
    let mut applied = ViewProperties::default();
    applied.set_grouping(GroupBy::None);

    let saved = store.remember_view("", applied, ViewScope::AllFolders).unwrap();

    let downloads = "file:///home/someone/Downloads";
    assert_eq!(saved.downloads_group_by, Some(GroupBy::None));
    assert_eq!(
        saved.view_in(downloads, Some(downloads)).grouping(),
        GroupBy::None
    );
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reread.downloads_group_by, Some(GroupBy::None), "saved to disk");
}

/// Reset folders forgets the Group by chosen in Downloads too, so
/// Downloads is grouped by date again, as it is by default.
/// parity: VIEW-020, VIEW-022
#[test]
fn reset_folders_forgets_the_group_by_chosen_in_downloads() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&PreferencesUpdate {
            downloads_group_by: Some(GroupBy::None),
            ..PreferencesUpdate::default()
        })
        .unwrap();

    let reset = store.reset_folder_views().unwrap();

    let downloads = "file:///home/someone/Downloads";
    assert_eq!(reset.downloads_group_by, None);
    assert_eq!(
        reset.view_in(downloads, Some(downloads)).grouping(),
        GroupBy::Modified
    );
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reread.downloads_group_by, None, "saved to disk");
}

/// Views saved before Folder Options replaced the display style dialog
/// still load: a folder saved "and its subfolders" still reaches below it,
/// and the dropped "hidden items last" is ignored. Saving another change
/// keeps them.
/// parity: VIEW-020
#[test]
fn views_saved_by_the_old_display_style_dialog_still_load() {
    let root = temporary_folder();
    let saved = json!({"preferences": {
        "view": "grid",
        "perFolderViews": true,
        "showHidden": false,
        "viewDefaults": {"mode": "icons", "sort": "type", "hiddenLast": true, "showHidden": true,
                         "showPreviews": false},
        "folderViews": [
            {"uri": "file:///home/someone/Projects", "mode": "details", "sort": "size",
             "hiddenLast": true, "subfolders": true, "showHidden": true, "showPreviews": false},
            {"uri": "file:///home/someone/Music", "mode": "compact", "sort": "modified"}
        ]
    }});
    std::fs::write(root.path().join("settings.json"), saved.to_string()).unwrap();

    let mut store = Settings::open(root.path());
    assert!(store.warning().is_none());
    let preferences = store.snapshot().preferences;
    assert_eq!(preferences.folder_views.len(), 2);
    assert_eq!(
        preferences.view_for("file:///home/someone/Projects/site").sort,
        "size"
    );
    assert_eq!(preferences.view_for("file:///home/someone/Music").mode, "compact");
    assert_eq!(
        preferences.view_for("file:///home/someone/Pictures").mode,
        "icons"
    );
    assert!(
        !preferences.show_hidden,
        "hidden files are one setting, not a folder's"
    );
    assert!(preferences.view_options.show_previews, "and so are thumbnails");
    let style = serde_json::to_value(preferences.view_for("file:///home/someone/Projects")).unwrap();
    assert!(style.get("showHidden").is_none() && style.get("showPreviews").is_none());

    save_preferences(&mut store, &json!({"theme": "dark"}));
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reread.folder_views, preferences.folder_views);
    assert!(reread.folder_views[0].subfolders);
    assert_eq!(reread.view_for("file:///home/someone/Projects/site").sort, "size");
}

/// Reset Folders shows every folder in the default view, Details by name,
/// even when the view the Python app reads was saved as icons; whether
/// hidden files show is not a view of a folder and stays, as in Windows.
/// parity: VIEW-020
#[test]
fn reset_folders_shows_details_by_name_again() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(
        &mut store,
        &json!({"view": "grid", "showHidden": true, "perFolderViews": true}),
    );
    store
        .remember_view(
            "",
            ViewProperties {
                mode: "icons".to_owned(),
                sort: "size".to_owned(),
                ..ViewProperties::default()
            },
            ViewScope::AllFolders,
        )
        .unwrap();

    let reset = store.reset_folder_views().unwrap();

    let style = reset.view_for("file:///home/someone/Pictures");
    assert_eq!(style.mode, "details");
    assert_eq!(style.sort, "name");
    assert!(reset.show_hidden, "hidden files stay as they were");
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!(reread.view_for("file:///home/someone/Pictures").mode, "details");
}

/// "Remember each folder's view settings" is on unless the user turned it
/// off, as in Windows 11: a settings file without the choice gets it on,
/// and turning it off is saved and kept.
/// parity: VIEW-020
#[test]
fn each_folder_remembers_its_view_unless_turned_off() {
    let root = temporary_folder();
    std::fs::write(
        root.path().join("settings.json"),
        r#"{"preferences": {"theme": "dark"}}"#,
    )
    .unwrap();
    let mut store = Settings::open(root.path());
    assert!(store.snapshot().preferences.per_folder_views);
    assert!(Preferences::default().per_folder_views);

    save_preferences(&mut store, &json!({"perFolderViews": false}));
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["preferences"]["perFolderViews"], json!(false));
    assert!(
        !Settings::open(root.path())
            .snapshot()
            .preferences
            .per_folder_views
    );

    save_preferences(&mut store, &json!({"perFolderViews": true}));
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.path().join("settings.json")).unwrap()).unwrap();
    assert!(
        saved["preferences"].get("perFolderViews").is_none(),
        "on is the default"
    );
    assert!(
        Settings::open(root.path())
            .snapshot()
            .preferences
            .per_folder_views
    );
}

/// Turning thumbnails off or on, as Folder Options' "Always show icons,
/// never thumbnails" does with the whole view options, keeps the columns
/// Apply to Folders gave every folder: only a change of the columns
/// themselves reaches the shared view.
/// parity: VIEW-020, VIEW-058
#[test]
fn a_thumbnail_toggle_keeps_the_applied_columns() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    let applied = ViewProperties {
        details_columns: Some(vec!["size".to_owned(), "owner".to_owned()]),
        ..ViewProperties::default()
    };
    store.remember_view("", applied, ViewScope::AllFolders).unwrap();
    let mut options = store.snapshot().preferences.view_options;
    options.show_previews = false;

    let saved = store
        .update_preferences(&PreferencesUpdate {
            view_options: Some(options.clone()),
            ..PreferencesUpdate::default()
        })
        .unwrap();

    let columns = vec!["size".to_owned(), "owner".to_owned()];
    assert_eq!(
        saved.view_for("file:///home/someone/Pictures").details_columns,
        Some(columns)
    );
    options.details_columns = vec!["type".to_owned()];
    let saved = store
        .update_preferences(&PreferencesUpdate {
            view_options: Some(options),
            ..PreferencesUpdate::default()
        })
        .unwrap();
    assert_eq!(
        saved.view_for("file:///home/someone/Pictures").details_columns,
        Some(vec!["type".to_owned()]),
        "choosing the columns in Settings still reaches the shared view"
    );
}
