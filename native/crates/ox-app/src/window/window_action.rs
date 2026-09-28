// SPDX-License-Identifier: AGPL-3.0-only
//! The names of the window's actions (`win.*`).
//!
//! Buttons, menus, sidebar rows and keyboard shortcuts run the command
//! handlers of `desktop/ui/app.js` as window actions, by name. GTK ignores
//! a name it does not know, so a misspelt name would leave a control that
//! silently does nothing. [`WindowAction`] keeps every name in one table,
//! which turns such a typo into a compile error. The templates in
//! `resources/ui/` therefore name no action: their buttons get one through
//! [`WindowAction::assign_to`]. [`super::actions`] registers the working
//! actions and [`super::unported`] the disabled ones.

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
    /// Shows the tab whose id is the `u64` target.
    SelectTab,
    /// Closes the tab whose id is the `u64` target (its close button).
    CloseTabById,
    /// Opens the location in the string target in a new tab in front.
    OpenTab,
    /// Opens the location in the string target in a new tab behind the
    /// active one (a middle-click).
    OpenTabBackground,
    /// Back in the active tab's history (Alt+Left).
    Back,
    /// Forward in the active tab's history (Alt+Right).
    Forward,
    /// Opens the folder that contains the current one (Alt+Up).
    Up,
    /// Lists the current folder again (F5, Ctrl+R).
    Refresh,
    /// Makes the address editable (Ctrl+L, Alt+D).
    Location,
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
    /// Pins the one selected folder to Quick access.
    PinSelected,
    /// Pins the current folder to Quick access.
    PinFolder,
    /// Copies the path of the selection, or of the folder.
    CopyPath,
    /// Shows what this build is.
    About,
    /// Opens the folder view's context menu from the keyboard.
    ContextMenu,
    /// The folder view: `details`, or an icon size (Ctrl+Shift+1 to 4).
    View,
    /// Shows or hides hidden files (Ctrl+H).
    Hidden,
    /// Shows or hides the details pane.
    DetailsPane,
    /// The column the details view sorts by.
    Sort,
    /// Whether the details view sorts ascending or descending.
    Direction,
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
    /// Cut (Ctrl+X).
    Cut,
    /// Copy (Ctrl+C).
    Copy,
    /// Paste (Ctrl+V).
    Paste,
    /// Rename (F2).
    Rename,
    /// Move to Trash (Delete).
    Trash,
    /// Opens the connect dialog for a network share.
    MapNetworkLocation,
    /// Looks for SMB servers that advertise themselves.
    DiscoverServers,
    /// Adds the current folder to the search cache.
    CacheFolder,
    /// Opens the Settings page (Ctrl+,), as a tab of its own.
    Settings,
    /// Shows the licence and where the source is.
    License,
    /// Opens Settings at Default apps, where this app becomes the
    /// desktop's default file manager.
    DefaultFileExplorer,
    /// Looks for a newer release.
    CheckUpdates,
}

impl WindowAction {
    /// The name the window registers the action under, such as `new-tab`.
    ///
    /// This is the one table of every action's name, so it is longer than
    /// a function should be.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            WindowAction::NewTab => "new-tab",
            WindowAction::CloseTab => "close-tab",
            WindowAction::NextTab => "next-tab",
            WindowAction::PreviousTab => "previous-tab",
            WindowAction::SelectTab => "select-tab",
            WindowAction::CloseTabById => "close-tab-by-id",
            WindowAction::OpenTab => "open-tab",
            WindowAction::OpenTabBackground => "open-tab-background",
            WindowAction::Back => "back",
            WindowAction::Forward => "forward",
            WindowAction::Up => "up",
            WindowAction::Refresh => "refresh",
            WindowAction::Location => "location",
            WindowAction::Search => "search",
            WindowAction::GoTo => "go-to",
            WindowAction::MountVolume => "mount-volume",
            WindowAction::OpenServerAddress => "open-server-address",
            WindowAction::Open => "open",
            WindowAction::SelectAll => "select-all",
            WindowAction::SelectNone => "select-none",
            WindowAction::InvertSelection => "invert-selection",
            WindowAction::PinSelected => "pin-selected",
            WindowAction::PinFolder => "pin-folder",
            WindowAction::CopyPath => "copy-path",
            WindowAction::About => "about",
            WindowAction::ContextMenu => "context-menu",
            WindowAction::View => "view",
            WindowAction::Hidden => "hidden",
            WindowAction::DetailsPane => "details-pane",
            WindowAction::Sort => "sort",
            WindowAction::Direction => "direction",
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
            WindowAction::Cut => "cut",
            WindowAction::Copy => "copy",
            WindowAction::Paste => "paste",
            WindowAction::Rename => "rename",
            WindowAction::Trash => "trash",
            WindowAction::MapNetworkLocation => "map-network-location",
            WindowAction::DiscoverServers => "discover-servers",
            WindowAction::CacheFolder => "cache-folder",
            WindowAction::Settings => "settings",
            WindowAction::License => "license",
            WindowAction::DefaultFileExplorer => "default-file-explorer",
            WindowAction::CheckUpdates => "check-updates",
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
