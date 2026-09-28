// SPDX-License-Identifier: AGPL-3.0-only
//! A tab's navigation state, as it moves to another window. Ports
//! `tab_snapshot` and `location` in `desktop/window_state.py`.
//!
//! The JSON form is the Python app's tab handoff format, so a snapshot
//! also suits a saved session. Locations are the native app's: the Python
//! app's `home:`, `pc:`, `network:` and `settings:` are read, and written
//! as the native URIs of [`crate::location::VirtualPlace`].

use serde_json::{json, Map, Value};

use super::WindowStateError;
use crate::location;

/// The most back-and-forward entries a tab keeps.
pub const MAX_HISTORY_ENTRIES: usize = 200;

/// The most selected items a snapshot keeps.
pub const MAX_SELECTED_ITEMS: usize = 10_000;

/// The largest scroll position, in pixels.
pub const MAX_SCROLL: f64 = 1e9;

/// Where a tab without a location opens: the Home page.
const HOME_PAGE: &str = "home:";

/// How a tab shows its folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TabView {
    /// The details list.
    #[default]
    Details,
    /// Icons in a grid.
    Grid,
}

/// What a tab sorts by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortField {
    /// Name.
    #[default]
    Name,
    /// Date modified.
    Modified,
    /// Type.
    Type,
    /// Size.
    Size,
}

/// Which way a tab sorts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortDirection {
    /// A to Z, oldest first, smallest first.
    #[default]
    Ascending,
    /// Z to A, newest first, largest first.
    Descending,
}

/// The section a Settings tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    /// Appearance.
    Appearance,
    /// Search & indexing.
    Search,
    /// Default file manager.
    Default,
    /// Brave's "Show in folder".
    Brave,
    /// Windows and tabs.
    Windows,
    /// Folder sizes.
    Sizes,
}

impl SettingsSection {
    /// Every section, in the order of the Settings page.
    const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Search,
        Self::Default,
        Self::Brave,
        Self::Windows,
        Self::Sizes,
    ];

    /// The section's name in the JSON form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Search => "search",
            Self::Default => "default",
            Self::Brave => "brave",
            Self::Windows => "windows",
            Self::Sizes => "sizes",
        }
    }

    /// The section named `name`, if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.as_str() == name)
    }
}

/// A tab's navigation state: where it is, how it got there, what is
/// selected and how it shows the folder.
#[derive(Debug, Clone, PartialEq)]
pub struct TabSnapshot {
    /// The tab's location.
    pub uri: String,
    /// Back and forward entries, oldest first; `history[index] == uri`.
    pub history: Vec<String>,
    /// The position of `uri` in `history`.
    pub index: usize,
    /// The scroll position in pixels, 0 to [`MAX_SCROLL`].
    pub scroll: f64,
    /// The selected items' locations.
    pub selection: Vec<String>,
    /// How the folder is shown.
    pub view: TabView,
    /// What it is sorted by.
    pub sort: SortField,
    /// Which way it is sorted.
    pub direction: SortDirection,
    /// The Settings section, for a Settings tab.
    pub settings_section: Option<SettingsSection>,
}

impl TabSnapshot {
    /// Validates a tab's state in the Python app's JSON form.
    ///
    /// Safety rule "only whitelisted tab state moves" (`tab_snapshot` in
    /// `desktop/window_state.py`): every other field (a password, for
    /// example) is dropped; every location must pass the location rules,
    /// which refuse unknown schemes; history and selection are bounded; the
    /// scroll position must be finite and is clamped. A history that does
    /// not hold the location at its position is replaced by the location
    /// alone.
    ///
    /// # Errors
    ///
    /// The first rule the state breaks, in `tab_snapshot`'s order.
    pub fn from_json(state: &Value) -> Result<Self, WindowStateError> {
        let Some(fields) = state.as_object() else {
            return Err(WindowStateError::InvalidTab);
        };
        let uri = match fields.get("uri") {
            Some(uri) => navigation_location(uri)?,
            None => navigation_location(&json!(HOME_PAGE))?,
        };
        let history = match fields.get("history") {
            Some(history) => history_entries(history)?,
            None => vec![uri.clone()],
        };
        let index = history_position(fields.get("index"), history.len())?;
        let (history, index) = if history[index] == uri {
            (history, index)
        } else {
            (vec![uri.clone()], 0)
        };
        let selected = selected_values(fields.get("selection"))?;
        let scroll = scroll_position(fields.get("scroll"))?;
        Ok(Self {
            uri,
            history,
            index,
            scroll,
            selection: selected
                .into_iter()
                .map(item_location)
                .collect::<Result<_, _>>()?,
            view: view(fields),
            sort: sort_field(fields),
            direction: sort_direction(fields),
            settings_section: text_field(fields, "settingsSection").and_then(SettingsSection::from_name),
        })
    }

    /// The snapshot in the Python app's JSON form.
    pub fn to_json(&self) -> Value {
        json!({
            "uri": self.uri,
            "history": self.history,
            "index": self.index,
            "scroll": self.scroll,
            "selection": self.selection,
            "view": if self.view == TabView::Grid { "grid" } else { "details" },
            "sort": sort_name(self.sort),
            "descending": self.direction == SortDirection::Descending,
            "settingsSection": self.settings_section.map(SettingsSection::as_str),
        })
    }
}

/// Python's `location()`: a place the tab can navigate to, the app's own
/// pages included.
fn navigation_location(value: &Value) -> Result<String, WindowStateError> {
    let text = value.as_str().ok_or(WindowStateError::LocationNotText)?;
    let uri = location::normalise_navigation(text, None, &glib::home_dir())?;
    Ok(uri)
}

/// A selected item: a file, SMB or device location, never an app page.
/// Anything but text reads as no address, which is refused with the same
/// message as in Python.
fn item_location(value: &Value) -> Result<String, WindowStateError> {
    let text = value.as_str().unwrap_or_default();
    Ok(location::normalise_location(text, None, &glib::home_dir())?)
}

/// The history: 1 to [`MAX_HISTORY_ENTRIES`] navigable locations.
fn history_entries(value: &Value) -> Result<Vec<String>, WindowStateError> {
    let entries = value.as_array().ok_or(WindowStateError::HistoryLength)?;
    if !(1..=MAX_HISTORY_ENTRIES).contains(&entries.len()) {
        return Err(WindowStateError::HistoryLength);
    }
    entries.iter().map(navigation_location).collect()
}

/// The history position: a JSON integer (not a boolean or a fraction)
/// inside the history; the last entry when missing.
fn history_position(value: Option<&Value>, history_length: usize) -> Result<usize, WindowStateError> {
    let Some(value) = value else {
        return Ok(history_length - 1);
    };
    value
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < history_length)
        .ok_or(WindowStateError::HistoryPosition)
}

/// The selection's values: a list of at most [`MAX_SELECTED_ITEMS`];
/// none when missing.
fn selected_values(value: Option<&Value>) -> Result<Vec<&Value>, WindowStateError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value.as_array() {
        Some(items) if items.len() <= MAX_SELECTED_ITEMS => Ok(items.iter().collect()),
        _ => Err(WindowStateError::Selection),
    }
}

/// Python's `float(value)`, which must be finite, clamped to 0 to
/// [`MAX_SCROLL`]; 0 when missing. Text is read as a number, as `float`
/// reads it, except that Python also allows `_` between digits.
fn scroll_position(value: Option<&Value>) -> Result<f64, WindowStateError> {
    let number = match value {
        None => Some(0.0),
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::Bool(flag)) => Some(f64::from(u8::from(*flag))),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(_) => None,
    };
    let number = number
        .filter(|number: &f64| number.is_finite())
        .ok_or(WindowStateError::ScrollPosition)?;
    Ok(number.clamp(0.0, MAX_SCROLL))
}

/// `grid` or else details.
fn view(fields: &Map<String, Value>) -> TabView {
    if text_field(fields, "view") == Some("grid") {
        TabView::Grid
    } else {
        TabView::Details
    }
}

/// One of the four sort fields, or else name.
fn sort_field(fields: &Map<String, Value>) -> SortField {
    match text_field(fields, "sort") {
        Some("modified") => SortField::Modified,
        Some("type") => SortField::Type,
        Some("size") => SortField::Size,
        _ => SortField::Name,
    }
}

/// Descending only for a JSON `true`, as Python's `is True`.
fn sort_direction(fields: &Map<String, Value>) -> SortDirection {
    if fields.get("descending") == Some(&Value::Bool(true)) {
        SortDirection::Descending
    } else {
        SortDirection::Ascending
    }
}

/// The name of a sort field in the JSON form.
fn sort_name(field: SortField) -> &'static str {
    match field {
        SortField::Name => "name",
        SortField::Modified => "modified",
        SortField::Type => "type",
        SortField::Size => "size",
    }
}

/// A field's text, if it is text.
fn text_field<'a>(fields: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    fields.get(name).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_positions_are_read_as_python_float_reads_them() {
        let cases = [
            (json!(1600), Ok(1600.0)),
            (json!(-5), Ok(0.0)),
            (json!(2e9), Ok(MAX_SCROLL)),
            (json!(true), Ok(1.0)),
            (json!(" 12.5 "), Ok(12.5)),
            (json!("inf"), Err(WindowStateError::ScrollPosition)),
            (json!("nan"), Err(WindowStateError::ScrollPosition)),
            (json!("wide"), Err(WindowStateError::ScrollPosition)),
            (json!(null), Err(WindowStateError::ScrollPosition)),
            (json!([1]), Err(WindowStateError::ScrollPosition)),
        ];

        for (value, expected) in cases {
            assert_eq!(scroll_position(Some(&value)), expected, "{value}");
        }
        assert_eq!(scroll_position(None), Ok(0.0));
    }

    #[test]
    fn settings_sections_round_trip() {
        for section in SettingsSection::ALL {
            assert_eq!(SettingsSection::from_name(section.as_str()), Some(section));
        }
        assert_eq!(SettingsSection::from_name("Brave"), None);
    }
}
