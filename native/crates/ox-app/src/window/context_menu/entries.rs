// SPDX-License-Identifier: AGPL-3.0-only
//! What the folder views' context menus list (CMD-009, CMD-010,
//! CMD-011, CMD-016, OPS-040).
//!
//! Ports `entryMenu`, `backgroundMenu` and `terminalMenuItem` of
//! `desktop/ui/app.js`, item for item and in their order, with their
//! shortcuts and the items a multi-selection disables. The menus are
//! plain data built from a few facts ([`ItemFacts`]), so their order is
//! tested without a window. Beyond the Python app: Duplicate after
//! Delete, Undo and Redo in the folder's menu (as Windows offers "Undo
//! Rename" there), and the Recycle Bin's own menus (Restore, Delete,
//! Empty).

use ox_core::search::Caching;

use crate::icons::Icon;
use crate::integration::EditorShortcut;
use crate::window::cache_folder::cache_item;
use crate::window::menu_popover::{MenuEntry, MenuItem, MenuStyle};
use crate::window::window_action::WindowAction;

/// What the right-clicked item is, as far as its menu cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemShape {
    /// It opens as a folder.
    Folder,
    /// A ZIP archive (`isZipEntry`), which can be extracted.
    ZipArchive,
    /// Any other file.
    File,
}

/// Where the right-clicked item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemLocation {
    /// On this computer, a phone or another backend.
    Local,
    /// On an SMB share, whose server the user can sign out of.
    SmbShare,
    /// An SMB server itself, whose size cannot be measured.
    SmbServer,
}

/// Whether Compare files is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comparison {
    /// Not two files, or no comparison tool is installed.
    Unavailable,
    /// Exactly two files are selected and a comparison tool is installed.
    TwoFiles,
}

/// What a file or folder's menu depends on: the right-clicked item and
/// the selection it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemFacts {
    /// Where the item opens: a folder's, share's or shortcut's target.
    pub(crate) navigation_uri: String,
    /// A folder, a ZIP archive or another file.
    pub(crate) shape: ItemShape,
    /// Where it is.
    pub(crate) location: ItemLocation,
    /// It is in a read-only previous version.
    pub(crate) is_read_only: bool,
    /// At most one item is selected (`state.selection.size<=1`).
    pub(crate) is_single: bool,
    /// It is a search result, listed away from its folder
    /// (`state.query`).
    pub(crate) is_search_result: bool,
    /// Whether the selection can be compared (Dolphin's Compare Files).
    pub(crate) comparison: Comparison,
    /// The installed code editors, each offered as "Open in <editor>".
    pub(crate) editors: Vec<EditorShortcut>,
    /// Whether a folder is cached for search; `None` for a file or a
    /// folder the search cache cannot take.
    pub(crate) caching: Option<Caching>,
    /// Delete's label: "Move to Trash" or "Delete permanently".
    pub(crate) delete_label: &'static str,
}

/// A context menu: its rows, and the icon strip of the compact style.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContextMenu {
    /// The rows, dividers included.
    pub(crate) entries: Vec<MenuEntry>,
    /// Cut, Copy, Paste, Rename and Delete in the compact style; empty in
    /// the classic one, which lists them.
    pub(crate) strip: Vec<MenuItem>,
}

/// An item that runs `action`.
fn item(label: &str, glyph: Icon, action: WindowAction) -> MenuItem {
    MenuItem::new(label, glyph, action)
}

/// The Open group: Open, the extraction commands, the applications, for
/// folders Open in new tab (Open in new tabs for several, TAB-027), Open
/// in new window (TAB-028) and Pin to Quick access, and for a search
/// result Open file location (SRCH-015), also in a new tab or window
/// (SRCH-016).
fn open_group(facts: &ItemFacts) -> Vec<MenuEntry> {
    let several = !facts.is_single;
    let is_folder = facts.shape == ItemShape::Folder;
    let open = item("Open", Icon::Folder, WindowAction::Open)
        .with_shortcut("Enter")
        .disabled_when(several || (!is_folder && facts.is_read_only));
    let mut entries: Vec<MenuEntry> = vec![open.into()];
    if facts.shape == ItemShape::ZipArchive {
        entries.extend(extraction_items(several));
    }
    entries.extend(application_items(facts));
    if is_folder {
        let new_tab = if several {
            item("Open in new tabs", Icon::Add, WindowAction::OpenSelectionInTabs)
        } else {
            MenuItem::with_text_target(
                "Open in new tab",
                Icon::Add,
                WindowAction::OpenTab,
                &facts.navigation_uri,
            )
        };
        let new_window = MenuItem::with_text_target(
            "Open in new window",
            Icon::WindowNew,
            WindowAction::OpenWindow,
            &facts.navigation_uri,
        );
        let pin = item("Pin to Quick access", Icon::Pin, WindowAction::PinSelected);
        entries.push(new_tab.into());
        entries.push(new_window.disabled_when(several).into());
        entries.push(pin.disabled_when(several).into());
    }
    if facts.comparison == Comparison::TwoFiles {
        entries.push(item("Compare files", Icon::DocumentCopy, WindowAction::CompareFiles).into());
    }
    if facts.is_search_result {
        let locations = [
            ("Open file location", Icon::Folder, WindowAction::OpenFileLocation),
            (
                "Open file location in new tab",
                Icon::Add,
                WindowAction::OpenFileLocationInTab,
            ),
            (
                "Open file location in new window",
                Icon::WindowNew,
                WindowAction::OpenFileLocationInWindow,
            ),
        ];
        for (label, glyph, action) in locations {
            entries.push(item(label, glyph, action).disabled_when(several).into());
        }
    }
    entries
}

/// Extract all… and, beyond the Python app, Dolphin's Extract here.
fn extraction_items(several: bool) -> [MenuEntry; 2] {
    let extract_all = item("Extract all…", Icon::FolderZip, WindowAction::ExtractAll);
    let extract_here = item("Extract here", Icon::FolderZip, WindowAction::ExtractHere);
    [
        extract_all.disabled_when(several).into(),
        extract_here.disabled_when(several).into(),
    ]
}

/// The Terminal entry (`terminalMenuItem`), Open with and one "Open in
/// <editor>" per installed code editor (`uniqueEditors`), all for one
/// item outside a previous version.
fn application_items(facts: &ItemFacts) -> Vec<MenuEntry> {
    let is_unavailable = !facts.is_single || facts.is_read_only;
    let is_folder = facts.shape == ItemShape::Folder;
    let terminal_label = if is_folder {
        "Open in Terminal"
    } else {
        "Open containing folder in Terminal"
    };
    let open_with_label = if is_folder {
        "Open folder with…"
    } else {
        "Open with…"
    };
    let terminal = item(terminal_label, Icon::WindowConsole, WindowAction::OpenInTerminal);
    let open_with = item(open_with_label, Icon::Apps, WindowAction::OpenWith);
    let mut entries = vec![
        terminal.disabled_when(is_unavailable).into(),
        open_with.disabled_when(is_unavailable).into(),
    ];
    for editor in &facts.editors {
        let label = format!("Open in {}", editor.name);
        let open_in_editor =
            MenuItem::with_text_target(&label, Icon::Document, WindowAction::OpenInEditor, &editor.id);
        entries.push(open_in_editor.disabled_when(is_unavailable).into());
    }
    entries
}

/// Cut, Copy, Paste, Rename and Delete, with their shortcuts; their
/// actions decide when they are enabled.
fn edit_items(facts: &ItemFacts) -> [MenuItem; 5] {
    [
        item("Cut", Icon::Cut, WindowAction::Cut).with_shortcut("Ctrl+X"),
        item("Copy", Icon::Copy, WindowAction::Copy).with_shortcut("Ctrl+C"),
        item("Paste", Icon::ClipboardPaste, WindowAction::Paste).with_shortcut("Ctrl+V"),
        item("Rename", Icon::Rename, WindowAction::Rename).with_shortcut("F2"),
        item(facts.delete_label, Icon::Delete, WindowAction::Trash).with_shortcut("Delete"),
    ]
}

/// Duplicate, which the Python app did not have.
fn duplicate_item() -> MenuEntry {
    item("Duplicate", Icon::DocumentCopy, WindowAction::Duplicate).into()
}

/// Copy path, for one item, with Explorer's key for "Copy as path"
/// (CLIP-013).
fn copy_path_item(facts: &ItemFacts) -> MenuEntry {
    let copy_path = item("Copy path", Icon::Link, WindowAction::CopyPath).with_shortcut("Ctrl+Shift+C");
    copy_path.disabled_when(!facts.is_single).into()
}

/// Compress to ZIP file, which the Python app did not have (Windows 11's
/// "Compress to ZIP file").
fn compress_item() -> MenuEntry {
    item(
        "Compress to ZIP file",
        Icon::FolderZip,
        WindowAction::CompressToZip,
    )
    .into()
}

/// The end of both styles: Calculate folder size for folders, Previous
/// versions and Properties.
fn details_group(facts: &ItemFacts) -> Vec<MenuEntry> {
    let several = !facts.is_single;
    let mut entries = Vec::new();
    let is_measurable = facts.location != ItemLocation::SmbServer;
    if facts.shape == ItemShape::Folder && is_measurable {
        let size = item(
            "Calculate folder size",
            Icon::HardDrive,
            WindowAction::CalculateFolderSize,
        );
        entries.push(size.into());
    }
    let versions = item("Previous versions", Icon::History, WindowAction::PreviousVersions);
    let properties = item("Properties", Icon::Info, WindowAction::Properties).with_shortcut("Alt+Enter");
    entries.push(versions.disabled_when(several).into());
    entries.push(properties.disabled_when(several).into());
    entries
}

/// The menu of a file or folder, in `style` (`entryMenu`).
pub(crate) fn item_menu(facts: &ItemFacts, style: MenuStyle) -> ContextMenu {
    match style {
        MenuStyle::Compact => compact_item_menu(facts),
        MenuStyle::Classic => classic_item_menu(facts),
    }
}

/// The Windows 11 style: the edit commands in the strip, the Open group,
/// Copy path, the details and "Show more options" (CMD-010).
fn compact_item_menu(facts: &ItemFacts) -> ContextMenu {
    let mut entries = open_group(facts);
    entries.push(duplicate_item());
    entries.push(copy_path_item(facts));
    entries.push(compress_item());
    entries.push(MenuEntry::Divider);
    entries.extend(details_group(facts));
    entries.push(MenuEntry::Divider);
    let more = item(
        "Show more options",
        Icon::MoreHorizontal,
        WindowAction::ShowMoreOptions,
    );
    entries.push(more.with_shortcut("Shift+F10").into());
    ContextMenu {
        entries,
        strip: edit_items(facts).to_vec(),
    }
}

/// The Windows 10 style: every command as a row, with the folder's cache
/// entry and an SMB item's Sign out (CMD-009).
fn classic_item_menu(facts: &ItemFacts) -> ContextMenu {
    let [cut, copy, paste, rename, delete] = edit_items(facts);
    let mut entries = open_group(facts);
    entries.push(MenuEntry::Divider);
    entries.extend([cut.into(), copy.into(), paste.into(), MenuEntry::Divider]);
    entries.extend([
        rename.into(),
        delete.into(),
        duplicate_item(),
        copy_path_item(facts),
        compress_item(),
    ]);
    if let Some(caching) = facts.caching {
        entries.push(cache_item(&facts.navigation_uri, caching).into());
    }
    if facts.location != ItemLocation::Local {
        let sign_out = MenuItem::with_text_target(
            "Sign out of server…",
            Icon::ArrowEject,
            WindowAction::SignOut,
            &facts.navigation_uri,
        );
        entries.push(sign_out.into());
    }
    entries.push(MenuEntry::Divider);
    entries.extend(details_group(facts));
    ContextMenu {
        entries,
        strip: Vec::new(),
    }
}

/// The menu of blank space in a folder, acting on the folder
/// (`backgroundMenu`, CMD-011). `undo_label` and `redo_label` name what
/// Undo and Redo would do, such as "Undo: Rename".
pub(crate) fn background_menu(undo_label: &str, redo_label: &str) -> Vec<MenuEntry> {
    vec![
        item("New…", Icon::Add, WindowAction::ShowNewMenu).into(),
        item("Paste", Icon::ClipboardPaste, WindowAction::Paste)
            .with_shortcut("Ctrl+V")
            .into(),
        item(undo_label, Icon::ArrowUndo, WindowAction::Undo)
            .with_shortcut("Ctrl+Z")
            .into(),
        item(redo_label, Icon::ArrowRedo, WindowAction::Redo)
            .with_shortcut("Ctrl+Y")
            .into(),
        item("Refresh", Icon::ArrowClockwise, WindowAction::Refresh)
            .with_shortcut("F5")
            .into(),
        item(
            "Open in Terminal",
            Icon::WindowConsole,
            WindowAction::OpenInTerminal,
        )
        .into(),
        MenuEntry::Divider,
        item("Pin this folder", Icon::Pin, WindowAction::PinFolder).into(),
        MenuItem::toggle(
            "Cache this folder for search",
            Icon::Search,
            WindowAction::CacheFolder,
        )
        .into(),
        item(
            "Calculate folder sizes",
            Icon::HardDrive,
            WindowAction::CalculateFolderSizes,
        )
        .into(),
        MenuEntry::Divider,
        item("Previous versions", Icon::History, WindowAction::PreviousVersions).into(),
        item("Properties", Icon::Info, WindowAction::Properties)
            .with_shortcut("Alt+Enter")
            .into(),
    ]
}

/// The menu of items in the Recycle Bin: Restore, Delete permanently and
/// Properties (OPS-040).
pub(crate) fn recycle_bin_item_menu(is_single: bool) -> Vec<MenuEntry> {
    let properties = item("Properties", Icon::Info, WindowAction::Properties).with_shortcut("Alt+Enter");
    vec![
        item("Restore", Icon::ArrowCounterclockwise, WindowAction::Restore).into(),
        item("Delete permanently", Icon::Delete, WindowAction::Trash)
            .with_shortcut("Delete")
            .into(),
        MenuEntry::Divider,
        properties.disabled_when(!is_single).into(),
    ]
}

/// The menu of blank space in the Recycle Bin: Empty Recycle Bin and
/// Refresh (OPS-040, OPS-042).
pub(crate) fn recycle_bin_background_menu() -> Vec<MenuEntry> {
    vec![
        item(
            "Empty Recycle Bin",
            Icon::DeleteDismiss,
            WindowAction::EmptyRecycleBin,
        )
        .into(),
        item("Refresh", Icon::ArrowClockwise, WindowAction::Refresh)
            .with_shortcut("F5")
            .into(),
    ]
}

#[cfg(test)]
mod tests {
    use gtk::prelude::ToVariant;

    use super::*;
    use crate::window::menu_popover::ItemAvailability;

    /// A single local file, not in a previous version.
    fn file() -> ItemFacts {
        ItemFacts {
            navigation_uri: "file:///home/user/report.pdf".to_owned(),
            shape: ItemShape::File,
            location: ItemLocation::Local,
            is_read_only: false,
            is_single: true,
            is_search_result: false,
            comparison: Comparison::Unavailable,
            editors: Vec::new(),
            caching: None,
            delete_label: "Move to Trash",
        }
    }

    /// A single local folder, not cached for search.
    fn folder() -> ItemFacts {
        ItemFacts {
            navigation_uri: "file:///home/user/Projects".to_owned(),
            shape: ItemShape::Folder,
            caching: Some(Caching::Disabled),
            ..file()
        }
    }

    /// The labels of `entries`, a divider as `-`.
    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    }

    /// The labels of the items `entries` disables.
    fn disabled(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) if item.availability == ItemAvailability::Disabled => {
                    Some(item.label.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// parity: CMD-009, CMD-016, TAB-028
    #[test]
    fn a_folder_classic_menu_keeps_the_python_order_and_shortcuts() {
        let menu = item_menu(&folder(), MenuStyle::Classic);

        assert_eq!(
            labels(&menu.entries),
            [
                "Open",
                "Open in Terminal",
                "Open folder with…",
                "Open in new tab",
                "Open in new window",
                "Pin to Quick access",
                "-",
                "Cut",
                "Copy",
                "Paste",
                "-",
                "Rename",
                "Move to Trash",
                "Duplicate",
                "Copy path",
                "Compress to ZIP file",
                "Cache this folder for search",
                "-",
                "Calculate folder size",
                "Previous versions",
                "Properties",
            ]
        );
        assert!(menu.strip.is_empty());
        let shortcuts: Vec<(String, &str)> = menu
            .entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => item.shortcut.map(|keys| (item.label.clone(), keys)),
                MenuEntry::Divider => None,
            })
            .collect();
        let expected = [
            ("Open", "Enter"),
            ("Cut", "Ctrl+X"),
            ("Copy", "Ctrl+C"),
            ("Paste", "Ctrl+V"),
            ("Rename", "F2"),
            ("Move to Trash", "Delete"),
            // A gain: the Python app had no key for Copy path.
            ("Copy path", "Ctrl+Shift+C"),
            ("Properties", "Alt+Enter"),
        ];
        let expected: Vec<(String, &str)> = expected
            .into_iter()
            .map(|(label, keys)| (label.to_owned(), keys))
            .collect();
        assert_eq!(shortcuts, expected);
    }

    /// parity: CMD-009
    #[test]
    fn a_zip_file_on_a_share_offers_extraction_and_sign_out() {
        let facts = ItemFacts {
            shape: ItemShape::ZipArchive,
            location: ItemLocation::SmbShare,
            delete_label: "Delete permanently",
            ..file()
        };

        let entries = labels(&item_menu(&facts, MenuStyle::Classic).entries);

        assert_eq!(entries[1], "Extract all…");
        assert_eq!(entries[2], "Extract here");
        assert_eq!(entries[3], "Open containing folder in Terminal");
        assert_eq!(entries[4], "Open with…");
        assert!(entries.contains(&"Delete permanently".to_owned()));
        assert!(entries.contains(&"Sign out of server…".to_owned()));
        assert!(!entries.contains(&"Calculate folder size".to_owned()));
    }

    /// parity: CMD-009, TAB-027
    #[test]
    fn several_selected_items_disable_what_acts_on_one() {
        let facts = ItemFacts {
            is_single: false,
            ..folder()
        };

        let menu = item_menu(&facts, MenuStyle::Classic);

        assert_eq!(
            disabled(&menu.entries),
            [
                "Open",
                "Open in Terminal",
                "Open folder with…",
                "Open in new window",
                "Pin to Quick access",
                "Copy path",
                "Previous versions",
                "Properties",
            ]
        );
    }

    /// parity: OPEN-015, OPEN-017
    #[test]
    fn each_code_editor_is_offered_after_open_with_for_one_item() {
        let code = EditorShortcut {
            id: "code.desktop".to_owned(),
            name: "Visual Studio Code".to_owned(),
        };
        let facts = ItemFacts {
            editors: vec![code],
            ..file()
        };
        let several = ItemFacts {
            is_single: false,
            ..facts.clone()
        };

        let entries = item_menu(&facts, MenuStyle::Classic).entries;

        assert_eq!(labels(&entries)[3], "Open in Visual Studio Code");
        let MenuEntry::Item(editor) = &entries[3] else {
            panic!("an editor is an item");
        };
        assert_eq!(editor.action, WindowAction::OpenInEditor.into());
        assert_eq!(editor.target, Some("code.desktop".to_variant()));
        let disabled_for_several = disabled(&item_menu(&several, MenuStyle::Classic).entries);
        assert!(disabled_for_several.contains(&"Open in Visual Studio Code".to_owned()));
    }

    /// parity: OPEN-023
    #[test]
    fn two_files_with_a_comparison_tool_offer_compare_files() {
        let two = ItemFacts {
            is_single: false,
            comparison: Comparison::TwoFiles,
            ..file()
        };
        assert!(labels(&item_menu(&two, MenuStyle::Classic).entries).contains(&"Compare files".to_owned()));
        assert!(
            !labels(&item_menu(&file(), MenuStyle::Classic).entries).contains(&"Compare files".to_owned())
        );
    }

    /// parity: SRCH-015
    #[test]
    fn a_search_result_offers_open_file_location_after_the_open_group() {
        let facts = ItemFacts {
            is_search_result: true,
            ..file()
        };

        let entries = labels(&item_menu(&facts, MenuStyle::Classic).entries);

        assert_eq!(
            entries[..6],
            [
                "Open",
                "Open containing folder in Terminal",
                "Open with…",
                "Open file location",
                "Open file location in new tab",
                "Open file location in new window"
            ]
        );
        assert!(!labels(&item_menu(&file(), MenuStyle::Classic).entries)
            .contains(&"Open file location".to_owned()));
    }

    /// parity: CMD-008, CMD-010
    #[test]
    fn the_compact_menu_puts_the_edit_commands_in_its_strip() {
        let menu = item_menu(&folder(), MenuStyle::Compact);

        let strip: Vec<&str> = menu.strip.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(strip, ["Cut", "Copy", "Paste", "Rename", "Move to Trash"]);
        assert_eq!(
            labels(&menu.entries),
            [
                "Open",
                "Open in Terminal",
                "Open folder with…",
                "Open in new tab",
                "Open in new window",
                "Pin to Quick access",
                "Duplicate",
                "Copy path",
                "Compress to ZIP file",
                "-",
                "Calculate folder size",
                "Previous versions",
                "Properties",
                "-",
                "Show more options",
            ]
        );
    }

    /// parity: CMD-011, OPS-029
    #[test]
    fn the_background_menu_acts_on_the_folder() {
        assert_eq!(
            labels(&background_menu("Undo: Rename", "Redo")),
            [
                "New…",
                "Paste",
                "Undo: Rename",
                "Redo",
                "Refresh",
                "Open in Terminal",
                "-",
                "Pin this folder",
                "Cache this folder for search",
                "Calculate folder sizes",
                "-",
                "Previous versions",
                "Properties",
            ]
        );
    }

    /// parity: OPS-040
    #[test]
    fn the_recycle_bin_menus_restore_delete_and_empty() {
        assert_eq!(
            labels(&recycle_bin_item_menu(true)),
            ["Restore", "Delete permanently", "-", "Properties"]
        );
        assert_eq!(
            labels(&recycle_bin_background_menu()),
            ["Empty Recycle Bin", "Refresh"]
        );
    }
}
