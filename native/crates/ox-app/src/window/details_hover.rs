// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane following the pointer (PROP-010), as Dolphin's
//! Information panel does with "Show item under the pointer": while the
//! option is on, the pane describes the item under the pointer, and the
//! selection again once the pointer leaves the items. The pane's menu
//! saves its options here.

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::DetailsPaneOptions;

use super::actions::text_action;
use super::preferences::Preference;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Follows the pointer over the items of `view` for the details pane.
    pub(super) fn attach_details_hover(&self, view: &gtk::Widget) {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |_, x, y| window.pointer_over_items(&view, Some((x, y)))
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |_| window.pointer_over_items(&view, None)
        ));
        view.add_controller(motion);
    }

    /// The pointer is at `point` in `view`, or has left it.
    pub(super) fn pointer_over_items(&self, view: &gtk::Widget, point: Option<(f64, f64)>) {
        let pane = self.details_pane();
        // The details pane speaks for the active pane of a split tab.
        let in_active_pane = self.side_holding(view) == Some(self.active_side());
        if !pane.options().follow_hover || !in_active_pane {
            return;
        }
        let pane_model = self.folder_pane();
        let item = point
            .and_then(|(x, y)| pane_model.owners().position_at(view, x, y))
            .and_then(|position| pane_model.model().item(position));
        if pane.set_hovered(item) {
            self.update_details_pane();
        }
    }

    /// Adds `win.details-pane-option`, which the pane's menu runs.
    pub(super) fn install_details_pane_actions(&self) {
        let option = text_action(WindowAction::DetailsPaneOption, |window, name| {
            window.details_pane().toggle_option(name);
        });
        self.add_action_entries([option]);
    }

    /// Saves the options chosen in the details pane's menu and redraws the
    /// pane with them.
    pub(super) fn details_options_changed(&self, options: DetailsPaneOptions) {
        self.save_preference(Preference::DetailsPaneOptions(options));
        self.update_details_pane();
    }
}
