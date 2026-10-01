// SPDX-License-Identifier: AGPL-3.0-only
//! "Adjust view display style…": Dolphin's View Display Style dialog
//! (VIEW-021).
//!
//! Ports `ViewPropertiesDialog` (`src/settings/viewpropertiesdialog.cpp`):
//! the view mode, the sort key and order, groups, folders first and hidden
//! files in one place. While each folder keeps its own style (VIEW-020) the
//! choices apply to the folder shown, to it and its sub-folders, or to all
//! folders, the last two after a confirmation, and can also become the
//! default style; with one style for every folder they apply everywhere, as
//! Dolphin hides the choice then. Built on the app's one [`Dialog`].

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::{may_remember, PreferencesUpdate, ViewProperties, ViewScope};

use super::dialog::Dialog;
use super::{BrowserWindow, ButtonStyle};
use crate::folder_view::sort_roles::{SortBy, SortRole};
use crate::folder_view::sorting::SortColumn;

/// The view modes the dialog offers, as Dolphin's View mode list does.
const MODES: [(&str, &str); 3] = [("details", "Details"), ("compact", "List"), ("icons", "Icons")];

/// The scopes the dialog offers, in order.
const SCOPES: [(ViewScope, &str); 3] = [
    (ViewScope::Folder, "This folder"),
    (ViewScope::FolderAndSubfolders, "This folder and its subfolders"),
    (ViewScope::AllFolders, "All folders"),
];

/// Every sort key the dialog offers, with its label.
fn sort_choices() -> Vec<(SortBy, &'static str)> {
    let columns = SortColumn::IN_SORT_MENU
        .into_iter()
        .map(|column| (SortBy::Column(column), column.label()));
    let roles = SortRole::ALL.into_iter().map(|role| (SortBy::Role(role), role.label()));
    columns.chain(roles).collect()
}

/// A drop-down of `labels` named `name`, showing `selected`.
fn drop_down(labels: &[&str], selected: usize) -> gtk::DropDown {
    let choice = gtk::DropDown::from_strings(labels);
    choice.set_halign(gtk::Align::Start);
    choice.set_selected(u32::try_from(selected).unwrap_or(0));
    choice
}

/// The dialog's controls.
struct StyleForm {
    mode: gtk::DropDown,
    sort: gtk::DropDown,
    descending: gtk::DropDown,
    groups: gtk::CheckButton,
    folders_first: gtk::CheckButton,
    hidden: gtk::CheckButton,
    scopes: Vec<gtk::CheckButton>,
    as_default: gtk::CheckButton,
}

impl StyleForm {
    /// The controls on `dialog`, showing `style`; the scopes only when each
    /// folder keeps its own style.
    fn add_to(dialog: &Dialog, style: &ViewProperties, per_folder: bool) -> Self {
        let mode_at = MODES.iter().position(|(mode, _)| *mode == style.mode);
        let mode = drop_down(&MODES.map(|(_, label)| label), mode_at.unwrap_or(0));
        dialog.add_labelled("View mode", &mode);
        let sorts = sort_choices();
        let sort_at = sorts.iter().position(|(by, _)| by.as_str() == style.sort);
        let labels: Vec<&str> = sorts.iter().map(|(_, label)| *label).collect();
        let sort = drop_down(&labels, sort_at.unwrap_or(0));
        dialog.add_labelled("Sort by", &sort);
        let descending = drop_down(&["Ascending", "Descending"], usize::from(style.descending));
        dialog.add_labelled("Order", &descending);
        let groups = dialog.add_check_button("Show in groups", style.groups);
        let folders_first = dialog.add_check_button("Show folders first", style.folders_first);
        let hidden = dialog.add_check_button("Show hidden files", style.show_hidden);
        let mut scopes: Vec<gtk::CheckButton> = Vec::new();
        for (index, (_, label)) in SCOPES.iter().enumerate() {
            let scope = dialog.add_check_button(label, index == 0);
            scope.set_group(scopes.first());
            Dialog::set_field_visible(&scope, per_folder);
            scopes.push(scope);
        }
        let as_default = dialog.add_check_button("Use these settings as the default for all folders", false);
        Dialog::set_field_visible(&as_default, per_folder);
        Self {
            mode,
            sort,
            descending,
            groups,
            folders_first,
            hidden,
            scopes,
            as_default,
        }
    }

    /// The style chosen, keeping `shown`'s icon size.
    fn style(&self, shown: &ViewProperties) -> ViewProperties {
        let chosen = |choice: &gtk::DropDown| choice.selected() as usize;
        let mode = MODES.get(chosen(&self.mode)).map_or("details", |(mode, _)| mode);
        let sort = sort_choices()
            .get(chosen(&self.sort))
            .map_or(SortColumn::Name.as_str(), |(by, _)| by.as_str());
        ViewProperties {
            mode: mode.to_owned(),
            icon_size: shown.icon_size,
            sort: sort.to_owned(),
            descending: chosen(&self.descending) == 1,
            groups: self.groups.is_active(),
            folders_first: self.folders_first.is_active(),
            show_hidden: self.hidden.is_active(),
        }
    }

    /// The scope chosen.
    fn scope(&self) -> ViewScope {
        let chosen = self.scopes.iter().position(gtk::CheckButton::is_active);
        SCOPES[chosen.unwrap_or(0)].0
    }
}

impl BrowserWindow {
    /// Opens the Adjust View Display Style dialog for the folder shown.
    pub(super) fn show_view_properties(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.ask_for_view_style().await }
        ));
    }

    /// Asks for the style, confirms a wide scope, then shows and saves it.
    async fn ask_for_view_style(&self) {
        let shown = self.current_style();
        let per_folder = self.context().settings_data().preferences.per_folder_views;
        let dialog = Dialog::new(
            self,
            "Adjust view display style",
            "Choose how the items of this folder are shown.",
        );
        let form = StyleForm::add_to(&dialog, &shown, per_folder);
        dialog.add_cancel_button();
        dialog.add_button("OK", ButtonStyle::Accent);
        dialog.open_on_first_button();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer.is_none() {
            return;
        }
        let style = form.style(&shown);
        let scope = per_folder.then(|| form.scope());
        if scope.is_some_and(|scope| scope != ViewScope::Folder) && !self.confirm_wide_style().await {
            return;
        }
        self.apply_style(&style);
        self.folder_pane().restore_scroll_position(0.0);
        self.save_chosen_style(style, scope, form.as_default.is_active());
    }

    /// Asks before a style replaces those of other folders, as Dolphin's
    /// dialog does.
    async fn confirm_wide_style(&self) -> bool {
        let dialog = Dialog::new(
            self,
            "Change the display style of other folders?",
            "Folders that kept a display style of their own will show this one instead.",
        );
        dialog.add_cancel_button();
        dialog.add_button("Change", ButtonStyle::Accent);
        dialog.open_on_first_button();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer.is_some()
    }

    /// Saves `style` with `scope` for the folder shown, or for every folder
    /// without a scope; `as_default` makes it the shared style too.
    fn save_chosen_style(&self, style: ViewProperties, scope: Option<ViewScope>, as_default: bool) {
        let folder = self.current_uri().filter(|uri| may_remember(uri));
        match (scope, folder) {
            (Some(scope), Some(uri)) => self.save_style(&uri, style.clone(), scope),
            // All folders needs no folder: it replaces the shared style.
            (Some(ViewScope::AllFolders), None) => self.save_style("", style.clone(), ViewScope::AllFolders),
            (Some(_), None) => {}
            (None, _) => self.remember_style(),
        }
        if as_default {
            let update = PreferencesUpdate {
                view_defaults: Some(style),
                ..PreferencesUpdate::default()
            };
            self.context()
                .update_preferences(update, self.preference_failure_reply());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::folder_pane::FolderView;

    #[test]
    fn every_sort_key_and_mode_is_offered_once() {
        let keys: Vec<&str> = sort_choices().iter().map(|(by, _)| by.as_str()).collect();
        let unique: std::collections::HashSet<&str> = keys.iter().copied().collect();
        assert_eq!(keys.len(), unique.len());
        assert!(keys.contains(&"owner") && keys.contains(&"size"));
        for (mode, _) in MODES {
            assert_eq!(FolderView::from_style(mode, 56).style_mode(), mode);
        }
    }
}
