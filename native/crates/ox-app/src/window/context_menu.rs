// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' context menus: right-click, the Menu key and
//! Shift+F10 (CMD-008 to CMD-015).
//!
//! Ports the rows' `contextmenu` handlers, `entryMenu`, `backgroundMenu`
//! and the `ContextMenu`/Shift+F10 keys of `onKey` in `desktop/ui/app.js`.
//! A right-click on an unselected item selects only it first and opens
//! that item's menu; on blank space it clears the selection and opens the
//! folder's menu. The Settings choice "Right-click menu" picks the
//! classic or the compact style for a right-click; the keyboard always
//! opens the classic menu (CMD-013), pointing at the first selected item
//! as in Windows 11 and Dolphin (CMD-014). The compact menu's "Show more
//! options" and the folder menu's "New…" open their menus at the same
//! point. What the menus list is in [`entries`].

mod entries;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};
use ox_core::entry::Entry;
use ox_core::location::{is_smb_location, is_smb_server};
use ox_core::ops::JournalDirection;
use ox_core::settings::ContextMenu as MenuStyleChoice;

use super::actions::{plain_action, text_action};
use super::command_bar::new_menu;
use super::menu_popover::{MenuPopover, MenuStyle};
use super::widget_tree::children;
use super::window_action::WindowAction;
use super::BrowserWindow;

use entries::{
    background_menu, item_menu, recycle_bin_background_menu, recycle_bin_item_menu, ContextMenu, ItemFacts,
    ItemLocation, ItemShape,
};

/// How far into a row, and from the view's corner without one, a menu
/// opened from the keyboard points.
const KEYBOARD_MENU_INSET: i32 = 40;

/// True for a ZIP archive, by its name or its type (`isZipEntry`).
fn is_zip(name: &str, content_type: Option<&str>) -> bool {
    let named_zip = name.to_lowercase().ends_with(".zip");
    let zip_types = [
        "application/zip",
        "application/x-zip",
        "application/x-zip-compressed",
    ];
    named_zip || content_type.is_some_and(|content_type| zip_types.contains(&content_type))
}

/// What `entry` is, as its menu cares.
fn item_shape(entry: &Entry) -> ItemShape {
    if entry.is_dir {
        ItemShape::Folder
    } else if is_zip(&entry.name, entry.content_type.as_deref()) {
        ItemShape::ZipArchive
    } else {
        ItemShape::File
    }
}

/// Where the item at `uri` is, as its menu cares.
fn item_location(uri: &str) -> ItemLocation {
    if is_smb_server(uri) {
        ItemLocation::SmbServer
    } else if is_smb_location(uri) {
        ItemLocation::SmbShare
    } else {
        ItemLocation::Local
    }
}

impl BrowserWindow {
    /// Gives `view` its context menu and the gestures and keys that open it.
    pub(super) fn attach_context_menu(&self, view: &gtk::Widget) {
        let popover = MenuPopover::new(Vec::new());
        // Context menus open at the pointer, without the offset of a
        // button's drop-down.
        popover.set_offset(0, 0);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_parent(view);
        // Escape, a click outside and a choice all give the keyboard back
        // to the file pane; a chosen command runs after this, so a
        // rename field or a dialog still takes it.
        popover.connect_closed(glib::clone!(
            #[weak]
            view,
            move |_| {
                view.grab_focus();
            }
        ));
        view.connect_destroy(glib::clone!(
            #[weak]
            popover,
            move |_| popover.unparent()
        ));
        view.add_controller(context_menu_shortcut());
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                window.select_for_context_menu(&view, x, y);
                #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
                let point = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                window.open_context_menu(&view, &point, window.preferred_menu_style());
            }
        ));
        view.add_controller(right_click);
    }

    /// The style the Settings choice "Right-click menu" asks for.
    fn preferred_menu_style(&self) -> MenuStyle {
        match self.context().settings_data().preferences.context_menu {
            MenuStyleChoice::Win10 => MenuStyle::Classic,
            MenuStyleChoice::Win11 => MenuStyle::Compact,
        }
    }

    /// A right-click on an unselected item selects only it; on blank space
    /// it clears the selection.
    fn select_for_context_menu(&self, view: &gtk::Widget, x: f64, y: f64) {
        let model = self.folder_pane().model();
        match self.folder_pane().owners().position_at(view, x, y) {
            Some(position) if !model.selection().is_selected(position) => model.select_only(position),
            Some(_) => {}
            None => model.select_none(),
        }
    }

    /// Opens the menu of the selection, or of the folder without one, in
    /// `style`, pointing at `point` in `view`.
    fn open_context_menu(&self, view: &gtk::Widget, point: &gdk::Rectangle, style: MenuStyle) {
        let Some(popover) = context_menu_of(view) else {
            return;
        };
        let menu = self.menu_for_selection(style);
        popover.set_entries(menu.entries);
        popover.set_style_and_strip(style, menu.strip);
        popover.set_pointing_to(Some(point));
        popover.popup();
    }

    /// What the menu lists for the selection now.
    fn menu_for_selection(&self, style: MenuStyle) -> ContextMenu {
        let items = self.folder_pane().model().selected_items();
        let in_recycle_bin = self.command_facts().folder.is_recycle_bin;
        let entries = match (items.first(), in_recycle_bin) {
            (None, true) => recycle_bin_background_menu(),
            (None, false) => background_menu(
                &self.journal_label(JournalDirection::Undo),
                &self.journal_label(JournalDirection::Redo),
            ),
            (Some(_), true) => recycle_bin_item_menu(items.len() == 1),
            (Some(first), false) => {
                let facts = self.item_facts(first.entry(), items.len() == 1);
                return item_menu(&facts, style);
            }
        };
        ContextMenu {
            entries,
            strip: Vec::new(),
        }
    }

    /// The facts the menu of `entry` reads; `is_single` when it is the
    /// only item selected.
    fn item_facts(&self, entry: &Entry, is_single: bool) -> ItemFacts {
        let caching = if entry.is_dir {
            self.caching_of(entry.navigation_uri())
        } else {
            None
        };
        ItemFacts {
            navigation_uri: entry.navigation_uri().to_owned(),
            shape: item_shape(entry),
            location: item_location(&entry.uri),
            is_read_only: self.imp().locations.borrow().is_snapshot_location(&entry.uri),
            is_single,
            is_search_result: self.is_searching(),
            editors: self.context().desktop_integration().known_editor_shortcuts(),
            caching,
            delete_label: self.delete_label(),
        }
    }

    /// Opens the classic menu from the Menu key or Shift+F10, pointing at
    /// the first selected item, or near the top of the view without one.
    pub(super) fn open_context_menu_from_keyboard(&self) {
        let view = self.folder_pane().view_widget();
        let selected = self.folder_pane().model().first_selected();
        if let Some(position) = selected {
            self.folder_pane().reveal(position);
        }
        let row = selected.and_then(|position| self.folder_pane().owners().widget_at(position));
        let bounds = row.and_then(|row| row.compute_bounds(&view));
        self.open_context_menu(&view, &keyboard_menu_anchor(bounds), MenuStyle::Classic);
    }

    /// "Show more options": the classic menu where the compact one was.
    pub(super) fn show_more_options(&self) {
        let view = self.folder_pane().view_widget();
        let Some(point) = context_menu_of(&view).and_then(|popover| popover.pointing_to().1.into()) else {
            return;
        };
        self.open_context_menu(&view, &point, MenuStyle::Classic);
    }

    /// The folder menu's "New…": the New menu where the folder menu was.
    pub(super) fn show_new_menu_in_place(&self) {
        let view = self.folder_pane().view_widget();
        let Some(popover) = context_menu_of(&view) else {
            return;
        };
        popover.set_entries(new_menu());
        popover.set_style_and_strip(MenuStyle::Classic, Vec::new());
        popover.popup();
    }

    /// Adds the actions the context menus run themselves: "Show more
    /// options", "New…", "Unpin from Quick access" and "Open windows…".
    pub(super) fn install_context_menu_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::ShowMoreOptions, BrowserWindow::show_more_options),
            plain_action(WindowAction::ShowNewMenu, BrowserWindow::show_new_menu_in_place),
            text_action(WindowAction::Unpin, BrowserWindow::unpin),
            plain_action(WindowAction::OpenWindows, BrowserWindow::show_open_windows),
        ]);
    }

    /// The context menu of the view shown, for tests.
    #[cfg(test)]
    pub(super) fn context_menu(&self) -> MenuPopover {
        context_menu_of(&self.folder_pane().view_widget()).expect("every view has a context menu")
    }

    /// Opens the context menu as a right-click at the item at `position`,
    /// or on blank space below the items without one, for tests: the
    /// selection changes as [`Self::select_for_context_menu`] decides.
    #[cfg(test)]
    pub(super) fn right_click(&self, position: Option<u32>) {
        let view = self.folder_pane().view_widget();
        let (x, y) = match position {
            Some(position) => {
                let point_on_item = || self.point_on_item(&view, position);
                crate::test_support::harness::wait_until("the right-clicked item to be laid out", || {
                    point_on_item().is_some()
                });
                point_on_item().expect("the right-clicked item is laid out")
            }
            None => (4.0, f64::from(view.height()) - 4.0),
        };
        self.select_for_context_menu(&view, x, y);
        let point = gdk::Rectangle::new(KEYBOARD_MENU_INSET, KEYBOARD_MENU_INSET, 1, 1);
        self.open_context_menu(&view, &point, self.preferred_menu_style());
    }

    /// A point in `view` on the item at `position`, once it is laid out
    /// there, for tests.
    #[cfg(test)]
    fn point_on_item(&self, view: &gtk::Widget, position: u32) -> Option<(f64, f64)> {
        let owners = self.folder_pane().owners();
        let bounds = owners.file_cell_at(position, view)?.compute_bounds(view)?;
        let (x, y) = (
            f64::from(bounds.x()) + 4.0,
            f64::from(bounds.y() + bounds.height() / 2.0),
        );
        (owners.position_at(view, x, y) == Some(position)).then_some((x, y))
    }
}

/// Where a menu opened from the keyboard points: into the selected row,
/// [`KEYBOARD_MENU_INSET`] from its start, or near the top of the view
/// without one.
fn keyboard_menu_anchor(row_bounds: Option<graphene::Rect>) -> gdk::Rectangle {
    let Some(bounds) = row_bounds else {
        return gdk::Rectangle::new(KEYBOARD_MENU_INSET, KEYBOARD_MENU_INSET, 1, 1);
    };
    let x = whole_pixels(bounds.x()) + KEYBOARD_MENU_INSET;
    let y = whole_pixels(bounds.y());
    let height = whole_pixels(bounds.height());
    gdk::Rectangle::new(x, y, 1, height)
}

/// A widget coordinate cut to whole pixels.
#[expect(clippy::cast_possible_truncation, reason = "widget bounds are small")]
fn whole_pixels(coordinate: f32) -> i32 {
    coordinate as i32
}

/// The context menu popover attached to `view`.
fn context_menu_of(view: &gtk::Widget) -> Option<MenuPopover> {
    children(view).find_map(|child| child.downcast::<MenuPopover>().ok())
}

/// The Menu key and Shift+F10 open the context menu. They are view
/// shortcuts, not application accelerators, so the address and search
/// entries keep their own text menus on those keys.
fn context_menu_shortcut() -> gtk::ShortcutController {
    let trigger = gtk::ShortcutTrigger::parse_string("Menu|<Shift>F10");
    let action = gtk::NamedAction::new(&WindowAction::ContextMenu.detailed_name());
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(action)));
    shortcuts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_archives_are_known_by_name_or_type() {
        assert!(is_zip("Photos.ZIP", None));
        assert!(is_zip("download", Some("application/x-zip-compressed")));
        assert!(!is_zip("notes.txt", Some("text/plain")));
    }
}
