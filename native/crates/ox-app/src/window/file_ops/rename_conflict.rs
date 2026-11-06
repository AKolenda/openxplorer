// SPDX-License-Identifier: AGPL-3.0-only
//! The "Rename File" question when a file renamed in place takes the name
//! of another file in its folder (OPS-008, OPS-010).
//!
//! Windows File Explorer says "There is already a file with the same name
//! in this location.", shows both files with their size and date, and asks
//! "Would you like to rename "x.txt" to "x (2).txt"?". Yes renames the file
//! to that numbered name, the first one free; No goes back to editing the
//! typed name. Nothing is overwritten: the numbered rename never replaces
//! anything either, and a numbered name taken meanwhile is refused with the
//! toast. A folder renamed to a taken name, or a file renamed to a
//! folder's name, is refused with the toast as before: Windows offers to
//! merge two folders there, which a rename here does not do.

use gtk::gio::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{entry_from_info, Entry, ATTRIBUTES};
use ox_core::format::{date_short_time_text, size_text};
use ox_core::location::{name_with_copy_marker, ItemKind};

use crate::dialog::Dialog;
use crate::icons::{Art, ArtImage};
use crate::window::{BrowserWindow, ButtonStyle};

/// The question's title, as Windows titles it.
const TITLE: &str = crate::i18n::message_id("Rename File");

/// The question's message, in Windows' words.
const MESSAGE: &str = crate::i18n::message_id("There is already a file with the same name in this location.");

/// How many numbered names are tried before none is offered.
const MAX_NUMBERED_NAMES: u32 = 10_000;

/// The size of the files' icons in the question.
const ITEM_ART_SIZE: i32 = 32;

/// What the "Rename File" question answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TakenNameAnswer {
    /// Yes: rename the file to this numbered name.
    Rename(String),
    /// No, Escape or closing it: go back to editing the name.
    KeepEditing,
    /// Not asked: the items are not two files, or no numbered name is
    /// free. The rename is refused as before.
    NotAsked,
}

/// `name` with the `number` Windows gives a renamed file whose name is
/// taken, before its extension: `x (2).txt`, `archive.tar (3).gz`.
fn numbered_name(name: &str, number: u32) -> Option<String> {
    name_with_copy_marker(name, &format!(" ({number})"), ItemKind::File).ok()
}

/// The item at `file`, or `None` when it cannot be read.
async fn entry_at(file: &gio::File) -> Option<Entry> {
    let info = file
        .query_info_future(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT)
        .await
        .ok()?;
    Some(entry_from_info(file, &info))
}

/// True when nothing in `folder` is called `name`, a link included.
async fn is_free(folder: &gio::File, name: &str) -> bool {
    folder
        .child(name)
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .is_err()
}

/// The first numbered `name` free in `folder`, from `(2)` on; the file's
/// own name, `own_name`, counts as free.
async fn first_free_numbered_name(folder: &gio::File, name: &str, own_name: &str) -> Option<String> {
    for number in 2..=MAX_NUMBERED_NAMES {
        let candidate = numbered_name(name, number)?;
        if candidate == own_name || is_free(folder, &candidate).await {
            return Some(candidate);
        }
    }
    None
}

/// The question's line, in Windows' words.
fn question(name: &str, numbered: &str) -> String {
    ox_core::i18n::format_message(
        "Would you like to rename \"{name}\" to \"{numbered}\"?",
        &[("name", name), ("numbered", numbered)],
    )
}

/// One file as the question shows it: its icon, name, size and date.
fn item_row(entry: &Entry) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["rename-file-item"])
        .build();
    row.append(&ArtImage::new(Art::for_entry(entry), ITEM_ART_SIZE));
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let size = ox_core::i18n::format_message("Size: {size}", &[("size", &size_text(entry.size))]);
    let date = ox_core::i18n::format_message(
        "Date modified: {date}",
        &[("date", &date_short_time_text(entry.modified))],
    );
    for (text, class) in [
        (entry.name.as_str(), "rename-file-name"),
        (&size, "rename-file-detail"),
        (&date, "rename-file-detail"),
    ] {
        let label = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(48)
            .css_classes([class])
            .build();
        lines.append(&label);
    }
    row.append(&lines);
    row
}

impl BrowserWindow {
    /// Asks, as Windows does, whether the file at `uri`, renamed to the
    /// taken `name`, should take the first free numbered name instead.
    pub(super) async fn ask_about_taken_name(&self, uri: &str, name: &str) -> TakenNameAnswer {
        let item = gio::File::for_uri(uri);
        let Some(folder) = item.parent() else {
            return TakenNameAnswer::NotAsked;
        };
        let (Some(renamed), Some(existing)) = (entry_at(&item).await, entry_at(&folder.child(name)).await)
        else {
            return TakenNameAnswer::NotAsked;
        };
        if renamed.is_dir || existing.is_dir {
            return TakenNameAnswer::NotAsked;
        }
        let Some(numbered) = first_free_numbered_name(&folder, name, &renamed.name).await else {
            return TakenNameAnswer::NotAsked;
        };
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(TITLE),
            ox_core::i18n::gettext_static(MESSAGE),
        );
        let line = gtk::Label::builder()
            .label(question(name, &numbered))
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(60)
            .build();
        dialog.add_field(&line);
        dialog.add_field(&item_row(&existing));
        dialog.add_field(&item_row(&renamed));
        let yes = dialog.add_button(ox_core::i18n::gettext_static("Yes"), ButtonStyle::Accent);
        dialog.add_button(ox_core::i18n::gettext_static("No"), ButtonStyle::Bordered);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer == Some(yes) {
            TakenNameAnswer::Rename(numbered)
        } else {
            TakenNameAnswer::KeepEditing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-008
    #[test]
    fn a_taken_name_is_numbered_before_its_extension_as_in_windows() {
        assert_eq!(numbered_name("x.txt", 2).as_deref(), Some("x (2).txt"));
        assert_eq!(
            numbered_name("Notes 10.txt", 3).as_deref(),
            Some("Notes 10 (3).txt")
        );
        assert_eq!(
            numbered_name("archive.tar.gz", 2).as_deref(),
            Some("archive.tar (2).gz")
        );
        assert_eq!(numbered_name("README", 2).as_deref(), Some("README (2)"));
        assert_eq!(
            question("x.txt", "x (2).txt"),
            "Would you like to rename \"x.txt\" to \"x (2).txt\"?"
        );
    }
}
