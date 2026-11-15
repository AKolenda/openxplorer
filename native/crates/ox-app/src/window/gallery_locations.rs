// SPDX-License-Identifier: AGPL-3.0-only
//! The "Gallery Locations" dialog of Collection > Manage collection
//! (HOME-025), laid out as Windows 11 File Explorer's: the blue heading
//! "Choose which folders to see in Gallery", what the list is for, the
//! "Gallery locations" list with each folder's name over its path (the
//! Pictures folder marked "Default save location" at the right), Add… and
//! Remove under it, and OK and Cancel. The Pictures folder cannot be
//! removed. Add… asks for a local folder; nothing changes until OK, which
//! saves the folders added as `galleryFolders` in settings.json; Cancel
//! and Escape drop the changes.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::places::KnownFolder;

use super::BrowserWindow;
use crate::dialog::{DialogFrame, DialogWidth};
use crate::icons::{self, Icon};
use crate::window::ButtonStyle;

/// The dialog's title.
const TITLE: &str = crate::i18n::message_id("Gallery Locations");
/// The blue heading.
const HEADING: &str = crate::i18n::message_id("Choose which folders to see in Gallery");
/// What the list is for, as Windows words it.
const BODY: &str = crate::i18n::message_id(
    "Manage your Gallery by adding or removing folders from locations across your PC. Folders added here will still be stored in their original locations.",
);
/// The list's label.
const LIST_LABEL: &str = crate::i18n::message_id("Gallery locations");
/// What the Pictures folder's row says at its right.
const DEFAULT_SAVE: &str = crate::i18n::message_id("Default save location");

/// One folder of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Location {
    name: String,
    uri: String,
    /// The Pictures folder: the default save location, never removed.
    is_default: bool,
}

impl Location {
    /// The folder's path as the list shows it.
    fn path(&self) -> String {
        gio::File::for_uri(&self.uri)
            .path()
            .map_or_else(|| self.uri.clone(), |path| path.display().to_string())
    }
}

/// The open dialog: its frame and the list as it stands.
#[derive(Debug, Clone)]
pub(crate) struct GalleryLocations {
    frame: DialogFrame,
    list: gtk::ListBox,
    remove: gtk::Button,
    locations: Rc<RefCell<Vec<Location>>>,
}

/// [`GalleryLocations`] without keeping it alive, for its own widgets'
/// handlers.
#[derive(Debug, Clone)]
struct WeakLocations {
    frame: glib::WeakRef<DialogFrame>,
    list: glib::WeakRef<gtk::ListBox>,
    remove: glib::WeakRef<gtk::Button>,
    locations: std::rc::Weak<RefCell<Vec<Location>>>,
}

impl WeakLocations {
    fn upgrade(&self) -> Option<GalleryLocations> {
        Some(GalleryLocations {
            frame: self.frame.upgrade()?,
            list: self.list.upgrade()?,
            remove: self.remove.upgrade()?,
            locations: self.locations.upgrade()?,
        })
    }
}

impl BrowserWindow {
    /// Collection > Manage collection: shows the Gallery Locations dialog.
    pub(super) fn ask_gallery_locations(&self) {
        let pictures: Vec<Location> = self
            .context()
            .known_folders()
            .into_iter()
            .filter(|place| place.known_folder == Some(KnownFolder::Pictures))
            .map(|place| Location {
                name: place.label,
                uri: place.uri,
                is_default: true,
            })
            .collect();
        let added = self.context().settings_data().preferences.gallery_folders;
        let mut locations = pictures;
        for uri in added {
            if !locations.iter().any(|location| location.uri == uri) {
                locations.push(Location {
                    name: folder_name(&uri),
                    uri,
                    is_default: false,
                });
            }
        }
        let dialog = GalleryLocations::new(locations);
        let frame = dialog.frame.clone();
        let add = dialog.add_button();
        let weak = dialog.downgrade();
        add.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            weak,
            move |_| {
                if let Some(dialog) = weak.upgrade() {
                    window.pick_gallery_location(&dialog);
                }
            }
        ));
        let ok = frame.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent);
        frame.add_closing_button(&ox_core::i18n::gettext("Cancel"), ButtonStyle::Bordered, || {});
        ok.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            weak,
            move |_| {
                if let Some(dialog) = weak.upgrade() {
                    window.keep_gallery_locations(&dialog.added_uris());
                    dialog.frame.close();
                }
            }
        ));
        self.imp().gallery_locations.replace(Some(dialog.clone()));
        frame.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.imp().gallery_locations.replace(None);
            }
        ));
        self.present_window_dialog(&frame);
        dialog.list.grab_focus();
    }

    /// Add…: asks for a folder and puts it in the list.
    fn pick_gallery_location(&self, dialog: &GalleryLocations) {
        let picker = gtk::FileDialog::builder()
            .title(ox_core::i18n::gettext("Add a folder to Gallery"))
            .modal(true)
            .build();
        picker.select_folder(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong]
                dialog,
                move |chosen| {
                    if let Ok(folder) = chosen {
                        dialog.add_folder(&folder.uri());
                    }
                }
            ),
        );
    }

    /// Saves `added` as the folders the user added to Gallery: the ones
    /// no longer listed are removed and the new ones added.
    fn keep_gallery_locations(&self, added: &[String]) {
        let saved = self.context().settings_data().preferences.gallery_folders;
        for uri in saved.iter().filter(|uri| !added.contains(uri)) {
            self.set_gallery_folder(uri, false);
        }
        for uri in added.iter().filter(|uri| !saved.contains(uri)) {
            self.set_gallery_folder(uri, true);
        }
    }

    /// The Gallery Locations dialog while it is open.
    #[cfg(test)]
    pub(crate) fn gallery_locations(&self) -> Option<GalleryLocations> {
        self.imp().gallery_locations.borrow().clone()
    }
}

/// The name a folder shows in the list: its own name.
fn folder_name(uri: &str) -> String {
    gio::File::for_uri(uri)
        .basename()
        .map_or_else(|| uri.to_owned(), |name| name.to_string_lossy().into_owned())
}

impl GalleryLocations {
    fn new(locations: Vec<Location>) -> Self {
        let frame = DialogFrame::new(ox_core::i18n::gettext_static(TITLE), DialogWidth::Standard);
        frame.add_css_class("gallery-locations");
        let body = frame.body();
        let heading = gtk::Label::builder()
            .label(ox_core::i18n::gettext_static(HEADING))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["gallery-locations-heading"])
            .build();
        heading.set_accessible_role(gtk::AccessibleRole::Heading);
        body.append(&heading);
        let text = gtk::Label::builder()
            .label(ox_core::i18n::gettext_static(BODY))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["gallery-locations-text"])
            .build();
        body.append(&text);
        let list_label = gtk::Label::builder()
            .label(ox_core::i18n::gettext_static(LIST_LABEL))
            .xalign(0.0)
            .css_classes(["gallery-locations-label"])
            .build();
        body.append(&list_label);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["gallery-locations-list", "frame"])
            .build();
        list.update_relation(&[gtk::accessible::Relation::LabelledBy(&[list_label.upcast_ref()])]);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(140)
            .max_content_height(220)
            .propagate_natural_height(true)
            .build();
        body.append(&scroller);
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["gallery-locations-buttons"])
            .build();
        let add = gtk::Button::with_mnemonic(&ox_core::i18n::gettext("_Add…"));
        add.add_css_class(ButtonStyle::Bordered.css_class());
        let remove = gtk::Button::with_mnemonic(&ox_core::i18n::gettext("_Remove"));
        remove.add_css_class(ButtonStyle::Bordered.css_class());
        remove.set_sensitive(false);
        buttons.append(&add);
        buttons.append(&remove);
        body.append(&buttons);
        let dialog = Self {
            frame,
            list,
            remove,
            locations: Rc::new(RefCell::new(locations)),
        };
        dialog.show_rows();
        let weak = dialog.downgrade();
        dialog.list.connect_row_selected(glib::clone!(
            #[strong]
            weak,
            move |_, _| {
                if let Some(dialog) = weak.upgrade() {
                    dialog.follow_selection();
                }
            }
        ));
        dialog.remove.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.remove_selected();
            }
        });
        dialog
    }

    fn downgrade(&self) -> WeakLocations {
        WeakLocations {
            frame: self.frame.downgrade(),
            list: self.list.downgrade(),
            remove: self.remove.downgrade(),
            locations: Rc::downgrade(&self.locations),
        }
    }

    /// Add…, the first button under the list.
    fn add_button(&self) -> gtk::Button {
        self.remove
            .prev_sibling()
            .and_downcast::<gtk::Button>()
            .expect("Add… comes before Remove")
    }

    /// Lists the folders again.
    fn show_rows(&self) {
        self.list.remove_all();
        for location in self.locations.borrow().iter() {
            self.list.append(&location_row(location));
        }
        self.follow_selection();
    }

    /// Remove can remove the selected folder unless it is Pictures.
    fn follow_selection(&self) {
        let removable = self
            .list
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok())
            .and_then(|index| {
                self.locations
                    .borrow()
                    .get(index)
                    .map(|location| !location.is_default)
            })
            .unwrap_or(false);
        self.remove.set_sensitive(removable);
    }

    /// Puts the local folder at `uri` in the list, once.
    pub(crate) fn add_folder(&self, uri: &str) {
        if gio::File::for_uri(uri).path().is_none() {
            self.frame.show_error(&ox_core::i18n::gettext(
                "Only folders on this computer can be added.",
            ));
            return;
        }
        if self.locations.borrow().iter().any(|location| location.uri == uri) {
            return;
        }
        self.locations.borrow_mut().push(Location {
            name: folder_name(uri),
            uri: uri.to_owned(),
            is_default: false,
        });
        self.frame.show_error("");
        self.show_rows();
        let last = i32::try_from(self.locations.borrow().len()).unwrap_or(1) - 1;
        if let Some(row) = self.list.row_at_index(last) {
            self.list.select_row(Some(&row));
        }
    }

    /// Takes the selected folder out of the list, unless it is Pictures.
    pub(crate) fn remove_selected(&self) {
        let Some(index) = self
            .list
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok())
        else {
            return;
        };
        let removed = {
            let mut locations = self.locations.borrow_mut();
            match locations.get(index) {
                Some(location) if !location.is_default => {
                    locations.remove(index);
                    true
                }
                _ => false,
            }
        };
        if removed {
            self.show_rows();
        }
    }

    /// The folders the user added, as listed now.
    fn added_uris(&self) -> Vec<String> {
        self.locations
            .borrow()
            .iter()
            .filter(|location| !location.is_default)
            .map(|location| location.uri.clone())
            .collect()
    }
}

/// A folder's row: its glyph, its name over its path, and "Default save
/// location" at the right for Pictures.
fn location_row(location: &Location) -> gtk::ListBoxRow {
    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["gallery-location"])
        .build();
    line.append(&icons::image(Icon::Folder, 20));
    let names = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let name = gtk::Label::builder()
        .label(&location.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["gallery-location-name"])
        .build();
    let path = gtk::Label::builder()
        .label(location.path())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["gallery-location-path", "dim-label"])
        .build();
    names.append(&name);
    names.append(&path);
    line.append(&names);
    let note = gtk::Label::builder()
        .label(if location.is_default {
            ox_core::i18n::gettext_static(DEFAULT_SAVE)
        } else {
            ""
        })
        .css_classes(["gallery-location-note", "dim-label"])
        .build();
    line.append(&note);
    let row = gtk::ListBoxRow::builder().child(&line).build();
    let label = if location.is_default {
        format!(
            "{}, {}, {}",
            location.name,
            location.path(),
            ox_core::i18n::gettext_static(DEFAULT_SAVE)
        )
    } else {
        format!("{}, {}", location.name, location.path())
    };
    row.update_property(&[gtk::accessible::Property::Label(&label)]);
    row
}

#[cfg(test)]
impl GalleryLocations {
    pub(crate) fn title(&self) -> String {
        self.frame.title().to_string()
    }
    pub(crate) fn is_visible(&self) -> bool {
        self.frame.is_mapped()
    }
    pub(crate) fn heading_text(&self) -> String {
        self.label_with_class("gallery-locations-heading")
    }
    pub(crate) fn body_text(&self) -> String {
        self.label_with_class("gallery-locations-text")
    }
    pub(crate) fn list_label(&self) -> String {
        self.label_with_class("gallery-locations-label")
    }
    fn label_with_class(&self, class: &str) -> String {
        crate::window::widget_tree::descendants::<gtk::Label>(&self.frame)
            .into_iter()
            .find(|label| label.has_css_class(class))
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }
    pub(crate) fn rows(&self) -> Vec<(String, String, String)> {
        crate::window::widget_tree::children(&self.list)
            .map(|row| {
                let text = |class: &str| {
                    crate::window::widget_tree::descendants::<gtk::Label>(&row)
                        .into_iter()
                        .find(|label| label.has_css_class(class))
                        .map(|label| label.text().to_string())
                        .unwrap_or_default()
                };
                (
                    text("gallery-location-name"),
                    text("gallery-location-path"),
                    text("gallery-location-note"),
                )
            })
            .collect()
    }
    pub(crate) fn select_row(&self, index: usize) {
        let row = i32::try_from(index)
            .ok()
            .and_then(|index| self.list.row_at_index(index));
        self.list.select_row(row.as_ref());
    }
    pub(crate) fn can_remove(&self) -> bool {
        self.remove.is_sensitive()
    }
    pub(crate) fn ok(&self) {
        let ok = crate::window::widget_tree::descendants::<gtk::Button>(&self.frame)
            .into_iter()
            .find(|button| button.label().as_deref() == Some("OK"))
            .expect("the dialog has OK");
        ok.emit_clicked();
    }
    pub(crate) fn cancel(&self) {
        let cancel = crate::window::widget_tree::descendants::<gtk::Button>(&self.frame)
            .into_iter()
            .find(|button| button.label().as_deref() == Some("Cancel"))
            .expect("the dialog has Cancel");
        cancel.emit_clicked();
    }
}
