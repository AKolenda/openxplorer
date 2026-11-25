// SPDX-License-Identifier: AGPL-3.0-only
//! The context menus of places: the drives and network locations of the
//! sidebar and This PC.
//!
//! Ports `driveMenu`, `networkLocationMenu` and the menus of This PC's
//! cards (`shares()` and the Devices and drives cards of `renderLanding`)
//! in `v2.0.0:desktop/ui/app.js`, with Dolphin's Eject and Safely remove added
//! where a drive allows them. A menu is data ([`PlaceMenu`]); its items
//! run window actions with the place as their target, and the menu opens
//! where the pointer is ([`popup_place_menu`]).
//!
//! Drives and network places also offer Open in Terminal
//! (`terminalMenuItem`, disabled where no terminal can open: phones,
//! cameras and a server's share list), "Cache this folder for search"
//! where the folder can be indexed (`cacheMenuItems`) and Properties. The
//! Python app had the Terminal item on network places only, and the cache
//! item on drives only. A removable drive also offers Format…, the app's
//! own Format dialog, and Open in Disks where GNOME Disks is installed.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};
use ox_core::location::{
    is_device_location, is_remote_location, is_smb_location, is_smb_server, is_smb_share_root,
    RECENT_LOCATIONS_URI, RECENT_URI, TRASH_URI,
};
use ox_core::places::{NetworkKind, NetworkLocation};
use ox_core::search::Caching;

use crate::devices::Removal;
use crate::icons::Icon;
use crate::volumes::{MountControls, VolumeKind};

use super::cache_folder::cache_item;
use super::context_menu::{classic_look, more_options_item};
use super::folder_pane::FolderPane;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover, MenuStyle};
use super::sidebar::menu::{compress_to_item, pin_menu};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What a place's menu is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PlaceMenu {
    /// A mounted drive or device in the sidebar, Local Disk included
    /// (`driveMenu`).
    Drive {
        /// Its root.
        uri: String,
        /// A drive or a phone.
        kind: VolumeKind,
        /// What the mount allows.
        controls: MountControls,
    },
    /// A volume still to be mounted (`driveMenu` with a volume).
    Volume {
        /// The volume's id for the mount request.
        id: String,
    },
    /// A network location in the sidebar (`networkLocationMenu`).
    Network(NetworkLocation),
    /// A saved share's card on This PC (`shares()`).
    SavedShare {
        /// The saved share.
        uri: String,
    },
    /// The sidebar's Recycle Bin (Dolphin's Trash place, SIDE-025).
    RecycleBin {
        /// Whether anything is in it; Empty Recycle Bin is disabled if not.
        has_items: bool,
    },
    /// The sidebar's Recent files.
    RecentFiles,
    /// The Recent locations row (SIDE-026).
    RecentLocations,
    /// A Quick access tile of Home: a pinned folder, `editable` when the
    /// user pinned it (SIDE-011), not for a standard folder.
    QuickAccessPin {
        /// The folder it opens.
        uri: String,
        /// Whether Edit… can change it.
        editable: bool,
    },
    /// A row of Home's Recent or Favorites list: the file at `uri`
    /// (HOME-036).
    HomeFile {
        /// The file it stands for.
        uri: String,
        /// Whether it is among the Favorites.
        favorite: bool,
        /// Whether it is a row of Recent, which offers Remove from list.
        recent: bool,
    },
}

/// An item that runs `action` with the place `target`.
fn item(label: &str, glyph: Icon, action: WindowAction, target: &str) -> MenuEntry {
    let item = MenuItem {
        target: Some(target.to_variant()),
        ..MenuItem::new(label, glyph, action)
    };
    MenuEntry::Item(item)
}

/// The window action of `removal`.
pub(super) fn removal_action(removal: Removal) -> WindowAction {
    match removal {
        Removal::Disconnect => WindowAction::Disconnect,
        Removal::Eject => WindowAction::Eject,
        Removal::SafelyRemove => WindowAction::SafelyRemove,
    }
}

/// Disconnect, Eject and Safely remove, as `controls` allow them.
fn removal_items(uri: &str, kind: VolumeKind, controls: MountControls) -> Vec<MenuEntry> {
    let removals = Removal::offered(controls).into_iter();
    let items = removals.map(|removal| {
        item(
            removal.label(kind),
            Icon::ArrowEject,
            removal_action(removal),
            uri,
        )
    });
    items.collect()
}

/// Open in Disks, where GNOME Disks is installed, and Format…, the app's
/// own dialog, for the removable drive at `uri` (DEV-012), as Dolphin
/// offers its partition manager.
fn disks_items(uri: &str, controls: MountControls) -> Vec<MenuEntry> {
    let open = controls.can_open_in_disks.then(|| {
        item(
            ox_core::i18n::gettext_static("Open in Disks"),
            Icon::Settings,
            WindowAction::OpenInDisks,
            uri,
        )
    });
    let format = controls.can_format.then(|| {
        item(
            ox_core::i18n::gettext_static("Format…"),
            Icon::HardDrive,
            WindowAction::FormatDrive,
            uri,
        )
    });
    open.into_iter().chain(format).collect()
}

/// Open in Terminal for the place at `uri`, disabled where no terminal
/// can open there (`terminalMenuItem`): anywhere but a local folder, an
/// SMB share or an SFTP, FTP, WebDAV or NFS place.
fn terminal_item(uri: &str) -> MenuEntry {
    let can_open = terminal_opens_at(uri);
    let terminal = MenuItem::with_text_target(
        &ox_core::i18n::gettext("Open in Terminal"),
        Icon::WindowConsole,
        WindowAction::OpenInTerminalOf,
        uri,
    );
    terminal.disabled_when(!can_open).into()
}

/// Whether a terminal can open at `uri` (`terminalMenuItem`): a local
/// folder, an SMB share or an SFTP, FTP, WebDAV or NFS place, not a
/// server's share list or a page such as Network or Gallery.
pub(super) fn terminal_opens_at(uri: &str) -> bool {
    let is_share = (is_smb_location(uri) || is_remote_location(uri)) && !is_smb_server(uri);
    uri.starts_with("file:") || is_share
}

/// "Cache this folder for search" for the place at `uri`, where it can be
/// indexed.
fn cache_entries(uri: &str, caching: Option<Caching>) -> Vec<MenuEntry> {
    caching
        .map(|caching| cache_item(uri, caching).into())
        .into_iter()
        .collect()
}

/// Why a phone's or a camera's root cannot be copied: it has no path on
/// this computer that a shortcut could point to.
const DEVICE_NOT_COPIED: &str =
    crate::i18n::message_id("A phone or camera cannot be copied. Open it and select what to copy.");

/// Why a drive cannot be renamed from its menu.
const DRIVE_NOT_RENAMED: &str = crate::i18n::message_id("Only a removable drive can be renamed here.");

/// Copy and Rename, the icon strip Windows 11 shows on a drive. Copy
/// copies the drive as Windows Explorer does: a paste of it into a folder
/// makes a shortcut to the drive there, never a copy of what it holds
/// ([`super::drive_shortcuts`]). A phone's or a camera's Copy is greyed
/// out, saying why: its root has no path a shortcut could point to.
/// Rename changes a removable drive's volume label ([`rename_drive_item`]).
fn drive_strip(uri: &str, controls: MountControls) -> Vec<MenuItem> {
    vec![
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Copy"),
            Icon::Copy,
            WindowAction::CopyFolder,
            uri,
        )
        .with_shortcut("Ctrl+C")
        .disabled_because(
            is_device_location(uri),
            ox_core::i18n::gettext_static(DEVICE_NOT_COPIED),
        ),
        rename_drive_item(uri, controls),
    ]
}

/// Rename for the drive at `uri`: it changes the volume label of a
/// removable drive, as Windows' Rename of a drive does, and is grayed out,
/// saying why, for Local Disk and other drives the app cannot relabel.
fn rename_drive_item(uri: &str, controls: MountControls) -> MenuItem {
    if controls.can_format {
        return MenuItem::with_text_target(
            &ox_core::i18n::gettext("Rename"),
            Icon::Rename,
            WindowAction::RenameDrive,
            uri,
        );
    }
    MenuItem::with_text_target(
        &ox_core::i18n::gettext("Rename"),
        Icon::Rename,
        WindowAction::RenameFolder,
        uri,
    )
    .disabled_because(true, ox_core::i18n::gettext_static(DRIVE_NOT_RENAMED))
}

/// Pin to Quick access for the drive at `uri`, or Unpin while it is
/// `pinned`.
fn drive_pin_item(uri: &str, pinned: bool) -> MenuEntry {
    let (label, action) = if pinned {
        ("Unpin from Quick access", WindowAction::Unpin)
    } else {
        ("Pin to Quick access", WindowAction::PinLocation)
    };
    MenuItem::with_text_target(&ox_core::i18n::gettext(label), Icon::Pin, action, uri).into()
}

/// A drive's menu in `style`, in Windows 11's order (checked on Windows 11
/// on 2026-11-07: Copy and Rename in the strip, then Open, Open in new tab,
/// Open in new window, Turn on `BitLocker`, Format…, Pin to Quick access,
/// Pin to Start, Properties and Show more options), without `BitLocker` and
/// Pin to Start, which the app has no counterpart of. Format… shows where
/// the app formats the drive, and Eject, Safely remove or Disconnect
/// where the drive allows them, as Windows lists Eject after Format….
///
/// The classic menu ("Show more options") lists every item: the same, then
/// Open in Terminal and "Cache this folder for search" (`driveMenu`), Open
/// in Disks, and Copy and Rename as rows.
fn drive_menu(
    uri: &str,
    kind: VolumeKind,
    controls: MountControls,
    facts: PlaceFacts,
    style: MenuStyle,
) -> (Vec<MenuEntry>, Vec<MenuItem>) {
    let open = MenuItem::with_text_target(
        &ox_core::i18n::gettext("Open"),
        Icon::Folder,
        WindowAction::GoTo,
        uri,
    )
    .with_shortcut("Enter");
    let mut entries: Vec<MenuEntry> = vec![
        open.into(),
        item(
            ox_core::i18n::gettext_static("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            uri,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            uri,
        ),
    ];
    let format = controls.can_format.then(|| {
        item(
            ox_core::i18n::gettext_static("Format…"),
            Icon::HardDrive,
            WindowAction::FormatDrive,
            uri,
        )
    });
    let removals = removal_items(uri, kind, controls);
    let properties: MenuEntry = MenuItem::with_text_target(
        &ox_core::i18n::gettext("Properties"),
        Icon::Info,
        WindowAction::PropertiesOf,
        uri,
    )
    .with_shortcut("Alt+Enter")
    .into();
    match style {
        MenuStyle::Compact => {
            entries.extend(format);
            entries.extend(removals);
            entries.push(drive_pin_item(uri, facts.pinned));
            entries.push(properties);
            (entries, drive_strip(uri, controls))
        }
        MenuStyle::Classic => {
            entries.push(drive_pin_item(uri, facts.pinned));
            entries.push(terminal_item(uri));
            if !is_device_location(uri) {
                entries.extend(cache_entries(uri, facts.caching));
            }
            let mut drive_tools: Vec<MenuEntry> = format.into_iter().collect();
            drive_tools.extend(disks_items(uri, controls).into_iter().filter(|entry| {
                !matches!(entry, MenuEntry::Item(item) if item.label == ox_core::i18n::gettext_static("Format…"))
            }));
            drive_tools.extend(removals);
            if !drive_tools.is_empty() {
                entries.push(MenuEntry::Divider);
                entries.extend(drive_tools);
            }
            entries.push(MenuEntry::Divider);
            entries.extend(drive_strip(uri, controls).into_iter().map(MenuEntry::from));
            entries.push(MenuEntry::Divider);
            entries.push(properties);
            (entries, Vec::new())
        }
    }
}

/// A Home Quick access tile's menu in `style`, in Windows 11's order
/// (checked on Windows 11 on 2026-11-14): Copy alone in the strip, then
/// Open (Enter), Open in new tab, Open in new window, Unpin from Quick
/// access, Compress to, Copy as path (Ctrl+Shift+C), Properties
/// (Alt+Enter) and Open in Terminal, without Pin to Start, which is
/// Windows' own. A share's root has no Copy, as in the navigation pane.
///
/// The classic menu ("Show more options") is the pin's whole menu, with
/// the app's own items Windows does not list (Open folder with…, "Cache
/// this folder for search", Sign out of server…, Edit…, Previous
/// versions) and Copy as a row.
fn quick_access_pin_menu(
    uri: &str,
    editable: bool,
    facts: PlaceFacts,
    style: MenuStyle,
) -> (Vec<MenuEntry>, Vec<MenuItem>) {
    let target = |label: &str, glyph: Icon, action: WindowAction| -> MenuItem {
        MenuItem::with_text_target(&ox_core::i18n::gettext(label), glyph, action, uri)
    };
    let copy: Vec<MenuItem> = (!is_smb_share_root(uri))
        .then(|| target("Copy", Icon::Copy, WindowAction::CopyFolder).with_shortcut("Ctrl+C"))
        .into_iter()
        .collect();
    match style {
        MenuStyle::Compact => {
            let entries = vec![
                target("Open", Icon::Folder, WindowAction::GoTo)
                    .with_shortcut("Enter")
                    .into(),
                target("Open in new tab", Icon::Add, WindowAction::OpenTab).into(),
                target("Open in new window", Icon::WindowNew, WindowAction::OpenWindow).into(),
                target("Unpin from Quick access", Icon::Pin, WindowAction::Unpin).into(),
                compress_to_item(uri).into(),
                target("Copy as path", Icon::Link, WindowAction::CopyPathOf)
                    .with_shortcut("Ctrl+Shift+C")
                    .into(),
                target("Properties", Icon::Info, WindowAction::PropertiesOf)
                    .with_shortcut("Alt+Enter")
                    .into(),
                MenuEntry::Divider,
                terminal_item(uri),
            ];
            (entries, copy)
        }
        MenuStyle::Classic => {
            let mut entries = pin_menu(uri, facts.caching, editable);
            if !copy.is_empty() {
                entries.push(MenuEntry::Divider);
                entries.extend(copy.into_iter().map(MenuEntry::from));
            }
            (entries, Vec::new())
        }
    }
}

/// `networkLocationMenu`: opening, a computer's Pin to Quick access (or
/// Unpin while `pinned`), Open in Terminal, the cache item,
/// keeping or removing a share, Sign out for SMB, Disconnect for a
/// connected SFTP, FTP, WebDAV or NFS place, and Properties of a mount or
/// a connected share.
fn network_entries(location: &NetworkLocation, caching: Option<Caching>, pinned: bool) -> Vec<MenuEntry> {
    let uri = location.uri.as_str();
    let mut entries = vec![
        item(
            ox_core::i18n::gettext_static("Open"),
            Icon::Folder,
            WindowAction::GoTo,
            uri,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            uri,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            uri,
        ),
    ];
    // A computer's Pin to Quick access, as Windows 11's (checked on build
    // 26200); Windows' Create shortcut is not offered yet.
    if location.kind == NetworkKind::Server {
        entries.push(drive_pin_item(uri, pinned));
    }
    entries.push(terminal_item(uri));
    entries.extend(cache_entries(uri, caching));
    let is_smb = is_smb_location(uri);
    let is_remote = is_remote_location(uri);
    let is_share = (is_smb || is_remote) && !is_smb_server(uri) && location.kind != NetworkKind::Server;
    if is_share && location.is_saved {
        entries.push(item(
            ox_core::i18n::gettext_static("Remove saved location"),
            Icon::Pin,
            WindowAction::RemoveSavedLocation,
            uri,
        ));
    } else if is_share {
        entries.push(item(
            ox_core::i18n::gettext_static("Keep in Network"),
            Icon::Pin,
            WindowAction::KeepInNetwork,
            uri,
        ));
    }
    if is_smb {
        entries.push(item(
            ox_core::i18n::gettext_static("Sign out of server…"),
            Icon::ArrowEject,
            WindowAction::SignOut,
            uri,
        ));
    }
    // SFTP, FTP, WebDAV and NFS keep no OpenXplorer credentials to forget:
    // their connection is ended as a mount, as Dolphin and Files do.
    if is_remote && location.is_connected {
        entries.push(item(
            ox_core::i18n::gettext_static("Disconnect"),
            Icon::ArrowEject,
            WindowAction::Disconnect,
            uri,
        ));
    }
    let is_readable = location.kind == NetworkKind::Mount || (is_share && location.is_connected);
    if is_readable {
        entries.push(item(
            ox_core::i18n::gettext_static("Properties"),
            Icon::Info,
            WindowAction::PropertiesOf,
            uri,
        ));
    }
    entries
}

/// Recent files' menu: the Open items, then Clear recent files, which
/// empties the recent files of the app and the desktop (SAFE-022).
fn recent_files_entries() -> Vec<MenuEntry> {
    vec![
        item(
            ox_core::i18n::gettext_static("Open"),
            Icon::History,
            WindowAction::GoTo,
            RECENT_URI,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            RECENT_URI,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            RECENT_URI,
        ),
        MenuEntry::Divider,
        MenuItem::new(
            &ox_core::i18n::gettext("Clear recent files"),
            Icon::Delete,
            WindowAction::ClearRecentFiles,
        )
        .into(),
    ]
}

/// The Recycle Bin's menu: the Open items, then Empty Recycle Bin,
/// disabled while it is empty (Dolphin's "Empty Trash").
fn recycle_bin_entries(has_items: bool) -> Vec<MenuEntry> {
    let empty = MenuItem::new(
        &ox_core::i18n::gettext("Empty Recycle Bin"),
        Icon::Delete,
        WindowAction::EmptyTrash,
    );
    vec![
        item(
            ox_core::i18n::gettext_static("Open"),
            Icon::Delete,
            WindowAction::GoTo,
            TRASH_URI,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            TRASH_URI,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            TRASH_URI,
        ),
        MenuEntry::Divider,
        empty.disabled_when(!has_items).into(),
    ]
}

/// The Recent locations row's menu: open it, or forget the folders.
fn recent_locations_entries() -> Vec<MenuEntry> {
    let uri = RECENT_LOCATIONS_URI;
    vec![
        item(
            ox_core::i18n::gettext_static("Open"),
            Icon::Clock,
            WindowAction::GoTo,
            uri,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            uri,
        ),
        item(
            ox_core::i18n::gettext_static("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            uri,
        ),
        MenuEntry::Divider,
        MenuItem::new(
            &ox_core::i18n::gettext("Clear recent locations"),
            Icon::DeleteDismiss,
            WindowAction::ClearRecentLocations,
        )
        .into(),
    ]
}

/// What a place's menu shows besides the place itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PlaceFacts {
    /// Whether the place is cached for search; `None` where it cannot be.
    pub caching: Option<Caching>,
    /// Whether the place is pinned to Quick access.
    pub pinned: bool,
}

impl PlaceMenu {
    /// The menu in `style` and, in the compact style, its icon strip: a
    /// drive's as Windows 11 lays it out ([`drive_menu`]); any other
    /// place's lines, with no strip.
    pub(super) fn styled(&self, facts: PlaceFacts, style: MenuStyle) -> (Vec<MenuEntry>, Vec<MenuItem>) {
        match self {
            PlaceMenu::Drive { uri, kind, controls } => drive_menu(uri, *kind, *controls, facts, style),
            PlaceMenu::QuickAccessPin { uri, editable } => {
                quick_access_pin_menu(uri, *editable, facts, style)
            }
            PlaceMenu::HomeFile {
                uri,
                favorite,
                recent,
            } => super::home_page::file_menu(uri, *favorite, *recent, style),
            PlaceMenu::Network(location) => {
                (network_entries(location, facts.caching, facts.pinned), Vec::new())
            }
            _ => (self.entries(facts.caching), Vec::new()),
        }
    }

    /// The location the menu is about, which may be cached for search.
    fn location(&self) -> Option<&str> {
        match self {
            PlaceMenu::Drive { uri, .. }
            | PlaceMenu::SavedShare { uri }
            | PlaceMenu::QuickAccessPin { uri, .. } => Some(uri),
            PlaceMenu::Network(location) => Some(&location.uri),
            PlaceMenu::Volume { .. }
            | PlaceMenu::RecycleBin { .. }
            | PlaceMenu::RecentFiles
            | PlaceMenu::RecentLocations
            | PlaceMenu::HomeFile { .. } => None,
        }
    }

    /// The menu's lines, with the cache item as `caching` says: `None`
    /// where the place cannot be indexed.
    pub(super) fn entries(&self, caching: Option<Caching>) -> Vec<MenuEntry> {
        match self {
            PlaceMenu::Drive { uri, kind, controls } => {
                let facts = PlaceFacts {
                    caching,
                    pinned: false,
                };
                drive_menu(uri, *kind, *controls, facts, MenuStyle::Classic).0
            }
            PlaceMenu::Volume { id } => vec![item(
                ox_core::i18n::gettext_static("Mount volume"),
                Icon::HardDrive,
                WindowAction::MountVolume,
                id,
            )],
            PlaceMenu::Network(location) => network_entries(location, caching, false),
            PlaceMenu::RecycleBin { has_items } => recycle_bin_entries(*has_items),
            PlaceMenu::RecentFiles => recent_files_entries(),
            PlaceMenu::RecentLocations => recent_locations_entries(),
            PlaceMenu::QuickAccessPin { uri, editable } => pin_menu(uri, caching, *editable),
            PlaceMenu::HomeFile {
                uri,
                favorite,
                recent,
            } => super::home_page::file_menu(uri, *favorite, *recent, MenuStyle::Classic).0,
            PlaceMenu::SavedShare { uri } => {
                let mut entries = vec![
                    item(
                        ox_core::i18n::gettext_static("Open"),
                        Icon::Folder,
                        WindowAction::GoTo,
                        uri,
                    ),
                    item(
                        ox_core::i18n::gettext_static("Open in new tab"),
                        Icon::Add,
                        WindowAction::OpenTab,
                        uri,
                    ),
                ];
                entries.extend(cache_entries(uri, caching));
                entries.push(item(
                    ox_core::i18n::gettext_static("Remove saved location"),
                    Icon::Pin,
                    WindowAction::RemoveSavedLocation,
                    uri,
                ));
                entries.push(item(
                    ox_core::i18n::gettext_static("Sign out of server…"),
                    Icon::ArrowEject,
                    WindowAction::SignOut,
                    uri,
                ));
                entries
            }
        }
    }
}

/// Opens `menu` at `x`, `y` in `anchor` in `style`, as the file list
/// opens its right-click menu at the pointer: the compact style ends with
/// "Show more options", which opens the classic menu in its place
/// ([`BrowserWindow::show_place_more_options`]). A menu without items
/// opens nothing, as a This PC card that cannot be removed has no menu.
///
/// The menu belongs to the folder pane around `anchor`, not to the card:
/// This PC and Network are drawn again whenever a drive, a share or a
/// found server changes, which destroys their cards, also while a menu is
/// open or its item runs. The pane stays.
pub(super) fn popup_place_menu(
    menu: &PlaceMenu,
    anchor: &impl IsA<gtk::Widget>,
    x: f64,
    y: f64,
    style: MenuStyle,
) -> Option<MenuPopover> {
    let anchor = anchor.upcast_ref::<gtk::Widget>();
    let owner = menu_owner(anchor);
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let point = anchor
        .compute_point(&owner, &graphene::Point::new(x as f32, y as f32))
        .unwrap_or_else(|| graphene::Point::new(x as f32, y as f32));
    popup_place_menu_in(menu, &owner, point, style, None)
}

/// [`popup_place_menu`] for the menu of `item`, which takes the keyboard
/// back when the menu closes, Escape included, as the file list's items,
/// the tabs and the navigation pane's rows do, also after "Show more
/// options" when that menu closes; unless the item is gone, as when its
/// Open item opened the folder.
pub(super) fn popup_item_place_menu(
    menu: &PlaceMenu,
    item: &impl IsA<gtk::Widget>,
    x: f64,
    y: f64,
    style: MenuStyle,
) -> Option<MenuPopover> {
    let item = item.upcast_ref::<gtk::Widget>();
    let owner = menu_owner(item);
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let point = item
        .compute_point(&owner, &graphene::Point::new(x as f32, y as f32))
        .unwrap_or_else(|| graphene::Point::new(x as f32, y as f32));
    popup_place_menu_in(menu, &owner, point, style, Some(item))
}

/// Opens `menu` in `style` at `point` of `owner`, its folder pane; the
/// `opener`, if any, takes the keyboard back when it closes.
pub(super) fn popup_place_menu_in(
    menu: &PlaceMenu,
    owner: &gtk::Widget,
    point: graphene::Point,
    style: MenuStyle,
    opener: Option<&gtk::Widget>,
) -> Option<MenuPopover> {
    let (entries, strip) = match (menu, owner.root().and_downcast::<BrowserWindow>()) {
        // A Recent row offers what this computer has for its file.
        (
            PlaceMenu::HomeFile {
                uri,
                favorite,
                recent: true,
            },
            Some(window),
        ) => window.home_recent_menu(uri, *favorite, style),
        _ => menu.styled(facts_in(menu, owner), style),
    };
    if entries.is_empty() {
        return None;
    }
    let entries = finish_menu(entries, style, WindowAction::PlaceMoreOptions);
    if let Some(window) = owner.root().and_downcast::<BrowserWindow>() {
        window.remember_place_menu(menu.clone(), owner, point, opener);
    }
    // The pane keeps one place menu and shows it again each time
    // ([`MenuPopover::kept_by`]).
    let popover = MenuPopover::kept_by(owner, PLACE_MENU);
    popover.set_entries(entries);
    popover.set_style_and_strip(style, strip);
    popover.set_offset(0, 0);
    popover.set_position(gtk::PositionType::Bottom);
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let pointing_to = gdk::Rectangle::new(point.x() as i32, point.y() as i32, 1, 1);
    popover.set_pointing_to(Some(&pointing_to));
    let opener = opener.map(ObjectExt::downgrade);
    popover.after_close(move |_| {
        if let Some(opener) = opener.as_ref().and_then(glib::WeakRef::upgrade) {
            // On Wayland the menu's grab takes the button's release, so a
            // tile or row would stay drawn as pressed.
            opener.unset_state_flags(gtk::StateFlags::ACTIVE);
            if opener.is_drawable() {
                opener.grab_focus();
            }
        }
    });
    popover.popup();
    Some(popover)
}

/// `entries` finished as the file list finishes its menu in `style`: the
/// compact menu ends with "Show more options", which runs `more`; the
/// classic one is drawn as the classic file menu, its Windows commands as
/// text alone.
pub(super) fn finish_menu(
    mut entries: Vec<MenuEntry>,
    style: MenuStyle,
    more: WindowAction,
) -> Vec<MenuEntry> {
    match style {
        MenuStyle::Compact => {
            entries.extend([MenuEntry::Divider, more_options_item(more)]);
            entries
        }
        MenuStyle::Classic => classic_look(entries),
    }
}

/// `entries` in `style`, for a menu that has nothing behind "Show more
/// options": as they are in the compact style, drawn as the classic file
/// menu in the classic one.
pub(super) fn classic_or_compact(entries: Vec<MenuEntry>, style: MenuStyle) -> Vec<MenuEntry> {
    match style {
        MenuStyle::Compact => entries,
        MenuStyle::Classic => classic_look(entries),
    }
}

/// The name of the place menu a folder pane keeps.
const PLACE_MENU: &str = "place-menu";

/// A place menu as it was opened, for "Show more options".
#[derive(Debug)]
pub(super) struct OpenPlaceMenu {
    menu: PlaceMenu,
    owner: glib::WeakRef<gtk::Widget>,
    point: graphene::Point,
    /// The item that takes the keyboard back, if any.
    opener: Option<glib::WeakRef<gtk::Widget>>,
}

impl BrowserWindow {
    /// Keeps `menu`, opened at `point` of `owner` from `opener`, for
    /// "Show more options".
    fn remember_place_menu(
        &self,
        menu: PlaceMenu,
        owner: &gtk::Widget,
        point: graphene::Point,
        opener: Option<&gtk::Widget>,
    ) {
        self.imp().place_menu.replace(Some(OpenPlaceMenu {
            menu,
            owner: owner.downgrade(),
            point,
            opener: opener.map(ObjectExt::downgrade),
        }));
    }

    /// "Show more options" of a place's menu: its classic menu where the
    /// compact one was.
    pub(super) fn show_place_more_options(&self) {
        let open = self.imp().place_menu.borrow_mut().take();
        let Some(open) = open else {
            return;
        };
        if let Some(owner) = open.owner.upgrade() {
            let opener = open.opener.as_ref().and_then(glib::WeakRef::upgrade);
            popup_place_menu_in(
                &open.menu,
                &owner,
                open.point,
                MenuStyle::Classic,
                opener.as_ref(),
            );
        }
    }
}

/// The widget a card's menu is parented to: the folder pane the card is
/// drawn in, which outlives the card, or the card itself outside a pane.
fn menu_owner(card: &gtk::Widget) -> gtk::Widget {
    card.ancestor(FolderPane::static_type())
        .unwrap_or_else(|| card.clone())
}

/// Whether the place of `menu` is cached for search, as the window of
/// `widget` knows it.
pub(super) fn caching_in(menu: &PlaceMenu, widget: &impl IsA<gtk::Widget>) -> Option<Caching> {
    let window = widget.root().and_downcast::<BrowserWindow>()?;
    window.caching_of(menu.location()?)
}

/// What the window of `widget` knows of the place of `menu`: whether it
/// is cached for search and whether it is pinned to Quick access.
pub(super) fn facts_in(menu: &PlaceMenu, widget: &impl IsA<gtk::Widget>) -> PlaceFacts {
    let window = widget.root().and_downcast::<BrowserWindow>();
    let pinned = window
        .as_ref()
        .zip(menu.location())
        .is_some_and(|(window, uri)| window.is_pinned(uri));
    PlaceFacts {
        caching: caching_in(menu, widget),
        pinned,
    }
}

#[cfg(test)]
mod tests;
