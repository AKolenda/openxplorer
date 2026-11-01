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

use super::{
    tab_drag_content, Carried, CarriedLook, DraggedTab, Hover, OutgoingTabDrag, TabDragOutcome, TabDropSpot,
};
use crate::window::session::TabId;
use crate::window::tab_moves::TabMoveRefusal;
use crate::window::BrowserWindow;

/// Where the picture of a carried tab's window is held, from its top left
/// corner: on its title bar.
const CARRIED_HOLD_Y: i32 = 16;

/// How far below the pointer the carried picture hangs over another
/// window's strip.
const CARRIED_BELOW: i32 = 48;

/// How far the carried picture goes away, past the screen, while hidden.
const CARRIED_AWAY: i32 = 100_000;

/// How far above or below its strip the pointer may go before a dragged tab
/// leaves the strip: Chrome's `kVerticalDetachMagnetism`.
const DETACH_MAGNETISM: f64 = 15.0;

/// What a primary press on the title bar is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Press {
    /// No press, or one the window handle has.
    Idle,
    /// Pressed here, in the title bar's coordinates, not yet past the drag
    /// threshold.
    Pending((f64, f64)),
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
        self.follow_tab_presses(&source, &press);
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
                window.prepare_tab(tab)
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
    /// leaves the tab where it was shown.
    fn follow_tab_presses(&self, source: &gtk::DragSource, press: &Rc<Cell<Press>>) {
        source.connect_begin(glib::clone!(
            #[strong]
            press,
            move |source, sequence| {
                if let Some(point) = source.point(sequence) {
                    press.set(Press::Pending(point));
                }
            }
        ));
        source.connect_update(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            move |source, sequence| {
                if let Some(point) = source.point(sequence) {
                    window.move_tab_press(source, &press, point);
                }
            }
        ));
        source.connect_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            move |_, _| {
                if let Press::Live(_) = press.replace(Press::Idle) {
                    window.let_go_of_tab(true);
                }
            }
        ));
        source.connect_cancel(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            press,
            move |_, _| {
                if let Press::Live(_) = press.replace(Press::Idle) {
                    window.let_go_of_tab(false);
                }
            }
        ));
    }

    /// The pointer of a primary press moved to `point` of the title bar.
    fn move_tab_press(&self, source: &gtk::DragSource, press: &Cell<Press>, point: (f64, f64)) {
        let Some((x, y)) = self.title_bar_point_in_strip(source, point.0, point.1) else {
            return;
        };
        match press.get() {
            Press::Pending(start) => {
                let Some(tab) = take_tab_presses_from_the_window_handle(self, source, start, point) else {
                    return;
                };
                if self.keeps_tab(tab) {
                    self.show_message(&TabMoveRefusal::SourceBusy.to_string());
                    press.set(Press::Ignored);
                    return;
                }
                if let Some((start_x, _)) = self.title_bar_point_in_strip(source, start.0, start.1) {
                    self.tab_strip().grab_tab(tab, start_x);
                }
                press.set(Press::Live(tab));
                self.tab_strip().follow_pointer(tab, x);
            }
            Press::Live(tab) if self.is_off_the_strip(y) => {
                if self.has_other_windows() {
                    press.set(Press::Leaving(tab));
                    self.carry_tab_out(tab);
                } else {
                    press.set(Press::Handed);
                    self.hand_tab_to_a_window(source, tab);
                }
            }
            Press::Live(tab) => self.tab_strip().follow_pointer(tab, x),
            Press::Leaving(tab) if !self.is_off_the_strip(y) => {
                press.set(Press::Live(tab));
                self.tab_strip().follow_pointer(tab, x);
            }
            Press::Idle | Press::Leaving(_) | Press::Handed | Press::Ignored => {}
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

    /// Tab `tab` leaves its strip with no other window of this app to drop
    /// it on, so a real window follows the pointer, as in Chrome: this
    /// window when it is the only tab, else a new window with the tab, as
    /// large as this one. The desktop moves it with the press itself
    /// (`gdk_toplevel_begin_move` with the press's button and time; on
    /// Wayland the press's serial), and the release leaves it there. A new
    /// window opens where the desktop places it and moves from there.
    fn hand_tab_to_a_window(&self, source: &gtk::DragSource, tab: TabId) {
        let Some(device) = source.device() else {
            return;
        };
        let (button, time) = (source.current_button(), source.current_event_time());
        let hold = self.tab_strip().grab_offset(tab).unwrap_or(0.0);
        self.tab_strip().end_drag_now();
        if self.tab_count() <= 1 {
            hand_press_to_window_move(self, &device, button, time, hold);
            return;
        }
        let window = match self.detach_tab(tab) {
            Ok(window) => window,
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                return;
            }
        };
        window.set_default_size(self.width(), self.height());
        let handler = Rc::new(Cell::new(None));
        let id = window.connect_map(glib::clone!(
            #[strong]
            handler,
            #[strong]
            device,
            move |window| {
                if let Some(id) = handler.take() {
                    window.disconnect(id);
                }
                // Once the desktop shows it.
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    window,
                    #[strong]
                    device,
                    move || hand_press_to_window_move(&window, &device, button, time, hold)
                ));
            }
        ));
        handler.set(Some(id));
        window.present_as_torn_out();
        self.release_moved_tab(tab);
    }

    /// Tab `tab` leaves its strip while another window of this app is
    /// open: it vanishes from the strip, which closes up, as Chrome takes a
    /// dragged-out tab off its strip, and GTK's drag and drop carries a
    /// picture of this window, which shows the tab, so it can be dropped on
    /// another window's strip.
    fn carry_tab_out(&self, tab: TabId) {
        self.tab_strip().detach_tab(tab);
    }

    /// Fades the tab drag and drop carries, if it still does.
    fn mark_carried_tab(&self) {
        let carried = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .map(|outgoing| outgoing.tab);
        if carried.is_some() && !self.tab_strip().is_following() {
            self.tab_strip().show_dragged_tab(carried);
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
                    window.mark_carried_tab();
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
            over: Some(Hover::Takes),
            carried: None,
        }));
        let dragged = DraggedTab {
            source: self.downgrade(),
            tab,
        };
        Some(tab_drag_content(&dragged))
    }

    /// The drag and drop started: the tab fades and a small live picture
    /// of its window follows the pointer, held by its title bar where the
    /// tab was grabbed.
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
        self.mark_carried_tab();
        let Some(picture) = self.tab_strip().drag_icon(tab, self.upcast_ref()) else {
            return;
        };
        gtk::DragIcon::for_drag(drag).set_child(Some(&picture));
        let along = self.tab_strip().grab_offset(tab).unwrap_or(0.0);
        #[expect(clippy::cast_possible_truncation, reason = "a point on a tab")]
        let hold_x = (along.round() as i32).clamp(CARRIED_HOLD_Y, picture.width_request() - CARRIED_HOLD_Y);
        drag.set_hotspot(hold_x, CARRIED_HOLD_Y);
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.carried = Some(Carried {
                drag: drag.clone(),
                hold: (hold_x, CARRIED_HOLD_Y),
                height: picture.height_request(),
            });
        }
    }

    /// Puts the carried picture at `look`, by its hotspot alone.
    pub(super) fn show_carried(&self, look: CarriedLook) {
        let outgoing = self.imp().outgoing_tab.borrow();
        let Some(carried) = outgoing.as_ref().and_then(|outgoing| outgoing.carried.as_ref()) else {
            return;
        };
        let (x, y) = carried.hold;
        let (x, y) = match look {
            CarriedLook::Window => (x, y),
            CarriedLook::Below => (x, -CARRIED_BELOW),
            CarriedLook::Hidden => (x, carried.height + CARRIED_AWAY),
        };
        carried.drag.set_hotspot(x, y);
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
fn take_tab_presses_from_the_window_handle(
    window: &BrowserWindow,
    source: &gtk::DragSource,
    start: (f64, f64),
    point: (f64, f64),
) -> Option<TabId> {
    if !is_past_drag_threshold(window, start, point) {
        return None;
    }
    let tab = window
        .title_bar_point_in_strip(source, start.0, start.1)
        .and_then(|(x, y)| window.tab_strip().draggable_tab_at(x, y));
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
            let is_live = matches!(press.get(), Press::Live(_));
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
