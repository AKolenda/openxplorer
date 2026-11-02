// SPDX-License-Identifier: AGPL-3.0-only
//! The tooltip of an item: Windows Explorer's infotip (its type, size and
//! date modified), or its full path while searching.
//!
//! Ports `row.title=state.query?displayUri(e.uri):e.name` of `renderRows`
//! in `v2.0.0:desktop/ui/app.js` (VIEW-001, VIEW-042), with Explorer's
//! infotip in place of a name the row already shows: it shows over the
//! item itself (its icon and name, or its tile), under the full name when
//! that is cut short, and the other columns show their own tooltips only. GTK reuses cells for other items, so the
//! text is worked out when the tooltip is asked for, from the item the
//! cell shows then and whether the window searches then.

use std::rc::Rc;

use gtk::prelude::*;
use ox_core::format;
use ox_core::i18n::format_message;
use ox_core::search::display_path;

use super::CellOwners;
use crate::folder_view::item::FileItem;

/// What the tooltip of every item names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RowTooltip {
    /// The item's infotip, while a folder is listed.
    #[default]
    Name,
    /// The item's full display path, while the window searches, so a
    /// result is told apart from others of the same name.
    FullPath,
}

impl RowTooltip {
    /// The tooltip of the item `item`.
    pub(crate) fn text(self, item: &FileItem) -> String {
        match self {
            RowTooltip::Name => infotip(item),
            RowTooltip::FullPath => display_path(&item.entry().uri),
        }
    }
}

/// Explorer's infotip for `item`: "Type", "Size" and "Date modified" for
/// a file, "Date modified" for a folder, each line only when known; the
/// name when nothing is.
fn infotip(item: &FileItem) -> String {
    let entry = item.entry();
    let mut lines = Vec::new();
    if !entry.is_dir {
        if !entry.type_label.is_empty() {
            lines.push(format_message("Type: {type}", &[("type", &entry.type_label)]));
        }
        if let Some(size) = item.file_size() {
            let size = format::size_text(Some(size));
            lines.push(format_message("Size: {size}", &[("size", &size)]));
        }
    }
    if entry.modified.is_some() {
        let date = format::date_short_time_text(entry.modified);
        lines.push(format_message("Date modified: {date}", &[("date", &date)]));
    }
    if lines.is_empty() {
        entry.name.clone()
    } else {
        lines.join("\n")
    }
}

/// A cell's own tooltip for `item`, which takes the place of the row's,
/// or `None` to show the row's.
pub(crate) type CellTooltip = fn(&FileItem) -> Option<String>;

/// Which cell of a row a tooltip is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TooltipCell {
    /// The item itself: its icon and name, or its tile. It shows the
    /// infotip.
    Item,
    /// Another column, which shows only its own tooltip, or the full path
    /// while searching.
    Column,
}

/// Gives `cell`, one of the cells registered in `owners`, the tooltip
/// `own_tooltip` gives for its item, else its row's where `kind` shows it.
pub(crate) fn show_row_tooltip(
    cell: &impl IsA<gtk::Widget>,
    owners: &Rc<CellOwners>,
    own_tooltip: CellTooltip,
    kind: TooltipCell,
) {
    let owners = Rc::clone(owners);
    cell.set_has_tooltip(true);
    cell.connect_query_tooltip(move |cell, _, _, _, tooltip| {
        let Some(item) = owners.item_of(cell) else {
            return false;
        };
        let row = owners.row_tooltip();
        let text = match own_tooltip(&item) {
            Some(text) => text,
            // A name cut short is read in full above the infotip.
            None if kind == TooltipCell::Item
                && row == RowTooltip::Name
                && name_is_elided(cell.upcast_ref()) =>
            {
                format!("{}\n{}", item.entry().name, row.text(&item))
            }
            None if kind == TooltipCell::Item || row == RowTooltip::FullPath => row.text(&item),
            None => return false,
        };
        tooltip.set_text(Some(&text));
        true
    });
}

/// Whether `cell` is an item whose name is cut short where it is shown.
fn name_is_elided(cell: &gtk::Widget) -> bool {
    cell.downcast_ref::<super::FileCell>()
        .is_some_and(super::FileCell::name_is_elided)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    /// An item's tooltip is Explorer's infotip, with only what is known,
    /// or its full path while searching.
    ///
    /// parity: VIEW-001, VIEW-041, VIEW-042
    #[test]
    fn an_item_shows_its_infotip_or_its_full_path_while_searching() {
        let mut entry = file_entry("Notes 2.txt");
        entry.type_label = "Text document".to_owned();
        entry.size = Some(2_048);
        entry.modified = Some(1_789_378_200);
        let item = FileItem::new(entry);
        assert_eq!(
            RowTooltip::default().text(&item),
            format!(
                "Type: Text document\nSize: {}\nDate modified: {}",
                format::size_text(Some(2_048)),
                format::date_short_time_text(Some(1_789_378_200))
            )
        );
        assert_eq!(RowTooltip::FullPath.text(&item), "/tmp/ox-test/Notes 2.txt");

        let mut folder = folder_entry("Photos");
        folder.modified = Some(1_789_378_200);
        assert_eq!(
            RowTooltip::default().text(&FileItem::new(folder)),
            format!(
                "Date modified: {}",
                format::date_short_time_text(Some(1_789_378_200))
            ),
            "a folder has no type or size line"
        );
        let mut unknown = folder_entry("Mystery");
        unknown.modified = None;
        assert_eq!(
            RowTooltip::default().text(&FileItem::new(unknown)),
            "Mystery",
            "nothing known: the name"
        );
    }
}
