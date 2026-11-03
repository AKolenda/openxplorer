// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane following the pointer (PROP-010), as Dolphin's
//! Information panel does with "Show item under the pointer": while the
//! option is on, the pane describes the item under the pointer, and the
//! selection again once the pointer leaves the items. The pane's menu
//! saves its options here.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::DetailsPaneOptions;

use super::actions::text_action;
use super::folder_pane::FolderPane;
use super::preferences::Preference;
use super::status_bar;
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::item::FileItem;

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

    /// The pointer is at `point` in `view`, or has left it: the status
    /// bar describes the item under it (VIEW-052), and so does the details
    /// pane while it follows the pointer.
    pub(super) fn pointer_over_items(&self, view: &gtk::Widget, point: Option<(f64, f64)>) {
        // Hover descriptions belong to the active pane only.
        if self.side_holding(view) != Some(self.active_side()) {
            return;
        }
        let pane_model = self.folder_pane();
        let item = point
            .and_then(|(x, y)| pane_model.owners().position_at(view, x, y))
            .and_then(|position| pane_model.model().item(position));
        self.imp().hovered_item.replace(item.clone());
        self.describe_hovered();
        let pane = self.details_pane();
        if !pane.options().follow_hover {
            return;
        }
        if pane.set_hovered(item) {
            self.update_details_pane();
        }
    }

    /// Describes the item under the pointer in the status bar again, with
    /// its item count while the folder pane counts items.
    pub(super) fn describe_hovered(&self) {
        let item = self.imp().hovered_item.borrow().clone();
        let counts_items = self.folder_pane().details().counts_items();
        self.status_bar()
            .show_hovered(item.map(|item| status_bar::item_text(&item, counts_items)));
    }

    /// Describes a folder of `pane` again once its items are counted,
    /// when the status bar describes it: selected alone or under the
    /// pointer.
    pub(super) fn follow_item_counts(&self, pane: &FolderPane) {
        pane.owners().connect_item_counted(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |item| window.item_counted(item)
        ));
    }

    /// `item` was counted: describes it again where the status bar does.
    fn item_counted(&self, item: &FileItem) {
        if self.imp().hovered_item.borrow().as_ref() == Some(item) {
            self.describe_hovered();
        }
        let model = self.folder_pane().model();
        if let [position] = model.selected_positions()[..] {
            if model.item(position).as_ref() == Some(item) {
                self.update_status();
            }
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
