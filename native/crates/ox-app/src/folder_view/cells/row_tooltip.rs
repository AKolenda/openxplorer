// SPDX-License-Identifier: AGPL-3.0-only
//! The tooltip of an item: Windows Explorer's infotip (a file's type, size
//! and date modified, a folder's date created), or its full path while
//! searching.
//!
//! Ports `row.title=state.query?displayUri(e.uri):e.name` of `renderRows`
//! in `v2.0.0:desktop/ui/app.js` (VIEW-001, VIEW-042), with Explorer's
//! infotip in place of a name the row already shows: it shows over the
//! item itself (its icon and name, or its tile), under the full name when
//! that is cut short, and the other columns show their own tooltips, or
//! their full text when it is cut short. GTK reuses cells for other items, so the
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
    /// Nothing but a name cut short, while a folder is listed with Folder
    /// Options' "Show pop-up description for folder and desktop items"
    /// off.
    NameOnly,
}

impl RowTooltip {
    /// The tooltip of the item `item`, if it has one.
    pub(crate) fn text(self, item: &FileItem) -> Option<String> {
        match self {
            RowTooltip::Name => infotip(item),
            RowTooltip::FullPath => Some(display_path(&item.entry().uri)),
            RowTooltip::NameOnly => None,
        }
    }
}

/// Explorer's infotip for `item`: "Type", "Size" and "Date modified" for
/// a file, "Date created" for a folder, each line only when known; none
/// when nothing is, as Explorer shows none then.
fn infotip(item: &FileItem) -> Option<String> {
    let entry = item.entry();
    let mut lines = Vec::new();
    if entry.is_dir {
        if let Some(created) = entry.meta.created {
            let date = format::date_short_time_text(Some(created));
            lines.push(format_message("Date created: {date}", &[("date", &date)]));
        }
    } else {
        if !entry.type_label.is_empty() {
            lines.push(format_message("Type: {type}", &[("type", &entry.type_label)]));
        }
        if let Some(size) = item.file_size() {
            let size = format::short_size(size);
            lines.push(format_message("Size: {size}", &[("size", &size)]));
        }
        if entry.modified.is_some() {
            let date = format::date_short_time_text(entry.modified);
            lines.push(format_message("Date modified: {date}", &[("date", &date)]));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
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
    /// Another column, which shows its own tooltip, its full text when that
    /// is cut short, or the full path while searching.
    Column,
}

/// Gives `cell`, one of the cells registered in `owners`, the tooltip
/// `own_tooltip` gives for its item, else what [`tooltip_text`] says.
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
        let cut_short = cut_short_text(cell.upcast_ref(), &owners.shown_name(&item));
        let row = match owners.row_tooltip() {
            RowTooltip::Name if owners.hides_infotips() => RowTooltip::NameOnly,
            row => row,
        };
        let Some(text) = tooltip_text(row, kind, &item, own_tooltip(&item), cut_short) else {
            return false;
        };
        tooltip.set_text(Some(&text));
        true
    });
}

/// The tooltip of a `kind` cell showing `item`, or `None` for none: the
/// cell's `own` tooltip, else the full path while searching, else the
/// item's infotip under its full name when that is `cut_short`, or a
/// column's full text when it is `cut_short`, as Explorer shows them.
fn tooltip_text(
    row: RowTooltip,
    kind: TooltipCell,
    item: &FileItem,
    own: Option<String>,
    cut_short: Option<String>,
) -> Option<String> {
    if own.is_some() {
        return own;
    }
    match (row, kind) {
        (RowTooltip::FullPath, _) => row.text(item),
        (RowTooltip::Name | RowTooltip::NameOnly, TooltipCell::Item) => match (cut_short, row.text(item)) {
            (Some(name), Some(tip)) => Some(format!("{name}\n{tip}")),
            (name, None) => name,
            (None, tip) => tip,
        },
        (RowTooltip::Name | RowTooltip::NameOnly, TooltipCell::Column) => cut_short,
    }
}

/// The full text `cell` cuts short with an ellipsis, if it does: the
/// item's name as it shows (`shown_name`), or a column's text.
fn cut_short_text(cell: &gtk::Widget, shown_name: &str) -> Option<String> {
    if let Some(file_cell) = cell.downcast_ref::<super::FileCell>() {
        return file_cell.name_is_elided().then(|| shown_name.to_owned());
    }
    let label = cell.downcast_ref::<gtk::Label>()?;
    label.layout().is_ellipsized().then(|| label.text().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    /// The infotip writes a size as the status bar and the details pane
    /// do, "4.38 KB" for 4,486 bytes, and one byte in the singular.
    ///
    /// parity: VIEW-001
    #[test]
    fn the_infotip_size_reads_as_the_status_bar_writes_it() {
        let size_line = |bytes| {
            let mut entry = file_entry("Notes 2.txt");
            entry.size = Some(bytes);
            let tooltip = RowTooltip::default()
                .text(&FileItem::new(entry))
                .unwrap_or_default();
            tooltip
                .lines()
                .find(|line| line.starts_with("Size:"))
                .map(str::to_owned)
        };
        assert_eq!(size_line(4_486).as_deref(), Some("Size: 4.38 KB"));
        assert_eq!(size_line(1).as_deref(), Some("Size: 1 byte"));
        assert_eq!(size_line(912).as_deref(), Some("Size: 912 bytes"));
    }

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
            Some(format!(
                "Type: Text document\nSize: {}\nDate modified: {}",
                format::short_size(2_048),
                format::date_short_time_text(Some(1_789_378_200))
            ))
        );
        assert_eq!(
            RowTooltip::FullPath.text(&item).as_deref(),
            Some("/tmp/ox-test/Notes 2.txt")
        );

        let mut folder = folder_entry("Photos");
        folder.modified = Some(1_789_378_200);
        folder.meta.created = Some(1_789_000_000);
        assert_eq!(
            RowTooltip::default().text(&FileItem::new(folder.clone())),
            Some(format!(
                "Date created: {}",
                format::date_short_time_text(Some(1_789_000_000))
            )),
            "a folder shows when it was created, as Explorer's infotip does"
        );
        folder.meta.created = None;
        let unknown = FileItem::new(folder);
        assert_eq!(
            RowTooltip::default().text(&unknown),
            None,
            "nothing Explorer shows is known: no infotip"
        );
        assert_eq!(
            tooltip_text(
                RowTooltip::Name,
                TooltipCell::Item,
                &unknown,
                None,
                Some("Photos".to_owned())
            ),
            Some("Photos".to_owned()),
            "a name cut short is still read in full"
        );
        assert_eq!(
            tooltip_text(RowTooltip::Name, TooltipCell::Item, &unknown, None, None),
            None
        );
    }

    /// With Folder Options' "Show pop-up description for folder and
    /// desktop items" off, an item shows no infotip, only its name when
    /// that is cut short, and a column its own tooltip as before.
    ///
    /// parity: VIEW-042
    #[test]
    fn pop_up_descriptions_off_leave_only_a_name_cut_short() {
        let mut entry = file_entry("Notes 2.txt");
        entry.type_label = "Text document".to_owned();
        entry.size = Some(2_048);
        let file = FileItem::new(entry);
        assert!(RowTooltip::Name.text(&file).is_some(), "an infotip by default");
        assert_eq!(RowTooltip::NameOnly.text(&file), None);
        assert_eq!(
            tooltip_text(RowTooltip::NameOnly, TooltipCell::Item, &file, None, None),
            None
        );
        assert_eq!(
            tooltip_text(
                RowTooltip::NameOnly,
                TooltipCell::Item,
                &file,
                None,
                Some("Notes 2.txt".to_owned())
            ),
            Some("Notes 2.txt".to_owned())
        );
        assert_eq!(
            tooltip_text(
                RowTooltip::NameOnly,
                TooltipCell::Column,
                &file,
                Some("Own".to_owned()),
                None
            ),
            Some("Own".to_owned())
        );
    }

    /// The item cell shows the infotip, under its full name when that is
    /// cut short; other columns show only their own tooltip, their full
    /// text when cut short, or the path while searching.
    ///
    /// parity: VIEW-001, VIEW-041, VIEW-042
    #[test]
    fn each_cell_shows_its_own_tooltip_or_the_infotip() {
        let file = FileItem::new({
            let mut entry = file_entry("A long report name.txt");
            entry.type_label = "Text document".to_owned();
            entry
        });
        let tip = RowTooltip::Name.text(&file).expect("a file's type is known");
        assert_eq!(
            tooltip_text(RowTooltip::Name, TooltipCell::Item, &file, None, None),
            Some(tip.clone())
        );
        assert_eq!(
            tooltip_text(
                RowTooltip::Name,
                TooltipCell::Item,
                &file,
                None,
                Some("A long report name.txt".to_owned())
            ),
            Some(format!("A long report name.txt\n{tip}")),
            "a name cut short is read in full above the infotip"
        );
        assert_eq!(
            tooltip_text(RowTooltip::Name, TooltipCell::Column, &file, None, None),
            None,
            "other columns do not repeat the infotip"
        );
        assert_eq!(
            tooltip_text(
                RowTooltip::Name,
                TooltipCell::Column,
                &file,
                None,
                Some("Text document".to_owned())
            ),
            Some("Text document".to_owned()),
            "a column cut short shows its full text"
        );
        assert_eq!(
            tooltip_text(RowTooltip::FullPath, TooltipCell::Column, &file, None, None),
            Some("/tmp/ox-test/A long report name.txt".to_owned()),
            "every cell shows the path while searching"
        );
        assert_eq!(
            tooltip_text(
                RowTooltip::Name,
                TooltipCell::Column,
                &file,
                Some("own".to_owned()),
                None
            ),
            Some("own".to_owned())
        );
    }
}
