// SPDX-License-Identifier: AGPL-3.0-only
//! What a file drag offers: the items' own addresses to this app's
//! windows, and to other apps a file list with a mounted share's items as
//! local files, the paths as text, and KDE's list of the own addresses.
//!
//! Ports `file_uri` and `prepare_files` of `desktop/native_file_drag.py`
//! and `fileDragEntry` of `desktop/ui/app.js`. Only real local and SMB
//! files, folders and links can leave, 1 to 200 of them, and one item that
//! may not refuses the whole drag with the Python app's message.
//!
//! Safety rule (DND-004): only an SMB item consults the mount resolver,
//! which finds a share that is already mounted and never mounts, downloads
//! or signs in during the gesture. A local file is its own path.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{Entry, EntryKind};
use ox_core::location::{file_uri, is_smb_location, is_smb_server, normalise};

/// The most items one drag carries (`MAX_ITEMS`).
pub(super) const MAX_DRAGGED_ITEMS: usize = 200;

/// The most bytes of addresses and paths one drag carries (`MAX_BYTES`).
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// KDE's list of the items' own addresses. Dolphin prefers it to
/// `text/uri-list`, so it gets `smb://` addresses rather than mount paths,
/// as this app's windows do.
const KDE_URI_LIST: &str = "application/x-kde4-urilist";

/// The dragged items' own addresses, offered only inside this process
/// because the type has no MIME name: a window of this app that takes the
/// drop reads these instead of the local paths other apps get (DND-010).
#[derive(Debug, Clone, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "OxDraggedItems")]
pub(crate) struct DraggedItems(pub(crate) Vec<String>);

/// Why items cannot be dragged out, in the Python app's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DragRefusal {
    /// No items, more than [`MAX_DRAGGED_ITEMS`], or an item that may not
    /// leave: a ZIP member, a share listing, a server, a special file.
    #[error("Select up to 200 files or folders. Extract ZIP contents before dragging them.")]
    NotDraggable,
    /// The addresses and paths are over [`MAX_PAYLOAD_BYTES`].
    #[error("This file selection is too large to drag.")]
    TooLarge,
}

/// What one drag offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DragPayload {
    /// The items' own addresses, in order and without duplicates.
    pub(crate) uris: Vec<String>,
    /// What other apps get as `text/uri-list`: a mounted share's items as
    /// local `file://` addresses, the rest as they are.
    pub(crate) exported: Vec<String>,
    /// The local paths, or the address of an item without one, one per
    /// line, for apps that take text.
    pub(crate) text: String,
    /// How many items have no local path, which apps that open only local
    /// files cannot use.
    pub(crate) remote_only: usize,
}

/// Whether the location `uri` may leave the app by a drag: a local or
/// SMB location that is not a whole server (`file_uri`). A share root may:
/// it is a reference to pin.
pub(crate) fn is_draggable_location(uri: &str) -> bool {
    let is_file_location = uri.starts_with("file:") || is_smb_location(uri);
    is_file_location && !is_smb_server(uri)
}

/// Whether `entry` may leave the app by a drag (`fileDragEntry`): a real
/// local or SMB file, folder or link, not a share listing or a shortcut,
/// whose address passes the location rules, which refuse a password or a
/// control character (`file_uri`).
pub(super) fn is_draggable(entry: &Entry) -> bool {
    let is_plain_item = matches!(
        entry.kind,
        EntryKind::File | EntryKind::Directory | EntryKind::Symlink
    );
    let is_valid_address = normalise(&entry.uri).is_ok();
    is_plain_item && !entry.is_virtual && is_valid_address && is_draggable_location(&entry.uri)
}

/// The URIs a drag of `entries` carries: every one, in order, without
/// duplicates.
///
/// # Errors
///
/// [`DragRefusal::NotDraggable`] for no entries, more than
/// [`MAX_DRAGGED_ITEMS`], or any entry that may not leave the app: the
/// whole drag is refused.
pub(super) fn dragged_uris(entries: &[&Entry]) -> Result<Vec<String>, DragRefusal> {
    let within_limit = (1..=MAX_DRAGGED_ITEMS).contains(&entries.len());
    if !within_limit || !entries.iter().all(|entry| is_draggable(entry)) {
        return Err(DragRefusal::NotDraggable);
    }
    let mut uris: Vec<String> = Vec::with_capacity(entries.len());
    for entry in entries {
        if !uris.contains(&entry.uri) {
            uris.push(entry.uri.clone());
        }
    }
    Ok(uris)
}

/// How other apps get one dragged item.
#[derive(Debug)]
struct ExportedItem {
    /// Its address in the file list: its own for a local item, the local
    /// file of a mounted share's item, else its `smb://` address.
    uri: String,
    /// Its line in the text: its local path, or its address without one.
    line: String,
    /// It has no local path.
    is_remote_only: bool,
}

/// How other apps get the item at `uri`; `mount_path` finds an SMB item's
/// path in a share that is already mounted, and is never asked about a
/// local item (DND-004).
fn export(uri: &str, mount_path: &impl Fn(&str) -> Option<PathBuf>) -> ExportedItem {
    let is_smb = is_smb_location(uri);
    let path = if is_smb {
        mount_path(uri)
    } else {
        gio::File::for_uri(uri).path()
    };
    let Some(path) = path else {
        return ExportedItem {
            uri: uri.to_owned(),
            line: uri.to_owned(),
            is_remote_only: true,
        };
    };
    let exported_uri = if is_smb { file_uri(&path) } else { uri.to_owned() };
    ExportedItem {
        uri: exported_uri,
        line: path.to_string_lossy().into_owned(),
        is_remote_only: false,
    }
}

impl DragPayload {
    /// What a drag of `uris`, as [`dragged_uris`] gives them, offers.
    /// `mount_path` finds an SMB item's path in a mounted share.
    ///
    /// # Errors
    ///
    /// [`DragRefusal::TooLarge`] when the addresses and paths are over
    /// 1 MiB together.
    pub(crate) fn new(
        uris: Vec<String>,
        mount_path: impl Fn(&str) -> Option<PathBuf>,
    ) -> Result<Self, DragRefusal> {
        let items: Vec<ExportedItem> = uris.iter().map(|uri| export(uri, &mount_path)).collect();
        let own_bytes: usize = uris.iter().map(String::len).sum();
        let exported_bytes: usize = items.iter().map(|item| item.uri.len() + item.line.len()).sum();
        if own_bytes + exported_bytes > MAX_PAYLOAD_BYTES {
            return Err(DragRefusal::TooLarge);
        }
        let lines: Vec<&str> = items.iter().map(|item| item.line.as_str()).collect();
        Ok(Self {
            text: lines.join("\n"),
            remote_only: items.iter().filter(|item| item.is_remote_only).count(),
            exported: items.into_iter().map(|item| item.uri).collect(),
            uris,
        })
    }

    /// The drag's content: the items' own addresses for this process, a
    /// file list (which GTK serves as `text/uri-list`, the desktop portal's
    /// formats and text), and KDE's address list.
    pub(crate) fn content(&self) -> gdk::ContentProvider {
        let own = gdk::ContentProvider::for_value(&DraggedItems(self.uris.clone()).to_value());
        let files: Vec<gio::File> = self.exported.iter().map(|uri| gio::File::for_uri(uri)).collect();
        let file_list = gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value());
        let kde_list = glib::Bytes::from_owned(self.uris.join("\r\n").into_bytes());
        let kde = gdk::ContentProvider::for_bytes(KDE_URI_LIST, &kde_list);
        let text = gdk::ContentProvider::for_value(&self.text.to_value());
        // Text first: GTK would otherwise serve the file list's own text.
        gdk::ContentProvider::new_union(&[own, text, file_list, kde])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    /// A resolver that finds no mounted share.
    fn no_mounts(_: &str) -> Option<PathBuf> {
        None
    }

    /// parity: DND-002, DND-003, DND-005
    #[test]
    fn a_drag_carries_every_selected_item_once_in_order() {
        let folder = folder_entry("Projects (2024) & more");
        let file = file_entry("Budget +20%.xlsx");

        let uris = dragged_uris(&[&folder, &file, &folder]);

        assert_eq!(uris, Ok(vec![folder.uri.clone(), file.uri.clone()]));
    }

    /// parity: DND-003
    #[test]
    fn one_item_that_may_not_leave_refuses_the_whole_drag() {
        let file = file_entry("notes.txt");
        let mut share = folder_entry("share");
        share.is_virtual = true;
        let mut socket = file_entry("socket");
        socket.kind = EntryKind::Special;
        let many: Vec<Entry> = (0..=MAX_DRAGGED_ITEMS)
            .map(|number| file_entry(&format!("file {number}")))
            .collect();
        let many_refs: Vec<&Entry> = many.iter().collect();

        assert_eq!(dragged_uris(&[&file, &share]), Err(DragRefusal::NotDraggable));
        assert_eq!(dragged_uris(&[&socket]), Err(DragRefusal::NotDraggable));
        let mut with_password = file_entry("a");
        with_password.uri = "smb://ada:secret@nas/Projects/a".to_owned();
        assert_eq!(dragged_uris(&[&with_password]), Err(DragRefusal::NotDraggable));
        assert_eq!(dragged_uris(&[]), Err(DragRefusal::NotDraggable));
        assert_eq!(dragged_uris(&many_refs), Err(DragRefusal::NotDraggable));
        assert_eq!(
            DragRefusal::NotDraggable.to_string(),
            "Select up to 200 files or folders. Extract ZIP contents before dragging them."
        );
    }

    /// parity: DND-003
    #[test]
    fn only_local_and_smb_locations_below_a_server_may_leave() {
        assert!(is_draggable_location("file:///home/ada/Documents"));
        assert!(is_draggable_location("smb://nas/projects"));
        assert!(!is_draggable_location("smb://nas/"));
        assert!(!is_draggable_location("mtp://phone/DCIM"));
        assert!(!is_draggable_location("trash:///"));
    }

    /// Ported from `desktop/tests/test_native_file_drag.py::PayloadTests::test_existing_smb_mount_exports_local_path_and_keeps_original`
    /// and `test_unmounted_smb_remains_a_uri_without_implicit_download`.
    ///
    /// parity: DND-004, NET-026
    #[test]
    fn a_mounted_share_exports_its_local_path_and_keeps_its_own_address() {
        let mounted = "smb://nas/projects/plan.odt".to_owned();
        let unmounted = "smb://other/share/a.txt".to_owned();
        let mount_path = |uri: &str| {
            let mounted_path = "/run/user/1000/gvfs/smb-share:server=nas,share=projects/plan.odt";
            (uri == "smb://nas/projects/plan.odt").then(|| PathBuf::from(mounted_path))
        };

        let payload =
            DragPayload::new(vec![mounted.clone(), unmounted.clone()], mount_path).expect("two items fit");

        assert_eq!(payload.uris, [mounted, unmounted.clone()]);
        assert_eq!(
            payload.exported,
            [
                "file:///run/user/1000/gvfs/smb-share%3Aserver%3Dnas%2Cshare%3Dprojects/plan.odt".to_owned(),
                unmounted.clone(),
            ]
        );
        assert_eq!(payload.remote_only, 1);
        assert_eq!(
            payload.text,
            format!("/run/user/1000/gvfs/smb-share:server=nas,share=projects/plan.odt\n{unmounted}")
        );
    }

    /// Ported from `desktop/tests/test_native_file_drag.py::PayloadTests::test_local_files_never_consult_mount_resolver`.
    ///
    /// parity: DND-004
    #[test]
    fn local_files_never_consult_the_mount_resolver() {
        let uri = file_entry("Résumé (1).txt").uri;
        let never = |_: &str| -> Option<PathBuf> { panic!("a local file has its own path") };

        let payload = DragPayload::new(vec![uri.clone()], never).expect("one item fits");

        assert_eq!(payload.exported, [uri]);
        assert_eq!(payload.text, "/tmp/ox-test/Résumé (1).txt");
        assert_eq!(payload.remote_only, 0);
    }

    /// Ported from `desktop/tests/test_native_file_drag.py::PayloadTests::test_rejects_oversized_individual_uri_and_total_payload`.
    ///
    /// parity: DND-003
    #[test]
    fn a_selection_over_one_mebibyte_of_addresses_is_refused() {
        let long_name = "x".repeat(4000);
        let uris: Vec<String> = (0..MAX_DRAGGED_ITEMS)
            .map(|number| format!("smb://nas/share/{long_name}{number}"))
            .collect();

        let payload = DragPayload::new(uris, no_mounts);

        assert_eq!(payload, Err(DragRefusal::TooLarge));
        assert_eq!(
            DragRefusal::TooLarge.to_string(),
            "This file selection is too large to drag."
        );
    }
}
