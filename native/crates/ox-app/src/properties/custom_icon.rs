// SPDX-License-Identifier: AGPL-3.0-only
//! Change icon… and Restore default icon on the General tab (PROP-016),
//! as Files' Properties and Dolphin's icon button offer them. The choice
//! is stored as `GVfs` metadata (`metadata::custom-icon`, the key Files
//! uses), so both file managers show it; the default Windows-style art
//! stays for every other item.

use gtk::prelude::*;
use gtk::{gio, glib};

use super::general_panel::glyph_button;
use crate::dialog::DialogFrame;
use crate::folder_view::CUSTOM_ICON;
use crate::icons::Icon;

/// The toasts after a change.
const ICON_CHANGED: &str = crate::i18n::message_id("Icon changed.");
const ICON_RESTORED: &str = crate::i18n::message_id("Default icon restored.");

/// Change icon…, and Restore default icon when `has_custom_icon`, for the
/// local item at `uri`.
pub(super) fn icon_buttons(row: &gtk::Box, uri: &str, has_custom_icon: bool) {
    let change = glyph_button(ox_core::i18n::gettext_static("Change icon…"), Icon::Image);
    let target = uri.to_owned();
    change.connect_clicked(move |button| choose_icon(button, &target));
    row.append(&change);
    if has_custom_icon {
        let restore = glyph_button(
            ox_core::i18n::gettext_static("Restore default icon"),
            Icon::ArrowReset,
        );
        let target = uri.to_owned();
        restore.connect_clicked(move |button| {
            let button = button.clone();
            let target = target.clone();
            glib::spawn_future_local(async move {
                let result = set_custom_icon(&target, None).await;
                report(
                    &button,
                    &target,
                    result,
                    ox_core::i18n::gettext_static(ICON_RESTORED),
                );
            });
        });
        row.append(&restore);
    }
}

/// Asks for a picture, then makes it the icon of the item at `uri`.
fn choose_icon(button: &gtk::Button, uri: &str) {
    let images = gtk::FileFilter::new();
    images.set_name(Some("Images"));
    images.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&images);
    let dialog = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Choose an icon"))
        .modal(true)
        .filters(&filters)
        .build();
    let window = button.root().and_downcast::<gtk::Window>();
    let button = button.clone();
    let uri = uri.to_owned();
    glib::spawn_future_local(async move {
        let Ok(chosen) = dialog.open_future(window.as_ref()).await else {
            return;
        };
        let result = set_custom_icon(&uri, Some(chosen.uri().to_string())).await;
        report(&button, &uri, result, ox_core::i18n::gettext_static(ICON_CHANGED));
    });
}

/// Stores `icon` as the custom icon of the item at `uri`, or removes it
/// for `None`. GIO's Rust binding cannot unset an attribute, so removing
/// runs `gio set -t unset`, as Files' "Reset" does through GIO.
///
/// # Errors
///
/// GIO's error, or the failure of `gio set`.
pub(crate) async fn set_custom_icon(uri: &str, icon: Option<String>) -> Result<(), glib::Error> {
    let Some(icon) = icon else {
        let unset = gio::Subprocess::newv(
            &[
                "gio".as_ref(),
                "set".as_ref(),
                "-t".as_ref(),
                "unset".as_ref(),
                uri.as_ref(),
                CUSTOM_ICON.as_ref(),
            ],
            gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE,
        )?;
        return unset.wait_check_future().await;
    };
    let file = gio::File::for_uri(uri);
    let info = gio::FileInfo::new();
    info.set_attribute_string(CUSTOM_ICON, &icon);
    file.set_attributes_future(&info, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT)
        .await
        .map(|_| ())
}

/// Says how the change went, and has the window redraw the item.
fn report(button: &gtk::Button, uri: &str, result: Result<(), glib::Error>, done: &str) {
    let window = button.root().and_downcast::<crate::window::BrowserWindow>();
    match result {
        Ok(()) => {
            if let Some(window) = window {
                window.refresh_item_icon(uri);
                window.show_message(done);
            }
        }
        Err(error) => {
            let frame = button
                .ancestor(DialogFrame::static_type())
                .and_downcast::<DialogFrame>();
            if let Some(frame) = frame {
                frame.show_error(&error.to_string());
            }
        }
    }
}
