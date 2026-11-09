// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' options in a window: previews (VIEW-057, VIEW-058)
//! and item counts (VIEW-037) for the folder shown, and the details
//! columns the user chose from the titles' menu or Choose Details
//! (VIEW-033) and dragged into order (VIEW-034).
//!
//! Previews and item counts follow the folder: a network folder gets
//! neither unless Settings allow previews there, as Dolphin skips remote
//! files. Every window follows a change made in Settings or another
//! window, as all windows share the preferences.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::i18n::gettext;
use ox_core::location::{location_kind, LocationKind};

use super::actions::{plain_action, text_action};
use super::folder_pane::{FolderView, PanePage};
use super::menu_popover::{ItemCheck, MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::sorting::SortColumn;
use crate::icons::Icon;
use crate::thumbnails::PreviewPolicy;

/// The columns the titles' menu shows and hides, in its order: Windows
/// 11 File Explorer's Name, Date modified, Type, Size and Date created,
/// then the app's own columns. Windows' Authors, Tags and Title are left
/// out, as the listing does not read them.
const MENU_COLUMNS: [SortColumn; 8] = [
    SortColumn::Name,
    SortColumn::Modified,
    SortColumn::Type,
    SortColumn::Size,
    SortColumn::Created,
    SortColumn::Extension,
    SortColumn::Owner,
    SortColumn::Permissions,
];

/// The titles' menu, as Windows 11 File Explorer's: "Size Column to Fit"
/// for the column it was opened on (`under`, unavailable outside the
/// titles), "Size All Columns to Fit", the columns that can be shown or
/// hidden, checked while they show (Name checked and greyed, as it cannot
/// be hidden), and "More...", which opens Choose Details. The rows show
/// check marks only, no icons.
fn column_menu_entries(chosen: &[SortColumn], under: Option<SortColumn>) -> Vec<MenuEntry> {
    let fit_one = MenuItem::with_text_target(
        &gettext("Size Column to Fit"),
        Icon::Document,
        WindowAction::FitColumn,
        under.map_or("", SortColumn::as_str),
    )
    .without_glyph()
    .disabled_when(under.is_none());
    let fit_all = MenuItem::new(
        &gettext("Size All Columns to Fit"),
        Icon::Document,
        WindowAction::FitAllColumns,
    )
    .without_glyph();
    let more =
        MenuItem::new(&gettext("More..."), Icon::Document, WindowAction::ChooseDetails).without_glyph();
    let mut entries = vec![
        MenuEntry::Item(fit_one),
        MenuEntry::Item(fit_all),
        MenuEntry::Divider,
    ];
    entries.extend(column_toggle_entries(chosen));
    entries.push(MenuEntry::Divider);
    entries.push(MenuEntry::Item(more));
    entries
}

/// The titles' menu items that show and hide columns.
fn column_toggle_entries(chosen: &[SortColumn]) -> Vec<MenuEntry> {
    let item = |column: SortColumn| {
        let is_name = column == SortColumn::Name;
        let item = MenuItem::with_text_target(
            column.label(),
            Icon::Document,
            WindowAction::DetailsColumn,
            column.as_str(),
        )
        .without_glyph()
        .disabled_when(is_name);
        MenuEntry::Item(MenuItem {
            check: ItemCheck::Fixed(is_name || chosen.contains(&column)),
            ..item
        })
    };
    MENU_COLUMNS.into_iter().map(item).collect()
}

/// `chosen` with `column` shown at the end, or hidden. Name always shows.
fn toggled(mut chosen: Vec<SortColumn>, column: SortColumn) -> Vec<SortColumn> {
    if !SortColumn::CHOOSABLE.contains(&column) {
        return chosen;
    }
    if chosen.contains(&column) {
        chosen.retain(|shown| *shown != column);
    } else {
        chosen.push(column);
    }
    chosen
}

impl BrowserWindow {
    /// Adds `win.details-column`, opens the titles' menu on a secondary
    /// click and saves the columns' order after a drag.
    pub(super) fn install_view_option_actions(&self) {
        let toggle = text_action(WindowAction::DetailsColumn, |window, key| {
            if let Some(column) = SortColumn::from_key(key) {
                window.toggle_details_column(column);
            }
        });
        let fit_one = text_action(WindowAction::FitColumn, |window, key| {
            if let Some(column) = SortColumn::from_key(key) {
                window.fit_details_columns(Some(column));
            }
        });
        let fit_all = plain_action(WindowAction::FitAllColumns, |window| {
            window.fit_details_columns(None);
        });
        let choose = plain_action(WindowAction::ChooseDetails, BrowserWindow::open_choose_details);
        self.add_action_entries([toggle, fit_one, fit_all, choose]);
        for pane in self.folder_panes() {
            let details = pane.details();
            details.connect_columns_chosen(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                pane,
                move |_| window.remember_pane_style(&pane)
            ));
            let Some(header) = details.header() else { continue };
            let column_view = details.column_view().clone();
            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_SECONDARY);
            // A real right-click on a title never reached a gesture in the
            // bubble phase, so the menu did not open: it takes the press
            // before the titles do. It sits on the column view, not the
            // titles' row, which ends at the last column: a right-click
            // on the empty part of the row beyond opens the menu too, as
            // in Windows.
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            // The press is claimed, so the titles do not take it, and the
            // menu opens on the release, as Windows opens it and as the
            // file list's menu opens: opened on the press, the menu lost
            // the keyboard to the list when the button came up.
            let in_titles = glib::clone!(
                #[weak]
                header,
                #[weak]
                column_view,
                #[upgrade_or]
                None,
                move |x: f64, y: f64| {
                    #[expect(clippy::cast_possible_truncation, reason = "a pointer position fits in f32")]
                    let press = gtk::graphene::Point::new(x as f32, y as f32);
                    let at = column_view.compute_point(&header, &press)?;
                    #[expect(clippy::cast_precision_loss, reason = "a row of titles is a few pixels high")]
                    let height = header.height() as f32;
                    (0.0..height).contains(&at.y()).then_some(at)
                }
            );
            let in_titles_on_press = in_titles.clone();
            click.connect_pressed(move |gesture, _, x, y| {
                if in_titles_on_press(x, y).is_some() {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            });
            click.connect_released(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                header,
                move |_, _, x, y| {
                    let Some(at) = in_titles(x, y) else { return };
                    if let Some(side) = window.side_holding(&header) {
                        window.activate_pane(side);
                    }
                    window.show_column_menu(&header, f64::from(at.x()), f64::from(at.y()));
                }
            ));
            column_view.add_controller(click);
        }
    }

    /// The titles' menu at (`x`, `y`) in `header`. It belongs to the
    /// details view: the titles' row takes no keyboard focus, and a menu
    /// inside it could not take the keyboard either. The view keeps one
    /// menu, shown again each time, as the file list keeps its menu: a
    /// menu destroyed as it closed left GTK pointing at its gone surface
    /// while the pointer was still over it.
    pub(super) fn show_column_menu(&self, header: &gtk::Widget, x: f64, y: f64) -> MenuPopover {
        let details = self.folder_pane().details();
        let chosen = details.chosen_columns();
        #[expect(clippy::cast_possible_truncation, reason = "a pointer position fits in f32")]
        let point = gtk::graphene::Point::new(x as f32, y as f32);
        let at = header.compute_point(details, &point).unwrap_or(point);
        let kept = std::iter::successors(details.first_child(), WidgetExt::next_sibling)
            .find_map(|child| child.downcast::<MenuPopover>().ok());
        let popover = kept.unwrap_or_else(|| {
            let popover = MenuPopover::new(Vec::new());
            popover.set_parent(details);
            popover
        });
        // The rows are drawn once the menu is in the window, which enables
        // them: the keyboard starts on the first one as the menu maps.
        popover.set_entries(column_menu_entries(&chosen, details.column_at(x)));
        #[expect(clippy::cast_possible_truncation, reason = "a pointer position fits in i32")]
        popover.set_pointing_to(Some(&gdk::Rectangle::new(at.x() as i32, at.y() as i32, 1, 1)));
        popover.popup();
        popover
    }

    /// Opens the titles' menu on the title of the column whose key is
    /// `key`, for the snapshot hook's `titles-menu=` step; false when the
    /// details view does not show it.
    pub(crate) fn open_titles_menu_on(&self, key: &str) -> bool {
        let details = self.folder_pane().details();
        let at = SortColumn::from_key(key).and_then(|column| details.title_middle(column));
        let (Some(x), Some(header)) = (at, details.header()) else {
            return false;
        };
        self.show_column_menu(&header, x, f64::from(header.height()));
        true
    }

    /// Shows or hides `column` and saves the choice.
    fn toggle_details_column(&self, column: SortColumn) {
        let details = self.folder_pane().details();
        let chosen = toggled(details.chosen_columns(), column);
        details.show_chosen_columns(chosen.clone());
        self.remember_style();
    }

    /// Fits `column` of the active pane's details view, or every shown
    /// column for `None`, which saves the widths as a drag does. Only
    /// the details view has columns: in the other views, and on a page
    /// without a listing, Ctrl+plus on the number pad does nothing, as in
    /// Windows File Explorer (VIEW-072).
    fn fit_details_columns(&self, column: Option<SortColumn>) {
        let pane = self.folder_pane();
        if pane.view() != FolderView::Details || pane.page() != Some(PanePage::Listing) {
            return;
        }
        let details = pane.details();
        match column {
            Some(column) => details.fit_column(column),
            None => details.fit_all_columns(),
        }
    }

    /// Applies the shared view options to the folder shown: which items
    /// show previews, whether folders count their items, and the details
    /// columns.
    pub(super) fn apply_view_options(&self) {
        for (side, uri) in self.shown_panes() {
            let options = self.context().settings_data().preferences.view_options;
            let pane = self.pane_on(side);
            let is_remote = self.is_remote_folder(&uri);
            pane.owners()
                .set_previews(PreviewPolicy::for_folder(&options, is_remote));
            pane.details()
                .set_counts_items(options.count_folder_items && !is_remote);
        }
    }

    /// Whether `uri` is reached over a network or a device link: a
    /// server, a share mounted by the kernel, a phone or camera.
    pub(super) fn is_remote_folder(&self, uri: &str) -> bool {
        match location_kind(uri) {
            LocationKind::Local => self.imp().locations.borrow().is_network_location(uri),
            LocationKind::Smb | LocationKind::Remote | LocationKind::Device => true,
            LocationKind::Other => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use ox_core::settings::ViewOptions;

    use super::*;
    use crate::folder_view::details::chosen_from_keys;
    use crate::window::menu_popover::ItemAvailability;

    /// The labels and marks of `entries`, a divider as `None`.
    fn shown(entries: &[MenuEntry]) -> Vec<Option<(String, ItemCheck, bool)>> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => Some((
                    item.label.clone(),
                    item.check.clone(),
                    item.availability == ItemAvailability::Disabled,
                )),
                MenuEntry::Divider => None,
            })
            .collect()
    }

    /// The titles' menu is Windows 11 File Explorer's: "Size Column to
    /// Fit", "Size All Columns to Fit", a divider, the columns as check
    /// items (Name first, checked and greyed, as it cannot be hidden;
    /// Windows' Date modified, Type, Size and Date created in its order,
    /// then the app's own File extension, Owner and Permissions), a
    /// divider and "More...". No row shows an icon. Choosing a column
    /// adds it at the end or hides it.
    ///
    /// parity: VIEW-033, VIEW-072
    #[test]
    fn the_titles_menu_is_windows_11s_with_check_marks_only() {
        let chosen = chosen_from_keys(&ViewOptions::default().details_columns);
        let entries = column_menu_entries(&chosen, Some(SortColumn::Size));
        let item = |label: &str, checked: bool, disabled: bool| {
            Some((label.to_owned(), ItemCheck::Fixed(checked), disabled))
        };
        let command = |label: &str| Some((label.to_owned(), ItemCheck::Plain, false));
        assert_eq!(
            shown(&entries),
            [
                command("Size Column to Fit"),
                command("Size All Columns to Fit"),
                None,
                item("Name", true, true),
                item("Date modified", true, false),
                item("Type", true, false),
                item("Size", true, false),
                item("Date created", false, false),
                item("File extension", false, false),
                item("Owner", false, false),
                item("Permissions", false, false),
                None,
                command("More..."),
            ]
        );
        let items = entries.iter().filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Divider => None,
        });
        for item in items {
            assert!(!item.shows_glyph, "{} shows no icon", item.label);
            assert!(item.application_icon.is_none(), "{} shows no icon", item.label);
        }

        let with_owner = toggled(chosen.clone(), SortColumn::Owner);
        assert_eq!(with_owner.last(), Some(&SortColumn::Owner));
        assert!(!toggled(chosen.clone(), SortColumn::Type).contains(&SortColumn::Type));
        assert_eq!(
            toggled(chosen.clone(), SortColumn::Name),
            chosen,
            "Name cannot be hidden"
        );
    }
}
