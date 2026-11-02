// SPDX-License-Identifier: AGPL-3.0-only
//! The dragging window's side of a tab drag: the press, which drags the
//! tab live along its strip and starts GTK's drag and drop once it leaves
//! the strip, following what became of the tab, and finishing the move
//! once GTK is done.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::window_drag::MousePress;
use super::{
    tab_drag_content, Carried, CarriedLook, DraggedTab, Hover, OutgoingTabDrag, TabDragOutcome, TabDropSpot,
};
use crate::window::session::TabId;
use crate::window::tab_moves::TabMoveRefusal;
use crate::window::tab_strip::CARRIED_MARGIN;
use crate::window::BrowserWindow;

/// How long a dragged tab stays in its strip, at most, waiting for the
/// tab GTK's drag and drop carries to be drawn.
const CARRIED_DRAWN_AT_LATEST: std::time::Duration = std::time::Duration::from_millis(250);

/// The carried tab's title, classes and drawn width, and where it is held,
/// from its top left corner; for tests.
#[cfg(test)]
pub(in crate::window) type CarriedTabLook = (String, Vec<String>, i32, (i32, i32));

/// How far the carried tab goes away, past the screen, while hidden.
const CARRIED_AWAY: i32 = 100_000;

/// How far above or below its strip the pointer may go before a dragged tab
/// leaves the strip: Chrome's `kVerticalDetachMagnetism`.
pub(super) const DETACH_MAGNETISM: f64 = 15.0;

/// What a primary press on the title bar is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Press {
    /// No press, or one the window handle has.
    Idle,
    /// Pressed here, in the title bar's coordinates, on this tab if one,
    /// not yet past the drag threshold.
    Pending((f64, f64), Option<TabId>),
    /// The tab follows the pointer along its strip.
    Live(TabId),
    /// The tab left its strip: GTK's drag and drop starts with the next
    /// motion it allows.
    Leaving(TabId),
    /// The tab left its strip as a window of its own, which the desktop
    /// now moves with the press.
    Handed,
    /// Escape, or a busy window: the rest of the press does nothing.
    Ignored,
}

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
        let press = Rc::new(Cell::new(Press::Idle));
        let pressed_with = Rc::new(Cell::new(None));
        self.follow_tab_presses(&source, &press, &pressed_with);
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            #[upgrade_or]
            None,
            move |_, _, _| {
                let Press::Leaving(tab) = press.get() else {
                    return None;
                };
                press.set(Press::Idle);
                let content = window.prepare_tab(tab);
                if content.is_none() {
                    // The window became busy after the tab began following
                    // the pointer, as a paste may make it: the tab goes back
                    // where it was, as no drag and drop will let it go.
                    window.let_go_of_tab(false);
                }
                content
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
        self.add_controller(escape_puts_tabs_back(self, &press));
    }

    /// Follows each primary press on the title bar through `source`: past
    /// the drag threshold on a tab it becomes the tab's live drag, which
    /// [`take_tab_presses_from_the_window_handle`] explains, and a release
    /// leaves the tab where it was shown. `pressed_with` is the press when
    /// it is one of the mouse, which may drag the tab out as a window.
    fn follow_tab_presses(
        &self,
        source: &gtk::DragSource,
        press: &Rc<Cell<Press>>,
        pressed_with: &Rc<Cell<Option<MousePress>>>,
    ) {
        source.connect_begin(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            #[strong]
            pressed_with,
            move |source, sequence| {
                pressed_with.set(MousePress::of(source));
                if let Some(point) = source.point(sequence) {
                    press.set(Press::Pending(point, window.grab_pressed_tab(source, point)));
                }
            }
        ));
        source.connect_update(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            #[strong]
            pressed_with,
            move |source, sequence| {
                if let Some(point) = source.point(sequence) {
                    window.move_tab_press(source, &press, pressed_with.get(), point);
                }
            }
        ));
        source.connect_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            move |_, _| match press.replace(Press::Idle) {
                Press::Live(_) => window.let_go_of_tab(true),
                // Released past the strip before GTK's drag and drop could
                // start: the tab tears out, as a release there does once it
                // has started.
                Press::Leaving(tab) => {
                    window.tab_strip().end_drag_now();
                    window.move_tab_to_new_window(tab);
                }
                _ => {}
            }
        ));
        source.connect_cancel(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            move |_, _| {
                if let Press::Live(_) | Press::Leaving(_) = press.replace(Press::Idle) {
                    window.let_go_of_tab(false);
                }
            }
        ));
    }

    /// The pointer of a primary press, `pressed_with` when it is the
    /// mouse's, moved to `point` of the title bar.
    fn move_tab_press(
        &self,
        source: &gtk::DragSource,
        press: &Cell<Press>,
        pressed_with: Option<MousePress>,
        point: (f64, f64),
    ) {
        let Some((x, y)) = self.title_bar_point_in_strip(source, point.0, point.1) else {
            return;
        };
        match press.get() {
            Press::Pending(start, pressed) => {
                let Some(tab) = take_tab_presses_from_the_window_handle(self, source, pressed, start, point)
                else {
                    return;
                };
                if self.keeps_tab(tab) {
                    self.show_message(&TabMoveRefusal::SourceBusy.to_string());
                    press.set(Press::Ignored);
                    return;
                }
                // A tab on its way back after Escape is not dragged again.
                if self.window_drag_holds(tab) {
                    self.show_message(ox_core::i18n::gettext_static(super::TAB_IS_MOVING));
                    press.set(Press::Ignored);
                    return;
                }
                press.set(Press::Live(tab));
                self.tab_strip().follow_pointer(tab, x);
                // GDK merges motions that wait together, so a quick flick's
                // first may already be off the strip.
                if self.is_off_the_strip(y) {
                    self.leave_strip_with_press(source, press, pressed_with, tab);
                }
            }
            Press::Live(tab) if self.is_off_the_strip(y) => {
                self.leave_strip_with_press(source, press, pressed_with, tab);
            }
            Press::Live(tab) => self.tab_strip().follow_pointer(tab, x),
            Press::Leaving(tab) if !self.is_off_the_strip(y) => {
                press.set(Press::Live(tab));
                self.tab_strip().follow_pointer(tab, x);
            }
            Press::Idle | Press::Leaving(_) | Press::Handed | Press::Ignored => {}
        }
    }

    /// The tab a press at `point` of the title bar would drag, held where
    /// the press met it as drawn. It is the tab the press drags, also when
    /// another slides under the point before the press moves, as the
    /// others do while a tab let go a moment ago slides into its place.
    fn grab_pressed_tab(&self, source: &gtk::DragSource, point: (f64, f64)) -> Option<TabId> {
        let (x, y) = self.title_bar_point_in_strip(source, point.0, point.1)?;
        let tab = self.tab_strip().draggable_tab_at(x, y)?;
        self.tab_strip().grab_tab(tab, (x, y));
        Some(tab)
    }

    /// Tab `tab`, following the pointer of a press (`pressed_with` when
    /// the mouse's), left its strip: its window is dragged out with it, the
    /// window itself moves for a window's only tab, or GTK's drag and drop
    /// carries it.
    fn leave_strip_with_press(
        &self,
        source: &gtk::DragSource,
        press: &Cell<Press>,
        pressed_with: Option<MousePress>,
        tab: TabId,
    ) {
        let grab = self.tab_strip().grab_offset(tab);
        if grab.is_some_and(|grab| self.drag_tab_out_as_window(tab, grab, pressed_with)) {
            // The desktop has the pointer now, and moves the window.
            press.set(Press::Handed);
        } else if self.tab_count() <= 1 && !self.has_other_windows() {
            press.set(Press::Handed);
            self.move_window_by_its_tab(source, tab);
        } else {
            // GTK's drag and drop starts with the next motion it allows,
            // and the tab leaves its strip once the tab it carries is on
            // screen.
            press.set(Press::Leaving(tab));
        }
    }

    /// Whether `y` of the strip is more than Chrome's 15 pixels
    /// (`kVerticalDetachMagnetism`) above or below it.
    fn is_off_the_strip(&self, y: f64) -> bool {
        let height = f64::from(self.tab_strip().height());
        y < -DETACH_MAGNETISM || y > height + DETACH_MAGNETISM
    }

    /// Whether another window of this app is open, which a tab dragged
    /// out of this one could be dropped on.
    fn has_other_windows(&self) -> bool {
        let windows = self.application().map(|app| app.windows()).unwrap_or_default();
        windows
            .into_iter()
            .filter_map(|window| window.downcast::<BrowserWindow>().ok())
            .any(|window| &window != self && window.is_visible())
    }

    /// The only tab of this window, `tab`, leaves its strip while no other
    /// window of this app is open to drop it on: the window itself follows
    /// the pointer, held by its tab, as Chrome moves a window by its only
    /// tab. The desktop moves it with the press itself
    /// (`gdk_toplevel_begin_move` with the press's button and time; on
    /// Wayland its serial), and the release leaves it there.
    ///
    /// A tab of a window with others cannot be held this way: GTK cannot
    /// place a new window under the pointer, nor move one with a drag
    /// (it has no `xdg-toplevel-drag-v1`), and a window the desktop moves
    /// first goes wherever the desktop places it.
    fn move_window_by_its_tab(&self, source: &gtk::DragSource, tab: TabId) {
        let hold = self.tab_strip().grab_offset(tab).map_or(0.0, |(x, _)| x);
        self.tab_strip().end_drag_now();
        let Some(device) = source.device() else {
            return;
        };
        let (button, time) = (source.current_button(), source.current_event_time());
        hand_press_to_window_move(self, &device, button, time, hold);
    }

    /// Tab `tab` leaves its strip once `carried`, the tab GTK's drag and
    /// drop carries, is on screen, so the tab is never nowhere: a new drag
    /// surface can take a while to draw its first frame. It vanishes from
    /// the strip, which closes up, as Chrome takes a dragged-out tab off its
    /// strip, unless the pointer came back over the strip by then.
    fn leave_strip_when_carried(&self, tab: TabId, carried: &gtk::Widget) {
        let left = Rc::new(Cell::new(false));
        let leave = Rc::new(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            left,
            move || {
                if !left.replace(true) {
                    window.leave_strip(tab);
                }
            }
        ));
        when_drawn(carried, leave.clone());
        // Should the drag surface never report its first frame.
        glib::timeout_add_local_once(CARRIED_DRAWN_AT_LATEST, move || leave());
    }

    /// Tab `tab`, carried by this window's drag and drop, leaves its strip,
    /// unless the pointer is over the strip.
    fn leave_strip(&self, tab: TabId) {
        let is_away = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .is_some_and(|outgoing| outgoing.tab == tab && !outgoing.over_own_strip);
        if is_away {
            self.take_tab_off_strip(tab);
        }
    }

    /// Tab `tab`, carried by this window's drag and drop, leaves its strip:
    /// it vanishes there, the others close up, and when it was in front the
    /// tab beside it comes to the front, the one closing it would show, as
    /// Chrome shows another tab while one is dragged out of a window.
    pub(super) fn take_tab_off_strip(&self, tab: TabId) {
        self.tab_strip().detach_tab(tab);
        let successor = {
            let session = self.imp().session.borrow();
            session.is_active(tab).then(|| session.successor(tab)).flatten()
        };
        let Some(successor) = successor else {
            return;
        };
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.stepped_back = true;
        }
        self.switch_tab(successor);
    }

    /// Tab `tab`, carried out of this window's strip, is back over it or
    /// stays: it comes to the front again when it was there.
    pub(super) fn bring_carried_tab_forward(&self, tab: TabId) {
        let stepped_back = self
            .imp()
            .outgoing_tab
            .borrow_mut()
            .as_mut()
            .filter(|outgoing| outgoing.tab == tab)
            .is_some_and(|outgoing| std::mem::take(&mut outgoing.stepped_back));
        if stepped_back {
            self.switch_tab(tab);
        }
    }

    /// Records whether the pointer of this window's drag and drop is over
    /// its own strip.
    pub(super) fn note_over_own_strip(&self, is_over: bool) {
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.over_own_strip = is_over;
        }
    }

    /// The tab following the pointer is let go: with `keep_order`, it
    /// stays where the strip shows it; otherwise every tab goes back.
    pub(super) fn let_go_of_tab(&self, keep_order: bool) {
        self.tab_strip().let_go(
            keep_order,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |moved| {
                    if let Some((tab, before)) = moved {
                        window.reorder_tab(tab, before);
                    }
                }
            ),
        );
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
    /// explains (TAB-031). Tests start drags here.
    #[cfg(test)]
    pub(super) fn prepare_tab_drag(&self, x: f64, y: f64) -> Option<gdk::ContentProvider> {
        let tab = self.tab_strip().draggable_tab_at(x, y)?;
        self.prepare_tab(tab)
    }

    /// What dragging tab `tab` offers; `None` while the window is busy.
    fn prepare_tab(&self, tab: TabId) -> Option<gdk::ContentProvider> {
        if self.keeps_tab(tab) {
            self.show_message(&TabMoveRefusal::SourceBusy.to_string());
            return None;
        }
        self.imp().outgoing_tab.replace(Some(OutgoingTabDrag {
            tab,
            outcome: TabDragOutcome::Pending,
            // Outside every window of this app until one says the drag is
            // over it: the drag may start with the pointer already outside,
            // where no window ever reports it.
            over: None,
            carried: None,
            over_own_strip: false,
            stepped_back: false,
        }));
        let dragged = DraggedTab {
            source: self.downgrade(),
            tab,
        };
        Some(tab_drag_content(&dragged))
    }

    /// The drag and drop started: the tab itself follows the pointer,
    /// held where it was grabbed.
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
        let Some(carried) = self.tab_strip().carried_tab(tab) else {
            return;
        };
        gtk::DragIcon::for_drag(drag).set_child(Some(&carried));
        self.leave_strip_when_carried(tab, &carried);
        let (along, down) = self.tab_strip().grab_offset(tab).unwrap_or((0.0, 0.0));
        #[expect(clippy::cast_possible_truncation, reason = "a point on a tab")]
        let hold = (
            CARRIED_MARGIN + along.round() as i32,
            CARRIED_MARGIN + down.round() as i32,
        );
        drag.set_hotspot(hold.0, hold.1);
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.carried = Some(Carried {
                drag: drag.clone(),
                #[cfg(test)]
                tab: carried,
                hold,
                at: Cell::new(hold),
            });
        }
    }

    /// Shows the carried tab under the pointer, or hides it while the tab
    /// follows the pointer along its own strip again, by the drag's hotspot
    /// alone: `KWin` shows a drag icon's first look but no later redraw,
    /// while it does follow a moved hotspot.
    pub(super) fn show_carried(&self, look: CarriedLook) {
        let outgoing = self.imp().outgoing_tab.borrow();
        let Some(carried) = outgoing.as_ref().and_then(|outgoing| outgoing.carried.as_ref()) else {
            return;
        };
        let (x, y) = carried.hold;
        let at = match look {
            CarriedLook::Shown => (x, y),
            CarriedLook::Hidden => (x, y + CARRIED_AWAY),
        };
        if carried.at.replace(at) != at {
            carried.drag.set_hotspot(at.0, at.1);
        }
    }

    /// The carried tab's title, classes and drawn width, and where it is
    /// held, from its top left corner; for tests.
    #[cfg(test)]
    pub(in crate::window) fn carried_look_for_test(&self) -> Option<CarriedTabLook> {
        let outgoing = self.imp().outgoing_tab.borrow();
        let carried = outgoing.as_ref()?.carried.as_ref()?;
        let title = crate::window::widget_tree::descendants::<gtk::Label>(&carried.tab)
            .into_iter()
            .find(|label| label.has_css_class("tab-title"))?
            .text()
            .to_string();
        let classes = carried
            .tab
            .css_classes()
            .iter()
            .map(ToString::to_string)
            .collect();
        let bounds = carried.tab.compute_bounds(&carried.tab)?;
        #[expect(clippy::cast_possible_truncation, reason = "a tab's width")]
        let width = bounds.width().round() as i32;
        let (x, y) = carried.at.get();
        Some((title, classes, width, (x - CARRIED_MARGIN, y - CARRIED_MARGIN)))
    }

    /// The carried tab, as GTK's drag icon shows it; for tests.
    #[cfg(test)]
    pub(in crate::window) fn carried_tab_for_test(&self) -> Option<gtk::Widget> {
        let outgoing = self.imp().outgoing_tab.borrow();
        Some(outgoing.as_ref()?.carried.as_ref()?.tab.clone())
    }

    /// The drag was cancelled: released where no window took it, refused,
    /// or Escape. A release outside every window of this app tears the tab
    /// out, as Chrome does; everything else keeps it. X11 tells Escape
    /// apart, but Wayland reports a release outside every window and Escape
    /// alike, so there Escape keeps the tab only over a window of this
    /// app; a busy window that refused it says so.
    pub(super) fn cancel_tab_drag(&self, reason: gdk::DragCancelReason) {
        let mut outgoing = self.imp().outgoing_tab.borrow_mut();
        let Some(outgoing) = outgoing.as_mut() else {
            return;
        };
        if outgoing.outcome != TabDragOutcome::Pending {
            return;
        }
        let is_released_outside = reason != gdk::DragCancelReason::UserCancelled && outgoing.over.is_none();
        if is_released_outside {
            outgoing.outcome = TabDragOutcome::TornOut;
            return;
        }
        outgoing.outcome = TabDragOutcome::Cancelled;
        if outgoing.over == Some(Hover::Refuses) {
            self.show_message(&TabMoveRefusal::DestinationBusy.to_string());
        }
    }

    /// The drag ended: the tab shows plainly again, and once GTK is done
    /// with the drag it leaves for a new window, or is removed here when
    /// another window took it. A drop no window of this app took, and
    /// that was not cancelled, is GNOME Shell's desktop drop: a tear-out.
    pub(super) fn end_tab_drag(&self) {
        let is_reordered = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .is_some_and(|outgoing| outgoing.outcome == TabDragOutcome::Reordered);
        self.tab_strip().show_dragged_tab(None);
        // A tab that was not dropped in its strip goes back to its place
        // there; one that leaves goes once the strip is still.
        if self.tab_strip().live_tab().is_some() && !is_reordered {
            self.let_go_of_tab(false);
        }
        let Some(outgoing) = self.imp().outgoing_tab.take() else {
            return;
        };
        let tab = outgoing.tab;
        let finish: fn(&BrowserWindow, TabId) = match outgoing.outcome {
            TabDragOutcome::Pending | TabDragOutcome::TornOut => BrowserWindow::move_tab_to_new_window,
            TabDragOutcome::MovedAway => BrowserWindow::release_moved_tab,
            TabDragOutcome::Reordered | TabDragOutcome::Cancelled => {
                // The tab stays, in front again when it was.
                if outgoing.stepped_back {
                    self.switch_tab(tab);
                }
                return;
            }
        };
        // After the drag's own signal handlers, so a window that closes
        // with its last tab is not destroyed in the middle of them.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || finish(&window, tab)
        ));
    }

    /// Records the window of this app under the pointer: whether it takes
    /// this window's tab, or `None` outside every one.
    pub(super) fn note_tab_hover(&self, over: Option<bool>) {
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.over = over.map(|takes| if takes { Hover::Takes } else { Hover::Refuses });
        }
    }

    /// Tab `tab`, carried back to this window's strip, is dropped where the
    /// strip made room for it; true when it is this window's drag.
    pub(super) fn settle_live_drop(&self, tab: TabId) -> bool {
        let is_this_drag = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .is_some_and(|outgoing| outgoing.tab == tab && outgoing.outcome == TabDragOutcome::Pending);
        if !is_this_drag {
            return false;
        }
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.outcome = TabDragOutcome::Reordered;
        }
        self.let_go_of_tab(true);
        true
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
                // Dropped where the strip did not follow it, which then
                // lays its tabs out as the session has them again.
                self.tab_strip().end_drag_now();
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

/// Claims for `source` a press that started on a tab once the pointer has
/// moved from `start` past the drag threshold to `point`, so the title
/// bar's window handle never moves the window for it; the tab, when it
/// did.
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
/// maximizes the window. The tab then follows the pointer along the strip,
/// and `prepare` offers it to GTK's drag and drop only once it left the
/// strip, so GTK starts that drag in the middle of the press, with the
/// press's own grab on Wayland.
///
/// `pressed` is the tab the press, at `start`, began on, if it is still
/// open; `point` is where the pointer is now.
fn take_tab_presses_from_the_window_handle(
    window: &BrowserWindow,
    source: &gtk::DragSource,
    pressed: Option<TabId>,
    start: (f64, f64),
    point: (f64, f64),
) -> Option<TabId> {
    if !is_past_drag_threshold(window, start, point) {
        return None;
    }
    let tab = pressed.filter(|tab| window.tab_strip().tab_view(*tab).is_some());
    if tab.is_some() {
        source.set_state(gtk::EventSequenceState::Claimed);
    }
    tab
}

/// Escape while a tab follows the pointer puts every tab back where it was
/// (Chrome and Explorer do the same); the rest of the press does nothing.
fn escape_puts_tabs_back(window: &BrowserWindow, press: &Rc<Cell<Press>>) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        window,
        #[strong]
        press,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            let is_live = matches!(press.get(), Press::Live(_) | Press::Leaving(_));
            if key != gdk::Key::Escape || !is_live {
                return glib::Propagation::Proceed;
            }
            press.set(Press::Ignored);
            window.let_go_of_tab(false);
            glib::Propagation::Stop
        }
    ));
    keys
}

/// Whether `point` is past the desktop's drag threshold from `start`, as
/// GTK's own drag gestures judge it (`gtk_drag_check_threshold_double`).
fn is_past_drag_threshold(widget: &impl IsA<gtk::Widget>, start: (f64, f64), point: (f64, f64)) -> bool {
    let threshold = f64::from(widget.settings().gtk_dnd_drag_threshold());
    (point.0 - start.0).abs() > threshold || (point.1 - start.1).abs() > threshold
}

/// The desktop moves `window` with the press of `button` on `device` at
/// `time`, held where its tab strip would show a tab grabbed `hold`
/// pixels along it.
/// Runs `then` once `widget` is drawn: after the first frame its window
/// paints while it is mapped. A widget may be mapped already, as GTK's
/// drag icon maps the child it is given at once.
pub(super) fn when_drawn(widget: &gtk::Widget, then: Rc<dyn Fn()>) {
    let after_next_frame = move |widget: &gtk::Widget| {
        let Some(clock) = widget.frame_clock() else {
            return;
        };
        let handler = Rc::new(Cell::new(None));
        let id = clock.connect_after_paint(glib::clone!(
            #[strong]
            handler,
            #[strong]
            then,
            move |clock| {
                if let Some(id) = handler.take() {
                    clock.disconnect(id);
                }
                then();
            }
        ));
        handler.set(Some(id));
        // A frame comes even if nothing else changes.
        widget.queue_draw();
    };
    if widget.is_mapped() {
        after_next_frame(widget);
        return;
    }
    let handler = Rc::new(Cell::new(None));
    let id = widget.connect_map(glib::clone!(
        #[strong]
        handler,
        move |widget| {
            if let Some(id) = handler.take() {
                widget.disconnect(id);
            }
            after_next_frame(widget);
        }
    ));
    handler.set(Some(id));
}

fn hand_press_to_window_move(
    window: &BrowserWindow,
    device: &gdk::Device,
    button: u32,
    time: u32,
    hold: f64,
) {
    let Some(toplevel) = window.surface().and_downcast::<gdk::Toplevel>() else {
        return;
    };
    let strip = window.tab_strip();
    let in_window = strip
        .compute_point(window, &gtk::graphene::Point::new(0.0, 0.0))
        .unwrap_or_else(|| gtk::graphene::Point::new(0.0, 0.0));
    let (shadow_x, shadow_y) = window.surface_transform();
    let x = f64::from(in_window.x()) + hold + shadow_x;
    let y = f64::from(in_window.y()) + f64::from(strip.height()) / 2.0 + shadow_y;
    #[expect(clippy::cast_possible_wrap, reason = "a pointer button number")]
    toplevel.begin_move(device, button as i32, x, y, time);
}
