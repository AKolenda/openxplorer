// SPDX-License-Identifier: AGPL-3.0-only
//! The data kept in `settings.json` and the bounds every preference obeys.
//!
//! Ports the `Settings.data` layout and `update_preferences` from
//! `desktop/core.py`. Field order matches the Python dictionaries, so both
//! applications write the same file layout.

use std::ops::RangeInclusive;

use serde::Serialize;

use super::choices::{ContextMenu, Theme, View};

/// Format version written to `settings.json`.
pub const SETTINGS_VERSION: u32 = 2;

/// Text sizes offered in Settings, in percent.
pub const TEXT_SIZES: [u32; 8] = [80, 90, 100, 110, 125, 150, 175, 200];

/// The text size of a new installation and of "Reset text size", in percent.
pub const DEFAULT_TEXT_SIZE: u32 = 100;

/// Accepted network refresh intervals, in seconds.
pub const NETWORK_INTERVALS: [u32; 3] = [30, 60, 300];

/// The network refresh interval of a new installation, in seconds.
const DEFAULT_NETWORK_INTERVAL: u32 = 60;

/// Accepted sidebar widths, in pixels.
pub const SIDEBAR_WIDTHS: RangeInclusive<u32> = 140..=560;

/// A saved location: a Quick access pin or a mapped network share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bookmark {
    /// Canonical location URI.
    pub uri: String,
    /// Sidebar label; at most 120 characters, no control characters.
    pub label: String,
}

/// A recently opened file, shown on the Home page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    /// Canonical file URI.
    pub uri: String,
    /// Display name, at most 512 characters when read back.
    pub name: String,
    /// Human-readable type, for example "PDF document".
    #[serde(rename = "type")]
    pub type_name: String,
    /// Always `false` when read back: only files are remembered.
    pub is_dir: bool,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub modified: u64,
}

/// A resizable column of the Details view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    /// The file name.
    Name,
    /// Date modified.
    Modified,
    /// The containing folder, shown in search results.
    ParentUri,
    /// The type description.
    Type,
    /// The size.
    Size,
}

impl Column {
    /// Every column, in the order the Python app stores them.
    pub const ALL: [Column; 5] = [
        Column::Name,
        Column::Modified,
        Column::ParentUri,
        Column::Type,
        Column::Size,
    ];

    /// The key used in `columnWidths` and by the UI (`parentUri`, ...).
    pub fn key(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Modified => "modified",
            Column::ParentUri => "parentUri",
            Column::Type => "type",
            Column::Size => "size",
        }
    }

    /// The widths, in pixels, this column may be saved with.
    pub fn width_range(self) -> RangeInclusive<u32> {
        match self {
            Column::Name | Column::ParentUri => 140..=1600,
            Column::Modified => 100..=1000,
            Column::Type => 80..=1000,
            Column::Size => 70..=600,
        }
    }
}

/// Saved Details-view column widths; unset columns use their default width.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnWidths {
    /// Width of [`Column::Name`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<u32>,
    /// Width of [`Column::Modified`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u32>,
    /// Width of [`Column::ParentUri`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<u32>,
    /// Width of [`Column::Type`].
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<u32>,
    /// Width of [`Column::Size`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
}

impl ColumnWidths {
    /// The saved width of `column`.
    pub fn get(&self, column: Column) -> Option<u32> {
        self.slot(column)
    }

    /// Keeps only in-range widths, rounded like Python's `round()`.
    pub fn from_values(values: &[(Column, f64)]) -> Self {
        let mut widths = Self::default();
        for &(column, value) in values {
            if let Some(width) = bounded_width(value, column.width_range()) {
                *widths.slot_mut(column) = Some(width);
            }
        }
        widths
    }

    /// True if no column has a saved width.
    pub fn is_empty(&self) -> bool {
        Column::ALL.iter().all(|&column| self.get(column).is_none())
    }

    fn slot(&self, column: Column) -> Option<u32> {
        match column {
            Column::Name => self.name,
            Column::Modified => self.modified,
            Column::ParentUri => self.parent_uri,
            Column::Type => self.kind,
            Column::Size => self.size,
        }
    }

    fn slot_mut(&mut self, column: Column) -> &mut Option<u32> {
        match column {
            Column::Name => &mut self.name,
            Column::Modified => &mut self.modified,
            Column::ParentUri => &mut self.parent_uri,
            Column::Type => &mut self.kind,
            Column::Size => &mut self.size,
        }
    }
}

/// User preferences shared by every window and by the Python app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// The colour theme.
    pub theme: Theme,
    /// Details list or icon grid.
    pub view: View,
    /// Details pane visible.
    pub details: bool,
    /// Hidden files shown.
    pub show_hidden: bool,
    /// Background search indexing enabled.
    pub auto_index: bool,
    /// Classic (Windows 10) or compact (Windows 11) context menus.
    pub context_menu: ContextMenu,
    /// Seconds between network location refreshes: 30, 60 or 300.
    pub network_interval: u32,
    /// Percent: 80, 90, 100, 110, 125, 150, 175 or 200.
    pub text_size: u32,
    /// Sidebar width in pixels, once the user resized it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<u32>,
    /// Details-view column widths, once the user resized or reset them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_widths: Option<ColumnWidths>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            view: View::default(),
            details: true,
            show_hidden: false,
            auto_index: true,
            context_menu: ContextMenu::default(),
            network_interval: DEFAULT_NETWORK_INTERVAL,
            text_size: DEFAULT_TEXT_SIZE,
            sidebar_width: None,
            column_widths: None,
        }
    }
}

impl Preferences {
    /// Applies every valid value in `update` and silently ignores the rest,
    /// exactly like `update_preferences` in the Python app. A present
    /// `column_widths` replaces all saved column widths.
    pub fn apply(&mut self, update: &PreferencesUpdate) {
        let text_size = update.text_size.filter(|size| TEXT_SIZES.contains(size));
        let sidebar_width = update
            .sidebar_width
            .and_then(|width| bounded_width(width, SIDEBAR_WIDTHS));
        let network_interval = update
            .network_interval
            .filter(|interval| NETWORK_INTERVALS.contains(interval));
        let column_widths = update
            .column_widths
            .as_deref()
            .map(ColumnWidths::from_values);

        replace_if_some(&mut self.theme, update.theme);
        replace_if_some(&mut self.view, update.view);
        replace_if_some(&mut self.details, update.details);
        replace_if_some(&mut self.show_hidden, update.show_hidden);
        replace_if_some(&mut self.auto_index, update.auto_index);
        replace_if_some(&mut self.context_menu, update.context_menu);
        replace_if_some(&mut self.network_interval, network_interval);
        replace_if_some(&mut self.text_size, text_size);
        if sidebar_width.is_some() {
            self.sidebar_width = sidebar_width;
        }
        if column_widths.is_some() {
            self.column_widths = column_widths;
        }
    }
}

/// A partial preferences change; `None` leaves a preference unchanged and
/// out-of-range numbers are ignored when applied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreferencesUpdate {
    /// New theme.
    pub theme: Option<Theme>,
    /// New view.
    pub view: Option<View>,
    /// Show or hide the details pane.
    pub details: Option<bool>,
    /// Show or hide hidden files.
    pub show_hidden: Option<bool>,
    /// Enable or disable background indexing.
    pub auto_index: Option<bool>,
    /// New text size in percent.
    pub text_size: Option<u32>,
    /// New sidebar width in pixels; rounded when saved.
    pub sidebar_width: Option<f64>,
    /// Replaces all column widths; an empty list resets them.
    pub column_widths: Option<Vec<(Column, f64)>>,
    /// New context menu style.
    pub context_menu: Option<ContextMenu>,
    /// New network refresh interval in seconds.
    pub network_interval: Option<u32>,
}

/// The validated contents of `settings.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsData {
    /// Quick access pins, in the order they were added.
    pub pins: Vec<Bookmark>,
    /// Mapped network shares.
    pub shares: Vec<Bookmark>,
    /// Quick access locations the user unpinned, including known folders.
    pub hidden_quick: Vec<String>,
    /// The Quick access order the user dragged into place.
    pub quick_order: Vec<String>,
    /// Recently opened files, newest first.
    pub recent: Vec<RecentEntry>,
    /// Preferences.
    pub preferences: Preferences,
}

impl SettingsData {
    /// The file contents as a JSON value, in the layout both apps write.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.file_layout()).expect("settings data holds only strings, numbers and lists")
    }

    /// The file contents as pretty-printed JSON with a final newline,
    /// matching Python's `json.dump(..., indent=2)` key order.
    pub fn to_file_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.file_layout())
            .expect("settings data holds only strings, numbers and lists");
        text.push('\n');
        text
    }

    fn file_layout(&self) -> FileLayout<'_> {
        FileLayout {
            version: SETTINGS_VERSION,
            pins: &self.pins,
            shares: &self.shares,
            hidden_quick: &self.hidden_quick,
            quick_order: &self.quick_order,
            recent: &self.recent,
            preferences: &self.preferences,
        }
    }
}

/// `settings.json` field order as the Python app writes it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileLayout<'a> {
    version: u32,
    pins: &'a [Bookmark],
    shares: &'a [Bookmark],
    hidden_quick: &'a [String],
    quick_order: &'a [String],
    recent: &'a [RecentEntry],
    preferences: &'a Preferences,
}

/// Rounds `value` half-to-even (Python's `round()`) if it lies in `range`.
/// The range is checked before rounding, so 139.6 is rejected for 140..=560.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value lies within a u32 range, so the cast is exact after rounding"
)]
pub(crate) fn bounded_width(value: f64, range: RangeInclusive<u32>) -> Option<u32> {
    let low = f64::from(*range.start());
    let high = f64::from(*range.end());
    let in_range = value >= low && value <= high;
    in_range.then(|| value.round_ties_even() as u32)
}

/// Stores `value` in `slot` if there is one.
fn replace_if_some<T>(slot: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *slot = value;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn update() -> PreferencesUpdate {
        PreferencesUpdate::default()
    }

    /// parity: SET-016
    #[test]
    fn defaults_match_the_python_app() {
        let prefs = Preferences::default();
        assert_eq!(prefs.theme, Theme::System);
        assert_eq!(prefs.view, View::Details);
        assert_eq!(prefs.context_menu, ContextMenu::Win10);
        assert_eq!(prefs.network_interval, 60);
        assert_eq!(prefs.text_size, 100);
        assert!(prefs.auto_index && prefs.details && !prefs.show_hidden);
    }

    /// The layout of `Settings.data['preferences']` in `desktop/core.py`.
    /// parity: SET-016
    #[test]
    fn default_preferences_are_stored_like_the_python_app() {
        let stored = serde_json::to_value(Preferences::default()).unwrap();
        let python = json!({
            "theme": "system", "view": "details", "details": true, "showHidden": false,
            "autoIndex": true, "contextMenu": "win10", "networkInterval": 60, "textSize": 100
        });
        assert_eq!(stored, python);
    }

    /// parity: SET-016, VIEW-045
    #[test]
    fn text_size_accepts_only_the_offered_sizes() {
        let mut prefs = Preferences::default();
        for size in TEXT_SIZES {
            prefs.apply(&PreferencesUpdate {
                text_size: Some(size),
                ..update()
            });
            assert_eq!(prefs.text_size, size);
        }
        for size in [0, 101, 201, 10_000] {
            prefs.apply(&PreferencesUpdate {
                text_size: Some(size),
                ..update()
            });
            assert_eq!(prefs.text_size, 200);
        }
    }

    #[test]
    fn widths_are_bounded_before_rounding_half_to_even() {
        assert_eq!(bounded_width(280.4, SIDEBAR_WIDTHS), Some(280));
        assert_eq!(bounded_width(140.5, SIDEBAR_WIDTHS), Some(140));
        assert_eq!(bounded_width(141.5, SIDEBAR_WIDTHS), Some(142));
        assert_eq!(bounded_width(139.6, SIDEBAR_WIDTHS), None);
        assert_eq!(bounded_width(f64::NAN, SIDEBAR_WIDTHS), None);
        assert_eq!(bounded_width(f64::INFINITY, SIDEBAR_WIDTHS), None);
    }

    /// parity: VIEW-028
    #[test]
    fn column_widths_keep_only_known_in_range_columns() {
        let widths = ColumnWidths::from_values(&[(Column::Name, 150.0), (Column::Size, 99_999.0)]);
        assert_eq!(widths.get(Column::Name), Some(150));
        assert_eq!(widths.get(Column::Size), None);
        let json = serde_json::to_value(&widths).unwrap();
        assert_eq!(json, json!({"name": 150}));
    }

    /// parity: SET-016
    #[test]
    fn network_intervals_outside_the_whitelist_are_ignored() {
        let mut prefs = Preferences::default();
        prefs.apply(&PreferencesUpdate {
            network_interval: Some(1),
            ..update()
        });
        assert_eq!(prefs.network_interval, 60);
        prefs.apply(&PreferencesUpdate {
            network_interval: Some(300),
            ..update()
        });
        assert_eq!(prefs.network_interval, 300);
    }

    #[test]
    fn file_text_uses_the_python_layout() {
        let text = SettingsData::default().to_file_text();
        let keys: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("  \"") && !line.starts_with("    "))
            .map(|line| line.trim().split('"').nth(1).unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "version",
                "pins",
                "shares",
                "hiddenQuick",
                "quickOrder",
                "recent",
                "preferences"
            ]
        );
        assert!(text.ends_with("}\n"));
    }
}
