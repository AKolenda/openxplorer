// SPDX-License-Identifier: AGPL-3.0-only
//! A folder on a drive the kernel mounted read-only: most often a Windows
//! drive, because Windows is hibernated or used Fast startup (DEV-015).
//!
//! Each time the free space is read, the window also asks GIO whether the
//! folder's file system is read-only and what type it is, off the main
//! thread. While it is, New, Paste, Cut, Rename, Duplicate and Delete are
//! off and say why, Copy stays, and the status bar shows "Read-only
//! drive" with what to do about it as its tooltip. A reply for a folder
//! the tab has left is dropped.
//!
//! The same query asks whether the user may write to the folder itself:
//! a folder without write permission on a writable drive takes no new
//! items, so New and Paste are off there too (CMD-011).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::read_only::ReadOnlyDrive;

use super::BrowserWindow;
use crate::locations::Page;

/// The file system attributes that say whether a drive is read-only.
const ACCESS_ATTRIBUTES: &str = "filesystem::readonly,filesystem::type";

/// Whether the file system of `file` is mounted read-only, and which kind
/// of drive it is; `None` when it is writable or GIO cannot tell.
async fn read_only_drive_of_file(file: &gio::File) -> Option<ReadOnlyDrive> {
    let filesystem = file
        .query_filesystem_info_future(ACCESS_ATTRIBUTES, glib::Priority::LOW)
        .await
        .ok()?;
    let read_only = filesystem.boolean("filesystem::readonly");
    let kind = filesystem.attribute_string("filesystem::type");
    ReadOnlyDrive::from_filesystem(read_only, kind.as_deref())
}

/// Whether GIO says the user may not create items in the local folder
/// `file`, which needs both write and execute (search) permission on it:
/// false when it may, or when GIO cannot tell. Only a local folder is
/// asked, where GIO answers with access(2); a remote backend may guess
/// from permission bits that do not apply to the user.
async fn is_unwritable_folder(file: &gio::File) -> bool {
    if !file.is_native() {
        return false;
    }
    let attributes = format!(
        "{},{}",
        gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE,
        gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE
    );
    let info = file
        .query_info_future(&attributes, gio::FileQueryInfoFlags::NONE, glib::Priority::LOW)
        .await;
    info.is_ok_and(|info| {
        let refuses = |attribute: &str| info.has_attribute(attribute) && !info.boolean(attribute);
        refuses(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE) || refuses(gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE)
    })
}

impl BrowserWindow {
    /// Asks whether the active tab's folder is on a drive mounted
    /// read-only, or one the user may not write to, and shows the answer.
    pub(super) fn refresh_drive_access(&self) {
        let folder = self.current_uri().filter(|uri| Page::from_uri(uri).is_none());
        let Some(uri) = folder else {
            self.show_drive_access(None);
            self.show_folder_access(None);
            return;
        };
        let file = gio::File::for_uri(&uri);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let drive = read_only_drive_of_file(&file).await;
                let unwritable = is_unwritable_folder(&file).await;
                if window.current_uri().as_deref() == Some(uri.as_str()) {
                    window.show_folder_access(unwritable.then(|| uri.clone()));
                    window.show_drive_access(drive.map(|drive| (uri, drive)));
                }
            }
        ));
    }

    /// Records that the user may not write to the folder `unwritable`, or
    /// that nothing says so, and updates the commands when that changed.
    fn show_folder_access(&self, unwritable: Option<String>) {
        if *self.imp().unwritable_folder.borrow() == unwritable {
            return;
        }
        self.imp().unwritable_folder.replace(unwritable);
        self.update_file_commands();
    }

    /// Whether the user may not write to the folder `uri`, as last read.
    pub(crate) fn is_unwritable_folder(&self, uri: &str) -> bool {
        self.imp().unwritable_folder.borrow().as_deref() == Some(uri)
    }

    /// Records that the folder `shown.0` is on the read-only drive
    /// `shown.1`, or on no read-only drive, and updates the commands and
    /// the status bar when that changed.
    pub(super) fn show_drive_access(&self, shown: Option<(String, ReadOnlyDrive)>) {
        let drive = shown.as_ref().map(|(_, drive)| *drive);
        if *self.imp().read_only_drive.borrow() == shown {
            return;
        }
        self.imp().read_only_drive.replace(shown);
        self.status_bar().show_read_only(drive);
        self.update_file_commands();
    }

    /// The read-only drive the folder `uri` is on, as last read.
    pub(crate) fn read_only_drive_of(&self, uri: &str) -> Option<ReadOnlyDrive> {
        let shown = self.imp().read_only_drive.borrow();
        shown
            .as_ref()
            .filter(|(folder, _)| folder == uri)
            .map(|(_, drive)| *drive)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::harness::{wait_until, Fixture, TestWindow};
    use crate::window::file_ops::FileCommand;

    use super::*;

    /// Whether the window action `name` is enabled.
    fn is_enabled(test: &TestWindow, name: &str) -> bool {
        test.window
            .lookup_action(name)
            .is_some_and(|action| action.is_enabled())
    }

    /// A writable folder keeps every command, and no note shows.
    ///
    /// parity: DEV-015
    #[gtk::test]
    fn a_writable_drive_keeps_its_commands() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window.refresh_drive_access();
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(200));

        assert_eq!(test.window.read_only_drive_of(&fixture.uri()), None);
        assert_eq!(test.window.status_bar().read_only_note(), None);
        assert!(is_enabled(&test, "new-folder"));
    }

    /// A test cannot mount a drive read-only, so the answer GIO gives for
    /// a hibernated Windows drive is shown as the query shows it: the
    /// commands that would change the drive turn off and say why, Copy
    /// stays, the status bar explains, and another folder is writable again.
    ///
    /// parity: DEV-015
    #[gtk::test]
    fn a_hibernated_windows_drive_turns_off_the_commands_that_change_it() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window.folder_pane().model().select_only(0);

        test.window
            .show_drive_access(Some((fixture.uri(), ReadOnlyDrive::Windows)));

        for action in ["new-folder", "rename", "trash", "cut", "duplicate"] {
            assert!(!is_enabled(&test, action), "{action} is off");
        }
        assert!(is_enabled(&test, "copy"), "copying out still works");
        let facts = test.window.command_facts();
        assert_eq!(
            facts.refusal(FileCommand::New),
            Some(ReadOnlyDrive::Windows.reason())
        );
        assert_eq!(
            test.window.status_bar().read_only_note().as_deref(),
            Some(ReadOnlyDrive::Windows.explanation())
        );

        test.window
            .navigate(&fixture.uri_of("Documents"))
            .expect("a valid folder");
        test.wait_for_listing("another folder");
        wait_until("the other folder's answer", || {
            test.window.status_bar().read_only_note().is_none()
        });
        assert!(is_enabled(&test, "new-folder"));
    }
}
