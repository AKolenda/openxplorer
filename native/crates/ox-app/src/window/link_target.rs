// SPDX-License-Identifier: AGPL-3.0-only
//! "Show target" on one selected symbolic link: the folder that holds
//! the link's destination opens with the destination selected, or a
//! message says that the destination is missing (CMD-030).
//!
//! Ports Dolphin's `show_target` (`DolphinMainWindow::showTarget`). The
//! link is read without following it, a relative target is resolved
//! against the link's folder, and the destination is checked before the
//! window goes there, so a dangling link never opens an empty folder.

use gtk::prelude::*;
use gtk::{gio, glib};

use super::result_location::LocationTarget;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The destination of the symbolic link at `uri`, as a URI, once it is
/// known to exist; otherwise the message that says why it cannot be
/// shown.
pub(super) async fn link_destination(uri: &str) -> Result<String, String> {
    let link = gio::File::for_uri(uri);
    let info = link
        .query_info_future(
            "standard::symlink-target,standard::display-name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .map_err(|error| error.message().to_owned())?;
    let name = info.display_name();
    let Some(target) = info.symlink_target() else {
        return Err(ox_core::i18n::format_message(
            "“{name}” is not a link.",
            &[("name", name.as_ref())],
        ));
    };
    // A relative target is relative to the folder that holds the link.
    let destination = match link.parent() {
        Some(folder) => folder.resolve_relative_path(&target),
        None => gio::File::for_path(&target),
    };
    let exists = destination
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .is_ok();
    if !exists {
        return Err(ox_core::i18n::format_message(
            "The target of “{name}” does not exist: {display}",
            &[
                ("name", name.as_ref()),
                ("display", &target.display().to_string()),
            ],
        ));
    }
    Ok(destination.uri().into())
}

impl BrowserWindow {
    /// Adds "Show target".
    pub(super) fn install_link_target_action(&self) {
        self.add_action_entries([super::actions::plain_action(
            WindowAction::ShowTarget,
            BrowserWindow::show_link_target,
        )]);
    }

    /// Enables "Show target" while exactly one symbolic link is selected.
    pub(super) fn update_show_target_action(&self) {
        let items = self.folder_pane().model().selected_items();
        let is_one_link = matches!(items.as_slice(), [item] if item.entry().is_symlink);
        self.set_action_enabled(WindowAction::ShowTarget, is_one_link);
    }

    /// Opens the folder of the selected link's destination with the
    /// destination selected, or says that it is missing.
    fn show_link_target(&self) {
        let items = self.folder_pane().model().selected_items();
        let [item] = items.as_slice() else {
            return;
        };
        let uri = item.entry().uri.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match link_destination(&uri).await {
                    Ok(destination) => window.open_item_location(destination, LocationTarget::ThisTab),
                    Err(message) => window.show_message(&message),
                }
            }
        ));
    }
}
