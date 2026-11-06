// SPDX-License-Identifier: AGPL-3.0-only
//! The Customize tab of a folder, laid out as Windows 11's, with its three
//! groups in Windows' order:
//!
//! - "What kind of folder do you want?" ([`folder_type`], PROP-036):
//!   "Optimize this folder for:" General items, Documents, Pictures, Music
//!   or Videos, and "Also apply this template to all subfolders";
//! - "Folder pictures" ([`folder_picture`], PROP-037): "Choose File..."
//!   puts a picture on the folder's icon in the larger icon views, and
//!   "Restore Default" takes it off;
//! - "Folder icons" (PROP-016): "Change Icon..." picks a picture, which
//!   shows beside it at once and becomes the folder's icon. Restore
//!   Default, beside it while the folder has its own icon, goes back to
//!   the standard folder. The choice is stored as `GVfs` metadata
//!   (`metadata::custom-icon`, the key Files uses), so both file managers
//!   show it.
//!
//! Every choice waits for OK or Apply, and none writes into the folder:
//! the template goes to the app's settings, beside each folder's view,
//! and the pictures to the user's `GVfs` metadata store.

mod folder_picture;
mod folder_type;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::window::{group, paragraph, push_button, ApplyFuture, Pending, PropertiesWindow};
use crate::folder_view::CUSTOM_ICON;
use crate::icons::{Art, ArtImage};

/// The size of the icon preview.
const PREVIEW_SIZE: i32 = 32;

/// The icon chosen, until OK or Apply.
struct IconChoice {
    window: glib::WeakRef<PropertiesWindow>,
    /// The folder's location, which a rename on General changes.
    uri: Rc<RefCell<String>>,
    /// The folder's own icon now, if it has one.
    current: RefCell<Option<String>>,
    /// The icon the user chose: a picture, or `None` for the default.
    chosen: RefCell<Option<String>>,
    preview: gtk::Box,
    restore: gtk::Button,
}

impl IconChoice {
    fn show(&self, icon: Option<&str>) {
        while let Some(child) = self.preview.first_child() {
            self.preview.remove(&child);
        }
        match icon {
            Some(icon) => {
                let picture = gtk::Image::from_gicon(&gio::FileIcon::new(&gio::File::for_uri(icon)));
                picture.set_pixel_size(PREVIEW_SIZE);
                self.preview.append(&picture);
            }
            None => self.preview.append(&ArtImage::new(Art::Folder, PREVIEW_SIZE)),
        }
        self.restore.set_visible(icon.is_some());
        if let Some(window) = self.window.upgrade() {
            window.changed();
        }
    }

    fn choose(&self, icon: Option<String>) {
        self.show(icon.as_deref());
        self.chosen.replace(icon);
        if let Some(window) = self.window.upgrade() {
            window.changed();
        }
    }
}

impl Pending for IconChoice {
    fn has_changes(&self) -> bool {
        *self.chosen.borrow() != *self.current.borrow()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            let icon = self.chosen.borrow().clone();
            let uri = self.uri.borrow().clone();
            set_custom_icon(&uri, icon.clone())
                .await
                .map_err(|error| error.to_string())?;
            self.current.replace(icon);
            if let Some(opener) = self.window.upgrade().and_then(|window| window.opener()) {
                opener.announce_item_icon(&uri);
            }
            Ok(())
        })
    }
}

/// The Customize tab of the local folder at `uri`, whose own icon is
/// `current`.
pub(super) fn customize_tab(
    window: &PropertiesWindow,
    uri: &Rc<RefCell<String>>,
    current: Option<&str>,
) -> gtk::Box {
    let page = super::window::page();
    page.append(&folder_type::folder_type_group(window, uri));
    page.append(&folder_picture::folder_picture_group(window, uri));
    let (frame, content) = group(&ox_core::i18n::gettext("Folder icons"));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.append(&paragraph(&ox_core::i18n::gettext(
        "You can change the folder icon. If you change the icon, it will no longer show a preview of the \
         folder's contents.",
    )));
    let buttons = gtk::Box::builder().css_classes(["properties-button-row"]).build();
    let change = push_button(&ox_core::i18n::gettext("Change Icon..."));
    let restore = push_button(&ox_core::i18n::gettext("Restore Default"));
    buttons.append(&change);
    buttons.append(&restore);
    text.append(&buttons);
    row.append(&text);
    let preview = gtk::Box::builder()
        .valign(gtk::Align::End)
        .css_classes(["properties-icon-preview"])
        .build();
    row.append(&preview);
    content.append(&row);
    page.append(&frame);
    let choice = Rc::new(IconChoice {
        window: window.downgrade(),
        uri: Rc::clone(uri),
        current: RefCell::new(current.map(str::to_owned)),
        chosen: RefCell::new(current.map(str::to_owned)),
        preview,
        restore: restore.clone(),
    });
    choice.show(current);
    let chosen = Rc::clone(&choice);
    change.connect_clicked(move |button| choose_icon(button, &chosen));
    let chosen = Rc::clone(&choice);
    restore.connect_clicked(move |_| chosen.choose(None));
    window.add_pending(choice);
    page
}

/// Asks for a picture for the folder's icon.
fn choose_icon(button: &gtk::Button, choice: &Rc<IconChoice>) {
    let choice = Rc::clone(choice);
    choose_picture(button, &ox_core::i18n::gettext("Change Icon"), move |icon| {
        choice.choose(Some(icon));
    });
}

/// Stores `icon` as the custom icon of the item at `uri`, or removes it
/// for `None`.
///
/// # Errors
///
/// GIO's error, or the failure of `gio set`.
pub(crate) async fn set_custom_icon(uri: &str, icon: Option<String>) -> Result<(), glib::Error> {
    set_metadata(uri, CUSTOM_ICON, icon).await
}

/// Stores `value` under the `GVfs` metadata `key` of the item at `uri`, or
/// removes the key for `None`. GIO's Rust binding cannot unset an
/// attribute, so removing runs `gio set -t unset`, as Files' "Reset" does
/// through GIO.
///
/// # Errors
///
/// GIO's error, or the failure of `gio set`.
pub(super) async fn set_metadata(uri: &str, key: &str, value: Option<String>) -> Result<(), glib::Error> {
    let Some(value) = value else {
        let unset = gio::Subprocess::newv(
            &[
                "gio".as_ref(),
                "set".as_ref(),
                "-t".as_ref(),
                "unset".as_ref(),
                uri.as_ref(),
                key.as_ref(),
            ],
            gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE,
        )?;
        return unset.wait_check_future().await;
    };
    let file = gio::File::for_uri(uri);
    let info = gio::FileInfo::new();
    info.set_attribute_string(key, &value);
    file.set_attributes_future(&info, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT)
        .await
        .map(|_| ())
}

/// Asks for a picture, titled `title`, and hands its location to `chosen`.
pub(super) fn choose_picture(button: &gtk::Button, title: &str, chosen: impl FnOnce(String) + 'static) {
    let images = gtk::FileFilter::new();
    images.set_name(Some(&ox_core::i18n::gettext("Images")));
    images.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&images);
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .modal(true)
        .filters(&filters)
        .build();
    let window = button.root().and_downcast::<gtk::Window>();
    glib::spawn_future_local(async move {
        if let Ok(file) = dialog.open_future(window.as_ref()).await {
            chosen(file.uri().to_string());
        }
    });
}
