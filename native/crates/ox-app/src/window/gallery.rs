// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery in the window (HOME-017 to HOME-024): the navigation pane row
//! under Home opens `ox:gallery`, which the tab lists like a folder from
//! the photos of the Pictures folder and its subfolders
//! ([`crate::folder_view::gallery`]).
//!
//! Gallery is always shown as thumbnails in justified rows, newest first
//! and without groups, whatever style the folders keep: the most recently
//! taken photos come first, as Microsoft describes Windows 11's Gallery
//! (<https://support.microsoft.com/en-us/topic/september-26-2023-windows-configuration-update-542780c2-594c-46cb-979d-11116fe164ba>),
//! at Medium icons, as Windows 11 opens it. The size of the thumbnails can
//! be changed while it is shown (View, Ctrl+mouse wheel); the style is
//! never saved, so neither Gallery nor the folders' shared style take the
//! other's.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::grouping::GroupBy;
use ox_core::location::GALLERY_URI;
use ox_core::places::KnownFolder;
use ox_core::settings::SettingsError;

use super::actions::{choice_action, plain_action};
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

/// Whether `pane` shows Gallery's style now.
pub(super) fn shows_gallery_style(pane: &FolderPane) -> bool {
    pane.shows_gallery()
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
            plain_action(WindowAction::GalleryManage, BrowserWindow::ask_gallery_locations),
            plain_action(WindowAction::RotateLeft, |window| window.rotate_selection(false)),
            plain_action(WindowAction::RotateRight, |window| window.rotate_selection(true)),
            // Not done: the app has no way of its own to set the desktop's
            // background, so the command shows, as in Windows, but grayed.
            plain_action(WindowAction::SetAsBackground, |_| {}),
        ]);
        self.set_action_enabled(WindowAction::SetAsBackground, false);
    }

    /// The selected photos Rotate can turn: JPEGs and PNGs on this
    /// computer; empty unless every selected item is one.
    fn rotatable_selection(&self) -> Vec<std::path::PathBuf> {
        let items = self.folder_pane().model().selected_items();
        let paths: Vec<std::path::PathBuf> = items
            .iter()
            .filter(|item| {
                let entry = item.entry();
                !entry.is_dir
                    && entry
                        .content_type
                        .as_deref()
                        .and_then(crate::photo_rotate::Rotatable::of_type)
                        .is_some()
            })
            .filter_map(|item| gio::File::for_uri(&item.entry().uri).path())
            .collect();
        if paths.len() == items.len() {
            paths
        } else {
            Vec::new()
        }
    }

    /// Rotate right (`clockwise`) or Rotate left: turns every selected
    /// photo a quarter, off the main thread (HOME-032).
    fn rotate_selection(&self, clockwise: bool) {
        let paths = self.rotatable_selection();
        if paths.is_empty() {
            return;
        }
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let failed = gio::spawn_blocking(move || {
                paths
                    .iter()
                    .filter_map(|path| {
                        crate::photo_rotate::rotate_file(path, clockwise)
                            .err()
                            .map(|error| (path.clone(), error.to_string()))
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let Some(window) = window.upgrade() else {
                return;
            };
            if let Some((path, error)) = failed.first() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                window.show_message(&ox_core::i18n::format_message(
                    "Could not rotate {name}: {error}",
                    &[("name", &name), ("error", error)],
                ));
            }
            // The watch lists Gallery again; a folder lists on its own.
            window.reload_gallery_tabs();
        });
    }

    /// Shows the command bar as Gallery has it while the window shows
    /// Gallery: its View menu, Sort grayed, Collection, and the photo
    /// commands while photos are selected (HOME-030, HOME-031).
    pub(super) fn show_gallery_commands(&self) {
        let in_gallery = self.shows_gallery();
        let collection = in_gallery.then(|| {
            let key = format!("{:?}", self.gallery_collection());
            (key, self.gallery_collection_entries())
        });
        let bar = self.command_bar();
        bar.show_collection(collection);
        bar.show_gallery_menus(in_gallery);
        let rotatable = !self.rotatable_selection().is_empty();
        let photos = in_gallery && self.folder_pane().model().summary().count > 0;
        bar.show_photo_commands(photos);
        self.set_action_enabled(WindowAction::RotateLeft, rotatable);
        self.set_action_enabled(WindowAction::RotateRight, rotatable);
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

    /// The entries of the Collection menu, as Windows 11 has them: Show
    /// all, chosen by default, then one choice per location, and Manage
    /// collection (HOME-025).
    pub(super) fn gallery_collection_entries(&self) -> Vec<MenuEntry> {
        let mut entries: Vec<MenuEntry> = vec![MenuItem::choice(
            &ox_core::i18n::gettext("Show all"),
            Icon::Image,
            WindowAction::GalleryCollection,
            ALL_FOLDERS,
        )
        .with_dot_mark()
        .into()];
        for (name, uri) in &self.gallery_collection() {
            entries.push(
                MenuItem::choice(name, Icon::Folder, WindowAction::GalleryCollection, uri)
                    .with_dot_mark()
                    .into(),
            );
        }
        entries.push(MenuEntry::Divider);
        entries.push(
            MenuItem::new(
                &ox_core::i18n::gettext("Manage collection"),
                Icon::Settings,
                WindowAction::GalleryManage,
            )
            .into(),
        );
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
    /// is shown: Medium icons in justified rows, newest first, without
    /// groups (HOME-019, HOME-026).
    pub(super) fn show_gallery_style(&self, pane: &FolderPane, store: &gio::ListStore) {
        let applying = self.imp().applying_style.replace(true);
        pane.set_styled_for(store, GALLERY_URI);
        let coming = !pane.shows_gallery();
        pane.set_gallery(true);
        if coming || !matches!(pane.view(), FolderView::Icons(_)) {
            pane.show_view(FolderView::Icons(IconSize::MEDIUM));
        }
        pane.model().set_folders_first(false);
        pane.model()
            .set_sort_role(Some((SortRole::Taken, SortDirection::Descending)));
        group_pane(pane, Grouping::of(GroupBy::None, GALLERY_SORT));
        self.update_expandability_for(pane);
        self.imp().applying_style.set(applying);
        if pane == self.folder_pane() {
            self.set_action_state(WindowAction::Sort, &GALLERY_SORT.by.as_str().to_variant());
            self.set_action_state(
                WindowAction::Direction,
                &GALLERY_SORT.direction.as_str().to_variant(),
            );
            self.set_action_state(WindowAction::GroupBy, &GroupBy::None.as_str().to_variant());
            self.show_pane_view_state();
            self.update_content();
        }
    }
}

/// A photo's tile on screen, in the view's content coordinates, for tests.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct ShownTile {
    pub(crate) name: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    /// The photo's thumbnail is drawn.
    pub(crate) thumbnail: bool,
}

#[cfg(test)]
impl BrowserWindow {
    /// The photos' tiles on screen.
    pub(crate) fn gallery_tiles(&self) -> Vec<ShownTile> {
        let pane = self.folder_pane();
        if !pane.shows_gallery() {
            return Vec::new();
        }
        let mut tiles: Vec<ShownTile> = pane
            .gallery()
            .shown_tiles()
            .into_iter()
            .map(|(name, bounds, thumbnail)| ShownTile {
                name,
                thumbnail,
                x: f64::from(bounds.x()),
                y: f64::from(bounds.y()),
                width: f64::from(bounds.width()),
                height: f64::from(bounds.height()),
            })
            .collect();
        tiles.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        tiles
    }

    /// The width the tiles' rows fill.
    pub(crate) fn gallery_content_width(&self) -> f64 {
        f64::from(self.folder_pane().gallery().canvas().width())
    }

    /// The heading over the photos.
    pub(crate) fn gallery_heading(&self) -> Option<String> {
        let pane = self.folder_pane();
        pane.shows_gallery()
            .then(|| pane.gallery().heading_text())
            .flatten()
    }

    /// Scrolls Gallery to `y`, clamped.
    pub(crate) fn gallery_scroll_to(&self, y: f64) {
        let adjustment = self.folder_pane().gallery().vadjustment();
        adjustment.set_value(y.min(adjustment.upper() - adjustment.page_size()));
    }

    /// Gallery's date scrubber.
    pub(crate) fn gallery_scrubber(&self) -> Option<crate::folder_view::gallery_scrubber::GalleryScrubber> {
        let pane = self.folder_pane();
        pane.shows_gallery().then(|| pane.gallery().scrubber().clone())
    }

    /// The tooltip of the photo at `position`.
    pub(crate) fn gallery_tooltip(&self, position: u32) -> Option<String> {
        self.folder_pane().gallery().canvas().tooltip_for(position)
    }
}
