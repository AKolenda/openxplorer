// SPDX-License-Identifier: AGPL-3.0-only
//! What each details column says about an item, and its tooltip.
//!
//! A folder's Size shows its measured size once measured; before that it
//! says how many items the folder holds (VIEW-037), as Dolphin's
//! "Number of items" does, counted off the main thread for folders on
//! this computer ([`count_items`]) and kept with the item. Date created,
//! File extension, Owner and Permissions are the columns the header's
//! menu adds (VIEW-033).

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::format;
use ox_core::search::display_path;

use crate::folder_view::cells::{CellOwners, CellTooltip};
use crate::folder_view::item::FileItem;
use crate::folder_view::sorting::SortColumn;
use crate::thumbnails::Slots;

/// How many folders are counted at a time.
const COUNTING_AT_ONCE: usize = 2;

thread_local! {
    static COUNTING: Rc<Slots> = Rc::new(Slots::new(COUNTING_AT_ONCE));
}

/// The text `column` shows for `item`. A folder shows its measured size
/// once measured, else its item count once counted, else nothing; a file
/// of unknown size shows `—` (`prettyBytes` in app.js).
pub(crate) fn cell_text(column: SortColumn, item: &FileItem) -> String {
    let entry = item.entry();
    match column {
        SortColumn::Name => entry.name.clone(),
        SortColumn::Modified => format::date_short_time_text(entry.modified),
        SortColumn::Created => entry
            .created
            .map(|created| format::date_short_time_text(Some(created)))
            .unwrap_or_default(),
        SortColumn::FolderPath => item.folder_path().text.clone(),
        SortColumn::Type => entry.type_label.clone(),
        SortColumn::Size => match (item.folder_size(), item.item_count()) {
            (Some(measured), _) => measured.size_text(),
            (None, Some(count)) => item_count_text(count),
            (None, None) if entry.is_dir => String::new(),
            (None, None) => format::size_text(item.file_size()),
        },
        SortColumn::Extension => extension_of(item).to_owned(),
        SortColumn::Owner => entry.owner.clone().unwrap_or_default(),
        SortColumn::Permissions => entry.unix_mode.map(permissions_text).unwrap_or_default(),
    }
}

/// The tooltip of `column`'s cell for `item` in place of the row's
/// (`renderRows` in app.js): a search result's full path in Folder path,
/// how a measured folder size was counted in Size, else none.
pub(super) fn cell_tooltip(column: SortColumn) -> CellTooltip {
    match column {
        SortColumn::FolderPath => |item| Some(display_path(&item.entry().uri)),
        SortColumn::Size => |item| item.folder_size().map(|measured| measured.cell_tooltip()),
        _ => |_| None,
    }
}

/// "1 item" or "12 items".
pub(crate) fn item_count_text(count: u32) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

/// A file's extension without the dot, as typed; empty for a folder, a
/// name without one, or a hidden file's leading dot.
pub(crate) fn extension_of(item: &FileItem) -> &str {
    let entry = item.entry();
    if entry.is_dir {
        return "";
    }
    match entry.name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => extension,
        _ => "",
    }
}

/// `drwxr-xr-x`, as `ls -l` and Dolphin's Permissions column write the
/// bits of `mode`.
pub(crate) fn permissions_text(mode: u32) -> String {
    let kind = match mode & 0o170_000 {
        0o040_000 => 'd',
        0o120_000 => 'l',
        _ => '-',
    };
    let bits = (0..9).rev().map(|bit| {
        let letter = ['x', 'w', 'r'][bit % 3];
        if mode & (1 << bit) != 0 {
            letter
        } else {
            '-'
        }
    });
    std::iter::once(kind).chain(bits).collect()
}

/// Counts the items of the folder `item` that `label`, a Size cell,
/// shows, when `owners` count items and it was neither counted nor
/// measured. The count shows once known, if the cell still shows the
/// folder; a cell that shows another item by the time a count could
/// start is skipped, as a row scrolled away is.
pub(super) fn request_item_count(label: &gtk::Label, item: &FileItem, owners: &Rc<CellOwners>) {
    let entry = item.entry();
    let wanted = owners.counts_items()
        && entry.is_dir
        && entry.uri.starts_with("file:")
        && item.folder_size().is_none()
        && item.item_count().is_none();
    if !wanted {
        return;
    }
    let (label, item, owners) = (label.downgrade(), item.clone(), Rc::downgrade(owners));
    glib::spawn_future_local(async move {
        let shows_item = || {
            let (Some(label), Some(owners)) = (label.upgrade(), owners.upgrade()) else {
                return None;
            };
            (owners.item_of(&label).as_ref() == Some(&item)).then_some(label)
        };
        let _slot = COUNTING.with(Rc::clone).take().await;
        if shows_item().is_none() || item.item_count().is_some() {
            return;
        }
        let uri = item.entry().uri.clone();
        let Ok(Some(count)) = gio::spawn_blocking(move || count_items(&uri)).await else {
            return;
        };
        item.set_item_count(count);
        if let Some(label) = shows_item() {
            label.set_text(&cell_text(SortColumn::Size, &item));
        }
    });
}

/// How many items the local folder at `uri` holds, hidden ones left
/// out as the views leave them out by default; `None` where it is not on
/// this computer or cannot be read. Blocks: run it off the main thread.
pub(crate) fn count_items(uri: &str) -> Option<u32> {
    let path = gio::File::for_uri(uri).path()?;
    let names = std::fs::read_dir(path).ok()?;
    let shown = names
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().as_encoded_bytes().starts_with(b"."))
        .count();
    Some(u32::try_from(shown).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder's Size says how many items it holds.
    ///
    /// parity: VIEW-037
    #[gtk::test]
    fn a_folder_size_counts_its_items() {
        let folder = tempfile::tempdir().expect("a folder");
        for name in ["a.txt", "b.txt", ".hidden"] {
            std::fs::write(folder.path().join(name), b"x").expect("a file");
        }
        let uri = gtk::gio::File::for_path(folder.path()).uri();
        let mut entry = crate::test_support::file_entry("Projects");
        entry.is_dir = true;
        let item = FileItem::new(entry);
        assert_eq!(cell_text(SortColumn::Size, &item), "");

        item.set_item_count(count_items(&uri).expect("a local folder"));

        assert_eq!(cell_text(SortColumn::Size, &item), "2 items");
        assert_eq!(permissions_text(0o040_755), "drwxr-xr-x");
    }
}
