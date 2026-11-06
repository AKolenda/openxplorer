// SPDX-License-Identifier: AGPL-3.0-only
//! What the folder views' context menus list (CMD-009, CMD-010,
//! CMD-011, CMD-016, OPS-040).
//!
//! Ports `entryMenu`, `backgroundMenu` and `terminalMenuItem` of
//! `v2.0.0:desktop/ui/app.js`, item for item and in their order, with their
//! shortcuts and the items a multi-selection disables. The menus are
//! plain data built from a few facts ([`ItemFacts`]), so their order is
//! tested without a window. Beyond the Python app: Duplicate after
//! Delete, Undo and Redo in the folder's menu (as Windows offers "Undo
//! Rename" there), and the Recycle Bin's own menus (Restore, Delete,
//! Empty).

use ox_core::integration::DiskTool;
use ox_core::search::Caching;

use crate::icons::Icon;
use crate::integration::{ApplicationChoice, EditorShortcut};
use crate::window::cache_folder::cache_item;
use crate::window::menu_popover::{MenuAction, MenuEntry, MenuItem, MenuStyle};
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent selection and location facts determine menu availability"
)]
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
    /// It is a symbolic link, whose target "Show target" opens (CMD-030).
    pub(crate) is_symlink: bool,
    /// Whether the selection can be compared (Dolphin's Compare Files).
    pub(crate) comparison: Comparison,
    /// The installed code editors, each offered as "Open in <editor>".
    pub(crate) editors: Vec<EditorShortcut>,
    /// The other applications for the item, each offered as "Open with
    /// <app>" (OPEN-013).
    pub(crate) applications: Vec<ApplicationChoice>,
    /// Whether a folder is cached for search; `None` for a file or a
    /// folder the search cache cannot take.
    pub(crate) caching: Option<Caching>,
    /// Delete's label: "Delete", or "Delete permanently" in the Recycle Bin.
    pub(crate) delete_label: &'static str,
    /// The installed disk tool the item offers: Mount disk image for a
    /// local `.iso` or `.img` file (DEV-011), Analyse disk usage for a
    /// local folder (PROP-015).
    pub(crate) disk_tool: Option<DiskTool>,
    /// Where Send to can send the selection (CMD-038).
    pub(crate) send_to: SendToTargets,
}

/// A drive or network location Send to copies to, named as This PC
/// names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SendToDrive {
    /// Its This PC name, such as `Backup (\\fileserver) (S:)`.
    pub(crate) label: String,
    /// Its root.
    pub(crate) uri: String,
    /// A saved network location rather than a removable drive. One that
    /// is not connected is offered too: choosing it connects it first.
    pub(crate) is_network: bool,
}

/// What Send to offers beside Compressed (zipped) folder, which is always
/// there (CMD-038): only what the computer has, as Windows lists only
/// what is set up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag says whether one independent destination exists"
)]
pub(crate) struct SendToTargets {
    /// A Bluetooth adapter and the desktop's sender exist.
    pub(crate) bluetooth: bool,
    /// The desktop has a Desktop folder.
    pub(crate) desktop: bool,
    /// The Documents folder, when the desktop has one.
    pub(crate) documents: Option<String>,
    /// The desktop's Email portal can attach files.
    pub(crate) mail: bool,
    /// The removable drives, then the saved network locations.
    pub(crate) drives: Vec<SendToDrive>,
    /// The selection holds a folder, which only files can be sent as.
    pub(crate) has_folder: bool,
    /// Why a copy cannot start now, when it cannot: Documents, the drives
    /// and the network locations wait for a transfer slot.
    pub(crate) copy_refusal: Option<&'static str>,
    /// Why Desktop (create shortcut) cannot start now, when it cannot: it
    /// runs as the window's one exclusive operation.
    pub(crate) shortcut_refusal: Option<&'static str>,
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

/// Why a command for one item is disabled while several are selected.
const ONE_ITEM_AT_A_TIME: &str = crate::i18n::message_id("Select only one item for this command.");

/// Why a command that runs or changes an item is disabled in a previous
/// version.
const READ_ONLY_VERSION: &str = crate::i18n::message_id("Items in a previous version are read-only.");

/// Why Send to ▸ Bluetooth device is disabled for a folder.
const BLUETOOTH_FILES_ONLY: &str = crate::i18n::message_id("Only files can be sent to a Bluetooth device.");

/// An item that runs `action`.
fn item(label: &str, glyph: Icon, action: WindowAction) -> MenuItem {
    MenuItem::new(label, glyph, action)
}

/// `item`, which acts on one item: disabled, saying why, while several
/// are selected, and where `needs_writable` holds, in a previous version.
fn for_one_item(item: MenuItem, facts: &ItemFacts, needs_writable: bool) -> MenuItem {
    if !facts.is_single {
        return item.disabled_because(true, ox_core::i18n::gettext_static(ONE_ITEM_AT_A_TIME));
    }
    item.disabled_because(
        needs_writable && facts.is_read_only,
        ox_core::i18n::gettext_static(READ_ONLY_VERSION),
    )
}

/// The Open group: Open, the extraction commands, the applications, for
/// folders Open in new tab (Open in new tabs for several, TAB-027), Open
/// in new window (TAB-028) and Pin to Quick access, and for a search
/// result Open file location (SRCH-015), also in a new tab or window
/// (SRCH-016).
fn open_group(facts: &ItemFacts, style: MenuStyle) -> Vec<MenuEntry> {
    let is_folder = facts.shape == ItemShape::Folder;
    let open = item(
        ox_core::i18n::gettext_static("Open"),
        Icon::Folder,
        WindowAction::Open,
    )
    .with_shortcut("Enter")
    .disabled_because(
        !is_folder && facts.is_read_only,
        ox_core::i18n::gettext_static(READ_ONLY_VERSION),
    );
    let mut entries: Vec<MenuEntry> = vec![open.into()];
    if facts.shape == ItemShape::ZipArchive {
        entries.extend(extraction_items(facts));
    }
    match style {
        MenuStyle::Classic => entries.extend(application_items(facts)),
        MenuStyle::Compact => entries.extend(compact_application_items(facts)),
    }
    if facts.disk_tool == Some(DiskTool::MountImage) {
        let mount = MenuItem::with_text_target(
            &ox_core::i18n::gettext("Mount disk image"),
            Icon::HardDrive,
            WindowAction::MountDiskImage,
            &facts.navigation_uri,
        );
        entries.push(for_one_item(mount, facts, false).into());
    }
    if is_folder {
        let new_tab = if facts.is_single {
            MenuItem::with_text_target(
                &ox_core::i18n::gettext("Open in new tab"),
                Icon::Add,
                WindowAction::OpenTab,
                &facts.navigation_uri,
            )
        } else {
            item(
                ox_core::i18n::gettext_static("Open in new tabs"),
                Icon::Add,
                WindowAction::OpenSelectionInTabs,
            )
        };
        let new_window = MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            &facts.navigation_uri,
        );
        let pin = item(
            ox_core::i18n::gettext_static("Pin to Quick access"),
            Icon::Pin,
            WindowAction::PinSelected,
        );
        entries.push(new_tab.into());
        entries.push(for_one_item(new_window, facts, false).into());
        entries.push(for_one_item(pin, facts, false).into());
    }
    if facts.comparison == Comparison::TwoFiles {
        entries.push(
            item(
                ox_core::i18n::gettext_static("Compare files"),
                Icon::DocumentCopy,
                WindowAction::CompareFiles,
            )
            .into(),
        );
    }
    if facts.is_symlink {
        let show_target = item(
            ox_core::i18n::gettext_static("Show target"),
            Icon::Open,
            WindowAction::ShowTarget,
        );
        entries.push(for_one_item(show_target, facts, false).into());
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
            entries.push(for_one_item(item(label, glyph, action), facts, false).into());
        }
    }
    entries
}

/// Extract all… and, beyond the Python app, Dolphin's Extract here.
fn extraction_items(facts: &ItemFacts) -> [MenuEntry; 2] {
    let extract_all = item(
        ox_core::i18n::gettext_static("Extract all…"),
        Icon::FolderZip,
        WindowAction::ExtractAll,
    );
    let extract_here = item(
        ox_core::i18n::gettext_static("Extract here"),
        Icon::FolderZip,
        WindowAction::ExtractHere,
    );
    [
        for_one_item(extract_all, facts, false).into(),
        for_one_item(extract_here, facts, false).into(),
    ]
}

/// The Terminal entry (`terminalMenuItem`), Open with and one "Open in
/// <editor>" per installed code editor (`uniqueEditors`, with the
/// editor's own icon), all for one item outside a previous version.
fn application_items(facts: &ItemFacts) -> Vec<MenuEntry> {
    let is_folder = facts.shape == ItemShape::Folder;
    let terminal_label = if is_folder {
        ox_core::i18n::gettext_static("Open in Terminal")
    } else {
        ox_core::i18n::gettext_static("Open containing folder in Terminal")
    };
    let open_with_label = if is_folder {
        ox_core::i18n::gettext_static("Open folder with…")
    } else {
        ox_core::i18n::gettext_static("Open with…")
    };
    let terminal = item(terminal_label, Icon::WindowConsole, WindowAction::OpenInTerminal);
    let open_with = item(open_with_label, Icon::Apps, WindowAction::OpenWith);
    let mut entries: Vec<MenuEntry> = vec![for_one_item(terminal, facts, true).into()];
    for application in &facts.applications {
        entries.push(for_one_item(open_with_application(application), facts, true).into());
    }
    entries.push(for_one_item(open_with, facts, true).into());
    for editor in &facts.editors {
        let label = ox_core::i18n::format_message("Open in {name}", &[("name", &editor.name)]);
        let open_in_editor =
            MenuItem::with_text_target(&label, Icon::Document, WindowAction::OpenInEditor, &editor.id)
                .with_application_icon(editor.icon.as_deref());
        entries.push(for_one_item(open_in_editor, facts, true).into());
    }
    entries
}

/// The compact menu's applications, as Windows 11 Explorer's: one "Open
/// with" submenu holding the item's applications and editors and "Choose
/// another app" (Open with…), or "Open with…" alone when there are none.
/// A folder's "Open in Terminal" sits above "Show more options"
/// ([`compact_item_menu`]); a file's "Open containing folder in Terminal"
/// is in the classic menu.
fn compact_application_items(facts: &ItemFacts) -> Vec<MenuEntry> {
    let is_folder = facts.shape == ItemShape::Folder;
    let mut entries = Vec::new();
    let mut choices: Vec<MenuEntry> = facts
        .applications
        .iter()
        .map(|application| for_one_item(open_with_application(application), facts, true).into())
        .collect();
    for editor in &facts.editors {
        let label = ox_core::i18n::format_message("Open in {name}", &[("name", &editor.name)]);
        let open_in_editor =
            MenuItem::with_text_target(&label, Icon::Document, WindowAction::OpenInEditor, &editor.id)
                .with_application_icon(editor.icon.as_deref());
        choices.push(for_one_item(open_in_editor, facts, true).into());
    }
    if choices.is_empty() {
        let label = if is_folder {
            ox_core::i18n::gettext_static("Open folder with…")
        } else {
            ox_core::i18n::gettext_static("Open with…")
        };
        entries.push(for_one_item(item(label, Icon::Apps, WindowAction::OpenWith), facts, true).into());
    } else {
        let another = item(
            ox_core::i18n::gettext_static("Choose another app"),
            Icon::Apps,
            WindowAction::OpenWith,
        );
        choices.extend([MenuEntry::Divider, for_one_item(another, facts, true).into()]);
        let open_with = MenuItem::submenu(
            ox_core::i18n::gettext_static("Open with"),
            Icon::Apps,
            WindowAction::OpenWith,
            choices,
        );
        entries.push(for_one_item(open_with, facts, true).into());
    }
    entries
}

/// Cut, Copy, Paste, Rename and Delete, with their shortcuts; their
/// actions decide when they are enabled. On one folder, Paste pastes into
/// it, as Explorer's does and Dolphin's "Paste into folder" (CMD-019).
fn edit_items(facts: &ItemFacts) -> [MenuItem; 5] {
    let paste = if facts.shape == ItemShape::Folder && facts.is_single {
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Paste into folder"),
            Icon::ClipboardPaste,
            WindowAction::PasteInto,
            &facts.navigation_uri,
        )
    } else {
        item(
            ox_core::i18n::gettext_static("Paste"),
            Icon::ClipboardPaste,
            WindowAction::Paste,
        )
        .with_shortcut("Ctrl+V")
    };
    [
        item(ox_core::i18n::gettext_static("Cut"), Icon::Cut, WindowAction::Cut).with_shortcut("Ctrl+X"),
        item(
            ox_core::i18n::gettext_static("Copy"),
            Icon::Copy,
            WindowAction::Copy,
        )
        .with_shortcut("Ctrl+C"),
        paste,
        item(
            ox_core::i18n::gettext_static("Rename"),
            Icon::Rename,
            WindowAction::Rename,
        )
        .with_shortcut("F2"),
        item(facts.delete_label, Icon::Delete, WindowAction::Trash).with_shortcut("Delete"),
    ]
}

/// Duplicate, which the Python app did not have.
fn duplicate_item() -> MenuEntry {
    item(
        ox_core::i18n::gettext_static("Duplicate"),
        Icon::DocumentCopy,
        WindowAction::Duplicate,
    )
    .into()
}

/// Copy as path, as Windows names it, one line per selected item
/// (CLIP-014), with Explorer's key (CLIP-013).
fn copy_path_item() -> MenuEntry {
    item(
        ox_core::i18n::gettext_static("Copy as path"),
        Icon::Link,
        WindowAction::CopyPath,
    )
    .with_shortcut("Ctrl+Shift+C")
    .into()
}

/// Compress to ZIP file, which the Python app did not have (Windows 11's
/// "Compress to ZIP file").
fn compress_item() -> MenuEntry {
    item(
        ox_core::i18n::gettext_static("Compress to ZIP file"),
        Icon::FolderZip,
        WindowAction::CompressToZip,
    )
    .into()
}

/// The end of both styles: Calculate folder size for folders, Previous
/// versions and Properties.
fn details_group(facts: &ItemFacts) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    let is_measurable = facts.location != ItemLocation::SmbServer;
    if facts.shape == ItemShape::Folder && is_measurable {
        let size = item(
            ox_core::i18n::gettext_static("Calculate folder size"),
            Icon::HardDrive,
            WindowAction::CalculateFolderSize,
        );
        entries.push(size.into());
    }
    if facts.disk_tool == Some(DiskTool::AnalyseUsage) {
        let analyse = MenuItem::with_text_target(
            &ox_core::i18n::gettext("Analyse disk usage"),
            Icon::HardDrive,
            WindowAction::AnalyseDiskUsage,
            &facts.navigation_uri,
        );
        entries.push(for_one_item(analyse, facts, false).into());
    }
    let versions = item(
        ox_core::i18n::gettext_static("Previous versions"),
        Icon::History,
        WindowAction::PreviousVersions,
    );
    entries.push(for_one_item(versions, facts, false).into());
    // Properties describe several items together (PROP-002).
    entries.push(properties_item());
    entries
}

/// "Send to", Windows Explorer's submenu in the classic menu (CMD-038), in
/// its order: Bluetooth device, Compressed (zipped) folder, Desktop
/// (create shortcut), Documents, Mail recipient, then each removable
/// drive and saved network location as This PC names it. Bluetooth,
/// Desktop, Documents and Mail recipient appear only where the computer
/// has them; no other application adds entries. Documents and Mail
/// recipient show Fluent's colour art, as Windows shows colour glyphs;
/// the others keep their glyphs, as Fluent has no colour art for them.
fn send_to_item(facts: &ItemFacts) -> MenuEntry {
    let targets = &facts.send_to;
    let mut entries: Vec<MenuEntry> = Vec::new();
    if targets.bluetooth {
        let bluetooth = item(
            ox_core::i18n::gettext_static("Bluetooth device"),
            Icon::Bluetooth,
            WindowAction::SendToBluetooth,
        )
        .disabled_because(
            targets.has_folder,
            ox_core::i18n::gettext_static(BLUETOOTH_FILES_ONLY),
        );
        entries.push(bluetooth.into());
    }
    entries.push(
        item(
            ox_core::i18n::gettext_static("Compressed (zipped) folder"),
            Icon::FolderZip,
            WindowAction::CompressToZip,
        )
        .into(),
    );
    if targets.desktop {
        let desktop = item(
            ox_core::i18n::gettext_static("Desktop (create shortcut)"),
            Icon::Desktop,
            WindowAction::SendToDesktop,
        )
        .disabled_because(
            facts.is_read_only,
            ox_core::i18n::gettext_static(READ_ONLY_VERSION),
        );
        let desktop = match targets.shortcut_refusal {
            Some(reason) => desktop.disabled_because(true, reason),
            None => desktop,
        };
        entries.push(desktop.into());
    }
    if let Some(documents) = &targets.documents {
        // Windows draws Send to's destinations in colour; Fluent's colour
        // document art stands for Documents.
        let glyph = Icon::DocumentColor20;
        entries.push(
            copy_item(
                MenuItem::with_text_target(
                    &ox_core::i18n::gettext("Documents"),
                    glyph,
                    WindowAction::SendTo,
                    documents,
                ),
                targets,
            )
            .into(),
        );
    }
    if targets.mail {
        entries.push(
            item(
                ox_core::i18n::gettext_static("Mail recipient"),
                Icon::MailColor20,
                WindowAction::Share,
            )
            .into(),
        );
    }
    for drive in &targets.drives {
        let glyph = if drive.is_network {
            Icon::Organization
        } else {
            Icon::HardDrive
        };
        let copy = MenuItem::with_text_target(&drive.label, glyph, WindowAction::SendTo, &drive.uri);
        entries.push(copy_item(copy, targets).into());
    }
    MenuItem::submenu(
        ox_core::i18n::gettext_static("Send to"),
        Icon::Share,
        WindowAction::SendTo,
        entries,
    )
    .into()
}

/// `item`, a destination Send to copies to, disabled with the reason
/// while no copy can start.
fn copy_item(item: MenuItem, targets: &SendToTargets) -> MenuItem {
    match targets.copy_refusal {
        Some(reason) => item.disabled_because(true, reason),
        None => item,
    }
}

/// Properties, of the selection or of the folder.
fn properties_item() -> MenuEntry {
    item(
        ox_core::i18n::gettext_static("Properties"),
        Icon::Info,
        WindowAction::Properties,
    )
    .with_shortcut("Alt+Enter")
    .into()
}

/// "Show more options", the compact menu's last item, which opens the
/// classic menu in its place (Shift+F10).
fn show_more_options_item() -> MenuEntry {
    item(
        ox_core::i18n::gettext_static("Show more options"),
        Icon::MoreHorizontal,
        WindowAction::ShowMoreOptions,
    )
    .with_shortcut("Shift+F10")
    .into()
}

/// Windows' own commands that its classic menu ("Show more options",
/// Shift+F10) draws as text alone: Open, Open in new tab and window, Pin
/// to Quick access, Cut, Copy, Paste, Rename, Delete, Copy as path,
/// Restore previous versions, Extract All…, Mount, View, Sort by, Group
/// by, New, Refresh, Undo, Redo, Open file location (and the app's Open
/// file location in a new tab or window, as Open in new tab and window)
/// and the Recycle Bin's Restore and Empty Recycle Bin. The rows Windows draws with an icon keep
/// their glyph: Open in Terminal, Open with and the applications' own
/// icons, Send to, Properties and Open as administrator's shield; so do
/// the app's own commands, which Windows does not have.
const TEXT_ONLY_IN_CLASSIC: [WindowAction; 28] = [
    WindowAction::Open,
    WindowAction::OpenTab,
    WindowAction::OpenSelectionInTabs,
    WindowAction::OpenWindow,
    WindowAction::PinSelected,
    WindowAction::PinFolder,
    WindowAction::Cut,
    WindowAction::Copy,
    WindowAction::Paste,
    WindowAction::PasteInto,
    WindowAction::Rename,
    WindowAction::Trash,
    WindowAction::CopyPath,
    WindowAction::PreviousVersions,
    WindowAction::ExtractAll,
    WindowAction::MountDiskImage,
    WindowAction::ShowViewMenu,
    WindowAction::ShowSortMenu,
    WindowAction::GroupBy,
    WindowAction::ShowNewMenu,
    WindowAction::Refresh,
    WindowAction::Undo,
    WindowAction::Redo,
    WindowAction::OpenFileLocation,
    WindowAction::OpenFileLocationInTab,
    WindowAction::OpenFileLocationInWindow,
    WindowAction::Restore,
    WindowAction::EmptyRecycleBin,
];

/// `entries` as Windows 11 draws its classic menu: no row shows shortcut
/// text, in its submenus either (the keys still work); Open, the command
/// Enter and a double-click run, is in bold, as Windows marks the default
/// command; and only the rows Windows draws with an icon show a glyph
/// ([`TEXT_ONLY_IN_CLASSIC`]). The submenus keep their glyphs, as the
/// command bar's menus and Windows' Send to and New draw them.
pub(crate) fn classic_look(entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    entries
        .into_iter()
        .map(|entry| match entry {
            MenuEntry::Item(item) => {
                let text_only = matches!(item.action, MenuAction::Window(action) if TEXT_ONLY_IN_CLASSIC.contains(&action));
                let is_default = item.action == MenuAction::Window(WindowAction::Open);
                MenuEntry::Item(MenuItem {
                    shows_glyph: item.shows_glyph && !text_only,
                    emphasised: item.emphasised || is_default,
                    ..without_shortcut(item)
                })
            }
            MenuEntry::Divider => MenuEntry::Divider,
        })
        .collect()
}

/// `item` without shortcut text, nor its submenu's rows.
fn without_shortcut(item: MenuItem) -> MenuItem {
    let submenu = item
        .submenu
        .into_iter()
        .map(|entry| match entry {
            MenuEntry::Item(inner) => MenuEntry::Item(without_shortcut(inner)),
            MenuEntry::Divider => MenuEntry::Divider,
        })
        .collect();
    MenuItem {
        shortcut: None,
        submenu,
        ..item
    }
}

/// The submenus of the folder's compact menu: the command bar's View,
/// Sort and New menus and Sort's Group by.
pub(crate) struct BackgroundSubmenus {
    /// View.
    pub view: Vec<MenuEntry>,
    /// Sort by.
    pub sort: Vec<MenuEntry>,
    /// Group by.
    pub group: Vec<MenuEntry>,
    /// New.
    pub new: Vec<MenuEntry>,
}

/// The folder menus' View, Sort by, Group by and New items, which open
/// `submenus` beside the menu.
fn background_submenu_items(submenus: BackgroundSubmenus) -> [MenuEntry; 4] {
    let BackgroundSubmenus {
        view,
        sort,
        group,
        new,
    } = submenus;
    [
        MenuItem::submenu(
            ox_core::i18n::gettext_static("View"),
            Icon::Grid,
            WindowAction::ShowViewMenu,
            view,
        )
        .into(),
        MenuItem::submenu(
            ox_core::i18n::gettext_static("Sort by"),
            Icon::ArrowSort,
            WindowAction::ShowSortMenu,
            sort,
        )
        .into(),
        MenuItem::submenu(
            ox_core::i18n::gettext_static("Group by"),
            Icon::TextBulletList,
            WindowAction::GroupBy,
            group,
        )
        .into(),
        MenuItem::submenu(
            ox_core::i18n::gettext_static("New"),
            Icon::Add,
            WindowAction::ShowNewMenu,
            new,
        )
        .into(),
    ]
}

/// The Windows 11 style of the folder's own menu, in the order of
/// Explorer's compact menu on blank space: View, Sort by, Group by and
/// Refresh; Paste and Undo; New; Properties; Open in Terminal; then "Show
/// more options" for the rest (CMD-011, CMD-012). View, Sort by, Group by
/// and New open `submenus` beside the menu.
pub(crate) fn compact_background_menu(undo_label: &str, submenus: BackgroundSubmenus) -> Vec<MenuEntry> {
    let [view, sort, group, new] = background_submenu_items(submenus);
    vec![
        view,
        sort,
        group,
        item(
            ox_core::i18n::gettext_static("Refresh"),
            Icon::ArrowClockwise,
            WindowAction::Refresh,
        )
        .with_shortcut("F5")
        .into(),
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Paste"),
            Icon::ClipboardPaste,
            WindowAction::Paste,
        )
        .with_shortcut("Ctrl+V")
        .into(),
        item(undo_label, Icon::ArrowUndo, WindowAction::Undo)
            .with_shortcut("Ctrl+Z")
            .into(),
        MenuEntry::Divider,
        new,
        MenuEntry::Divider,
        properties_item(),
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Open in Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenInTerminal,
        )
        .into(),
        MenuEntry::Divider,
        show_more_options_item(),
    ]
}

/// The menu of a file or folder, in `style` (`entryMenu`).
pub(crate) fn item_menu(facts: &ItemFacts, style: MenuStyle) -> ContextMenu {
    match style {
        MenuStyle::Compact => compact_item_menu(facts),
        MenuStyle::Classic => classic_item_menu(facts),
    }
}

/// The Windows 11 style: the edit commands in the strip, the Open group,
/// Compress, Copy as path, Properties and "Show more options" (CMD-010),
/// as Explorer's compact menu lists them. Duplicate, Calculate folder
/// size, Analyse disk usage and Previous versions are in the classic menu
/// behind it.
fn compact_item_menu(facts: &ItemFacts) -> ContextMenu {
    let mut entries = open_group(facts, MenuStyle::Compact);
    entries.push(compress_item());
    entries.push(copy_path_item());
    entries.push(MenuEntry::Divider);
    // Properties describe several items together (PROP-002).
    entries.push(properties_item());
    entries.push(MenuEntry::Divider);
    // A folder's Terminal sits where the folder's own compact menu has it.
    if facts.shape == ItemShape::Folder {
        let terminal = item(
            ox_core::i18n::gettext_static("Open in Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenInTerminal,
        );
        entries.push(for_one_item(terminal, facts, true).into());
        entries.push(MenuEntry::Divider);
    }
    entries.push(show_more_options_item());
    ContextMenu {
        entries,
        strip: edit_items(facts).to_vec(),
    }
}

/// Rename items…, under Rename while several items are selected: the
/// "Rename items" dialog with its `#` template (OPS-014). Disabled, saying
/// why, in a previous version; its action is off wherever Rename is.
fn rename_items_item(facts: &ItemFacts) -> Option<MenuEntry> {
    if facts.is_single {
        return None;
    }
    let rename_items = item(
        ox_core::i18n::gettext_static("Rename items…"),
        Icon::Rename,
        WindowAction::RenameItems,
    )
    .disabled_because(
        facts.is_read_only,
        ox_core::i18n::gettext_static(READ_ONLY_VERSION),
    );
    Some(rename_items.into())
}

/// The Windows 10 style: every command as a row, with Send to (CMD-038),
/// the folder's cache entry and an SMB item's Sign out (CMD-009), and
/// Rename items… for several items (OPS-014).
fn classic_item_menu(facts: &ItemFacts) -> ContextMenu {
    let [cut, copy, paste, rename, delete] = edit_items(facts);
    let mut entries = open_group(facts, MenuStyle::Classic);
    // Explorer's classic menu puts Send to in a group of its own above
    // Cut and Copy.
    entries.extend([MenuEntry::Divider, send_to_item(facts), MenuEntry::Divider]);
    entries.extend([cut.into(), copy.into(), paste.into(), MenuEntry::Divider]);
    entries.push(rename.into());
    entries.extend(rename_items_item(facts));
    entries.extend([
        delete.into(),
        duplicate_item(),
        copy_path_item(),
        compress_item(),
        item(
            ox_core::i18n::gettext_static("Compress to…"),
            Icon::FolderZip,
            WindowAction::CompressTo,
        )
        .into(),
    ]);
    if let Some(caching) = facts.caching {
        entries.push(cache_item(&facts.navigation_uri, caching).into());
    }
    if facts.location != ItemLocation::Local {
        let sign_out = MenuItem::with_text_target(
            &ox_core::i18n::gettext("Sign out of server…"),
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
/// (`backgroundMenu`, CMD-011, CMD-012). `undo_label` and `redo_label` name what
/// Undo and Redo would do, such as "Undo Rename". View, Sort by, Group by
/// and New open `submenus` beside the menu, as in Windows 10's.
pub(crate) fn background_menu(
    undo_label: &str,
    redo_label: &str,
    applications: &[ApplicationChoice],
    submenus: BackgroundSubmenus,
) -> Vec<MenuEntry> {
    // Explorer's and Dolphin's View and Sort by (CMD-012), as the command
    // bar's menus.
    let [view, sort, group, new] = background_submenu_items(submenus);
    let mut entries: Vec<MenuEntry> = vec![
        view,
        sort,
        group,
        MenuEntry::Divider,
        new,
        item(
            ox_core::i18n::gettext_static("Paste"),
            Icon::ClipboardPaste,
            WindowAction::Paste,
        )
        .with_shortcut("Ctrl+V")
        .into(),
        item(undo_label, Icon::ArrowUndo, WindowAction::Undo)
            .with_shortcut("Ctrl+Z")
            .into(),
        item(redo_label, Icon::ArrowRedo, WindowAction::Redo)
            .with_shortcut("Ctrl+Y")
            .into(),
        item(
            ox_core::i18n::gettext_static("Refresh"),
            Icon::ArrowClockwise,
            WindowAction::Refresh,
        )
        .with_shortcut("F5")
        .into(),
        item(
            ox_core::i18n::gettext_static("Open in Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenInTerminal,
        )
        .into(),
    ];
    // The folder's applications and Open folder with… (OPEN-013).
    entries.extend(
        applications
            .iter()
            .map(|application| open_with_application(application).into()),
    );
    entries.push(
        item(
            ox_core::i18n::gettext_static("Open folder with…"),
            Icon::Apps,
            WindowAction::OpenWith,
        )
        .into(),
    );
    entries.extend([
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Pin this folder"),
            Icon::Pin,
            WindowAction::PinFolder,
        )
        .into(),
        MenuItem::toggle(
            &ox_core::i18n::gettext("Cache this folder for search"),
            Icon::Search,
            WindowAction::CacheFolder,
        )
        .into(),
        item(
            ox_core::i18n::gettext_static("Calculate folder sizes"),
            Icon::HardDrive,
            WindowAction::CalculateFolderSizes,
        )
        .into(),
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Previous versions"),
            Icon::History,
            WindowAction::PreviousVersions,
        )
        .into(),
        item(
            ox_core::i18n::gettext_static("Properties"),
            Icon::Info,
            WindowAction::Properties,
        )
        .with_shortcut("Alt+Enter")
        .into(),
    ]);
    entries
}

/// "Open with <app>", opening the item, or the folder when nothing is
/// selected, in `application` (OPEN-013).
fn open_with_application(application: &ApplicationChoice) -> MenuItem {
    let label = ox_core::i18n::format_message("Open with {name}", &[("name", &application.name)]);
    MenuItem::with_text_target(&label, Icon::Apps, WindowAction::OpenWithApp, &application.id)
        .with_application_icon(application.icon.as_deref())
}

/// The menu of items inside a ZIP opened like a folder, as Explorer's
/// inside a "Compressed (zipped) Folder": Open, Copy, Copy as path and
/// Extract all (ARC-026). The ZIP is read-only, so nothing else applies.
pub(crate) fn zip_item_menu() -> Vec<MenuEntry> {
    vec![
        item(
            ox_core::i18n::gettext_static("Open"),
            Icon::Open,
            WindowAction::Open,
        )
        .with_shortcut("Enter")
        .into(),
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Copy"),
            Icon::Copy,
            WindowAction::Copy,
        )
        .with_shortcut("Ctrl+C")
        .into(),
        item(
            ox_core::i18n::gettext_static("Copy as path"),
            Icon::Link,
            WindowAction::CopyPath,
        )
        .with_shortcut("Ctrl+Shift+C")
        .into(),
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Extract all…"),
            Icon::FolderZip,
            WindowAction::ExtractAll,
        )
        .into(),
    ]
}

/// The menu of blank space inside a ZIP opened like a folder: Extract
/// all and Refresh (ARC-026).
pub(crate) fn zip_background_menu() -> Vec<MenuEntry> {
    vec![
        item(
            ox_core::i18n::gettext_static("Extract all…"),
            Icon::FolderZip,
            WindowAction::ExtractAll,
        )
        .into(),
        item(
            ox_core::i18n::gettext_static("Refresh"),
            Icon::ArrowClockwise,
            WindowAction::Refresh,
        )
        .with_shortcut("F5")
        .into(),
    ]
}

/// The menu of items in the Recycle Bin: Restore, Delete permanently and
/// Properties (OPS-040).
pub(crate) fn recycle_bin_item_menu(is_single: bool) -> Vec<MenuEntry> {
    let properties = item(
        ox_core::i18n::gettext_static("Properties"),
        Icon::Info,
        WindowAction::Properties,
    )
    .with_shortcut("Alt+Enter");
    vec![
        item(
            ox_core::i18n::gettext_static("Restore"),
            Icon::ArrowCounterclockwise,
            WindowAction::Restore,
        )
        .into(),
        item(
            ox_core::i18n::gettext_static("Delete permanently"),
            Icon::Delete,
            WindowAction::Trash,
        )
        .with_shortcut("Delete")
        .into(),
        MenuEntry::Divider,
        properties
            .disabled_because(!is_single, ox_core::i18n::gettext_static(ONE_ITEM_AT_A_TIME))
            .into(),
    ]
}

/// The menu of blank space in the Recycle Bin: Empty Recycle Bin and
/// Refresh (OPS-040, OPS-042).
pub(crate) fn recycle_bin_background_menu() -> Vec<MenuEntry> {
    vec![
        item(
            ox_core::i18n::gettext_static("Empty Recycle Bin"),
            Icon::DeleteDismiss,
            WindowAction::EmptyRecycleBin,
        )
        .into(),
        item(
            ox_core::i18n::gettext_static("Refresh"),
            Icon::ArrowClockwise,
            WindowAction::Refresh,
        )
        .with_shortcut("F5")
        .into(),
    ]
}

#[cfg(test)]
mod tests {
    use gtk::prelude::ToVariant;

    use super::*;
    use crate::window::menu_popover::{ItemAvailability, MenuAction};

    /// A single local file, not in a previous version.
    fn file() -> ItemFacts {
        ItemFacts {
            navigation_uri: "file:///home/user/report.pdf".to_owned(),
            shape: ItemShape::File,
            location: ItemLocation::Local,
            is_read_only: false,
            is_single: true,
            is_search_result: false,
            is_symlink: false,
            comparison: Comparison::Unavailable,
            editors: Vec::new(),
            applications: Vec::new(),
            caching: None,
            delete_label: "Delete",
            disk_tool: None,
            send_to: SendToTargets::default(),
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
    /// Submenus with one item each, for the folder menus.
    fn submenus() -> BackgroundSubmenus {
        let one = |label: &str| vec![item(label, Icon::ArrowSort, WindowAction::Sort).into()];
        BackgroundSubmenus {
            view: one("Details"),
            sort: one("Name"),
            group: one("Type"),
            new: one("Folder"),
        }
    }

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

    /// Every Send to destination this computer has.
    fn everything() -> SendToTargets {
        SendToTargets {
            bluetooth: true,
            desktop: true,
            documents: Some("file:///home/demo/Documents".to_owned()),
            mail: true,
            drives: vec![
                SendToDrive {
                    label: "Holiday stick".to_owned(),
                    uri: "file:///media/demo/HOLIDAY".to_owned(),
                    is_network: false,
                },
                SendToDrive {
                    label: "Backup (\\\\fileserver) (S:)".to_owned(),
                    uri: "smb://fileserver/backup".to_owned(),
                    is_network: true,
                },
                SendToDrive {
                    label: "Media (M:)".to_owned(),
                    uri: "smb://studio-nas/media".to_owned(),
                    is_network: true,
                },
            ],
            has_folder: false,
            copy_refusal: None,
            shortcut_refusal: None,
        }
    }

    /// The Send to submenu of `facts`' classic menu.
    fn send_to_of(facts: &ItemFacts) -> MenuItem {
        item_menu(facts, MenuStyle::Classic)
            .entries
            .into_iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == "Send to" => Some(item),
                _ => None,
            })
            .expect("Send to is listed")
    }

    /// Send to lists Windows' destinations in Windows' order, each with
    /// the action and target that sends there, and only the classic menu
    /// has it, as in Windows 11.
    ///
    /// parity: CMD-038
    #[test]
    fn send_to_lists_windows_destinations_in_windows_order() {
        let facts = ItemFacts {
            send_to: everything(),
            ..file()
        };
        let send_to = send_to_of(&facts);

        assert_eq!(
            labels(&send_to.submenu),
            [
                "Bluetooth device",
                "Compressed (zipped) folder",
                "Desktop (create shortcut)",
                "Documents",
                "Mail recipient",
                "Holiday stick",
                "Backup (\\\\fileserver) (S:)",
                "Media (M:)",
            ]
        );
        let actions: Vec<(MenuAction, Option<String>)> = items_of(&send_to.submenu)
            .map(|item| {
                (
                    item.action,
                    item.target.as_ref().and_then(gtk::glib::Variant::get::<String>),
                )
            })
            .collect();
        let text = |uri: &str| Some(uri.to_owned());
        assert_eq!(
            actions,
            [
                (WindowAction::SendToBluetooth.into(), None),
                (WindowAction::CompressToZip.into(), None),
                (WindowAction::SendToDesktop.into(), None),
                (WindowAction::SendTo.into(), text("file:///home/demo/Documents")),
                (WindowAction::Share.into(), None),
                (WindowAction::SendTo.into(), text("file:///media/demo/HOLIDAY")),
                (WindowAction::SendTo.into(), text("smb://fileserver/backup")),
                (WindowAction::SendTo.into(), text("smb://studio-nas/media")),
            ]
        );
        // A saved location that is not connected stays enabled, as in
        // Windows: choosing it connects first.
        assert!(disabled(&send_to.submenu).is_empty());
        let compact = labels(&item_menu(&facts, MenuStyle::Compact).entries);
        assert!(!compact.contains(&"Send to".to_owned()), "{compact:?}");
    }

    /// As in Windows, Documents and Mail recipient show colour glyphs:
    /// Fluent's own colour art, which exists for these two; the others keep
    /// their Fluent glyphs, as no colour art exists for them.
    ///
    /// parity: CMD-038
    #[test]
    fn send_to_shows_fluent_colour_art_where_it_exists() {
        let facts = ItemFacts {
            send_to: everything(),
            ..file()
        };
        let glyphs: Vec<(String, Icon)> = items_of(&send_to_of(&facts).submenu)
            .map(|item| (item.label.clone(), item.glyph))
            .collect();

        assert_eq!(
            glyphs[..5],
            [
                ("Bluetooth device".to_owned(), Icon::Bluetooth),
                ("Compressed (zipped) folder".to_owned(), Icon::FolderZip),
                ("Desktop (create shortcut)".to_owned(), Icon::Desktop),
                ("Documents".to_owned(), Icon::DocumentColor20),
                ("Mail recipient".to_owned(), Icon::MailColor20),
            ]
        );
    }

    /// Only what the computer has is offered: without a Bluetooth sender,
    /// a mail app, the standard folders or drives, Send to holds
    /// Compressed (zipped) folder alone.
    ///
    /// parity: CMD-038
    #[test]
    fn send_to_leaves_out_what_the_computer_lacks() {
        let send_to = send_to_of(&file());

        assert_eq!(labels(&send_to.submenu), ["Compressed (zipped) folder"]);
    }

    /// Entries that cannot run are disabled and say why: Bluetooth for a
    /// folder, Desktop (create shortcut) in a previous version.
    ///
    /// parity: CMD-038, CMD-031
    #[test]
    fn send_to_disables_what_cannot_run_and_says_why() {
        let with_folder = ItemFacts {
            send_to: SendToTargets {
                has_folder: true,
                ..everything()
            },
            ..file()
        };
        let in_version = ItemFacts {
            is_read_only: true,
            send_to: everything(),
            ..file()
        };
        let reasons = |facts: &ItemFacts| -> Vec<(String, Option<&'static str>)> {
            items_of(&send_to_of(facts).submenu)
                .filter(|item| item.availability == ItemAvailability::Disabled)
                .map(|item| (item.label.clone(), item.disabled_reason))
                .collect()
        };

        assert_eq!(
            reasons(&with_folder),
            [("Bluetooth device".to_owned(), Some(BLUETOOTH_FILES_ONLY)),]
        );
        assert_eq!(
            reasons(&in_version),
            [("Desktop (create shortcut)".to_owned(), Some(READ_ONLY_VERSION)),]
        );
    }

    /// While the window's file operations leave no room, the entries
    /// that would start one are disabled and say why; Bluetooth device,
    /// Compressed (zipped) folder and Mail recipient, which use no
    /// transfer slot, keep their own rules.
    ///
    /// parity: CMD-038, CMD-031
    #[test]
    fn send_to_disables_copies_and_shortcuts_without_room_and_says_why() {
        let wait = "Wait for a running file operation to finish.";
        let facts = ItemFacts {
            send_to: SendToTargets {
                copy_refusal: Some(wait),
                shortcut_refusal: Some(wait),
                ..everything()
            },
            ..file()
        };
        let reasons: Vec<(String, Option<&'static str>)> = items_of(&send_to_of(&facts).submenu)
            .filter(|item| item.availability == ItemAvailability::Disabled)
            .map(|item| (item.label.clone(), item.disabled_reason))
            .collect();

        let disabled = |label: &str| (label.to_owned(), Some(wait));
        assert_eq!(
            reasons,
            [
                disabled("Desktop (create shortcut)"),
                disabled("Documents"),
                disabled("Holiday stick"),
                disabled("Backup (\\\\fileserver) (S:)"),
                disabled("Media (M:)"),
            ]
        );
    }

    /// The items among `entries`.
    fn items_of(entries: &[MenuEntry]) -> impl Iterator<Item = &MenuItem> {
        entries.iter().filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Divider => None,
        })
    }

    /// Rename items… follows Rename in the classic menu ("Show more
    /// options") only while several items are selected, and is disabled,
    /// saying why, in a previous version.
    ///
    /// parity: OPS-014, CMD-009
    #[test]
    fn rename_items_follows_rename_in_the_classic_menu_for_several_items_only() {
        let several = ItemFacts {
            is_single: false,
            ..file()
        };
        let read_only = ItemFacts {
            is_read_only: true,
            ..several.clone()
        };
        let classic = labels(&item_menu(&several, MenuStyle::Classic).entries);
        let one = labels(&item_menu(&file(), MenuStyle::Classic).entries);
        let compact = labels(&item_menu(&several, MenuStyle::Compact).entries);
        let rename = classic
            .iter()
            .position(|label| label == "Rename")
            .expect("Rename is listed");

        assert_eq!(classic[rename + 1], "Rename items…");
        assert!(!one.iter().any(|label| label == "Rename items…"));
        assert!(!compact.iter().any(|label| label == "Rename items…"));
        assert!(
            !disabled(&item_menu(&several, MenuStyle::Classic).entries).contains(&"Rename items…".to_owned())
        );
        assert!(disabled(&item_menu(&read_only, MenuStyle::Classic).entries)
            .contains(&"Rename items…".to_owned()));
        let action = item_menu(&several, MenuStyle::Classic)
            .entries
            .into_iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == "Rename items…" => Some(item.action),
                _ => None,
            });
        assert_eq!(action, Some(WindowAction::RenameItems.into()));
    }

    /// parity: CMD-009, CMD-016, TAB-028, CMD-038
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
                "Send to",
                "-",
                "Cut",
                "Copy",
                "Paste into folder",
                "-",
                "Rename",
                "Delete",
                "Duplicate",
                "Copy as path",
                "Compress to ZIP file",
                "Compress to…",
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
            // CMD-019: Paste on one folder pastes into it, without a key.
            ("Rename", "F2"),
            ("Delete", "Delete"),
            // A gain: the Python app had no key for Copy path.
            ("Copy as path", "Ctrl+Shift+C"),
            ("Properties", "Alt+Enter"),
        ];
        let expected: Vec<(String, &str)> = expected
            .into_iter()
            .map(|(label, keys)| (label.to_owned(), keys))
            .collect();
        assert_eq!(shortcuts, expected);
    }

    /// As Windows 11 draws its classic menu: no shortcut text, in the
    /// submenus either; Open, the default command, in bold; a glyph only
    /// on the rows Windows draws with an icon (and the app's own), Send
    /// to's destinations keeping theirs. The compact menu is not changed.
    ///
    /// parity: LOOK-019, CMD-038
    #[test]
    fn the_classic_look_drops_shortcut_text_bolds_open_and_keeps_windows_icons() {
        let entries = classic_look(item_menu(&folder(), MenuStyle::Classic).entries);
        let items: Vec<&MenuItem> = items_of(&entries).collect();
        assert!(
            items.iter().all(|item| item.shortcut.is_none()),
            "no shortcut text"
        );
        let bold: Vec<&str> = items
            .iter()
            .filter(|item| item.emphasised)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(bold, ["Open"]);
        let glyphs: Vec<&str> = items
            .iter()
            .filter(|item| item.shows_glyph)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(
            glyphs,
            [
                "Open in Terminal",
                "Open folder with…",
                "Send to",
                "Duplicate",
                "Compress to ZIP file",
                "Compress to…",
                "Cache this folder for search",
                "Calculate folder size",
                "Properties",
            ]
        );
        let send_to = items
            .iter()
            .find(|item| item.label == "Send to")
            .expect("Send to");
        assert!(items_of(&send_to.submenu).all(|item| item.shows_glyph));

        let new_folder =
            item("Folder", Icon::FolderAdd, WindowAction::NewFolder).with_shortcut("Ctrl+Shift+N");
        let submenus = BackgroundSubmenus {
            new: vec![new_folder.into()],
            ..submenus()
        };
        let background = classic_look(background_menu("Undo", "Redo", &[], submenus));
        let new = items_of(&background)
            .find(|item| item.label == "New")
            .expect("New");
        assert!(!new.shows_glyph, "New is text alone");
        assert!(
            items_of(&new.submenu).all(|item| item.shortcut.is_none() && item.shows_glyph),
            "its rows keep their glyphs, without shortcut text"
        );

        let compact = item_menu(&folder(), MenuStyle::Compact).entries;
        let open = items_of(&compact).next().expect("Open");
        assert!(open.shows_glyph && !open.emphasised && open.shortcut == Some("Enter"));
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

    /// A disk image offers Mount disk image after Open with, and a folder
    /// Analyse disk usage after Calculate folder size, where their tools
    /// are installed.
    ///
    /// parity: DEV-011, PROP-015
    #[test]
    fn disk_images_mount_and_folders_analyse_their_usage_where_the_tools_exist() {
        let image = ItemFacts {
            navigation_uri: "file:///home/user/distro.iso".to_owned(),
            disk_tool: Some(DiskTool::MountImage),
            ..file()
        };
        let folder = ItemFacts {
            disk_tool: Some(DiskTool::AnalyseUsage),
            ..folder()
        };

        let image_menu = labels(&item_menu(&image, MenuStyle::Classic).entries);
        let folder_menu = labels(&item_menu(&folder, MenuStyle::Classic).entries);

        assert_eq!(image_menu[3], "Mount disk image");
        let size = folder_menu
            .iter()
            .position(|label| label == "Calculate folder size");
        let analyse = folder_menu.iter().position(|label| label == "Analyse disk usage");
        assert_eq!(analyse, size.map(|size| size + 1));
        assert!(
            !labels(&item_menu(&file(), MenuStyle::Classic).entries).contains(&"Mount disk image".to_owned())
        );
    }

    /// Open stays: it opens each item (OPEN-003).
    ///
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
                "Open in Terminal",
                "Open folder with…",
                "Open in new window",
                "Pin to Quick access",
                "Previous versions",
            ]
        );
    }

    /// parity: OPEN-015, OPEN-017
    #[test]
    fn each_code_editor_is_offered_after_open_with_for_one_item() {
        let code = EditorShortcut {
            id: "code.desktop".to_owned(),
            name: "Visual Studio Code".to_owned(),
            icon: Some("com.visualstudio.code".to_owned()),
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

    /// In the compact menu the applications and editors go into one "Open
    /// with" submenu ending with "Choose another app", as Windows 11
    /// Explorer's, and a file's Terminal entry stays in the classic menu.
    ///
    /// parity: CMD-010, OPEN-013
    #[test]
    fn the_compact_menu_keeps_the_applications_in_open_with() {
        let code = EditorShortcut {
            id: "code.desktop".to_owned(),
            name: "Visual Studio Code".to_owned(),
            icon: None,
        };
        let facts = ItemFacts {
            editors: vec![code],
            ..file()
        };
        let entries = item_menu(&facts, MenuStyle::Compact).entries;
        let names = labels(&entries);
        assert_eq!(names[..2], ["Open", "Open with"], "{names:?}");
        assert!(!names.iter().any(|name| name.contains("Terminal")), "{names:?}");
        let MenuEntry::Item(open_with) = &entries[1] else {
            panic!("Open with is an item");
        };
        assert_eq!(
            labels(&open_with.submenu),
            ["Open in Visual Studio Code", "-", "Choose another app"]
        );
    }

    /// An editor's item carries the editor's desktop ID for its icon, and
    /// every item a menu disables says why.
    ///
    /// parity: CMD-031
    #[test]
    fn editors_show_their_icon_and_disabled_items_say_why() {
        let code = EditorShortcut {
            id: "code.desktop".to_owned(),
            name: "Visual Studio Code".to_owned(),
            icon: Some("com.visualstudio.code".to_owned()),
        };
        let in_version = ItemFacts {
            editors: vec![code],
            is_read_only: true,
            ..file()
        };
        let several = ItemFacts {
            is_single: false,
            ..folder()
        };

        let reasons = |facts: &ItemFacts| -> Vec<(String, Option<&'static str>)> {
            let menu = item_menu(facts, MenuStyle::Classic);
            let items = menu.entries.into_iter().filter_map(|entry| match entry {
                MenuEntry::Item(item) if item.availability == ItemAvailability::Disabled => Some(item),
                _ => None,
            });
            items.map(|item| (item.label, item.disabled_reason)).collect()
        };

        let editor = item_menu(&in_version, MenuStyle::Classic).entries[3].clone();
        let MenuEntry::Item(editor) = editor else {
            panic!("an editor is an item");
        };
        assert_eq!(editor.application_icon.as_deref(), Some("com.visualstudio.code"));
        for (label, reason) in reasons(&in_version) {
            assert_eq!(reason, Some(READ_ONLY_VERSION), "{label}");
        }
        for (label, reason) in reasons(&several) {
            assert_eq!(reason, Some(ONE_ITEM_AT_A_TIME), "{label}");
        }
    }

    /// The item's other applications come before Open with…, each
    /// opening the item in that application.
    ///
    /// parity: OPEN-013
    #[test]
    fn the_items_other_applications_are_offered_before_open_with() {
        let viewer = ApplicationChoice {
            id: "org.gnome.Papers.desktop".to_owned(),
            name: "Papers".to_owned(),
            is_default: false,
            is_recommended: true,
            is_available: true,
            icon: None,
        };
        let facts = ItemFacts {
            applications: vec![viewer],
            ..file()
        };

        let entries = item_menu(&facts, MenuStyle::Classic).entries;

        assert_eq!(labels(&entries)[2..4], ["Open with Papers", "Open with…"]);
        let MenuEntry::Item(open) = &entries[2] else {
            panic!("an application is an item");
        };
        assert_eq!(open.action, WindowAction::OpenWithApp.into());
        assert_eq!(open.target, Some("org.gnome.Papers.desktop".to_variant()));
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
        assert_eq!(strip, ["Cut", "Copy", "Paste into folder", "Rename", "Delete"]);
        assert_eq!(
            labels(&menu.entries),
            [
                "Open",
                "Open folder with…",
                "Open in new tab",
                "Open in new window",
                "Pin to Quick access",
                "Compress to ZIP file",
                "Copy as path",
                "-",
                "Properties",
                "-",
                "Open in Terminal",
                "-",
                "Show more options",
            ]
        );
    }

    /// The folder's compact menu lists what Explorer's does on blank
    /// space and ends with "Show more options", which opens the classic
    /// one with everything else; View, Sort by and New are submenus.
    ///
    /// parity: CMD-010, CMD-011
    #[test]
    fn the_compact_background_menu_ends_with_show_more_options() {
        let sort = vec![item("Name", Icon::ArrowSort, WindowAction::Sort).into()];
        let submenus = BackgroundSubmenus {
            view: Vec::new(),
            sort: sort.clone(),
            group: Vec::new(),
            new: Vec::new(),
        };
        let menu = compact_background_menu("Undo", submenus);
        let MenuEntry::Item(sort_by) = &menu[1] else {
            panic!("Sort by is an item");
        };
        assert_eq!(sort_by.submenu, sort, "Sort by opens the sort menu in place");
        assert_eq!(
            labels(&menu),
            [
                "View",
                "Sort by",
                "Group by",
                "Refresh",
                "-",
                "Paste",
                "Undo",
                "-",
                "New",
                "-",
                "Properties",
                "-",
                "Open in Terminal",
                "-",
                "Show more options",
            ]
        );
    }

    /// The folder's applications and Open folder with… follow Open in
    /// Terminal (OPEN-013).
    ///
    /// parity: CMD-011, OPS-029, OPEN-013
    #[test]
    fn the_background_menu_acts_on_the_folder() {
        let files = ApplicationChoice {
            id: "org.gnome.Nautilus.desktop".to_owned(),
            name: "Files".to_owned(),
            is_default: true,
            is_recommended: true,
            is_available: true,
            icon: None,
        };
        assert_eq!(
            labels(&background_menu("Undo Rename", "Redo", &[files], submenus())),
            [
                "View",
                "Sort by",
                "Group by",
                "-",
                "New",
                "Paste",
                "Undo Rename",
                "Redo",
                "Refresh",
                "Open in Terminal",
                "Open with Files",
                "Open folder with…",
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
