// SPDX-License-Identifier: AGPL-3.0-only
//! Split view (VIEW-059, INT-005): a tab that shows two folders side by
//! side.
//!
//! Ports Dolphin's Split (F3, `DolphinMainWindow::toggleSplitView` and
//! `DolphinTabPage`). The window has two folder panes; a tab that is not
//! split shows in the left one. Each pane of a split tab is a tab of the
//! session of its own ([`super::session`]), so it keeps its own location,
//! history, selection and listing, and every command, the address bar,
//! the status bar and the details pane follow the active pane, because
//! the active pane is what the rest of the window calls the active tab.
//! A click, a drag or keyboard focus entering the other pane makes it
//! active. The active pane's caption carries the accent line, as the
//! selected item of a Windows 11 navigation view does, and the other pane
//! is dimmed, as Dolphin dims it. F3 again closes the active pane.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::LocationError;

use crate::locations::Page;

use super::actions::toggle_action;
use super::folder_pane::FolderPane;
use super::loading::LoadMode;
use super::navigation::FocusOnShow;
use super::pane_content::{show_pane_state, PaneState};
use super::session::{PaneSide, Tab, TabId};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The class of the active pane's column while the tab is split.
const ACTIVE_PANE: &str = "active-pane";

/// The class of the other pane's column.
const INACTIVE_PANE: &str = "inactive-pane";

/// What the pane beside the active one needs to be put on screen.
struct BesideView {
    id: TabId,
    store: gtk::gio::ListStore,
    selected: Vec<String>,
    scroll: f64,
    needs_listing: bool,
}

impl BrowserWindow {
    /// The folder pane on `side`.
    pub(super) fn pane_on(&self, side: PaneSide) -> &FolderPane {
        match side {
            PaneSide::Start => &self.imp().folder_pane,
            PaneSide::End => &self.imp().split_pane,
        }
    }

    /// Both folder panes, the left one first.
    pub(super) fn folder_panes(&self) -> [&FolderPane; 2] {
        [self.pane_on(PaneSide::Start), self.pane_on(PaneSide::End)]
    }

    /// The column that holds the folder pane on `side` and its caption.
    fn pane_column(&self, side: PaneSide) -> &gtk::Box {
        match side {
            PaneSide::Start => &self.imp().start_pane_column,
            PaneSide::End => &self.imp().end_pane_column,
        }
    }

    /// The caption above the folder pane on `side`.
    fn pane_caption(&self, side: PaneSide) -> &gtk::Label {
        match side {
            PaneSide::Start => &self.imp().start_pane_caption,
            PaneSide::End => &self.imp().end_pane_caption,
        }
    }

    /// The side whose folder pane holds `widget`, if one does.
    pub(super) fn side_holding(&self, widget: &gtk::Widget) -> Option<PaneSide> {
        [PaneSide::Start, PaneSide::End]
            .into_iter()
            .find(|side| widget.is_ancestor(self.pane_column(*side)))
    }

    /// The side of the folder pane that shows the active pane.
    pub(super) fn active_side(&self) -> PaneSide {
        self.imp().active_side.get()
    }

    /// Whether `pane` is the folder pane of the active pane.
    pub(super) fn is_active_pane(&self, pane: &FolderPane) -> bool {
        pane == self.folder_pane()
    }

    /// Adds `win.split-view` (F3) and makes a click, a key or a drag in
    /// either pane of a split tab make that pane active.
    pub(super) fn install_split_view(&self) {
        self.add_action_entries([toggle_action(WindowAction::SplitView, false, |window, _| {
            window.toggle_split_view();
        })]);
        for side in [PaneSide::Start, PaneSide::End] {
            self.follow_pane_activation(side);
        }
    }

    /// Makes the pane on `side` active when a press lands in it or keyboard
    /// focus enters it, before the pane itself sees the press.
    fn follow_pane_activation(&self, side: PaneSide) {
        let column = self.pane_column(side);
        let press = gtk::GestureClick::new();
        press.set_button(0);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.activate_pane(side)
        ));
        column.add_controller(press);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.activate_pane(side)
        ));
        column.add_controller(focus);
    }

    /// F3: splits the tab, or closes its active pane.
    pub(super) fn toggle_split_view(&self) {
        let is_split = self.imp().session.borrow().active().is_some_and(Tab::is_split);
        if self.refuse_while_active_tab_moves() {
            self.show_split_state();
        } else if is_split {
            self.close_active_pane();
        } else if let Err(error) = self.split_tab(None) {
            self.show_message(&error.to_string());
        }
    }

    /// Splits the active tab: a new pane opens beside it at `address`, or
    /// at the active folder, and becomes active.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(crate) fn split_tab(&self, address: Option<&str>) -> Result<(), LocationError> {
        let uri = match address {
            Some(address) => Some(self.resolve_address(address)?),
            None => self.current_uri(),
        };
        // Settings has one page per window, never a pane of its own.
        let Some(uri) = uri.filter(|uri| Page::from_uri(uri) != Some(Page::Settings)) else {
            return Ok(());
        };
        self.leave_pane();
        let split = self.imp().session.borrow_mut().split_active(&uri);
        if let Some(id) = split {
            // The new pane starts with the view and order of the one it
            // opened from.
            self.copy_pane_view(self.active_side(), self.active_side().other());
            self.show_tab(id);
        }
        Ok(())
    }

    /// Closes the active pane of a split tab; the other pane stays, on
    /// its own, in the left folder pane, with its view and order.
    fn close_active_pane(&self) {
        self.leave_pane();
        self.save_beside_view();
        let remaining_side = self.active_side().other();
        let remaining = self.imp().session.borrow_mut().close_active_pane();
        if let Some(id) = remaining {
            self.copy_pane_view(remaining_side, PaneSide::Start);
            self.show_tab(id);
        }
    }

    /// Shows the folder pane on `to` in the view and order of the one on
    /// `from`.
    fn copy_pane_view(&self, from: PaneSide, to: PaneSide) {
        if from == to {
            return;
        }
        let (from, to) = (self.pane_on(from), self.pane_on(to));
        to.show_view(from.view());
        to.details().sort_by(from.details().sort_order());
    }

    /// Makes the pane on `side` active, when the tab in front is split and
    /// that pane is not active yet. Keyboard focus stays where it goes.
    pub(super) fn activate_pane(&self, side: PaneSide) {
        let is_split = self.imp().session.borrow().active().is_some_and(Tab::is_split);
        if !is_split || side == self.imp().active_side.get() {
            return;
        }
        self.leave_pane();
        self.save_beside_view();
        let active = self.imp().session.borrow_mut().activate_beside();
        if let Some(id) = active {
            self.show_tab_with(id, FocusOnShow::Keep);
        }
    }

    /// Tab in a folder view of a split tab, when the settings ask for it:
    /// the other pane becomes active with keyboard focus in its list.
    /// False when Tab does something else.
    pub(super) fn tab_to_other_pane(&self) -> bool {
        let switches = self
            .context()
            .settings_data()
            .preferences
            .tab_switches_split_panes;
        let is_split = self.imp().session.borrow().active().is_some_and(Tab::is_split);
        if !(switches && is_split) {
            return false;
        }
        self.activate_pane(self.imp().active_side.get().other());
        self.folder_pane().focus_view();
        true
    }

    /// Ends what belongs to the active pane alone before another pane
    /// becomes active: its search and its typed prefix.
    fn leave_pane(&self) {
        if self.is_searching() {
            self.change_model(|| self.end_search());
        }
        self.save_tab_view();
    }

    /// Remembers where the pane beside the active one was scrolled and
    /// what it had selected, from its folder pane.
    pub(super) fn save_beside_view(&self) {
        let pane = self.pane_on(self.imp().active_side.get().other());
        let scroll = pane.scroll_position();
        let selected = pane.model().selected_uris();
        let mut session = self.imp().session.borrow_mut();
        let Some(beside) = session.active_mut().and_then(Tab::beside_mut) else {
            return;
        };
        beside.scroll = scroll;
        // A selection still waiting for its listing stays.
        if beside.listing_state.is_listed() {
            beside.selected = selected;
        }
    }

    /// Shows the pane beside the active one in the other folder pane, or
    /// hides that pane when the tab in front is not split, and marks which
    /// pane is active. Called when a tab is shown.
    pub(super) fn show_beside_pane(&self) {
        let beside = {
            let session = self.imp().session.borrow();
            session.active().and_then(Tab::beside).map(|tab| BesideView {
                id: tab.id,
                store: tab.store.clone(),
                selected: tab.selected.clone(),
                scroll: tab.scroll,
                needs_listing: tab.listing_state.needs_listing(),
            })
        };
        self.show_split_layout(beside.is_some());
        let Some(beside) = beside else { return };
        let pane = self.pane_on(self.imp().active_side.get().other());
        self.change_model(|| {
            let model = pane.model();
            model.set_query("");
            model.set_store(Some(&beside.store));
            model.select_uris(&beside.selected);
        });
        pane.restore_scroll_position(beside.scroll);
        self.update_beside_pane();
        if beside.needs_listing {
            self.load_tab(beside.id, LoadMode::Navigate);
        }
    }

    /// Shows one folder pane, or both with their captions, the active
    /// one marked, and keeps `win.split-view` in step.
    fn show_split_layout(&self, is_split: bool) {
        let end = self.pane_column(PaneSide::End);
        let was_split = end.is_visible();
        end.set_visible(is_split);
        if is_split && !was_split {
            halve_once_laid_out(&self.imp().pane_split);
        }
        let active = self.imp().active_side.get();
        for side in [PaneSide::Start, PaneSide::End] {
            let column = self.pane_column(side);
            column.remove_css_class(ACTIVE_PANE);
            column.remove_css_class(INACTIVE_PANE);
            if is_split {
                column.add_css_class(if side == active {
                    ACTIVE_PANE
                } else {
                    INACTIVE_PANE
                });
            }
            self.pane_caption(side).set_visible(is_split);
        }
        self.show_split_state();
    }

    /// Keeps `win.split-view` checked while the tab in front is split.
    fn show_split_state(&self) {
        let is_split = self.imp().session.borrow().active().is_some_and(Tab::is_split);
        self.set_action_state(WindowAction::SplitView, &is_split.to_variant());
    }

    /// Draws the page of the pane beside the active one and both panes'
    /// captions, after its listing or its location changed.
    pub(super) fn update_beside_pane(&self) {
        let beside = {
            let session = self.imp().session.borrow();
            session
                .active()
                .and_then(Tab::beside)
                .map(|tab| (PaneState::of(tab), tab.uri().to_owned()))
        };
        let Some((state, uri)) = beside else { return };
        let side = self.imp().active_side.get().other();
        // The toast speaks for the active pane only.
        let listing = if ox_core::location::same_location(&uri, ox_core::location::TRASH_URI) {
            crate::folder_view::details::DetailsListing::RecycleBin
        } else {
            crate::folder_view::details::DetailsListing::Folder
        };
        self.pane_on(side).details().show_listing(listing);
        let _ = show_pane_state(self.pane_on(side), state, || None);
        self.apply_view_options();
        if Page::from_uri(&uri).is_some() {
            self.render_landing_in(side, &uri, &self.places());
        }
        self.show_pane_captions();
    }

    /// Writes where each pane of a split tab is above it.
    pub(super) fn show_pane_captions(&self) {
        let shown = self.shown_panes();
        if shown.len() < 2 {
            return;
        }
        let locations = self.imp().locations.borrow();
        for (side, uri) in shown {
            let caption = self.pane_caption(side);
            caption.set_label(&locations.title_for(&uri));
            caption.set_tooltip_text(Some(&locations.display_location(&uri)));
        }
    }

    /// The locations on screen: the active pane's, and the one beside it
    /// in a split tab, with the side of each.
    pub(super) fn shown_panes(&self) -> Vec<(PaneSide, String)> {
        let session = self.imp().session.borrow();
        let Some(active) = session.active() else {
            return Vec::new();
        };
        std::iter::once(active)
            .chain(active.beside())
            .map(|tab| (tab.side, tab.uri().to_owned()))
            .collect()
    }

    /// Redraws the page of pane `id` if it is on screen: the active pane
    /// with everything that follows it, the pane beside it on its own.
    pub(super) fn redraw_pane(&self, id: TabId) {
        let (is_active, is_beside) = {
            let session = self.imp().session.borrow();
            (session.is_active(id), session.beside_active() == Some(id))
        };
        if is_active {
            self.update_content();
        } else if is_beside {
            self.update_beside_pane();
        }
    }

    /// The pane beside the active one finished a listing: it selects what
    /// it had selected, scrolls where a restored session left it and shows
    /// its page.
    pub(super) fn finish_beside_listing(&self, id: TabId) {
        let (selected, scroll) = {
            let mut session = self.imp().session.borrow_mut();
            if session.beside_active() != Some(id) {
                return;
            }
            let Some(tab) = session.tab_mut(id) else { return };
            (tab.selected.clone(), tab.scroll_after_listing.take())
        };
        let pane = self.pane_on(self.imp().active_side.get().other());
        self.change_model(|| pane.model().select_uris(&selected));
        if let Some(scroll) = scroll {
            pane.restore_scroll_position(scroll);
        }
        self.update_beside_pane();
    }

    /// Opens `uris` paired into split tabs (`--split`, INT-005): the first
    /// pair in the active tab when `first_is_shown` says it shows the
    /// first location already, every other pair in a new tab. A location
    /// left without a partner is split with itself.
    pub(crate) fn open_split_tabs(&self, uris: &[String], first_is_shown: bool) {
        for (index, pair) in uris.chunks(2).enumerate() {
            let (left, right) = match pair {
                [left, right] => (left, right),
                [only] => (only, only),
                _ => continue,
            };
            let shown = if index == 0 && first_is_shown {
                Ok(())
            } else {
                self.add_tab(left)
            };
            if let Err(error) = shown.and_then(|()| self.split_tab(Some(right))) {
                self.show_message(&error.to_string());
            }
        }
    }

    /// The location of the pane beside the active one, for tests.
    #[cfg(test)]
    pub(crate) fn beside_uri(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        session.active()?.beside().map(|tab| tab.uri().to_owned())
    }

    /// Shows the active pane's own view and sort order in the View and
    /// Sort menus and the status bar: each pane of a split tab keeps its
    /// own, as in Dolphin.
    pub(super) fn show_pane_view_state(&self) {
        let view = self.folder_pane().view();
        self.status_bar().show_view(view);
        self.set_action_state(WindowAction::View, &view.as_str().to_variant());
        self.show_sort_state();
    }
}

/// Gives both panes of `split` the same width, as Dolphin opens them: now,
/// or once the split is first laid out.
fn halve_once_laid_out(split: &gtk::Paned) {
    let width = split.width();
    if width > 0 {
        split.set_position(width / 2);
        return;
    }
    let handler: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
    let id = split.connect_max_position_notify(glib::clone!(
        #[strong]
        handler,
        move |split| {
            let width = split.width();
            if width > 0 {
                split.set_position(width / 2);
                if let Some(id) = handler.take() {
                    split.disconnect(id);
                }
            }
        }
    ));
    handler.set(Some(id));
}

/// Whether `key` with `modifiers` is Tab alone, which may switch panes;
/// Shift+Tab, Ctrl+Tab and the others keep their own jobs.
pub(super) fn is_plain_tab(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let held = gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::SUPER_MASK;
    key == gdk::Key::Tab && !modifiers.intersects(held)
}
