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
//! it can attach: local files, files on a share mounted by the kernel or
//! by `GVfs` (by their mount path), and Recent rows (by the file they
//! stand for). A folder, an item of the Recycle Bin or one inside a ZIP,
//! and an item with no local file, disable it. One email takes at most
//! [`MAX_ATTACHMENTS`] files; more are refused with the reason.
//!
//! Before, the button with the share glyph copied the path ("Copy path
//! (does not change sharing permissions)"); Copy as path stays in the
//! context menu and on Ctrl+Shift+C ([`super::copy_path`]).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::EntryKind;
use ox_core::integration::{
    compose_email, email_portal_version, open_attachments, EmailError, MAX_ATTACHMENTS,
};
use ox_core::location::ARCHIVE_SCHEME;
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

/// The local file Share attaches for the row at `uri` of kind `kind`, as
/// `local_path` finds it. A Recent row attaches the file it stands for,
/// `recent_target`, whatever type `GVfs` gave the row, and only while that
/// is a regular file, as `is_file` says; any other row must be
/// shareable itself.
fn attachment_path(
    (uri, recent_target, kind): (&str, Option<&str>, EntryKind),
    local_path: impl Fn(&str) -> Option<PathBuf>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if is_recent_row(uri) {
        return local_path(recent_target?).filter(|path| is_file(path));
    }
    is_shareable(uri, kind).then(|| local_path(uri)).flatten()
}

/// The local files Share attaches for `items`, each `(uri, Recent target,
/// kind)` ([`attachment_path`]); `None` unless there is at least one and
/// every one has a file to attach, so Share is offered only for what it
/// can attach.
fn attachment_paths<'a>(
    items: impl IntoIterator<Item = (&'a str, Option<&'a str>, EntryKind)>,
    local_path: impl Fn(&str) -> Option<PathBuf>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<Vec<PathBuf>> {
    let paths = items
        .into_iter()
        .map(|item| attachment_path(item, &local_path, &is_file))
        .collect::<Option<Vec<PathBuf>>>()?;
    (!paths.is_empty()).then_some(paths)
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
    /// selection of files it can attach. More files than one email takes
    /// keep it enabled without looking for them, so a click says why.
    pub(super) fn update_share_command(&self) {
        let offered = self.share_portal().is_some();
        self.command_bar().offer_share(offered);
        let items = self.folder_pane().model().selected_items();
        let enabled = offered
            && if items.len() > MAX_ATTACHMENTS {
                items.iter().all(|item| {
                    let entry = item.entry();
                    is_recent_row(&entry.uri) || is_shareable(&entry.uri, entry.kind)
                })
            } else {
                self.selected_attachments().is_some()
            };
        self.set_action_enabled(WindowAction::Share, enabled);
    }

    /// The local files of the selected items, when Share can attach every
    /// one: files on a share mounted by the kernel or by `GVfs` by their
    /// mount path, and Recent rows by the file they stand for.
    fn selected_attachments(&self) -> Option<Vec<PathBuf>> {
        let items = self.folder_pane().model().selected_items();
        attachment_paths(
            items.iter().map(|item| {
                let entry = item.entry();
                (
                    entry.uri.as_str(),
                    entry.meta.recent_target.as_deref(),
                    entry.kind,
                )
            }),
            local_path,
            Path::is_file,
        )
    }

    /// Opens a new email in the desktop's email app with the selected
    /// files attached; a failure shows in the toast.
    pub(super) fn share(&self) {
        let Some(portal) = self.share_portal() else {
            return;
        };
        if self.folder_pane().model().selected_uris().len() > MAX_ATTACHMENTS {
            self.show_message(&ox_core::i18n::format_message(
                "Share can attach up to {count} files to one email.",
                &[("count", &MAX_ATTACHMENTS.to_string())],
            ));
            return;
        }
        let Some(paths) = self.selected_attachments() else {
            return;
        };
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let result = email_files(&portal, paths).await;
            if let (Err(error), Some(window)) = (result, window.upgrade()) {
                window.show_message(&error.to_string());
            }
        });
    }
}

/// Opens the files at `paths` on a worker thread and asks `portal` to
/// email them.
async fn email_files(portal: &str, paths: Vec<PathBuf>) -> Result<(), EmailError> {
    let opening = gio::spawn_blocking(move || open_attachments(&paths));
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

    /// A stand-in for `ox_core::network::local_path`: local files, and
    /// the one share that a kernel CIFS mount serves at /mnt/projects.
    fn resolve(uri: &str) -> Option<PathBuf> {
        if let Some(path) = uri.strip_prefix("smb://studio-nas/projects/") {
            return Some(PathBuf::from("/mnt/projects").join(path));
        }
        uri.strip_prefix("file://").map(PathBuf::from)
    }

    /// A stand-in for the file system: every path is a regular file but
    /// /srv/deleted.txt, which is gone.
    fn is_file(path: &Path) -> bool {
        path != Path::new("/srv/deleted.txt")
    }

    /// The paths Share would attach for `items`, each `(uri, Recent
    /// target, kind)`.
    fn paths(items: &[(&str, Option<&str>, EntryKind)]) -> Option<Vec<PathBuf>> {
        attachment_paths(items.iter().copied(), resolve, is_file)
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

        assert_eq!(paths(&[]), None, "nothing selected");
        assert_eq!(paths(&[file]), Some(vec![PathBuf::from("/srv/plan.txt")]));
        assert_eq!(paths(&[folder]), None, "Windows cannot share a folder");
        assert_eq!(paths(&[file, folder]), None);
        assert_eq!(paths(&[pipe]), None, "opening a pipe would wait for a writer");
        assert_eq!(paths(&[in_the_bin]), None);
        assert_eq!(paths(&[file, in_a_zip]), None);
    }

    /// A file on a share mounted by the kernel attaches by its mount
    /// path; an item with no local file leaves Share disabled rather than
    /// failing on click.
    ///
    /// parity: CMD-037
    #[test]
    fn a_file_on_a_mounted_share_attaches_by_its_mount_path() {
        let mounted = ("smb://studio-nas/projects/plan.txt", None, EntryKind::File);
        let unmounted = ("smb://other-nas/media/song.ogg", None, EntryKind::File);

        assert_eq!(
            paths(&[mounted]),
            Some(vec![PathBuf::from("/mnt/projects/plan.txt")])
        );
        assert_eq!(paths(&[mounted, unmounted]), None, "no local file");
    }

    /// A Recent row, whose own URI is a hash `GVfs` made, attaches the file
    /// it stands for, whether `GVfs` types the row as a file or a shortcut;
    /// a row whose file is gone, is a folder, or was not named leaves
    /// Share disabled.
    ///
    /// parity: CMD-037
    #[test]
    fn a_recent_row_attaches_the_file_it_stands_for() {
        let row = "recent:///2c26b46b68ffc68ff99b453c1d304134";
        let recent = (row, Some("file:///srv/notes.txt"), EntryKind::File);
        let shortcut = (
            row,
            Some("smb://studio-nas/projects/plan.txt"),
            EntryKind::Shortcut,
        );
        let deleted = (row, Some("file:///srv/deleted.txt"), EntryKind::File);
        let unnamed = (row, None, EntryKind::File);

        assert_eq!(
            paths(&[recent, shortcut]),
            Some(vec![
                PathBuf::from("/srv/notes.txt"),
                PathBuf::from("/mnt/projects/plan.txt")
            ])
        );
        assert_eq!(paths(&[deleted]), None, "the file is gone");
        assert_eq!(paths(&[unnamed]), None, "GVfs named no file");
    }
}
