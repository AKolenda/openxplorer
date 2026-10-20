// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's View > Show > Item check boxes (SEL-014), here View >
//! Item check boxes and Settings > Item check boxes: one choice, saved,
//! that every window follows. On by default, as in Windows 11.
//!
//! The check boxes are the folder views' (`folder_view/cells/item_check.rs`
//! and `folder_view/details/select_all.rs`); this is the toggle. The
//! preference keeps its saved name, `selectionMarker`, from when this was a
//! hover marker.
//!
//! "Show check boxes only while Ctrl is held" (`itemChecksOnCtrl`, on by
//! default) keeps a hovered item's box transparent, and lets clicks
//! through to its row, until Ctrl is held: the window carries
//! `checks-on-ctrl` while that is so (folder-views.css). A checked box
//! still shows; the Details header's select-all box is not affected.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::actions::toggle_action;
use super::preferences::Preference;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Adds the Item check boxes toggle, starting from the saved
    /// preference.
    pub(super) fn install_item_check_boxes_action(&self) {
        let shown = self.context().settings_data().preferences.selection_marker;
        self.add_action_entries([toggle_action(
            WindowAction::ItemCheckBoxes,
            shown,
            |window, shown| {
                window.show_item_check_boxes(shown);
                window.save_preference(Preference::ItemCheckBoxes(shown));
            },
        )]);
    }

    /// Follows Ctrl as it is pressed and released anywhere in the window,
    /// for item check boxes that wait for it. The two Ctrl keys are
    /// followed apart, so releasing one while the other is held keeps the
    /// boxes. A window that loses focus forgets Ctrl, since it will not
    /// hear the release, and asks the keyboard again when it gets focus
    /// back; any other key, and any pointer move, also tells whether Ctrl
    /// is still down, which ends a release a menu or popover heard
    /// instead of the window. Starts from the saved preference.
    pub(super) fn install_ctrl_for_item_checks(&self) {
        let wait = self.context().settings_data().preferences.item_checks_on_ctrl;
        self.wait_for_ctrl_with_item_checks(wait);
        let held = Rc::new(Cell::new(CtrlKeys::default()));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            held,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| {
                let keys = match key {
                    gtk::gdk::Key::Control_L => CtrlKeys {
                        left: true,
                        ..held.get()
                    },
                    gtk::gdk::Key::Control_R => CtrlKeys {
                        right: true,
                        ..held.get()
                    },
                    // A release missed while another window had the
                    // keyboard ends here.
                    _ if !state.contains(gtk::gdk::ModifierType::CONTROL_MASK) => CtrlKeys::default(),
                    _ => held.get(),
                };
                held.set(keys);
                window.hold_ctrl(keys.is_held());
                glib::Propagation::Proceed
            }
        ));
        keys.connect_key_released(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            held,
            move |_, key, _, _| {
                let keys = match key {
                    gtk::gdk::Key::Control_L => CtrlKeys {
                        left: false,
                        ..held.get()
                    },
                    gtk::gdk::Key::Control_R => CtrlKeys {
                        right: false,
                        ..held.get()
                    },
                    _ => return,
                };
                held.set(keys);
                window.hold_ctrl(keys.is_held());
            }
        ));
        self.add_controller(keys);
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            held,
            move |motion, _, _| {
                let is_down = motion
                    .current_event_state()
                    .contains(gtk::gdk::ModifierType::CONTROL_MASK);
                if is_down != held.get().is_held() {
                    // Which Ctrl key is down is unknown here; either counts.
                    held.set(CtrlKeys {
                        left: is_down,
                        right: false,
                    });
                    window.hold_ctrl(is_down);
                }
            }
        ));
        self.add_controller(motion);
        self.connect_is_active_notify(move |window| {
            // Which Ctrl key is down is unknown on return; either counts.
            let is_down = window.is_active() && is_ctrl_down(window);
            held.set(CtrlKeys {
                left: is_down,
                right: false,
            });
            window.hold_ctrl(is_down);
        });
    }

    /// Notes whether Ctrl is `held` and shows or hides the hovered item's
    /// check box to match.
    pub(super) fn hold_ctrl(&self, held: bool) {
        if held {
            self.add_css_class("ctrl-held");
        } else {
            self.remove_css_class("ctrl-held");
        }
        self.update_item_checks_waiting();
    }

    /// Makes item check boxes wait for Ctrl, or show on hover as before.
    pub(super) fn wait_for_ctrl_with_item_checks(&self, wait: bool) {
        if wait {
            self.add_css_class("item-checks-wait");
        } else {
            self.remove_css_class("item-checks-wait");
        }
        self.update_item_checks_waiting();
    }

    /// Hides the hovered item's check box, and lets clicks through it,
    /// while the boxes wait for a Ctrl that is not held.
    fn update_item_checks_waiting(&self) {
        let waiting = self.has_css_class("item-checks-wait") && !self.has_css_class("ctrl-held");
        if waiting {
            self.add_css_class("checks-on-ctrl");
        } else {
            self.remove_css_class("checks-on-ctrl");
        }
        for pane in self.folder_panes() {
            pane.owners().set_item_checks_take_clicks(!waiting);
        }
    }

    /// Gives every item of both panes a check box, and the Details header
    /// its select-all box, or none.
    pub(super) fn show_item_check_boxes(&self, shown: bool) {
        for pane in self.folder_panes() {
            pane.owners().set_item_checks(shown);
            pane.details().show_select_all(shown);
        }
    }

    /// Whether the toggle is on, `None` before the action is installed.
    fn item_check_boxes_state(&self) -> Option<bool> {
        let state = self.window_action_state(WindowAction::ItemCheckBoxes)?;
        state.get::<bool>()
    }

    /// Ticks the toggle for a choice that Settings or another window
    /// saved; the panes take it up in `follow_item_preferences`.
    pub(super) fn follow_item_check_boxes_preference(&self) {
        let saved = self.context().settings_data().preferences.selection_marker;
        if self.item_check_boxes_state().is_some_and(|shown| shown != saved) {
            self.set_action_state(WindowAction::ItemCheckBoxes, &saved.to_variant());
        }
    }
}

/// Which Ctrl keys are held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CtrlKeys {
    left: bool,
    right: bool,
}

impl CtrlKeys {
    /// Whether either is held.
    const fn is_held(self) -> bool {
        self.left || self.right
    }
}

/// Whether the keyboard of `window`'s seat holds Ctrl now.
fn is_ctrl_down(window: &BrowserWindow) -> bool {
    let keyboard = WidgetExt::display(window)
        .default_seat()
        .and_then(|seat: gtk::gdk::Seat| seat.keyboard());
    keyboard.is_some_and(|keyboard: gtk::gdk::Device| {
        keyboard
            .modifier_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
    })
}
