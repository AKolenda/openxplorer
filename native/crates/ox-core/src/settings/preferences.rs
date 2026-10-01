// SPDX-License-Identifier: AGPL-3.0-only
//! User preferences: their defaults, the values each one accepts, and how an
//! untrusted update is read and applied.
//!
//! Ports `update_preferences` in `v2.0.0:desktop/core.py`, including the JSON type
//! checks Python makes (`type(size) is int`, `isinstance(value, bool)`), so
//! both applications accept and ignore exactly the same values.

use std::ops::RangeInclusive;

use serde::Serialize;
use serde_json::Value;

use super::choices::{ContextMenu, Theme, View};
use super::pane_options::DetailsPaneOptions;
use super::view_properties::{read_folder_views, FolderView, ViewProperties};
use super::SettingsError;

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

/// Accepted window widths, in pixels: from the window's minimum up.
pub const WINDOW_WIDTHS: RangeInclusive<u32> = 670..=16_384;

/// Accepted window heights, in pixels: from the window's minimum up.
pub const WINDOW_HEIGHTS: RangeInclusive<u32> = 470..=16_384;

/// The size new windows open at, and whether they open maximized: the
/// last window's (TAB-054). The Python app ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSize {
    /// The width when not maximized, in pixels.
    pub width: u32,
    /// The height when not maximized, in pixels.
    pub height: u32,
    /// Whether the window was maximized.
    pub maximized: bool,
}

impl WindowSize {
    /// Reads `{"width", "height", "maximized"}`; `None` unless both sizes
    /// are within [`WINDOW_WIDTHS`] and [`WINDOW_HEIGHTS`].
    fn from_json(value: &Value) -> Option<Self> {
        let size = |key: &str, range: RangeInclusive<u32>| {
            let pixels = value.get(key)?.as_u64()?;
            u32::try_from(pixels).ok().filter(|pixels| range.contains(pixels))
        };
        Some(Self {
            width: size("width", WINDOW_WIDTHS)?,
            height: size("height", WINDOW_HEIGHTS)?,
            maximized: value.get("maximized").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// Whether both sizes are ones a window may open at.
    fn is_valid(self) -> bool {
        WINDOW_WIDTHS.contains(&self.width) && WINDOW_HEIGHTS.contains(&self.height)
    }
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
    pub const fn as_str(self) -> &'static str {
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

/// The width the Details view measured for one column, before it is
/// checked against [`Column::width_range`] and rounded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidth {
    /// The column.
    pub column: Column,
    /// Its width in pixels.
    pub pixels: f64,
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
    pub file_type: Option<u32>,
    /// Width of [`Column::Size`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
}

impl ColumnWidths {
    /// The saved width of `column`.
    pub fn get(&self, column: Column) -> Option<u32> {
        match column {
            Column::Name => self.name,
            Column::Modified => self.modified,
            Column::ParentUri => self.parent_uri,
            Column::Type => self.file_type,
            Column::Size => self.size,
        }
    }

    /// Keeps only in-range widths, rounded like Python's `round()`.
    pub fn from_values(values: &[ColumnWidth]) -> Self {
        let mut widths = Self::default();
        for requested in values {
            let column = requested.column;
            if let Some(width) = bounded_width(requested.pixels, column.width_range()) {
                *widths.width_mut(column) = Some(width);
            }
        }
        widths
    }

    /// True if no column has a saved width.
    pub fn is_empty(&self) -> bool {
        Column::ALL.iter().all(|&column| self.get(column).is_none())
    }

    fn width_mut(&mut self, column: Column) -> &mut Option<u32> {
        match column {
            Column::Name => &mut self.name,
            Column::Modified => &mut self.modified,
            Column::ParentUri => &mut self.parent_uri,
            Column::Type => &mut self.file_type,
            Column::Size => &mut self.size,
        }
    }
}

/// User preferences shared by every window and by the Python app.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate saved on/off preference"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// The colour theme.
    pub theme: Theme,
    /// Details list or icon grid.
    pub view: View,
    /// Details pane visible. Stored as `details`, the Python app's key.
    #[serde(rename = "details")]
    pub show_details_pane: bool,
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
    /// The last window's size, once a window was resized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_size: Option<WindowSize>,
    /// The address bar's crumbs start at `/` instead of the closest place
    /// (Dolphin's `ShowFullPath`). Stored only when on, as are the next
    /// two, so the Python app's file keeps its layout.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub show_full_path: bool,
    /// New windows show the address as editable text instead of crumbs
    /// (Dolphin's `EditableUrl`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub editable_location: bool,
    /// Folders opened from other apps open in a new window instead of a
    /// new tab (Dolphin's `OpenExternallyCalledFolderInNewTab`, inverted).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub external_folders_in_new_window: bool,
    /// Archives the app can read open as folders in its archive browser
    /// (Dolphin's "Open archives as folder", ARC-022); off, they open in
    /// their default application. Saved only when off, like the options
    /// below.
    #[serde(skip_serializing_if = "is_true")]
    pub browse_archives: bool,
    /// The details pane's own options; saved only once changed, so the
    /// settings of a new installation stay as the Python app writes them.
    #[serde(skip_serializing_if = "DetailsPaneOptions::is_default")]
    pub details_pane_options: DetailsPaneOptions,
    /// The window's title is the folder's full path instead of its name
    /// (Dolphin's `ShowFullPathInTitlebar`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub full_path_in_title: bool,
    /// Ask before moving items to the Trash. On by default, as the Python
    /// app always asked; this and the next two are stored only when off.
    #[serde(skip_serializing_if = "is_true")]
    pub confirm_trash: bool,
    /// Ask before deleting items permanently.
    #[serde(skip_serializing_if = "is_true")]
    pub confirm_delete: bool,
    /// Ask before emptying the Recycle Bin.
    #[serde(skip_serializing_if = "is_true")]
    pub confirm_empty_trash: bool,
    /// Ask before closing a window with several tabs (Dolphin's
    /// `ConfirmClosingMultipleTabs`); off, as Explorer never asks.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub confirm_close_tabs: bool,
    /// Opening a program or script asks whether to run it or open it in
    /// its application (Dolphin's "Always ask"); off, it only ever opens.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ask_to_run_programs: bool,
    /// Text uses the desktop's interface font and its size instead of the
    /// Windows font stack. Stored only when on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub desktop_font: bool,
    /// The navigation pane is hidden (Dolphin's Places panel closed,
    /// Explorer's View > Show > Navigation pane off).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hide_sidebar: bool,
    /// The sidebar's icon size in pixels (16, 22, 32 or 48), or 0 for the
    /// automatic size (Dolphin's Places panel Icon Size). Stored only when
    /// chosen.
    #[serde(skip_serializing_if = "is_automatic_icon_size")]
    pub sidebar_icon_size: u32,
    /// The sidebar sections the user hid (Dolphin's "Hide Section"), by
    /// the keys the app gives them. Stored only when one is hidden.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_sidebar_sections: Vec<String>,
    /// The sidebar places the user hid one by one (Dolphin's "Hide"), by
    /// location: drives, network locations, Recent files, the Recycle
    /// Bin. Hidden standard folders are `hiddenQuick`. Stored only when
    /// one is hidden.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_sidebar_places: Vec<String>,
    /// Date columns show absolute dates instead of "Today at 3:00 PM"
    /// (Dolphin's `UseShortRelativeDates`, inverted). Stored only when on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub absolute_dates: bool,
    /// Each folder remembers its own display style (Dolphin's
    /// `GlobalViewProps`, inverted). Stored only when on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub per_folder_views: bool,
    /// Hovering an item shows the marker that adds it to the selection or
    /// takes it out (Dolphin's `ShowSelectionToggle`). Stored only when off.
    #[serde(skip_serializing_if = "is_true")]
    pub selection_marker: bool,
    /// Folders in the details view expand in place (Dolphin's
    /// `ExpandableFolders`). Stored only when off.
    #[serde(skip_serializing_if = "is_true")]
    pub expandable_folders: bool,
    /// The display style of every folder without its own, once the user
    /// changed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_defaults: Option<ViewProperties>,
    /// The folders that keep their own display style, oldest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folder_views: Vec<FolderView>,
}

/// The most sidebar sections that may be hidden, and the longest key.
const MAX_HIDDEN_SECTIONS: usize = 16;
const MAX_SECTION_KEY: usize = 32;

/// The most sidebar places that may be hidden one by one, and the longest
/// location.
const MAX_HIDDEN_PLACES: usize = 64;
const MAX_PLACE_LOCATION: usize = 4096;

/// The sidebar icon sizes the user may choose, in pixels; 0 is automatic.
pub const SIDEBAR_ICON_SIZES: [u32; 5] = [0, 16, 22, 32, 48];

/// True for the automatic sidebar icon size, which is not stored.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes the field by reference"
)]
fn is_automatic_icon_size(size: &u32) -> bool {
    *size == 0
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            view: View::default(),
            show_details_pane: true,
            show_hidden: false,
            auto_index: true,
            context_menu: ContextMenu::default(),
            network_interval: DEFAULT_NETWORK_INTERVAL,
            text_size: DEFAULT_TEXT_SIZE,
            sidebar_width: None,
            column_widths: None,
            window_size: None,
            show_full_path: false,
            editable_location: false,
            external_folders_in_new_window: false,
            browse_archives: true,
            details_pane_options: DetailsPaneOptions::default(),
            full_path_in_title: false,
            confirm_trash: true,
            confirm_delete: true,
            confirm_empty_trash: true,
            confirm_close_tabs: false,
            ask_to_run_programs: false,
            desktop_font: false,
            hide_sidebar: false,
            sidebar_icon_size: 0,
            hidden_sidebar_sections: Vec::new(),
            hidden_sidebar_places: Vec::new(),
            absolute_dates: false,
            per_folder_views: false,
            selection_marker: true,
            expandable_folders: true,
            view_defaults: None,
            folder_views: Vec::new(),
        }
    }
}

impl Preferences {
    /// Applies every valid value in `update` and silently ignores the rest,
    /// exactly like `update_preferences` in the Python app. A present
    /// `column_widths` replaces all saved column widths.
    pub fn apply(&mut self, update: &PreferencesUpdate) {
        let text_size = update.text_size.filter(|size| TEXT_SIZES.contains(size));
        let network_interval = update
            .network_interval
            .filter(|interval| NETWORK_INTERVALS.contains(interval));
        let sidebar_width = update
            .sidebar_width
            .and_then(|width| bounded_width(width, SIDEBAR_WIDTHS));
        let column_widths = update.column_widths.as_deref().map(ColumnWidths::from_values);

        replace_if_some(&mut self.theme, update.theme);
        replace_if_some(&mut self.view, update.view);
        replace_if_some(&mut self.show_details_pane, update.show_details_pane);
        replace_if_some(&mut self.show_hidden, update.show_hidden);
        replace_if_some(&mut self.auto_index, update.auto_index);
        replace_if_some(&mut self.context_menu, update.context_menu);
        replace_if_some(&mut self.network_interval, network_interval);
        replace_if_some(&mut self.text_size, text_size);
        replace_if_some(&mut self.show_full_path, update.show_full_path);
        replace_if_some(&mut self.editable_location, update.editable_location);
        replace_if_some(
            &mut self.external_folders_in_new_window,
            update.external_folders_in_new_window,
        );
        replace_if_some(&mut self.full_path_in_title, update.full_path_in_title);
        replace_if_some(&mut self.confirm_trash, update.confirm_trash);
        replace_if_some(&mut self.confirm_delete, update.confirm_delete);
        replace_if_some(&mut self.confirm_empty_trash, update.confirm_empty_trash);
        replace_if_some(&mut self.confirm_close_tabs, update.confirm_close_tabs);
        replace_if_some(&mut self.ask_to_run_programs, update.ask_to_run_programs);
        replace_if_some(&mut self.desktop_font, update.desktop_font);
        replace_if_some(&mut self.hide_sidebar, update.hide_sidebar);
        let icon_size = update
            .sidebar_icon_size
            .filter(|size| SIDEBAR_ICON_SIZES.contains(size));
        replace_if_some(&mut self.sidebar_icon_size, icon_size);
        let sections = update.hidden_sidebar_sections.as_ref().filter(|sections| {
            sections.len() <= MAX_HIDDEN_SECTIONS
                && sections.iter().all(|key| {
                    !key.is_empty()
                        && key.len() <= MAX_SECTION_KEY
                        && key.chars().all(|c| c.is_ascii_alphanumeric())
                })
        });
        if let Some(sections) = sections {
            self.hidden_sidebar_sections.clone_from(sections);
        }
        let places = update.hidden_sidebar_places.as_ref().filter(|places| {
            places.len() <= MAX_HIDDEN_PLACES
                && places.iter().all(|uri| {
                    !uri.is_empty() && uri.len() <= MAX_PLACE_LOCATION && !uri.contains(char::is_control)
                })
        });
        if let Some(places) = places {
            self.hidden_sidebar_places.clone_from(places);
        }
        if let Some(width) = sidebar_width {
            self.sidebar_width = Some(width);
        }
        if let Some(widths) = column_widths {
            self.column_widths = Some(widths);
        }
        if let Some(size) = update.window_size.filter(|size| size.is_valid()) {
            self.window_size = Some(size);
        }
        replace_if_some(&mut self.browse_archives, update.browse_archives);
        if let Some(options) = &update.details_pane_options {
            self.details_pane_options = options.clone();
        }
        replace_if_some(&mut self.absolute_dates, update.absolute_dates);
        replace_if_some(&mut self.per_folder_views, update.per_folder_views);
        replace_if_some(&mut self.selection_marker, update.selection_marker);
        replace_if_some(&mut self.expandable_folders, update.expandable_folders);
        if let Some(defaults) = &update.view_defaults {
            self.view_defaults = Some(defaults.clone());
        }
        if let Some(folder_views) = &update.folder_views {
            self.folder_views.clone_from(folder_views);
        }
    }

    /// The display style `uri` is shown in: its own when each folder
    /// keeps one, else the shared style (VIEW-020).
    pub fn view_for(&self, uri: &str) -> ViewProperties {
        // Before a style was saved, the Python app's view and hidden files.
        let defaults = self.view_defaults.clone().unwrap_or_else(|| ViewProperties {
            mode: match self.view {
                View::Details => "details",
                View::Grid => "icons",
            }
            .to_owned(),
            show_hidden: self.show_hidden,
            ..ViewProperties::default()
        });
        if !self.per_folder_views {
            return defaults;
        }
        super::view_properties::style_for(&self.folder_views, &defaults, uri)
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
    pub show_details_pane: Option<bool>,
    /// Show or hide hidden files.
    pub show_hidden: Option<bool>,
    /// Enable or disable background indexing.
    pub auto_index: Option<bool>,
    /// New text size in percent.
    pub text_size: Option<u32>,
    /// New sidebar width in pixels; rounded when saved.
    pub sidebar_width: Option<f64>,
    /// Replaces all column widths; an empty list resets them.
    pub column_widths: Option<Vec<ColumnWidth>>,
    /// New context menu style.
    pub context_menu: Option<ContextMenu>,
    /// New network refresh interval in seconds.
    pub network_interval: Option<u32>,
    /// The size new windows open at.
    pub window_size: Option<WindowSize>,
    /// Show the full path in the address bar, or start at the closest place.
    pub show_full_path: Option<bool>,
    /// Open new windows with an editable address.
    pub editable_location: Option<bool>,
    /// Open folders from other apps in a new window, or in a new tab.
    pub external_folders_in_new_window: Option<bool>,
    /// Open archives as folders, or in their default application.
    pub browse_archives: Option<bool>,
    /// Replaces the details pane's options.
    pub details_pane_options: Option<DetailsPaneOptions>,
    /// Show the folder's full path in the window title.
    pub full_path_in_title: Option<bool>,
    /// Ask before moving items to the Trash.
    pub confirm_trash: Option<bool>,
    /// Ask before deleting items permanently.
    pub confirm_delete: Option<bool>,
    /// Ask before emptying the Recycle Bin.
    pub confirm_empty_trash: Option<bool>,
    /// Ask before closing a window with several tabs.
    pub confirm_close_tabs: Option<bool>,
    /// Ask whether to run a program or script that is opened.
    pub ask_to_run_programs: Option<bool>,
    /// Use the desktop's font, or the Windows font stack.
    pub desktop_font: Option<bool>,
    /// Hide or show the navigation pane.
    pub hide_sidebar: Option<bool>,
    /// New sidebar icon size; one of [`SIDEBAR_ICON_SIZES`] or ignored.
    pub sidebar_icon_size: Option<u32>,
    /// Replaces the hidden sidebar sections; up to 16 short ASCII keys,
    /// else ignored.
    pub hidden_sidebar_sections: Option<Vec<String>>,
    /// Replaces the sidebar places hidden one by one; up to 64 locations,
    /// else ignored.
    pub hidden_sidebar_places: Option<Vec<String>>,
    /// Show absolute dates instead of relative ones.
    pub absolute_dates: Option<bool>,
    /// Remember a display style for each folder.
    pub per_folder_views: Option<bool>,
    /// Show the hover selection marker.
    pub selection_marker: Option<bool>,
    /// Let folders expand in place in the details view.
    pub expandable_folders: Option<bool>,
    /// Replaces the shared display style.
    pub view_defaults: Option<ViewProperties>,
    /// Replaces the folders' own styles; read from the file only, as
    /// windows change one folder at a time with
    /// [`Settings::remember_view`](super::Settings::remember_view).
    pub folder_views: Option<Vec<FolderView>>,
}

impl PreferencesUpdate {
    /// Reads a preferences object from untrusted JSON (the file, or a
    /// request from another window). Values of the wrong JSON type and
    /// unknown choices are dropped here; numeric ranges are checked in
    /// [`Preferences::apply`].
    ///
    /// # Errors
    ///
    /// [`SettingsError::Invalid`] if `value` is not an object.
    pub fn from_json(value: &Value) -> Result<Self, SettingsError> {
        let Some(values) = value.as_object() else {
            return Err(SettingsError::invalid("Preferences must be an object."));
        };
        let text = |key: &str| values.get(key).and_then(Value::as_str);
        let flag = |key: &str| values.get(key).and_then(Value::as_bool);
        Ok(Self {
            theme: text("theme").and_then(Theme::from_key),
            view: text("view").and_then(View::from_key),
            show_details_pane: flag("details"),
            show_hidden: flag("showHidden"),
            auto_index: flag("autoIndex"),
            text_size: values.get("textSize").and_then(read_text_size),
            sidebar_width: values.get("sidebarWidth").and_then(Value::as_f64),
            column_widths: values.get("columnWidths").and_then(read_column_widths),
            context_menu: text("contextMenu").and_then(ContextMenu::from_key),
            network_interval: values.get("networkInterval").and_then(read_network_interval),
            window_size: values.get("windowSize").and_then(WindowSize::from_json),
            show_full_path: flag("showFullPath"),
            editable_location: flag("editableLocation"),
            external_folders_in_new_window: flag("externalFoldersInNewWindow"),
            browse_archives: flag("browseArchives"),
            details_pane_options: values
                .get("detailsPaneOptions")
                .and_then(DetailsPaneOptions::from_json),
            full_path_in_title: flag("fullPathInTitle"),
            confirm_trash: flag("confirmTrash"),
            confirm_delete: flag("confirmDelete"),
            confirm_empty_trash: flag("confirmEmptyTrash"),
            confirm_close_tabs: flag("confirmCloseTabs"),
            ask_to_run_programs: flag("askToRunPrograms"),
            desktop_font: flag("desktopFont"),
            hide_sidebar: flag("hideSidebar"),
            sidebar_icon_size: values
                .get("sidebarIconSize")
                .and_then(Value::as_u64)
                .and_then(|size| u32::try_from(size).ok()),
            hidden_sidebar_sections: values.get("hiddenSidebarSections").and_then(read_keys),
            hidden_sidebar_places: values.get("hiddenSidebarPlaces").and_then(read_keys),
            absolute_dates: flag("absoluteDates"),
            per_folder_views: flag("perFolderViews"),
            selection_marker: flag("selectionMarker"),
            expandable_folders: flag("expandableFolders"),
            view_defaults: values.get("viewDefaults").and_then(ViewProperties::from_json),
            folder_views: values.get("folderViews").and_then(read_folder_views),
        })
    }
}

/// A list of strings, or `None` when `value` is not one.
fn read_keys(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|key| key.as_str().map(str::to_owned))
        .collect()
}

/// A text size given as a true integer. Python checks `type(size) is int`,
/// so 150.0 and "150" are ignored.
fn read_text_size(value: &Value) -> Option<u32> {
    let size = value.as_u64()?;
    u32::try_from(size).ok()
}

/// One of the offered network intervals. Python compares with
/// `in (30, 60, 300)`, so 60.0 matches as well.
#[expect(
    clippy::float_cmp,
    reason = "Python's `in` compares with ==, so only exact values match"
)]
fn read_network_interval(value: &Value) -> Option<u32> {
    let seconds = value.as_f64()?;
    NETWORK_INTERVALS
        .into_iter()
        .find(|&choice| f64::from(choice) == seconds)
}

/// The numeric widths of the known columns in a `columnWidths` object;
/// `None` if it is not an object. Out-of-range widths are dropped when
/// applied.
fn read_column_widths(value: &Value) -> Option<Vec<ColumnWidth>> {
    let columns = value.as_object()?;
    let numeric_width = |column: Column| {
        let pixels = columns.get(column.as_str())?.as_f64()?;
        Some(ColumnWidth { column, pixels })
    };
    Some(Column::ALL.into_iter().filter_map(numeric_width).collect())
}

/// Rounds `value` half-to-even (Python's `round()`) if it lies in `range`.
/// The range is checked before rounding, so 139.6 is rejected for 140..=560.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value lies within a u32 range, so the cast is exact after rounding"
)]
fn bounded_width(value: f64, range: RangeInclusive<u32>) -> Option<u32> {
    let low = f64::from(*range.start());
    let high = f64::from(*range.end());
    let in_range = value >= low && value <= high;
    in_range.then(|| value.round_ties_even() as u32)
}

/// Whether `value` is true, for preferences saved only when turned off.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes the field by reference"
)]
fn is_true(value: &bool) -> bool {
    *value
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

    /// parity: SET-016
    #[test]
    fn defaults_match_the_python_app() {
        let preferences = Preferences::default();
        assert_eq!(preferences.theme, Theme::System);
        assert_eq!(preferences.view, View::Details);
        assert_eq!(preferences.context_menu, ContextMenu::Win10);
        assert_eq!(preferences.network_interval, 60);
        assert_eq!(preferences.text_size, 100);
        assert!(preferences.auto_index);
        assert!(preferences.show_details_pane);
        assert!(!preferences.show_hidden);
    }

    /// The layout of `Settings.data['preferences']` in `v2.0.0:desktop/core.py`.
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
        let mut preferences = Preferences::default();
        for size in TEXT_SIZES {
            preferences.apply(&PreferencesUpdate {
                text_size: Some(size),
                ..PreferencesUpdate::default()
            });
            assert_eq!(preferences.text_size, size);
        }
        let kept = preferences.text_size;
        for size in [0, 101, 201, 10_000] {
            preferences.apply(&PreferencesUpdate {
                text_size: Some(size),
                ..PreferencesUpdate::default()
            });
            assert_eq!(preferences.text_size, kept, "{size} is not offered");
        }
    }

    /// parity: SIDE-023, VIEW-028
    #[test]
    fn widths_are_bounded_before_rounding_half_to_even() {
        assert_eq!(bounded_width(280.4, SIDEBAR_WIDTHS), Some(280));
        assert_eq!(bounded_width(140.5, SIDEBAR_WIDTHS), Some(140));
        assert_eq!(bounded_width(141.5, SIDEBAR_WIDTHS), Some(142));
        assert_eq!(bounded_width(139.6, SIDEBAR_WIDTHS), None);
        assert_eq!(bounded_width(f64::NAN, SIDEBAR_WIDTHS), None);
        assert_eq!(bounded_width(f64::INFINITY, SIDEBAR_WIDTHS), None);
    }

    /// parity: TAB-054
    #[test]
    fn the_window_size_is_kept_only_within_the_window_limits() {
        let read = |value| PreferencesUpdate::from_json(&json!({ "windowSize": value })).unwrap();
        let saved = read(json!({"width": 1000, "height": 700, "maximized": true}));
        let mut preferences = Preferences::default();
        preferences.apply(&saved);
        let stored = serde_json::to_value(&preferences).unwrap();

        assert_eq!(
            preferences.window_size,
            Some(WindowSize {
                width: 1000,
                height: 700,
                maximized: true
            })
        );
        assert_eq!(
            stored["windowSize"],
            json!({"width": 1000, "height": 700, "maximized": true})
        );
        assert_eq!(
            read(json!({"width": 300, "height": 700})).window_size,
            None,
            "below the minimum"
        );
        assert_eq!(read(json!({"width": 1000.5, "height": 700})).window_size, None);
        assert_eq!(read(json!("big")).window_size, None);
    }

    /// parity: VIEW-028
    #[test]
    fn column_widths_keep_only_known_in_range_columns() {
        let widths = ColumnWidths::from_values(&[
            ColumnWidth {
                column: Column::Name,
                pixels: 150.0,
            },
            ColumnWidth {
                column: Column::Size,
                pixels: 99_999.0,
            },
        ]);
        assert_eq!(widths.get(Column::Name), Some(150));
        assert_eq!(widths.get(Column::Size), None);
        let json = serde_json::to_value(&widths).unwrap();
        assert_eq!(json, json!({"name": 150}));
    }

    /// parity: SET-016
    #[test]
    fn network_intervals_outside_the_whitelist_are_ignored() {
        let mut preferences = Preferences::default();
        preferences.apply(&PreferencesUpdate {
            network_interval: Some(1),
            ..PreferencesUpdate::default()
        });
        assert_eq!(preferences.network_interval, 60);
        preferences.apply(&PreferencesUpdate {
            network_interval: Some(300),
            ..PreferencesUpdate::default()
        });
        assert_eq!(preferences.network_interval, 300);
    }

    /// parity: SET-016
    #[test]
    fn choices_outside_the_whitelist_are_ignored() {
        let values = json!({
            "theme": "dark", "view": "bogus", "contextMenu": "win11", "networkInterval": 1
        });
        let mut preferences = Preferences::default();
        preferences.apply(&PreferencesUpdate::from_json(&values).unwrap());
        assert_eq!(preferences.theme, Theme::Dark);
        assert_eq!(preferences.view, View::Details);
        assert_eq!(preferences.context_menu, ContextMenu::Win11);
        assert_eq!(preferences.network_interval, 60);
    }

    #[test]
    fn address_bar_and_external_folder_options_are_stored_only_when_on() {
        let values =
            json!({"showFullPath": true, "editableLocation": "yes", "externalFoldersInNewWindow": true});
        let mut preferences = Preferences::default();

        preferences.apply(&PreferencesUpdate::from_json(&values).unwrap());

        assert!(preferences.show_full_path && preferences.external_folders_in_new_window);
        assert!(!preferences.editable_location, "only a JSON boolean counts");
        let stored = serde_json::to_value(&preferences).unwrap();
        assert_eq!(stored["showFullPath"], json!(true));
        assert!(stored.get("editableLocation").is_none());
    }

    /// parity: SET-010
    #[test]
    fn the_confirmations_ask_by_default_and_are_stored_only_when_changed() {
        let mut preferences = Preferences::default();
        assert!(preferences.confirm_trash && preferences.confirm_delete && preferences.confirm_empty_trash);
        assert!(!preferences.confirm_close_tabs);
        let values = json!({"confirmTrash": false, "confirmCloseTabs": true, "confirmDelete": 0});

        preferences.apply(&PreferencesUpdate::from_json(&values).unwrap());

        assert!(!preferences.confirm_trash && preferences.confirm_close_tabs);
        assert!(preferences.confirm_delete, "only a JSON boolean counts");
        let stored = serde_json::to_value(&preferences).unwrap();
        assert_eq!(stored["confirmTrash"], json!(false));
        assert_eq!(stored["confirmCloseTabs"], json!(true));
        assert!(stored.get("confirmDelete").is_none() && stored.get("fullPathInTitle").is_none());
    }

    /// parity: SET-016
    #[test]
    fn choices_are_case_sensitive_and_must_be_strings() {
        let values = json!({"theme": "Dark", "view": ["grid"], "contextMenu": 11});
        let update = PreferencesUpdate::from_json(&values).unwrap();
        assert_eq!(
            (update.theme, update.view, update.context_menu),
            (None, None, None)
        );
    }
}
