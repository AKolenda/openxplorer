// SPDX-License-Identifier: AGPL-3.0-only
//! "Folder pictures" (PROP-037): "Choose a file to show on this folder
//! icon.", with "Choose File..." and "Restore Default" and a preview of
//! the folder with its picture beside them, as Windows 11's Customize tab
//! has them. The picture is put on the folder on OK or Apply, as `GVfs`
//! metadata ([`FOLDER_PICTURE`]); nothing is written into the folder.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::super::window::{group, paragraph, push_button, ApplyFuture, Pending, PropertiesWindow};
use super::{choose_picture, set_metadata};
use crate::folder_view::cells::folder_picture_of;
use crate::folder_view::{icon_texture, FolderPicture, FOLDER_PICTURE};
use crate::icons::{Art, ArtImage};

/// The size of the preview: the folder as Extra large icons draw it.
const PREVIEW_SIZE: i32 = 96;

/// The picture chosen, until OK or Apply.
struct PictureChoice {
    window: glib::WeakRef<PropertiesWindow>,
    /// The folder's location, which a rename on General changes.
    uri: Rc<RefCell<String>>,
    /// The folder's picture now, if it has one.
    current: RefCell<Option<String>>,
    /// The picture the user chose, or `None` for none.
    chosen: RefCell<Option<String>>,
    preview: FolderPicture,
    restore: gtk::Button,
}

impl PictureChoice {
    /// Shows `picture` on the preview; Restore Default is grayed out without
    /// one.
    fn show(self: &Rc<Self>, picture: Option<&str>) {
        self.restore.set_sensitive(picture.is_some());
        self.preview.show_texture(None);
        let Some(picture) = picture.map(str::to_owned) else {
            return;
        };
        let choice = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let texture = icon_texture(picture.clone()).await;
            let Some(choice) = choice.upgrade() else {
                return;
            };
            // Only the picture still chosen is shown.
            if choice.chosen.borrow().as_deref() == Some(picture.as_str()) {
                choice.preview.show_texture(texture.as_ref());
            }
        });
    }

    fn choose(self: &Rc<Self>, picture: Option<String>) {
        self.show(picture.as_deref());
        self.chosen.replace(picture);
        if let Some(window) = self.window.upgrade() {
            window.changed();
        }
    }

    /// Takes the folder's picture as read, unless one was chosen already.
    fn found(self: &Rc<Self>, picture: Option<String>) {
        if *self.chosen.borrow() != *self.current.borrow() {
            self.current.replace(picture);
            return;
        }
        self.current.replace(picture.clone());
        self.chosen.replace(picture.clone());
        self.show(picture.as_deref());
    }
}

impl Pending for PictureChoice {
    fn has_changes(&self) -> bool {
        *self.chosen.borrow() != *self.current.borrow()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            let picture = self.chosen.borrow().clone();
            let uri = self.uri.borrow().clone();
            set_metadata(&uri, FOLDER_PICTURE, picture.clone())
                .await
                .map_err(|error| error.to_string())?;
            self.current.replace(picture);
            if let Some(opener) = self.window.upgrade().and_then(|window| window.opener()) {
                opener.announce_item_icon(&uri);
            }
            Ok(())
        })
    }
}

/// The "Folder pictures" group of the local folder at `uri`.
pub(super) fn folder_picture_group(window: &PropertiesWindow, uri: &Rc<RefCell<String>>) -> gtk::Frame {
    let (frame, content) = group(&ox_core::i18n::gettext("Folder pictures"));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.append(&paragraph(&ox_core::i18n::gettext(
        "Choose a file to show on this folder icon.",
    )));
    // Windows stacks the two buttons, as wide as each other.
    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Start)
        .css_classes(["properties-button-column"])
        .build();
    let choose = push_button(&ox_core::i18n::gettext("Choose File..."));
    let restore = push_button(&ox_core::i18n::gettext("Restore Default"));
    for button in [&choose, &restore] {
        button.set_halign(gtk::Align::Fill);
        buttons.append(button);
    }
    text.append(&buttons);
    row.append(&text);
    let preview = gtk::Overlay::builder()
        .valign(gtk::Align::Center)
        .css_classes(["folder-picture-preview"])
        .child(&ArtImage::new(Art::Folder, PREVIEW_SIZE))
        .build();
    let picture = FolderPicture::default();
    picture.place_on_icon(PREVIEW_SIZE);
    picture.set_visible(false);
    preview.add_overlay(&picture);
    row.append(&preview);
    content.append(&row);
    let choice = Rc::new(PictureChoice {
        window: window.downgrade(),
        uri: Rc::clone(uri),
        current: RefCell::new(None),
        chosen: RefCell::new(None),
        preview: picture,
        restore: restore.clone(),
    });
    choice.show(None);
    let chosen = Rc::clone(&choice);
    choose.connect_clicked(move |button| {
        let chosen = Rc::clone(&chosen);
        choose_picture(button, &ox_core::i18n::gettext("Browse"), move |picture| {
            chosen.choose(Some(picture));
        });
    });
    let chosen = Rc::clone(&choice);
    restore.connect_clicked(move |_| chosen.choose(None));
    read_folder_picture(&choice);
    window.add_pending(choice);
    frame
}

/// Reads the folder's picture off the main thread.
fn read_folder_picture(choice: &Rc<PictureChoice>) {
    let uri = choice.uri.borrow().clone();
    let choice = Rc::downgrade(choice);
    glib::spawn_future_local(async move {
        let picture = folder_picture_of(&uri).await;
        if let Some(choice) = choice.upgrade() {
            choice.found(picture);
        }
    });
}
