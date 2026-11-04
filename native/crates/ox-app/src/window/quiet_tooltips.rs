// SPDX-License-Identifier: AGPL-3.0-only
//! No tooltips while items are dragged or the drop menu is open (DND-027).
//!
//! Windows Explorer shows no tooltip during a drag or while a menu is
//! open. GTK 4 shows the tooltips of the rows a drag passes over on
//! Wayland, and of the row under a drop menu, and has no switch for
//! tooltips, so the window turns off the tooltip of each of its widgets
//! that has one, menus apart, and turns them on again once neither reason
//! holds. A drag's tooltips go off once it has started: changing widgets
//! while GTK starts it stops the drag.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::BrowserWindow;

/// Why tooltips are off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TooltipsOff {
    /// This window's items are being dragged.
    pub(crate) drag: bool,
    /// The drop menu is open or about to open.
    pub(crate) drop_menu: bool,
}

impl TooltipsOff {
    /// True while any reason holds.
    fn any(self) -> bool {
        self.drag || self.drop_menu
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

impl BrowserWindow {
    /// Turns tooltips off while the drop menu is open or about to open
    /// (`open`), or lets them back once it closed.
    pub(super) fn quiet_tooltips_for_drop_menu(&self, open: bool) {
        let off = self.imp().tooltips_off.get();
        self.set_tooltips_off(TooltipsOff {
            drop_menu: open,
            ..off
        });
    }

    /// Turns tooltips off while a drag of this window's items lasts, once
    /// GTK has started it (`during`), or lets them back once it ended.
    pub(super) fn quiet_tooltips_for_drag(&self, during: bool) {
        if !during {
            let off = self.imp().tooltips_off.get();
            self.set_tooltips_off(TooltipsOff { drag: false, ..off });
            return;
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                // The drag may have ended already.
                if window.imp().outgoing_drag.borrow().is_some() {
                    let off = window.imp().tooltips_off.get();
                    window.set_tooltips_off(TooltipsOff { drag: true, ..off });
                }
            }
        ));
    }

    /// Records why tooltips are off, and turns them off or on as the
    /// reasons now ask.
    fn set_tooltips_off(&self, off: TooltipsOff) {
        let was = self.imp().tooltips_off.replace(off);
        match (was.any(), off.any()) {
            (false, true) => {
                let mut found = Vec::new();
                widgets_with_tooltips(self.upcast_ref(), &mut found);
                for widget in &found {
                    widget.set_has_tooltip(false);
                }
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
