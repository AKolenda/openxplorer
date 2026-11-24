// SPDX-License-Identifier: AGPL-3.0-only
//! The "Rename File" question when a file renamed in place takes the name
//! of another file in its folder (OPS-008, OPS-010).
//!
//! Windows 11 File Explorer (checked on Windows 11 on 2026-11-14) asks in
//! a small message box titled "Rename File": "Do you want to rename
//! "Notes 2.md" to "Notes 1 (2).md"?", naming the file being renamed and
//! the name it is offered, and on the next line "There is already a file
//! with the same name in this location.", with Yes and No. Yes renames the
//! file to that numbered name, the first one free; No goes back to editing
//! the typed name. Nothing is overwritten: the numbered rename never
//! replaces anything either, and a numbered name taken meanwhile is
//! refused with the toast. A file renamed to a folder's name, or a folder
//! to a file's, is refused with the toast as before.
//!
//! A folder renamed to the name of another folder asks, as Windows 11 does
//! (checked in Windows 11 26200), "Confirm Folder Replace": "This
//! destination already contains a folder named 'Beta'.", "If any files
//! have the same names, you will be asked if you want to replace those
//! files.", "Do you still want to merge this folder", the folder already
//! there with its icon, name and date created, "with this one?" and the
//! renamed folder the same way, with Yes and No. Yes merges the renamed
//! folder into the other ([`BrowserWindow::merge_folder_into`]); No goes
//! back to editing the typed name.

use gtk::gio::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{entry_from_info, Entry, ATTRIBUTES};
use ox_core::location::{name_with_copy_marker, ItemKind};

use gtk::prelude::*;

use crate::dialog::Dialog;
use crate::folder_view::item::FileItem;
use crate::icons::ArtImage;
use crate::window::{BrowserWindow, ButtonStyle};

/// The folder question's title, as Windows titles it.
const MERGE_TITLE: &str = crate::i18n::message_id("Confirm Folder Replace");

/// The size of each folder's icon in the folder question.
const MERGE_ICON_SIZE: i32 = 48;

/// How far the folder question's lines below its message start in, to
/// line up with the message beside the warning icon.
const MERGE_INDENT: i32 = 44;

/// What the "Confirm Folder Replace" question answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MergeAnswer {
    /// Yes: merge the renamed folder into the folder at this URI.
    Merge(String),
    /// No, Escape or closing it: go back to editing the name.
    KeepEditing,
    /// Not asked: the items are not two folders. The rename is refused as
    /// before.
    NotAsked,
}

/// The folder question's lines above the folder already there, in Windows
/// 11's words: `name` is that folder's name.
fn merge_question(name: &str) -> String {
    ox_core::i18n::format_message(
        "This destination already contains a folder named '{name}'.\n\nIf any files have the same \
         names, you will be asked if you want to replace those files.\n\nDo you still want to merge \
         this folder",
        &[("name", name)],
    )
}

/// One folder of the question: its icon, then its name and, under it,
/// "Date created:" and when, as Windows shows them.
fn folder_row(entry: &Entry, created: Option<u64>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_start(MERGE_INDENT);
    let art = FileItem::new(entry.clone()).art();
    row.append(&ArtImage::new(art, MERGE_ICON_SIZE));
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    lines.set_valign(gtk::Align::Center);
    let name = gtk::Label::builder()
        .label(entry.name.as_str())
        .xalign(0.0)
        .build();
    let date = ox_core::i18n::format_message(
        "Date created: {date}",
        &[("date", &ox_core::format::date_short_time_text(created))],
    );
    let date = gtk::Label::builder().label(date).xalign(0.0).build();
    lines.append(&name);
    lines.append(&date);
    row.append(&lines);
    row
}

/// When the item at `file` was created, if its file system says.
async fn created_at(file: &gio::File) -> Option<u64> {
    let info = file
        .query_info_future(
            gio::FILE_ATTRIBUTE_TIME_CREATED,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    info.has_attribute(gio::FILE_ATTRIBUTE_TIME_CREATED)
        .then(|| info.attribute_uint64(gio::FILE_ATTRIBUTE_TIME_CREATED))
}

/// The question's title, as Windows titles it.
const TITLE: &str = crate::i18n::message_id("Rename File");

/// How many numbered names are tried before none is offered.
const MAX_NUMBERED_NAMES: u32 = 10_000;

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
/// taken, before its extension: `x (2).txt`, `archive.tar (3).gz`. A
/// shortcut's name has no extension (its `.lnk` is hidden in Windows, and
/// a link here has none), so its number goes at the end: `Report.txt -
/// Shortcut (2)`.
fn numbered_name(name: &str, number: u32, is_shortcut: bool) -> Option<String> {
    let kind = if is_shortcut {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    name_with_copy_marker(name, &format!(" ({number})"), kind).ok()
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

/// The first numbered `name` free in `folder`, from `(2)` on, numbered
/// as a shortcut's name when `is_shortcut`; the file's own name,
/// `own_name`, counts as free.
async fn first_free_numbered_name(
    folder: &gio::File,
    name: &str,
    own_name: &str,
    is_shortcut: bool,
) -> Option<String> {
    for number in 2..=MAX_NUMBERED_NAMES {
        let candidate = numbered_name(name, number, is_shortcut)?;
        if candidate == own_name || is_free(folder, &candidate).await {
            return Some(candidate);
        }
    }
    None
}

/// The question, in Windows 11's words and on its two lines: the file
/// being renamed, `name`, and the name it is offered, `numbered`.
fn question(name: &str, numbered: &str) -> String {
    ox_core::i18n::format_message(
        "Do you want to rename \"{name}\" to \"{numbered}\"?\nThere is already a file with the same name \
         in this location.",
        &[("name", name), ("numbered", numbered)],
    )
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
        let Some(numbered) =
            first_free_numbered_name(&folder, name, &renamed.name, renamed.is_shortcut()).await
        else {
            return TakenNameAnswer::NotAsked;
        };
        // A message box as Windows 11 draws it: Yes is focused, so Enter
        // renames; No, Escape and the close button go back to editing.
        let dialog = Dialog::message_box(
            self,
            ox_core::i18n::gettext_static(TITLE),
            &question(&renamed.name, &numbered),
        );
        let yes = dialog.add_button(ox_core::i18n::gettext_static("Yes"), ButtonStyle::Accent);
        let no = dialog.add_button(ox_core::i18n::gettext_static("No"), ButtonStyle::Bordered);
        // Alt+Y and Alt+N in every language, as Windows keeps them.
        dialog.also_press_with(yes, 'y');
        dialog.also_press_with(no, 'n');
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

impl BrowserWindow {
    /// Asks, as Windows does, whether the folder at `uri`, renamed to the
    /// name `name` of another folder in its folder, should be merged into
    /// that folder.
    pub(super) async fn ask_about_merging(&self, uri: &str, name: &str) -> MergeAnswer {
        let item = gio::File::for_uri(uri);
        let Some(folder) = item.parent() else {
            return MergeAnswer::NotAsked;
        };
        let existing_file = folder.child(name);
        let (Some(renamed), Some(existing)) = (entry_at(&item).await, entry_at(&existing_file).await) else {
            return MergeAnswer::NotAsked;
        };
        let real_folder = |entry: &Entry| entry.is_dir && !entry.is_symlink;
        if !real_folder(&renamed) || !real_folder(&existing) {
            return MergeAnswer::NotAsked;
        }
        let (renamed_created, existing_created) = (created_at(&item).await, created_at(&existing_file).await);
        let dialog = Dialog::message_box(
            self,
            ox_core::i18n::gettext_static(MERGE_TITLE),
            &merge_question(&existing.name),
        );
        dialog.add_field(&folder_row(&existing, existing_created));
        let with = gtk::Label::builder()
            .label(ox_core::i18n::gettext("with this one?"))
            .xalign(0.0)
            .margin_start(MERGE_INDENT)
            .build();
        dialog.add_field(&with);
        dialog.add_field(&folder_row(&renamed, renamed_created));
        let yes = dialog.add_button(ox_core::i18n::gettext_static("Yes"), ButtonStyle::Accent);
        let no = dialog.add_button(ox_core::i18n::gettext_static("No"), ButtonStyle::Bordered);
        dialog.also_press_with(yes, 'y');
        dialog.also_press_with(no, 'n');
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer == Some(yes) {
            MergeAnswer::Merge(existing.uri)
        } else {
            MergeAnswer::KeepEditing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-008
    #[test]
    fn a_taken_name_is_numbered_before_its_extension_as_in_windows() {
        assert_eq!(numbered_name("x.txt", 2, false).as_deref(), Some("x (2).txt"));
        assert_eq!(
            numbered_name("Notes 10.txt", 3, false).as_deref(),
            Some("Notes 10 (3).txt")
        );
        assert_eq!(
            numbered_name("archive.tar.gz", 2, false).as_deref(),
            Some("archive.tar (2).gz")
        );
        assert_eq!(numbered_name("README", 2, false).as_deref(), Some("README (2)"));
        assert_eq!(
            numbered_name("Report.txt - Shortcut", 2, true).as_deref(),
            Some("Report.txt - Shortcut (2)"),
            "a shortcut's whole name is its name"
        );
        assert_eq!(numbered_name("Q3.v1", 2, true).as_deref(), Some("Q3.v1 (2)"));
        assert_eq!(
            question("Notes 2.md", "Notes 1 (2).md"),
            "Do you want to rename \"Notes 2.md\" to \"Notes 1 (2).md\"?\nThere is already a file with \
             the same name in this location."
        );
    }
}
