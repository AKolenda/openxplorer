// SPDX-License-Identifier: AGPL-3.0-only
//! Presentation classification of a listed item.
//!
//! Ports `classify_entry` in `desktop/entry_model.py`. It never performs a
//! stat, mount or transfer. A GIO directory is not the only navigable
//! object: gvfsd-smb-browse lists shares as mountables and servers as
//! shortcuts, with a `standard::target-uri` and the `inode/directory` MIME
//! type. Navigability stays separate from mutability: a share can be opened
//! from the server browser, but not renamed or sent to the Trash there.

use super::EntryKind;
use crate::location::{normalise, split_location, LocationParts};

/// Type column text for an SMB share in a server listing.
const NETWORK_SHARE: &str = "Network share";
/// Type column text for other navigable virtual items (servers, shortcuts).
const NETWORK_LOCATION: &str = "Network location";
/// Type column text for an ordinary folder.
const FILE_FOLDER: &str = "File folder";

/// The MIME type GIO reports for folders and folder-like items.
const FOLDER_MIME_TYPE: &str = "inode/directory";

/// The GIO metadata of one item, as [`classify_entry`] reads it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ItemMetadata<'a> {
    /// What GIO says the item is.
    pub kind: EntryKind,
    /// The item's own URI.
    pub uri: &'a str,
    /// `standard::content-type`, when reported.
    pub content_type: Option<&'a str>,
    /// `standard::target-uri`: untrusted backend metadata.
    pub target_uri: Option<&'a str>,
    /// `standard::is-virtual`.
    pub is_virtual: bool,
}

/// How an item behaves in the list, independent of its size or dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Classification {
    /// Opens as a folder when activated.
    pub is_dir: bool,
    /// A network share, server or shortcut rather than a real item.
    pub is_virtual: bool,
    /// Can be copied, moved, renamed and trashed.
    pub can_operate: bool,
    /// Where a navigable virtual item leads: the validated backend target,
    /// or the item's own URI when the backend gave none.
    pub target_uri: Option<String>,
    /// Type column text for navigable items; `None` for files, whose type
    /// comes from their content type.
    pub folder_type: Option<&'static str>,
}

/// Classifies one item from its GIO metadata.
pub(super) fn classify_entry(item: &ItemMetadata<'_>) -> Classification {
    let target = followable_target(item);
    let is_dir = is_navigable(item, target.as_deref());
    let is_virtual = has_virtual_kind(item.kind) || item.is_virtual;
    // A navigable share or server opens its validated target, or itself
    // when the backend named none.
    let target_uri = if is_dir && is_virtual {
        Some(target.unwrap_or_else(|| item.uri.to_owned()))
    } else {
        None
    };
    Classification {
        is_dir,
        is_virtual,
        can_operate: has_real_kind(item.kind) && !is_virtual,
        target_uri,
        folder_type: is_dir.then(|| folder_type(item, is_virtual)),
    }
}

/// Shares and shortcuts stand for another location rather than being one.
fn has_virtual_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Mountable | EntryKind::Shortcut)
}

/// Folders, files and links are the items that copy, move, rename and
/// trash act on; device nodes, shares and shortcuts are not.
fn has_real_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Directory | EntryKind::File | EntryKind::Symlink)
}

/// Type column text for a navigable item.
fn folder_type(item: &ItemMetadata<'_>, is_virtual: bool) -> &'static str {
    if item.kind == EntryKind::Mountable && is_smb(item.uri) {
        NETWORK_SHARE
    } else if is_virtual {
        NETWORK_LOCATION
    } else {
        FILE_FOLDER
    }
}

/// True for an `smb://` address.
fn is_smb(uri: &str) -> bool {
    split_location(uri).is_ok_and(|parts| parts.scheme == "smb")
}

/// `standard::target-uri`, with an empty value counted as missing.
fn backend_target<'a>(item: &ItemMetadata<'a>) -> Option<&'a str> {
    item.target_uri.filter(|target| !target.is_empty())
}

/// The backend's target for a share or shortcut, when it is safe to follow.
///
/// Safety rule (untrusted backend targets): a target is only followed when
/// it normalises to a `file://`, `smb://` or device location without
/// credentials, so a listing can never navigate to `javascript:` or persist
/// a password.
fn followable_target(item: &ItemMetadata<'_>) -> Option<String> {
    if !has_virtual_kind(item.kind) {
        return None;
    }
    let target = backend_target(item)?;
    normalise(target).ok()
}

/// Whether activating the item opens it as a folder. Decided from GIO's
/// type and MIME type, never from the name.
fn is_navigable(item: &ItemMetadata<'_>, target: Option<&str>) -> bool {
    let has_folder_mime_type = item.content_type == Some(FOLDER_MIME_TYPE);
    match item.kind {
        EntryKind::Directory => true,
        EntryKind::Unknown => has_folder_mime_type,
        EntryKind::Mountable | EntryKind::Shortcut => {
            let leads_to_a_folder = target.is_some() && has_folder_mime_type;
            leads_to_a_folder || is_mountable_smb_share(item, target)
        }
        EntryKind::File | EntryKind::Symlink | EntryKind::Special => false,
    }
}

/// A mountable item that names a share on an SMB server
/// (`smb://server/share`).
///
/// Some SMB backends omit the content type and target, so such an item is a
/// folder even without them. The fallback is limited to an actual share,
/// not every extensionless file or shortcut, and never applies when the
/// backend named a target that is not safe to follow.
fn is_mountable_smb_share(item: &ItemMetadata<'_>, target: Option<&str>) -> bool {
    if item.kind != EntryKind::Mountable {
        return false;
    }
    let target_was_refused = backend_target(item).is_some() && target.is_none();
    if target_was_refused {
        return false;
    }
    split_location(item.uri).is_ok_and(|source| names_an_smb_share(&source))
}

/// True for `smb://server/share` and the folders below it: an SMB address
/// with both a server and a path.
fn names_an_smb_share(source: &LocationParts) -> bool {
    let has_server = !source.netloc.is_empty();
    let has_share_name = !source.path.trim_matches('/').is_empty();
    source.scheme == "smb" && has_server && has_share_name
}

#[cfg(test)]
mod tests {
    //! Ported from `desktop/tests/test_entry_model.py`. Fixtures mirror
    //! gvfsd-smb-browse metadata, not a connection to a real NAS.

    use super::*;
    use EntryKind::{Directory, File, Mountable, Shortcut, Unknown};

    const DIRECTORY_MIME: Option<&str> = Some(FOLDER_MIME_TYPE);

    /// Classifies an item the backend did not flag as virtual.
    fn classify(
        kind: EntryKind,
        uri: &str,
        content_type: Option<&str>,
        target: Option<&str>,
    ) -> Classification {
        classify_entry(&ItemMetadata {
            kind,
            uri,
            content_type,
            target_uri: target,
            is_virtual: false,
        })
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_smb_share_is_navigable_not_a_mutable_regular_directory`
    ///
    /// parity: NET-003
    #[test]
    fn smb_share_is_navigable_not_a_mutable_regular_directory() {
        let share = classify(
            Mountable,
            "smb://nas/work",
            DIRECTORY_MIME,
            Some("smb://NAS/work"),
        );
        assert!(share.is_dir);
        assert!(share.is_virtual);
        assert!(!share.can_operate);
        assert_eq!(share.target_uri.as_deref(), Some("smb://nas/work"));
        assert_eq!(share.folder_type, Some("Network share"));
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_shortcut_server_uses_real_target`
    ///
    /// parity: NET-003
    #[test]
    fn shortcut_server_uses_real_target() {
        let server = classify(
            Shortcut,
            "smb://workgroup/ALPHA",
            DIRECTORY_MIME,
            Some("smb://ALPHA/"),
        );
        assert!(server.is_dir);
        assert_eq!(server.target_uri.as_deref(), Some("smb://alpha/"));
        assert_eq!(server.folder_type, Some("Network location"));
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_normal_smb_directory`
    ///
    /// parity: NET-003
    #[test]
    fn folder_inside_a_share_is_an_ordinary_operable_folder() {
        let folder = classify(Directory, "smb://nas/work/Design", DIRECTORY_MIME, None);
        assert!(folder.is_dir);
        assert!(!folder.is_virtual);
        assert!(folder.can_operate);
        assert_eq!(folder.target_uri, None);
        assert_eq!(folder.folder_type, Some("File folder"));
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_local_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn local_directory_is_a_folder() {
        assert!(classify(Directory, "file:///tmp/Test", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_extensionless_smb_file_is_not_a_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn extensionless_smb_file_is_not_a_folder() {
        let file = classify(File, "smb://nas/work/README", Some("text/plain"), None);
        assert!(!file.is_dir);
        assert!(file.can_operate);
        assert_eq!(file.folder_type, None);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_empty_regular_file_with_directory_mime_does_not_become_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn empty_regular_file_with_directory_mime_does_not_become_folder() {
        assert!(!classify(File, "file:///tmp/file", DIRECTORY_MIME, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_unknown_directory_mime`
    ///
    /// parity: NAV-040
    #[test]
    fn unknown_kind_with_directory_mime_is_a_folder() {
        assert!(classify(Unknown, "smb://nas/work/dir", DIRECTORY_MIME, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_unknown_without_metadata_is_not_falsely_a_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn unknown_without_metadata_is_not_falsely_a_directory() {
        assert!(!classify(Unknown, "smb://nas/work/unknown", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_mountable_share_without_optional_metadata`
    ///
    /// parity: NAV-040, NET-003
    #[test]
    fn mountable_share_without_optional_metadata_is_a_folder() {
        assert!(classify(Mountable, "smb://nas/work", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_non_smb_mountable_not_assumed_to_be_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn non_smb_mountable_not_assumed_to_be_directory() {
        assert!(!classify(Mountable, "file:///tmp/thing", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_mtp_directory_with_bracketed_usb_identifier`
    ///
    /// parity: DEV-005
    #[test]
    fn mtp_directory_with_bracketed_usb_identifier_is_an_operable_folder() {
        let folder = classify(Directory, "mtp://[usb:001,010]/Internal%20storage", None, None);
        assert!(folder.is_dir);
        assert!(folder.can_operate);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_file_shortcut_is_not_a_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn file_shortcut_is_not_a_folder() {
        let shortcut = classify(
            Shortcut,
            "file:///tmp/shortcut",
            Some("text/plain"),
            Some("file:///tmp/file.txt"),
        );
        assert!(!shortcut.is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_virtual_bad_scheme_is_not_followed`
    ///
    /// parity: NAV-040, SAFE-010
    #[test]
    fn virtual_bad_scheme_is_not_followed() {
        let shortcut = classify(
            Shortcut,
            "smb://group/evil",
            DIRECTORY_MIME,
            Some("javascript:alert(1)"),
        );
        assert!(!shortcut.is_dir);
        assert_eq!(shortcut.target_uri, None);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_credentials_in_backend_target_not_used`
    ///
    /// parity: NAV-040, SAFE-010
    #[test]
    fn credentials_in_backend_target_not_used() {
        let share = classify(
            Mountable,
            "smb://nas/work",
            DIRECTORY_MIME,
            Some("smb://u:secret@nas/work"),
        );
        assert!(!share.is_dir);
        assert_eq!(share.target_uri, None);
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_unicode_target_and_spaces`
    ///
    /// parity: NAV-040
    #[test]
    fn unicode_and_spaces_in_a_target_stay_encoded() {
        let share = classify(
            Mountable,
            "smb://nas/Team%20files",
            DIRECTORY_MIME,
            Some("smb://nas/Team%20files/%C3%89t%C3%A9"),
        );
        assert_eq!(
            share.target_uri.as_deref(),
            Some("smb://nas/Team%20files/%C3%89t%C3%A9")
        );
    }

    /// Ported from `desktop/tests/test_entry_model.py::EntryModelTests::test_metadata_does_not_override_a_real_file_target`
    ///
    /// parity: NAV-040
    #[test]
    fn metadata_does_not_override_a_real_file_target() {
        let file = classify(
            File,
            "file:///tmp/file",
            Some("text/plain"),
            Some("smb://nas/other"),
        );
        assert_eq!(file.target_uri, None);
        assert!(!file.is_dir);
    }

    #[test]
    fn virtual_flag_blocks_operations_on_real_kinds() {
        let mount = classify_entry(&ItemMetadata {
            kind: Directory,
            uri: "file:///run/media/usb",
            content_type: None,
            target_uri: None,
            is_virtual: true,
        });
        assert!(mount.is_dir);
        assert!(mount.is_virtual);
        assert!(!mount.can_operate);
        assert_eq!(mount.target_uri.as_deref(), Some("file:///run/media/usb"));
        assert_eq!(mount.folder_type, Some("Network location"));
    }

    /// parity: NET-003
    #[test]
    fn empty_target_counts_as_missing() {
        let share = classify(Mountable, "smb://nas/work", None, Some(""));
        assert!(share.is_dir);
        assert_eq!(share.target_uri.as_deref(), Some("smb://nas/work"));
    }
}
