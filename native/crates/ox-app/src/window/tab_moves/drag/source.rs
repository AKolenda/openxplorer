// SPDX-License-Identifier: AGPL-3.0-only
//! The dragging window's side of a tab drag: starting it, following what
//! became of the tab, and finishing the move once GTK is done.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::{tab_drag_content, DraggedTab, OutgoingTabDrag, TabDragOutcome, TabDropSpot};
use crate::window::session::TabId;
use crate::window::tab_moves::TabMoveRefusal;
use crate::window::BrowserWindow;

impl BrowserWindow {
    pub(super) fn attach_tab_drag_source(&self) {
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            None,
            move |_, x, y| window.prepare_tab_drag(x, y)
        ));
        source.connect_drag_begin(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, drag| window.begin_tab_drag(drag)
        ));
        source.connect_drag_cancel(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, _, reason| {
                window.cancel_tab_drag(reason);
                false
            }
        ));
        source.connect_drag_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| window.end_tab_drag()
        ));
        self.tab_strip().add_controller(source);
    }

    /// What dragging the tab at (`x`, `y`) of the strip offers; `None`
    /// on a close button, or while the window is busy, which the toast
    /// explains (TAB-031).
    pub(super) fn prepare_tab_drag(&self, x: f64, y: f64) -> Option<gdk::ContentProvider> {
        let tab = self.tab_strip().draggable_tab_at(x, y)?;
        if self.keeps_tab(tab) {
            self.show_message(&TabMoveRefusal::SourceBusy.to_string());
            return None;
        }
        self.imp().outgoing_tab.replace(Some(OutgoingTabDrag {
            tab,
            outcome: TabDragOutcome::Pending,
            is_refused_here: false,
        }));
        let dragged = DraggedTab {
            source: self.downgrade(),
            tab,
        };
        Some(tab_drag_content(&dragged))
    }

    /// The drag started: the tab fades and its likeness follows the
    /// pointer.
    fn begin_tab_drag(&self, drag: &gdk::Drag) {
        let Some(tab) = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .map(|outgoing| outgoing.tab)
        else {
            return;
        };
        self.tab_strip().show_dragged_tab(Some(tab));
        if let Some(icon) = self.tab_strip().drag_icon(tab) {
            gtk::DragIcon::for_drag(drag).set_child(Some(&icon));
        }
    }

    /// The drag was cancelled: released where no window took it, refused,
    /// or Escape. Only a release where nothing takes drops, with no
    /// window of this app refusing it last, tears the tab out; everything
    /// else keeps it and says how to detach it (`drag_failed`).
    pub(super) fn cancel_tab_drag(&self, reason: gdk::DragCancelReason) {
        let mut outgoing = self.imp().outgoing_tab.borrow_mut();
        let Some(outgoing) = outgoing.as_mut() else {
            return;
        };
        if outgoing.outcome != TabDragOutcome::Pending {
            return;
        }
        let is_released_outside = reason == gdk::DragCancelReason::NoTarget && !outgoing.is_refused_here;
        if is_released_outside {
            outgoing.outcome = TabDragOutcome::TornOut;
        } else {
            outgoing.outcome = TabDragOutcome::Cancelled;
            self.show_message(&TabMoveRefusal::Cancelled.to_string());
        }
    }

    /// The drag ended: the tab shows plainly again, and once GTK is done
    /// with the drag it leaves for a new window, or is removed here when
    /// another window took it. A drop no window of this app took, and
    /// that was not cancelled, is GNOME Shell's desktop drop: a tear-out.
    pub(super) fn end_tab_drag(&self) {
        self.tab_strip().show_dragged_tab(None);
        self.show_tab_drop_spot(None);
        let Some(outgoing) = self.imp().outgoing_tab.take() else {
            return;
        };
        let tab = outgoing.tab;
        let finish: fn(&BrowserWindow, TabId) = match outgoing.outcome {
            TabDragOutcome::Pending | TabDragOutcome::TornOut => BrowserWindow::move_tab_to_new_window,
            TabDragOutcome::MovedAway => BrowserWindow::release_moved_tab,
            TabDragOutcome::Reordered | TabDragOutcome::Cancelled => return,
        };
        // After the drag's own signal handlers, so a window that closes
        // with its last tab is not destroyed in the middle of them.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || finish(&window, tab)
        ));
    }

    /// Records whether the window under the pointer refuses this window's
    /// tab drag.
    pub(super) fn note_tab_refusal(&self, is_refused: bool) {
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.is_refused_here = is_refused;
        }
    }

    /// The source's side of a drop of its tab `tab` on `destination` at
    /// `spot`: reorders, hands the tab over or marks the tear-out; true
    /// when the tab is taken. A late or repeated drop of a drag that is
    /// over takes nothing.
    pub(super) fn settle_tab_drag(&self, tab: TabId, destination: &BrowserWindow, spot: TabDropSpot) -> bool {
        let is_this_drag = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .is_some_and(|outgoing| outgoing.tab == tab && outgoing.outcome == TabDragOutcome::Pending);
        if !is_this_drag {
            return false;
        }
        let outcome = match spot {
            TabDropSpot::TearOut => TabDragOutcome::TornOut,
            TabDropSpot::Strip { before } if destination == self => {
                self.reorder_tab(tab, before);
                TabDragOutcome::Reordered
            }
            TabDropSpot::Strip { before } => match self.hand_over_tab(tab, destination, before) {
                Ok(()) => TabDragOutcome::MovedAway,
                Err(refusal) => {
                    self.show_message(&refusal.to_string());
                    return false;
                }
            },
        };
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.outcome = outcome;
        }
        true
    }
}
