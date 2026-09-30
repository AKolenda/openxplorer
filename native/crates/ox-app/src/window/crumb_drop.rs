// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging files over the breadcrumbs (NAV-021), as Dolphin's location
//! bar takes them. A crumb takes a drop for its folder
//! ([`super::file_drop`], DND-011); holding the drag over the divider after
//! a crumb for [`DIVIDER_HOVER_DELAY`] opens that folder's subfolder menu,
//! whose rows take the drop too, so it can go deeper. The menu stays open
//! without grabbing the pointer, so the drag goes on, and closes when the
//! drag leaves it, drops anywhere, or leaves the crumbs for somewhere
//! other than the menu.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::address_bar::{AddressBar, AddressMode};
use super::crumb_menus::{list_subfolders, subfolder_menu};
use super::file_drop::DropZone;
use super::menu_popover::{MenuAction, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// How long a drag must stay over a divider before its menu opens, as
/// long as over a tab before the tab shows.
const DIVIDER_HOVER_DELAY: Duration = Duration::from_millis(800);

/// How long the drag menu stays open after the drag left the crumbs, so
/// the drag can reach the menu.
const LEAVE_CLOSE_DELAY: Duration = Duration::from_millis(300);

/// The CSS class of the menu row a drop would go into.
const ROW_DROP_CLASS: &str = "file-drop-active";

/// The subfolder menu a file drag opened, which takes the drop.
#[derive(Debug, Default)]
pub(super) struct DragCrumbMenu {
    /// The open menu.
    menu: RefCell<Option<MenuPopover>>,
    /// The timer that closes it after the drag left the crumbs.
    closing: RefCell<Option<glib::SourceId>>,
    /// Counts closings, so a listing that ends after one opens nothing.
    generation: Cell<u64>,
}

impl DragCrumbMenu {
    /// Stops the timer that would close the menu.
    fn keep_open(&self) {
        if let Some(timer) = self.closing.take() {
            timer.remove();
        }
    }
}

/// The folder a crumb button opens.
fn crumb_folder(widget: &gtk::Widget) -> Option<String> {
    let target = widget.downcast_ref::<gtk::Button>()?.action_target_value()?;
    target.str().map(str::to_owned)
}

impl AddressBar {
    /// The folder of the crumb before the divider at (`x`, `y`), and the
    /// folder of the crumb after it; `None` elsewhere.
    pub(super) fn divider_at(&self, x: f64, y: f64) -> Option<(String, String)> {
        if self.mode() != AddressMode::Crumbs {
            return None;
        }
        let picked = self.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let divider = std::iter::successors(Some(picked), WidgetExt::parent)
            .find(|widget| widget.has_css_class("crumb-divider"))?;
        let folder = crumb_folder(&divider.prev_sibling()?)?;
        let shown = crumb_folder(&divider.next_sibling()?)?;
        Some((folder, shown))
    }
}

impl BrowserWindow {
    /// A drag moved to (`x`, `y`) of the address bar: over a divider, its
    /// menu opens after [`DIVIDER_HOVER_DELAY`]; anywhere else the wait
    /// stops.
    pub(super) fn open_subfolders_after_hover(&self, x: f64, y: f64) {
        let divider = self.address_bar().divider_at(x, y);
        let waiting = self
            .imp()
            .divider_hover
            .borrow()
            .as_ref()
            .map(|(folder, _)| folder.clone());
        if waiting.is_some() && waiting.as_deref() == divider.as_ref().map(|(folder, _)| folder.as_str()) {
            return;
        }
        self.stop_divider_hover();
        let Some((folder, shown)) = divider else { return };
        let hovered = folder.clone();
        let timer = glib::timeout_add_local_once(
            DIVIDER_HOVER_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().divider_hover.replace(None);
                    window.open_drag_crumb_menu(folder, shown);
                }
            ),
        );
        self.imp().divider_hover.replace(Some((hovered, timer)));
    }

    /// Stops waiting to open a divider's menu.
    pub(super) fn stop_divider_hover(&self) {
        if let Some((_, timer)) = self.imp().divider_hover.take() {
            timer.remove();
        }
    }

    /// Opens `folder`'s subfolder menu for a drag, with `shown` in bold,
    /// unless the drag ended or left while the folder was listed.
    pub(super) fn open_drag_crumb_menu(&self, folder: String, shown: String) {
        let generation = self.imp().drag_crumb_menu.generation.get();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let folders = list_subfolders(&folder, window.shows_hidden_files()).await;
                let state = &window.imp().drag_crumb_menu;
                if state.generation.get() != generation {
                    return;
                }
                let entries = subfolder_menu(&folders, &folder, &shown, 0);
                window.close_drag_crumb_menu();
                let Some(menu) = window.popup_crumb_menu(&folder, entries, false) else {
                    return;
                };
                window.attach_file_drop_zone(&menu.row_list(), DropZone::CrumbMenu);
                window.imp().drag_crumb_menu.menu.replace(Some(menu));
            }
        ));
    }

    /// The drag is over the crumbs or the menu, so the menu stays.
    pub(super) fn keep_drag_crumb_menu(&self) {
        self.imp().drag_crumb_menu.keep_open();
    }

    /// The drag left the crumbs: the menu closes unless it reaches the
    /// menu or the crumbs again within [`LEAVE_CLOSE_DELAY`], and a
    /// divider's menu no longer opens.
    pub(super) fn leave_crumbs_during_drag(&self) {
        self.stop_divider_hover();
        let state = &self.imp().drag_crumb_menu;
        state.keep_open();
        let timer = glib::timeout_add_local_once(
            LEAVE_CLOSE_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().drag_crumb_menu.closing.replace(None);
                    window.close_drag_crumb_menu();
                }
            ),
        );
        state.closing.replace(Some(timer));
    }

    /// The folder of the drag menu's row at `y`, where a drop would go.
    pub(super) fn drag_crumb_menu_folder_at(&self, y: f64) -> Option<String> {
        let menu = self.imp().drag_crumb_menu.menu.borrow().clone()?;
        let item = menu.item_at(y)?;
        if item.action != MenuAction::Window(WindowAction::GoTo) {
            return None;
        }
        item.target?.str().map(str::to_owned)
    }

    /// Highlights the drag menu's row of `folder`, or none.
    pub(super) fn highlight_drag_crumb_menu(&self, folder: Option<&str>) {
        if let Some(menu) = self.imp().drag_crumb_menu.menu.borrow().as_ref() {
            let target = folder.map(ToVariant::to_variant);
            menu.mark_row(target.as_ref(), ROW_DROP_CLASS);
        }
    }

    /// Closes the menu a drag opened, and any it is still listing for.
    pub(super) fn close_drag_crumb_menu(&self) {
        let state = &self.imp().drag_crumb_menu;
        state.keep_open();
        state.generation.set(state.generation.get().wrapping_add(1));
        if let Some(menu) = state.menu.take() {
            menu.popdown();
        }
    }

    /// The menu a drag opened, for tests.
    #[cfg(test)]
    pub(super) fn drag_crumb_menu(&self) -> Option<super::menu_popover::MenuPopover> {
        self.imp().drag_crumb_menu.menu.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::harness::{wait_until, Fixture, TestWindow};

    use super::*;

    /// parity: NAV-021
    #[gtk::test]
    fn holding_a_drag_over_a_divider_opens_a_menu_whose_rows_take_the_drop() {
        let fixture = Fixture::standard();
        std::fs::create_dir(fixture.path("Documents/Letters")).expect("fixture subfolder");
        let test = TestWindow::open(&fixture.uri_of("Documents/Letters"));
        let address = test.window.address_bar();
        let crumbs = address.crumb_buttons();
        let documents = &crumbs[crumbs.len() - 2];
        let divider = documents.next_sibling().expect("a divider after the crumb");
        let bounds = divider.compute_bounds(address).expect("a shown divider");
        let (x, y) = (f64::from(bounds.x()) + 1.0, f64::from(bounds.y()) + 2.0);

        assert_eq!(
            address.divider_at(x, y),
            Some((fixture.uri_of("Documents"), fixture.uri_of("Documents/Letters")))
        );
        test.window.open_subfolders_after_hover(x, y);

        wait_until("the drag menu", || test.window.drag_crumb_menu().is_some());
        let menu = test.window.drag_crumb_menu().expect("the menu");
        assert!(!menu.is_autohide(), "the drag goes on");
        wait_until("the menu to show", || {
            menu.rows().iter().all(|row| row.height() > 0)
        });
        let row = menu.row("Letters");
        let row_y = row.compute_bounds(&menu.row_list()).expect("a shown row").y();
        let folder = test.window.drag_crumb_menu_folder_at(f64::from(row_y) + 2.0);
        assert_eq!(folder, Some(fixture.uri_of("Documents/Letters")));
        test.window.highlight_drag_crumb_menu(folder.as_deref());
        assert!(row.has_css_class(ROW_DROP_CLASS));
        test.window.leave_crumbs_during_drag();
        test.window.keep_drag_crumb_menu();
        crate::test_support::harness::wait_for(LEAVE_CLOSE_DELAY * 2);
        assert!(
            test.window.drag_crumb_menu().is_some(),
            "reaching the menu keeps it"
        );
        test.window.leave_crumbs_during_drag();
        wait_until("the menu to close", || test.window.drag_crumb_menu().is_none());

        test.window
            .open_drag_crumb_menu(fixture.uri_of("Documents"), String::new());
        test.window.close_drag_crumb_menu();
        crate::test_support::harness::wait_for(LEAVE_CLOSE_DELAY);
        assert!(
            test.window.drag_crumb_menu().is_none(),
            "a late listing opens nothing"
        );
    }
}
