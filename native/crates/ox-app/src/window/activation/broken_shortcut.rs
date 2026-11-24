// SPDX-License-Identifier: AGPL-3.0-only
//! Opening a shortcut, as Windows Explorer opens one: its target, or
//! "Problem with Shortcut" when the target is gone (OPEN-027, OPEN-028).
//!
//! A shortcut (a symbolic link the app names "… - Shortcut") opens what it
//! points to at that item's own address, so a folder's address bar and Up
//! lead from where the folder really is, and an application gets the
//! file's own name, which tells its type. A symbolic link named for a type
//! ("photo.jpg") keeps opening at its own path, as Windows opens a symbolic
//! link. Where it leads is read on a worker thread
//! ([`ox_core::ops::resolve_shortcut`]), so a link onto a share that stopped
//! answering never blocks the window.
//!
//! A link to nothing, a loop of links included, or a `.desktop` link to a
//! deleted folder asks Windows' "Problem with Shortcut" question and
//! offers to delete the shortcut: Yes, the default, deletes the shortcut
//! permanently, as Windows 11 does. One onto a removable drive or a
//! share that is not connected says, in Windows' words, that the drive is
//! unavailable, and offers nothing to delete.

use gtk::gio;
use gtk::prelude::*;
use ox_core::entry::Entry;
use ox_core::ops::{destination_of, resolve_shortcut, ShortcutDestination};

use crate::dialog::Dialog;
use crate::window::{BrowserWindow, ButtonStyle};

/// The title of both of Windows' questions about a broken shortcut.
const PROBLEM_WITH_SHORTCUT: &str = crate::i18n::message_id("Problem with Shortcut");

/// What Windows says of a shortcut whose item is gone.
const CHANGED_OR_MOVED: &str = crate::i18n::message_id(
    "The item ‘{name}’ that this shortcut refers to has been changed or moved, so this shortcut will \
     no longer work properly.\n\nDo you want to delete this shortcut?",
);

/// What Windows says of a shortcut onto a drive that is not connected.
const UNAVAILABLE: &str = crate::i18n::message_id(
    "The drive or network connection that the shortcut ‘{name}’ refers to is unavailable. Make sure \
     that the disk is properly inserted or the network resource is available, and then try again.",
);

/// A shortcut that leads nowhere, to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct BrokenShortcut {
    /// The shortcut's own address, which Yes deletes.
    pub(in crate::window) link: String,
    /// The shortcut's own name.
    pub(in crate::window) name: String,
    /// Why it leads nowhere.
    pub(in crate::window) destination: ShortcutDestination,
}

/// Where the local symbolic link `entry` leads; `None` for an item that is
/// not a local link.
pub(super) async fn link_destination(entry: &Entry) -> Option<ShortcutDestination> {
    if !entry.is_symlink {
        return None;
    }
    let path = gio::File::for_uri(&entry.uri).path()?;
    gio::spawn_blocking(move || resolve_shortcut(&path).ok())
        .await
        .ok()
        .flatten()
}

/// Where a `.desktop` link's address `uri` leads, when it names a path on
/// this computer; `None` for any other address.
pub(super) async fn location_destination(uri: &str) -> Option<ShortcutDestination> {
    let path = gio::File::for_uri(uri).path()?;
    gio::spawn_blocking(move || destination_of(&path)).await.ok()
}

/// The shortcut `entry` as a broken one leading to `destination`.
pub(super) fn broken(entry: &Entry, destination: ShortcutDestination) -> BrokenShortcut {
    let name = gio::File::for_uri(&entry.uri)
        .basename()
        .map_or_else(|| entry.name.clone(), |name| name.to_string_lossy().into_owned());
    BrokenShortcut {
        link: entry.uri.clone(),
        name,
        destination,
    }
}

impl BrowserWindow {
    /// Asks "Problem with Shortcut", a message box with the warning icon as
    /// Windows shows it, about `broken`: whether to delete a
    /// shortcut whose item is gone, or, for one onto a drive that is not
    /// connected, says so.
    pub(in crate::window) async fn report_broken_shortcut(&self, broken: BrokenShortcut) {
        let title = ox_core::i18n::gettext_static(PROBLEM_WITH_SHORTCUT);
        if matches!(broken.destination, ShortcutDestination::Unavailable { .. }) {
            let message = ox_core::i18n::format_message(UNAVAILABLE, &[("name", &broken.name)]);
            let dialog = Dialog::message_box(self, title, &message);
            dialog.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent);
            dialog.open();
            dialog.next_response().await;
            dialog.finish();
            return;
        }
        let message =
            ox_core::i18n::format_message(CHANGED_OR_MOVED, &[("name", &broken.destination.item_name())]);
        let dialog = Dialog::message_box(self, title, &message);
        let yes = dialog.add_button(ox_core::i18n::gettext_static("Yes"), ButtonStyle::Accent);
        let no = dialog.add_button(ox_core::i18n::gettext_static("No"), ButtonStyle::Bordered);
        // Alt+Y and Alt+N in every language, and Y and N alone, as Windows'
        // message box takes its buttons' letters.
        dialog.also_press_with(yes, 'y');
        dialog.also_press_with(no, 'n');
        dialog.press_with_letters_alone();
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer == Some(yes) {
            self.delete_shortcut(broken.link).await;
        }
    }
}
