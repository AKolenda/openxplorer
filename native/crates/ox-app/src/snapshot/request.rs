// SPDX-License-Identifier: AGPL-3.0-only
//! What the developer asks the snapshot hook to show: the variables of
//! [`super`] and how they are read.
//!
//! Each variable is optional except `OPENXPLORER_SNAPSHOT`, whose presence
//! turns the hook on. A value the hook does not know is refused with the
//! variable's name and what it may hold, rather than ignored, so a typo
//! never produces a picture of something else.

use std::path::PathBuf;

use ox_core::settings::Theme;

use super::scene::SceneStep;
use super::SnapshotError;
use crate::settings_page::SettingsView;
use crate::window::FolderView;

/// The variable naming the PNG to write; its presence turns the hook on.
const SNAPSHOT_VARIABLE: &str = "OPENXPLORER_SNAPSHOT";
/// The first tab's location, a path or URI.
const START_VARIABLE: &str = "OPENXPLORER_START";
/// The theme to draw: `light`, `dark` or `system`.
const THEME_VARIABLE: &str = "OPENXPLORER_THEME";
/// The folder view: `details` or an icon size.
const VIEW_VARIABLE: &str = "OPENXPLORER_VIEW";
/// The window size, `<width>x<height>`.
const SIZE_VARIABLE: &str = "OPENXPLORER_SIZE";
/// The Settings category or page to open.
const SETTINGS_VARIABLE: &str = "OPENXPLORER_SETTINGS";
/// What to type into the settings search.
const SETTINGS_SEARCH_VARIABLE: &str = "OPENXPLORER_SETTINGS_SEARCH";
/// What to type into the window's search box.
const SEARCH_VARIABLE: &str = "OPENXPLORER_SEARCH";
/// The scene steps to run before the window is saved.
const SCENE_VARIABLE: &str = "OPENXPLORER_SCENE";
/// The JSON file to write the controls' rectangles to.
const HOTSPOTS_VARIABLE: &str = "OPENXPLORER_HOTSPOTS";

/// The size of a window's title bar and contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowSize {
    /// Width in logical pixels, as `gtk::Window::set_default_size` takes it.
    pub width: i32,
    /// Height in logical pixels.
    pub height: i32,
}

impl WindowSize {
    /// Parses `<width>x<height>`, such as `1440x900`.
    fn parse(text: &str) -> Option<Self> {
        let (width, height) = text.trim().split_once('x')?;
        let width = width.parse().ok().filter(|width| *width > 0)?;
        let height = height.parse().ok().filter(|height| *height > 0)?;
        Some(Self { width, height })
    }
}

/// What the developer asked the snapshot to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotRequest {
    /// The PNG to write.
    pub png: PathBuf,
    /// The first tab's location, or `None` for the home folder.
    pub start: Option<String>,
    /// The theme to draw, or `None` for the saved one.
    pub theme: Option<Theme>,
    /// The folder view to show, or `None` for the saved one.
    pub view: Option<FolderView>,
    /// The window size, or `None` for the app's default.
    pub size: Option<WindowSize>,
    /// The Settings page to open, or `None` to leave Settings closed.
    pub settings: Option<SettingsView>,
    /// What to type into the settings search, or `None` for nothing.
    pub settings_search: Option<String>,
    /// What to type into the window's search box, or `None` for nothing.
    pub search: Option<String>,
    /// The scene steps to run once the window is listed, in order.
    pub scene: Vec<SceneStep>,
    /// Where to write the controls' rectangles, or `None` for nowhere.
    pub hotspots: Option<PathBuf>,
}

impl SnapshotRequest {
    /// The request in the process environment, or `None` when
    /// `OPENXPLORER_SNAPSHOT` is not set.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::InvalidValue`] when a variable holds an unknown
    /// theme, view, size or scene step.
    pub(crate) fn from_environment() -> Result<Option<Self>, SnapshotError> {
        Self::from_variables(|name| std::env::var(name).ok())
    }

    /// The request that the variables `lookup` finds describe; separate
    /// from the process environment so it can be tested.
    fn from_variables(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, SnapshotError> {
        let Some(png) = non_empty(&lookup, SNAPSHOT_VARIABLE) else {
            return Ok(None);
        };
        let theme = parse_variable(&lookup, THEME_VARIABLE, "light, dark or system", |value| {
            Theme::from_key(value)
        })?;
        let view = parse_variable(&lookup, VIEW_VARIABLE, "details or an icon size", |value| {
            FolderView::from_key(value)
        })?;
        let size = parse_variable(&lookup, SIZE_VARIABLE, "<width>x<height>", WindowSize::parse)?;
        let settings = parse_variable(
            &lookup,
            SETTINGS_VARIABLE,
            "a settings category or page",
            SettingsView::from_key,
        )?;
        let scene = parse_variable(
            &lookup,
            SCENE_VARIABLE,
            "select=<name> or action=<name>[:<target>] steps separated by ;",
            SceneStep::parse_scene,
        )?;
        Ok(Some(Self {
            png: PathBuf::from(png),
            start: non_empty(&lookup, START_VARIABLE),
            theme,
            view,
            size,
            settings,
            settings_search: non_empty(&lookup, SETTINGS_SEARCH_VARIABLE),
            search: non_empty(&lookup, SEARCH_VARIABLE),
            scene: scene.unwrap_or_default(),
            hotspots: non_empty(&lookup, HOTSPOTS_VARIABLE).map(PathBuf::from),
        }))
    }
}

/// The value of the variable `name`, or `None` when it is unset or empty.
fn non_empty(lookup: impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    lookup(name).filter(|value| !value.is_empty())
}

/// Parses the optional variable `name` with `parse`.
///
/// # Errors
///
/// [`SnapshotError::InvalidValue`], naming `expected`, when `parse`
/// refuses the value.
fn parse_variable<T>(
    lookup: impl Fn(&str) -> Option<String>,
    name: &'static str,
    expected: &'static str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, SnapshotError> {
    let Some(value) = non_empty(lookup, name) else {
        return Ok(None);
    };
    match parse(&value) {
        Some(parsed) => Ok(Some(parsed)),
        None => Err(SnapshotError::InvalidValue {
            variable: name,
            value,
            expected,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::folder_view::grid::IconSize;

    fn request(variables: &[(&str, &str)]) -> Result<Option<SnapshotRequest>, SnapshotError> {
        let variables: HashMap<String, String> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        SnapshotRequest::from_variables(|name| variables.get(name).cloned())
    }

    #[test]
    fn without_a_snapshot_file_the_app_runs_normally() {
        let asked = request(&[(THEME_VARIABLE, "dark")]).expect("valid variables");
        assert_eq!(asked, None);
    }

    #[test]
    fn the_snapshot_variables_describe_the_window() {
        let asked = request(&[
            (SNAPSHOT_VARIABLE, "/tmp/window.png"),
            (START_VARIABLE, "pc:"),
            (THEME_VARIABLE, "dark"),
            (VIEW_VARIABLE, "large"),
            (SIZE_VARIABLE, "1440x900"),
        ]);
        let expected = SnapshotRequest {
            png: PathBuf::from("/tmp/window.png"),
            start: Some("pc:".to_owned()),
            theme: Some(Theme::Dark),
            view: Some(FolderView::Icons(IconSize::LARGE)),
            size: Some(WindowSize {
                width: 1440,
                height: 900,
            }),
            settings: None,
            settings_search: None,
            search: None,
            scene: Vec::new(),
            hotspots: None,
        };
        assert_eq!(asked.expect("valid variables"), Some(expected));
    }

    #[test]
    fn the_settings_variables_open_a_page_and_type_a_search() {
        let asked = request(&[
            (SNAPSHOT_VARIABLE, "/tmp/settings.png"),
            (SETTINGS_VARIABLE, "indexed-folders"),
            (SETTINGS_SEARCH_VARIABLE, "zoom"),
        ])
        .expect("valid variables")
        .expect("a snapshot is asked for");
        assert_eq!(
            asked.settings,
            Some(SettingsView::Subpage(
                crate::settings_page::Subpage::IndexedFolders
            ))
        );
        assert_eq!(asked.settings_search.as_deref(), Some("zoom"));
        assert_eq!(asked.search, None);
        let refused = request(&[(SNAPSHOT_VARIABLE, "a.png"), (SETTINGS_VARIABLE, "general")]);
        assert!(refused.is_err(), "general is not a settings page");
    }

    #[test]
    fn the_scene_variables_add_steps_and_a_hotspot_file() {
        let asked = request(&[
            (SNAPSHOT_VARIABLE, "/tmp/menu.png"),
            (SCENE_VARIABLE, "select=Notes.md;action=context-menu"),
            (HOTSPOTS_VARIABLE, "/tmp/menu.json"),
        ])
        .expect("valid variables")
        .expect("a snapshot is asked for");
        assert_eq!(asked.scene.len(), 2);
        assert_eq!(asked.hotspots, Some(PathBuf::from("/tmp/menu.json")));
        let refused = request(&[(SNAPSHOT_VARIABLE, "a.png"), (SCENE_VARIABLE, "rename=Notes.md")]);
        assert!(refused.is_err(), "rename is not a scene step");
    }

    #[test]
    fn an_unknown_value_is_refused_with_its_variable() {
        let refused = request(&[(SNAPSHOT_VARIABLE, "a.png"), (VIEW_VARIABLE, "tiles")]);
        let message = refused.expect_err("tiles is not a view").to_string();
        assert_eq!(
            message,
            "OPENXPLORER_VIEW cannot be “tiles”: expected details or an icon size"
        );
        let refused = request(&[(SNAPSHOT_VARIABLE, "a.png"), (SIZE_VARIABLE, "wide")]);
        assert!(refused.is_err(), "a size needs a width and a height");
    }
}
