// SPDX-License-Identifier: AGPL-3.0-only
//! The dragging window's side of a tab drag: starting it, following what
//! became of the tab, and finishing the move once GTK is done.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::{tab_drag_content, DraggedTab, OutgoingTabDrag, TabDragOutcome, TabDropSpot};
use crate::window::session::TabId;
use crate::window::tab_moves::TabMoveRefusal;
use crate::window::BrowserWindow;

impl BrowserWindow {
    /// Makes the title bar the drag source of its tabs; see
    /// [`take_tab_presses_from_the_window_handle`] for why it is the
    /// title bar's window handle and not the tab strip.
    pub(super) fn attach_tab_drag_source(&self) {
        let Some(handle) = self.titlebar().and_downcast::<gtk::WindowHandle>() else {
            return;
        };
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        source.set_propagation_phase(gtk::PropagationPhase::Capture);
        take_tab_presses_from_the_window_handle(self, &source);
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            None,
            move |source, x, y| {
                let (x, y) = window.title_bar_point_in_strip(source, x, y)?;
                window.prepare_tab_drag(x, y)
            }
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
        handle.add_controller(source);
    }

    /// (`x`, `y`) of the drag source `source`'s widget, the title bar, in
    /// the tab strip's coordinates.
    fn title_bar_point_in_strip(&self, source: &gtk::DragSource, x: f64, y: f64) -> Option<(f64, f64)> {
        let bar = source.widget()?;
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = gtk::graphene::Point::new(x as f32, y as f32);
        let in_strip = bar.compute_point(self.tab_strip(), &point)?;
        Some((f64::from(in_strip.x()), f64::from(in_strip.y())))
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

/// Makes `source` take a press on a tab for itself once the pointer has
/// moved past the drag threshold, so the title bar's window handle never
/// moves the window for it.
///
/// The title bar is a `GtkWindowHandle` with the tab strip inside it. Its
/// drag gesture moves the window (`gdk_toplevel_begin_move`) as soon as a
/// press it sees passes the drag threshold, and GTK lets it see every
/// press no other gesture has claimed. `GtkDragSource` never claims one:
/// it starts no drag in the first 100 milliseconds after a press, so a
/// brisk drag reaches the handle first, and when it does start a drag it
/// resets its widget's gestures (`gtk_widget_reset_controllers`), so the
/// motion that started it goes on to the handle, which asks to move the
/// window as well. Either way the window moved instead of the tab, on
/// X11 and on Wayland alike.
///
/// So the drag source sits on the handle itself, in the capture phase,
/// where it sees each event before the handle's own gestures, and claims a
/// press that started on a tab when the pointer passes the threshold GTK
/// uses (`gtk_drag_check_threshold_double`). A gesture that claims denies
/// the press to the other gestures of its widget, the handle's among them,
/// and cancels it for the widgets inside, so the tab's own click does not
/// show a dragged tab. When the drag then starts, the reset clears the
/// handle's gestures too. A press that has not moved that far stays a
/// click, and a press beside the tabs stays the handle's, which drags or
/// maximizes the window.
fn take_tab_presses_from_the_window_handle(window: &BrowserWindow, source: &gtk::DragSource) {
    // Where the press was, until the source has claimed it. A mouse's
    // events have no sequence, so the press is followed here.
    let pending = Rc::new(Cell::new(None));
    source.connect_begin(glib::clone!(
        #[strong]
        pending,
        move |source, sequence| pending.set(source.point(sequence))
    ));
    source.connect_update(glib::clone!(
        #[weak]
        window,
        move |source, sequence| {
            let Some(start) = pending.get() else {
                return;
            };
            let Some(point) = source.point(sequence) else {
                return;
            };
            if !is_past_drag_threshold(&window, start, point) {
                return;
            }
            pending.set(None);
            let on_tab = window
                .title_bar_point_in_strip(source, start.0, start.1)
                .and_then(|(x, y)| window.tab_strip().draggable_tab_at(x, y));
            if on_tab.is_some() {
                source.set_state(gtk::EventSequenceState::Claimed);
            }
        }
    ));
}

/// Whether `point` is past the desktop's drag threshold from `start`, as
/// GTK's own drag gestures judge it (`gtk_drag_check_threshold_double`).
fn is_past_drag_threshold(widget: &impl IsA<gtk::Widget>, start: (f64, f64), point: (f64, f64)) -> bool {
    let threshold = f64::from(widget.settings().gtk_dnd_drag_threshold());
    (point.0 - start.0).abs() > threshold || (point.1 - start.1).abs() > threshold
}
