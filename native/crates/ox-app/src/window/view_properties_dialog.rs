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
use ox_core::i18n::{gettext, gettext_static};
use ox_core::settings::{may_remember, PreferencesUpdate, ViewProperties, ViewScope};

use super::{BrowserWindow, ButtonStyle};
use crate::dialog::Dialog;
use crate::folder_view::sort_roles::{SortBy, SortRole};
use crate::folder_view::sorting::SortColumn;
use crate::i18n::message_id;

/// The view modes the dialog offers, as Dolphin's View mode list does.
const MODES: [(&str, &str); 3] = [
    ("details", message_id("Details")),
    ("compact", message_id("List")),
    ("icons", message_id("Icons")),
];

/// The scopes the dialog offers, in order.
const SCOPES: [(ViewScope, &str); 3] = [
    (ViewScope::Folder, message_id("This folder")),
    (
        ViewScope::FolderAndSubfolders,
        message_id("This folder and its subfolders"),
    ),
    (ViewScope::AllFolders, message_id("All folders")),
];

/// Every sort key the dialog offers, with its label.
fn sort_choices() -> Vec<(SortBy, &'static str)> {
    let columns = SortColumn::IN_SORT_MENU
        .into_iter()
        .map(|column| (SortBy::Column(column), column.label()));
    let roles = SortRole::ALL
        .into_iter()
        .map(|role| (SortBy::Role(role), role.label()));
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
    hidden_last: gtk::CheckButton,
    previews: gtk::CheckButton,
    columns: Vec<(SortColumn, gtk::CheckButton)>,
    scopes: Vec<gtk::CheckButton>,
    as_default: gtk::CheckButton,
}

impl StyleForm {
    /// The controls on `dialog`, showing `style`; the scopes only when each
    /// folder keeps its own style.
    fn add_to(dialog: &Dialog, style: &ViewProperties, per_folder: bool) -> Self {
        let mode_at = MODES.iter().position(|(mode, _)| *mode == style.mode);
        let mode = drop_down(
            &MODES.map(|(_, label)| gettext_static(label)),
            mode_at.unwrap_or(0),
        );
        dialog.add_labelled(&gettext("View mode"), &mode);
        let sorts = sort_choices();
        let sort_at = sorts.iter().position(|(by, _)| by.as_str() == style.sort);
        let labels: Vec<&str> = sorts.iter().map(|(_, label)| *label).collect();
        let sort = drop_down(&labels, sort_at.unwrap_or(0));
        dialog.add_labelled(&gettext("Sort by"), &sort);
        let descending = drop_down(
            &[gettext_static("Ascending"), gettext_static("Descending")],
            usize::from(style.descending),
        );
        dialog.add_labelled(&gettext("Order"), &descending);
        let groups = dialog.add_check_button(&gettext("Show in groups"), style.groups);
        let folders_first = dialog.add_check_button(&gettext("Show folders first"), style.folders_first);
        let hidden = dialog.add_check_button(&gettext("Show hidden files"), style.show_hidden);
        let hidden_last = dialog.add_check_button(&gettext("Show hidden items last"), style.hidden_last);
        let previews =
            dialog.add_check_button(&gettext("Show previews"), style.show_previews.unwrap_or(true));
        let columns_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let columns = SortColumn::CHOOSABLE
            .into_iter()
            .map(|column| {
                let shown = style
                    .details_columns
                    .as_ref()
                    .is_some_and(|columns| columns.iter().any(|key| key == column.as_str()));
                let check = gtk::CheckButton::with_label(column.label());
                check.set_active(shown);
                columns_box.append(&check);
                (column, check)
            })
            .collect();
        dialog.add_labelled(&gettext("Additional information"), &columns_box);
        let mut scopes: Vec<gtk::CheckButton> = Vec::new();
        for (index, (_, label)) in SCOPES.iter().enumerate() {
            let scope = dialog.add_check_button(&gettext(label), index == 0);
            scope.set_group(scopes.first());
            Dialog::set_field_visible(&scope, per_folder);
            scopes.push(scope);
        }
        let as_default = dialog.add_check_button(
            &gettext("Use these settings as the default for all folders"),
            false,
        );
        Dialog::set_field_visible(&as_default, per_folder);
        Self {
            mode,
            sort,
            descending,
            groups,
            folders_first,
            hidden,
            hidden_last,
            previews,
            columns,
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
        let selected: Vec<String> = self
            .columns
            .iter()
            .filter(|(_, check)| check.is_active())
            .map(|(column, _)| column.as_str().to_owned())
            .collect();
        let mut columns = shown.details_columns.clone().unwrap_or_default();
        columns.retain(|column| selected.contains(column));
        for column in selected {
            if !columns.contains(&column) {
                columns.push(column);
            }
        }
        ViewProperties {
            mode: mode.to_owned(),
            icon_size: shown.icon_size,
            sort: sort.to_owned(),
            descending: chosen(&self.descending) == 1,
            groups: self.groups.is_active(),
            folders_first: self.folders_first.is_active(),
            show_hidden: self.hidden.is_active(),
            hidden_last: self.hidden_last.is_active(),
            show_previews: Some(self.previews.is_active()),
            details_columns: Some(columns),
            column_widths: shown.column_widths.clone(),
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
            &gettext("Adjust view display style"),
            &gettext("Choose how the items of this folder are shown."),
        );
        let form = StyleForm::add_to(&dialog, &shown, per_folder);
        // Windows' Folder Options > View > Folder views.
        let apply_to_all = dialog.add_button(&gettext("Apply to all folders"), ButtonStyle::Bordered);
        let reset_all = dialog.add_button(&gettext("Reset folders"), ButtonStyle::Bordered);
        dialog.add_cancel_button();
        dialog.add_button(&gettext("OK"), ButtonStyle::Accent);
        dialog.open_on_first_button();
        let answer = dialog.next_response().await;
        dialog.finish();
        match answer {
            None => return,
            Some(button) if button == apply_to_all => return self.apply_view_to_all_folders(shown).await,
            Some(button) if button == reset_all => return self.reset_folder_views().await,
            Some(_) => {}
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

    /// Folder views > Apply to all folders: after asking, every folder
    /// shows `shown`, this folder's view, and keeps no style of its own.
    async fn apply_view_to_all_folders(&self, shown: ViewProperties) {
        let asked = self
            .confirm(
                message_id("Apply this view to all folders?"),
                message_id(
                    "Every folder will show this folder's layout, sorting, grouping and other view settings. \
                     Folders that kept a view of their own will lose it.",
                ),
                message_id("Apply"),
            )
            .await;
        if asked {
            self.context()
                .apply_view_to_all_folders(shown, self.preference_failure_reply());
        }
    }

    /// Folder views > Reset folders: after asking, every folder forgets
    /// its view and shows the default one.
    async fn reset_folder_views(&self) {
        let asked = self
            .confirm(
                message_id("Reset all folders to the default view?"),
                message_id(
                    "Every folder will forget its layout, sorting, grouping and other view settings, and show \
                     the default view.",
                ),
                message_id("Reset"),
            )
            .await;
        if asked {
            self.context().reset_folder_views(self.preference_failure_reply());
        }
    }

    /// Asks `title` with `message`; true when `action` was chosen.
    async fn confirm(&self, title: &str, message: &str, action: &str) -> bool {
        let dialog = Dialog::new(self, &gettext(title), &gettext(message));
        dialog.add_cancel_button();
        dialog.add_button(&gettext(action), ButtonStyle::Accent);
        dialog.open_on_first_button();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer.is_some()
    }

    /// Asks before a style replaces those of other folders, as Dolphin's
    /// dialog does.
    async fn confirm_wide_style(&self) -> bool {
        let dialog = Dialog::new(
            self,
            &gettext("Change the display style of other folders?"),
            &gettext("Folders that kept a display style of their own will show this one instead."),
        );
        dialog.add_cancel_button();
        dialog.add_button(&gettext("Change"), ButtonStyle::Accent);
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
