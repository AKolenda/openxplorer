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
use ox_core::grouping::GroupBy;
use ox_core::places::KnownFolder;
use ox_core::settings::{
    may_remember, ColumnWidths, Preferences, PreferencesUpdate, ViewProperties, ViewScope,
};

use super::folder_pane::{FolderPane, FolderView};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::details::GroupTitle;
use crate::folder_view::groups::Grouping;
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
        pane_sort_state(self.folder_pane())
    }

    /// Sorts by `state`, regroups, and shows it in the Sort menu, without
    /// saving it.
    pub(super) fn show_sort(&self, state: SortState) {
        let applying = self.imp().applying_style.replace(true);
        show_pane_sort(self.folder_pane(), state);
        self.show_sort_state();
        self.imp().applying_style.set(applying);
    }

    /// Brings the Sort menu and the groups in step with the sort order.
    pub(super) fn show_sort_state(&self) {
        let state = self.sort_state();
        self.set_action_state(WindowAction::Sort, &state.by.as_str().to_variant());
        self.set_action_state(WindowAction::Direction, &state.direction.as_str().to_variant());
        let pane = self.folder_pane();
        if let Some(grouping) = pane.model().grouping() {
            // Grouped by the sort key, the groups follow the new sort.
            group_pane(pane, Grouping::of(grouping.by, state));
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

    /// Group by: groups the listing by `by`, a key of its own or the sort
    /// key, or stops grouping it, without saving it.
    pub(super) fn show_group_by(&self, by: GroupBy) {
        let pane = self.folder_pane();
        group_pane(pane, Grouping::of(by, self.sort_state()));
        self.set_action_state(WindowAction::GroupBy, &by.as_str().to_variant());
        self.update_expandability();
    }

    /// Group by, from the menu: groups by the choice `key` and saves it.
    pub(super) fn group_by_key(&self, key: &str) -> bool {
        let Some(by) = GroupBy::from_key(key) else {
            return false;
        };
        self.show_group_by(by);
        self.remember_style();
        true
    }

    /// Lists folders before files, or among them.
    pub(super) fn show_folders_first(&self, first: bool) {
        self.folder_pane().model().set_folders_first(first);
        self.set_action_state(WindowAction::FoldersFirst, &first.to_variant());
    }

    /// The style the folder is shown in now.
    pub(super) fn current_style(&self) -> ViewProperties {
        Self::style_of_pane(self.folder_pane())
    }

    pub(super) fn style_of_pane(pane: &FolderPane) -> ViewProperties {
        let view = pane.view();
        let sort = pane_sort_state(pane);
        let model = pane.model();
        let mut style = ViewProperties {
            mode: view.style_mode().to_owned(),
            icon_size: match view {
                FolderView::Icons(size) => u32::try_from(size.pixels()).unwrap_or(56),
                FolderView::Details | FolderView::Compact => {
                    u32::try_from(pane.icon_view().icon_size().pixels()).unwrap_or(56)
                }
            },
            sort: sort.by.as_str().to_owned(),
            descending: sort.direction == SortDirection::Descending,
            groups: false,
            group_by: None,
            folders_first: model.folders_first(),
            show_hidden: model.shows_hidden(),
            hidden_last: model.hidden_last(),
            show_previews: Some(pane.previews_enabled()),
            details_columns: Some(
                pane.details()
                    .chosen_columns()
                    .iter()
                    .map(|column| column.as_str().to_owned())
                    .collect(),
            ),
            column_widths: Some(ColumnWidths::from_values(&pane.details().widths_to_save())),
        };
        style.set_grouping(model.grouping().map_or(GroupBy::None, |grouping| grouping.by));
        style
    }

    /// Shows the folder in `style`, without saving it.
    pub(super) fn apply_style(&self, style: &ViewProperties) {
        self.apply_style_to(self.folder_pane(), style);
        self.show_pane_view_state();
        self.update_content();
        self.update_details_pane();
    }

    /// Applies a style to its own pane without changing the active pane.
    pub(super) fn apply_style_to(&self, pane: &FolderPane, style: &ViewProperties) {
        let applying = self.imp().applying_style.replace(true);
        let view = FolderView::from_style(&style.mode, style.icon_size);
        pane.icon_view()
            .set_icon_size(crate::folder_view::grid::IconSize::nearest(style.icon_size));
        pane.show_view(view);
        let model = pane.model();
        model.set_folders_first(style.folders_first);
        model.set_hidden_last(style.hidden_last);
        let state = SortState {
            by: SortBy::from_key(&style.sort).unwrap_or(SortBy::Column(SortColumn::Name)),
            direction: if style.descending {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            },
        };
        show_pane_sort(pane, state);
        group_pane(pane, Grouping::of(style.grouping(), state));
        model.set_show_hidden(style.show_hidden);
        pane.set_previews_enabled(style.show_previews.unwrap_or(true));
        if let Some(columns) = &style.details_columns {
            pane.details()
                .show_chosen_columns(crate::folder_view::details::chosen_from_keys(columns));
        }
        pane.details().apply_column_widths(style.column_widths.as_ref());
        self.update_expandability_for(pane);
        self.imp().applying_style.set(applying);
    }

    /// Shows the folder at `uri` in its own style, when each folder keeps
    /// one (VIEW-020); with one style for all folders, the window keeps
    /// the style it shows, but for the groups of Downloads, which is
    /// grouped by date modified until the user chooses otherwise there
    /// (VIEW-022).
    pub(super) fn follow_folder_style(&self, uri: &str) {
        let preferences = self.context().settings_data().preferences;
        let downloads = self.downloads_location();
        let style = preferences.view_in(uri, downloads.as_deref());
        if preferences.per_folder_views {
            self.apply_style(&style);
            return;
        }
        let shown = self
            .folder_pane()
            .model()
            .grouping()
            .map_or(GroupBy::None, |grouping| grouping.by);
        if shown != style.grouping() {
            self.show_group_by(style.grouping());
        }
    }

    /// The Downloads folder's location, if the user has one.
    pub(super) fn downloads_location(&self) -> Option<String> {
        self.context()
            .known_folders()
            .into_iter()
            .find(|place| place.known_folder == Some(KnownFolder::Downloads))
            .map(|place| place.uri)
    }

    /// Saves the style shown now: for the folder shown when each folder
    /// keeps its own (a place that keeps none, such as a search, keeps it
    /// in this window only), else for every folder. Showing a saved style
    /// saves nothing.
    pub(super) fn remember_style(&self) {
        self.remember_pane_style(self.folder_pane());
    }

    /// A delayed header resize belongs to the pane that emitted it, even
    /// if focus has moved to its neighbour.
    pub(super) fn remember_pane_style(&self, pane: &FolderPane) {
        if self.imp().applying_style.get() {
            return;
        }
        let mut style = Self::style_of_pane(pane);
        let preferences = self.context().settings_data().preferences;
        let shown_uri = self
            .shown_panes()
            .into_iter()
            .find_map(|(side, uri)| (self.pane_on(side) == pane).then_some(uri));
        if preferences.per_folder_views {
            if let Some(uri) = shown_uri.filter(|uri| may_remember(uri)) {
                self.save_style(&uri, style, ViewScope::Folder);
            }
        } else {
            // Downloads keeps its own Group by apart from the shared style.
            let downloads = self.downloads_location();
            let in_downloads = shown_uri
                .as_deref()
                .zip(downloads.as_deref())
                .is_some_and(|(uri, downloads)| ox_core::location::same_location(uri, downloads));
            let mut downloads_group_by = None;
            if let (true, Some(uri)) = (in_downloads, shown_uri.as_deref()) {
                let chosen = style.grouping();
                if chosen != preferences.view_in(uri, downloads.as_deref()).grouping() {
                    downloads_group_by = Some(chosen);
                }
                style.set_grouping(preferences.view_for(uri).grouping());
            }
            let mut options = preferences.view_options;
            options.show_previews = pane.previews_enabled();
            options.details_columns = style.details_columns.clone().unwrap_or_default();
            let mut update = shared_style_update(style, pane.view());
            update.downloads_group_by = downloads_group_by;
            update.view_options = Some(options);
            update.column_widths = Some(pane.details().widths_to_save());
            self.context()
                .update_preferences(update, self.preference_failure_reply());
        }
    }

    /// Saves `style` for the folder at `uri` with `scope`, saying so when
    /// it could not be saved.
    pub(super) fn save_style(&self, uri: &str, style: ViewProperties, scope: ViewScope) {
        let reply = self.preference_failure_reply();
        self.context().remember_view(uri.to_owned(), style, scope, reply);
    }

    /// Follows the preferences of how items are shown: the date style
    /// (VIEW-004), redrawn if it changed, item check boxes (SEL-014) and
    /// expandable folders (VIEW-035).
    pub(super) fn follow_item_preferences(&self) {
        let preferences = self.context().settings_data().preferences;
        self.show_item_check_boxes(preferences.selection_marker);
        for pane in self.folder_panes() {
            pane.details().set_date_style(date_style(&preferences));
            self.update_expandability_for(pane);
        }
        for (side, uri) in self.shown_panes() {
            let pane = self.pane_on(side);
            let style = preferences.view_for(&uri);
            pane.set_previews_enabled(style.show_previews.unwrap_or(true));
            if let Some(columns) = &style.details_columns {
                pane.details()
                    .show_chosen_columns(crate::folder_view::details::chosen_from_keys(columns));
            }
        }
        self.apply_view_options();
    }
}

/// The pane's actual sorter, whether or not it has a visible column.
fn pane_sort_state(pane: &FolderPane) -> SortState {
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

/// Changes one pane's sorting without touching window action state.
/// Groups `pane`'s items by `grouping`, or stops grouping them with
/// `None`, with the details view's group headings to match.
///
/// The headings come off before the items change and go back on after.
/// GTK's list keeps a heading for each group it has shown; when its model
/// changes under the headings, between the grouped sort model and the
/// ungrouped tree or from one set of groups to another, GTK 4.22 aborts
/// in `gtk_list_item_manager_ensure_items` on a long list scrolled into
/// its groups. Nothing changes when the grouping, its day and the
/// headings already match.
pub(super) fn group_pane(pane: &FolderPane, grouping: Option<Grouping>) {
    let model = pane.model();
    let details = pane.details();
    let has_headings = details.column_view().header_factory().is_some();
    let unchanged = model.grouping() == grouping && !model.day_changed();
    if unchanged && has_headings == grouping.is_some() {
        return;
    }
    details.show_group_headers(None);
    model.set_grouping(grouping);
    if grouping.is_some() {
        let titles: GroupTitle = model.group_titles();
        details.show_group_headers(Some(titles));
    }
}

fn show_pane_sort(pane: &FolderPane, state: SortState) {
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
            details
                .column_view()
                .sort_by_column(None, state.direction.to_sort_type());
        }
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
