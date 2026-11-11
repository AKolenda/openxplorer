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
//! last had the pointer's motion, as GTK does, and takes such a press,
//! and its release, from the window, wherever on the menu it is, so it
//! never reaches what is under the menu. It then does what the click
//! would have done in the menu: a primary press and release on the same
//! row or button runs it; a press between the items, one dragged off its
//! item before the release, or a right or middle click runs nothing.
//!
//! The keyboard is unchanged: a menu takes it when it opens, Escape
//! closes it and gives it back to the button.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::super::menu_popover::MenuPopover;
use super::super::widget_tree::{children, descendants};
use super::super::BrowserWindow;
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
            gdk::EventType::ButtonPress => {
                if imp.held_press.borrow().is_some() {
                    // Another button pressed while one is held: part of
                    // the same click on the menu.
                    return glib::Propagation::Stop;
                }
                match self.press_sent_past_menu(event) {
                    Some(held) => {
                        imp.held_press.replace(Some(held));
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            }
            gdk::EventType::ButtonRelease => {
                let Some(pointer_button) = pointer_button(event) else {
                    return glib::Propagation::Proceed;
                };
                let held = imp.held_press.borrow().as_ref().map(|held| held.button);
                match held {
                    None => glib::Propagation::Proceed,
                    Some(button) if button != pointer_button => glib::Propagation::Stop,
                    Some(_) => {
                        if let Some(held) = imp.held_press.take() {
                            held.release(event);
                        }
                        glib::Propagation::Stop
                    }
                }
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// For a press on the open menu, or a submenu of it, while GTK's
    /// pointer target is in the window, not in that menu: the press to
    /// take from the window, with the menu's row or button under it, if
    /// any.
    fn press_sent_past_menu(&self, event: &gdk::Event) -> Option<HeldPress> {
        let button = self.open_menu()?;
        let surface = event.surface()?;
        if self.imp().pointer_surface.upgrade().as_ref() == Some(&surface) {
            return None;
        }
        let menu = gtk::Native::for_surface(&surface).and_downcast::<MenuPopover>()?;
        if !menu.is_ancestor(&button) {
            return None;
        }
        Some(HeldPress {
            button: pointer_button(event)?,
            target: item_at(&menu, event),
            menu,
        })
    }

    /// Opens the menu of the shown menu button named `name` ("View",
    /// "See more"), as a click on it does; false when the bar shows none.
    fn open_menu_named(&self, name: &str) -> bool {
        let named = self.menu_buttons().into_iter().find(|button| {
            button.is_visible() && button.is_sensitive() && button_name(button).as_deref() == Some(name)
        });
        named.inspect(gtk::MenuButton::popup).is_some()
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

impl BrowserWindow {
    /// Opens the command bar's menu named `name`, for the snapshot hook's
    /// `command-menu=` step; one open already closes, as after a click.
    pub(crate) fn open_command_bar_menu(&self, name: &str) -> bool {
        self.command_bar().open_menu_named(name)
    }
}

/// What a menu button shows, or its tooltip's first line for one that
/// shows only a glyph (See more).
fn button_name(button: &gtk::MenuButton) -> Option<String> {
    let label = button
        .child()
        .and_then(|content| descendants::<gtk::Label>(&content).into_iter().next());
    match label {
        Some(label) => Some(label.text().into()),
        None => button.tooltip_text()?.lines().next().map(str::to_owned),
    }
}

/// A press on the open menu taken from the window, until its release.
#[derive(Debug)]
pub(super) struct HeldPress {
    /// The pointer button pressed.
    button: u32,
    /// The menu, or submenu, pressed on.
    menu: MenuPopover,
    /// Its row or button under the press; none between its items.
    target: Option<gtk::Widget>,
}

impl HeldPress {
    /// Does what the click ends as in the menu, at `release`: runs the row
    /// or button pressed when the primary button is let go on it again.
    fn release(self, release: &gdk::Event) {
        let Some(target) = self.target else {
            return;
        };
        let on_menu = release.surface() == self.menu.surface();
        let still_on_target = on_menu && item_at(&self.menu, release).as_ref() == Some(&target);
        if self.button == gdk::BUTTON_PRIMARY && still_on_target {
            click(&target);
        }
    }
}

/// The pointer button of a press or release.
fn pointer_button(event: &gdk::Event) -> Option<u32> {
    event
        .downcast_ref::<gdk::ButtonEvent>()
        .map(gdk::ButtonEvent::button)
}

/// The row or button of `menu` at `event`'s position on its surface.
fn item_at(menu: &MenuPopover, event: &gdk::Event) -> Option<gtk::Widget> {
    let (x, y) = event.position()?;
    let (shadow_x, shadow_y) = menu.surface_transform();
    let picked = menu.pick(x - shadow_x, y - shadow_y, gtk::PickFlags::DEFAULT)?;
    picked
        .ancestor(gtk::ListBoxRow::static_type())
        .or_else(|| picked.ancestor(gtk::Button::static_type()))
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
