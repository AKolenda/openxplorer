// SPDX-License-Identifier: AGPL-3.0-only
//! Presentation classification of a listed item.
//!
//! Ports `classify_entry` in `v2.0.0:desktop/entry_model.py`. It never performs a
//! stat, mount or transfer. A GIO directory is not the only navigable
//! object: gvfsd-smb-browse lists shares as mountables and servers as
//! shortcuts, with a `standard::target-uri` and the `inode/directory` MIME
//! type. Navigability stays separate from mutability: a share can be opened
//! from the server browser, but not renamed or sent to the Trash there.

use super::EntryKind;
use crate::location::{normalise, split_location, LocationParts};

/// The MIME type GIO reports for folders and folder-like items.
pub(super) const FOLDER_MIME_TYPE: &str = "inode/directory";

/// What the Type column calls a navigable item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FolderType {
    /// An ordinary folder.
    FileFolder,
    /// An SMB share in a server listing.
    NetworkShare,
    /// Another navigable virtual item, such as a server or a shortcut.
    NetworkLocation,
}

impl FolderType {
    /// The Type column text, in the Python app's wording.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::FileFolder => "File folder",
            Self::NetworkShare => "Network share",
            Self::NetworkLocation => "Network location",
        }
    }
}

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
    /// `standard::is-virtual`: the backend flags the item as virtual.
    pub has_virtual_flag: bool,
}

impl ItemMetadata<'_> {
    /// Shares and shortcuts, and items the backend flags as virtual, stand
    /// for another location rather than being one.
    fn is_virtual(&self) -> bool {
        has_virtual_kind(self.kind) || self.has_virtual_flag
    }
}

/// How an item behaves in the list, independent of its size or dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Classification {
    /// A network share, server or shortcut rather than a real item.
    pub is_virtual: bool,
    /// Can be copied, moved, renamed and trashed.
    pub can_operate: bool,
    /// Where a navigable virtual item leads: the validated backend target,
    /// or the item's own URI when the backend gave none.
    pub target_uri: Option<String>,
    /// What the Type column calls the item: `Some` exactly when it opens as
    /// a folder, and `None` for files, whose type comes from their content
    /// type.
    pub folder_type: Option<FolderType>,
}

impl Classification {
    /// Opens as a folder when activated. Derived from
    /// [`Classification::folder_type`], so the two can never disagree.
    pub(super) fn is_dir(&self) -> bool {
        self.folder_type.is_some()
    }
}

/// Classifies one item from its GIO metadata.
pub(super) fn classify_entry(item: &ItemMetadata<'_>) -> Classification {
    let target = followable_target(item);
    let opens_as_folder = is_navigable(item, target.as_deref());
    let is_virtual = item.is_virtual();
    // A navigable share or server opens its validated target, or itself
    // when the backend named none.
    let target_uri = if opens_as_folder && is_virtual {
        Some(target.unwrap_or_else(|| item.uri.to_owned()))
    } else {
        None
    };
    Classification {
        is_virtual,
        can_operate: has_real_kind(item.kind) && !is_virtual,
        target_uri,
        folder_type: opens_as_folder.then(|| folder_type_of(item)),
    }
}

/// Shares and shortcuts are kinds of item that stand for another location.
fn has_virtual_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Mountable | EntryKind::Shortcut)
}

/// Folders, files and links are the items that copy, move, rename and
/// trash act on; device nodes, shares and shortcuts are not.
fn has_real_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Directory | EntryKind::File | EntryKind::Symlink)
}

/// What the Type column calls a navigable item.
fn folder_type_of(item: &ItemMetadata<'_>) -> FolderType {
    if item.kind == EntryKind::Mountable && is_smb(item.uri) {
        FolderType::NetworkShare
    } else if item.is_virtual() {
        FolderType::NetworkLocation
    } else {
        FolderType::FileFolder
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
    let has_server = !source.authority.is_empty();
    let has_share_name = !source.path.trim_matches('/').is_empty();
    source.scheme == "smb" && has_server && has_share_name
}

#[cfg(test)]
mod tests {
    //! Ported from `v2.0.0:desktop/tests/test_entry_model.py`. Fixtures mirror
    //! gvfsd-smb-browse metadata, not a connection to a real NAS.

    use super::*;
    use EntryKind::{Directory, File, Mountable, Shortcut, Unknown};

    /// An item of `kind` at `uri` for which the backend reported no content
    /// type, no target and no virtual flag. Tests add the metadata they need
    /// by field name, so a content type can never be mistaken for a target.
    fn item(kind: EntryKind, uri: &str) -> ItemMetadata<'_> {
        ItemMetadata {
            kind,
            uri,
            content_type: None,
            target_uri: None,
            has_virtual_flag: false,
        }
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_smb_share_is_navigable_not_a_mutable_regular_directory`
    ///
    /// parity: NET-003
    #[test]
    fn smb_share_is_navigable_not_a_mutable_regular_directory() {
        let share = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            target_uri: Some("smb://NAS/work"),
            ..item(Mountable, "smb://nas/work")
        });
        assert!(share.is_dir());
        assert!(share.is_virtual);
        assert!(!share.can_operate);
        assert_eq!(share.target_uri.as_deref(), Some("smb://nas/work"));
        assert_eq!(share.folder_type, Some(FolderType::NetworkShare));
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_shortcut_server_uses_real_target`
    ///
    /// parity: NET-003
    #[test]
    fn shortcut_server_uses_real_target() {
        let server = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            target_uri: Some("smb://ALPHA/"),
            ..item(Shortcut, "smb://workgroup/ALPHA")
        });
        assert!(server.is_dir());
        assert_eq!(server.target_uri.as_deref(), Some("smb://alpha/"));
        assert_eq!(server.folder_type, Some(FolderType::NetworkLocation));
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_normal_smb_directory`
    ///
    /// parity: NET-003
    #[test]
    fn folder_inside_a_share_is_an_ordinary_operable_folder() {
        let folder = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            ..item(Directory, "smb://nas/work/Design")
        });
        assert!(folder.is_dir());
        assert!(!folder.is_virtual);
        assert!(folder.can_operate);
        assert_eq!(folder.target_uri, None);
        assert_eq!(folder.folder_type, Some(FolderType::FileFolder));
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_local_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn local_directory_is_a_folder() {
        let folder = classify_entry(&item(Directory, "file:///tmp/Test"));
        assert!(folder.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_extensionless_smb_file_is_not_a_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn extensionless_smb_file_is_not_a_folder() {
        let file = classify_entry(&ItemMetadata {
            content_type: Some("text/plain"),
            ..item(File, "smb://nas/work/README")
        });
        assert!(!file.is_dir());
        assert!(file.can_operate);
        assert_eq!(file.folder_type, None);
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_empty_regular_file_with_directory_mime_does_not_become_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn empty_regular_file_with_directory_mime_does_not_become_folder() {
        let file = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            ..item(File, "file:///tmp/file")
        });
        assert!(!file.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_unknown_directory_mime`
    ///
    /// parity: NAV-040
    #[test]
    fn unknown_kind_with_directory_mime_is_a_folder() {
        let folder = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            ..item(Unknown, "smb://nas/work/dir")
        });
        assert!(folder.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_unknown_without_metadata_is_not_falsely_a_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn unknown_without_metadata_is_not_falsely_a_directory() {
        let unknown = classify_entry(&item(Unknown, "smb://nas/work/unknown"));
        assert!(!unknown.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_mountable_share_without_optional_metadata`
    ///
    /// parity: NAV-040, NET-003
    #[test]
    fn mountable_share_without_optional_metadata_is_a_folder() {
        let share = classify_entry(&item(Mountable, "smb://nas/work"));
        assert!(share.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_non_smb_mountable_not_assumed_to_be_directory`
    ///
    /// parity: NAV-040
    #[test]
    fn non_smb_mountable_not_assumed_to_be_directory() {
        let mountable = classify_entry(&item(Mountable, "file:///tmp/thing"));
        assert!(!mountable.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_mtp_directory_with_bracketed_usb_identifier`
    ///
    /// parity: DEV-005
    #[test]
    fn mtp_directory_with_bracketed_usb_identifier_is_an_operable_folder() {
        let folder = classify_entry(&item(Directory, "mtp://[usb:001,010]/Internal%20storage"));
        assert!(folder.is_dir());
        assert!(folder.can_operate);
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_file_shortcut_is_not_a_folder`
    ///
    /// parity: NAV-040
    #[test]
    fn file_shortcut_is_not_a_folder() {
        let shortcut = classify_entry(&ItemMetadata {
            content_type: Some("text/plain"),
            target_uri: Some("file:///tmp/file.txt"),
            ..item(Shortcut, "file:///tmp/shortcut")
        });
        assert!(!shortcut.is_dir());
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_virtual_bad_scheme_is_not_followed`
    ///
    /// parity: NAV-040, SAFE-010
    #[test]
    fn virtual_bad_scheme_is_not_followed() {
        let shortcut = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            target_uri: Some("javascript:alert(1)"),
            ..item(Shortcut, "smb://group/evil")
        });
        assert!(!shortcut.is_dir());
        assert_eq!(shortcut.target_uri, None);
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_credentials_in_backend_target_not_used`
    ///
    /// parity: NAV-040, SAFE-010
    #[test]
    fn credentials_in_backend_target_not_used() {
        let share = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            target_uri: Some("smb://u:secret@nas/work"),
            ..item(Mountable, "smb://nas/work")
        });
        assert!(!share.is_dir());
        assert_eq!(share.target_uri, None);
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_unicode_target_and_spaces`
    ///
    /// parity: NAV-040
    #[test]
    fn unicode_and_spaces_in_a_target_stay_encoded() {
        let share = classify_entry(&ItemMetadata {
            content_type: Some(FOLDER_MIME_TYPE),
            target_uri: Some("smb://nas/Team%20files/%C3%89t%C3%A9"),
            ..item(Mountable, "smb://nas/Team%20files")
        });
        assert_eq!(
            share.target_uri.as_deref(),
            Some("smb://nas/Team%20files/%C3%89t%C3%A9")
        );
    }

    /// Ported from `v2.0.0:desktop/tests/test_entry_model.py::EntryModelTests::test_metadata_does_not_override_a_real_file_target`
    ///
    /// parity: NAV-040
    #[test]
    fn metadata_does_not_override_a_real_file_target() {
        let file = classify_entry(&ItemMetadata {
            content_type: Some("text/plain"),
            target_uri: Some("smb://nas/other"),
            ..item(File, "file:///tmp/file")
        });
        assert_eq!(file.target_uri, None);
        assert!(!file.is_dir());
    }

    #[test]
    fn virtual_flag_blocks_operations_on_real_kinds() {
        let mount = classify_entry(&ItemMetadata {
            has_virtual_flag: true,
            ..item(Directory, "file:///run/media/usb")
        });
        assert!(mount.is_dir());
        assert!(mount.is_virtual);
        assert!(!mount.can_operate);
        assert_eq!(mount.target_uri.as_deref(), Some("file:///run/media/usb"));
        assert_eq!(mount.folder_type, Some(FolderType::NetworkLocation));
    }

    /// The wording is the `description` of `classify_entry` in
    /// `v2.0.0:desktop/entry_model.py`.
    ///
    /// parity: VIEW-002
    #[test]
    fn folder_types_use_the_python_wording() {
        assert_eq!(FolderType::FileFolder.label(), "File folder");
        assert_eq!(FolderType::NetworkShare.label(), "Network share");
        assert_eq!(FolderType::NetworkLocation.label(), "Network location");
    }

    /// parity: NET-003
    #[test]
    fn empty_target_counts_as_missing() {
        let share = classify_entry(&ItemMetadata {
            target_uri: Some(""),
            ..item(Mountable, "smb://nas/work")
        });
        assert!(share.is_dir());
        assert_eq!(share.target_uri.as_deref(), Some("smb://nas/work"));
    }
}
