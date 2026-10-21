// SPDX-License-Identifier: AGPL-3.0-only
//! Share: the command bar button that Windows 11 has between Rename and
//! Delete (CMD-036). It emails the selected files through the desktop's
//! Email portal ([`ox_core::integration::compose_email`]), which opens the
//! user's own email app with the files attached; nothing is sent until the
//! user sends the message there.
//!
//! The button shows only where the desktop can share: the window asks the
//! portal for its Email interface when it opens, and keeps the button
//! hidden, not disabled, when no portal backend of the desktop implements
//! it (as on a desktop without KDE's or GTK's portal). An Open or Save
//! dialog never shows it. Like Windows' Share it needs files: a folder,
//! an item of the Recycle Bin or one inside a ZIP disables it, as nothing
//! can attach them. One email takes at most
//! [`MAX_ATTACHMENTS`] files; more are refused with the reason.
//!
//! Before, the button with the share glyph copied the path ("Copy path
//! (does not change sharing permissions)"); Copy as path stays in the
//! context menu and on Ctrl+Shift+C ([`super::copy_path`]).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::EntryKind;
use ox_core::integration::{
    compose_email, email_portal_version, open_attachments, EmailError, MAX_ATTACHMENTS,
};
use ox_core::location::ARCHIVE_SCHEME;

use super::window_action::WindowAction;
use super::BrowserWindow;

/// How long opening the files may take: a share that stopped answering
/// must not leave Share waiting for ever.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// What the window knows about the desktop's Email portal.
#[derive(Debug, Default)]
pub(super) struct ShareState {
    /// Numbers the checks, so only the latest one's answer counts.
    check: Cell<u32>,
    /// The portal's bus name and whether it can share, once the latest
    /// check answered.
    service: RefCell<Option<(String, bool)>>,
}

/// Whether the item at `uri` of kind `kind` can be attached to an email:
/// a regular file (not a folder, pipe or device) outside the Recycle Bin
/// and outside a ZIP.
fn is_shareable(uri: &str, kind: EntryKind) -> bool {
    kind == EntryKind::File && !uri.starts_with("trash:") && !uri.starts_with(ARCHIVE_SCHEME)
}

/// Whether Share can send `items`, each `(uri, kind)`: at least one, and
/// every one shareable.
fn can_share<'a>(items: impl IntoIterator<Item = (&'a str, EntryKind)>) -> bool {
    let mut items = items.into_iter().peekable();
    items.peek().is_some() && items.all(|(uri, kind)| is_shareable(uri, kind))
}

impl BrowserWindow {
    /// Asks the portal called `portal` whether it can share by email, and
    /// shows or hides Share by its answer.
    pub(crate) fn follow_share_service(&self, portal: &str) {
        let state = &self.imp().share;
        let check = state.check.get().wrapping_add(1);
        state.check.set(check);
        let (window, portal) = (self.downgrade(), portal.to_owned());
        glib::spawn_future_local(async move {
            let available = match gio::bus_get_future(gio::BusType::Session).await {
                Ok(connection) => {
                    crate::theme::system::portal_ready(&connection).await;
                    email_portal_version(&connection, &portal).await.is_some()
                }
                Err(_) => false,
            };
            let Some(window) = window.upgrade() else {
                return;
            };
            let state = &window.imp().share;
            if state.check.get() == check {
                state.service.replace(Some((portal, available)));
                window.update_share_command();
            }
        });
    }

    /// Whether the latest check asked `portal` and has its answer.
    #[cfg(test)]
    pub(crate) fn share_service_checked(&self, portal: &str) -> bool {
        let service = self.imp().share.service.borrow();
        service.as_ref().is_some_and(|(asked, _)| asked == portal)
    }

    /// The bus name of the portal that can share; `None` where none can,
    /// and in an Open or Save dialog.
    fn share_portal(&self) -> Option<String> {
        if self.is_picking() {
            return None;
        }
        let service = self.imp().share.service.borrow();
        service
            .as_ref()
            .filter(|(_, available)| *available)
            .map(|(portal, _)| portal.clone())
    }

    /// Shows Share where the desktop can share, and enables it for a
    /// selection of files.
    pub(super) fn update_share_command(&self) {
        let offered = self.share_portal().is_some();
        self.command_bar().offer_share(offered);
        let items = self.folder_pane().model().selected_items();
        let enabled = offered
            && can_share(
                items
                    .iter()
                    .map(|item| (item.entry().uri.as_str(), item.entry().kind)),
            );
        self.set_action_enabled(WindowAction::Share, enabled);
    }

    /// Opens a new email in the desktop's email app with the selected
    /// files attached; a failure shows in the toast.
    pub(super) fn share(&self) {
        let Some(portal) = self.share_portal() else {
            return;
        };
        let uris = self.folder_pane().model().selected_uris();
        if uris.len() > MAX_ATTACHMENTS {
            self.show_message(&ox_core::i18n::format_message(
                "Share can attach up to {count} files to one email.",
                &[("count", &MAX_ATTACHMENTS.to_string())],
            ));
            return;
        }
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let result = email_files(&portal, uris).await;
            if let (Err(error), Some(window)) = (result, window.upgrade()) {
                window.show_message(&error.to_string());
            }
        });
    }
}

/// The local paths of the items at `uris`; shares and devices that `GVfs`
/// mounts have one too. Asking `GVfs` can block, so it runs on a worker
/// thread.
fn local_paths(uris: &[String]) -> Result<Vec<PathBuf>, EmailError> {
    uris.iter()
        .map(|uri| {
            gio::File::for_uri(uri)
                .path()
                .ok_or_else(|| EmailError::Unreadable(std::io::ErrorKind::NotFound.into()))
        })
        .collect()
}

/// Opens the items at `uris` on a worker thread and asks `portal` to
/// email them.
async fn email_files(portal: &str, uris: Vec<String>) -> Result<(), EmailError> {
    let opening = gio::spawn_blocking(move || open_attachments(&local_paths(&uris)?));
    let timed_out = || EmailError::Unreadable(std::io::ErrorKind::TimedOut.into());
    let files = match glib::future_with_timeout(OPEN_TIMEOUT, opening).await {
        Ok(Ok(opened)) => opened?,
        Ok(Err(_)) | Err(_) => return Err(timed_out()),
    };
    let connection = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(EmailError::Unavailable)?;
    compose_email(&connection, portal, &files).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: CMD-036
    #[test]
    fn share_needs_files_outside_the_recycle_bin_and_zips() {
        let file = ("file:///srv/plan.txt", EntryKind::File);
        let folder = ("file:///srv/Drafts", EntryKind::Directory);
        let pipe = ("file:///srv/queue", EntryKind::Special);
        let on_a_share = ("smb://studio-nas/projects/plan.txt", EntryKind::File);
        let in_the_bin = ("trash:///plan.txt", EntryKind::File);
        let zip_member = ox_core::location::ArchiveLocation::root("file:///srv/bundle.zip")
            .member("plan.txt")
            .uri();
        let in_a_zip = (zip_member.as_str(), EntryKind::File);

        assert!(!can_share([]), "nothing selected");
        assert!(can_share([file]));
        assert!(can_share([file, on_a_share]), "files on a share too");
        assert!(!can_share([folder]), "Windows cannot share a folder");
        assert!(!can_share([file, folder]));
        assert!(!can_share([pipe]), "opening a pipe would wait for a writer");
        assert!(!can_share([in_the_bin]));
        assert!(!can_share([file, in_a_zip]));
    }
}
