// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' context menu: right-click, the Menu key and
//! Shift+F10.
//!
//! Ports the rows' `contextmenu` handlers and `entryMenu` in
//! `desktop/ui/app.js`, and the `ContextMenu`/Shift+F10 keys of `onKey`.
//! A right-click on an unselected item selects only it first; on blank
//! space it clears the selection. From the keyboard the menu points at the
//! first selected item, as in Windows 11 and Dolphin.

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene};

use super::widget_tree::children;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// How far into a row, and from the view's corner without one, a menu
/// opened from the keyboard points.
const KEYBOARD_MENU_INSET: i32 = 40;

/// Adds an item to `menu` that runs `action`.
fn append_item(menu: &gio::Menu, label: &str, action: WindowAction) {
    menu.append(Some(label), Some(&action.detailed_name()));
}

/// The right-click and keyboard context menu of one view.
fn context_menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    append_item(&menu, "Open", WindowAction::Open);
    append_item(&menu, "Refresh", WindowAction::Refresh);
    let selection = gio::Menu::new();
    append_item(&selection, "Select all", WindowAction::SelectAll);
    append_item(&selection, "Select none", WindowAction::SelectNone);
    append_item(&selection, "Invert selection", WindowAction::InvertSelection);
    menu.append_section(None, &selection);
    menu
}

impl BrowserWindow {
    /// Gives `view` its context menu and the gestures and keys that open it.
    pub(super) fn attach_context_menu(&self, view: &gtk::Widget) {
        let popover = gtk::PopoverMenu::from_model(Some(&context_menu_model()));
        // Windows and app.js draw context menus without an arrow.
        popover.set_has_arrow(false);
        popover.add_css_class("ox-menu");
        popover.set_parent(view);
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
            popover,
            #[weak]
            view,
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                window.select_for_context_menu(&view, x, y);
                #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
                let point = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                popover.set_pointing_to(Some(&point));
                popover.popup();
            }
        ));
        view.add_controller(right_click);
    }

    /// A right-click on an unselected item selects only it; on blank space
    /// it clears the selection.
    fn select_for_context_menu(&self, view: &gtk::Widget, x: f64, y: f64) {
        let model = &self.content().model;
        match self.content().owners.position_at(view, x, y) {
            Some(position) if !model.selection().is_selected(position) => model.select_only(position),
            Some(_) => {}
            None => model.select_none(),
        }
    }

    /// Opens the context menu from the Menu key or Shift+F10, pointing at
    /// the first selected item, or near the top of the view without one.
    pub(super) fn open_context_menu_from_keyboard(&self) {
        let view = self.content().view_widget();
        let Some(popover) = context_menu_of(&view) else {
            return;
        };
        let selected = self.content().model.first_selected();
        if let Some(position) = selected {
            self.content().reveal(position);
        }
        let row = selected.and_then(|position| self.content().owners.widget_at(position));
        let bounds = row.and_then(|row| row.compute_bounds(&view));
        popover.set_pointing_to(Some(&keyboard_menu_anchor(bounds)));
        popover.popup();
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
fn context_menu_of(view: &gtk::Widget) -> Option<gtk::PopoverMenu> {
    children(view).find_map(|child| child.downcast::<gtk::PopoverMenu>().ok())
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
