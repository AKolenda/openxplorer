// SPDX-License-Identifier: AGPL-3.0-only
//! How a folder is displayed, saved for all folders or for each folder.
//!
//! Ports Dolphin's `ViewProperties` (`src/views/viewproperties.cpp`) and its
//! `GlobalViewProps` setting, as Windows' "Remember each folder's view
//! settings": with it on, the default as in Windows 11, each folder the
//! user restyles keeps its own (`folderViews`) and the others show the
//! default (`viewDefaults`); off, one style is shared by every folder.
//! Hidden files and thumbnails are not part of a style: they are one
//! setting for every folder, as in Windows, and the `showHidden` and
//! `showPreviews` keys of styles saved before are ignored. A style saved for
//! a folder "and its sub-folders" by Dolphin's View Display Style dialog,
//! which Folder Options replaced, still applies below it, as Dolphin's
//! `ApplyViewPropsJob` does; none is saved any more. The Python app reads
//! neither key.
//!
//! A folder's own entry can also hold the template the Properties
//! window's Customize tab optimized it for (Windows' "What kind of folder
//! do you want?", PROP-036), as `folderType`, with `subfolders` set when
//! "Also apply this template to all subfolders" was ticked. Customizing
//! saves the template's view into the entry, so the folder and, with
//! that box, its sub-folders show it as any saved style; nothing is
//! written into the folder itself.

use serde::Serialize;
use serde_json::Value;

use super::preferences::{read_column_widths, ColumnWidths};
use super::view_options::read_column_keys;

use crate::grouping::GroupBy;
use crate::location::{parent_location, same_location};

/// The most folders whose own style is kept; the oldest go first.
pub const MAX_FOLDER_VIEWS: usize = 500;

/// The longest location a folder style is kept for.
const MAX_FOLDER_LOCATION: usize = 4096;

/// The longest sort key kept, such as `modified`.
const MAX_SORT_KEY: usize = 32;

/// The icon sizes a style may hold, in pixels.
const ICON_SIZES: std::ops::RangeInclusive<u32> = 16..=256;

/// How one folder is displayed: Dolphin's view properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewProperties {
    /// `details`, `compact` or `icons`.
    pub mode: String,
    /// The icon edge of the icon view, in pixels (16 to 256).
    pub icon_size: u32,
    /// The sort key, such as `name` or `modified`.
    pub sort: String,
    /// Sorted descending.
    pub descending: bool,
    /// Items shown in groups: by [`Self::group_by`] when it is set, else
    /// by the sort key. Read the grouping with [`Self::grouping`].
    pub groups: bool,
    /// What the items are grouped by while `groups` is set, when it is a
    /// key of its own (Explorer's Group by) rather than the sort key.
    /// Stored only then, so styles without it keep their layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<GroupBy>,
    /// Folders listed before files.
    pub folders_first: bool,
    /// Visible details columns after Name, in their display order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_columns: Option<Vec<String>>,
    /// Saved widths, inheriting the shared layout until customized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_widths: Option<ColumnWidths>,
}

impl Default for ViewProperties {
    /// Details, by name ascending, folders first: a new installation.
    /// The icon view's size is Large icons, 96 pixels as in Windows 11
    /// (the web app's were 56).
    fn default() -> Self {
        Self {
            mode: "details".to_owned(),
            icon_size: 96,
            sort: "name".to_owned(),
            descending: false,
            groups: false,
            group_by: None,
            folders_first: true,
            details_columns: None,
            column_widths: None,
        }
    }
}

impl ViewProperties {
    /// The style stored in `value`: a missing or mistyped property keeps
    /// its default; `None` when `value` is not an object.
    pub fn from_json(value: &Value) -> Option<Self> {
        let values = value.as_object()?;
        let defaults = Self::default();
        let flag = |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        let key = |name: &str| {
            values
                .get(name)
                .and_then(Value::as_str)
                .filter(|key| is_short_key(key))
                .map(str::to_owned)
        };
        let icon_size = values
            .get("iconSize")
            .and_then(Value::as_u64)
            .and_then(|size| u32::try_from(size).ok())
            .filter(|size| ICON_SIZES.contains(size));
        Some(Self {
            mode: key("mode").unwrap_or(defaults.mode),
            icon_size: icon_size.unwrap_or(defaults.icon_size),
            sort: key("sort").unwrap_or(defaults.sort),
            descending: flag("descending", defaults.descending),
            groups: flag("groups", defaults.groups),
            group_by: values
                .get("groupBy")
                .and_then(Value::as_str)
                .and_then(GroupBy::from_key)
                .filter(|key| own_group_key(*key)),
            folders_first: flag("foldersFirst", defaults.folders_first),
            details_columns: values.get("detailsColumns").and_then(read_column_keys),
            column_widths: values
                .get("columnWidths")
                .and_then(read_column_widths)
                .map(|widths| ColumnWidths::from_values(&widths)),
        })
    }
}

impl ViewProperties {
    /// What the items are grouped by: nothing, the sort key (Dolphin's
    /// "Show in groups"), or a key of their own (Explorer's Group by).
    pub fn grouping(&self) -> GroupBy {
        if !self.groups {
            return GroupBy::None;
        }
        self.group_by.unwrap_or(GroupBy::SortKey)
    }

    /// Groups the items by `group_by`, storing it as [`Self::grouping`]
    /// reads it.
    pub fn set_grouping(&mut self, group_by: GroupBy) {
        self.groups = group_by.is_grouped();
        self.group_by = own_group_key(group_by).then_some(group_by);
    }
}

/// Whether `key` is a grouping of its own, stored as `groupBy`; no
/// grouping and the sort key are `groups` alone.
fn own_group_key(key: GroupBy) -> bool {
    !matches!(key, GroupBy::None | GroupBy::SortKey)
}

/// A key the app gives a mode or a sort order: short ASCII letters.
fn is_short_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_SORT_KEY && key.chars().all(|c| c.is_ascii_alphanumeric())
}

/// What a folder is optimized for: the templates of Windows' Customize
/// tab, in the order its list gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderTemplate {
    /// "General items", the template of a folder never customized.
    GeneralItems,
    /// "Documents".
    Documents,
    /// "Pictures".
    Pictures,
    /// "Music".
    Music,
    /// "Videos".
    Videos,
}

impl FolderTemplate {
    /// Every template, in the order Windows lists them.
    pub const ALL: [FolderTemplate; 5] = [
        FolderTemplate::GeneralItems,
        FolderTemplate::Documents,
        FolderTemplate::Pictures,
        FolderTemplate::Music,
        FolderTemplate::Videos,
    ];

    /// The key the settings keep it under.
    pub const fn as_str(self) -> &'static str {
        match self {
            FolderTemplate::GeneralItems => "generalItems",
            FolderTemplate::Documents => "documents",
            FolderTemplate::Pictures => "pictures",
            FolderTemplate::Music => "music",
            FolderTemplate::Videos => "videos",
        }
    }

    /// The template kept under `key`, if it is one.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|template| template.as_str() == key)
    }

    /// `style` as this template shows a folder, as Windows' templates
    /// pick their view: Pictures and Videos in Large icons, the others in
    /// Details. The sort, groups and columns stay as they are: the
    /// columns Windows adds (Tags and Rating for pictures, Contributing
    /// artists and Album for music, Length for videos) are not ones the
    /// Details view has.
    pub fn style(self, style: &ViewProperties) -> ViewProperties {
        let (mode, icon_size) = match self {
            FolderTemplate::Pictures | FolderTemplate::Videos => ("icons", LARGE_ICONS),
            FolderTemplate::GeneralItems | FolderTemplate::Documents | FolderTemplate::Music => {
                ("details", style.icon_size)
            }
        };
        ViewProperties {
            mode: mode.to_owned(),
            icon_size,
            ..style.clone()
        }
    }
}

/// The icon edge of Explorer's Large icons, in pixels.
const LARGE_ICONS: u32 = 56;

/// One folder's own style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    /// The folder.
    pub uri: String,
    /// Its style.
    #[serde(flatten)]
    pub properties: ViewProperties,
    /// The style applies to the folder's sub-folders too, unless they have
    /// their own: set by the Customize tab's "Also apply this template to
    /// all subfolders", and by the display style dialog before Folder
    /// Options replaced it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub subfolders: bool,
    /// The template the Customize tab optimized the folder for.
    #[serde(rename = "folderType", skip_serializing_if = "Option::is_none")]
    pub template: Option<FolderTemplate>,
}

impl FolderView {
    /// The folder style stored in `value`, if it names a folder that may
    /// be kept.
    fn from_json(value: &Value) -> Option<Self> {
        let uri = value.get("uri")?.as_str()?;
        if !may_remember(uri) {
            return None;
        }
        Some(Self {
            uri: uri.to_owned(),
            properties: ViewProperties::from_json(value)?,
            subfolders: value.get("subfolders").and_then(Value::as_bool).unwrap_or(false),
            template: value
                .get("folderType")
                .and_then(Value::as_str)
                .and_then(FolderTemplate::from_key),
        })
    }
}

/// Whether a style may be kept for `uri`: a bounded location without
/// control characters or a user name, so no account ends up in the file.
pub fn may_remember(uri: &str) -> bool {
    let authority = uri
        .split_once("://")
        .map_or("", |(_, rest)| rest.split('/').next().unwrap_or(""));
    !uri.is_empty()
        && uri.len() <= MAX_FOLDER_LOCATION
        && !uri.contains(char::is_control)
        && !authority.contains('@')
}

/// The folder styles stored in `value`, at most [`MAX_FOLDER_VIEWS`] of the
/// newest; `None` when `value` is not a list.
pub(super) fn read_folder_views(value: &Value) -> Option<Vec<FolderView>> {
    let stored: Vec<FolderView> = value
        .as_array()?
        .iter()
        .filter_map(FolderView::from_json)
        .collect();
    let skip = stored.len().saturating_sub(MAX_FOLDER_VIEWS);
    Some(stored.into_iter().skip(skip).collect())
}

/// How far a saved style reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewScope {
    /// The folder alone.
    Folder,
    /// Every folder: the style becomes the default and every folder's own
    /// style is dropped.
    AllFolders,
}

/// The style `uri` is shown in: its own, else that of the nearest folder
/// above it saved for its sub-folders, else `defaults`.
pub fn style_for(folder_views: &[FolderView], defaults: &ViewProperties, uri: &str) -> ViewProperties {
    saved_style_for(folder_views, uri).map_or_else(|| defaults.clone(), |saved| saved.properties.clone())
}

/// The saved style `uri` is shown in: its own, else the nearest one saved
/// for a folder above it and its sub-folders; `None` when it shows the
/// default.
pub fn saved_style_for<'a>(folder_views: &'a [FolderView], uri: &str) -> Option<&'a FolderView> {
    if let Some(own) = folder_views.iter().find(|view| same_location(&view.uri, uri)) {
        return Some(own);
    }
    let mut above = parent_location(uri);
    while let Some(folder) = above {
        let inherited = folder_views
            .iter()
            .find(|view| view.subfolders && same_location(&view.uri, &folder));
        if inherited.is_some() {
            return inherited;
        }
        above = parent_location(&folder);
    }
    None
}

/// The template `uri` is optimized for: its own, else that of the nearest
/// folder above it customized with its sub-folders; `None` when it was
/// never customized.
pub fn template_for(folder_views: &[FolderView], uri: &str) -> Option<FolderTemplate> {
    saved_style_for(folder_views, uri).and_then(|view| view.template)
}

/// Optimizes the folder at `uri` for `template` (the Customize tab's OK or
/// Apply): its entry keeps the template and `properties`, the template's
/// view of the style the folder had, and reaches its sub-folders when
/// `subfolders` is set, whose own styles are then dropped, as Windows
/// gives every sub-folder the template. A template once applied to the
/// sub-folders keeps reaching them: they have no entries of their own to
/// keep the earlier one. Returns whether `uri` may be kept.
pub(super) fn customize(
    folder_views: &mut Vec<FolderView>,
    uri: &str,
    template: FolderTemplate,
    properties: ViewProperties,
    subfolders: bool,
) -> bool {
    if !may_remember(uri) {
        return false;
    }
    let reached_below = folder_views
        .iter()
        .any(|view| same_location(&view.uri, uri) && view.subfolders && view.template.is_some());
    // The folder's own entry goes, and with the box every sub-folder's.
    let replaced =
        |view: &FolderView| same_location(&view.uri, uri) || (subfolders && is_inside(&view.uri, uri));
    folder_views.retain(|view| !replaced(view));
    folder_views.push(FolderView {
        uri: uri.to_owned(),
        properties,
        subfolders: subfolders || reached_below,
        template: Some(template),
    });
    let excess = folder_views.len().saturating_sub(MAX_FOLDER_VIEWS);
    folder_views.drain(..excess);
    true
}

/// Whether `uri` is below the folder `folder`.
fn is_inside(uri: &str, folder: &str) -> bool {
    let mut above = parent_location(uri);
    while let Some(parent) = above {
        if same_location(&parent, folder) {
            return true;
        }
        above = parent_location(&parent);
    }
    false
}

/// Saves `properties` for `uri` with `scope` into `folder_views` and
/// `defaults`. A location that may not be kept changes nothing but the
/// default of [`ViewScope::AllFolders`]. A folder customized on the
/// Customize tab keeps its template, and its sub-folders, through a
/// change of view; a sub-folder restyled keeps the template it inherits.
pub(super) fn remember(
    folder_views: &mut Vec<FolderView>,
    defaults: &mut Option<ViewProperties>,
    uri: &str,
    properties: ViewProperties,
    scope: ViewScope,
) {
    if scope == ViewScope::AllFolders {
        folder_views.clear();
        *defaults = Some(properties);
        return;
    }
    if !may_remember(uri) {
        return;
    }
    let template = template_for(folder_views, uri);
    let own = folder_views
        .iter()
        .position(|view| same_location(&view.uri, uri))
        .map(|own| folder_views.remove(own));
    folder_views.push(FolderView {
        uri: uri.to_owned(),
        properties,
        subfolders: own.is_some_and(|own| own.subfolders && own.template.is_some()),
        template,
    });
    let excess = folder_views.len().saturating_sub(MAX_FOLDER_VIEWS);
    folder_views.drain(..excess);
}

/// Forgets every folder's own style and the shared style, so that every
/// folder shows the default again, as Windows' Folder Options > Reset
/// Folders does. Returns whether anything was saved.
pub(super) fn reset(folder_views: &mut Vec<FolderView>, defaults: &mut Option<ViewProperties>) -> bool {
    let had_any = !folder_views.is_empty() || defaults.is_some();
    folder_views.clear();
    *defaults = None;
    had_any
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sorted_by(key: &str) -> ViewProperties {
        ViewProperties {
            sort: key.to_owned(),
            ..ViewProperties::default()
        }
    }

    /// Group by is stored apart from the sort only when it is a key of
    /// its own: no grouping and grouping by the sort key keep the layout
    /// of styles saved before it, which read back as before.
    ///
    /// parity: VIEW-022
    #[test]
    fn group_by_is_stored_only_as_a_key_of_its_own() {
        let mut style = ViewProperties::default();
        assert_eq!(style.grouping(), GroupBy::None);
        let plain = serde_json::to_value(&style).expect("a style");
        assert!(plain.get("groupBy").is_none(), "{plain}");

        style.set_grouping(GroupBy::SortKey);
        let by_sort = serde_json::to_value(&style).expect("a style");
        assert_eq!(by_sort["groups"], json!(true));
        assert!(by_sort.get("groupBy").is_none(), "Show in groups as before");
        assert_eq!(
            ViewProperties::from_json(&by_sort).expect("read").grouping(),
            GroupBy::SortKey
        );

        style.set_grouping(GroupBy::Modified);
        let by_date = serde_json::to_value(&style).expect("a style");
        assert_eq!(by_date["groups"], json!(true));
        assert_eq!(by_date["groupBy"], json!("modified"));
        assert_eq!(ViewProperties::from_json(&by_date), Some(style.clone()));

        style.set_grouping(GroupBy::None);
        assert_eq!(style, ViewProperties::default());
        let stray = json!({"groups": false, "groupBy": "name"});
        assert_eq!(
            ViewProperties::from_json(&stray).expect("read").grouping(),
            GroupBy::None,
            "groupBy alone groups nothing"
        );
        let unknown = json!({"groups": true, "groupBy": "colour"});
        assert_eq!(
            ViewProperties::from_json(&unknown).expect("read").grouping(),
            GroupBy::SortKey
        );
    }

    /// A folder's own style wins, a style the old display style dialog
    /// saved for sub-folders still reaches below its folder, and every
    /// other folder shows the default; saving for all folders drops the
    /// folders' own styles.
    ///
    /// parity: VIEW-020
    #[test]
    fn folders_keep_their_own_style_and_the_rest_share_the_default() {
        let mut views = Vec::new();
        let mut defaults = None;
        remember(
            &mut views,
            &mut defaults,
            "file:///a",
            sorted_by("size"),
            ViewScope::Folder,
        );
        let old = json!([{"uri": "file:///b", "sort": "type", "subfolders": true}]);
        views.extend(read_folder_views(&old).expect("a list"));
        remember(
            &mut views,
            &mut defaults,
            "sftp://me@host/x",
            sorted_by("type"),
            ViewScope::Folder,
        );
        let shared = ViewProperties::default();
        assert_eq!(style_for(&views, &shared, "file:///a/").sort, "size");
        assert_eq!(style_for(&views, &shared, "file:///a/inner").sort, "name");
        assert_eq!(style_for(&views, &shared, "file:///b/c/d").sort, "type");
        assert_eq!(views.len(), 2, "no user name is kept");

        let stored = serde_json::to_value(&views).expect("serialises");
        assert_eq!(read_folder_views(&stored), Some(views.clone()));
        assert_eq!(stored[1]["subfolders"], json!(true));

        remember(
            &mut views,
            &mut defaults,
            "file:///a",
            sorted_by("size"),
            ViewScope::AllFolders,
        );
        assert!(views.is_empty());
        assert_eq!(defaults.map(|style| style.sort).as_deref(), Some("size"));
    }

    /// Reset Folders forgets every folder's own style and the shared one;
    /// it reports whether anything was saved.
    ///
    /// parity: VIEW-020
    #[test]
    fn resetting_forgets_every_folders_style_and_the_shared_one() {
        let mut views = Vec::new();
        let mut defaults = Some(sorted_by("type"));
        remember(
            &mut views,
            &mut defaults,
            "file:///a",
            sorted_by("size"),
            ViewScope::Folder,
        );
        let old = json!([{"uri": "file:///b", "sort": "modified", "subfolders": true}]);
        views.extend(read_folder_views(&old).expect("a list"));

        let changed = reset(&mut views, &mut defaults);
        let again = reset(&mut views, &mut defaults);

        assert!(changed);
        assert!(!again, "nothing left to reset");
        assert!(views.is_empty());
        assert_eq!(defaults, None);
        assert_eq!(
            style_for(&views, &ViewProperties::default(), "file:///b/c").sort,
            "name"
        );
    }

    /// A template gives the folder its view, Large icons for Pictures and
    /// Videos and Details for the others, keeping its sort; with "Also
    /// apply this template to all subfolders" it reaches below, dropping
    /// the sub-folders' own styles. A later change of view keeps the
    /// template, and a sub-folder restyled keeps the one it inherits.
    ///
    /// parity: PROP-036
    #[test]
    fn a_template_picks_the_view_and_reaches_subfolders_when_asked() {
        let mut views = Vec::new();
        let mut defaults = None;
        remember(
            &mut views,
            &mut defaults,
            "file:///p/inner",
            sorted_by("size"),
            ViewScope::Folder,
        );
        let pictures = FolderTemplate::Pictures.style(&sorted_by("type"));
        assert_eq!(
            (pictures.mode.as_str(), pictures.icon_size, pictures.sort.as_str()),
            ("icons", 56, "type")
        );
        assert_eq!(FolderTemplate::Music.style(&pictures).mode, "details");

        assert!(customize(
            &mut views,
            "file:///p",
            FolderTemplate::Pictures,
            pictures.clone(),
            false
        ));
        assert_eq!(template_for(&views, "file:///p/"), Some(FolderTemplate::Pictures));
        assert_eq!(
            template_for(&views, "file:///p/other"),
            None,
            "not without the box"
        );
        assert_eq!(
            style_for(&views, &ViewProperties::default(), "file:///p/inner").sort,
            "size"
        );

        assert!(customize(
            &mut views,
            "file:///p",
            FolderTemplate::Videos,
            pictures.clone(),
            true
        ));
        assert_eq!(views.len(), 1, "the sub-folder's own style is dropped");
        assert_eq!(
            template_for(&views, "file:///p/inner/deep"),
            Some(FolderTemplate::Videos)
        );
        assert_eq!(
            style_for(&views, &ViewProperties::default(), "file:///p/inner").mode,
            "icons"
        );

        remember(
            &mut views,
            &mut defaults,
            "file:///p",
            sorted_by("name"),
            ViewScope::Folder,
        );
        remember(
            &mut views,
            &mut defaults,
            "file:///p/inner",
            sorted_by("size"),
            ViewScope::Folder,
        );
        assert_eq!(
            template_for(&views, "file:///p/elsewhere"),
            Some(FolderTemplate::Videos)
        );
        assert_eq!(
            template_for(&views, "file:///p/inner"),
            Some(FolderTemplate::Videos)
        );

        let stored = serde_json::to_value(&views).expect("serializes");
        assert_eq!(stored[0]["folderType"], json!("videos"));
        assert_eq!(stored[0]["subfolders"], json!(true));
        assert_eq!(read_folder_views(&stored), Some(views.clone()));
        let odd = json!([{"uri": "file:///q", "folderType": "contacts"}]);
        assert_eq!(read_folder_views(&odd).expect("a list")[0].template, None);
        assert!(customize(
            &mut views,
            "file:///p",
            FolderTemplate::Music,
            pictures.clone(),
            false
        ));
        assert_eq!(
            template_for(&views, "file:///p/elsewhere"),
            Some(FolderTemplate::Music)
        );
        assert!(!customize(
            &mut views,
            "sftp://me@host/x",
            FolderTemplate::Music,
            pictures,
            false
        ));
    }
}
