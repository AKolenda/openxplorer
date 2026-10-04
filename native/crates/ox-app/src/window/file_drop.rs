// SPDX-License-Identifier: AGPL-3.0-only
//! Dropping files and folders onto the window (DND-009 to DND-014,
//! DND-017 to DND-021, DND-025, DND-026, TAB-018).
//!
//! Ports `decode_uris` and `NativeFileDrop` of
//! `v2.0.0:desktop/native_file_drop.py` and `receiveFileDrop` and
//! `showFileDropHint` of `v2.0.0:desktop/ui/app.js` on GTK's asynchronous drop
//! target. The folder views, the sidebar, the breadcrumbs and the tabs
//! take drops ([`targets`]). Where the items go is a [`DropDestination`]:
//! a folder, Quick access, a program or launcher ([`program`],
//! [`launcher`]), or the Recycle Bin, which moves them to the Trash as
//! Delete does (OPS-045). Items dragged out of the Recycle Bin into a
//! folder are always moved there (OPS-046). What happens to them in a folder is a [`DropAction`]
//! ([`action`]): copy, move, link, or the drop menu that asks.
//!
//! Safety rules:
//! - "The source never deletes" (DND-009): every drop is finished as soon
//!   as its items are read and before anything runs, as a copy whenever
//!   the source allows one, so a source is never told to delete what it
//!   offered. A move is done by the window's own transfer engine, with
//!   all its rules.
//! - "A plain drop copies": without a modifier a drop copies, as the
//!   Python app always did; Shift moves and Ctrl+Shift links only when the
//!   user holds them ([`action`]).
//! - The drag has finished before any dialog or menu opens, and a drop
//!   never touches the clipboard.
//! - Items dragged out of a ZIP arrive as copies that are removed after a
//!   day, so they are never linked to (ARC-026).

mod action;
mod launcher;
mod program;
mod targets;

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{parent_location, require_item_uri, same_location, LocationError, TRASH_URI};
use ox_core::ops::{is_recycle_bin_item, LinkRequest};
use ox_core::transfer::TransferMode;

use super::activation::query_entry;
use super::file_drag::{has_open_popover, DraggedItems};
use super::file_ops::IncomingItems;
use super::session::{TabPlacement, TabPosition};
use super::zip_copies::{is_zip_copy, ZipDragContent};
use super::BrowserWindow;

pub(crate) use action::{DropAction, FirstOffer, PendingDrop};
pub(super) use program::{query_program, ProgramChecks, ProgramTarget};
pub(super) use targets::{DragScroll, DropZone};

/// The most items one drop brings.
const MAX_DROPPED_ITEMS: usize = 200;

/// How long the window waits for a drop's items (`timeout_add_seconds(10,
/// ...)` in `native_file_drop.py`).
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The actions a finished drop may report, safest first.
const FINISH_ACTIONS: [gdk::DragAction; 3] = [
    gdk::DragAction::COPY,
    gdk::DragAction::MOVE,
    gdk::DragAction::LINK,
];

/// Where dropped items go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DropDestination {
    /// Into this folder.
    Folder(String),
    /// Into the root of the volume with this identifier, once it is
    /// mounted (DEV-010).
    Volume(String),
    /// Pinned to Quick access before the pin at this location, or at the
    /// end (DND-014).
    QuickAccess {
        /// The pin the new pins go before.
        before: Option<String>,
    },
    /// Given to this program to open (DND-026).
    Program(ProgramTarget),
    /// Moved to the Trash: a drop on the Recycle Bin (OPS-045).
    RecycleBin,
    /// Each folder opened in a new tab at the end, behind the active one:
    /// a drop on the tab strip beside the tabs (TAB-018). Files are left
    /// alone, as in Dolphin.
    NewTabs,
}

impl DropDestination {
    /// A drop into `folder`: the Recycle Bin for its root, else the folder.
    pub(crate) fn for_folder(folder: String) -> Self {
        if same_location(&folder, TRASH_URI) {
            DropDestination::RecycleBin
        } else {
            DropDestination::Folder(folder)
        }
    }
}

/// Why a drop is refused, in the Python app's words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DropRefusal {
    /// No items, or more than [`MAX_DROPPED_ITEMS`].
    #[error("{}", ox_core::i18n::gettext("Drop between 1 and 200 files or folders."))]
    ItemCount,
    /// A link or text rather than local or SMB files.
    #[error(
        "{}",
        ox_core::i18n::gettext("Drop files or folders, rather than links or text.")
    )]
    NotFiles,
    /// A whole server, or anything but a local or SMB location, dropped on
    /// Quick access (`receiveFileDrop`).
    #[error(
        "{}",
        ox_core::i18n::gettext("Drop up to 200 local files or connected network items.")
    )]
    NotPinnable,
    /// An item that is not a file or folder that can be copied, such as a
    /// whole share.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// A file operation runs or is being planned.
    #[error(
        "{}",
        ox_core::i18n::gettext("Finish the current operation before dropping files.")
    )]
    Busy,
    /// A dialog, sign-in prompt or menu is open.
    #[error(
        "{}",
        ox_core::i18n::gettext("Close the dialog and finish the current operation before dropping files.")
    )]
    DialogOpen,
    /// Nothing under the pointer takes files: a search, a page, a server
    /// listing or a previous version.
    #[error(
        "{}",
        ox_core::i18n::gettext("Open a writable destination folder before dropping files.")
    )]
    NoDestination,
    /// A folder dropped onto itself.
    #[error("{}", ox_core::i18n::gettext("A folder cannot be copied into itself."))]
    IntoItself,
    /// The tab moved to another folder while the items were read.
    #[error(
        "{}",
        ox_core::i18n::gettext("The destination changed. Drop the files again.")
    )]
    DestinationChanged,
    /// The items could not be read in time, or at all.
    #[error("{}", ox_core::i18n::gettext("The file drop is empty or too large."))]
    Unreadable,
    /// Recycle Bin items dropped on the Recycle Bin.
    #[error("{}", ox_core::i18n::gettext("These items are in the Recycle Bin already."))]
    InRecycleBin,
    /// Recycle Bin items dropped together with other items.
    #[error(
        "{}",
        ox_core::i18n::gettext("Drag items out of the Recycle Bin on their own.")
    )]
    MixedWithRecycleBin,
    /// Links asked for copies taken out of a ZIP, which are removed after
    /// a day (ARC-026).
    #[error(
        "{}",
        ox_core::i18n::gettext("Items from a ZIP cannot be linked. Copy or move them instead.")
    )]
    LinkToZipCopy,
    /// The copies of items dragged out of a ZIP could not be made: the
    /// reason, already translated.
    #[error("{0}")]
    NotCopied(String),
}

/// The items of a drop of `uris`: canonical, in order and without
/// duplicates (`decode_uris`).
///
/// # Errors
///
/// [`DropRefusal::ItemCount`], [`DropRefusal::NotFiles`] for anything but
/// `file:` and `smb:` URIs, or [`DropRefusal::Location`] for an item that
/// cannot be copied.
pub(super) fn dropped_uris(uris: &[String]) -> Result<Vec<String>, DropRefusal> {
    if !(1..=MAX_DROPPED_ITEMS).contains(&uris.len()) {
        return Err(DropRefusal::ItemCount);
    }
    if !uris.iter().all(|uri| is_file_location(uri)) {
        return Err(DropRefusal::NotFiles);
    }
    let mut items: Vec<String> = Vec::with_capacity(uris.len());
    for uri in uris {
        let item = require_item_uri(uri)?;
        if !items.contains(&item) {
            items.push(item);
        }
    }
    Ok(items)
}

/// True for a `file:` or `smb:` URI.
fn is_file_location(uri: &str) -> bool {
    let scheme = uri.split(':').next().unwrap_or_default().to_ascii_lowercase();
    scheme == "file" || scheme == "smb"
}

/// The items of `uris` that are not in `folder` already: moving an item
/// into the folder it is in has nothing to do.
fn outside_folder(uris: Vec<String>, folder: &str) -> Vec<String> {
    uris.into_iter()
        .filter(|uri| !parent_location(uri).is_some_and(|parent| same_location(&parent, folder)))
        .collect()
}

/// The action `drop` is finished with: a copy whenever the drag offers
/// one, else the action it settled on, which is never "ask" (a drop
/// finished with "ask" breaks the Wayland protocol).
fn finish_action(drop: &gdk::Drop) -> gdk::DragAction {
    let offered = drop.actions();
    FINISH_ACTIONS
        .into_iter()
        .find(|action| offered.contains(*action))
        .unwrap_or(gdk::DragAction::COPY)
}

/// The items this process's own drag carries, when `drop` comes from one
/// of its windows: their own addresses, never the local mount paths other
/// apps get (DND-010).
fn own_dragged_items(drop: &gdk::Drop) -> Option<Vec<String>> {
    let content = drop.drag()?.content();
    let value = content.value(DraggedItems::static_type()).ok()?;
    let items = value.get::<DraggedItems>().ok()?;
    Some(items.0)
}

/// The content of `drop` when it is a drag of this process out of a ZIP
/// opened like a folder (ARC-026).
fn own_zip_drag(drop: &gdk::Drop) -> Option<ZipDragContent> {
    drop.drag()?.content().downcast().ok()
}

/// The URIs `drop` brings: its own items for a drag of this process, else
/// its file list, waiting at most [`READ_TIMEOUT`].
async fn read_dropped_uris(drop: &gdk::Drop) -> Result<Vec<String>, DropRefusal> {
    if let Some(own) = own_dragged_items(drop) {
        return Ok(own);
    }
    let reading = drop.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT);
    let value = within(READ_TIMEOUT, reading).await?;
    let files = value
        .get::<gdk::FileList>()
        .map_err(|_| DropRefusal::Unreadable)?;
    Ok(files.files().iter().map(|file| file.uri().to_string()).collect())
}

/// The value `reading` gives within `timeout`: a drop whose data is late
/// or fails is [`DropRefusal::Unreadable`], and data arriving after the
/// timeout is never used.
async fn within<T>(
    timeout: Duration,
    reading: impl std::future::Future<Output = Result<T, glib::Error>>,
) -> Result<T, DropRefusal> {
    glib::future_with_timeout(timeout, reading)
        .await
        .map_err(|_| DropRefusal::Unreadable)?
        .map_err(|_| DropRefusal::Unreadable)
}

impl BrowserWindow {
    /// Lets the sidebar, the breadcrumbs and the tabs take dropped files,
    /// and the sidebar's folders be dragged out. The folder views get
    /// theirs with the rest of their input.
    pub(super) fn connect_drag_and_drop(&self) {
        self.attach_file_drop_zone(self.sidebar().list(), DropZone::Sidebar);
        self.attach_file_drop_zone(self.address_bar(), DropZone::Breadcrumbs);
        self.attach_file_drop_zone(self.tab_strip(), DropZone::Tabs);
        self.attach_sidebar_file_drag();
    }

    /// Runs `action` on the dropped `uris` in the folder at `position` of
    /// the folder view, or in the folder shown, as a drop there does once
    /// its items are read; true when the drop is taken. For tests, which
    /// have no pointer to drag with.
    #[cfg(test)]
    pub(super) fn drop_files(&self, uris: &[String], position: Option<u32>, action: DropAction) -> bool {
        let destination = self.folder_view_destination(position);
        self.complete_drop(uris, destination, action)
    }

    /// Reads the items of `drop`, which is going to `destination`,
    /// finishes it, then runs `action` on them.
    async fn receive_drop(&self, drop: gdk::Drop, destination: DropDestination, action: DropAction) {
        let shown = self.current_uri();
        if let Some(content) = own_zip_drag(&drop) {
            // Items dragged out of a ZIP in this app: nothing more is read
            // from the drop, and their copies are made without the
            // timeout, as a large member can take longer to extract.
            drop.finish(finish_action(&drop));
            let read = content.copies().await.map_err(DropRefusal::NotCopied);
            self.take_read_drop(shown.as_deref(), read, destination, action);
            return;
        }
        let read = read_dropped_uris(&drop).await;
        // Safety rule "the source never deletes": finished, as a copy when
        // the source allows one, or refused, before anything runs.
        let finished_as = if read.is_ok() {
            finish_action(&drop)
        } else {
            gdk::DragAction::empty()
        };
        drop.finish(finished_as);
        self.take_read_drop(shown.as_deref(), read, destination, action);
    }

    /// Runs `action` on the items a drop read, unless reading failed or
    /// the tab left `shown`, the folder it showed when the drop began;
    /// true when the drop is taken.
    fn take_read_drop(
        &self,
        shown: Option<&str>,
        read: Result<Vec<String>, DropRefusal>,
        destination: DropDestination,
        action: DropAction,
    ) -> bool {
        let uris = match read {
            Ok(uris) => uris,
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                return false;
            }
        };
        if self.current_uri().as_deref() != shown {
            self.show_message(&DropRefusal::DestinationChanged.to_string());
            return false;
        }
        if matches!(destination, DropDestination::Volume(_)) {
            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = window)]
                self,
                async move { window.deliver_drop(&uris, destination, action).await }
            ));
            return true;
        }
        self.complete_drop(&uris, Some(destination), action)
    }

    /// Sends the dropped `uris` to `destination`, mounting a volume first.
    pub(super) async fn deliver_drop(
        &self,
        uris: &[String],
        destination: DropDestination,
        action: DropAction,
    ) {
        let destination = match destination {
            DropDestination::Volume(id) => match self.mount_for_drop(&id).await {
                Some(root) => DropDestination::Folder(root),
                None => return,
            },
            other => other,
        };
        self.complete_drop(uris, Some(destination), action);
    }

    /// Sends the dropped `uris` to `destination` with `action`; true when
    /// the drop is taken. A refusal shows in the toast.
    fn complete_drop(
        &self,
        uris: &[String],
        destination: Option<DropDestination>,
        action: DropAction,
    ) -> bool {
        match self.check_ready() {
            Ok(()) => self.run_drop(uris, destination, action),
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                false
            }
        }
    }

    /// [`Self::complete_drop`] with the drop menu's answer, while the
    /// menu may still be closing.
    fn run_drop(&self, uris: &[String], destination: Option<DropDestination>, action: DropAction) -> bool {
        let outcome = self.check_idle().and_then(|()| {
            let destination = destination.ok_or(DropRefusal::NoDestination)?;
            self.send_dropped_items(uris, destination, action)
        });
        match outcome {
            Ok(()) => true,
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                false
            }
        }
    }

    /// Refuses a drop while a file operation runs or is being planned.
    fn check_idle(&self) -> Result<(), DropRefusal> {
        if self.imp().file_operations.borrow().is_idle() {
            Ok(())
        } else {
            Err(DropRefusal::Busy)
        }
    }

    /// Refuses a drop while a file operation runs or is being planned, or
    /// while a dialog, sign-in prompt or menu is open, as drags are held
    /// back then too (DND-006). The in-window dialogs leave the tab strip
    /// usable, so a drop on a tab could otherwise slip past them.
    fn check_ready(&self) -> Result<(), DropRefusal> {
        self.check_idle()?;
        let dialog_open = self.dialog_layer().shown().is_some()
            || self.has_open_dialog()
            || has_open_popover(self.upcast_ref());
        if dialog_open {
            Err(DropRefusal::DialogOpen)
        } else {
            Ok(())
        }
    }

    /// Sends `uris` to `destination` with `action`.
    fn send_dropped_items(
        &self,
        uris: &[String],
        destination: DropDestination,
        action: DropAction,
    ) -> Result<(), DropRefusal> {
        match destination {
            DropDestination::Folder(folder) => self.drop_into_folder(uris, folder, action),
            // `deliver_drop` mounts the volume and sends its root instead.
            DropDestination::Volume(_) => Err(DropRefusal::NoDestination),
            DropDestination::QuickAccess { before } => self.pin_dropped(uris, before),
            DropDestination::Program(program) => {
                let items = dropped_uris(uris)?;
                self.open_with_program(program, items);
                Ok(())
            }
            DropDestination::NewTabs => {
                let items = dropped_uris(uris)?;
                glib::spawn_future_local(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    async move { window.open_dropped_folders(items).await }
                ));
                Ok(())
            }
            DropDestination::RecycleBin => {
                if uris.iter().any(|uri| is_recycle_bin_item(uri)) {
                    return Err(DropRefusal::InRecycleBin);
                }
                let items = dropped_uris(uris)?;
                glib::spawn_future_local(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    async move { window.trash_dropped(items).await }
                ));
                Ok(())
            }
        }
    }

    /// Copies, moves or links `uris` into `folder`, or asks which. Items
    /// moved into the folder they are in are left alone.
    fn drop_into_folder(
        &self,
        uris: &[String],
        folder: String,
        action: DropAction,
    ) -> Result<(), DropRefusal> {
        if uris.iter().any(|uri| is_recycle_bin_item(uri)) {
            return self.drop_from_recycle_bin(uris, folder);
        }
        let uris = dropped_uris(uris)?;
        if uris.iter().any(|uri| same_location(uri, &folder)) {
            return Err(DropRefusal::IntoItself);
        }
        let mode = match action {
            DropAction::Copy => TransferMode::Copy,
            DropAction::Move => TransferMode::Move,
            DropAction::Link if uris.iter().any(|uri| is_zip_copy(uri)) => {
                return Err(DropRefusal::LinkToZipCopy);
            }
            DropAction::Link => {
                self.spawn_links(LinkRequest {
                    uris,
                    destination_folder: folder,
                });
                return Ok(());
            }
            DropAction::Ask => {
                self.ask_drop_action(uris, folder);
                return Ok(());
            }
        };
        let uris = if mode == TransferMode::Move {
            outside_folder(uris, &folder)
        } else {
            uris
        };
        if !uris.is_empty() {
            self.spawn_transfer(IncomingItems {
                mode,
                uris,
                destination_folder: folder,
            });
        }
        Ok(())
    }

    /// Moves the Recycle Bin items `uris` into `folder`, whatever the
    /// drop's action, as Dolphin does (OPS-046, DND-018).
    fn drop_from_recycle_bin(&self, uris: &[String], folder: String) -> Result<(), DropRefusal> {
        if !(1..=MAX_DROPPED_ITEMS).contains(&uris.len()) {
            return Err(DropRefusal::ItemCount);
        }
        if !uris.iter().all(|uri| is_recycle_bin_item(uri)) {
            return Err(DropRefusal::MixedWithRecycleBin);
        }
        let uris = uris.to_vec();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.move_out_of_recycle_bin(uris, folder).await }
        ));
        Ok(())
    }

    /// Opens each of `items` that is a folder in a new tab at the end,
    /// behind the active one, in their order (`tabDropEvent` in Dolphin).
    pub(super) async fn open_dropped_folders(&self, items: Vec<String>) {
        for uri in items {
            let is_folder = query_entry(&uri).await.is_ok_and(|entry| entry.is_dir);
            if !is_folder {
                continue;
            }
            if let Err(error) = self.open_tab_at(&uri, TabPlacement::Background, TabPosition::End) {
                self.show_message(&error.to_string());
            }
        }
    }

    /// Runs `incoming` through the conflict check and the transfer engine
    /// once the drop handler has returned, so the drag has finished before
    /// the conflict dialog can open.
    fn spawn_transfer(&self, incoming: IncomingItems) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window.transfer_with_conflicts(incoming).await;
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{Fixture, TestWindow};

    fn uris(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    /// parity: DND-012
    #[test]
    fn a_drop_takes_local_and_smb_items_once_in_order() {
        let dropped = uris(&[
            "file:///tmp/a%20b.txt",
            "smb://nas/share/report.pdf",
            "file:///tmp/a%20b.txt",
        ]);

        let items = dropped_uris(&dropped);

        assert_eq!(
            items,
            Ok(uris(&["file:///tmp/a%20b.txt", "smb://nas/share/report.pdf"]))
        );
    }

    /// parity: DND-012
    #[test]
    fn links_text_and_whole_shares_are_not_dropped() {
        let web = dropped_uris(&uris(&["https://example.com/file.txt"]));
        let too_many = dropped_uris(&vec!["file:///tmp/a".to_owned(); MAX_DROPPED_ITEMS + 1]);
        let share = dropped_uris(&uris(&["smb://nas/share"]));

        assert_eq!(web, Err(DropRefusal::NotFiles));
        assert_eq!(too_many, Err(DropRefusal::ItemCount));
        assert_eq!(dropped_uris(&[]), Err(DropRefusal::ItemCount));
        assert!(matches!(share, Err(DropRefusal::Location(_))), "{share:?}");
        assert_eq!(
            DropRefusal::NotFiles.to_string(),
            "Drop files or folders, rather than links or text."
        );
    }

    /// parity: DND-017
    #[test]
    fn moving_items_into_the_folder_they_are_in_leaves_them_alone() {
        let dropped = uris(&["file:///tmp/a/one.txt", "file:///tmp/b/two.txt"]);

        let moved = outside_folder(dropped, "file:///tmp/a");

        assert_eq!(moved, uris(&["file:///tmp/b/two.txt"]));
    }

    /// parity: DND-013
    #[gtk::test]
    fn a_drop_read_after_the_tab_moved_or_too_late_is_refused() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let destination = DropDestination::Folder(fixture.uri());
        let items = vec![fixture.uri_of("Notes 2.txt")];

        let moved = test.window.take_read_drop(
            Some(&fixture.uri_of("Documents")),
            Ok(items),
            destination,
            DropAction::Copy,
        );
        let late = glib::MainContext::default().block_on(within(
            Duration::from_millis(50),
            std::future::pending::<Result<(), glib::Error>>(),
        ));

        assert!(!moved);
        assert_eq!(
            test.window.shown_message(),
            "The destination changed. Drop the files again."
        );
        assert_eq!(late, Err(DropRefusal::Unreadable));
        assert_eq!(READ_TIMEOUT, Duration::from_secs(10));
    }
}
