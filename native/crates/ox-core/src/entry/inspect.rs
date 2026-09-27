// SPDX-License-Identifier: AGPL-3.0-only
//! One item on its own, and pinning it to Quick access.
//!
//! Ports `inspect` and `verify_pin` in `desktop/gio_backend.py`. A pin
//! inspects only the dropped item, never its children, so a share need not
//! be mounted to pin the validated target from a server listing.

use gio::prelude::*;

use super::{entry_from_info, Entry, EnumerateError, ATTRIBUTES};
use crate::location::normalise;

/// Why an item cannot be pinned (`verify_pin` in Python).
pub const NOT_PINNABLE: &str = "Only folders and network shares can be pinned to Quick access.";

/// A validated Quick access pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinTarget {
    /// The canonical location to save: a share's or shortcut's real target,
    /// not its server-browser URI.
    pub uri: String,
    /// The label to show: the caller's label, or the item's name.
    pub label: String,
}

/// Queries one item with GIO's synchronous API; call it on a worker
/// thread. `uri` may be any form `normalise_location` accepts (a path, a
/// UNC name, an `smb://` or device URI) and is normalised first.
pub fn inspect(uri: &str, cancellable: Option<&gio::Cancellable>) -> Result<Entry, EnumerateError> {
    let uri = normalise(uri).map_err(|error| EnumerateError::Invalid(error.0))?;
    let file = gio::File::for_uri(&uri);
    let info = file
        .query_info(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, cancellable)
        .map_err(|error| EnumerateError::from_glib(&error))?;
    Ok(entry_from_info(&file, &info))
}

/// Inspects the item at `uri` and returns the pin to save for it. Runs GIO
/// synchronously; call it on a worker thread.
pub fn verify_pin(
    uri: &str,
    label: &str,
    cancellable: Option<&gio::Cancellable>,
) -> Result<PinTarget, EnumerateError> {
    verify_pin_with(uri, label, |uri| inspect(uri, cancellable))
}

/// The pin for an already inspected item: folders, shares and navigable
/// shortcuts only, saved under their validated target.
pub fn pin_target(entry: &Entry, label: &str) -> Result<PinTarget, EnumerateError> {
    if !entry.is_dir {
        return Err(EnumerateError::Invalid(NOT_PINNABLE.to_string()));
    }
    let uri = normalise(entry.navigation_uri()).map_err(|error| EnumerateError::Invalid(error.0))?;
    let label = if label.is_empty() {
        entry.name.clone()
    } else {
        label.to_string()
    };
    Ok(PinTarget { uri, label })
}

/// [`verify_pin`] with the query supplied by the caller, so the flow can be
/// tested without a filesystem.
fn verify_pin_with(
    uri: &str,
    label: &str,
    inspect: impl FnOnce(&str) -> Result<Entry, EnumerateError>,
) -> Result<PinTarget, EnumerateError> {
    let entry = inspect(uri)?;
    pin_target(&entry, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::entry_for_uri;

    fn info(kind: gio::FileType, name: &str, content_type: &str) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_file_type(kind);
        info.set_display_name(name);
        info.set_content_type(content_type);
        info
    }

    fn with_target(info: gio::FileInfo, target: &str) -> gio::FileInfo {
        info.set_attribute_string("standard::target-uri", target);
        info
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_pin_inspects_browse_item_but_saves_real_share_target
    #[test]
    fn pin_inspects_browse_item_but_saves_real_share_target() {
        let mut queried = Vec::new();
        let pin = verify_pin_with("smb://nas/._work", "work", |uri| {
            queried.push(uri.to_string());
            let share = info(gio::FileType::Mountable, "work", "inode/directory");
            Ok(entry_for_uri(uri, &with_target(share, "smb://nas/work")))
        });
        let expected = PinTarget {
            uri: "smb://nas/work".into(),
            label: "work".into(),
        };
        assert_eq!(pin, Ok(expected));
        assert_eq!(queried, ["smb://nas/._work"]);
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_verify_pin_uses_validated_target_not_browse_uri
    #[test]
    fn verify_pin_uses_validated_target_not_browse_uri() {
        let pin = verify_pin_with("smb://group/alpha", "Alpha", |uri| {
            let server = info(gio::FileType::Shortcut, "alpha", "inode/directory");
            Ok(entry_for_uri(uri, &with_target(server, "smb://ALPHA/")))
        });
        assert_eq!(pin.map(|pin| pin.uri).as_deref(), Ok("smb://alpha/"));
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_verify_pin_rejects_regular_file
    #[test]
    fn verify_pin_rejects_regular_file() {
        let pin = verify_pin_with("smb://nas/work/file", "", |uri| {
            Ok(entry_for_uri(
                uri,
                &info(gio::FileType::Regular, "file", "text/plain"),
            ))
        });
        assert_eq!(pin, Err(EnumerateError::Invalid(NOT_PINNABLE.into())));
    }

    #[test]
    fn empty_label_uses_the_item_name() {
        let folder = info(gio::FileType::Directory, "Projects", "inode/directory");
        let entry = entry_for_uri("file:///home/demo/Projects", &folder);
        let pin = pin_target(&entry, "").expect("a folder can be pinned");
        assert_eq!(pin.label, "Projects");
        assert_eq!(pin.uri, "file:///home/demo/Projects");
    }

    #[test]
    fn inspect_refuses_unsupported_addresses_before_any_query() {
        let error = inspect("javascript:alert(1)", None).expect_err("not a location");
        assert_eq!(error.code(), "error");
    }

    #[test]
    fn inspect_reads_a_real_folder() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().to_str().expect("temporary paths are UTF-8");
        let entry = inspect(path, None).expect("the folder exists");
        assert!(entry.is_dir);
        assert_eq!(entry.type_label, "File folder");
        let pin = pin_target(&entry, "").expect("a folder can be pinned");
        assert_eq!(pin.uri, gio::File::for_path(folder.path()).uri());
    }
}
