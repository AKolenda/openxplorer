// SPDX-License-Identifier: AGPL-3.0-only
//! Dropping items onto a `.desktop` launcher starts its application with
//! them (DND-020), as Dolphin's drop job does.
//!
//! A launcher is a program target ([`super::program`]) when it is a
//! regular desktop entry of type Application with a command, and the user
//! trusts it the way GNOME does: it is executable ("Allow Launching") on a
//! filesystem whose execute bits are real, or it lies in an `applications`
//! folder of the desktop's data folders, where installed applications
//! are. On NTFS, FAT and SMB mounts every file is executable, so there the
//! bit trusts nothing. The application's name comes from
//! the entry, so the hint reads "Open with Text Editor".
//!
//! The application starts through `GLib`'s own `gio launch`, which reads the
//! entry's command, field codes and terminal flag exactly as the desktop
//! does; the dropped items are its file arguments, never a command line.
//! Launchers get the same "Run this program?" question as programs on a
//! network share, a removable drive or a drive without Unix permissions.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::network::local_path;

use super::program::{ProgramKind, ProgramTarget};

/// The content type of a desktop entry.
const DESKTOP_CONTENT_TYPE: &str = "application/x-desktop";

/// The group of a desktop entry's keys.
const DESKTOP_GROUP: &str = "Desktop Entry";

/// `GLib`'s command that starts an application from a desktop entry.
const GIO_COMMAND: &str = "gio";

/// True when GIO's `info` describes a desktop entry file.
pub(super) fn is_desktop_entry(info: &gio::FileInfo) -> bool {
    let is_regular = info.file_type() == gio::FileType::Regular;
    let content_type = info.content_type();
    is_regular
        && content_type
            .is_some_and(|content_type| gio::content_type_is_a(&content_type, DESKTOP_CONTENT_TYPE))
}

/// The launcher `entry` is, going by GIO's `info` about it: `None` for an
/// entry the user has not trusted, that is not an application, or that
/// cannot be read.
pub(super) async fn query_launcher(entry: &Entry, info: &gio::FileInfo) -> Option<ProgramTarget> {
    let path = local_path(&entry.uri)?;
    let keeps_permissions = super::program::keeps_permissions(&entry.uri).await;
    if !is_trusted(&path, info, keeps_permissions) {
        return None;
    }
    let (contents, _) = gio::File::for_uri(&entry.uri).load_contents_future().await.ok()?;
    let name = application_name(&glib::Bytes::from_owned(contents))?;
    Some(ProgramTarget {
        uri: entry.uri.clone(),
        name,
        kind: ProgramKind::Launcher,
    })
}

/// Whether the launcher at `path`, which GIO's `info` describes, may run:
/// it is executable on a filesystem that `keeps_permissions`, or installed
/// in an applications folder.
pub(super) fn is_trusted(path: &Path, info: &gio::FileInfo, keeps_permissions: bool) -> bool {
    let allowed_to_launch = keeps_permissions && info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE);
    allowed_to_launch || is_in_applications_folder(path)
}

/// True for a launcher in the `applications` folder of the user's or the
/// system's data folders.
fn is_in_applications_folder(path: &Path) -> bool {
    let Some(folder) = path.parent() else {
        return false;
    };
    let data_folders = std::iter::once(glib::user_data_dir()).chain(glib::system_data_dirs());
    data_folders
        .map(|data| data.join("applications"))
        .any(|applications| folder.starts_with(&applications))
}

/// The application's name in the desktop entry `contents`: `None` unless
/// it is a shown application with a command.
fn application_name(contents: &glib::Bytes) -> Option<String> {
    let entry = glib::KeyFile::new();
    entry.load_from_bytes(contents, glib::KeyFileFlags::NONE).ok()?;
    let is_application = entry.string(DESKTOP_GROUP, "Type").ok()? == "Application";
    let has_command = entry
        .string(DESKTOP_GROUP, "Exec")
        .is_ok_and(|command| !command.trim().is_empty());
    let is_hidden = entry.boolean(DESKTOP_GROUP, "Hidden").unwrap_or(false);
    if !is_application || !has_command || is_hidden {
        return None;
    }
    let name = entry.locale_string(DESKTOP_GROUP, "Name", None).ok()?;
    Some(name.to_string())
}

/// The command that starts the launcher at `path` with `items` as its
/// file arguments.
pub(super) fn launch_command(path: &Path, items: &[OsString]) -> Vec<OsString> {
    let mut command = vec![OsString::from(GIO_COMMAND), OsString::from("launch")];
    command.push(path.as_os_str().to_owned());
    command.extend(items.iter().cloned());
    command
}

/// The folder a launcher's application starts in: the user's home, as
/// the desktop starts applications.
pub(super) fn launch_folder() -> PathBuf {
    glib::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> glib::Bytes {
        glib::Bytes::from_owned(text.as_bytes().to_vec())
    }

    /// parity: DND-020
    #[test]
    fn only_shown_applications_with_a_command_are_launchers() {
        let application =
            entry("[Desktop Entry]\nType=Application\nName=Text Editor\nExec=gnome-text-editor %U\n");
        let link = entry("[Desktop Entry]\nType=Link\nName=Web\nURL=https://example.com\n");
        let hidden = entry("[Desktop Entry]\nType=Application\nName=Old\nExec=old\nHidden=true\n");
        let no_command = entry("[Desktop Entry]\nType=Application\nName=Empty\n");

        assert_eq!(application_name(&application).as_deref(), Some("Text Editor"));
        assert_eq!(application_name(&link), None);
        assert_eq!(application_name(&hidden), None);
        assert_eq!(application_name(&no_command), None);
    }

    /// On NTFS, FAT and SMB mounts every launcher is executable, so the
    /// bit trusts it only where permissions are real.
    ///
    /// parity: DND-020
    #[test]
    fn an_executable_launcher_is_trusted_only_where_permissions_are_real() {
        let executable = gio::FileInfo::new();
        executable.set_attribute_boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE, true);
        let downloaded = Path::new("/media/ada/Windows/Users/ada/Downloads/tool.desktop");

        assert!(is_trusted(downloaded, &executable, true));
        assert!(!is_trusted(downloaded, &executable, false));
    }

    /// parity: DND-020
    #[test]
    fn a_launcher_runs_through_gio_launch_with_each_item_as_one_argument() {
        let items = [OsString::from("/home/ada/a;b $(rm).txt")];

        let command = launch_command(Path::new("/home/ada/Desktop/editor.desktop"), &items);

        let expected: Vec<OsString> = [
            "gio",
            "launch",
            "/home/ada/Desktop/editor.desktop",
            "/home/ada/a;b $(rm).txt",
        ]
        .map(OsString::from)
        .to_vec();
        assert_eq!(command, expected);
    }
}
