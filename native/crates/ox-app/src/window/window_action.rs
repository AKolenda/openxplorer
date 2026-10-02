// SPDX-License-Identifier: AGPL-3.0-only
//! The names of the window's actions (`win.*`).
//!
//! Buttons, menus, sidebar rows and keyboard shortcuts run the command
//! handlers of `v2.0.0:desktop/ui/app.js` as window actions, by name. GTK ignores
//! a name it does not know, so a misspelt name would leave a control that
//! silently does nothing. [`WindowAction`] keeps every name in one table,
//! which turns such a typo into a compile error. The templates in
//! `resources/ui/` therefore name no action: their buttons get one through
//! [`WindowAction::assign_to`]. [`super::actions`] registers them.

use gtk::glib;
use gtk::prelude::*;

use crate::text_size::Step;

/// A window action, as the window registers it and widgets name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowAction {
    /// Opens a tab on the home folder (Ctrl+T).
    NewTab,
    /// Closes the active tab (Ctrl+W).
    CloseTab,
    /// Shows the next tab, wrapping around (Ctrl+Tab).
    NextTab,
    /// Shows the previous tab, wrapping around (Ctrl+Shift+Tab).
    PreviousTab,
    /// Closes the window after asking whether it may close: the caption's
    /// Close button.
    CloseWindow,
    /// Shows the tab whose id is the `u64` target.
    SelectTab,
    /// Closes the tab whose id is the `u64` target (its close button).
    CloseTabById,
    /// Shows the tab whose number, counted from 1, is the `u32` target;
    /// 0 shows the last tab (Alt+1…Alt+9, Alt+0).
    ShowTabNumber,
    /// Closes every tab but the one whose id is the `u64` target.
    CloseOtherTabs,
    /// Reopens the most recently closed tab (Ctrl+Shift+T).
    ReopenClosedTab,
    /// Reopens the closed tab at the `u32` target, 0 being the most
    /// recent.
    RestoreClosedTab,
    /// Opens every selected folder in a tab of its own, behind the active
    /// one (Open in new tabs).
    OpenSelectionInTabs,
    /// Opens the location in the string target in a new tab in front.
    OpenTab,
    /// Opens the location in the string target in a new tab behind the
    /// active one (a middle-click).
    OpenTabBackground,
    /// Opens the location in the string target in a new window ("Open in
    /// new window" of a place's menu).
    OpenWindow,
    /// Back in the active tab's history (Alt+Left).
    Back,
    /// Forward in the active tab's history (Alt+Right).
    Forward,
    /// Opens the folder that contains the current one (Alt+Up).
    Up,
    /// Opens the home folder in the active tab (Alt+Home).
    Home,
    /// Moves the active tab's history by the `i32` target, skipping the
    /// steps between (the Back and Forward menus).
    GoHistory,
    /// Lists the current folder again (F5, Ctrl+R).
    Refresh,
    /// Makes the address editable (Ctrl+L, Alt+D).
    Location,
    /// Edits the address with the typed history listed (F4, the chevron;
    /// NAV-043).
    AddressHistory,
    /// Copies the current location as the address bar shows it.
    CopyAddress,
    /// Opens the clipboard's text as a typed address.
    PasteAddress,
    /// Keeps the address editable text instead of crumbs (NAV-029).
    EditableLocation,
    /// Shows the full path in the crumbs (NAV-024).
    ShowFullPath,
    /// Opens the subfolder menu of a crumb; the target is the folder, the
    /// subfolder shown in bold and the first one listed (NAV-020).
    CrumbSubfolders,
    /// Goes to the folder beside a crumb's; the target is the folder and
    /// the step (NAV-022).
    CrumbSibling,
    /// Moves keyboard focus to the search box (Ctrl+F).
    Search,
    /// Moves the active tab to the location in the string target.
    GoTo,
    /// Mounts the volume whose id is the string target, then opens it.
    MountVolume,
    /// Opens the SMB server or share typed on the Network page.
    OpenServerAddress,
    /// Opens the one selected item (Enter).
    Open,
    /// Selects every item (Ctrl+A).
    SelectAll,
    /// Clears the selection.
    SelectNone,
    /// Selects exactly the items that were not selected.
    InvertSelection,
    /// Asks for a wildcard pattern and selects the items it matches.
    SelectMatching,
    /// Pins the one selected folder to Quick access.
    PinSelected,
    /// Pins the current folder to Quick access.
    PinFolder,
    /// Copies the path of the selection, or of the folder.
    CopyPath,
    /// Shows what this build is.
    /// Opens the offline user manual at the topic of what the window
    /// shows (F1, CMD-033).
    Help,
    /// Opens the keyboard shortcuts window (Ctrl+?, CMD-032).
    KeyboardShortcuts,
    About,
    /// Opens the folder view's context menu from the keyboard.
    ContextMenu,
    /// The folder view: `details`, or an icon size (Ctrl+Shift+1 to 4).
    View,
    /// Shows or hides hidden files (Ctrl+H).
    Hidden,
    /// Shows or hides the details pane.
    DetailsPane,
    /// Switches the details pane option or field named by the string
    /// target (the pane's menu).
    DetailsPaneOption,
    /// Shows or hides the details column whose key is the string target
    /// (the column titles' menu, VIEW-033).
    DetailsColumn,
    /// Stops the listing of the folder shown (VIEW-049).
    Stop,
    /// Shows or hides the navigation pane (F9, SIDE-024).
    Sidebar,
    /// Splits the tab into two panes, or closes its active pane (F3,
    /// VIEW-059).
    SplitView,
    /// Shows or hides the folder tree (F7, SIDE-028).
    FolderTree,
    /// Switches the folder tree option named by the string target (its
    /// menu).
    FolderTreeOption,
    /// The sidebar's icon size: `0` (automatic), `16`, `22`, `32` or `48`.
    SidebarIconSize,
    /// Lists the hidden sidebar rows, dimmed (SIDE-010).
    SidebarShowAll,
    /// Hides the sidebar section whose key is the string target.
    HideSection,
    /// Shows the hidden sidebar section whose key is the string target.
    ShowSection,
    /// Hides the sidebar place at the string target (SIDE-010).
    HidePlace,
    /// Shows the hidden place in the string target again: a standard
    /// folder or a place hidden with Hide.
    ShowPlace,
    /// The key the listing sorts by: a details column or a further key.
    Sort,
    /// Whether the details view sorts ascending or descending.
    Direction,
    /// Shows the items in groups by the sort key (VIEW-022).
    Groups,
    /// Lists folders before files.
    FoldersFirst,
    /// Opens the Adjust View Display Style dialog (VIEW-021).
    ViewProperties,
    /// The light, dark or system appearance.
    Theme,
    /// Makes text larger, smaller or its default size (Ctrl+plus, minus
    /// and 0).
    TextSize(Step),
    /// Returns every window's sidebar and columns to their default widths
    /// (Settings > Appearance).
    ResetLayout,
    /// New ▸ Folder (Ctrl+Shift+N).
    NewFolder,
    /// New ▸ Text document.
    NewTextDocument,
    /// New ▸ File….
    NewFile,
    /// New ▸ Markdown document.
    NewMarkdownDocument,
    /// New ▸ CSV file.
    NewCsvFile,
    /// New ▸ JSON file.
    NewJsonFile,
    /// New ▸ HTML document.
    NewHtmlDocument,
    /// New ▸ From template….
    NewFromTemplate,
    /// Opens the New from template dialog with the template whose id is
    /// the string target chosen: a user template listed in the New menu
    /// (OPS-003).
    NewFromUserTemplate,
    /// New ▸ Link to file or folder… (OPS-004).
    NewLink,
    /// Cut (Ctrl+X).
    Cut,
    /// Copy (Ctrl+C).
    Copy,
    /// Paste (Ctrl+V).
    Paste,
    /// Pastes into the folder in the string target, the one selected
    /// folder (CMD-019).
    PasteInto,
    /// Rename (F2): asks for a new name for the one selected item.
    Rename,
    /// Delete: Move to Trash, or Delete permanently where the folder has
    /// no Trash, after asking.
    Trash,
    /// Shift+Delete: deletes the selection permanently, after asking.
    DeletePermanently,
    /// Copies each selected item next to itself.
    Duplicate,
    /// Cut of the folder in the string target (the folder tree's menu).
    CutFolder,
    /// Copy of the folder in the string target.
    CopyFolder,
    /// Pastes into the folder tree destination in the string target.
    PasteIntoFolder,
    /// Asks for a new name for the folder in the string target.
    RenameFolder,
    /// Moves the folder in the string target to the Trash, after asking.
    TrashFolder,
    /// Deletes the folder in the string target permanently, after asking.
    DeleteFolder,
    /// Reverses the newest file operation (Ctrl+Z).
    Undo,
    /// Takes the newest Undo back (Ctrl+Shift+Z, Ctrl+Y).
    Redo,
    /// Stops the running file operation (the transfer panel's Cancel).
    CancelOperation,
    /// Puts the selected Recycle Bin items back where they came from.
    Restore,
    /// Deletes everything in the Recycle Bin, after asking.
    EmptyRecycleBin,
    /// The same from the sidebar's Recycle Bin, wherever the window is.
    EmptyTrash,
    /// Empties the recent files of the app and the desktop, from the
    /// sidebar's Recent files (SAFE-022).
    ClearRecentFiles,
    /// Forgets the recently visited folders (SIDE-026).
    ClearRecentLocations,
    /// Opens the New menu where the last context menu opened (the folder
    /// background's "New…").
    ShowNewMenu,
    /// Opens the classic context menu where the compact one was ("Show
    /// more options").
    ShowMoreOptions,
    /// Opens the Sort menu where the folder's context menu was (its
    /// "Sort by", CMD-012).
    ShowSortMenu,
    /// Opens the View menu where the folder's context menu was (its
    /// "View", CMD-012).
    ShowViewMenu,
    /// Removes the Quick access pin of the location in the string target.
    Unpin,
    /// Asks for a label and a location and pins them (SIDE-031).
    AddPlace,
    /// Asks for a new label and location for the pin in the string target
    /// (SIDE-011).
    EditPin,
    /// Shows the menu of open windows (the tab menu's "Open windows…").
    OpenWindows,
    /// Moves a tab into a window of its own.
    MoveTabToNewWindow,
    /// Lists the other open windows to move a tab into ("Move tab to
    /// window…").
    MoveTabToWindow,
    /// Moves a tab into another open window; the target is the tab's id
    /// and the window's.
    MoveTabIntoWindow,
    /// Runs the drop the drop menu asks about as the string target says:
    /// `copy`, `move`, `link` or `cancel`.
    DropChoice,
    /// Opens the connect dialog for a network share.
    MapNetworkLocation,
    /// Looks for SMB servers that advertise themselves.
    DiscoverServers,
    /// Stops looking for servers.
    StopDiscovery,
    /// Saves the network location in the string target under Network
    /// ("Keep in Network").
    KeepInNetwork,
    /// Removes the saved network location in the string target; it stays
    /// mounted and its credentials stay saved.
    RemoveSavedLocation,
    /// Signs out of the server of the location in the string target.
    SignOut,
    /// Unmounts the drive or device that holds the location in the string
    /// target.
    Disconnect,
    /// Ejects the medium that holds the location in the string target.
    Eject,
    /// Powers off the drive that holds the location in the string target.
    SafelyRemove,
    /// Shows the drive mounted at the string target in GNOME Disks.
    OpenInDisks,
    /// Opens GNOME Disks' Format dialog for the drive mounted at the
    /// string target.
    FormatDrive,
    /// Attaches the disk image whose URI is the string target, read-only.
    MountDiskImage,
    /// Opens a disk-usage analyser at the folder whose URI is the string
    /// target.
    AnalyseDiskUsage,
    /// Caches the current folder for search, or stops caching it (a
    /// check item).
    CacheFolder,
    /// Caches the folder whose URI is the string target for search, or
    /// stops caching it (the menus of a pin and of a folder).
    CacheFolderOf,
    /// Opens the folder of the one selected search result, with the
    /// result selected.
    OpenFileLocation,
    /// Opens the folder of the selected symbolic link's target with the
    /// target selected (CMD-030).
    ShowTarget,
    /// Explicit GVfs administrator access; never runs the app as root.
    OpenAsAdministrator,
    /// Enable installed service actions, each initially disabled.
    ManageServiceActions,
    /// Run an enabled service action on the current selection.
    RunServiceAction,
    /// Opens the folder of the one selected search result in a new tab
    /// behind, with the result selected there.
    OpenFileLocationInTab,
    /// Opens the folder of the one selected search result in a new
    /// window, with the result selected there.
    OpenFileLocationInWindow,
    /// Save search: adds the search shown to the sidebar (SRCH-038).
    SaveSearch,
    /// Opens the saved search whose `(folder, text)` is the target.
    OpenSavedSearch,
    /// Removes the saved search whose `(folder, text)` is the target from
    /// the sidebar.
    ForgetSavedSearch,
    /// Opens the Settings page (Ctrl+,), as a tab of its own.
    Settings,
    /// Shows the licence and where the source is.
    License,
    /// Opens Settings at Default apps, where this app becomes the
    /// desktop's default file manager.
    DefaultFileExplorer,
    /// Looks for a newer release.
    CheckUpdates,
    /// Properties of the first selected item, or of the folder
    /// (Alt+Enter).
    Properties,
    /// Properties on its Previous versions tab.
    PreviousVersions,
    /// Properties of the location in the string target, for the menus of
    /// the sidebar, drives and network places, and `ShowItemProperties`.
    PropertiesOf,
    /// Properties of the location in the string target, on its Previous
    /// versions tab (a Quick access pin's menu).
    PreviousVersionsOf,
    /// Measures the selected folders.
    CalculateFolderSize,
    /// Measures every folder shown.
    CalculateFolderSizes,
    /// Measures the folder whose URI is the string target (Properties).
    CalculateFolderSizeOf,
    /// Cancels the running folder-size scan, or hides the finished bar.
    CancelSizeScan,
    /// Opens a snapshot folder in a new tab; the target is `(uri, snapshot
    /// root, snapshot name)`.
    BrowseSnapshot,
    /// Restores a copy of a previous version; the target is `(version
    /// URI, snapshot name, item name)`.
    RestoreVersion,
    /// Extract all…: the selected ZIP into a new folder of the user's
    /// choice.
    ExtractAll,
    /// Extract here: the selected ZIP into a new folder beside it.
    ExtractHere,
    /// Compress to ZIP file: the selection into a new ZIP beside it.
    CompressToZip,
    /// Compress to…: asks for the new archive's name and format first
    /// (ARC-023).
    CompressTo,
    /// Open with…: the Open with dialog for the one selected item, or the
    /// folder.
    OpenWith,
    /// Change app… in Properties: the Open with dialog for the file whose
    /// URI is the string target.
    ChangeApp,
    /// Open folder with…: the Open with dialog for the folder whose URI is
    /// the string target (a Quick access pin's menu).
    OpenWithOf,
    /// Open in Terminal: the terminal in the selected folder, the folder
    /// of the selected file, or the folder shown.
    OpenInTerminal,
    /// Open in Terminal in the folder whose URI is the string target (a
    /// Quick access pin's menu).
    OpenInTerminalOf,
    /// Open Terminal (Shift+F4): the terminal in the folder shown.
    OpenTerminal,
    /// Open Terminal Here (Shift+Alt+F4): a terminal in each folder of the
    /// selection, or in the folder shown.
    OpenTerminalHere,
    /// Compare Files: the two selected files in a comparison tool.
    CompareFiles,
    /// Open Preferred Search Tool (Ctrl+Shift+F) at the folder shown.
    SearchTool,
    /// Opens the selected item in the code editor whose desktop ID is the
    /// string target.
    OpenInEditor,
    /// Opens the selected item in the application whose desktop ID is
    /// the string target (the item menu's "Open with <app>").
    OpenWithApp,
    /// The applications of the file type that is the string target
    /// (Properties' "Apps for this type…").
    TypeApplications,
}

impl WindowAction {
    /// The name the window registers the action under, such as `new-tab`.
    ///
    /// This is the one table of every action's name, so it is longer than
    /// a function should be.
    #[expect(
        clippy::too_many_lines,
        reason = "one line per action: the table of every name"
    )]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            WindowAction::NewTab => "new-tab",
            WindowAction::CloseTab => "close-tab",
            WindowAction::NextTab => "next-tab",
            WindowAction::PreviousTab => "previous-tab",
            WindowAction::CloseWindow => "close-window",
            WindowAction::SelectTab => "select-tab",
            WindowAction::CloseTabById => "close-tab-by-id",
            WindowAction::ShowTabNumber => "show-tab-number",
            WindowAction::CloseOtherTabs => "close-other-tabs",
            WindowAction::ReopenClosedTab => "reopen-closed-tab",
            WindowAction::RestoreClosedTab => "restore-closed-tab",
            WindowAction::OpenSelectionInTabs => "open-selection-in-tabs",
            WindowAction::OpenTab => "open-tab",
            WindowAction::OpenTabBackground => "open-tab-background",
            WindowAction::OpenWindow => "open-window",
            WindowAction::Back => "back",
            WindowAction::Forward => "forward",
            WindowAction::Up => "up",
            WindowAction::Home => "home",
            WindowAction::GoHistory => "go-history",
            WindowAction::Refresh => "refresh",
            WindowAction::Location => "location",
            WindowAction::AddressHistory => "address-history",
            WindowAction::CopyAddress => "copy-address",
            WindowAction::PasteAddress => "paste-address",
            WindowAction::EditableLocation => "editable-location",
            WindowAction::ShowFullPath => "show-full-path",
            WindowAction::CrumbSubfolders => "crumb-subfolders",
            WindowAction::CrumbSibling => "crumb-sibling",
            WindowAction::Search => "search",
            WindowAction::GoTo => "go-to",
            WindowAction::MountVolume => "mount-volume",
            WindowAction::OpenServerAddress => "open-server-address",
            WindowAction::Open => "open",
            WindowAction::SelectAll => "select-all",
            WindowAction::SelectNone => "select-none",
            WindowAction::InvertSelection => "invert-selection",
            WindowAction::SelectMatching => "select-matching",
            WindowAction::PinSelected => "pin-selected",
            WindowAction::PinFolder => "pin-folder",
            WindowAction::CopyPath => "copy-path",
            WindowAction::About => "about",
            WindowAction::Help => "help",
            WindowAction::KeyboardShortcuts => "keyboard-shortcuts",
            WindowAction::ContextMenu => "context-menu",
            WindowAction::View => "view",
            WindowAction::Hidden => "hidden",
            WindowAction::DetailsPane => "details-pane",
            WindowAction::DetailsPaneOption => "details-pane-option",
            WindowAction::DetailsColumn => "details-column",
            WindowAction::Stop => "stop",
            WindowAction::Sidebar => "sidebar",
            WindowAction::SplitView => "split-view",
            WindowAction::FolderTree => "folder-tree",
            WindowAction::FolderTreeOption => "folder-tree-option",
            WindowAction::SidebarIconSize => "sidebar-icon-size",
            WindowAction::SidebarShowAll => "sidebar-show-all",
            WindowAction::HideSection => "hide-section",
            WindowAction::ShowSection => "show-section",
            WindowAction::HidePlace => "hide-place",
            WindowAction::ShowPlace => "show-place",
            WindowAction::Sort => "sort",
            WindowAction::Direction => "direction",
            WindowAction::Groups => "groups",
            WindowAction::FoldersFirst => "folders-first",
            WindowAction::ViewProperties => "view-properties",
            WindowAction::Theme => "theme",
            WindowAction::TextSize(step) => step.action_name(),
            WindowAction::ResetLayout => "reset-layout",
            WindowAction::NewFolder => "new-folder",
            WindowAction::NewTextDocument => "new-text-document",
            WindowAction::NewFile => "new-file",
            WindowAction::NewMarkdownDocument => "new-markdown-document",
            WindowAction::NewCsvFile => "new-csv-file",
            WindowAction::NewJsonFile => "new-json-file",
            WindowAction::NewHtmlDocument => "new-html-document",
            WindowAction::NewFromTemplate => "new-from-template",
            WindowAction::NewFromUserTemplate => "new-from-user-template",
            WindowAction::NewLink => "new-link",
            WindowAction::Cut => "cut",
            WindowAction::Copy => "copy",
            WindowAction::Paste => "paste",
            WindowAction::PasteInto => "paste-into",
            WindowAction::Rename => "rename",
            WindowAction::Trash => "trash",
            WindowAction::DeletePermanently => "delete-permanently",
            WindowAction::Duplicate => "duplicate",
            WindowAction::CutFolder => "cut-folder",
            WindowAction::CopyFolder => "copy-folder",
            WindowAction::PasteIntoFolder => "paste-into-folder",
            WindowAction::RenameFolder => "rename-folder",
            WindowAction::TrashFolder => "trash-folder",
            WindowAction::DeleteFolder => "delete-folder",
            WindowAction::Undo => "undo",
            WindowAction::Redo => "redo",
            WindowAction::CancelOperation => "cancel-operation",
            WindowAction::Restore => "restore",
            WindowAction::EmptyRecycleBin => "empty-recycle-bin",
            WindowAction::EmptyTrash => "empty-trash",
            WindowAction::ClearRecentFiles => "clear-recent-files",
            WindowAction::ClearRecentLocations => "clear-recent-locations",
            WindowAction::ShowNewMenu => "show-new-menu",
            WindowAction::ShowMoreOptions => "show-more-options",
            WindowAction::ShowSortMenu => "show-sort-menu",
            WindowAction::ShowViewMenu => "show-view-menu",
            WindowAction::Unpin => "unpin",
            WindowAction::AddPlace => "add-place",
            WindowAction::EditPin => "edit-pin",
            WindowAction::OpenWindows => "open-windows",
            WindowAction::MoveTabToNewWindow => "move-tab-to-new-window",
            WindowAction::MoveTabToWindow => "move-tab-to-window",
            WindowAction::MoveTabIntoWindow => "move-tab-into-window",
            WindowAction::DropChoice => "drop-choice",
            WindowAction::MapNetworkLocation => "map-network-location",
            WindowAction::DiscoverServers => "discover-servers",
            WindowAction::StopDiscovery => "stop-discovery",
            WindowAction::KeepInNetwork => "keep-in-network",
            WindowAction::RemoveSavedLocation => "remove-saved-location",
            WindowAction::SignOut => "sign-out",
            WindowAction::Disconnect => "disconnect",
            WindowAction::Eject => "eject",
            WindowAction::SafelyRemove => "safely-remove",
            WindowAction::OpenInDisks => "open-in-disks",
            WindowAction::FormatDrive => "format-drive",
            WindowAction::MountDiskImage => "mount-disk-image",
            WindowAction::AnalyseDiskUsage => "analyse-disk-usage",
            WindowAction::CacheFolder => "cache-folder",
            WindowAction::CacheFolderOf => "cache-folder-of",
            WindowAction::OpenFileLocation => "open-file-location",
            WindowAction::ShowTarget => "show-target",
            WindowAction::OpenAsAdministrator => "open-as-administrator",
            WindowAction::ManageServiceActions => "manage-service-actions",
            WindowAction::RunServiceAction => "run-service-action",
            WindowAction::OpenFileLocationInTab => "open-file-location-in-tab",
            WindowAction::OpenFileLocationInWindow => "open-file-location-in-window",
            WindowAction::SaveSearch => "save-search",
            WindowAction::OpenSavedSearch => "open-saved-search",
            WindowAction::ForgetSavedSearch => "forget-saved-search",
            WindowAction::Settings => "settings",
            WindowAction::License => "license",
            WindowAction::DefaultFileExplorer => "default-file-explorer",
            WindowAction::CheckUpdates => "check-updates",
            WindowAction::Properties => "properties",
            WindowAction::PreviousVersions => "previous-versions",
            WindowAction::PropertiesOf => "properties-of",
            WindowAction::PreviousVersionsOf => "previous-versions-of",
            WindowAction::CalculateFolderSize => "calculate-folder-size",
            WindowAction::CalculateFolderSizes => "calculate-folder-sizes",
            WindowAction::CalculateFolderSizeOf => "calculate-folder-size-of",
            WindowAction::CancelSizeScan => "cancel-size-scan",
            WindowAction::BrowseSnapshot => "browse-snapshot",
            WindowAction::RestoreVersion => "restore-version",
            WindowAction::ExtractAll => "extract-all",
            WindowAction::ExtractHere => "extract-here",
            WindowAction::CompressToZip => "compress-to-zip",
            WindowAction::CompressTo => "compress-to",
            WindowAction::OpenWith => "open-with",
            WindowAction::ChangeApp => "change-app",
            WindowAction::OpenWithOf => "open-with-of",
            WindowAction::OpenInTerminal => "open-in-terminal",
            WindowAction::OpenInTerminalOf => "open-in-terminal-of",
            WindowAction::OpenTerminal => "open-terminal",
            WindowAction::OpenTerminalHere => "open-terminal-here",
            WindowAction::CompareFiles => "compare-files",
            WindowAction::SearchTool => "search-tool",
            WindowAction::OpenInEditor => "open-in-editor",
            WindowAction::OpenWithApp => "open-with-app",
            WindowAction::TypeApplications => "type-applications",
        }
    }

    /// The name widgets, menus and accelerators use: `win.` and the name.
    pub(crate) fn detailed_name(self) -> String {
        format!("win.{}", self.name())
    }

    /// Makes `control` run this action when it is clicked or toggled.
    pub(crate) fn assign_to(self, control: &impl IsA<gtk::Actionable>) {
        control.set_action_name(Some(&self.detailed_name()));
    }

    /// Makes `control` run this action with `target` when it is clicked.
    pub(crate) fn assign_with_target_to(self, control: &impl IsA<gtk::Actionable>, target: &glib::Variant) {
        self.assign_to(control);
        control.set_action_target_value(Some(target));
    }

    /// Runs the action with `target` from `widget`, through the browser
    /// window that holds it.
    pub(super) fn activate_from(self, widget: &impl IsA<gtk::Widget>, target: Option<&glib::Variant>) {
        // GTK fails only when no ancestor has the action. Every browser
        // window registers them all, so that is a widget outside one,
        // which has nothing to run.
        let _ = widget.activate_action(&self.detailed_name(), target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_name_an_action_with_the_window_prefix() {
        assert_eq!(WindowAction::NewTab.detailed_name(), "win.new-tab");
        assert_eq!(
            WindowAction::TextSize(Step::Increase).detailed_name(),
            "win.text-larger"
        );
    }
}
