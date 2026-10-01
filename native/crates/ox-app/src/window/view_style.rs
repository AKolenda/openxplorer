// SPDX-License-Identifier: AGPL-3.0-only
//! The folder's display style: view, sorting, groups, folders first and
//! hidden files, shown and remembered as Dolphin's view properties.
//!
//! Ports Dolphin's `DolphinView::applyViewProperties` and
//! `ViewProperties::save` (VIEW-020): the style the user sets is saved for
//! every folder, or, with "Remember display style for each folder" on
//! (`perFolderViews`), for the folder shown, and a folder opened later is
//! shown in its own style. The Sort menu's further keys (VIEW-019), groups
//! (VIEW-022) and the date style (VIEW-004) are applied here too.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format::DateStyle;
use ox_core::settings::{may_remember, Preferences, PreferencesUpdate, ViewProperties, ViewScope};

use super::folder_pane::FolderView;
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::details::GroupTitle;
use crate::folder_view::sort_roles::{SortBy, SortState};
use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};

/// The date style the preferences ask for.
pub(super) fn date_style(preferences: &Preferences) -> DateStyle {
    if preferences.absolute_dates {
        DateStyle::Absolute
    } else {
        DateStyle::Relative
    }
}

/// The preferences change that saves `style` as every folder's, with the
/// view and hidden files the Python app reads too.
fn shared_style_update(style: ViewProperties, view: FolderView) -> PreferencesUpdate {
    PreferencesUpdate {
        view: Some(view.setting()),
        show_hidden: Some(style.show_hidden),
        view_defaults: Some(style),
        ..PreferencesUpdate::default()
    }
}

impl BrowserWindow {
    /// What the listing sorts by: a further key while one is chosen, else
    /// the details view's column.
    pub(super) fn sort_state(&self) -> SortState {
        let pane = self.folder_pane();
        if let Some((role, direction)) = pane.model().sort_role() {
            return SortState {
                by: SortBy::Role(role),
                direction,
            };
        }
        let order = pane.details().sort_order();
        SortState {
            by: SortBy::Column(order.column),
            direction: order.direction,
        }
    }

    /// Sorts by `state`, regroups, and shows it in the Sort menu, without
    /// saving it.
    pub(super) fn show_sort(&self, state: SortState) {
        let applying = self.imp().applying_style.replace(true);
        let pane = self.folder_pane();
        let details = pane.details();
        match state.by {
            SortBy::Column(column) => {
                pane.model().set_sort_role(None);
                details.sort_by(SortOrder {
                    column,
                    direction: state.direction,
                });
            }
            SortBy::Role(role) => {
                pane.model().set_sort_role(Some((role, state.direction)));
                // The titles show no arrow for a key that is not a column.
                details.column_view().sort_by_column(None, state.direction.to_sort_type());
            }
        }
        self.show_sort_state();
        self.imp().applying_style.set(applying);
    }

    /// Brings the Sort menu and the groups in step with the sort order.
    pub(super) fn show_sort_state(&self) {
        let state = self.sort_state();
        self.set_action_state(WindowAction::Sort, &state.by.as_str().to_variant());
        self.set_action_state(WindowAction::Direction, &state.direction.as_str().to_variant());
        let model = self.folder_pane().model();
        if model.grouping().is_some() {
            model.set_grouping(Some(state));
        }
    }

    /// Sorts by the key `key` in the current direction, and saves it.
    pub(super) fn sort_by_key(&self, key: &str) -> bool {
        let Some(by) = SortBy::from_key(key) else {
            return false;
        };
        let direction = self.sort_state().direction;
        self.show_sort(SortState { by, direction });
        self.remember_style();
        true
    }

    /// Sorts the current key in `direction`, and saves it.
    pub(super) fn sort_in_direction(&self, direction: SortDirection) {
        let by = self.sort_state().by;
        self.show_sort(SortState { by, direction });
        self.remember_style();
    }

    /// "Show in groups": groups the listing by its sort key, or stops.
    pub(super) fn show_groups(&self, grouped: bool) {
        let pane = self.folder_pane();
        let model = pane.model();
        model.set_grouping(grouped.then(|| self.sort_state()));
        let titles: Option<GroupTitle> = grouped.then(|| model.group_titles());
        pane.details().show_group_headers(titles);
        self.set_action_state(WindowAction::Groups, &grouped.to_variant());
        self.update_expandability();
    }

    /// Lists folders before files, or among them.
    pub(super) fn show_folders_first(&self, first: bool) {
        self.folder_pane().model().set_folders_first(first);
        self.set_action_state(WindowAction::FoldersFirst, &first.to_variant());
    }

    /// The style the folder is shown in now.
    pub(super) fn current_style(&self) -> ViewProperties {
        let pane = self.folder_pane();
        let view = pane.view();
        let sort = self.sort_state();
        let model = pane.model();
        ViewProperties {
            mode: view.style_mode().to_owned(),
            icon_size: match view {
                FolderView::Icons(size) => u32::try_from(size.pixels()).unwrap_or(56),
                FolderView::Details | FolderView::Compact => {
                    u32::try_from(pane.icon_view().icon_size().pixels()).unwrap_or(56)
                }
            },
            sort: sort.by.as_str().to_owned(),
            descending: sort.direction == SortDirection::Descending,
            groups: model.grouping().is_some(),
            folders_first: model.folders_first(),
            show_hidden: model.shows_hidden(),
        }
    }

    /// Shows the folder in `style`, without saving it.
    pub(super) fn apply_style(&self, style: &ViewProperties) {
        let applying = self.imp().applying_style.replace(true);
        let view = FolderView::from_style(&style.mode, style.icon_size);
        self.set_action_state(WindowAction::View, &view.as_str().to_variant());
        self.show_view(view);
        self.show_folders_first(style.folders_first);
        let by = SortBy::from_key(&style.sort).unwrap_or(SortBy::Column(SortColumn::Name));
        let direction = if style.descending {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
        self.show_sort(SortState { by, direction });
        self.show_groups(style.groups);
        if self.folder_pane().model().shows_hidden() != style.show_hidden {
            self.set_action_state(WindowAction::Hidden, &style.show_hidden.to_variant());
            self.show_hidden_files(style.show_hidden);
        }
        self.imp().applying_style.set(applying);
    }

    /// Shows the folder at `uri` in its own style, when each folder keeps
    /// one (VIEW-020); with one style for all folders, the window keeps
    /// the style it shows.
    pub(super) fn follow_folder_style(&self, uri: &str) {
        let preferences = self.context().settings_data().preferences;
        if preferences.per_folder_views {
            self.apply_style(&preferences.view_for(uri));
        }
    }

    /// Saves the style shown now: for the folder shown when each folder
    /// keeps its own, else for every folder. Showing a saved style saves
    /// nothing.
    pub(super) fn remember_style(&self) {
        if self.imp().applying_style.get() {
            return;
        }
        let style = self.current_style();
        let per_folder = self.context().settings_data().preferences.per_folder_views;
        match self.current_uri().filter(|uri| per_folder && may_remember(uri)) {
            Some(uri) => self.save_style(&uri, style, ViewScope::Folder),
            None => {
                let view = self.folder_pane().view();
                let reply = self.preference_failure_reply();
                self.context().update_preferences(shared_style_update(style, view), reply);
            }
        }
    }

    /// Saves `style` for the folder at `uri` with `scope`, saying so when
    /// it could not be saved.
    pub(super) fn save_style(&self, uri: &str, style: ViewProperties, scope: ViewScope) {
        let reply = self.preference_failure_reply();
        self.context().remember_view(uri.to_owned(), style, scope, reply);
    }

    /// Follows the preferences of how items are shown: the date style
    /// (VIEW-004), redrawn if it changed, the selection marker (SEL-014)
    /// and expandable folders (VIEW-035).
    pub(super) fn follow_item_preferences(&self) {
        let preferences = self.context().settings_data().preferences;
        let pane = self.folder_pane();
        pane.details().set_date_style(date_style(&preferences));
        pane.owners().set_selection_markers(preferences.selection_marker);
        self.update_expandability();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One style for all folders also saves the view and hidden files the
    /// Python app reads.
    #[test]
    fn the_shared_style_keeps_the_python_keys_in_step() {
        let style = ViewProperties {
            show_hidden: true,
            ..ViewProperties::default()
        };
        let update = shared_style_update(style.clone(), FolderView::Compact);
        assert_eq!(update.view, Some(ox_core::settings::View::Details));
        assert_eq!(update.show_hidden, Some(true));
        assert_eq!(update.view_defaults, Some(style));
    }
}
