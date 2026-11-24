// SPDX-License-Identifier: AGPL-3.0-only
//! Settings > General > "Back up settings": one readable JSON file that
//! carries the app's settings to another computer, and reading it back.
//!
//! The file holds what the user chooses in Settings and in the navigation
//! pane: every preference of `settings.json` except those that belong to
//! this computer ([`COMPUTER_PREFERENCES`]), the Quick access pins, the
//! standard folders unpinned from it and its order, and the saved network
//! folders. It never holds the recent files, which are history, and
//! nothing that is kept outside `settings.json`: passwords and other
//! keyring entries, the addresses typed in the address bar, the folders
//! visited lately, the thumbnail cache, or the opt-in to passwordless
//! updates.
//!
//! ```json
//! {
//!   "kind": "openxplorer-settings",
//!   "format": 1,
//!   "exportedBy": "OpenXplorer 2.0.0",
//!   "pins": [{ "uri": "file:///home/demo/Projects", "label": "Projects" }],
//!   "shares": [],
//!   "hiddenQuick": [],
//!   "quickOrder": [],
//!   "preferences": { "theme": "dark", "view": "details" }
//! }
//! ```
//!
//! Reading it back replaces those settings and keeps the rest. The file is
//! read as `settings.json` is ([`super::read`]): keys this version does not
//! know are ignored, values out of range keep their defaults, and a file
//! of a newer [`FORMAT`] gives what this version understands of it. A file
//! that is not JSON, not a settings file of this app, or has a section of
//! the wrong type changes nothing. Folders and files that are not on this
//! computer are kept as they are, and [`missing_paths`] names them.

use std::path::PathBuf;

use gio::prelude::*;
use serde::Serialize;
use serde_json::{Map, Value};

use super::model::SettingsData;
use super::preferences::Preferences;
use super::{read, Settings, SettingsError};

/// The format of the file this version writes. A reader takes any format
/// from 1 up and keeps what it understands.
pub const FORMAT: u64 = 1;

/// What the file says it is, so another JSON file is not mistaken for it.
pub const KIND: &str = "openxplorer-settings";

/// The name the Save dialog suggests.
pub const SUGGESTED_FILE_NAME: &str = "OpenXplorer settings.json";

/// The largest file read, in bytes: as large as `settings.json` may be.
pub const SIZE_LIMIT: u64 = 4 * 1024 * 1024;

/// The preferences that stay on this computer, by their key in
/// `settings.json`: the size windows and dialogs open at, the daily update
/// check, which is an opt-in of each computer, with when it last ran and
/// the release whose notice was dismissed, the GitHub star prompt's count,
/// and the installed service actions turned on, which are keyed by the
/// files of this computer and stay opt-in on each one.
pub const COMPUTER_PREFERENCES: [&str; 8] = [
    "windowSize",
    "dialogSize",
    "updateChecks",
    "updateCheckedAt",
    "updateNoticeDismissed",
    "starPromptLaunches",
    "starPromptDismissed",
    "enabledServiceActions",
];

/// The sections of the file besides the preferences, in the order
/// `settings.json` lists them.
const SECTIONS: [&str; 4] = ["pins", "shares", "hiddenQuick", "quickOrder"];

/// The file's layout: what it is first, then the sections in the order
/// `settings.json` lists them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileLayout {
    kind: &'static str,
    format: u64,
    exported_by: String,
    pins: Value,
    shares: Value,
    hidden_quick: Value,
    quick_order: Value,
    preferences: Map<String, Value>,
}

/// The settings of `data` that the file carries, as its text: pretty
/// JSON with a final newline.
///
/// # Panics
///
/// Never: the data holds only strings, numbers, flags and lists.
pub fn export_text(data: &SettingsData) -> String {
    let mut stored = data.to_json();
    let mut section = |key: &str| stored.get_mut(key).map(Value::take).unwrap_or_default();
    let (pins, shares, hidden_quick, quick_order) = (
        section("pins"),
        section("shares"),
        section("hiddenQuick"),
        section("quickOrder"),
    );
    let mut preferences = match section("preferences") {
        Value::Object(preferences) => preferences,
        _ => Map::new(),
    };
    for key in COMPUTER_PREFERENCES {
        preferences.remove(key);
    }
    let file = FileLayout {
        kind: KIND,
        format: FORMAT,
        exported_by: format!("OpenXplorer {}", env!("CARGO_PKG_VERSION")),
        pins,
        shares,
        hidden_quick,
        quick_order,
        preferences,
    };
    let mut text = serde_json::to_string_pretty(&file).expect("JSON values only");
    text.push('\n');
    text
}

/// Whether `text` is a settings file this version can import, before the
/// user is asked to replace anything.
///
/// # Errors
///
/// The errors of [`imported`].
pub fn check(text: &str) -> Result<(), SettingsError> {
    imported(&SettingsData::default(), text).map(|_| ())
}

/// `current` with the settings of the file `text` in place of its own:
/// the file's pins, shares, Quick access order and preferences, and
/// `current`'s recent files and [`COMPUTER_PREFERENCES`]. A setting the
/// file leaves out takes its default, as the file leaves out the settings
/// at their defaults.
///
/// # Errors
///
/// [`SettingsError::Invalid`] with a message for the user when `text` is
/// not JSON, not a settings file of this app, or has a section of the
/// wrong type.
pub fn imported(current: &SettingsData, text: &str) -> Result<SettingsData, SettingsError> {
    let file = read_file(text)?;
    let stored = current.to_json();
    let mut source = Map::new();
    for section in SECTIONS {
        if let Some(value) = file.get(section) {
            source.insert(section.to_owned(), value.clone());
        }
    }
    source.insert("recent".to_owned(), stored["recent"].clone());
    let mut preferences = match file.get("preferences") {
        None => Map::new(),
        Some(Value::Object(preferences)) => preferences.clone(),
        Some(_) => return Err(damaged()),
    };
    for key in COMPUTER_PREFERENCES {
        preferences.remove(key);
        if let Some(value) = stored["preferences"].get(key) {
            preferences.insert(key.to_owned(), value.clone());
        }
    }
    source.insert("preferences".to_owned(), Value::Object(preferences));
    let mut data = SettingsData::default();
    read::read_sections(&Value::Object(source), &mut data).map_err(|_| damaged())?;
    Ok(data)
}

/// The file's sections, once it is known to be a settings file of this
/// app, of format 1 or later.
fn read_file(text: &str) -> Result<Map<String, Value>, SettingsError> {
    let Ok(Value::Object(file)) = serde_json::from_str::<Value>(text) else {
        return Err(foreign());
    };
    let is_settings = file.get("kind").and_then(Value::as_str) == Some(KIND);
    let format = file.get("format").and_then(Value::as_u64);
    if !is_settings || format.is_none_or(|format| format < 1) {
        return Err(foreign());
    }
    Ok(file)
}

/// The error of a file that is not one of these.
fn foreign() -> SettingsError {
    SettingsError::invalid(crate::i18n::gettext(
        "This is not an OpenXplorer settings file. Nothing was changed.",
    ))
}

/// The error of a settings file whose contents cannot be read.
fn damaged() -> SettingsError {
    SettingsError::invalid(crate::i18n::gettext(
        "This settings file is damaged. Nothing was changed.",
    ))
}

/// The folders and files `data` names that are not on this computer: the
/// pinned folders, Favorites, Gallery's folders, the folder new windows
/// open to, the Windows icon files and symbol font. Network folders are
/// not looked at, since looking would connect to them. They are kept as
/// they are, and the app names them after an import.
pub fn missing_paths(data: &SettingsData) -> Vec<String> {
    let preferences: &Preferences = &data.preferences;
    let locations = data
        .pins
        .iter()
        .map(|pin| pin.uri.as_str())
        .chain(preferences.favorites.iter().map(String::as_str))
        .chain(preferences.gallery_folders.iter().map(String::as_str))
        .chain(preferences.startup_folder.as_deref())
        .filter_map(|uri| {
            gio::File::for_uri(uri)
                .path()
                .filter(|_| uri.starts_with("file:"))
        });
    let files = preferences
        .icon_files
        .iter()
        .chain(preferences.symbol_font.as_ref())
        .map(PathBuf::from);
    let mut missing = Vec::new();
    for path in locations.chain(files) {
        let shown = path.display().to_string();
        if !path.exists() && !missing.contains(&shown) {
            missing.push(shown);
        }
    }
    missing
}

impl Settings {
    /// Replaces the settings with those of the file `text` (see
    /// [`imported`]) and saves them; returns the folders and files they
    /// name that are not on this computer ([`missing_paths`]).
    ///
    /// # Errors
    ///
    /// The errors of [`imported`], which change nothing, and every error
    /// of [`update_preferences`](Self::update_preferences).
    pub fn import(&mut self, text: &str) -> Result<Vec<String>, SettingsError> {
        self.mutate(|data| {
            *data = imported(data, text)?;
            Ok(missing_paths(data))
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::settings::{Bookmark, PreferencesUpdate, RecentEntry};

    /// Settings with a pin, a share and a few changed preferences,
    /// including some that belong to this computer.
    fn mine() -> SettingsData {
        let mut data = SettingsData {
            pins: vec![Bookmark {
                uri: "file:///home/demo/Projects".into(),
                label: "Projects".into(),
            }],
            shares: vec![Bookmark {
                uri: "smb://nas.example/projects".into(),
                label: "Projects (Z:)".into(),
            }],
            ..SettingsData::default()
        };
        let update = PreferencesUpdate::from_json(&json!({
            "theme": "dark",
            "showDriveLetters": false,
            "windowSize": {"width": 900, "height": 700, "maximized": false},
            "updateChecks": false,
            "starPromptLaunches": 4,
        }))
        .unwrap();
        data.preferences.apply(&update);
        data
    }

    /// The file names what it is and its format, and leaves out what
    /// belongs to this computer and the history.
    #[test]
    fn the_file_holds_the_choices_and_not_this_computers_state() {
        let mut data = mine();
        data.recent.push(RecentEntry {
            uri: "file:///home/demo/notes.txt".into(),
            name: "notes.txt".into(),
            type_label: "Text".into(),
            is_dir: false,
            size: 1,
            modified: 1,
            opened: None,
        });
        let text = export_text(&data);
        let file: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(file["kind"], KIND);
        assert_eq!(file["format"], 1);
        assert_eq!(file["pins"][0]["label"], "Projects");
        assert_eq!(file["shares"][0]["label"], "Projects (Z:)");
        assert_eq!(file["preferences"]["theme"], "dark");
        assert_eq!(file["preferences"]["showDriveLetters"], false);
        assert!(file.get("recent").is_none(), "{text}");
        for key in COMPUTER_PREFERENCES {
            assert!(file["preferences"].get(key).is_none(), "{key} in {text}");
        }
        assert!(text.ends_with("}\n"));
        let first_keys: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("  \""))
            .map(|line| line.trim().split('"').nth(1).unwrap())
            .collect();
        assert_eq!(
            first_keys,
            [
                "kind",
                "format",
                "exportedBy",
                "pins",
                "shares",
                "hiddenQuick",
                "quickOrder",
                "preferences"
            ]
        );
        assert_eq!(check(&text).ok(), Some(()));
        assert!(check("{}").is_err());
    }

    /// Reading the file back gives its settings and keeps this
    /// computer's own; a setting the file leaves out goes back to its
    /// default.
    #[test]
    fn importing_replaces_the_choices_and_keeps_this_computers_state() {
        let text = export_text(&mine());
        let mut here = SettingsData::default();
        let update = PreferencesUpdate::from_json(&json!({
            "theme": "light",
            "showHidden": true,
            "windowSize": {"width": 1200, "height": 800, "maximized": true},
        }))
        .unwrap();
        here.preferences.apply(&update);

        let after = imported(&here, &text).unwrap();
        assert_eq!(after.pins, mine().pins);
        assert_eq!(after.shares, mine().shares);
        assert_eq!(after.preferences.theme, mine().preferences.theme);
        assert!(!after.preferences.show_drive_letters);
        assert!(!after.preferences.show_hidden, "not in the file: the default");
        assert_eq!(after.preferences.window_size, here.preferences.window_size);
        assert!(after.preferences.update_checks, "this computer's opt-in stays");
        assert_eq!(after.preferences.star_prompt_launches, 0);
    }

    /// Keys this version does not know are skipped, and a newer format
    /// gives what this version understands.
    #[test]
    fn a_newer_file_imports_what_this_version_knows() {
        let text = json!({
            "kind": KIND,
            "format": 7,
            "somethingNew": {"a": 1},
            "preferences": {"theme": "dark", "aSettingFromLater": true, "textSize": 9999},
        })
        .to_string();
        let after = imported(&SettingsData::default(), &text).unwrap();
        assert_eq!(after.preferences.theme, mine().preferences.theme);
        assert_eq!(after.preferences.text_size, Preferences::default().text_size);
    }

    /// Another JSON file, text that is not JSON and a section of the
    /// wrong type are refused with a message.
    #[test]
    fn a_broken_or_foreign_file_is_refused() {
        let foreign = [
            "not json",
            "[1, 2]",
            r#"{"theme": "dark"}"#,
            r#"{"kind": "openxplorer-settings"}"#,
            r#"{"kind": "openxplorer-settings", "format": 0}"#,
            r#"{"kind": "openxplorer-settings", "format": "1"}"#,
            r#"{"kind": "something-else", "format": 1}"#,
        ];
        for text in foreign {
            let error = imported(&mine(), text).expect_err(text).to_string();
            assert!(
                error.contains("not an OpenXplorer settings file"),
                "{text}: {error}"
            );
        }
        let damaged = [
            r#"{"kind": "openxplorer-settings", "format": 1, "pins": 5}"#,
            r#"{"kind": "openxplorer-settings", "format": 1, "preferences": []}"#,
        ];
        for text in damaged {
            let error = imported(&mine(), text).expect_err(text).to_string();
            assert!(error.contains("damaged"), "{text}: {error}");
        }
    }

    /// Every preference of `settings.json` at a value other than its
    /// default, so each one is written and each one can be told apart.
    fn every_preference_changed() -> Value {
        serde_json::from_str(
            r#"{
            "theme": "dark", "view": "grid", "details": true, "showHidden": true,
            "autoIndex": false, "contextMenu": "win10", "classicMenuChosen": true,
            "itemCountsChosen": true, "remotePreviewsChosen": true,
            "menuStripPosition": "top", "columnHeaderStyle": "windows", "columnTitleLine": false,
            "shiftF10Classic": true, "f6LikeWindows": true, "networkInterval": 300,
            "showDriveLetters": false, "textSize": 125, "sidebarWidth": 300,
            "columnWidths": {"name": 320, "size": 90},
            "windowSize": {"width": 900, "height": 700, "maximized": true},
            "dialogSize": {"width": 800, "height": 600, "maximized": false},
            "showFullPath": true, "editableLocation": true, "externalFoldersInNewWindow": true,
            "browseArchives": false, "zipOpening": "window",
            "detailsPaneOptions": {"followHover": true, "hiddenFields": ["Size"]},
            "viewOptions": {"showPreviews": false, "countFolderItems": true, "remotePreviews": true,
                            "detailsColumns": ["size", "created"]},
            "fullPathInTitle": true, "confirmTrash": true, "confirmDelete": false,
            "confirmEmptyTrash": false, "deleteRemovablePermanently": true,
            "confirmCloseTabs": true, "askToRunPrograms": true, "runInstalledLaunchers": true,
            "progressDetails": true, "recycleReplacedFiles": false, "restoreOverOriginal": true,
            "extensionWordStarts": true, "showAllSearchMatches": true,
            "neutralSelectionColors": true, "recentFilesInNavigationPane": false,
            "connectExternalShares": true, "modalDialogsOnGnome": true,
            "allowLinuxNameCharacters": true, "searchSystemDirectories": false,
            "searchCompressedFiles": true, "enabledServiceActions": ["0123abcd"],
            "desktopFont": true, "hideSidebar": true, "hideExpandArrows": true,
            "moreNavigationPaneItems": false, "moreTabMenuItems": false, "sidebarIconSize": 32,
            "hiddenSidebarSections": ["network"],
            "hiddenSidebarPlaces": ["file:///media/demo/USB"], "openTabsAtEnd": true,
            "homeQuickAccess": true, "favorites": ["file:///home/demo/Plans/q3.txt"],
            "homeFolded": ["recent"], "galleryFolders": ["file:///home/demo/Photos"],
            "useIconFiles": true, "iconFiles": ["/mnt/windows/imageres.dll.mun"],
            "symbolFont": "/mnt/windows/SegoeIcons.ttf", "fileIcons": {"zip": "zip-document"},
            "restoreSession": true, "startupFolder": "file:///home/demo/Plans",
            "beginInSplitView": true, "tabSwitchesSplitPanes": true,
            "folderTree": {"shown": true, "showHidden": true},
            "absoluteDates": false, "perFolderViews": false, "selectionMarker": true,
            "itemChecksOnCtrl": false, "expandableFolders": true, "compactView": true,
            "showExtensions": true, "previewPane": true, "openFoldersInOwnWindow": true,
            "hideInfotips": true, "hideStatusBar": true, "hidePreviewHandlers": true,
            "typeIntoSearchBox": true, "searchContents": true, "showSearchFilterBar": true,
            "pastePicturesAsFiles": false, "copyPathsAsText": false,
            "viewDefaults": {"mode": "icons", "iconSize": 96, "sort": "modified",
                             "descending": true, "groups": false, "foldersFirst": false},
            "folderViews": [{"uri": "file:///home/demo/Plans", "mode": "details", "iconSize": 48,
                             "sort": "size", "descending": false, "groups": true,
                             "foldersFirst": true}],
            "downloadsGroupBy": "type", "updateChecks": false, "showUpdateButton": false,
            "updateCheckedAt": 1700000000, "updateNoticeDismissed": "2.0.1",
            "starPromptLaunches": 3, "starPromptDismissed": true
        }"#,
        )
        .expect("JSON")
    }

    /// How many fields [`Preferences`] has, read from its source, so the
    /// round trip below fails when a new preference is not in it.
    fn preference_fields() -> usize {
        let source = include_str!("preferences.rs");
        let start = source.find("pub struct Preferences {").expect("the struct");
        let body = &source[start..];
        let body = &body[..body.find("\n}\n").expect("its end")];
        body.lines().filter(|line| line.starts_with("    pub ")).count()
    }

    /// The round trip: settings with every preference changed, pins,
    /// shares and a Quick access order are exported; then everything is
    /// changed; importing the file brings every exported setting back,
    /// and this computer's own (the window size, the update check, the
    /// recent files) stay as they were just before the import.
    ///
    /// parity: SET-021
    #[test]
    fn export_change_everything_import_and_everything_is_back() {
        let folder = tempfile::tempdir().unwrap();
        let first = json!({
            "version": 2,
            "pins": [{"uri": "file:///home/demo/Plans", "label": "Plans"},
                     {"uri": "file:///home/demo/Photos", "label": "Photos"}],
            "shares": [{"uri": "smb://nas.example/projects", "label": "Projects (Z:)"}],
            "hiddenQuick": ["file:///home/demo/Music"],
            "quickOrder": ["file:///home/demo/Photos", "file:///home/demo/Plans"],
            "recent": [],
            "preferences": every_preference_changed(),
        });
        std::fs::write(folder.path().join(Settings::FILE_NAME), first.to_string()).unwrap();
        let mut settings = Settings::open(folder.path());
        assert_eq!(settings.warning(), None);
        let before = settings.snapshot();
        let stored = serde_json::to_value(&before.preferences).unwrap();
        let stored = stored.as_object().unwrap();
        assert_eq!(
            stored.len(),
            preference_fields(),
            "every preference is changed, so every one is written: {:?}",
            stored.keys().collect::<Vec<_>>()
        );
        let exported = export_text(&before);

        // Everything changes: other pins and shares, no order, every
        // preference back at its default, and this computer's own.
        let second = json!({
            "version": 2,
            "pins": [{"uri": "file:///home/demo/Other", "label": "Other"}],
            "shares": [],
            "hiddenQuick": [],
            "quickOrder": [],
            "recent": [{"uri": "file:///home/demo/notes.txt", "name": "notes.txt",
                        "type": "Text", "isDir": false, "size": 1, "modified": 2}],
            "preferences": {"windowSize": {"width": 1200, "height": 800, "maximized": false},
                            "updateChecks": true},
        });
        std::fs::write(folder.path().join(Settings::FILE_NAME), second.to_string()).unwrap();
        let changed = settings.snapshot();
        let changed_file: Value = serde_json::from_str(&export_text(&changed)).unwrap();
        let exported_file: Value = serde_json::from_str(&exported).unwrap();
        for section in SECTIONS {
            assert_ne!(changed_file[section], exported_file[section], "{section} changed");
        }
        for (key, value) in exported_file["preferences"].as_object().unwrap() {
            assert_ne!(changed_file["preferences"].get(key), Some(value), "{key} changed");
        }

        let missing = settings.import(&exported).unwrap();
        assert!(!missing.is_empty(), "the demo folders are not on this computer");

        let after = Settings::open(folder.path()).data().clone();
        assert_eq!(export_text(&after), exported, "every exported setting is back");
        let mut expected = before.preferences.clone();
        expected.window_size = changed.preferences.window_size;
        expected.dialog_size = changed.preferences.dialog_size;
        expected.update_checks = changed.preferences.update_checks;
        expected.update_checked_at = changed.preferences.update_checked_at;
        expected.update_notice_dismissed = changed.preferences.update_notice_dismissed.clone();
        expected.star_prompt_launches = changed.preferences.star_prompt_launches;
        expected.star_prompt_dismissed = changed.preferences.star_prompt_dismissed;
        expected.enabled_service_actions = changed.preferences.enabled_service_actions.clone();
        assert_eq!(after.preferences, expected);
        assert_eq!(
            (
                &after.pins,
                &after.shares,
                &after.hidden_quick,
                &after.quick_order
            ),
            (
                &before.pins,
                &before.shares,
                &before.hidden_quick,
                &before.quick_order
            )
        );
        assert_eq!(after.recent, changed.recent, "the history stays this computer's");
    }

    /// A pin or icon file that is not on this computer is kept and
    /// named; network folders are not looked at.
    #[test]
    fn paths_not_on_this_computer_are_kept_and_named() {
        let here = tempfile::tempdir().unwrap();
        let present = here.path().join("present");
        std::fs::create_dir(&present).unwrap();
        let gone = here.path().join("gone");
        let mut data = mine();
        data.pins = vec![
            Bookmark {
                uri: gio::File::for_path(&present).uri().into(),
                label: "Present".into(),
            },
            Bookmark {
                uri: gio::File::for_path(&gone).uri().into(),
                label: "Gone".into(),
            },
        ];
        data.preferences.icon_files = vec!["/nowhere/imageres.dll".into()];
        let text = export_text(&data);

        let after = imported(&SettingsData::default(), &text).unwrap();
        assert_eq!(after.pins.len(), 2, "kept");
        assert_eq!(after.preferences.icon_files, ["/nowhere/imageres.dll"]);
        assert_eq!(
            missing_paths(&after),
            [gone.display().to_string(), "/nowhere/imageres.dll".to_owned()]
        );
    }
}
