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
use crate::location::{normalise, split_location};

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
    let is_real_item = matches!(
        item.kind,
        EntryKind::Directory | EntryKind::File | EntryKind::Symlink
    );
    let folder_type = if !is_dir {
        None
    } else if item.kind == EntryKind::Mountable && is_smb(item.uri) {
        Some(NETWORK_SHARE)
    } else if is_virtual {
        Some(NETWORK_LOCATION)
    } else {
        Some(FILE_FOLDER)
    };
    // A navigable share or server opens its validated target, or itself
    // when the backend named none.
    let target_uri = (is_dir && is_virtual).then(|| target.unwrap_or_else(|| item.uri.to_owned()));
    Classification {
        is_dir,
        is_virtual,
        can_operate: is_real_item && !is_virtual,
        target_uri,
        folder_type,
    }
}

/// Shares and shortcuts stand for another location rather than being one.
fn has_virtual_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Mountable | EntryKind::Shortcut)
}

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
            (target.is_some() && has_folder_mime_type) || is_mountable_smb_share(item, target)
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
    let Ok(source) = split_location(item.uri) else {
        return false;
    };
    let names_a_share =
        source.scheme == "smb" && !source.netloc.is_empty() && !source.path.trim_matches('/').is_empty();
    let target_was_refused = backend_target(item).is_some() && target.is_none();
    names_a_share && !target_was_refused
}

#[cfg(test)]
mod tests {
    //! Ported from `desktop/tests/test_entry_model.py`. Fixtures mirror
    //! gvfsd-smb-browse metadata, not a connection to a real NAS.

    use super::*;
    use EntryKind::{Directory, File, Mountable, Shortcut, Unknown};

    const DIRECTORY_MIME: Option<&str> = Some("inode/directory");

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

    /// Ported from `desktop/tests/test_entry_model.py::test_smb_share_is_navigable_not_a_mutable_regular_directory`
    ///
    /// parity: NET-003
    #[test]
    fn smb_share_is_navigable_not_a_mutable_regular_directory() {
        let e = classify(
            Mountable,
            "smb://nas/work",
            DIRECTORY_MIME,
            Some("smb://NAS/work"),
        );
        assert!(e.is_dir);
        assert!(e.is_virtual);
        assert!(!e.can_operate);
        assert_eq!(e.target_uri.as_deref(), Some("smb://nas/work"));
        assert_eq!(e.folder_type, Some("Network share"));
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_shortcut_server_uses_real_target`
    ///
    /// parity: NET-003
    #[test]
    fn shortcut_server_uses_real_target() {
        let e = classify(
            Shortcut,
            "smb://workgroup/ALPHA",
            DIRECTORY_MIME,
            Some("smb://ALPHA/"),
        );
        assert!(e.is_dir);
        assert_eq!(e.target_uri.as_deref(), Some("smb://alpha/"));
        assert_eq!(e.folder_type, Some("Network location"));
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_normal_smb_directory`
    ///
    /// parity: NET-003
    #[test]
    fn normal_smb_directory() {
        let e = classify(Directory, "smb://nas/work/Design", DIRECTORY_MIME, None);
        assert!(e.is_dir);
        assert!(!e.is_virtual);
        assert!(e.can_operate);
        assert_eq!(e.target_uri, None);
        assert_eq!(e.folder_type, Some("File folder"));
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_local_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn local_directory() {
        assert!(classify(Directory, "file:///tmp/Test", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_extensionless_smb_file_is_not_a_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn extensionless_smb_file_is_not_a_folder() {
        let e = classify(File, "smb://nas/work/README", Some("text/plain"), None);
        assert!(!e.is_dir);
        assert!(e.can_operate);
        assert_eq!(e.folder_type, None);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_empty_regular_file_with_directory_mime_does_not_become_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn empty_regular_file_with_directory_mime_does_not_become_folder() {
        assert!(!classify(File, "file:///tmp/file", DIRECTORY_MIME, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_unknown_directory_mime`
    ///
    /// parity: NAV-040
    #[test]
    fn unknown_directory_mime() {
        assert!(classify(Unknown, "smb://nas/work/dir", DIRECTORY_MIME, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_unknown_without_metadata_is_not_falsely_a_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn unknown_without_metadata_is_not_falsely_a_directory() {
        assert!(!classify(Unknown, "smb://nas/work/unknown", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_mountable_share_without_optional_metadata`
    ///
    /// parity: NAV-040, NET-003
    #[test]
    fn mountable_share_without_optional_metadata() {
        assert!(classify(Mountable, "smb://nas/work", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_non_smb_mountable_not_assumed_to_be_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn non_smb_mountable_not_assumed_to_be_directory() {
        assert!(!classify(Mountable, "file:///tmp/thing", None, None).is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_mtp_directory_with_bracketed_usb_identifier`
    ///
    /// parity: DEV-005
    #[test]
    fn mtp_directory_with_bracketed_usb_identifier() {
        let e = classify(Directory, "mtp://[usb:001,010]/Internal%20storage", None, None);
        assert!(e.is_dir);
        assert!(e.can_operate);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_file_shortcut_is_not_a_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn file_shortcut_is_not_a_folder() {
        let e = classify(
            Shortcut,
            "file:///tmp/shortcut",
            Some("text/plain"),
            Some("file:///tmp/file.txt"),
        );
        assert!(!e.is_dir);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_virtual_bad_scheme_is_not_followed`
    ///
    /// parity: NAV-040, SAFE-010
    #[test]
    fn virtual_bad_scheme_is_not_followed() {
        let e = classify(
            Shortcut,
            "smb://group/evil",
            DIRECTORY_MIME,
            Some("javascript:alert(1)"),
        );
        assert!(!e.is_dir);
        assert_eq!(e.target_uri, None);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_credentials_in_backend_target_not_used`
    ///
    /// parity: NAV-040, SAFE-010
    #[test]
    fn credentials_in_backend_target_not_used() {
        let e = classify(
            Mountable,
            "smb://nas/work",
            DIRECTORY_MIME,
            Some("smb://u:secret@nas/work"),
        );
        assert!(!e.is_dir);
        assert_eq!(e.target_uri, None);
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_unicode_target_and_spaces`
    ///
    /// parity: NAV-040
    #[test]
    fn unicode_target_and_spaces() {
        let e = classify(
            Mountable,
            "smb://nas/Team%20files",
            DIRECTORY_MIME,
            Some("smb://nas/Team%20files/%C3%89t%C3%A9"),
        );
        assert_eq!(
            e.target_uri.as_deref(),
            Some("smb://nas/Team%20files/%C3%89t%C3%A9")
        );
    }

    /// Ported from `desktop/tests/test_entry_model.py::test_metadata_does_not_override_a_real_file_target`
    ///
    /// parity: NAV-040
    #[test]
    fn metadata_does_not_override_a_real_file_target() {
        let e = classify(
            File,
            "file:///tmp/file",
            Some("text/plain"),
            Some("smb://nas/other"),
        );
        assert_eq!(e.target_uri, None);
        assert!(!e.is_dir);
    }

    #[test]
    fn virtual_flag_blocks_operations_on_real_kinds() {
        let e = classify_entry(&ItemMetadata {
            kind: Directory,
            uri: "file:///run/media/usb",
            content_type: None,
            target_uri: None,
            is_virtual: true,
        });
        assert!(e.is_dir);
        assert!(e.is_virtual);
        assert!(!e.can_operate);
        assert_eq!(e.target_uri.as_deref(), Some("file:///run/media/usb"));
        assert_eq!(e.folder_type, Some("Network location"));
    }

    #[test]
    fn empty_target_counts_as_missing() {
        let e = classify(Mountable, "smb://nas/work", None, Some(""));
        assert!(e.is_dir);
        assert_eq!(e.target_uri.as_deref(), Some("smb://nas/work"));
    }
}
