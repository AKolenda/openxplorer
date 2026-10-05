// SPDX-License-Identifier: AGPL-3.0-only
//! Moving from one command bar menu to another with one click, as in
//! Windows Explorer: while Sort is open, clicking View closes Sort and
//! opens View, instead of only closing Sort.
//!
//! An open menu holds the pointer, as every GTK popover does, so the click
//! that closes it never reaches the button under the pointer. The bar
//! therefore remembers which of its menu buttons the pointer is over (its
//! motion still arrives while a menu is open) and, when a menu closes
//! while the primary button is held over another menu button, opens that
//! one. A menu closed by Escape, by choosing an item or by a click
//! anywhere else stays closed.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::CommandBar;

/// The menu button under the pointer, if any.
type Hovered = Rc<RefCell<glib::WeakRef<gtk::MenuButton>>>;

impl CommandBar {
    /// Lets a click on another menu button of the bar move to its menu
    /// while one is open.
    pub(super) fn switch_menus_on_click(&self) {
        let hovered: Hovered = Rc::default();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[strong]
            hovered,
            move |motion, x, y| {
                let under = motion.widget().and_then(|bar| menu_button_at(&bar, x, y));
                hovered.borrow().set(under.as_ref());
            }
        ));
        motion.connect_leave(glib::clone!(
            #[strong]
            hovered,
            move |_| hovered.borrow().set(None)
        ));
        self.add_controller(motion);
        for button in self.menu_buttons() {
            let hovered = Rc::clone(&hovered);
            button.connect_active_notify(move |closed| {
                if closed.is_active() {
                    return;
                }
                let Some(next) = hovered.borrow().upgrade() else {
                    return;
                };
                if next == *closed
                    || !next.is_sensitive()
                    || !next.is_visible()
                    || !primary_button_held(closed)
                {
                    return;
                }
                // Once the closed menu has let go of the pointer and its
                // button has taken the keyboard back.
                glib::idle_add_local_once(move || {
                    if !next.is_active() {
                        next.popup();
                    }
                });
            });
        }
    }

    /// The bar's menu buttons, left to right: New, Sort, View, More
    /// options and the appearance.
    pub(super) fn menu_buttons(&self) -> Vec<gtk::MenuButton> {
        let mut buttons: Vec<gtk::MenuButton> = children(&self.imp_file_commands())
            .filter_map(|child| child.downcast::<gtk::MenuButton>().ok())
            .collect();
        buttons.push(self.imp_appearance_button());
        buttons
    }
}

/// The children of `widget`, first to last.
fn children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}

/// The menu button at `x`, `y` in `bar`, if any.
fn menu_button_at(bar: &gtk::Widget, x: f64, y: f64) -> Option<gtk::MenuButton> {
    bar.pick(x, y, gtk::PickFlags::DEFAULT)?
        .ancestor(gtk::MenuButton::static_type())
        .and_downcast()
}

/// Whether the primary pointer button is down: the menu closed because of
/// a click, not Escape or an item chosen.
fn primary_button_held(widget: &impl IsA<gtk::Widget>) -> bool {
    #[cfg(test)]
    if let Some(held) = tests::HELD.get() {
        return held;
    }
    widget
        .display()
        .default_seat()
        .and_then(|seat| seat.pointer())
        .is_some_and(|pointer| pointer.modifier_state().contains(gdk::ModifierType::BUTTON1_MASK))
}

#[cfg(test)]
pub(in crate::window) mod tests {
    use std::cell::Cell;

    thread_local! {
        /// Whether tests say the primary button is down: GTK cannot press
        /// a real one under Xvfb.
        pub(in crate::window) static HELD: Cell<Option<bool>> = const { Cell::new(None) };
    }
}
