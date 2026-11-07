// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar's menus: New, Sort, View, More options and the
//! appearance choices.
//!
//! Ports the Sort and View menus of `setup()`, the More options menu and
//! `appearanceMenu` in `v2.0.0:desktop/ui/app.js`, in their order; the New
//! menu, once `openNewMenu`, is Windows 11's. Each item runs a window or
//! application action; the choices and toggles show a check mark while
//! their action's state matches.

use ox_core::grouping::GroupBy;
use ox_core::i18n::gettext;
use ox_core::settings::Theme;

use crate::folder_view::grid::IconSize;
use crate::folder_view::sort_roles::SortRole;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::icons::Icon;
use crate::text_size::Step;
use crate::window::folder_pane::FolderView;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;

/// A menu line that runs `action`.
fn item(label: &str, glyph: Icon, action: WindowAction) -> MenuEntry {
    MenuItem::new(label, glyph, action).into()
}

/// The New menu, which the folder background's "New…" opens too, laid
/// out as Windows 11's: Folder and Shortcut, then the document types a
/// stock Windows 11 registers, then the user's `templates`, Linux's
/// counterpart of further registered types (OPS-003). Every file is made
/// at once and renamed in place; Shortcut opens the link dialog (OPS-004).
pub(in crate::window) fn new_menu(templates: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuItem::new(&gettext("Folder"), Icon::FolderAdd, WindowAction::NewFolder)
            .with_shortcut("Ctrl+Shift+N")
            .into(),
        item(&gettext("Shortcut"), Icon::Link, WindowAction::NewLink),
        MenuEntry::Divider,
        item(
            &gettext("Bitmap image"),
            Icon::Image,
            WindowAction::NewBitmapImage,
        ),
        item(
            &gettext("Rich Text Document"),
            Icon::Document,
            WindowAction::NewRichTextDocument,
        ),
        item(
            &gettext("Text Document"),
            Icon::DocumentText,
            WindowAction::NewTextDocument,
        ),
        item(
            &gettext("Compressed (zipped) Folder"),
            Icon::FolderZip,
            WindowAction::NewCompressedFolder,
        ),
    ];
    if !templates.is_empty() {
        entries.push(MenuEntry::Divider);
        entries.extend(templates);
    }
    entries
}

/// The Sort menu's item for `column`: no glyph, as Windows 11's Sort menu
/// draws none, and a dot while it is the sort.
fn column_item(column: SortColumn) -> MenuEntry {
    MenuItem::choice(
        column.label(),
        Icon::ArrowSort,
        WindowAction::Sort,
        column.as_str(),
    )
    .without_glyph()
    .into()
}

/// The Sort menu's item for `direction`.
fn direction_item(label: &str, direction: SortDirection) -> MenuEntry {
    MenuItem::choice(
        label,
        Icon::ArrowSort,
        WindowAction::Direction,
        direction.as_str(),
    )
    .without_glyph()
    .into()
}

/// The Sort menu's item for a further key (VIEW-019).
fn role_item(role: SortRole) -> MenuEntry {
    MenuItem::choice(role.label(), Icon::ArrowSort, WindowAction::Sort, role.as_str())
        .without_glyph()
        .into()
}

/// The Sort menu, laid out as Windows Explorer's: Name, Date modified and
/// Type, then More with Size and Dolphin's further keys; the direction,
/// with an item each where app.js had one that flips it; then Group by,
/// whose choices group apart from the sort (VIEW-022), and the app's own
/// Folders first. As in Windows 11, no row has a glyph: a dot marks the
/// chosen column, direction and grouping, and a check mark Folders first.
pub(in crate::window) fn sort_menu() -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = [SortColumn::Name, SortColumn::Modified, SortColumn::Type]
        .into_iter()
        .map(column_item)
        .collect();
    let mut more = vec![column_item(SortColumn::Size)];
    more.extend(SortRole::ALL.into_iter().map(role_item));
    entries.extend([
        MenuItem::submenu(&gettext("More"), Icon::ArrowSort, WindowAction::Sort, more)
            .without_glyph()
            .into(),
        MenuEntry::Divider,
        direction_item(&gettext("Ascending"), SortDirection::Ascending),
        direction_item(&gettext("Descending"), SortDirection::Descending),
        MenuEntry::Divider,
        MenuItem::submenu(
            &gettext("Group by"),
            Icon::TextBulletList,
            WindowAction::GroupBy,
            group_by_menu(),
        )
        .without_glyph()
        .into(),
        MenuEntry::Divider,
        MenuItem::toggle(
            &ox_core::i18n::gettext("Folders first"),
            Icon::Folder,
            WindowAction::FoldersFirst,
        )
        .without_glyph()
        .into(),
    ]);
    entries
}

/// The Group by submenu: every way of grouping, the current one checked.
pub(in crate::window) fn group_by_menu() -> Vec<MenuEntry> {
    GroupBy::ALL.into_iter().map(group_item).collect()
}

/// The Group by submenu's item for `group_by`.
fn group_item(group_by: GroupBy) -> MenuEntry {
    MenuItem::choice(
        ox_core::i18n::gettext_static(group_by.label()),
        Icon::TextBulletList,
        WindowAction::GroupBy,
        group_by.as_str(),
    )
    .without_glyph()
    .into()
}

/// The View menu's item for `view`. Windows 11's View menu shows no
/// shortcut text, so neither does this one; Explorer's Ctrl+Shift+1 to 8
/// still choose the views (VIEW-009).
fn view_item(label: &str, glyph: Icon, view: FolderView) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::View, view.as_str()).into()
}

/// The View menu's item for a text-size `step`, whose keys (Ctrl++, Ctrl+−
/// and Ctrl+0) work without being shown, as in the rest of the menu.
fn text_size_item(label: &str, glyph: Icon, step: Step) -> MenuEntry {
    MenuItem::new(label, glyph, WindowAction::TextSize(step)).into()
}

/// The glyph Windows 11's View menu draws for `size`.
fn icon_size_glyph(size: IconSize) -> Icon {
    if size == IconSize::MEDIUM {
        Icon::Desktop
    } else if size == IconSize::SMALL {
        Icon::Grid
    } else {
        Icon::RectangleLandscape
    }
}

/// The View menu, as Windows 11 File Explorer's: the icon sizes from the
/// largest, List, Details, Tiles and Content (VIEW-071); the Details pane, with a dot while it is
/// open, and the Preview pane, which takes its place; Show, with the
/// navigation pane, Compact view, item check boxes, File name extensions
/// and Hidden items; then Dolphin's Split view, Folder tree and Terminal,
/// at the top level where they were before Show existed; then the text
/// size.
pub(in crate::window) fn view_menu() -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = IconSize::NAMED
        .into_iter()
        .filter_map(|size| {
            let label = size.label()?;
            Some(view_item(label, icon_size_glyph(size), FolderView::Icons(size)))
        })
        .collect();
    entries.extend([
        view_item(&gettext("List"), Icon::TextColumnTwo, FolderView::Compact),
        view_item(&gettext("Details"), Icon::TextBulletList, FolderView::Details),
        view_item(&gettext("Tiles"), Icon::AppsListDetail, FolderView::Tiles),
        view_item(&gettext("Content"), Icon::ContentView, FolderView::Content),
        MenuEntry::Divider,
        // Windows 11 marks the open pane with a dot, as Details and its
        // Preview pane are one choice there.
        MenuItem::toggle(
            &gettext("Details pane"),
            Icon::PanelRight,
            WindowAction::DetailsPane,
        )
        .with_dot_mark()
        .into(),
        // The same pane showing only the preview; choosing one closes the
        // other, as in Windows (VIEW-070).
        MenuItem::toggle(
            &gettext("Preview pane"),
            Icon::PanelRightGallery,
            WindowAction::PreviewPane,
        )
        .with_dot_mark()
        .into(),
        MenuItem::submenu(&gettext("Show"), Icon::Eye, WindowAction::Hidden, show_menu()).into(),
        MenuEntry::Divider,
        MenuItem::toggle(
            &ox_core::i18n::gettext("Split view"),
            Icon::PanelRight,
            WindowAction::SplitView,
        )
        .into(),
        MenuItem::toggle(
            &gettext("Folder tree"),
            Icon::Organization,
            WindowAction::FolderTree,
        )
        .into(),
        // Dolphin's Terminal panel embeds Konsole; VTE for GTK 4 is not
        // available everywhere the app ships, so this opens the desktop's
        // terminal in the folder shown instead (OPEN-022).
        MenuItem::new(
            &gettext("Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenTerminal,
        )
        .into(),
        MenuEntry::Divider,
        text_size_item(&gettext("Larger text"), Icon::Add, Step::Increase),
        text_size_item(&gettext("Smaller text"), Icon::Subtract, Step::Decrease),
        text_size_item(&gettext("Reset text size"), Icon::ArrowReset, Step::Reset),
    ]);
    entries
}

/// View > Show, as Windows 11 File Explorer's: the navigation pane,
/// Compact view, item check boxes, File name extensions and Hidden items,
/// the hidden-files toggle that Ctrl+H and Folder Options change too
/// (VIEW-023), each with a check mark while on.
fn show_menu() -> Vec<MenuEntry> {
    vec![
        MenuItem::toggle(
            &gettext("Navigation pane"),
            Icon::PanelLeft,
            WindowAction::Sidebar,
        )
        .into(),
        MenuEntry::Divider,
        MenuItem::toggle(
            &gettext("Compact view"),
            Icon::ArrowAutofitHeight,
            WindowAction::CompactDensity,
        )
        .into(),
        MenuEntry::Divider,
        MenuItem::toggle(
            &gettext("Item check boxes"),
            Icon::CheckboxChecked,
            WindowAction::ItemCheckBoxes,
        )
        .into(),
        // Hidden for known file types by default, as in Windows (VIEW-069).
        MenuItem::toggle(
            &gettext("File name extensions"),
            Icon::Document,
            WindowAction::FileNameExtensions,
        )
        .into(),
        MenuItem::toggle(&gettext("Hidden items"), Icon::Eye, WindowAction::Hidden).into(),
    ]
}

/// The appearance menu's item for `theme`.
fn theme_item(label: &str, glyph: Icon, theme: Theme) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::Theme, theme.as_str()).into()
}

/// The three appearance choices (`appearanceMenu`).
pub(super) fn appearance_items() -> [MenuEntry; 3] {
    [
        theme_item(&gettext("Light appearance"), Icon::WeatherSunny, Theme::Light),
        theme_item(&gettext("Dark appearance"), Icon::WeatherMoon, Theme::Dark),
        theme_item(&gettext("Use system appearance"), Icon::Desktop, Theme::System),
    ]
}

/// Where the window is, for what "…" lists there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum MorePlace {
    /// A folder, or any location that is not a page: Undo and Redo, Pin
    /// or Unpin, then the selection commands.
    Folder,
    /// This PC: the network drive commands in place of Pin, as Windows
    /// 11's "…" lists them, and Properties opening the system information
    /// app.
    ThisPc,
    /// Network: the same network drive commands; its Properties, as a
    /// page's, has nothing to describe.
    Network,
}

/// Whether the folder shown is pinned to Quick access, which decides
/// whether "…" pins it or unpins it, as Windows 11 File Explorer's does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum FolderPin {
    /// Not pinned, or a page that cannot be: "Pin to Quick access".
    Unpinned,
    /// Pinned as the Quick access pin at this location: "Unpin from
    /// Quick access", which removes that pin as the sidebar's Unpin does.
    Pinned(String),
}

/// A connected network share that Disconnect network drive can end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct NetworkDrive {
    /// Its name, as the sidebar shows it.
    pub label: String,
    /// The location `win.disconnect` unmounts.
    pub uri: String,
}

/// What the "…" menu depends on: where the window is, whether the folder
/// shown is pinned, what Undo and Redo would do, and the connected
/// shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct MoreMenuState {
    /// Where the window is.
    pub place: MorePlace,
    /// Whether the folder shown is pinned.
    pub pin: FolderPin,
    /// Whether there is something to undo.
    pub can_undo: bool,
    /// Whether there is something to redo.
    pub can_redo: bool,
    /// The connected network shares.
    pub network_drives: Vec<NetworkDrive>,
    /// Whether the desktop has a system information app for This PC's
    /// Properties to open.
    pub system_information: bool,
}

impl Default for MoreMenuState {
    /// A folder that is not pinned, with nothing to undo or redo.
    fn default() -> Self {
        Self {
            place: MorePlace::Folder,
            pin: FolderPin::Unpinned,
            can_undo: false,
            can_redo: false,
            network_drives: Vec::new(),
            system_information: false,
        }
    }
}

/// Why Disconnect network drive cannot run.
const NO_NETWORK_DRIVE: &str = crate::i18n::message_id("No network drive is connected.");

/// The More options ("…") menu, laid out as Windows 11 File Explorer's.
///
/// In a folder: Undo and Redo while each can run, named plainly as
/// Windows names them (the folder's own menu names the step); Pin to Quick access or Unpin from
/// Quick access; Select all, Select none, Invert selection and the app's
/// Select items matching…; then Properties, of the selection or the
/// folder, and Options. In This PC and Network, in Windows' order: Add a
/// network location (the connect dialog with a Type list, at a web
/// address, as Windows' wizard adds websites and FTP sites), Map network
/// drive (Windows' dialog) and Disconnect network drive, in place of Pin. Windows' Connect to a media
/// server has no `GVfs` backend to run on Linux, so it is not offered.
///
/// The rest of what "…" used to list moved where Windows has it (the
/// user's request of 2026-11-03, CMD-007): New window to the title bar's
/// window menu, Ctrl+N and a folder's "Open in new window"; Settings to
/// the bar's gear and Ctrl+,; Default
/// file explorer to Settings > Default apps; Cache this folder for search
/// to Settings > Search, the search strip and the folder's menu; the
/// appearances to Settings > Appearance; Show hidden files to View >
/// Show > Hidden items; Keyboard shortcuts, Help, License & source and
/// About this build to Settings > About, with F1 and Ctrl+? as before.
/// Map network drive is also in the navigation pane's This PC and Network
/// menus; the pane no longer has a button for it, as Windows' has none.
pub(super) fn more_menu(state: &MoreMenuState) -> Vec<MenuEntry> {
    let mut entries = match state.place {
        MorePlace::Folder => folder_commands(state),
        MorePlace::ThisPc | MorePlace::Network => network_drive_commands(&state.network_drives),
    };
    entries.extend([
        MenuEntry::Divider,
        // No shortcut text, as Windows 11's "…" shows none; Ctrl+A still
        // selects all. The three glyphs are original icons made to match
        // Windows', blue squares included (no Fluent glyph has them).
        item(&gettext("Select all"), Icon::SelectAll24, WindowAction::SelectAll),
        item(
            &gettext("Select none"),
            Icon::SelectNone24,
            WindowAction::SelectNone,
        ),
        item(
            &gettext("Invert selection"),
            Icon::InvertSelection24,
            WindowAction::InvertSelection,
        ),
        item(
            &gettext("Select items matching…"),
            Icon::Search,
            WindowAction::SelectMatching,
        ),
        MenuEntry::Divider,
    ]);
    // This PC's Properties opens the system information app, as Windows'
    // opens the system's properties; without one it is left out, not
    // greyed (the user's decision, 2026-11-04).
    match (state.place, state.system_information) {
        (MorePlace::ThisPc, true) => entries.push(item(
            &gettext("Properties"),
            Icon::Wrench,
            WindowAction::SystemProperties,
        )),
        (MorePlace::ThisPc, false) => {}
        (MorePlace::Folder | MorePlace::Network, _) => {
            entries.push(item(
                &gettext("Properties"),
                Icon::Wrench,
                WindowAction::Properties,
            ));
        }
    }
    // Last, as in Windows 11's "…" menu.
    entries.push(item(
        &gettext("Options"),
        Icon::Options24,
        WindowAction::FolderOptions,
    ));
    entries
}

/// A folder's first commands: Undo and Redo while each can run, then Pin
/// or Unpin.
fn folder_commands(state: &MoreMenuState) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    // Plainly "Undo" and "Redo", as Windows 11's "…" names them; the
    // folder's own menu names the step.
    // Windows 11 draws Undo, Redo and Pin in blue.
    if state.can_undo {
        entries.push(
            MenuItem::new(&gettext("Undo"), Icon::ArrowUndo, WindowAction::Undo)
                .with_accent_glyph()
                .into(),
        );
    }
    if state.can_redo {
        // Windows 11 draws Redo as Undo's arrow turned the other way.
        entries.push(
            MenuItem::new(&gettext("Redo"), Icon::ArrowUndo, WindowAction::Redo)
                .mirrored()
                .with_accent_glyph()
                .into(),
        );
    }
    if !entries.is_empty() {
        entries.push(MenuEntry::Divider);
    }
    let pin = match &state.pin {
        FolderPin::Unpinned => MenuItem::new(
            &gettext("Pin to Quick access"),
            Icon::Pin,
            WindowAction::PinFolder,
        ),
        FolderPin::Pinned(uri) => MenuItem::with_text_target(
            &gettext("Unpin from Quick access"),
            Icon::Pin,
            WindowAction::Unpin,
            uri,
        ),
    };
    let pin = pin.with_accent_glyph().into();
    entries.push(pin);
    entries
}

/// This PC's and Network's first commands: Add a network location, Map
/// network drive and Disconnect network drive, in Windows 11's order.
fn network_drive_commands(drives: &[NetworkDrive]) -> Vec<MenuEntry> {
    // In Windows 11's order, with icons drawn to match its glyphs.
    vec![
        item(
            &gettext("Add a network location"),
            Icon::AddNetworkLocation24,
            WindowAction::AddNetworkLocation,
        ),
        item(
            &gettext("Map network drive"),
            Icon::MapNetworkDrive24,
            WindowAction::MapNetworkLocation,
        ),
        disconnect_network_drive(drives).into(),
    ]
}

/// "Disconnect network drive", which lists `drives` to disconnect, or
/// says why it cannot run while none is connected; "…" and the
/// navigation pane's This PC and Network menus share it.
pub(in crate::window) fn disconnect_network_drive(drives: &[NetworkDrive]) -> MenuItem {
    if drives.is_empty() {
        return MenuItem::new(
            &gettext("Disconnect network drive"),
            Icon::DisconnectNetworkDrive24,
            WindowAction::Disconnect,
        )
        .disabled_because(true, ox_core::i18n::gettext_static(NO_NETWORK_DRIVE));
    }
    let choices = drives
        .iter()
        .map(|drive| {
            MenuItem::with_text_target(&drive.label, Icon::Server, WindowAction::Disconnect, &drive.uri)
                .into()
        })
        .collect();
    MenuItem::submenu(
        &gettext("Disconnect network drive"),
        Icon::DisconnectNetworkDrive24,
        WindowAction::Disconnect,
        choices,
    )
}

#[cfg(test)]
mod tests {
    use gtk::glib;

    use super::*;
    use crate::window::menu_popover::MenuAction;

    /// The labels of the items of `entries` that run `action`.
    fn labels_running(entries: &[MenuEntry], action: WindowAction) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) if item.action == MenuAction::Window(action) => {
                    Some(item.label.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Folder Options is "…" > Options, as in Windows 11, and the View
    /// menu no longer opens Dolphin's display style dialog.
    ///
    /// parity: VIEW-020
    #[test]
    fn options_in_the_more_menu_opens_folder_options() {
        assert_eq!(
            labels_running(&more_menu(&MoreMenuState::default()), WindowAction::FolderOptions),
            ["Options"]
        );
        assert!(labels_running(&view_menu(), WindowAction::FolderOptions).is_empty());
        let view_labels: Vec<String> = view_menu()
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item.label.clone()),
                MenuEntry::Divider => None,
            })
            .collect();
        assert!(!view_labels.iter().any(|label| label.contains("display style")));
    }

    /// Disconnect network drive lists each connected share, running
    /// `win.disconnect` on it, and is disabled, saying why, while none is.
    ///
    /// parity: CMD-007, DEV-006
    #[test]
    fn disconnect_network_drive_lists_the_connected_shares_or_says_why_not() {
        let find = |entries: Vec<MenuEntry>| {
            entries.into_iter().find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == "Disconnect network drive" => Some(item),
                _ => None,
            })
        };
        let state = MoreMenuState {
            place: MorePlace::ThisPc,
            ..MoreMenuState::default()
        };
        let none = find(more_menu(&state)).expect("This PC has it");
        assert!(none.submenu.is_empty());
        assert_eq!(none.disabled_reason, Some("No network drive is connected."));

        let state = MoreMenuState {
            network_drives: vec![NetworkDrive {
                label: "Backup (S:)".to_owned(),
                uri: "smb://fileserver/backup".to_owned(),
            }],
            ..state
        };
        let some = find(more_menu(&state)).expect("This PC has it");
        let choices: Vec<(String, MenuAction, Option<String>)> = some
            .submenu
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some((
                    item.label.clone(),
                    item.action,
                    item.target.as_ref().and_then(glib::Variant::get::<String>),
                )),
                MenuEntry::Divider => None,
            })
            .collect();
        assert_eq!(
            choices,
            [(
                "Backup (S:)".to_owned(),
                MenuAction::Window(WindowAction::Disconnect),
                Some("smb://fileserver/backup".to_owned())
            )]
        );
        assert!(
            find(more_menu(&MoreMenuState::default())).is_none(),
            "a folder has Pin instead"
        );
    }
}
