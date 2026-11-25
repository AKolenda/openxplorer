// SPDX-License-Identifier: AGPL-3.0-only
//! "Behave like Windows Explorer": one switch that sets every setting
//! whose default is the app's own choice to the value that makes it work
//! as Windows 11 File Explorer does.
//!
//! The switch changes no behavior by itself. It only saves settings, each
//! of which keeps its own row and works on its own:
//!
//! * Turned on, it saves the Windows value of every entry of
//!   [`WINDOWS_SETTINGS`], and remembers what each one was before, stored
//!   as `behaveLikeWindows` (an object of those earlier values). Turned on
//!   again while it is on, it keeps what it remembered.
//! * Turned off, it puts the remembered values back and forgets them.
//! * While it is on, a setting changed on its own row makes it read
//!   [`WindowsMode::Custom`], as a partly ticked box does in Windows, not
//!   silently off.
//!
//! Off by default, so the owner's defaults stay untouched.
//!
//! Each setting with a Windows value is one line of [`WINDOWS_SETTINGS`].
//! The Settings page's tests fail when a row titled "(like Windows)" or
//! "(Windows)" has no line there.

use serde::Serialize;
use serde_json::{Map, Value};

use super::choices::ColumnHeaderStyle;
use super::preferences::{Preferences, PreferencesUpdate};

/// The key `settings.json` stores the earlier values under.
pub const STORED_KEY: &str = "behaveLikeWindows";

/// The most earlier values kept, which is well above the registry's size.
const MAX_REMEMBERED: usize = 64;
/// The longest key of a remembered value.
const MAX_KEY: usize = 64;
/// The longest remembered choice, such as `"openxplorer"`.
const MAX_CHOICE: usize = 32;

/// One setting the switch sets: its key in `settings.json`, the row that
/// shows it, how to read it and the value Windows 11 File Explorer has.
#[derive(Debug, Clone, Copy)]
pub struct WindowsSetting {
    /// The key of `preferences` in `settings.json`, as
    /// [`PreferencesUpdate::from_json`] reads it.
    pub key: &'static str,
    /// The title of the setting's row in Settings, in English.
    pub title: &'static str,
    /// The setting's current value, as `settings.json` writes it.
    pub read: fn(&Preferences) -> Value,
    /// The value that works as Windows 11 File Explorer does.
    pub windows: fn() -> Value,
}

impl WindowsSetting {
    /// Whether `preferences` has this setting at its Windows value.
    pub fn is_windows(&self, preferences: &Preferences) -> bool {
        (self.read)(preferences) == (self.windows)()
    }
}

/// `value` as JSON; every value of the registry is a flag or a choice.
fn json(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// One line of [`WINDOWS_SETTINGS`]: the key in `settings.json`, the
/// field of [`Preferences`], the Windows value and the row's title.
macro_rules! like_windows {
    ($key:literal, $field:ident, $windows:expr, $title:literal) => {
        WindowsSetting {
            key: $key,
            title: $title,
            read: |preferences| json(preferences.$field),
            windows: || json($windows),
        }
    };
}

/// Every setting the switch sets, one line each, in the order of the
/// Settings pages. A new "(like Windows)" setting adds its line here.
#[rustfmt::skip]
pub const WINDOWS_SETTINGS: &[WindowsSetting] = &[
    like_windows!("homeQuickAccess", home_quick_access, true, "Open Quick access on Home"),
    like_windows!("fixedPropertiesSize", fixed_properties_size, true, "Keep Properties windows a fixed size (like Windows)"),
    like_windows!("askForStar", ask_for_star, false, "Ask for a GitHub star"),
    like_windows!("ctrlPlusMinusTextSize", ctrl_plus_minus_text_size, false, "Ctrl+plus and Ctrl+minus change text size"),
    like_windows!("hideExpandArrows", hide_expand_arrows, true, "Hide expand arrows in the sidebar"),
    like_windows!("moreNavigationPaneItems", more_navigation_pane_items, false, "Show more items in the navigation pane"),
    like_windows!("moreTabMenuItems", more_tab_menu_items, false, "Show more items in the tab menu"),
    like_windows!("columnHeaderStyle", column_header_style, ColumnHeaderStyle::Windows, "Column header style"),
    like_windows!("columnTitleLine", column_title_line, false, "Show a line under column titles"),
    like_windows!("nameColumnWindowsWidth", name_column_windows_width, true, "Name column starts at Windows' width (256 px)"),
    like_windows!("recycleReplacedFiles", recycle_replaced_files, false, "When replacing a file, move the old one to the Recycle Bin"),
    like_windows!("deleteRemovablePermanently", delete_removable_permanently, true, "Delete files on removable drives permanently (like Windows)"),
    like_windows!("pastePicturesAsFiles", paste_pictures_as_files, false, "Paste pictures as new files"),
    like_windows!("copyPathsAsText", copy_paths_as_text, false, "Also copy paths as text"),
    like_windows!("f6LikeWindows", f6_like_windows, true, "F6 moves like Windows"),
    like_windows!("runInstalledLaunchers", run_installed_launchers, true, "Run installed apps' launchers on double-click"),
    like_windows!("restoreOverOriginal", restore_over_original, true, "Restore over the original (Windows)"),
    like_windows!("extensionWordStarts", extension_word_starts, true, "Match words at the start of file extensions (like Windows)"),
    like_windows!("showAllSearchMatches", show_all_search_matches, true, "Show all search matches (like Windows)"),
    like_windows!("neutralSelectionColors", neutral_selection_colors, true, "Neutral selection colors (like Windows)"),
    like_windows!("recentFilesInNavigationPane", recent_files_in_navigation_pane, false, "Show Recent files in the navigation pane"),
    like_windows!("connectExternalShares", connect_external_shares, true, "Connect to shares opened by other apps right away (like Windows)"),
    like_windows!("modalDialogsOnGnome", modal_dialogs_on_gnome, true, "Keep Open/Save dialogs modal on GNOME (like Windows)"),
    like_windows!("itemChecksOnCtrl", item_checks_on_ctrl, false, "Show check boxes only while Ctrl is held"),
];

/// A change of the switch, as an update carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsModeChange {
    /// On, with the values from before it was turned on, by key.
    On(Map<String, Value>),
    /// Off: nothing is remembered.
    Off,
}

/// What the switch shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsMode {
    /// Off: the settings are the user's own.
    Off,
    /// On, and every setting has its Windows value.
    On,
    /// On, but a setting was changed on its own row since.
    Custom,
}

/// What the switch shows for `preferences`.
pub fn windows_mode(preferences: &Preferences) -> WindowsMode {
    if preferences.windows_mode_before.is_none() {
        WindowsMode::Off
    } else if differing(preferences) == 0 {
        WindowsMode::On
    } else {
        WindowsMode::Custom
    }
}

/// How many settings of the registry are not at their Windows value.
pub fn differing(preferences: &Preferences) -> usize {
    let settings = WINDOWS_SETTINGS.iter();
    settings
        .filter(|setting| !setting.is_windows(preferences))
        .count()
}

/// The change that turns the switch on: every setting at its Windows
/// value, and, unless the switch is on already, the current values
/// remembered.
pub fn turn_on(preferences: &Preferences) -> PreferencesUpdate {
    let mut values = Map::new();
    for setting in WINDOWS_SETTINGS {
        values.insert(setting.key.to_owned(), (setting.windows)());
    }
    if preferences.windows_mode_before.is_none() {
        let before = WINDOWS_SETTINGS
            .iter()
            .map(|setting| (setting.key.to_owned(), (setting.read)(preferences)));
        values.insert(STORED_KEY.to_owned(), Value::Object(before.collect()));
    }
    update_from(&Value::Object(values))
}

/// The change that turns the switch off: the remembered values back, and
/// nothing remembered. A setting without a remembered value keeps its own.
pub fn turn_off(preferences: &Preferences) -> PreferencesUpdate {
    let mut values = preferences.windows_mode_before.clone().unwrap_or_default();
    values.insert(STORED_KEY.to_owned(), Value::Bool(false));
    update_from(&Value::Object(values))
}

/// `values` read as any other change, so a remembered value is checked as
/// one from the settings file is.
fn update_from(values: &Value) -> PreferencesUpdate {
    PreferencesUpdate::from_json(values).unwrap_or_default()
}

/// Reads `behaveLikeWindows` from untrusted JSON: an object of earlier
/// values turns the switch on, `false` or `null` off; anything else is
/// ignored. Only short keys with a flag or a short choice are kept, as
/// many as [`MAX_REMEMBERED`]; keys this build does not know are kept for
/// a newer one, and every value is checked again when it is put back.
pub(super) fn read_before(value: &Value) -> Option<WindowsModeChange> {
    match value {
        Value::Object(values) => {
            let kept = values.iter().filter(|(key, value)| {
                !key.is_empty()
                    && key.len() <= MAX_KEY
                    && key.chars().all(|c| c.is_ascii_alphanumeric())
                    && match value {
                        Value::Bool(_) => true,
                        Value::String(choice) => {
                            choice.len() <= MAX_CHOICE && choice.chars().all(|c| c.is_ascii_alphanumeric())
                        }
                        _ => false,
                    }
            });
            let kept = kept.take(MAX_REMEMBERED);
            let kept = kept.map(|(key, value)| (key.clone(), value.clone()));
            Some(WindowsModeChange::On(kept.collect()))
        }
        Value::Bool(false) | Value::Null => Some(WindowsModeChange::Off),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// `preferences` after `update`.
    fn after(preferences: &Preferences, update: &PreferencesUpdate) -> Preferences {
        let mut changed = preferences.clone();
        changed.apply(update);
        changed
    }

    /// `preferences` with the setting `key` changed to `value`, as its own
    /// row saves it.
    fn with(preferences: &Preferences, key: &str, value: &Value) -> Preferences {
        let change = PreferencesUpdate::from_json(&json!({ key: value })).expect("an object");
        after(preferences, &change)
    }

    /// Each line names a key the settings file reads into the field the
    /// line reads, and the owner's default is not the Windows value, so
    /// the switch has something to change.
    #[test]
    fn every_line_reads_and_writes_the_same_setting() {
        let defaults = Preferences::default();
        for setting in WINDOWS_SETTINGS {
            assert!(
                !setting.is_windows(&defaults),
                "{} is already Windows' by default",
                setting.key
            );
            let windows = with(&defaults, setting.key, &(setting.windows)());
            assert!(
                setting.is_windows(&windows),
                "{} reads what it writes",
                setting.key
            );
            let others = WINDOWS_SETTINGS.iter().filter(|other| other.key != setting.key);
            for other in others {
                assert_eq!(
                    (other.read)(&windows),
                    (other.read)(&defaults),
                    "{} changes only itself, not {}",
                    setting.key,
                    other.key
                );
            }
        }
    }

    /// On sets every Windows value at once and remembers the earlier ones;
    /// off puts them back exactly.
    #[test]
    fn on_sets_every_windows_value_and_off_puts_the_earlier_ones_back() {
        let mine = with(&Preferences::default(), "pastePicturesAsFiles", &json!(false));
        let on = after(&mine, &turn_on(&mine));
        assert_eq!(windows_mode(&on), WindowsMode::On);
        assert!(WINDOWS_SETTINGS.iter().all(|setting| setting.is_windows(&on)));
        let stored = serde_json::to_value(&on).unwrap();
        assert_eq!(
            stored[STORED_KEY]["pastePicturesAsFiles"],
            json!(false),
            "{stored}"
        );
        assert_eq!(stored[STORED_KEY]["copyPathsAsText"], json!(true), "{stored}");

        let off = after(&on, &turn_off(&on));
        assert_eq!(off, mine, "off restores everything as it was");
        assert_eq!(windows_mode(&off), WindowsMode::Off);
    }

    /// Off → on → one setting changed on its own row → the switch reads
    /// Custom, not off; off then restores what came before the switch,
    /// that setting too.
    ///
    /// parity: SET-020
    #[test]
    fn a_setting_changed_while_on_shows_custom_and_off_still_restores() {
        let mine = Preferences::default();
        let on = after(&mine, &turn_on(&mine));
        let changed = with(&on, "copyPathsAsText", &json!(true));
        assert_eq!(windows_mode(&changed), WindowsMode::Custom);
        assert_eq!(differing(&changed), 1);
        assert_eq!(changed.paste_pictures_as_files, on.paste_pictures_as_files);

        let back = with(&changed, "copyPathsAsText", &json!(false));
        assert_eq!(windows_mode(&back), WindowsMode::On, "all Windows' again");

        let off = after(&changed, &turn_off(&changed));
        assert_eq!(off, mine);
    }

    /// A hand-edited `behaveLikeWindows` keeps only short keys with flags
    /// or short choices; a value of another type changes nothing, and a
    /// bad remembered value is dropped when it is put back.
    #[test]
    fn untrusted_remembered_values_are_checked() {
        let before = read_before(&json!({
            "pastePicturesAsFiles": true,
            "columnHeaderStyle": "openxplorer",
            "textSize": 200,
            "bad key": true,
            "lists": ["a"],
            "columnHeaderStyleX": "no spaces allowed",
        }))
        .unwrap();
        let WindowsModeChange::On(before) = before else {
            panic!("an object turns it on")
        };
        let keys: Vec<&String> = before.keys().collect();
        assert_eq!(keys, ["columnHeaderStyle", "pastePicturesAsFiles"]);
        assert_eq!(read_before(&json!(false)), Some(WindowsModeChange::Off));
        assert_eq!(read_before(&Value::Null), Some(WindowsModeChange::Off));
        assert_eq!(read_before(&json!(true)), None);
        assert_eq!(read_before(&json!("on")), None);

        let on = Preferences {
            windows_mode_before: Some(Map::from_iter([(
                "columnHeaderStyle".to_owned(),
                json!("nonsense"),
            )])),
            column_header_style: ColumnHeaderStyle::Windows,
            ..Preferences::default()
        };
        let off = after(&on, &turn_off(&on));
        assert_eq!(off.column_header_style, ColumnHeaderStyle::Windows, "kept");
        assert_eq!(windows_mode(&off), WindowsMode::Off);
    }
}
