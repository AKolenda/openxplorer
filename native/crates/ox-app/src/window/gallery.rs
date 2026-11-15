// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery in the window (HOME-017 to HOME-024): the navigation pane row
//! under Home opens `ox:gallery`, which the tab lists like a folder from
//! the photos of the Pictures folder and its subfolders
//! ([`crate::folder_view::gallery`]).
//!
//! Gallery is always shown as thumbnails, newest first in month groups,
//! whatever style the folders keep: the most recently taken photos come
//! first, as Microsoft describes Windows 11's Gallery
//! (<https://support.microsoft.com/en-us/topic/september-26-2023-windows-configuration-update-542780c2-594c-46cb-979d-11116fe164ba>).
//! The size of the thumbnails can be changed while it is shown (Ctrl+mouse
//! wheel, as Windows has it since build 23440); the style is never saved,
//! so neither Gallery nor the folders' shared style take the other's.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::grouping::GroupBy;
use ox_core::location::GALLERY_URI;
use ox_core::places::KnownFolder;
use ox_core::settings::SettingsError;

use super::actions::{choice_action, plain_action, text_action};
use super::folder_pane::{FolderPane, FolderView};
use super::menu_popover::{MenuEntry, MenuItem};
use super::session::TabId;
use super::view_style::group_pane;
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::gallery::GalleryWatch;
use crate::folder_view::groups::Grouping;
use crate::folder_view::icon_size::IconSize;
use crate::folder_view::sort_roles::{SortBy, SortRole, SortState};
use crate::folder_view::sorting::SortDirection;
use crate::icons::Icon;
use crate::settings_store::Change;

/// The Collection choice that shows every folder of the collection.
const ALL_FOLDERS: &str = "all";

/// Gallery's sort: the date taken, newest first.
const GALLERY_SORT: SortState = SortState {
    by: SortBy::Role(SortRole::Taken),
    direction: SortDirection::Descending,
};

/// Whether `pane` shows Gallery's style now: only Gallery sorts by the
/// date taken.
pub(super) fn shows_gallery_style(pane: &FolderPane) -> bool {
    pane.model().sort_role().map(|(role, _)| role) == Some(SortRole::Taken)
}

impl BrowserWindow {
    /// Every folder of Gallery's collection, with its name: the Pictures
    /// folder, then the folders the user added (HOME-025).
    pub(super) fn gallery_collection(&self) -> Vec<(String, String)> {
        let mut folders: Vec<(String, String)> = self
            .context()
            .known_folders()
            .into_iter()
            .filter(|place| place.known_folder == Some(KnownFolder::Pictures))
            .map(|place| (place.label, place.uri))
            .collect();
        for uri in self.context().settings_data().preferences.gallery_folders {
            if folders.iter().any(|(_, kept)| kept == &uri) {
                continue;
            }
            let name = gio::File::for_uri(&uri)
                .basename()
                .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
            folders.push((name, uri));
        }
        folders
    }

    /// The folders Gallery lists: the one chosen under Collection, or
    /// every folder of the collection.
    pub(super) fn gallery_folders(&self) -> Vec<String> {
        let folders: Vec<String> = self
            .gallery_collection()
            .into_iter()
            .map(|(_, uri)| uri)
            .collect();
        let chosen = self
            .lookup_action(WindowAction::GalleryCollection.name())
            .and_then(|action| action.state())
            .and_then(|state| state.get::<String>());
        match chosen {
            Some(chosen) if folders.contains(&chosen) => vec![chosen],
            _ => folders,
        }
    }

    /// The actions of Gallery's Collection menu.
    pub(super) fn install_gallery_actions(&self) {
        self.add_action_entries([
            choice_action(WindowAction::GalleryCollection, ALL_FOLDERS, |window, value| {
                let known =
                    value == ALL_FOLDERS || window.gallery_collection().iter().any(|(_, uri)| uri == value);
                if known {
                    // The choice is read when Gallery is listed again.
                    glib::idle_add_local_once(glib::clone!(
                        #[weak]
                        window,
                        move || window.reload_gallery_tabs()
                    ));
                }
                known
            }),
            plain_action(WindowAction::GalleryAddFolder, BrowserWindow::ask_gallery_folder),
            text_action(WindowAction::GalleryRemoveFolder, |window, uri| {
                window.set_gallery_folder(uri, false);
            }),
        ]);
    }

    /// Collection > Add folder…: asks for a folder and adds it.
    fn ask_gallery_folder(&self) {
        let picker = gtk::FileDialog::builder()
            .title(ox_core::i18n::gettext("Add a folder to Gallery"))
            .modal(true)
            .build();
        picker.select_folder(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |chosen| {
                    if let Ok(folder) = chosen {
                        window.set_gallery_folder(&folder.uri(), true);
                    }
                }
            ),
        );
    }

    /// Adds the folder at `uri` to Gallery's collection, or removes it,
    /// then lists Gallery again (HOME-025). Only a local folder is added.
    pub(super) fn set_gallery_folder(&self, uri: &str, added: bool) {
        let uri = uri.to_owned();
        let change: Change = Box::new(move |settings| settings.set_gallery_folder(&uri, added).map(|_| ()));
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    if let Err(error) = result {
                        window.show_message(&ox_core::i18n::format_message(
                            "Could not save Gallery's folders: {error}",
                            &[("error", &error.to_string())],
                        ));
                    }
                }
            ),
        );
    }

    /// The entries of the Collection menu: every folder, or one of them,
    /// to show; Add folder…; and the added folders to remove.
    pub(super) fn gallery_collection_entries(&self) -> Vec<MenuEntry> {
        let collection = self.gallery_collection();
        let added = self.context().settings_data().preferences.gallery_folders;
        let mut entries: Vec<MenuEntry> = vec![MenuItem::choice(
            &ox_core::i18n::gettext("All folders"),
            Icon::Image,
            WindowAction::GalleryCollection,
            ALL_FOLDERS,
        )
        .into()];
        for (name, uri) in &collection {
            entries.push(MenuItem::choice(name, Icon::Folder, WindowAction::GalleryCollection, uri).into());
        }
        entries.push(MenuEntry::Divider);
        entries.push(
            MenuItem::new(
                &ox_core::i18n::gettext("Add folder…"),
                Icon::Add,
                WindowAction::GalleryAddFolder,
            )
            .into(),
        );
        let removable: Vec<MenuEntry> = collection
            .iter()
            .filter(|(_, uri)| added.contains(uri))
            .map(|(name, uri)| {
                MenuItem::with_text_target(name, Icon::Folder, WindowAction::GalleryRemoveFolder, uri).into()
            })
            .collect();
        if !removable.is_empty() {
            entries.push(
                MenuItem::submenu(
                    &ox_core::i18n::gettext("Remove folder"),
                    Icon::Delete,
                    WindowAction::GalleryRemoveFolder,
                    removable,
                )
                .into(),
            );
        }
        entries
    }

    /// Lists Gallery again, and shows its Collection menu anew, when the
    /// folders of its collection changed since last seen: the Pictures
    /// folder moved, or a folder was added or removed, here or in another
    /// window.
    pub(super) fn follow_gallery_collection(&self) {
        let collection = self.gallery_collection();
        if *self.imp().gallery_collection.borrow() == collection {
            return;
        }
        let first = self.imp().gallery_collection.replace(collection).is_empty();
        if !first {
            self.reload_gallery_tabs();
            self.update_file_commands();
        }
    }

    /// Opens Gallery's Collection menu, as a click on it does, while it
    /// shows; for the snapshot hook's scenes.
    pub(crate) fn open_gallery_collection(&self) -> bool {
        self.command_bar()
            .collection_button()
            .filter(WidgetExt::is_visible)
            .inspect(gtk::MenuButton::popup)
            .is_some()
    }

    /// Whether the active tab shows Gallery.
    pub(super) fn shows_gallery(&self) -> bool {
        self.current_uri().as_deref() == Some(GALLERY_URI)
    }

    /// Watches Gallery's folders for tab `id`, which lists Gallery now:
    /// the tab keeps one watch while it stays on Gallery, and every listing
    /// adds the folders that appeared since (HOME-021).
    pub(super) fn watch_gallery(&self, id: TabId) {
        let folders = self.gallery_folders();
        let mut session = self.imp().session.borrow_mut();
        let Some(tab) = session.tab_mut(id) else {
            return;
        };
        tab.watch = None;
        let watch = tab.gallery_watch.get_or_insert_with(|| {
            GalleryWatch::start(gtk::glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.folder_changed(id)
            ))
        });
        watch.watch_folders(folders);
    }

    /// Lists every tab on Gallery again, as Show hidden items changes
    /// which folders it walks.
    pub(super) fn reload_gallery_tabs(&self) {
        let ids: Vec<TabId> = self
            .imp()
            .session
            .borrow()
            .tabs()
            .iter()
            .filter(|tab| tab.uri() == GALLERY_URI)
            .map(|tab| tab.id)
            .collect();
        for id in ids {
            self.folder_changed(id);
        }
    }

    /// Shows `pane`, which shows the tab of `store` on Gallery, as Gallery
    /// is shown: Large icons, newest first in month groups.
    pub(super) fn show_gallery_style(&self, pane: &FolderPane, store: &gio::ListStore) {
        let applying = self.imp().applying_style.replace(true);
        pane.set_styled_for(store, GALLERY_URI);
        pane.icon_view().set_icon_groups(true);
        if !shows_gallery_style(pane) {
            pane.icon_view().set_icon_size(IconSize::LARGE);
            pane.show_view(FolderView::Icons(IconSize::LARGE));
        }
        pane.model().set_folders_first(false);
        pane.model()
            .set_sort_role(Some((SortRole::Taken, SortDirection::Descending)));
        group_pane(pane, Grouping::of(GroupBy::Month, GALLERY_SORT));
        self.update_expandability_for(pane);
        self.imp().applying_style.set(applying);
        if pane == self.folder_pane() {
            self.set_action_state(WindowAction::Sort, &GALLERY_SORT.by.as_str().to_variant());
            self.set_action_state(
                WindowAction::Direction,
                &GALLERY_SORT.direction.as_str().to_variant(),
            );
            self.set_action_state(WindowAction::GroupBy, &GroupBy::Month.as_str().to_variant());
            self.show_pane_view_state();
            self.update_content();
        }
    }
}
