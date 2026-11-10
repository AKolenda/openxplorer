// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar's menus as one group, worked as in Windows 11 File
//! Explorer: while one is open, one click on another menu button closes
//! it and opens that one, a click on its own button closes it, and a
//! click anywhere else in the window closes it and still does what it
//! does there, such as selecting the file clicked. Resting the pointer on
//! another menu button opens nothing, as in Windows 11.
//!
//! A GTK menu normally holds the pointer while it is open and spends the
//! next click outside it on closing itself, so that click never reaches
//! the button or the file under it. The bar's menus hold no pointer grab
//! (`autohide` off), so every click goes where it is, and they are closed
//! instead by the window's own press handler
//! ([`super::super::menu_popover::close_menus_on_press`]), which leaves a
//! press on a menu's own button to that button, by the window going
//! inactive (a click in another app), and by this group when another of
//! its menus opens. [`CommandBar::open_menu`] is the one menu of the bar
//! that is open.
//!
//! Without a grab, one thing needs mending, on X11 only. GTK keeps one
//! pointer target per window, shared with its menus, and after the
//! window lays itself out again it sends the window a made-up motion
//! event at the pointer's position (`gdk_surface_ensure_motion`), even
//! while the pointer is over a menu; a grab is what normally stops that.
//! The next press on the menu, before the pointer moves, would then go to
//! the file or control under the menu. The group follows which surface
//! last had the pointer's motion, as GTK does, and takes such a press
//! from the window and gives it to the menu's row or button under it.
//!
//! The keyboard is unchanged: a menu takes it when it opens, Escape
//! closes it and gives it back to the button.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::super::menu_popover::MenuPopover;
use super::super::widget_tree::children;
use super::CommandBar;

impl CommandBar {
    /// Makes the bar's menu buttons one group: their menus let clicks
    /// through and at most one of them is open.
    pub(super) fn group_menus(&self) {
        for button in self.menu_buttons() {
            if let Some(menu) = button.popover() {
                menu.set_autohide(false);
            }
            button.connect_active_notify(glib::clone!(
                #[weak(rename_to = bar)]
                self,
                move |button| bar.follow_menu(button)
            ));
        }
        self.connect_realize(CommandBar::watch_window);
    }

    /// The menu button whose menu is open, if any.
    pub(in crate::window) fn open_menu(&self) -> Option<gtk::MenuButton> {
        self.imp().open_menu.borrow().clone()
    }

    /// Notes that `button`'s menu opened or closed; one that opens closes
    /// the one that was open.
    fn follow_menu(&self, button: &gtk::MenuButton) {
        let open = &self.imp().open_menu;
        if button.is_active() {
            let before = open.replace(Some(button.clone()));
            if let Some(before) = before.filter(|before| before != button) {
                before.popdown();
            }
        } else if open.borrow().as_ref() == Some(button) {
            open.replace(None);
            self.imp().held_press.replace(None);
        }
    }

    /// Closes the open menu when the bar's window stops being the active
    /// one, as a menu that held the pointer did on a click in another app,
    /// and mends the presses GTK would send past the open menu.
    fn watch_window(&self) {
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let watched = &self.imp().watched_window;
        if watched.upgrade().as_ref() == Some(&window) {
            return;
        }
        watched.set(Some(&window));
        window.connect_is_active_notify(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |window| {
                if !window.is_active() {
                    if let Some(button) = bar.open_menu() {
                        button.popdown();
                    }
                }
            }
        ));
        let pointer_events = gtk::EventControllerLegacy::new();
        pointer_events.set_propagation_phase(gtk::PropagationPhase::Capture);
        pointer_events.connect_event(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, event| bar.follow_pointer_event(event)
        ));
        window.add_controller(pointer_events);
    }

    /// Follows which surface has the pointer, and takes a press on the
    /// open menu that GTK sends to the window under it, and its release.
    fn follow_pointer_event(&self, event: &gdk::Event) -> glib::Propagation {
        let imp = self.imp();
        match event.event_type() {
            gdk::EventType::MotionNotify | gdk::EventType::EnterNotify => {
                imp.pointer_surface.set(event.surface().as_ref());
                glib::Propagation::Proceed
            }
            gdk::EventType::ButtonPress => match self.press_sent_past_menu(event) {
                Some(target) => {
                    imp.held_press.replace(Some(target));
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
            gdk::EventType::ButtonRelease => match imp.held_press.take() {
                Some(target) => {
                    click(&target);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
            _ => glib::Propagation::Proceed,
        }
    }

    /// For a press on the open menu, or a submenu of it, while GTK's
    /// pointer target is in the window, not in that menu: the menu's row
    /// or button under the press.
    fn press_sent_past_menu(&self, event: &gdk::Event) -> Option<gtk::Widget> {
        let button = self.open_menu()?;
        let surface = event.surface()?;
        if self.imp().pointer_surface.upgrade().as_ref() == Some(&surface) {
            return None;
        }
        let menu = gtk::Native::for_surface(&surface).and_downcast::<MenuPopover>()?;
        if !menu.is_ancestor(&button) {
            return None;
        }
        let (x, y) = event.position()?;
        let (shadow_x, shadow_y) = menu.surface_transform();
        let picked = menu.pick(x - shadow_x, y - shadow_y, gtk::PickFlags::DEFAULT)?;
        clickable(&picked)
    }

    /// The bar's menu buttons: New, Sort, View, Search options, See more
    /// and the appearance toggle.
    fn menu_buttons(&self) -> Vec<gtk::MenuButton> {
        let imp = self.imp();
        let group = imp.file_commands.upcast_ref::<gtk::Widget>();
        let mut buttons: Vec<gtk::MenuButton> = children(group)
            .filter_map(|child| child.downcast::<gtk::MenuButton>().ok())
            .collect();
        buttons.push(imp.appearance_button.get());
        buttons
    }
}

/// The row or button of a menu that holds `widget`.
fn clickable(widget: &gtk::Widget) -> Option<gtk::Widget> {
    widget
        .ancestor(gtk::ListBoxRow::static_type())
        .or_else(|| widget.ancestor(gtk::Button::static_type()))
}

/// Does what a click on a menu's row or button does.
fn click(target: &gtk::Widget) {
    if !target.is_sensitive() || !target.is_mapped() {
        return;
    }
    if let Some(row) = target.downcast_ref::<gtk::ListBoxRow>() {
        row.activate();
    } else if let Some(button) = target.downcast_ref::<gtk::Button>() {
        button.emit_clicked();
    }
}
