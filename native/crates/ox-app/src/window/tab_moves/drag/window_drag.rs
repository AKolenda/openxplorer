// SPDX-License-Identifier: AGPL-3.0-only
//! A tab dragged out of its row as its own window, which the desktop moves
//! with the pointer, as Chrome drags a torn-out tab on Wayland (TAB-033,
//! TAB-034): xdg-toplevel-drag-v1, in the order of Chromium's
//! `WaylandWindowDragController`.
//!
//! When the tab leaves its row, a drag and drop starts with a data source
//! that offers only [`WINDOW_DRAG_MIME`], with no drag icon, and the window
//! the tab now has is attached to the drag before it is first drawn, held
//! where the tab was grabbed. The desktop then moves that window with the
//! pointer, and leaves it where the pointer is released. Every window of
//! this app takes drops of that format: over a tab row the dragged window
//! hides and a stand-in for its tab follows the pointer there, as Chrome
//! takes the tab into that row, and a release there moves the tab into that
//! window, whose own window goes away. Escape puts the tab back where it
//! was.
//!
//! GTK has no API for any of this, so its objects are made on GTK's own
//! Wayland connection, in a queue of their own; the window's `xdg_toplevel`
//! comes from [`crate::gtk_xdg_toplevel`], the one place that reads GTK's
//! private structures. Where any of it is missing (X11, a GTK release that
//! was not checked, a desktop without the protocol) the tab is carried as
//! before.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gdk_wayland::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_pointer, wl_registry, wl_seat,
};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::xdg::toplevel_drag::v1::client::{xdg_toplevel_drag_manager_v1, xdg_toplevel_drag_v1};

use crate::window::session::TabId;
use crate::window::tab_strip::Landing;
use crate::window::BrowserWindow;

/// The only format a window drag offers, as Chrome offers
/// `chromium/x-window`: it carries nothing, and only this process's windows
/// take it.
const WINDOW_DRAG_MIME: &str = "application/x-openxplorer-window-drag";

/// The primary button, as Linux input numbers it (`BTN_LEFT`).
const PRIMARY_BUTTON: u32 = 0x110;

/// How often the drag's own events are handled while it lasts.
const PUMP: Duration = Duration::from_millis(8);

/// How far below a row that took the tab the pointer goes before the tab
/// leaves it: Chrome's `kVerticalDetachMagnetism`.
#[expect(clippy::cast_possible_truncation, reason = "a few pixels")]
const MAGNETISM: f32 = super::source::DETACH_MAGNETISM as f32;

/// How long a dragged-out tab stays in its row, at most, waiting for its
/// new window to be drawn.
const WINDOW_DRAWN_AT_LATEST: Duration = Duration::from_millis(800);

/// What the desktop told this process's own Wayland objects.
#[derive(Debug, Default)]
struct Events {
    /// The serial of the last primary press on a window of this app, which
    /// a drag must start with.
    press: Option<u32>,
    /// What happened to the drag's data source, oldest first.
    source: Vec<SourceEvent>,
}

/// What happened to a drag's data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceEvent {
    /// The pointer was released (`dnd_drop_performed`).
    Dropped,
    /// The window it was released over took it (`dnd_finished`).
    Finished,
    /// Nothing took it, or Escape ended the drag (`cancelled`).
    Cancelled,
}

/// The Wayland objects window drags need, on GTK's connection.
struct Protocol {
    /// GTK's connection.
    connection: Connection,
    /// The queue these objects' events wait in until [`Self::pump`].
    queue: RefCell<EventQueue<Events>>,
    /// What they said so far.
    events: RefCell<Events>,
    data_devices: wl_data_device_manager::WlDataDeviceManager,
    toplevel_drags: xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1,
    seat: wl_seat::WlSeat,
    /// This process's own pointer, which sees the serial of each press.
    _pointer: wl_pointer::WlPointer,
}

thread_local! {
    /// The protocol objects, once a window of this app is shown on a
    /// desktop that has them.
    static PROTOCOL: RefCell<Option<Rc<Protocol>>> = const { RefCell::new(None) };
    /// Whether making them was tried, so a desktop without them is asked
    /// once.
    static TRIED: Cell<bool> = const { Cell::new(false) };
    /// The window drag in progress.
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

impl Protocol {
    /// The protocol objects of `display`, made the first time: `None` off
    /// Wayland, on a GTK release whose window objects cannot be read, or on
    /// a desktop without xdg-toplevel-drag-v1.
    fn of(display: &gdk::Display) -> Option<Rc<Self>> {
        if TRIED.replace(true) {
            return PROTOCOL.with_borrow(Clone::clone);
        }
        let protocol = Rc::new(Self::new(display)?);
        PROTOCOL.set(Some(Rc::clone(&protocol)));
        Some(protocol)
    }

    fn new(display: &gdk::Display) -> Option<Self> {
        let display = display.downcast_ref::<gdk_wayland::WaylandDisplay>()?;
        if !crate::gtk_xdg_toplevel::is_readable() {
            return None;
        }
        let connection = Connection::from_backend(display.wl_display()?.backend().upgrade()?);
        let (globals, queue) = registry_queue_init::<Events>(&connection).ok()?;
        let handle = queue.handle();
        let toplevel_drags = globals.bind(&handle, 1..=1, ()).ok()?;
        let data_devices = globals.bind(&handle, 3..=3, ()).ok()?;
        let seat = display
            .default_seat()?
            .downcast::<gdk_wayland::WaylandSeat>()
            .ok()?
            .wl_seat()?;
        let pointer = seat.get_pointer(&handle, ());
        connection.flush().ok()?;
        Some(Self {
            connection,
            queue: RefCell::new(queue),
            events: RefCell::new(Events::default()),
            data_devices,
            toplevel_drags,
            seat,
            _pointer: pointer,
        })
    }

    /// Handles what the desktop sent these objects since last time.
    fn pump(&self) {
        let mut events = self.events.borrow_mut();
        // A broken connection is GTK's to report.
        let _ = self.queue.borrow_mut().dispatch_pending(&mut events);
    }

    fn handle(&self) -> QueueHandle<Events> {
        self.queue.borrow().handle()
    }
}

/// A window drag in progress.
struct Session {
    protocol: Rc<Protocol>,
    source: wl_data_source::WlDataSource,
    toplevel_drag: xdg_toplevel_drag_v1::XdgToplevelDragV1,
    device: wl_data_device::WlDataDevice,
    /// The window the tab was dragged out of.
    origin: glib::WeakRef<BrowserWindow>,
    /// The window the pointer carries: a new one with the tab, or the
    /// origin itself when the tab was its only one.
    dragged: glib::WeakRef<BrowserWindow>,
    /// The tab, in the dragged window.
    tab: TabId,
    /// The tab in the origin, until the origin lets it go.
    origin_tab: TabId,
    /// The origin let the tab go, or keeps it for good.
    released: Cell<bool>,
    /// Where Escape puts the tab back in the origin; `None` when the dragged
    /// window is the origin.
    back: Option<Landing>,
    /// Where the dragged window is held, from its top left corner.
    offset: (i32, i32),
    /// Where along itself the tab was grabbed, for stand-ins.
    grab_x: f64,
    /// The dragged window is hidden: its tab is over a row.
    hidden: Cell<bool>,
    /// The dragged window hides once the row over it shows the stand-in.
    hide_pending: Cell<bool>,
    /// The window whose row shows a stand-in for the tab.
    host: RefCell<glib::WeakRef<BrowserWindow>>,
    /// The tab moved into another window.
    merged: Cell<bool>,
    /// The pointer was released.
    dropped: Cell<bool>,
    /// Handles the drag's events while it lasts.
    pump: RefCell<Option<glib::SourceId>>,
}

impl Session {
    fn current() -> Option<Rc<Self>> {
        SESSION.with_borrow(Clone::clone)
    }

    /// Whether this drag is still the one in progress.
    fn is_current(&self) -> bool {
        SESSION.with_borrow(|session| {
            session
                .as_ref()
                .is_some_and(|session| std::ptr::eq(Rc::as_ptr(session), self))
        })
    }

    /// Whether `window`'s row shows the stand-in for the tab.
    fn is_host(&self, window: &BrowserWindow) -> bool {
        self.host.borrow().upgrade().as_ref() == Some(window)
    }

    /// A drag of tab `tabs.0` of `origin`, which is `tabs.1` of `dragged`:
    /// its data source, offering only [`WINDOW_DRAG_MIME`] and a move, its
    /// window drag and the device that starts it.
    fn new(
        protocol: &Rc<Protocol>,
        origin: &BrowserWindow,
        dragged: &BrowserWindow,
        (origin_tab, tab): (TabId, TabId),
        back: Option<Landing>,
        offset: (i32, i32),
        grab_x: f64,
    ) -> Rc<Self> {
        let handle = protocol.handle();
        let source = protocol.data_devices.create_data_source(&handle, ());
        source.offer(WINDOW_DRAG_MIME.to_owned());
        source.set_actions(wl_data_device_manager::DndAction::Move);
        let toplevel_drag = protocol
            .toplevel_drags
            .get_xdg_toplevel_drag(&source, &handle, ());
        let device = protocol.data_devices.get_data_device(&protocol.seat, &handle, ());
        Rc::new(Self {
            protocol: Rc::clone(protocol),
            source,
            toplevel_drag,
            device,
            origin: origin.downgrade(),
            dragged: dragged.downgrade(),
            tab,
            origin_tab,
            released: Cell::new(back.is_none()),
            back,
            offset,
            grab_x,
            hidden: Cell::new(false),
            hide_pending: Cell::new(false),
            host: RefCell::new(glib::WeakRef::new()),
            merged: Cell::new(false),
            dropped: Cell::new(false),
            pump: RefCell::new(None),
        })
    }

    /// Lets the drag's objects go.
    fn release(&self) {
        self.toplevel_drag.destroy();
        self.source.destroy();
        self.device.release();
        let _ = self.protocol.connection.flush();
    }

    /// The tab stays in the origin's row, where the pointer left it, until
    /// `window` is on screen with it, so it is never nowhere.
    fn release_origin_once_drawn(self: &Rc<Self>, window: &BrowserWindow) {
        let leave = Rc::new(glib::clone!(
            #[weak(rename_to = session)]
            self,
            move || session.release_origin()
        ));
        super::source::when_drawn(window.upcast_ref(), leave.clone());
        glib::timeout_add_local_once(WINDOW_DRAWN_AT_LATEST, move || leave());
    }

    /// Handles the drag's events while it lasts.
    fn start_pump(self: &Rc<Self>) {
        let pump = glib::timeout_add_local(
            PUMP,
            glib::clone!(
                #[weak(rename_to = session)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    session.on_source_events();
                    glib::ControlFlow::Continue
                }
            ),
        );
        self.pump.replace(Some(pump));
    }

    /// Hides the dragged window once `host`, whose row now shows a
    /// stand-in for its tab, has drawn it, so the tab is never nowhere:
    /// unmapped, the window leaves the drag, as Chrome hides it.
    fn hide_window_under(self: &Rc<Self>, host: &BrowserWindow) {
        if self.hidden.get() || self.hide_pending.replace(true) {
            return;
        }
        let session = Rc::clone(self);
        let hide = move || {
            session.hide_pending.set(false);
            // A release may end the drag first: the window then stays.
            if !session.is_current()
                || session.host.borrow().upgrade().is_none()
                || session.hidden.replace(true)
            {
                return;
            }
            if let Some(window) = session.dragged.upgrade() {
                window.set_visible(false);
            }
        };
        super::source::when_drawn(host.upcast_ref(), Rc::new(hide));
    }

    /// Shows the dragged window again under the pointer, attached anew
    /// before it is drawn, once its tab is off every row; `drawn` runs once
    /// it is on screen, or at once when it never hid.
    fn show_window(self: &Rc<Self>, drawn: impl Fn() + 'static) {
        if !self.hidden.replace(false) {
            drawn();
            return;
        }
        let Some(window) = self.dragged.upgrade() else {
            return;
        };
        window.present();
        self.attach(&window);
        super::source::when_drawn(window.upcast_ref(), Rc::new(drawn));
    }

    /// The tab left the row of `window`: the window it belongs to shows
    /// again, and only then does `window` let the stand-in go.
    fn leave_row_of(self: &Rc<Self>, window: &BrowserWindow) {
        let session = Rc::clone(self);
        let row = window.downgrade();
        self.show_window(move || {
            let Some(row) = row.upgrade() else { return };
            if session.host.borrow().upgrade().as_ref() != Some(&row) {
                row.tab_strip().dismiss_guest();
            }
        });
    }

    /// Attaches `window`, just presented and not drawn yet, to the drag.
    fn attach(&self, window: &BrowserWindow) -> bool {
        let Some(toplevel) = crate::gtk_xdg_toplevel::xdg_toplevel(window, &self.protocol.connection) else {
            return false;
        };
        self.toplevel_drag.attach(&toplevel, self.offset.0, self.offset.1);
        let _ = self.protocol.connection.flush();
        true
    }

    /// The origin lets the tab go, now that its window is on screen.
    fn release_origin(&self) {
        if self.released.replace(true) {
            return;
        }
        if let Some(origin) = self.origin.upgrade() {
            origin.tab_strip().end_drag_now();
            origin.release_moved_tab(self.origin_tab);
        }
    }

    /// Handles what happened to the drag's data source.
    fn on_source_events(self: &Rc<Self>) {
        self.protocol.pump();
        let events = std::mem::take(&mut self.protocol.events.borrow_mut().source);
        for event in events {
            match event {
                SourceEvent::Dropped => {
                    self.dropped.set(true);
                    // After GTK handled the drop on the window under the
                    // pointer, which it reads in the same batch.
                    let session = Rc::clone(self);
                    glib::idle_add_local_once(move || session.end());
                }
                SourceEvent::Finished => self.end(),
                // A drop nothing took is cancelled after the drop: only a
                // cancel without one is Escape.
                SourceEvent::Cancelled if self.dropped.get() => self.end(),
                SourceEvent::Cancelled => self.cancel(),
            }
        }
    }

    /// The pointer was released: the tab stays where it went, in another
    /// window or in its own window where the desktop left it.
    fn end(self: &Rc<Self>) {
        if !self.finish() {
            return;
        }
        if let Some(host) = self.host.borrow().upgrade() {
            host.tab_strip().dismiss_guest();
        }
        self.release_origin();
        if !self.merged.get() && self.hidden.get() {
            if let Some(window) = self.dragged.upgrade() {
                window.present();
            }
        }
    }

    /// Escape: the tab goes back where it was, and its new window away.
    fn cancel(self: &Rc<Self>) {
        if !self.finish() {
            return;
        }
        if let Some(host) = self.host.borrow().upgrade() {
            host.tab_strip().dismiss_guest();
        }
        let (Some(dragged), Some(origin)) = (self.dragged.upgrade(), self.origin.upgrade()) else {
            return;
        };
        if !self.released.replace(true) {
            // The origin still has the tab: only the new window goes.
            origin.tab_strip().end_drag_now();
            dragged.close();
            return;
        }
        match self.back {
            Some(Landing { before }) if dragged.hand_over_tab(self.tab, &origin, before).is_ok() => {
                dragged.release_moved_tab(self.tab);
            }
            _ if self.hidden.get() => dragged.present(),
            _ => {}
        }
    }

    /// Ends the session and releases its objects; false when it was over.
    fn finish(&self) -> bool {
        if !self.is_current() {
            return false;
        }
        SESSION.set(None);
        if let Some(pump) = self.pump.take() {
            pump.remove();
        }
        self.release();
        true
    }
}

impl BrowserWindow {
    /// Readies window drags for this window: the protocol objects, once,
    /// when it is first shown, and their events with every event of the
    /// window, so the serial of each press is known.
    pub(super) fn connect_window_drags(&self) {
        self.connect_realize(|window| {
            let _ = Protocol::of(&WidgetExt::display(window));
        });
        let pump = gtk::EventControllerLegacy::new();
        pump.set_propagation_phase(gtk::PropagationPhase::Capture);
        pump.connect_event(|_, _| {
            if let Some(protocol) = PROTOCOL.with_borrow(Clone::clone) {
                protocol.pump();
            }
            glib::Propagation::Proceed
        });
        self.add_controller(pump);
        self.attach_window_drop_target();
    }

    /// Drags tab `id`, which just left its row, out as a window the desktop
    /// moves with the pointer, held at `grab`, where it was grabbed on the
    /// tab: true when it started. Without what that needs, nothing changes
    /// and the caller carries the tab as before.
    pub(super) fn drag_tab_out_as_window(&self, id: TabId, grab: (f64, f64)) -> bool {
        let Some(protocol) = Protocol::of(&WidgetExt::display(self)) else {
            return false;
        };
        protocol.pump();
        let serial = protocol.events.borrow().press;
        let origin_surface = self
            .surface()
            .and_downcast::<gdk_wayland::WaylandSurface>()
            .and_then(|surface| surface.wl_surface());
        let (Some(serial), Some(origin_surface), Some(offset)) =
            (serial, origin_surface, self.window_offset_of_first_tab(id, grab))
        else {
            return false;
        };
        if self.keeps_tab(id) {
            return false;
        }
        // A new press means any earlier drag is over, even if the desktop
        // never said so, as Chrome resets a drag it was not told ended.
        if let Some(stale) = Session::current() {
            stale.end();
        }
        let single = self.tab_count() <= 1;
        let back = (!single).then(|| Landing {
            before: self.tab_after(id),
        });
        let dragged = if single {
            self.tab_strip().end_drag_now();
            self.clone()
        } else {
            match self.window_for_tab(id) {
                Some(window) => window,
                None => return false,
            }
        };
        let tab = dragged.imp().session.borrow().active_id().unwrap_or(id);
        let session = Session::new(&protocol, self, &dragged, (id, tab), back, offset, grab.0);
        // Attached before it is drawn, then dragged with the press, as the
        // protocol and Chrome do.
        if !session.attach(&dragged) {
            session.release();
            if !single {
                // This window still has the tab.
                dragged.close();
            }
            return false;
        }
        session
            .device
            .start_drag(Some(&session.source), &origin_surface, None, serial);
        let _ = protocol.connection.flush();
        if !single {
            session.release_origin_once_drawn(&dragged);
        }
        session.start_pump();
        SESSION.set(Some(session));
        true
    }

    /// A new window with tab `id` of this one, at this window's size, shown
    /// but not drawn yet; this window keeps the tab for now.
    fn window_for_tab(&self, id: TabId) -> Option<BrowserWindow> {
        let window = self.detach_tab(id).ok()?;
        let (width, height) = if self.is_maximized() {
            self.default_size()
        } else {
            (self.width(), self.height())
        };
        window.set_default_size(width, height);
        window.present_as_torn_out();
        Some(window)
    }

    /// Where tab `id`, grabbed at `grab` on itself, is held in a window like
    /// this one where it is the first tab, from the window's top left
    /// corner, where the desktop measures a window from.
    fn window_offset_of_first_tab(&self, id: TabId, (along, down): (f64, f64)) -> Option<(i32, i32)> {
        let strip = self.tab_strip();
        let row = strip.tab_list().compute_point(self, &graphene::Point::zero())?;
        let tab = strip.tab_bounds_in(id, self)?;
        #[expect(clippy::cast_possible_truncation, reason = "a point of a window")]
        let offset = (
            (f64::from(row.x()) + along).round() as i32,
            (f64::from(tab.y()) + down).round() as i32,
        );
        Some(offset)
    }

    /// The tab after tab `id`, or `None` at the end.
    fn tab_after(&self, id: TabId) -> Option<TabId> {
        let session = self.imp().session.borrow();
        let tabs = session.tabs();
        let at = tabs.iter().position(|tab| tab.id == id)?;
        tabs.get(at + 1).map(|tab| tab.id)
    }

    /// Lets this window take window drags: over its tab row the dragged
    /// window gives way to a stand-in for its tab, and a release there
    /// moves the tab here.
    fn attach_window_drop_target(&self) {
        let formats = gdk::ContentFormats::new(&[WINDOW_DRAG_MIME]);
        let target = gtk::DropTargetAsync::new(Some(formats), gdk::DragAction::MOVE);
        target.connect_accept(|_, drop| {
            Session::current().is_some() && drop.formats().contain_mime_type(WINDOW_DRAG_MIME)
        });
        let hover = glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |_: &gtk::DropTargetAsync, _: &gdk::Drop, x: f64, y: f64| window.hover_window_drag(x, y)
        );
        target.connect_drag_enter(hover.clone());
        target.connect_drag_motion(hover);
        target.connect_drag_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.leave_window_drag()
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, drop, x, y| window.drop_window_drag(drop, x, y)
        ));
        self.add_controller(target);
    }

    /// The dragged window is over this one at (`x`, `y`): over the tab row
    /// its tab joins the row as a stand-in that follows the pointer, and
    /// the window hides; anywhere else the window shows.
    fn hover_window_drag(&self, x: f64, y: f64) -> gdk::DragAction {
        let Some(session) = Session::current() else {
            return gdk::DragAction::empty();
        };
        session.on_source_events();
        let dragged = session.dragged.upgrade();
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        let takes = self.row_takes_tab(session.is_host(self), point);
        let view = dragged
            .as_ref()
            .and_then(|window| window.tab_strip().tab_view(session.tab));
        if let (true, Some(view)) = (takes, view) {
            // Back over its own row before its window was drawn: the row
            // lets the tab go now, so it shows once, as the stand-in.
            if session.origin.upgrade().as_ref() == Some(self) {
                session.release_origin();
            }
            let in_strip = self
                .compute_point(self.tab_strip(), &point)
                .map_or(0.0, |point| f64::from(point.x()));
            self.tab_strip().host_guest(&view, in_strip, session.grab_x);
            let previous = session.host.replace(self.downgrade()).upgrade();
            if let Some(previous) = previous.filter(|previous| previous != self) {
                previous.tab_strip().dismiss_guest();
            }
            session.hide_window_under(self);
        } else if session.host.borrow().upgrade().as_ref() == Some(self) {
            session.host.replace(glib::WeakRef::new());
            session.leave_row_of(self);
        }
        // Every window takes the drop, so the desktop leaves the dragged
        // window where it is released, as over Chrome's windows.
        gdk::DragAction::MOVE
    }

    /// Whether this window's row takes the dragged window's tab at `point`:
    /// over the row, and for the row that took it already (`is_host`) down
    /// to Chrome's 15 pixels below it, as its own tabs do.
    fn row_takes_tab(&self, is_host: bool, point: graphene::Point) -> bool {
        let held = is_host && self.is_in_tab_row(graphene::Point::new(point.x(), point.y() - MAGNETISM));
        !self.is_busy_for_tab_moves() && (self.is_in_tab_row(point) || held)
    }

    /// The pointer left this window during a window drag: its row lets the
    /// stand-in go, and the dragged window shows unless another row takes
    /// its tab at once.
    fn leave_window_drag(&self) {
        let Some(session) = Session::current() else {
            self.tab_strip().dismiss_guest();
            return;
        };
        let was_host = session.host.borrow().upgrade().as_ref() == Some(self);
        if !was_host {
            return;
        }
        session.host.replace(glib::WeakRef::new());
        // Another window's row may take the tab in the same moment.
        let window = self.downgrade();
        glib::idle_add_local_once(move || {
            let Some(window) = window.upgrade() else { return };
            match session.host.borrow().upgrade() {
                Some(host) if host == window => {}
                Some(_) => window.tab_strip().dismiss_guest(),
                None => session.leave_row_of(&window),
            }
        });
    }

    /// The dragged window is released over this one at (`x`, `y`): over the
    /// tab row its tab moves in where the stand-in was, and its window goes
    /// away; anywhere else the window stays where it is.
    fn drop_window_drag(&self, drop: &gdk::Drop, x: f64, y: f64) -> bool {
        let Some(session) = Session::current() else {
            return false;
        };
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        // Where the hover showed the tab in this row, also in the band below
        // a row that took it.
        let landing = self
            .row_takes_tab(session.is_host(self), point)
            .then(|| self.tab_strip().land_guest())
            .flatten();
        if let (Some(landing), Some(dragged)) = (landing, session.dragged.upgrade()) {
            match dragged.hand_over_tab(session.tab, self, landing.before) {
                Ok(()) => {
                    session.merged.set(true);
                    dragged.release_moved_tab(session.tab);
                }
                Err(refusal) => self.show_message(&refusal.to_string()),
            }
        }
        drop.finish(gdk::DragAction::MOVE);
        true
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Events {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Events {
    fn event(
        events: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            button: PRIMARY_BUTTON,
            state: WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            events.press = Some(serial);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Events {
    fn event(
        events: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // Nothing is sent: closing the file ends the transfer.
            wl_data_source::Event::Send { fd, .. } => drop(fd),
            wl_data_source::Event::DndDropPerformed => events.source.push(SourceEvent::Dropped),
            wl_data_source::Event::DndFinished => events.source.push(SourceEvent::Finished),
            wl_data_source::Event::Cancelled => events.source.push(SourceEvent::Cancelled),
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Events {
    fn event(
        _: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // GTK's own device takes the clipboard and drops; offers that reach
        // this one are let go.
        match event {
            wl_data_device::Event::Selection { id: Some(offer) }
            | wl_data_device::Event::Enter { id: Some(offer), .. } => {
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(Events, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

delegate_noop!(Events: ignore wl_data_offer::WlDataOffer);
delegate_noop!(Events: wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(Events: xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1);
delegate_noop!(Events: xdg_toplevel_drag_v1::XdgToplevelDragV1);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{wait_for_frames, Fixture, TestWindow};

    /// Off Wayland, as on the tests' X11 display, no window drag is readied
    /// and none starts: a tab dragged out of its row is carried as before.
    ///
    /// parity: TAB-034
    #[gtk::test]
    fn window_drags_need_wayland() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        wait_for_frames(&test.window, 3);
        let id = test.window.imp().session.borrow().active_id().expect("a tab");

        assert!(Protocol::of(&WidgetExt::display(&test.window)).is_none());
        assert!(!test.window.drag_tab_out_as_window(id, (40.0, 10.0)));
        assert!(Session::current().is_none());
    }

    /// A torn-out tab's window is held where the tab was grabbed, the tab
    /// being its new window's first: from the row's start and at the tab's
    /// height, whichever tab of the row it was, as the desktop places the
    /// window from its top left corner.
    ///
    /// parity: TAB-034
    #[gtk::test]
    fn a_torn_out_tab_is_held_where_it_was_grabbed_as_its_windows_first_tab() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        wait_for_frames(&test.window, 3);
        let ids: Vec<TabId> = test
            .window
            .imp()
            .session
            .borrow()
            .tabs()
            .iter()
            .map(|tab| tab.id)
            .collect();
        let first = test
            .window
            .tab_strip()
            .tab_bounds_in(ids[0], &test.window)
            .expect("a shown tab");
        let grab = (40.0, 10.0);
        #[expect(clippy::cast_possible_truncation, reason = "a point of a window")]
        let held = (
            (f64::from(first.x()) + grab.0).round() as i32,
            (f64::from(first.y()) + grab.1).round() as i32,
        );

        for id in &ids {
            assert_eq!(test.window.window_offset_of_first_tab(*id, grab), Some(held));
        }
        assert_eq!(test.window.tab_after(ids[0]), Some(ids[1]));
        assert_eq!(test.window.tab_after(ids[1]), None);
    }

    /// A row takes the dragged window's tab over itself, and the row that
    /// took it keeps it down to 15 pixels below, for a hover and a release
    /// alike, so a release in that band lands the tab where it was shown.
    ///
    /// parity: TAB-033
    #[gtk::test]
    fn a_row_that_took_the_tab_keeps_it_15_pixels_below_itself() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        wait_for_frames(&test.window, 3);
        let strip = test
            .window
            .tab_strip()
            .compute_bounds(&test.window)
            .expect("a strip");
        let bottom = test.window.title_bar_bottom().expect("a title bar");
        let at = |below: f32| graphene::Point::new(strip.x() + 10.0, bottom + below);

        assert!(test.window.row_takes_tab(false, at(-5.0)));
        assert!(test.window.row_takes_tab(true, at(-5.0)));
        assert!(
            !test.window.row_takes_tab(false, at(8.0)),
            "a row that did not take it"
        );
        assert!(test.window.row_takes_tab(true, at(8.0)), "the row that took it");
        assert!(!test.window.row_takes_tab(true, at(20.0)), "past the band");
    }
}
