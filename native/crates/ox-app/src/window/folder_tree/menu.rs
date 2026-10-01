// SPDX-License-Identifier: AGPL-3.0-only
//! The folder tree's context menu (SIDE-028), as Dolphin's
//! `TreeViewContextMenu` offers it: open the folder in a tab or a window,
//! Cut, Copy, Paste into it, Rename…, Move to Trash, Delete permanently,
//! Open in Terminal and Properties, each acting on the folder clicked;
//! then the tree's own options with check marks: Show hidden folders,
//! Limit to home folder and Scroll to the folder shown.

use gtk::prelude::*;
use ox_core::i18n::gettext;
use ox_core::location::{is_smb_share_root, is_virtual_location};
use ox_core::settings::FolderTreeOptions;

use super::FolderTree;
use crate::icons::Icon;
use crate::window::menu_popover::{ItemCheck, MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// The option names of `win.folder-tree-option`.
const SHOW_HIDDEN: &str = "show-hidden";
const LIMIT_TO_HOME: &str = "limit-to-home";
const AUTO_SCROLL: &str = "auto-scroll";

/// What the menu of a folder may offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FolderFacts {
    /// The folder can be cut, renamed and deleted: not the tree's top, a
    /// share or a virtual place.
    changeable: bool,
    /// The folder can be copied.
    copyable: bool,
    /// Files on the clipboard can be pasted into it.
    can_paste: bool,
}

/// `options` with the option `name` switched; `None` for a name it does
/// not have.
pub(super) fn toggled(mut options: FolderTreeOptions, name: &str) -> Option<FolderTreeOptions> {
    match name {
        SHOW_HIDDEN => options.show_hidden = !options.show_hidden,
        LIMIT_TO_HOME => options.limit_to_home = !options.limit_to_home,
        AUTO_SCROLL => options.auto_scroll = !options.auto_scroll,
        _ => return None,
    }
    Some(options)
}

/// An item that runs `action` on the folder `uri`, disabled unless
/// `allowed`.
fn folder_item(label: &str, glyph: Icon, action: WindowAction, uri: &str, allowed: bool) -> MenuEntry {
    MenuItem::with_text_target(label, glyph, action, uri)
        .disabled_when(!allowed)
        .into()
}

/// A checkable item for the option `name`.
fn option_item(label: &str, glyph: Icon, name: &str, is_on: bool) -> MenuEntry {
    let item = MenuItem::with_text_target(label, glyph, WindowAction::FolderTreeOption, name);
    MenuEntry::Item(MenuItem {
        check: ItemCheck::Fixed(is_on),
        ..item
    })
}

/// The menu of the folder `uri` with `facts`, then the tree's options.
fn entries(uri: &str, facts: FolderFacts, options: FolderTreeOptions) -> Vec<MenuEntry> {
    vec![
        MenuItem::with_text_target(&gettext("Open in new tab"), Icon::Add, WindowAction::OpenTab, uri).into(),
        MenuItem::with_text_target(
            &gettext("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            uri,
        )
        .into(),
        MenuEntry::Divider,
        folder_item(
            &gettext("Cut"),
            Icon::Cut,
            WindowAction::CutFolder,
            uri,
            facts.changeable,
        ),
        folder_item(
            &gettext("Copy"),
            Icon::Copy,
            WindowAction::CopyFolder,
            uri,
            facts.copyable,
        ),
        folder_item(
            &gettext("Paste"),
            Icon::ClipboardPaste,
            WindowAction::PasteInto,
            uri,
            facts.can_paste,
        ),
        folder_item(
            &gettext("Rename…"),
            Icon::Rename,
            WindowAction::RenameFolder,
            uri,
            facts.changeable,
        ),
        folder_item(
            &gettext("Move to Trash"),
            Icon::Delete,
            WindowAction::TrashFolder,
            uri,
            facts.changeable,
        ),
        folder_item(
            &gettext("Delete permanently"),
            Icon::DeleteDismiss,
            WindowAction::DeleteFolder,
            uri,
            facts.changeable,
        ),
        MenuEntry::Divider,
        MenuItem::with_text_target(
            &gettext("Open in Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenInTerminalOf,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &gettext("Properties"),
            Icon::Info,
            WindowAction::PropertiesOf,
            uri,
        )
        .into(),
        MenuEntry::Divider,
        option_item(
            &gettext("Show hidden folders"),
            Icon::Eye,
            SHOW_HIDDEN,
            options.show_hidden,
        ),
        option_item(
            &gettext("Limit to home folder"),
            Icon::Home,
            LIMIT_TO_HOME,
            options.limit_to_home,
        ),
        option_item(
            &gettext("Scroll to the folder shown"),
            Icon::ArrowDown,
            AUTO_SCROLL,
            options.auto_scroll,
        ),
    ]
}

impl FolderTree {
    /// The menu of the folder at `position`.
    pub(super) fn menu_entries(&self, position: u32) -> Option<Vec<MenuEntry>> {
        let uri = self.uri_at(position)?;
        let window = self.root().and_downcast::<BrowserWindow>()?;
        let is_real = !is_virtual_location(&uri);
        let facts = FolderFacts {
            changeable: position > 0 && is_real && !is_smb_share_root(&uri),
            copyable: is_real && !is_smb_share_root(&uri),
            can_paste: window.can_paste_into(&uri),
        };
        Some(entries(&uri, facts, self.options()))
    }
}
