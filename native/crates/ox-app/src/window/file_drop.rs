// SPDX-License-Identifier: AGPL-3.0-only
//! Dropping files and folders onto the folder views (DND-009, DND-011,
//! DND-012, DND-013).
//!
//! Ports `decode_uris` and `received` of `desktop/native_file_drop.py`
//! and the drop half of `transferWithConflicts` in `desktop/ui/app.js` on
//! GTK's own drop target. Files dropped from another app or window go
//! into the writable folder under the pointer, or into the folder shown
//! when the pointer is over blank space or a file. The drop runs as a
//! paste does: the name-conflict check and dialog first, then the copy on
//! the transfer engine, one operation at a time.
//!
//! Safety rule (DND-009): a drop is always a copy. The target accepts only
//! Copy, so it never moves or deletes the source, and never touches the
//! clipboard. The drag finishes before the conflict dialog opens.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{require_item_uri, LocationError};
use ox_core::transfer::TransferMode;

use super::file_ops::IncomingItems;
use super::BrowserWindow;

/// The most items one drop brings.
const MAX_DROPPED_ITEMS: usize = 200;

/// Why a drop is refused, in the Python app's words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum DropRefusal {
    /// No items, or more than [`MAX_DROPPED_ITEMS`].
    #[error("Drop between 1 and 200 files or folders.")]
    ItemCount,
    /// A link or text rather than local or SMB files.
    #[error("Drop files or folders, rather than links or text.")]
    NotFiles,
    /// An item that is not a file or folder that can be copied, such as a
    /// whole share.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// A file operation runs or is being planned.
    #[error("Finish the current operation before dropping files.")]
    Busy,
    /// Nothing under the pointer takes files: a search, a page, a server
    /// listing or a previous version.
    #[error("Open a writable destination folder before dropping files.")]
    NoDestination,
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
    let is_file_location = |uri: &String| {
        let scheme = uri.split(':').next().unwrap_or_default().to_ascii_lowercase();
        scheme == "file" || scheme == "smb"
    };
    if !uris.iter().all(is_file_location) {
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

impl BrowserWindow {
    /// Lets `view` take files dropped from other windows and apps.
    pub(super) fn attach_file_drop(&self, view: &gtk::Widget) {
        // DND-009: Copy only, so a drop never moves or deletes its source.
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            #[upgrade_or]
            false,
            move |_, value, x, y| {
                let Ok(files) = value.get::<gdk::FileList>() else {
                    return false;
                };
                let uris: Vec<String> = files.files().iter().map(|file| file.uri().to_string()).collect();
                let position = window.folder_pane().owners().position_at(&view, x, y);
                window.drop_files(&uris, position)
            }
        ));
        view.add_controller(target);
    }

    /// Copies the dropped `uris` into the folder at `position`, or into
    /// the folder shown; true when the drop is taken. A refusal shows in
    /// the toast.
    pub(super) fn drop_files(&self, uris: &[String], position: Option<u32>) -> bool {
        match self.incoming_drop(uris, position) {
            Ok(incoming) => {
                // Started after this handler returns, so the drag has
                // finished before the conflict dialog can open.
                glib::spawn_future_local(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    async move {
                        window.transfer_with_conflicts(incoming).await;
                    }
                ));
                true
            }
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                false
            }
        }
    }

    /// The copy a drop of `uris` at `position` asks for.
    fn incoming_drop(&self, uris: &[String], position: Option<u32>) -> Result<IncomingItems, DropRefusal> {
        if !self.imp().file_operations.borrow().is_idle() {
            return Err(DropRefusal::Busy);
        }
        let uris = dropped_uris(uris)?;
        let destination_folder = self
            .drop_destination(position)
            .ok_or(DropRefusal::NoDestination)?;
        Ok(IncomingItems {
            mode: TransferMode::Copy,
            uris,
            destination_folder,
        })
    }

    /// Where a drop at `position` goes: the writable folder there, else
    /// the folder shown when it is writable and not searched (DND-011).
    fn drop_destination(&self, position: Option<u32>) -> Option<String> {
        let model = self.folder_pane().model();
        let locations = self.imp().locations.borrow();
        let folder_under_pointer = position
            .and_then(|position| model.item(position))
            .filter(|item| item.entry().is_dir && !item.entry().is_virtual)
            .map(|item| item.entry().navigation_uri().to_owned())
            .filter(|folder| locations.is_writable_location(folder));
        if folder_under_pointer.is_some() {
            return folder_under_pointer;
        }
        let shown = self.current_uri()?;
        let takes_drops = locations.is_writable_location(&shown) && !model.is_searching();
        takes_drops.then_some(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
