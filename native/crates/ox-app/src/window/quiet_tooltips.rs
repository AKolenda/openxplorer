// SPDX-License-Identifier: AGPL-3.0-only
//! No tooltips while items are dragged or the drop menu is open (DND-027),
//! and none left over a menu, the rename box or the next folder
//! (VIEW-073).
//!
//! Windows Explorer shows no tooltip during a drag or while a menu is
//! open. GTK 4 shows the tooltips of the rows a drag passes over on
//! Wayland, in any window of the app, and of the row under a drop menu,
//! and has no switch for tooltips, so each window turns off the tooltip of
//! each of its widgets that has one, menus apart: every window during a
//! drag, the drop menu's window while it is open, the window renaming an
//! item in place while the rename lasts, and the window whose menu or
//! other popover has the keyboard while it is open. They come back once
//! no reason holds. A drag's tooltips go off once it has started: changing widgets
//! while GTK starts it stops the drag.
//!
//! GTK 4 also keeps a tooltip on screen until the pointer moves, so one
//! shown over an item stayed over the menu Shift+F10, the Menu key or a
//! right-click opened from it, over the rename box F2 opened and over the
//! folder a double-click opened. As in Windows, any key or button pressed
//! in the window, and every change of location, hides the tooltip shown,
//! and none shows until the pointer moves: GTK would show the tooltip of
//! whatever came under the resting pointer at once.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::BrowserWindow;

/// How far, in pixels, the pointer moves before tooltips come back after
/// a press hid them: GTK reports a resting pointer again when the widget
/// under it changes.
const POINTER_SLACK: f64 = 3.0;

/// The CSS class of the text field that renames an item in place
/// (`file_ops/inline_rename.rs`).
const RENAME_FIELD_CLASS: &str = "rename-field";

/// A reason for a window's tooltips to be off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TooltipReason {
    /// This window's items are being dragged.
    Drag,
    /// The drop menu is open or about to open.
    DropMenu,
    /// The keyboard is in a name renamed in place.
    Rename,
    /// A key or a button was pressed, or the window went elsewhere,
    /// while a tooltip showed: none shows until the pointer moves.
    Pressed,
    /// The keyboard is in a menu or another popover.
    Menu,
}

impl TooltipReason {
    /// The reason's bit in [`TooltipsOff`].
    const fn bit(self) -> u8 {
        match self {
            TooltipReason::Drag => 1,
            TooltipReason::DropMenu => 2,
            TooltipReason::Rename => 4,
            TooltipReason::Menu => 8,
            TooltipReason::Pressed => 16,
        }
    }
}

/// Why tooltips are off: the [`TooltipReason`]s that hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TooltipsOff(u8);

impl TooltipsOff {
    /// True while any reason holds.
    fn any(self) -> bool {
        self.0 != 0
    }

    /// Whether `reason` holds.
    fn holds(self, reason: TooltipReason) -> bool {
        self.0 & reason.bit() != 0
    }

    /// These reasons, with `reason` holding or not as `holds` says.
    fn with(self, reason: TooltipReason, holds: bool) -> Self {
        if holds {
            Self(self.0 | reason.bit())
        } else {
            Self(self.0 & !reason.bit())
        }
    }
}

/// Turns tooltips off in every window of the app for a drag (`during`),
/// or lets them back.
fn set_drag_tooltips_off_everywhere(during: bool) {
    for window in gtk::Window::list_toplevels() {
        if let Ok(window) = window.downcast::<BrowserWindow>() {
            window.set_drag_tooltips_off(during);
        }
    }
}

/// The widgets under `widget`, `widget` included, that show a tooltip,
/// leaving out menus and other popovers.
fn widgets_with_tooltips(widget: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
    if widget.is::<gtk::Popover>() {
        return;
    }
    if widget.has_tooltip() {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets_with_tooltips(&current, found);
        child = current.next_sibling();
    }
}

/// Turns off the tooltip of every widget under `root` that shows one,
/// menus apart, and returns those widgets, to turn them on again.
fn mute_tooltips(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    widgets_with_tooltips(root, &mut found);
    for widget in &found {
        widget.set_has_tooltip(false);
    }
    found
}

impl BrowserWindow {
    /// Hides the tooltip shown on every key or button pressed in the
    /// window, before the widget under it has its turn, as a menu, a
    /// rename or another folder may follow; and keeps tooltips off while
    /// the keyboard is in a menu or another popover, where the pointer
    /// crossing into the menu would show one again.
    pub(super) fn install_tooltip_hiding(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, _, _| {
                window.hide_tooltip();
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(keys);
        let press = gtk::GestureClick::new();
        press.set_button(super::gestures::EVERY_BUTTON);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.hide_tooltip()
        ));
        self.add_controller(press);
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |motion, x, y| {
                // Only the window's own surface: a menu's pointer moves
                // in the menu's coordinates.
                let on_window = motion
                    .current_event()
                    .and_then(|event| event.surface())
                    .zip(window.surface())
                    .is_some_and(|(surface, own)| surface == own);
                if on_window {
                    window.pointer_moved(x, y);
                }
            }
        ));
        self.add_controller(motion);
        self.connect_notify_local(Some("focus-widget"), |window, _| {
            window.follow_focus_for_tooltips();
        });
    }

    /// Keeps tooltips off while the keyboard is in a menu or another
    /// popover, or in a name renamed in place, so none covers the menu or
    /// the rename box, also once the pointer moves on.
    fn follow_focus_for_tooltips(&self) {
        let focus = gtk::prelude::GtkWindowExt::focus(self);
        let popover = focus
            .as_ref()
            .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
            .filter(WidgetExt::is_visible);
        let in_popover = popover.is_some();
        if let Some(popover) = popover {
            self.look_again_once_closed(&popover);
        }
        let in_rename = focus.as_ref().is_some_and(|focus| {
            std::iter::successors(Some(focus.clone()), WidgetExt::parent)
                .take(3)
                .any(|widget| widget.has_css_class(RENAME_FIELD_CLASS))
        });
        let off = self.imp().tooltips_off.get();
        if off.holds(TooltipReason::Menu) != in_popover {
            self.set_tooltip_reason(TooltipReason::Menu, in_popover);
        }
        if off.holds(TooltipReason::Rename) != in_rename {
            self.set_tooltip_reason(TooltipReason::Rename, in_rename);
        }
    }

    /// Follows the keyboard again once `popover` closes: GTK may leave
    /// the window's focus on a widget of the closed popover.
    fn look_again_once_closed(&self, popover: &gtk::Popover) {
        let watched = self.imp().watched_popover.upgrade();
        if watched.as_ref() == Some(popover) {
            return;
        }
        self.imp().watched_popover.set(Some(popover));
        popover.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |popover| {
                if window.imp().watched_popover.upgrade().as_ref() == Some(popover) {
                    window.imp().watched_popover.set(None);
                }
                window.follow_focus_for_tooltips();
            }
        ));
    }

    /// Hides the tooltip the window shows now, if any, and shows none
    /// until the pointer moves: GTK would otherwise show the tooltip of
    /// whatever comes under the resting pointer at once, such as the
    /// first item of the folder a double-click opened. GTK has no call
    /// that hides a tooltip: the widgets' tooltips go off, and GTK, asked
    /// to look under the pointer again, finds none and hides it.
    pub(super) fn hide_tooltip(&self) {
        // Off already, and hidden when they went off; or none shows.
        if self.imp().tooltips_off.get().any() || !self.shows_tooltip() {
            return;
        }
        self.watch_tooltip_window();
        self.imp().pointer_when_hidden.set(self.imp().pointer_now.get());
        self.set_tooltip_reason(TooltipReason::Pressed, true);
    }

    /// Hides a tooltip that shows while tooltips are off, once GTK has
    /// made its popup: a widget made since they went off, such as a row
    /// of the folder just opened, still has its tooltip.
    fn watch_tooltip_window(&self) {
        if self.imp().watches_tooltip.get() {
            return;
        }
        let Some(popup) =
            super::widget_tree::children(self).find(|child| child.type_().name() == "GtkTooltipWindow")
        else {
            return;
        };
        self.imp().watches_tooltip.set(true);
        popup.connect_visible_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |popup| {
                if popup.is_visible() && window.imp().tooltips_off.get().any() {
                    glib::idle_add_local_once(glib::clone!(
                        #[weak]
                        window,
                        move || window.mute_tooltips_again()
                    ));
                }
            }
        ));
    }

    /// Turns off the tooltips of widgets made since tooltips went off,
    /// and hides the one they showed.
    fn mute_tooltips_again(&self) {
        if !self.imp().tooltips_off.get().any() {
            return;
        }
        let found = mute_tooltips(self.upcast_ref());
        self.trigger_tooltip_query();
        self.imp()
            .muted_tooltips
            .borrow_mut()
            .extend(found.iter().map(ObjectExt::downgrade));
    }

    /// Notes where the pointer is in the window, and lets tooltips back
    /// once it moved a few pixels from where it was when a press hid them.
    fn pointer_moved(&self, x: f64, y: f64) {
        let imp = self.imp();
        imp.pointer_now.set(Some((x, y)));
        if !imp.tooltips_off.get().holds(TooltipReason::Pressed) {
            return;
        }
        let moved = imp
            .pointer_when_hidden
            .get()
            .is_none_or(|(from_x, from_y)| (x - from_x).abs() + (y - from_y).abs() > POINTER_SLACK);
        if moved {
            self.set_tooltip_reason(TooltipReason::Pressed, false);
        }
    }

    /// Whether no reason keeps tooltips off now, for tests.
    #[cfg(test)]
    pub(super) fn tooltips_may_show(&self) -> bool {
        !self.imp().tooltips_off.get().any()
    }

    /// Where the window last saw the pointer, in its coordinates, for
    /// tests.
    #[cfg(test)]
    pub(super) fn pointer_seen_at(&self) -> Option<(f64, f64)> {
        self.imp().pointer_now.get()
    }

    /// Whether a tooltip shows over the window: GTK draws it in a popup
    /// of its own, a child of the window.
    fn shows_tooltip(&self) -> bool {
        super::widget_tree::children(self)
            .any(|child| child.type_().name() == "GtkTooltipWindow" && child.is_visible())
    }

    /// Turns tooltips off while the drop menu is open or about to open
    /// (`open`), or lets them back once it closed.
    pub(super) fn quiet_tooltips_for_drop_menu(&self, open: bool) {
        self.set_tooltip_reason(TooltipReason::DropMenu, open);
    }

    /// Turns tooltips off in every window of the app while a drag of this
    /// window's items lasts, once GTK has started it (`during`), as the
    /// drag can pass over any of them; or lets them back once it ended.
    pub(super) fn quiet_tooltips_for_drag(&self, during: bool) {
        if !during {
            set_drag_tooltips_off_everywhere(false);
            return;
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                // The drag may have ended already.
                if window.imp().outgoing_drag.borrow().is_some() {
                    set_drag_tooltips_off_everywhere(true);
                }
            }
        ));
    }

    /// Turns tooltips off in this window for a drag (`during`), or lets
    /// them back.
    fn set_drag_tooltips_off(&self, during: bool) {
        self.set_tooltip_reason(TooltipReason::Drag, during);
    }

    /// Records whether `reason` holds, and turns tooltips off or on as
    /// the reasons now ask.
    fn set_tooltip_reason(&self, reason: TooltipReason, holds: bool) {
        let off = self.imp().tooltips_off.get().with(reason, holds);
        self.set_tooltips_off(off);
    }

    /// Records why tooltips are off, and turns them off or on as the
    /// reasons now ask.
    fn set_tooltips_off(&self, off: TooltipsOff) {
        let was = self.imp().tooltips_off.replace(off);
        match (was.any(), off.any()) {
            (false, true) => {
                self.watch_tooltip_window();
                let found = mute_tooltips(self.upcast_ref());
                // The one shown now goes too.
                self.trigger_tooltip_query();
                self.imp()
                    .muted_tooltips
                    .replace(found.iter().map(ObjectExt::downgrade).collect());
            }
            (true, false) => {
                for widget in self.imp().muted_tooltips.take() {
                    if let Some(widget) = widget.upgrade() {
                        widget.set_has_tooltip(true);
                    }
                }
            }
            _ => {}
        }
    }
}
