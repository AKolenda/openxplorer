// SPDX-License-Identifier: AGPL-3.0-only
//! One item on its own, and pinning it to Quick access.
//!
//! Ports `inspect` and `verify_pin` in `desktop/gio_backend.py`. A pin
//! inspects only the dropped item, never its children, so a share need not
//! be mounted to pin the validated target from a server listing.

use gio::prelude::*;

use super::{entry_from_info, Entry, EntryError, ATTRIBUTES};
use crate::location::normalise;

/// A validated Quick access pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinTarget {
    /// The canonical location to save: a share's or shortcut's real target,
    /// not its server-browser URI.
    pub uri: String,
    /// The label to show: the caller's label, or the item's name when the
    /// caller gave none.
    pub label: String,
}

/// Queries one item with GIO's synchronous API; call it on a worker
/// thread. `uri` may be any form [`normalise_location`] accepts (a path, a
/// UNC name, an `smb://` or device URI) and is normalised first.
///
/// [`normalise_location`]: crate::location::normalise_location
///
/// # Errors
///
/// [`EntryError::Invalid`] when `uri` is not a supported location, or the
/// GIO failure sorted by [`EntryError`].
pub fn inspect(uri: &str, cancellable: Option<&gio::Cancellable>) -> Result<Entry, EntryError> {
    let uri = normalise(uri)?;
    let file = gio::File::for_uri(&uri);
    let info = file.query_info(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, cancellable)?;
    Ok(entry_from_info(&file, &info))
}

/// Inspects the item at `uri` and returns the pin to save for it, labelled
/// as [`pin_target`] labels it. Runs GIO synchronously; call it on a worker
/// thread.
///
/// # Errors
///
/// Everything [`inspect`] and [`pin_target`] return.
pub fn verify_pin(
    uri: &str,
    label: Option<&str>,
    cancellable: Option<&gio::Cancellable>,
) -> Result<PinTarget, EntryError> {
    verify_pin_with(uri, label, |uri| inspect(uri, cancellable))
}

/// The pin for an already inspected item: folders, shares and navigable
/// shortcuts only, saved under their validated target.
///
/// Without a `label`, or with an empty one, the pin is labelled with the
/// item's name, as `label or entry['name']` is in Python.
///
/// # Errors
///
/// [`EntryError::NotPinnable`] for a file, and [`EntryError::Invalid`] for
/// a target that is not a location that can be saved.
pub fn pin_target(entry: &Entry, label: Option<&str>) -> Result<PinTarget, EntryError> {
    if !entry.is_dir {
        return Err(EntryError::NotPinnable);
    }
    let uri = normalise(entry.navigation_uri())?;
    let label = label
        .filter(|label| !label.is_empty())
        .map_or_else(|| entry.name.clone(), str::to_owned);
    Ok(PinTarget { uri, label })
}

/// [`verify_pin`] with the query supplied by the caller, so the flow can be
/// tested without a filesystem.
fn verify_pin_with(
    uri: &str,
    label: Option<&str>,
    inspect: impl FnOnce(&str) -> Result<Entry, EntryError>,
) -> Result<PinTarget, EntryError> {
    let entry = inspect(uri)?;
    pin_target(&entry, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::test_support::{
        entry_for_uri, file_info, smb_share_info, with_target, FOLDER_MIME_TYPE,
    };

    /// Ported from `desktop/tests/test_gio_serialization.py::GioSerializationTests::test_pin_inspects_browse_item_but_saves_real_share_target`
    ///
    /// parity: SIDE-007
    #[test]
    fn pin_inspects_browse_item_but_saves_real_share_target() {
        let mut queried = Vec::new();
        let pin = verify_pin_with("smb://nas/._work", Some("work"), |uri| {
            queried.push(uri.to_owned());
            let share = with_target(smb_share_info(), "smb://nas/work");
            Ok(entry_for_uri(uri, &share))
        });
        let expected = PinTarget {
            uri: "smb://nas/work".into(),
            label: "work".into(),
        };
        assert_eq!(pin, Ok(expected));
        assert_eq!(queried, ["smb://nas/._work"]);
    }

    /// Ported from `desktop/tests/test_gio_serialization.py::GioSerializationTests::test_verify_pin_uses_validated_target_not_browse_uri`
    ///
    /// parity: SIDE-007
    #[test]
    fn verify_pin_uses_validated_target_not_browse_uri() {
        let pin = verify_pin_with("smb://group/alpha", Some("Alpha"), |uri| {
            let shortcut = file_info(gio::FileType::Shortcut, "alpha", Some(FOLDER_MIME_TYPE));
            let server = with_target(shortcut, "smb://ALPHA/");
            Ok(entry_for_uri(uri, &server))
        });
        assert_eq!(pin.map(|pin| pin.uri).as_deref(), Ok("smb://alpha/"));
    }

    /// Ported from `desktop/tests/test_gio_serialization.py::GioSerializationTests::test_verify_pin_rejects_regular_file`
    ///
    /// parity: SIDE-007
    #[test]
    fn verify_pin_rejects_regular_file() {
        let pin = verify_pin_with("smb://nas/work/file", None, |uri| {
            let file = file_info(gio::FileType::Regular, "file", Some("text/plain"));
            Ok(entry_for_uri(uri, &file))
        });
        let error = pin.expect_err("a file cannot be pinned");
        assert_eq!(error, EntryError::NotPinnable);
        assert_eq!(
            error.to_string(),
            "Only folders and network shares can be pinned to Quick access."
        );
    }

    /// A local folder named `Projects`, ready to be pinned.
    fn projects_folder() -> Entry {
        let info = file_info(gio::FileType::Directory, "Projects", Some(FOLDER_MIME_TYPE));
        entry_for_uri("file:///home/demo/Projects", &info)
    }

    /// parity: SIDE-007
    #[test]
    fn missing_or_empty_label_uses_the_item_name() {
        let folder = projects_folder();
        for label in [None, Some("")] {
            let pin = pin_target(&folder, label).expect("a folder can be pinned");
            assert_eq!(pin.label, "Projects", "label: {label:?}");
            assert_eq!(pin.uri, "file:///home/demo/Projects");
        }
    }

    /// parity: SIDE-007
    #[test]
    fn given_label_is_kept() {
        let pin = pin_target(&projects_folder(), Some("Work")).expect("a folder can be pinned");
        assert_eq!(pin.label, "Work");
    }

    #[test]
    fn inspect_refuses_unsupported_addresses_before_any_query() {
        let error = inspect("javascript:alert(1)", None).expect_err("not a location");
        assert!(matches!(error, EntryError::Invalid(_)), "{error:?}");
        assert_eq!(error.code(), "error");
    }

    #[test]
    fn inspect_reads_a_real_folder() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().to_str().expect("temporary paths are UTF-8");
        let entry = inspect(path, None).expect("the folder exists");
        assert!(entry.is_dir);
        assert_eq!(entry.type_label, "File folder");
        let pin = pin_target(&entry, None).expect("a folder can be pinned");
        assert_eq!(pin.uri, gio::File::for_path(folder.path()).uri());
    }
}
