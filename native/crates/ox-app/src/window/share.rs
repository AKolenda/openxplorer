// SPDX-License-Identifier: AGPL-3.0-only
//! Share: the command bar button that Windows 11 has between Rename and
//! Delete (CMD-037). It emails the selected files through the desktop's
//! Email portal ([`ox_core::integration::compose_email`]), which opens the
//! user's own email app with the files attached; nothing is sent until the
//! user sends the message there.
//!
//! The button shows only where the desktop can share: the window asks the
//! portal for its Email interface when it opens, and keeps the button
//! hidden, not disabled, when no portal backend of the desktop implements
//! it (as on a desktop without KDE's or GTK's portal). An Open or Save
//! dialog never shows it. Like Windows' Share it is offered only for what
//! it can attach: files, on this computer or on a share, and Recent rows
//! (by the file they stand for). A folder, an item of the Recycle Bin or
//! one inside a ZIP, and a Recent row that names no file, disable it.
//! Following the selection reads only addresses, never the disk: a share
//! that is not mounted, or a Recent file that is gone, is found when
//! Share opens the files on a worker thread, and says so. One email
//! takes at most
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
use ox_core::location::{is_virtual_location, scheme, ARCHIVE_SCHEME};
use ox_core::network::local_path;

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

/// Whether `uri` is a row of Recently used files, which stands for
/// another file.
fn is_recent_row(uri: &str) -> bool {
    uri.starts_with("recent:")
}

/// Whether a file at `uri` can have a local path at all, decided from
/// the address alone: not a virtual place such as the Recycle Bin or
/// Recent, and not inside a ZIP.
fn can_be_local(uri: &str) -> bool {
    !is_virtual_location(uri) && !uri.starts_with(ARCHIVE_SCHEME)
}

/// The address of the file Share attaches for the row at `uri` of kind
/// `kind`, decided without touching the disk. A Recent row attaches the
/// file it stands for, `recent_target`, whatever type `GVfs` gave the
/// row; any other row must be a shareable file itself.
fn attachment_source<'a>(
    (uri, recent_target, kind): (&'a str, Option<&'a str>, EntryKind),
) -> Option<&'a str> {
    let source = if is_recent_row(uri) {
        recent_target?
    } else {
        is_shareable(uri, kind).then_some(uri)?
    };
    can_be_local(source).then_some(source)
}

/// The addresses of the files Share attaches for `items`, each `(uri,
/// Recent target, kind)` ([`attachment_source`]); `None` unless there is
/// at least one and every one can have a local file. Only addresses are
/// read, so the selection can follow it on the main thread: whether a
/// share is mounted or a file still exists is found out when Share opens
/// them ([`attachment_path`]).
fn attachment_sources<'a>(
    items: impl IntoIterator<Item = (&'a str, Option<&'a str>, EntryKind)>,
) -> Option<Vec<String>> {
    let sources = items
        .into_iter()
        .map(|item| attachment_source(item).map(str::to_owned))
        .collect::<Option<Vec<String>>>()?;
    (!sources.is_empty()).then_some(sources)
}

/// The local path of the file at `uri`: a listed `file:` address's exact
/// path, a name that is not UTF-8 included, else the mount path of a
/// share mounted by the kernel or by `GVfs`. Can touch the disk and wait
/// for a share, so it runs on a worker thread.
fn attachment_path(uri: &str) -> Option<PathBuf> {
    if scheme(uri).as_deref() == Some("file") {
        return gio::File::for_uri(uri).path();
    }
    local_path(uri)
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
    /// selection of files it can attach. It reads only addresses, never
    /// the disk, as it follows every selection change.
    pub(super) fn update_share_command(&self) {
        let offered = self.share_portal().is_some();
        self.command_bar().offer_share(offered);
        let enabled = offered && self.selected_sources().is_some();
        self.set_action_enabled(WindowAction::Share, enabled);
    }

    /// The addresses of the files Share attaches for the selection
    /// ([`attachment_sources`]): Recent rows by the file they stand for.
    fn selected_sources(&self) -> Option<Vec<String>> {
        let items = self.folder_pane().model().selected_items();
        attachment_sources(items.iter().map(|item| {
            let entry = item.entry();
            (
                entry.uri.as_str(),
                entry.meta.recent_target.as_deref(),
                entry.kind,
            )
        }))
    }

    /// Opens a new email in the desktop's email app with the selected
    /// files attached; a failure shows in the toast.
    pub(super) fn share(&self) {
        let Some(portal) = self.share_portal() else {
            return;
        };
        let Some(sources) = self.selected_sources() else {
            return;
        };
        if sources.len() > MAX_ATTACHMENTS {
            self.show_message(&ox_core::i18n::format_message(
                "Share can attach up to {count} files to one email.",
                &[("count", &MAX_ATTACHMENTS.to_string())],
            ));
            return;
        }
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let result = email_files(&portal, sources).await;
            if let (Err(error), Some(window)) = (result, window.upgrade()) {
                window.show_message(&error.to_string());
            }
        });
    }
}

/// Finds and opens the files at `sources` on a worker thread, within
/// [`OPEN_TIMEOUT`], and asks `portal` to email them. A file with no local
/// path, gone or not a regular file fails as unreadable.
async fn email_files(portal: &str, sources: Vec<String>) -> Result<(), EmailError> {
    let opening = gio::spawn_blocking(move || {
        let paths = sources
            .iter()
            .map(|uri| {
                attachment_path(uri)
                    .ok_or_else(|| EmailError::Unreadable(std::io::ErrorKind::NotFound.into()))
            })
            .collect::<Result<Vec<PathBuf>, EmailError>>()?;
        open_attachments(&paths)
    });
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

    /// The addresses Share would attach for `items`, each `(uri, Recent
    /// target, kind)`.
    fn sources(items: &[(&str, Option<&str>, EntryKind)]) -> Option<Vec<String>> {
        attachment_sources(items.iter().copied())
    }

    fn owned(uris: &[&str]) -> Vec<String> {
        uris.iter().map(|uri| (*uri).to_owned()).collect()
    }

    /// parity: CMD-037
    #[test]
    fn share_needs_files_outside_the_recycle_bin_and_zips() {
        let file = ("file:///srv/plan.txt", None, EntryKind::File);
        let folder = ("file:///srv/Drafts", None, EntryKind::Directory);
        let pipe = ("file:///srv/queue", None, EntryKind::Special);
        let in_the_bin = ("trash:///plan.txt", None, EntryKind::File);
        let zip_member = ox_core::location::ArchiveLocation::root("file:///srv/bundle.zip")
            .member("plan.txt")
            .uri();
        let in_a_zip = (zip_member.as_str(), None, EntryKind::File);

        assert_eq!(sources(&[]), None, "nothing selected");
        assert_eq!(sources(&[file]), Some(owned(&["file:///srv/plan.txt"])));
        assert_eq!(sources(&[folder]), None, "Windows cannot share a folder");
        assert_eq!(sources(&[file, folder]), None);
        assert_eq!(sources(&[pipe]), None, "opening a pipe would wait for a writer");
        assert_eq!(sources(&[in_the_bin]), None);
        assert_eq!(sources(&[file, in_a_zip]), None);
    }

    /// A file on a share, and a Recent row by the file it stands for
    /// (whatever type `GVfs` gave the row), are offered from their
    /// addresses alone; whether a share is mounted or a Recent file still
    /// exists is found out when Share opens them, off the main thread.
    ///
    /// parity: CMD-037
    #[test]
    fn shares_and_recent_rows_are_offered_without_touching_the_disk() {
        let on_a_share = ("smb://studio-nas/projects/plan.txt", None, EntryKind::File);
        let row = "recent:///2c26b46b68ffc68ff99b453c1d304134";
        let recent = (row, Some("file:///srv/notes.txt"), EntryKind::File);
        let shortcut = (
            row,
            Some("smb://studio-nas/projects/plan.txt"),
            EntryKind::Shortcut,
        );
        let unnamed = (row, None, EntryKind::File);

        assert_eq!(
            sources(&[on_a_share, recent, shortcut]),
            Some(owned(&[
                "smb://studio-nas/projects/plan.txt",
                "file:///srv/notes.txt",
                "smb://studio-nas/projects/plan.txt",
            ]))
        );
        assert_eq!(sources(&[unnamed]), None, "GVfs named no file");
        let not_utf8 = (row, Some("file:///tmp/bad%FF"), EntryKind::File);
        assert_eq!(sources(&[not_utf8]), Some(owned(&["file:///tmp/bad%FF"])));
    }

    /// A listed local file maps to its exact path, a name that is not
    /// UTF-8 included; a share without a mount has none.
    ///
    /// parity: CMD-037
    #[test]
    fn a_local_file_resolves_to_its_exact_path() {
        use std::os::unix::ffi::OsStrExt;

        let not_utf8 = std::ffi::OsStr::from_bytes(b"/tmp/bad\xFF");
        assert_eq!(
            attachment_path("file:///tmp/bad%FF"),
            Some(PathBuf::from(not_utf8))
        );
        assert_eq!(
            attachment_path("file:///srv/Plan%20Q3.txt"),
            Some(PathBuf::from("/srv/Plan Q3.txt"))
        );
        assert_eq!(attachment_path("smb://nowhere-nas/media/song.ogg"), None);
    }
}
