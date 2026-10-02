// SPDX-License-Identifier: AGPL-3.0-only
//! Explorer's "Group by" for each folder (VIEW-022).
//!
//! The Sort menu's Group by choices group the active folder and are
//! remembered for that folder, as Explorer remembers a folder's view
//! (`folderGroupBy` in the settings). A folder never grouped by the user
//! is not grouped, except Downloads, which Explorer groups by date
//! modified out of the box. Opening a folder, switching tabs and going
//! back all apply the folder's grouping; grouping by date counts from the
//! day the folder was shown.

use gtk::glib;
use gtk::prelude::*;
use ox_core::grouping::GroupBy;
use ox_core::location::same_location;
use ox_core::places::KnownFolder;
use ox_core::settings::SettingsError;

use crate::settings_store::Change;

use super::window_action::WindowAction;
use super::BrowserWindow;

/// The Group by of the folder at `uri`: the user's choice for it in
/// `saved`, else by date for Downloads, else none.
fn folder_group_by<'a>(
    uri: &str,
    saved: impl IntoIterator<Item = (&'a String, &'a String)>,
    downloads: Option<&str>,
) -> GroupBy {
    let chosen = saved
        .into_iter()
        .find(|(folder, _)| same_location(folder, uri))
        .and_then(|(_, key)| GroupBy::from_key(key));
    chosen.unwrap_or_else(|| {
        if downloads.is_some_and(|downloads| same_location(downloads, uri)) {
            GroupBy::Modified
        } else {
            GroupBy::None
        }
    })
}

impl BrowserWindow {
    /// Groups the folder at `uri` as it was last grouped, and shows that
    /// choice in the Sort menu.
    pub(super) fn apply_folder_grouping(&self, uri: &str) {
        let group_by = self.group_by_for(uri);
        self.folder_pane().set_group_by(group_by);
        self.set_action_state(WindowAction::GroupBy, &group_by.as_str().to_variant());
    }

    /// The Group by the folder at `uri` opens with.
    fn group_by_for(&self, uri: &str) -> GroupBy {
        let context = self.context();
        let preferences = context.settings_data().preferences;
        let downloads = context
            .known_folders()
            .into_iter()
            .find(|place| place.known_folder == Some(KnownFolder::Downloads))
            .map(|place| place.uri);
        folder_group_by(uri, &preferences.folder_group_by, downloads.as_deref())
    }

    /// The Sort menu's Group by choice: groups the active folder by
    /// `group_by` and remembers it for that folder.
    pub(super) fn choose_group_by(&self, group_by: GroupBy) {
        let pane = self.folder_pane();
        if pane.set_group_by(group_by) {
            // The items all move; show the regrouped list from its top,
            // as Explorer does, rather than wherever GTK's scroll anchor
            // went (often the end).
            pane.restore_scroll_position(0.0);
        }
        self.update_content();
        let Some(uri) = self.current_uri() else {
            return;
        };
        let change: Change =
            Box::new(move |settings| settings.set_folder_group_by(&uri, Some(group_by)).map(|_| ()));
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    if let Err(error) = result {
                        window.show_message(&format!("Could not save how this folder is grouped: {error}"));
                    }
                }
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// parity: VIEW-022
    #[test]
    fn a_folder_opens_as_the_user_last_grouped_it() {
        let downloads = Some("file:///home/ana/Downloads");
        let mut saved = BTreeMap::new();
        assert_eq!(
            folder_group_by("file:///home/ana/Downloads/", &saved, downloads),
            GroupBy::Modified,
            "Downloads is grouped by date out of the box"
        );
        assert_eq!(
            folder_group_by("file:///home/ana/Music", &saved, downloads),
            GroupBy::None
        );
        assert_eq!(
            folder_group_by("file:///home/ana/Downloads", &saved, None),
            GroupBy::None
        );
        saved.insert("file:///home/ana/Downloads".to_owned(), "none".to_owned());
        saved.insert("file:///home/ana/Music".to_owned(), "type".to_owned());
        saved.insert("file:///home/ana/Notes".to_owned(), "colour".to_owned());
        assert_eq!(
            folder_group_by("file:///home/ana/Downloads", &saved, downloads),
            GroupBy::None
        );
        assert_eq!(
            folder_group_by("file:///home/ana/Music", &saved, downloads),
            GroupBy::Type
        );
        assert_eq!(
            folder_group_by("file:///home/ana/Notes", &saved, downloads),
            GroupBy::None
        );
    }
}
