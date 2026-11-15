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
//! its menus opens, when the keyboard moves out of it (Shift+Tab), and
//! when the window shows another tab or folder. A menu that holds the
//! pointer is closed by all of these as a matter of course; one that does
//! not must be told. [`CommandBar::open_menu`] is the one menu of the bar
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
//! item before the release, or a right or middle click runs nothing. The
//! wheel and the touchpad's scrolling and gestures are sent the same
//! wrong way, and are taken too, so they never scroll or zoom the files
//! under the menu; a scroll scrolls the menu's own rows instead.
//!
//! The keyboard is unchanged: a menu takes it when it opens, Escape
//! closes it and gives it back to the button.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::super::menu_popover::MenuPopover;
use super::super::session::TabId;
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
            // After the button's own click, or its cancelling: GTK runs
            // the click from the end of its gesture, so a handler run
            // after that end sees whether the button closed its menu.
            if let Some(click) = click_gesture_of(&button) {
                let pressed = button.downgrade();
                click.connect_local(
                    "end",
                    true,
                    glib::clone!(
                        #[weak(rename_to = bar)]
                        self,
                        #[upgrade_or]
                        None,
                        move |_| {
                            if let Some(button) = pressed.upgrade() {
                                bar.after_click_on(&button);
                            }
                            None
                        }
                    ),
                );
            }
        }
        self.connect_realize(CommandBar::watch_window);
    }

    /// After a press on `button` ended: when it began on the button of
    /// the menu open then, and the menu is still open, the button's click
    /// was cancelled, as when the press is dragged off the button. The
    /// press moved the keyboard to the button, which `follow_keyboard`
    /// left to the click; the menu closes if the keyboard is outside it.
    fn after_click_on(&self, button: &gtk::MenuButton) {
        let pressed_on_open = self.imp().pressed_with_menu_open.take();
        if pressed_on_open.as_ref() != Some(button) || self.open_menu().as_ref() != Some(button) {
            return;
        }
        let Some(menu) = button.popover() else {
            return;
        };
        let focus = button.root().and_then(|root| root.focus());
        let inside =
            focus.is_some_and(|focus| focus == *menu.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&menu));
        if !inside {
            button.popdown();
        }
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
                    // A release outside the window is never seen here.
                    bar.imp().pointer_down.set(false);
                    if let Some(button) = bar.open_menu() {
                        button.popdown();
                    }
                }
            }
        ));
        window.connect_focus_widget_notify(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |window| bar.follow_keyboard(gtk::prelude::GtkWindowExt::focus(window).as_ref())
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

    /// Closes the open menu when the keyboard moves from inside it, or a
    /// submenu of it, to anything else in the window, as Shift+Tab can.
    /// A focus that goes for a moment, as when the menu rebuilds its rows,
    /// closes nothing, nor does one a pointer press moves: the press itself
    /// closes the menu, or, on the menu's own button, the click does.
    fn follow_keyboard(&self, focus: Option<&gtk::Widget>) {
        let imp = self.imp();
        let before = imp.keyboard_on.upgrade();
        imp.keyboard_on.set(focus);
        if imp.pointer_down.get() {
            return;
        }
        let Some((button, focus)) = self.open_menu().zip(focus) else {
            return;
        };
        let Some(menu) = button.popover() else {
            return;
        };
        let inside =
            |widget: &gtk::Widget| widget == menu.upcast_ref::<gtk::Widget>() || widget.is_ancestor(&menu);
        if before.as_ref().is_some_and(inside) && !inside(focus) {
            button.popdown();
        }
    }

    /// Closes the open menu when the window shows another tab or another
    /// location than when it last told the bar, as after Ctrl+Tab or Alt+Up
    /// pressed while the menu is open.
    pub(in crate::window) fn follow_shown_location(&self, tab: Option<TabId>, uri: &str) {
        let shown = (tab, uri.to_owned());
        let before = self.imp().shown_location.replace(Some(shown.clone()));
        if before.is_some_and(|before| before != shown) {
            if let Some(button) = self.open_menu() {
                button.popdown();
            }
        }
    }

    /// Follows which surface has the pointer, and takes a press on the
    /// open menu that GTK sends to the window under it, and its release.
    fn follow_pointer_event(&self, event: &gdk::Event) -> glib::Propagation {
        let imp = self.imp();
        match event.event_type() {
            gdk::EventType::ButtonPress | gdk::EventType::TouchBegin => {
                imp.pointer_down.set(true);
                imp.pressed_with_menu_open.replace(self.open_menu());
            }
            gdk::EventType::ButtonRelease | gdk::EventType::TouchEnd | gdk::EventType::TouchCancel => {
                imp.pointer_down.set(false);
            }
            _ => {}
        }
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
            // GTK sends these to the same remembered target as presses:
            // a scroll goes to the menu's rows instead, so a tall menu
            // still scrolls before the pointer moves again.
            gdk::EventType::Scroll => match self.menu_sent_past(event) {
                Some(menu) => {
                    if let Some(scroll) = event.downcast_ref::<gdk::ScrollEvent>() {
                        menu.scroll_rows_for(scroll);
                    }
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
            gdk::EventType::TouchpadSwipe | gdk::EventType::TouchpadPinch | gdk::EventType::TouchpadHold
                if self.menu_sent_past(event).is_some() =>
            {
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// The open menu, or a submenu of it, when `event` happened on it
    /// while GTK's pointer target is in the window, not in that menu.
    fn menu_sent_past(&self, event: &gdk::Event) -> Option<MenuPopover> {
        let button = self.open_menu()?;
        let surface = event.surface()?;
        if self.imp().pointer_surface.upgrade().as_ref() == Some(&surface) {
            return None;
        }
        let menu = gtk::Native::for_surface(&surface).and_downcast::<MenuPopover>()?;
        menu.is_ancestor(&button).then_some(menu)
    }

    /// For a press on the open menu, or a submenu of it, while GTK's
    /// pointer target is in the window, not in that menu: the press to
    /// take from the window, with the menu's row or button under it, if
    /// any.
    fn press_sent_past_menu(&self, event: &gdk::Event) -> Option<HeldPress> {
        let menu = self.menu_sent_past(event)?;
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

/// The click gesture of the button inside `button`, through which GTK
/// toggles the menu on a click.
fn click_gesture_of(button: &gtk::MenuButton) -> Option<gtk::GestureClick> {
    let inner = children(button).find(ObjectExt::is::<gtk::ToggleButton>)?;
    inner
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
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
