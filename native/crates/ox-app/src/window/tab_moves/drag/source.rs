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
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// Where the picture of a carried tab's window is held, from its top left
/// corner: on its title bar.
const CARRIED_HOLD_Y: i32 = 16;

/// How wide that picture is (`tab_strip::carried_window`).
const CARRIED_WIDTH: i32 = 320;

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
                self.bring_dragged_tab_to_front(tab);
            }
            Press::Live(tab) if self.is_off_the_strip(y) => {
                press.set(Press::Leaving(tab));
                self.carry_tab_out(tab);
            }
            Press::Live(tab) => self.tab_strip().follow_pointer(tab, x),
            Press::Leaving(tab) if !self.is_off_the_strip(y) => {
                press.set(Press::Live(tab));
                self.tab_strip().follow_pointer(tab, x);
            }
            Press::Idle | Press::Leaving(_) | Press::Ignored => {}
        }
    }

    /// The dragged tab `tab` comes to the front, as Chrome selects a tab
    /// it drags: its folder shows at once, and the strip, which keeps its
    /// tab widgets while the drag lasts (drawing them anew would end the
    /// press), marks it active until then.
    fn bring_dragged_tab_to_front(&self, tab: TabId) {
        if self.imp().session.borrow().active_id() == Some(tab) {
            return;
        }
        WindowAction::SelectTab.activate_from(self, Some(&tab.to_variant()));
        self.tab_strip().mark_active(tab);
    }

    /// Whether `y` of the strip is more than a tab's height above or
    /// below it.
    fn is_off_the_strip(&self, y: f64) -> bool {
        let height = f64::from(self.tab_strip().height());
        y < -height || y > 2.0 * height
    }

    /// Tab `tab` leaves its strip: the other tabs slide back while GTK's
    /// drag and drop carries a picture of the window, which shows the tab
    /// since it came to the front.
    fn carry_tab_out(&self, _tab: TabId) {
        self.tab_strip().let_go(
            false,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_| window.mark_carried_tab()
            ),
        );
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
        let Some(content) = self.child() else {
            return;
        };
        let strip = self.tab_strip();
        let (Some(picture), Some(carried_tab)) = (strip.drag_icon(tab, &content), strip.carried_tab(tab))
        else {
            return;
        };
        let icon = gtk::DragIcon::for_drag(drag);
        icon.set_child(Some(&picture));
        let along = self.tab_strip().grab_offset(tab).unwrap_or(0.0);
        #[expect(clippy::cast_possible_truncation, reason = "a point on a tab")]
        let hold_x = (along.round() as i32).clamp(CARRIED_HOLD_Y, CARRIED_WIDTH - CARRIED_HOLD_Y);
        drag.set_hotspot(hold_x, CARRIED_HOLD_Y);
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.carried = Some(Carried {
                icon,
                window: picture,
                tab: carried_tab,
            });
        }
    }

    /// Shows `look` of the carried tab under the pointer.
    pub(super) fn show_carried(&self, look: CarriedLook) {
        let outgoing = self.imp().outgoing_tab.borrow();
        let Some(carried) = outgoing.as_ref().and_then(|outgoing| outgoing.carried.as_ref()) else {
            return;
        };
        let shown = match look {
            CarriedLook::Window => &carried.window,
            CarriedLook::Tab | CarriedLook::Hidden => &carried.tab,
        };
        if carried.icon.child().as_ref() != Some(shown) {
            carried.icon.set_child(Some(shown));
        }
        shown.set_opacity(if look == CarriedLook::Hidden { 0.0 } else { 1.0 });
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
        self.tab_strip().show_dragged_tab(None);
        if self.tab_strip().is_following() {
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
