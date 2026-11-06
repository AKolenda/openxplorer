// SPDX-License-Identifier: AGPL-3.0-only
//! Delete and Shift+Delete, with their confirmations (OPS-015, OPS-016,
//! OPS-017, OPS-018).
//!
//! Ports `trash` in `v2.0.0:desktop/ui/app.js`, behaving as Windows 11's File
//! Explorer does. Delete decides each item by its own folder: where the
//! folder has a Trash the item goes to the Recycle Bin, elsewhere it is
//! deleted permanently (a folder GIO cannot be asked about counts as
//! having a Trash, so a failed check never becomes a permanent delete). As
//! in Windows, Delete moves items to the Recycle Bin without asking unless
//! the settings say to ask (SET-010), and a permanent delete asks first.
//! The Recycle Bin items run first, then the others, each as an operation
//! of its own. Shift+Delete, which the Python app did not have, deletes
//! the selection permanently after its own confirmation.
//!
//! The confirmations are Windows Explorer's: "Delete File", "Delete
//! Folder" or "Delete Multiple Items", "Are you sure you want to
//! permanently delete this file?" (or to move it to the Recycle Bin), with
//! one item's picture, name, Type, Size and Date modified (a folder's Date
//! created) under the question, and Yes, which has focus, and No.
//! Afterwards the item that followed the removed ones is selected, as in
//! Dolphin, so Delete can be pressed again (SEL-017). In the Recycle Bin,
//! both delete the selected items for good ([`super::recycle_bin`]). Items
//! dropped on the Recycle Bin go the way of Delete (OPS-045).

use std::time::Duration;

use gtk::gio::prelude::*;
use gtk::prelude::*;
#[cfg(test)]
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{entry_from_info, Entry, EntryKind, ATTRIBUTES};
use ox_core::i18n::{format_message, gettext};
use ox_core::ops::{
    permanent_delete_confirmation, plan_delete, DeleteConfirmation, DeleteItem, DeleteSubject,
    TransferRequest,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use super::FileCommand;
use crate::dialog::Dialog;
use crate::icons::{Art, ArtImage};
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// How long a confirmation waits for its one item's details before it
/// asks without them, so a slow share never holds the question back.
const DETAILS_TIMEOUT: Duration = Duration::from_secs(2);

/// The size of the item's picture beside its details.
const DETAILS_ART_SIZE: i32 = 48;

/// `uris` with the names their confirmations show.
fn named_items(uris: Vec<String>) -> Vec<DeleteItem> {
    uris.into_iter()
        .map(|uri| {
            let name = gio::File::for_uri(&uri)
                .basename()
                .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
            DeleteItem {
                uri,
                name,
                can_trash: None,
            }
        })
        .collect()
}

/// `items`' one item as GIO reports it now, for the details its
/// confirmation shows; `None` for several items, and when the item cannot
/// be read in [`DETAILS_TIMEOUT`]. A link is read as itself, as Delete
/// removes the link.
async fn single_entry(items: &[DeleteItem]) -> Option<Entry> {
    let [item] = items else {
        return None;
    };
    let file = gio::File::for_uri(&item.uri);
    let query = file.query_info_future(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        glib::Priority::DEFAULT,
    );
    let info = glib::future_with_timeout(DETAILS_TIMEOUT, query)
        .await
        .ok()?
        .ok()?;
    Some(entry_from_info(&file, &info))
}

/// Whether `entry` is a folder, which Windows asks about as "this folder".
fn is_folder(entry: &Entry) -> bool {
    entry.kind == EntryKind::Directory
}

/// The lines Windows' Delete File dialog shows under an item's name: a
/// file's Type, Size and Date modified, or a folder's Date created when
/// the backend reports it.
fn detail_lines(entry: &Entry) -> Vec<String> {
    if is_folder(entry) {
        return entry
            .meta
            .created
            .map(|created| {
                format_message(
                    "Date created: {date}",
                    &[("date", &ox_core::format::date_short_time_text(Some(created)))],
                )
            })
            .into_iter()
            .collect();
    }
    vec![
        format_message("Type: {type}", &[("type", &entry.type_label)]),
        format_message(
            "Size: {size}",
            &[("size", &ox_core::format::size_text(entry.size))],
        ),
        format_message(
            "Date modified: {date}",
            &[("date", &ox_core::format::date_short_time_text(entry.modified))],
        ),
    ]
}

/// The one item a confirmation asks about, under its question: its picture
/// and name, then [`detail_lines`], as Windows shows them; only the name
/// when the item could not be read.
fn item_details(item: &DeleteItem, entry: Option<&Entry>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["delete-item"])
        .build();
    if let Some(entry) = entry {
        let art = ArtImage::new(Art::for_entry(entry), DETAILS_ART_SIZE);
        art.set_valign(gtk::Align::Start);
        row.append(&art);
    }
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let details = entry.map(detail_lines).unwrap_or_default();
    for text in std::iter::once(&item.name).chain(&details) {
        let line = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(50)
            .build();
        lines.append(&line);
    }
    row.append(&lines);
    row
}

/// A Trash or delete request for `uris`.
fn removal(mode: TransferMode, uris: Vec<String>) -> TransferRequest {
    TransferRequest {
        mode,
        uris,
        destination_folder: None,
        policy: ConflictPolicy::Skip,
    }
}

impl BrowserWindow {
    /// The selected items, as the confirmations name them.
    pub(super) fn items_to_delete(&self) -> Vec<DeleteItem> {
        let items = self.folder_pane().model().selected_items();
        items
            .iter()
            .map(|item| DeleteItem {
                uri: item.entry().uri.clone(),
                name: item.entry().name.clone(),
                can_trash: item.entry().can_trash,
            })
            .collect()
    }

    /// Whether Shift is held now. A button or a menu item runs its action
    /// with no key event to read, so this asks the keyboard, as Windows
    /// Explorer does when Delete is clicked with Shift held.
    pub(super) fn shift_is_held(&self) -> bool {
        #[cfg(test)]
        if self.imp().test_shift_held.get() {
            return true;
        }
        let keyboard = WidgetExt::display(self)
            .default_seat()
            .and_then(|seat| seat.keyboard());
        keyboard.is_some_and(|keyboard| {
            keyboard
                .modifier_state()
                .contains(gtk::gdk::ModifierType::SHIFT_MASK)
        })
    }

    /// Holds Shift, or lets go of it, for the commands run next, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn hold_shift_for_tests(&self, held: bool) {
        self.imp().test_shift_held.set(held);
    }

    /// Delete: asks, then moves each selected item to its folder's Trash,
    /// or deletes it where the folder has none.
    pub(crate) async fn delete_selection(&self) {
        if !self.allows(FileCommand::Delete) {
            return;
        }
        if self.shows_recycle_bin() {
            self.delete_from_recycle_bin().await;
            return;
        }
        let next = self.uri_after_selection();
        self.trash_items(&self.items_to_delete(), next.as_deref()).await;
    }

    /// Items dropped on the Recycle Bin, or a folder of the folder tree:
    /// moved to the Trash with Delete's confirmation (OPS-045, SIDE-028).
    pub(crate) async fn trash_dropped(&self, uris: Vec<String>) {
        self.trash_items(&named_items(uris), None).await;
    }

    /// Deletes the folder tree's folder at `uri` permanently, after
    /// Shift+Delete's confirmation (SIDE-028).
    pub(crate) async fn delete_permanently_at(&self, uri: &str) {
        let items = named_items(vec![uri.to_owned()]);
        if !self.confirms_permanent_delete(&items).await {
            return;
        }
        self.run_deletion(&removal(TransferMode::Delete, vec![uri.to_owned()]), None)
            .await;
    }

    /// Asks, then moves each of `items` to its folder's Trash, or deletes
    /// it where the folder has none; `next` is selected afterwards.
    async fn trash_items(&self, items: &[DeleteItem], next: Option<&str>) {
        // Only a cancellation fails the plan, and nothing cancels it here.
        let Ok(plan) = plan_delete(items, &Cancellation::new()).await else {
            return;
        };
        let preferences = self.context().settings_data().preferences;
        let asks = (!plan.to_trash.is_empty() && preferences.confirm_trash)
            || (!plan.to_delete.is_empty() && preferences.confirm_delete);
        if asks
            && !self
                .confirm_deletion(items, |subject| plan.confirmation(subject))
                .await
        {
            return;
        }
        if !plan.to_trash.is_empty() {
            self.run_deletion(&removal(TransferMode::Trash, plan.to_trash), next)
                .await;
        }
        if !plan.to_delete.is_empty() {
            self.run_deletion(&removal(TransferMode::Delete, plan.to_delete), next)
                .await;
        }
    }

    /// Shift+Delete: asks, then deletes the selection permanently, even
    /// where a Trash exists (OPS-016).
    pub(crate) async fn delete_selection_permanently(&self) {
        if !self.allows(FileCommand::DeletePermanently) {
            return;
        }
        if self.shows_recycle_bin() {
            self.delete_from_recycle_bin().await;
            return;
        }
        let items = self.items_to_delete();
        let next = self.uri_after_selection();
        if !self.confirms_permanent_delete(&items).await {
            return;
        }
        let uris = items.into_iter().map(|item| item.uri).collect();
        self.run_deletion(&removal(TransferMode::Delete, uris), next.as_deref())
            .await;
    }

    /// Asks before `items` are deleted permanently, unless the settings
    /// say not to ask (SET-010); true when they may be deleted.
    pub(super) async fn confirms_permanent_delete(&self, items: &[DeleteItem]) -> bool {
        !self.context().settings_data().preferences.confirm_delete
            || self.confirm_deletion(items, permanent_delete_confirmation).await
    }

    /// Asks about `items` as Windows Explorer does: the question
    /// `confirmation` words for what they are, one item's details under
    /// it, and Yes, which has focus and Enter presses, and No, which
    /// Escape also answers. True for Yes.
    async fn confirm_deletion(
        &self,
        items: &[DeleteItem],
        confirmation: impl FnOnce(DeleteSubject) -> DeleteConfirmation,
    ) -> bool {
        let entry = single_entry(items).await;
        let subject = DeleteSubject::of(items.len(), entry.as_ref().is_some_and(is_folder));
        let confirmation = confirmation(subject);
        let dialog = Dialog::new(self, confirmation.title, &confirmation.body);
        if let [item] = items {
            dialog.add_field(&item_details(item, entry.as_ref()));
        }
        let yes = dialog.add_button(&gettext("Yes"), ButtonStyle::Accent);
        dialog.add_button(&gettext("No"), ButtonStyle::Bordered);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer == Some(yes)
    }

    /// True while the tab shows the Recycle Bin itself.
    pub(super) fn shows_recycle_bin(&self) -> bool {
        self.command_facts().folder.is_recycle_bin
    }
}
