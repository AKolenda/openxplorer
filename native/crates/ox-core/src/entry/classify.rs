// SPDX-License-Identifier: AGPL-3.0-only
//! Presentation classification of a listed item.
//!
//! Ports `classify_entry` in `desktop/entry_model.py`. It never performs a
//! stat, mount or transfer. GIO `DIRECTORY` is not the only navigable object:
//! GVfs smb-browse emits `MOUNTABLE` shares and `SHORTCUT` servers with
//! `standard::target-uri` and `inode/directory`. Navigability stays separate
//! from mutability: a share can be opened from the server browser, but not
//! renamed or sent to the Trash there.

use super::EntryKind;
use crate::location::{normalise, split_location};

/// Type column text for an SMB share in a server listing.
pub const NETWORK_SHARE: &str = "Network share";
/// Type column text for other navigable virtual items (servers, shortcuts).
pub const NETWORK_LOCATION: &str = "Network location";
/// Type column text for an ordinary folder.
pub const FILE_FOLDER: &str = "File folder";

/// How an item behaves in the list, independent of its size or dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// The GIO file type the classification started from.
    pub kind: EntryKind,
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
///
/// `target_uri` is untrusted backend metadata: it is only followed when it
/// normalises to a `file://`, `smb://` or device location without
/// credentials, so a listing can never navigate to `javascript:` or persist
/// a password. `is_virtual` is `standard::is-virtual`.
pub fn classify_entry(
    kind: EntryKind,
    uri: &str,
    content_type: Option<&str>,
    target_uri: Option<&str>,
    is_virtual: bool,
) -> Classification {
    let virtual_kind = matches!(kind, EntryKind::Mountable | EntryKind::Shortcut);
    let target_uri = target_uri.filter(|target| !target.is_empty());
    let target = match target_uri {
        Some(raw) if virtual_kind => normalise(raw).ok(),
        _ => None,
    };
    let folder_mime = content_type == Some("inode/directory");
    let source = split_location(uri).ok();
    let source_is_smb = source.as_ref().is_some_and(|parts| parts.scheme == "smb");

    // Some SMB backends omit the content type and target. Limit that
    // fallback to an actual mountable SMB share, not to every extensionless
    // file or every shortcut.
    let names_a_share = source
        .as_ref()
        .is_some_and(|parts| !parts.netloc.is_empty() && !parts.path.trim_matches('/').is_empty());
    let target_is_usable = target_uri.is_none() || target.is_some();
    let smb_mount = kind == EntryKind::Mountable && source_is_smb && names_a_share && target_is_usable;

    let navigable = match kind {
        EntryKind::Directory => true,
        EntryKind::Unknown => folder_mime,
        EntryKind::Mountable | EntryKind::Shortcut => (target.is_some() && folder_mime) || smb_mount,
        EntryKind::File | EntryKind::Symlink | EntryKind::Special => false,
    };
    let is_virtual = virtual_kind || is_virtual;
    let is_share = navigable && kind == EntryKind::Mountable && source_is_smb;

    let folder_type = if is_share {
        Some(NETWORK_SHARE)
    } else if navigable && is_virtual {
        Some(NETWORK_LOCATION)
    } else if navigable {
        Some(FILE_FOLDER)
    } else {
        None
    };
    let operable_kind = matches!(kind, EntryKind::Directory | EntryKind::File | EntryKind::Symlink);
    let target_uri = if navigable && is_virtual {
        Some(target.unwrap_or_else(|| uri.to_string()))
    } else {
        None
    };
    Classification {
        kind,
        is_dir: navigable,
        is_virtual,
        can_operate: !is_virtual && operable_kind,
        target_uri,
        folder_type,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from `desktop/tests/test_entry_model.py`. Fixtures mirror
    //! GVfs smb-browse metadata, not a connection to a real NAS.

    use super::*;
    use EntryKind::*;

    const DIRECTORY_MIME: Option<&str> = Some("inode/directory");

    fn classify(
        kind: EntryKind,
        uri: &str,
        content_type: Option<&str>,
        target: Option<&str>,
    ) -> Classification {
        classify_entry(kind, uri, content_type, target, false)
    }

    /// Ported from desktop/tests/test_entry_model.py::test_smb_share_is_navigable_not_a_mutable_regular_directory
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

    /// Ported from desktop/tests/test_entry_model.py::test_shortcut_server_uses_real_target
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

    /// Ported from desktop/tests/test_entry_model.py::test_normal_smb_directory
    #[test]
    fn normal_smb_directory() {
        let e = classify(Directory, "smb://nas/work/Design", DIRECTORY_MIME, None);
        assert!(e.is_dir);
        assert!(!e.is_virtual);
        assert!(e.can_operate);
        assert_eq!(e.target_uri, None);
        assert_eq!(e.folder_type, Some("File folder"));
    }

    /// Ported from desktop/tests/test_entry_model.py::test_local_directory
    #[test]
    fn local_directory() {
        assert!(classify(Directory, "file:///tmp/Test", None, None).is_dir);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_extensionless_smb_file_is_not_a_folder
    #[test]
    fn extensionless_smb_file_is_not_a_folder() {
        let e = classify(File, "smb://nas/work/README", Some("text/plain"), None);
        assert!(!e.is_dir);
        assert!(e.can_operate);
        assert_eq!(e.folder_type, None);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_empty_regular_file_with_directory_mime_does_not_become_folder
    #[test]
    fn empty_regular_file_with_directory_mime_does_not_become_folder() {
        assert!(!classify(File, "file:///tmp/file", DIRECTORY_MIME, None).is_dir);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_unknown_directory_mime
    #[test]
    fn unknown_directory_mime() {
        assert!(classify(Unknown, "smb://nas/work/dir", DIRECTORY_MIME, None).is_dir);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_unknown_without_metadata_is_not_falsely_a_directory
    #[test]
    fn unknown_without_metadata_is_not_falsely_a_directory() {
        assert!(!classify(Unknown, "smb://nas/work/unknown", None, None).is_dir);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_mountable_share_without_optional_metadata
    #[test]
    fn mountable_share_without_optional_metadata() {
        assert!(classify(Mountable, "smb://nas/work", None, None).is_dir);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_non_smb_mountable_not_assumed_to_be_directory
    #[test]
    fn non_smb_mountable_not_assumed_to_be_directory() {
        assert!(!classify(Mountable, "file:///tmp/thing", None, None).is_dir);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_mtp_directory_with_bracketed_usb_identifier
    #[test]
    fn mtp_directory_with_bracketed_usb_identifier() {
        let e = classify(Directory, "mtp://[usb:001,010]/Internal%20storage", None, None);
        assert!(e.is_dir);
        assert!(e.can_operate);
    }

    /// Ported from desktop/tests/test_entry_model.py::test_file_shortcut_is_not_a_folder
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

    /// Ported from desktop/tests/test_entry_model.py::test_virtual_bad_scheme_is_not_followed
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

    /// Ported from desktop/tests/test_entry_model.py::test_credentials_in_backend_target_not_used
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

    /// Ported from desktop/tests/test_entry_model.py::test_unicode_target_and_spaces
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

    /// Ported from desktop/tests/test_entry_model.py::test_metadata_does_not_override_a_real_file_target
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
        let e = classify_entry(Directory, "file:///run/media/usb", None, None, true);
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
