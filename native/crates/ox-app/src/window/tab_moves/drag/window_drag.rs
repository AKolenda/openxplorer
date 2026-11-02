// SPDX-License-Identifier: AGPL-3.0-only
//! A tab dragged out of its row as its own window, which the desktop moves
//! with the pointer, as Chrome drags a torn-out tab on Wayland (TAB-033,
//! TAB-034): xdg-toplevel-drag-v1, in the order of Chromium's
//! `WaylandWindowDragController`.
//!
//! When a tab pressed with the mouse leaves its row, a drag and drop starts
//! with the press's serial and a data source that offers only
//! [`WINDOW_DRAG_MIME`], with no drag icon, and the window the tab now has
//! is attached to the drag before it is first drawn, held
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
//! was not checked, a desktop without the protocol, a touch or a pen) the
//! tab is carried as before.

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

use crate::window::session::{Tab, TabId};
use crate::window::tab_strip::LaterTabs;
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

/// How often a tab Escape takes back tries again while a window is busy.
const RETURN_POLL: Duration = Duration::from_millis(200);

/// What the desktop told this process's own Wayland objects.
#[derive(Debug, Default)]
struct Events {
    /// The primary press held on a window of this app, as this process's
    /// pointer saw it: a drag must start with its serial.
    press: Option<PointerPress>,
    /// How many primary releases this process's pointer saw.
    releases: u64,
    /// What happened to each drag's data source, oldest first, by drag,
    /// with the releases this process's pointer had seen by then.
    source: Vec<(u64, SourceEvent, u64)>,
}

/// A primary press, as this process's pointer saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PointerPress {
    serial: u32,
    /// When, in the desktop's milliseconds.
    time: u32,
}

/// A press of the mouse's primary button that began a tab's drag, as GTK
/// saw it: when it was, in the desktop's milliseconds, which GTK passes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MousePress(u32);

impl MousePress {
    /// The press `controller` handles now, when it is one of the primary button
    /// of the seat's pointer: not a touch, nor a pen, whose drag the desktop
    /// starts only with a serial of their own, which GTK keeps to itself.
    pub(super) fn of(controller: &impl IsA<gtk::EventController>) -> Option<Self> {
        let event = controller.current_event()?;
        let is_primary = event
            .downcast_ref::<gdk::ButtonEvent>()
            .is_some_and(|press| press.button() == gdk::BUTTON_PRIMARY);
        let from_pointer = event
            .device()
            .is_some_and(|device| event.seat().and_then(|seat| seat.pointer()).as_ref() == Some(&device));
        let is_press = event.event_type() == gdk::EventType::ButtonPress;
        (is_press && is_primary && from_pointer && event.device_tool().is_none()).then(|| Self(event.time()))
    }
}

impl Events {
    /// The primary button went down (`pressed`) or up.
    fn on_primary_button(&mut self, serial: u32, time: u32, pressed: bool) {
        if pressed {
            self.press = Some(PointerPress { serial, time });
        } else {
            self.press = None;
            self.releases += 1;
        }
    }

    /// The serial of `press`, when this process's pointer saw it and it is
    /// still held.
    fn serial_of(&self, press: MousePress) -> Option<u32> {
        self.press
            .filter(|seen| seen.time == press.0)
            .map(|seen| seen.serial)
    }

    /// The serial of `press`, as [`Self::serial_of`], for a drag to start
    /// with: one serial starts one drag.
    fn take_serial(&mut self, press: MousePress) -> Option<u32> {
        let serial = self.serial_of(press)?;
        self.press = None;
        Some(serial)
    }

    /// The data source of drag `drag` said `event`.
    fn on_source_event(&mut self, drag: u64, event: SourceEvent) {
        self.source.push((drag, event, self.releases));
    }

    /// What happened to the data source of drag `drag`, oldest first, each
    /// with the releases seen by then. What earlier drags' sources said is
    /// let go: those drags are over.
    fn take_source_events(&mut self, drag: u64) -> Vec<(SourceEvent, u64)> {
        std::mem::take(&mut self.source)
            .into_iter()
            .filter_map(|(of, event, releases)| (of == drag).then_some((event, releases)))
            .collect()
    }
}

/// How giving a tab back went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GiveBack {
    Done,
    /// A window is busy: later.
    Waits,
    /// The tab or a window is gone, or the other window refused it.
    Gone,
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
    /// GTK's seat, held so its `wl_seat` lives as long as these objects.
    gdk_seat: gdk::Seat,
    seat: wl_seat::WlSeat,
    /// This process's own pointer, which sees the serial of each press,
    /// when the seat has one ([`own_pointer`]).
    pointer: RefCell<Option<wl_pointer::WlPointer>>,
    /// How many drags started, which numbers each drag's data source.
    drags: Cell<u64>,
}

thread_local! {
    /// The display the protocol objects were made for, once a window of
    /// this app was shown, and the objects, `None` where it has none: a
    /// display is asked once.
    static PROTOCOL: RefCell<Option<(gdk::Display, Option<Rc<Protocol>>)>> = const { RefCell::new(None) };
    /// The window drag in progress.
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
    /// Tabs Escape takes back that wait for a window to be free, each in
    /// the window it waits in.
    static RETURNING: RefCell<Vec<(glib::WeakRef<BrowserWindow>, TabId)>> = const { RefCell::new(Vec::new()) };
}

impl Protocol {
    /// The protocol objects of `display`, made the first time: `None` off
    /// Wayland, on a GTK release whose window objects cannot be read, on a
    /// desktop without xdg-toplevel-drag-v1, or on another display than the
    /// first, whose windows are on another connection.
    fn of(display: &gdk::Display) -> Option<Rc<Self>> {
        if let Some((made_for, protocol)) = PROTOCOL.with_borrow(Clone::clone) {
            return protocol.filter(|_| made_for == *display);
        }
        let protocol = Self::new(display).map(Rc::new);
        PROTOCOL.set(Some((display.clone(), protocol.clone())));
        protocol
    }

    /// The protocol objects, when they were made.
    fn made() -> Option<Rc<Self>> {
        PROTOCOL.with_borrow(|made| made.as_ref().and_then(|(_, protocol)| protocol.clone()))
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
        let gdk_seat = display.default_seat()?;
        let seat = gdk_seat.downcast_ref::<gdk_wayland::WaylandSeat>()?.wl_seat()?;
        let pointer = own_pointer(&gdk_seat, &seat, &handle);
        connection.flush().ok()?;
        Some(Self {
            connection,
            queue: RefCell::new(queue),
            events: RefCell::new(Events::default()),
            data_devices,
            toplevel_drags,
            gdk_seat,
            seat,
            pointer: RefCell::new(pointer),
            drags: Cell::new(0),
        })
    }

    /// Handles what the desktop sent these objects since last time.
    fn pump(&self) {
        let mut events = self.events.borrow_mut();
        // A broken connection is GTK's to report.
        let _ = self.queue.borrow_mut().dispatch_pending(&mut events);
    }

    /// Reads what the desktop sent so far, then handles it: GTK reads only
    /// between events, so what was sent while one is handled, such as a
    /// release while a window is made, waits unread until then. The read
    /// takes only what is there (libwayland reads without waiting), and
    /// what it brings GTK's objects waits in GTK's own queue for GTK.
    fn read_now(&self) {
        self.pump();
        if let Some(guard) = self.queue.borrow().prepare_read() {
            // Nothing there yet is no error, and a broken connection is
            // GTK's to report.
            let _ = guard.read();
        }
        self.pump();
    }

    fn handle(&self) -> QueueHandle<Events> {
        self.queue.borrow().handle()
    }

    /// Replaces this process's pointer, which missed a press GTK saw: the
    /// desktop leaves a pointer behind when the seat loses its pointer, as
    /// when the only mouse is unplugged, and the next press reaches only a
    /// new one.
    fn renew_pointer(&self) {
        let pointer = own_pointer(&self.gdk_seat, &self.seat, &self.handle());
        let old = self.pointer.replace(pointer);
        // A pointer can be released from version 3 of the seat on.
        if let Some(old) = old.filter(|old| old.version() >= 3) {
            old.release();
        }
        let _ = self.connection.flush();
    }

    /// The number of a new drag.
    fn next_drag(&self) -> u64 {
        self.drags.set(self.drags.get() + 1);
        self.drags.get()
    }
}

/// A pointer of this process's own on `seat`, GTK's `gdk_seat`, when the
/// seat has a pointer and this build knows every event of the seat's
/// version: asking a seat that never had a pointer for one is a protocol
/// error, and so is an event the connection cannot read, either of which
/// would end GTK's connection, and the app with it.
fn own_pointer(
    gdk_seat: &gdk::Seat,
    seat: &wl_seat::WlSeat,
    handle: &QueueHandle<Events>,
) -> Option<wl_pointer::WlPointer> {
    let has_pointer = gdk_seat.capabilities().contains(gdk::SeatCapabilities::POINTER);
    let is_known = seat.version() <= wl_pointer::WlPointer::interface().version;
    (has_pointer && is_known).then(|| seat.get_pointer(handle, ()))
}

/// A window drag in progress.
struct Session {
    protocol: Rc<Protocol>,
    /// Which drag this is, as its data source's events say.
    drag: u64,
    /// The releases this process's pointer had seen when the drag started.
    releases_at_start: u64,
    source: wl_data_source::WlDataSource,
    toplevel_drag: xdg_toplevel_drag_v1::XdgToplevelDragV1,
    device: wl_data_device::WlDataDevice,
    /// The window the tab was dragged out of, until it goes away.
    origin: RefCell<glib::WeakRef<BrowserWindow>>,
    /// The window of this app under the pointer, which the desktop sends
    /// the release to.
    target: RefCell<glib::WeakRef<BrowserWindow>>,
    /// The window the pointer carries: a new one with the tab, or the
    /// origin itself when the tab was its only one.
    dragged: glib::WeakRef<BrowserWindow>,
    /// The tab, in the dragged window.
    tab: TabId,
    /// The tab in the origin, until the origin lets it go.
    origin_tab: TabId,
    /// The origin let the tab go, or keeps it for good.
    released: Cell<bool>,
    /// Where Escape puts the tab back in the origin: before the first of
    /// the tabs after it there still open; `None` when the dragged window
    /// is the origin.
    back: Option<LaterTabs>,
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

    /// Whether this drag holds tab `id` of `window`: its tab in the dragged
    /// window, and in the window it was dragged out of until that window
    /// lets it go, which a merge or Escape still moves.
    fn holds(&self, window: &BrowserWindow, id: TabId) -> bool {
        let is = |weak: &glib::WeakRef<BrowserWindow>| weak.upgrade().as_ref() == Some(window);
        (id == self.tab && is(&self.dragged))
            || (id == self.origin_tab && !self.released.get() && is(&self.origin.borrow()))
    }

    /// A drag of tab `tabs.0` of `origin`, which is `tabs.1` of `dragged`:
    /// its data source, offering only [`WINDOW_DRAG_MIME`] and a move, its
    /// window drag and the device that starts it.
    fn new(
        protocol: &Rc<Protocol>,
        origin: &BrowserWindow,
        dragged: &BrowserWindow,
        (origin_tab, tab): (TabId, TabId),
        back: Option<LaterTabs>,
        offset: (i32, i32),
        grab_x: f64,
    ) -> Rc<Self> {
        let handle = protocol.handle();
        let drag = protocol.next_drag();
        let source = protocol.data_devices.create_data_source(&handle, drag);
        source.offer(WINDOW_DRAG_MIME.to_owned());
        source.set_actions(wl_data_device_manager::DndAction::Move);
        let toplevel_drag = protocol
            .toplevel_drags
            .get_xdg_toplevel_drag(&source, &handle, ());
        let device = protocol.data_devices.get_data_device(&protocol.seat, &handle, ());
        Rc::new(Self {
            protocol: Rc::clone(protocol),
            drag,
            releases_at_start: protocol.events.borrow().releases,
            source,
            toplevel_drag,
            device,
            origin: RefCell::new(origin.downgrade()),
            target: RefCell::new(glib::WeakRef::new()),
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

    /// Lets the drag's objects go: the data source first, as Chromium
    /// does, which ends any drag it has.
    fn release(&self) {
        // KWin refuses to let a toplevel drag go while its source is neither
        // dropped nor cancelled, with a protocol error that would end GTK's
        // connection, and the app with it; without its source, it lets go.
        self.source.destroy();
        self.toplevel_drag.destroy();
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
    /// it is on screen, or at once when it never hid or the drag is over.
    fn show_window(self: &Rc<Self>, drawn: impl Fn() + 'static) {
        // Once the drag is over its window is where the drag's end left it:
        // shown, or gone with its tab moved into another window.
        if !self.is_current() || !self.hidden.replace(false) {
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

    /// Attaches `window` again, presented anew and not drawn yet, to the
    /// drag. Hiding it destroyed the toplevel attached before, so none is
    /// attached now: attaching while one is would be a protocol error.
    fn attach(&self, window: &BrowserWindow) {
        if let Some(toplevel) = crate::gtk_xdg_toplevel::xdg_toplevel(window, &self.protocol.connection) {
            self.toplevel_drag.attach(&toplevel, self.offset.0, self.offset.1);
            let _ = self.protocol.connection.flush();
        }
    }

    /// The origin lets the tab go, now that its window is on screen.
    fn release_origin(&self) {
        if self.released.replace(true) {
            return;
        }
        let origin = self.origin.borrow().upgrade();
        if let Some(origin) = origin {
            origin.tab_strip().end_drag_now();
            origin.release_moved_tab(self.origin_tab);
        }
    }

    /// Handles what happened to the drag's data source.
    fn on_source_events(self: &Rc<Self>) {
        self.protocol.pump();
        let (events, released) = {
            let mut seen = self.protocol.events.borrow_mut();
            (
                seen.take_source_events(self.drag),
                seen.releases != self.releases_at_start,
            )
        };
        for (event, releases) in events {
            let after_release = releases != self.releases_at_start;
            match event {
                SourceEvent::Dropped => {
                    self.dropped.set(true);
                    // After GTK handled the drop on the window under the
                    // pointer, which it reads in the same batch.
                    let session = Rc::clone(self);
                    glib::idle_add_local_once(move || session.end());
                }
                SourceEvent::Finished => self.end(),
                // A drop nothing took is cancelled after the drop, and a
                // drag the desktop refused, its button already up, after the
                // release this process's pointer saw: Escape cancels with
                // the button held, after neither.
                SourceEvent::Cancelled if self.dropped.get() || after_release => self.end(),
                SourceEvent::Cancelled => self.cancel(),
            }
        }
        // The release reached this process's pointer, so the desktop never
        // took the drag and did not say so, as when the button was up
        // before the drag could start (KWin cancels a drag it refuses, other
        // desktops may not). The tab stays in its window, where it is.
        if released {
            self.end();
        }
    }

    /// The pointer was released: the tab stays where it went, in another
    /// window or in its own window where the desktop left it.
    fn end(self: &Rc<Self>) {
        self.end_without(None);
    }

    /// `window` left the screen during the drag. As the window under the
    /// pointer or the row showing the stand-in, the drag ends first, its tab
    /// where it is, so the desktop cancels the drag; the dragged window,
    /// which the drag hides itself over a row, and any other window are
    /// none of this.
    fn on_window_hidden(self: &Rc<Self>, window: &BrowserWindow) {
        // KWin still sends the release to the window it last had under the
        // pointer after that window left the screen: a drop GTK cannot take,
        // as it has no drop then, and crashes.
        let is =
            |weak: &RefCell<glib::WeakRef<BrowserWindow>>| weak.borrow().upgrade().as_ref() == Some(window);
        if self.dragged.upgrade().as_ref() != Some(window) && (is(&self.host) || is(&self.target)) {
            self.end_without(Some(window));
        }
    }

    /// `window` goes away during the drag: the dragged window ends the drag
    /// at once, as one under the pointer or showing the stand-in does
    /// ([`Self::on_window_hidden`]); the window the tab left only loses its
    /// claim to the tab, and any other window is none of the drag's business.
    fn on_window_gone(self: &Rc<Self>, window: &BrowserWindow) {
        let is =
            |weak: &RefCell<glib::WeakRef<BrowserWindow>>| weak.borrow().upgrade().as_ref() == Some(window);
        if self.dragged.upgrade().as_ref() == Some(window) || is(&self.host) || is(&self.target) {
            self.end_without(Some(window));
        } else if is(&self.origin) {
            self.origin.replace(glib::WeakRef::new());
            self.released.set(true);
        }
    }

    /// [`Self::end`], with `gone`, a window going away, left alone.
    fn end_without(self: &Rc<Self>, gone: Option<&BrowserWindow>) {
        if !self.finish() {
            return;
        }
        let stays = |window: &BrowserWindow| Some(window) != gone;
        if let Some(host) = self.host.borrow().upgrade().filter(stays) {
            host.tab_strip().dismiss_guest();
        }
        let origin = self.origin.borrow().upgrade();
        if origin.is_some_and(|origin| stays(&origin)) {
            self.release_origin();
        } else {
            // The window it left goes away, and the tab with it.
            self.released.set(true);
        }
        if let Some(window) = self.dragged.upgrade().filter(stays) {
            show_dragged_again(&window, self.tab, self.merged.get(), self.hidden.get());
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
        let Some(dragged) = self.dragged.upgrade() else {
            return;
        };
        let origin = self.origin.borrow().upgrade();
        let released = self.released.replace(true);
        put_tab_back(
            &dragged,
            origin.as_ref(),
            (self.tab, self.back.clone()),
            released,
            self.hidden.get(),
        );
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

/// The drag over, whichever way it ended: a `hidden` dragged window shows
/// again while it keeps a tab, its own `tab` unless that `left` it, merged
/// into another window or given back by Escape, or one opened in it during
/// the drag, so no tab is left in a window no one sees.
fn show_dragged_again(dragged: &BrowserWindow, tab: TabId, left: bool, hidden: bool) {
    let has_another =
        |session: &crate::window::session::Session| session.tabs().iter().any(|kept| kept.id != tab);
    if hidden && (!left || has_another(&dragged.imp().session.borrow())) {
        dragged.present();
    }
}

/// Escape, the drag over: tab `tabs.0` of `dragged` goes back to `origin`,
/// before the first of `tabs.1` still open there, or only its copy in `dragged` goes while `origin` has
/// not `released` the tab yet, with `dragged` unless a tab opened there
/// meanwhile. A `hidden` window that keeps a tab then shows again
/// ([`show_dragged_again`]), as when `origin` went away during the drag.
fn put_tab_back(
    dragged: &BrowserWindow,
    origin: Option<&BrowserWindow>,
    (tab, back): (TabId, Option<LaterTabs>),
    released: bool,
    hidden: bool,
) {
    let left = if let Some(origin) = origin.filter(|_| !released) {
        // The origin still has the tab: only its copy here goes.
        origin.tab_strip().end_drag_now();
        if dragged.tab_count() <= 1 {
            dragged.close();
            return;
        }
        dragged.release_moved_tab(tab);
        true
    } else {
        origin
            .zip(back)
            .is_some_and(|(origin, later)| dragged.take_tab_back(origin, tab, later) == GiveBack::Done)
    };
    show_dragged_again(dragged, tab, left, hidden);
}

impl BrowserWindow {
    /// Readies window drags for this window: the protocol objects, once,
    /// when it is first shown, and their events with every event of the
    /// window, so the serial of each press is known.
    pub(super) fn connect_window_drags(&self) {
        self.connect_realize(|window| {
            let _ = Protocol::of(&WidgetExt::display(window));
        });
        self.connect_unmap(|window| {
            if let Some(session) = Session::current() {
                session.on_window_hidden(window);
            }
        });
        self.connect_unrealize(|window| {
            if let Some(session) = Session::current() {
                session.on_window_gone(window);
            }
        });
        let pump = gtk::EventControllerLegacy::new();
        pump.set_propagation_phase(gtk::PropagationPhase::Capture);
        pump.connect_event(|_, _| {
            if let Some(protocol) = Protocol::made() {
                protocol.pump();
            }
            glib::Propagation::Proceed
        });
        self.add_controller(pump);
        self.attach_window_drop_target();
    }

    /// Drags tab `id`, which just left its row, out as a window the desktop
    /// moves with the pointer, held at `grab`, where it was grabbed on the
    /// tab: true when it started. It starts only with the press of the
    /// mouse's primary button that began the tab's drag, `pressed_with`,
    /// still held. Without what that needs, nothing changes and the caller
    /// carries the tab as before.
    pub(super) fn drag_tab_out_as_window(
        &self,
        id: TabId,
        grab: (f64, f64),
        pressed_with: Option<MousePress>,
    ) -> bool {
        // A touch or a pen: GTK's drag and drop carries the tab.
        let Some(pressed_with) = pressed_with else {
            return false;
        };
        let Some(protocol) = Protocol::of(&WidgetExt::display(self)) else {
            return false;
        };
        protocol.pump();
        if protocol.events.borrow().serial_of(pressed_with).is_none() {
            // A press GTK saw and this process's pointer did not, or one
            // already released: a new pointer sees the next press.
            protocol.renew_pointer();
            return false;
        }
        let origin_surface = self
            .surface()
            .and_downcast::<gdk_wayland::WaylandSurface>()
            .and_then(|surface| surface.wl_surface());
        let (Some(origin_surface), Some(offset)) =
            (origin_surface, self.window_offset_of_first_tab(id, grab))
        else {
            return false;
        };
        if !self.drags_out_as_window(id) {
            return false;
        }
        // A new press means any earlier drag is over, even if the desktop
        // never said so, as Chrome resets a drag it was not told ended:
        // what its source said last is its own, then it ends.
        if let Some(stale) = Session::current() {
            stale.on_source_events();
            stale.end();
        }
        let single = self.tab_count() <= 1;
        let back = (!single).then(|| self.tabs_after(id));
        let dragged = if single {
            self.clone()
        } else {
            match self.window_for_tab(id) {
                Some(window) => window,
                None => return false,
            }
        };
        let tab = dragged.imp().session.borrow().active_id().unwrap_or(id);
        // What may fail comes before the drag's objects are made, so they
        // always serve a drag: the window's toplevel, and the press, which
        // may have ended while the window was made: what the desktop sent
        // meanwhile is read first.
        protocol.read_now();
        let toplevel = crate::gtk_xdg_toplevel::xdg_toplevel(&dragged, &protocol.connection);
        let serial = toplevel
            .is_some()
            .then(|| protocol.events.borrow_mut().take_serial(pressed_with))
            .flatten();
        let (Some(toplevel), Some(serial)) = (toplevel, serial) else {
            if !single {
                // This window still has the tab.
                dragged.close();
            }
            return false;
        };
        let session = Session::new(&protocol, self, &dragged, (id, tab), back, offset, grab.0);
        // Attached before it is drawn, then dragged with the press, as the
        // protocol and Chrome do.
        session.toplevel_drag.attach(&toplevel, offset.0, offset.1);
        if single {
            self.tab_strip().end_drag_now();
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

    /// Whether the window drag in progress holds tab `id` of this window,
    /// which then neither closes nor changes location (TAB-003).
    pub(super) fn window_drag_holds(&self, id: TabId) -> bool {
        let is_returning = RETURNING.with_borrow(|returning| {
            returning
                .iter()
                .any(|(window, tab)| *tab == id && window.upgrade().as_ref() == Some(self))
        });
        is_returning || Session::current().is_some_and(|session| session.holds(self, id))
    }

    /// Whether tab `id` may be dragged out as a window. Not while it must
    /// stay (TAB-031), nor a split tab of a window with other tabs: its
    /// window would get a copy of only the active pane, and the original,
    /// both panes, goes before the drag ends, so Escape could not put it
    /// back whole. GTK's drag and drop carries that one, and keeps it here
    /// until it lands. A window's only tab moves with its window, split or
    /// not.
    fn drags_out_as_window(&self, id: TabId) -> bool {
        if self.keeps_tab(id) {
            return false;
        }
        let session = self.imp().session.borrow();
        session.tabs().len() <= 1 || !session.tab(id).is_some_and(Tab::is_split)
    }

    /// Whether a window drag holds the mouse with this window: its dragged
    /// window, the window it left or the row showing its stand-in.
    pub(super) fn is_in_window_drag(&self) -> bool {
        Session::current().is_some_and(|session| {
            let is = |weak: &glib::WeakRef<BrowserWindow>| weak.upgrade().as_ref() == Some(self);
            is(&session.dragged) || is(&session.origin.borrow()) || is(&session.host.borrow())
        })
    }

    /// Escape takes tab `tab` of this window, the window it was dragged
    /// out as, back into `origin`, before the first of `later` still open
    /// there when it goes: at once, true, or
    /// once neither window is busy (a file operation, a dialog, the tab's
    /// Properties), the tab held meanwhile, so whatever opened during the
    /// drag, Escape puts the tab back where it was (TAB-035). How it went
    /// at once: [`GiveBack::Waits`] while it waits.
    pub(super) fn take_tab_back(&self, origin: &BrowserWindow, tab: TabId, later: LaterTabs) -> GiveBack {
        let gave = self.give_tab_back(origin, tab, &later);
        if gave != GiveBack::Waits {
            return gave;
        }
        self.show_message(ox_core::i18n::gettext_static(super::TAB_IS_MOVING));
        RETURNING.with_borrow_mut(|returning| returning.push((self.downgrade(), tab)));
        let (from, to) = (self.downgrade(), origin.downgrade());
        glib::timeout_add_local(RETURN_POLL, move || {
            let gave = match (from.upgrade(), to.upgrade()) {
                (Some(from), Some(to)) => from.give_tab_back(&to, tab, &later),
                _ => GiveBack::Gone,
            };
            if gave == GiveBack::Waits {
                return glib::ControlFlow::Continue;
            }
            RETURNING.with_borrow_mut(|returning| {
                returning.retain(|(window, waiting)| !(*waiting == tab && *window == from));
            });
            glib::ControlFlow::Break
        });
        GiveBack::Waits
    }

    /// Gives tab `tab` back to `origin` when neither window is busy, before
    /// the first of `later` still open there.
    fn give_tab_back(&self, origin: &BrowserWindow, tab: TabId, later: &LaterTabs) -> GiveBack {
        if self.imp().session.borrow().tab(tab).is_none() {
            return GiveBack::Gone;
        }
        if self.keeps_tab(tab) || origin.is_busy_for_tab_moves() {
            return GiveBack::Waits;
        }
        let before = later.first_open(|next| {
            origin
                .imp()
                .session
                .borrow()
                .tabs()
                .iter()
                .any(|open| open.id == next)
        });
        match self.hand_over_tab(tab, origin, before) {
            Ok(()) => {
                self.release_moved_tab(tab);
                GiveBack::Done
            }
            Err(_) => GiveBack::Gone,
        }
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
    /// corner, where the desktop measures a window from. The row starts
    /// where it does unscrolled, as in the new window.
    fn window_offset_of_first_tab(&self, id: TabId, (along, down): (f64, f64)) -> Option<(i32, i32)> {
        let strip = self.tab_strip();
        let row = strip
            .tab_list()
            .parent()?
            .compute_point(self, &graphene::Point::zero())?;
        let tab = strip.tab_bounds_in(id, self)?;
        #[expect(clippy::cast_possible_truncation, reason = "a point of a window")]
        let offset = (
            (f64::from(row.x()) + along).round() as i32,
            (f64::from(tab.y()) + down).round() as i32,
        );
        Some(offset)
    }

    /// The tabs after tab `id`, left to right.
    fn tabs_after(&self, id: TabId) -> LaterTabs {
        let session = self.imp().session.borrow();
        let tabs = session.tabs();
        let at = tabs
            .iter()
            .position(|tab| tab.id == id)
            .map_or(tabs.len(), |at| at + 1);
        LaterTabs::new(tabs[at..].iter().map(|tab| tab.id))
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
        session.target.replace(self.downgrade());
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
            if session.origin.borrow().upgrade().as_ref() == Some(self) {
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
        // The pointer left this window, unless another took it meanwhile: not
        // at once, as GTK also says so for a window leaving the screen, which
        // the desktop still takes for the window under the pointer
        // (Session::on_window_hidden).
        let (left, window) = (Rc::clone(&session), self.downgrade());
        glib::idle_add_local_once(move || {
            let is_target = left.target.borrow().upgrade() == window.upgrade();
            if is_target {
                left.target.replace(glib::WeakRef::new());
            }
        });
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
            time,
            button: PRIMARY_BUTTON,
            state: WEnum::Value(state),
        } = event
        {
            events.on_primary_button(serial, time, state == wl_pointer::ButtonState::Pressed);
        }
    }
}

/// A data source's events, for drag `drag`, its user data.
impl Dispatch<wl_data_source::WlDataSource, u64> for Events {
    fn event(
        events: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        drag: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let event = match event {
            // Nothing is sent: closing the file ends the transfer.
            wl_data_source::Event::Send { fd, .. } => {
                drop(fd);
                return;
            }
            wl_data_source::Event::DndDropPerformed => SourceEvent::Dropped,
            wl_data_source::Event::DndFinished => SourceEvent::Finished,
            wl_data_source::Event::Cancelled => SourceEvent::Cancelled,
            _ => return,
        };
        events.on_source_event(*drag, event);
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
    use crate::test_support::harness::{
        wait_for, wait_for_frames, wait_until, Fixture, OpenedWindows, TestWindow,
    };
    use crate::test_support::pointer::{screen_point, Button, Pointer};

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
        assert!(!test
            .window
            .drag_tab_out_as_window(id, (40.0, 10.0), Some(MousePress(0))));
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
        assert_eq!(test.window.tabs_after(ids[0]), LaterTabs::new([ids[1]]));
        assert_eq!(test.window.tabs_after(ids[1]), LaterTabs::default());
    }

    /// A tab torn out of a row scrolled sideways is held where it was
    /// grabbed in its new window, whose row is not scrolled: as far from
    /// the window's corner as with the row unscrolled.
    ///
    /// parity: TAB-034
    #[gtk::test]
    fn a_tab_torn_out_of_a_scrolled_row_is_held_where_it_was_grabbed() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let viewport = test
            .window
            .tab_strip()
            .tab_list()
            .parent()
            .and_downcast::<gtk::Viewport>()
            .expect("the row scrolls in a viewport");
        let scroll = viewport.hadjustment().expect("a sideways scroll");
        for _ in 0..60 {
            if scroll.upper() > scroll.page_size() + 100.0 {
                break;
            }
            test.window.add_tab(&fixture.uri()).expect("a folder");
            test.wait_for_listing("another tab");
            wait_for_frames(&test.window, 2);
        }
        let id = test.window.imp().session.borrow().active_id().expect("a tab");
        let grab = (40.0, 10.0);
        scroll.set_value(0.0);
        wait_for_frames(&test.window, 2);
        let unscrolled = test.window.window_offset_of_first_tab(id, grab);

        scroll.set_value(scroll.upper() - scroll.page_size());
        wait_for_frames(&test.window, 2);

        assert!(scroll.value() > 50.0, "the row is scrolled");
        assert_eq!(test.window.window_offset_of_first_tab(id, grab), unscrolled);
    }

    /// A window drag starts only with the press GTK began the tab's drag
    /// with, while it is held: not with an earlier click, a released press
    /// or one a drag already started with, whose serial the desktop would
    /// refuse. Each release is counted.
    ///
    /// parity: TAB-034
    #[test]
    fn a_window_drag_starts_only_with_the_held_press_that_began_it() {
        let mut events = Events::default();

        events.on_primary_button(7, 100, true);
        assert_eq!(events.serial_of(MousePress(100)), Some(7));
        assert_eq!(events.serial_of(MousePress(90)), None, "an earlier press");
        assert_eq!(events.take_serial(MousePress(100)), Some(7));
        assert_eq!(events.serial_of(MousePress(100)), None, "used by a drag");

        events.on_primary_button(8, 200, true);
        events.on_primary_button(9, 250, false);
        assert_eq!(events.serial_of(MousePress(200)), None, "released");
        assert_eq!(events.releases, 1);
    }

    /// A drag handles only what happened to its own data source: what an
    /// earlier drag's source said, not handled when the next drag started,
    /// is let go, not taken for the new drag's drop, end or cancel.
    ///
    /// parity: TAB-034
    #[test]
    fn a_drag_handles_only_what_its_own_source_said() {
        let mut events = Events::default();
        events.on_source_event(1, SourceEvent::Dropped);
        events.on_source_event(1, SourceEvent::Cancelled);
        events.on_source_event(2, SourceEvent::Dropped);

        assert_eq!(events.take_source_events(2), vec![(SourceEvent::Dropped, 0)]);
        assert!(events.source.is_empty(), "the earlier drag's events are let go");
    }

    /// What a drag's source said is kept with the releases this process's
    /// pointer had seen by then: a cancel after a release is a desktop
    /// refusing a drag whose button was already up, never Escape, which
    /// cancels with the button held.
    ///
    /// parity: TAB-034
    #[test]
    fn a_cancel_is_kept_with_the_releases_seen_before_it() {
        let mut events = Events::default();
        events.on_source_event(1, SourceEvent::Cancelled);
        events.on_primary_button(5, 50, false);
        events.on_source_event(1, SourceEvent::Cancelled);

        assert_eq!(
            events.take_source_events(1),
            vec![(SourceEvent::Cancelled, 0), (SourceEvent::Cancelled, 1)]
        );
    }

    /// A press of the mouse's primary button is one a window drag may start
    /// with, at the time GTK gives it, which is the desktop's; another
    /// button is not.
    ///
    /// parity: TAB-034
    #[gtk::test]
    fn a_primary_mouse_press_may_start_a_window_drag() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        wait_for_frames(&test.window, 3);
        let mut pointer = Pointer::connect();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[strong]
            seen,
            move |click, _, _, _| {
                seen.borrow_mut()
                    .push((MousePress::of(click), click.current_event_time()));
            }
        ));
        test.window.add_controller(click);
        let height = f64::from(test.window.height());
        let at = screen_point(&mut pointer, &test.window, (40.0, height - 6.0), (8, 8));

        pointer.move_to(at.0, at.1);
        for button in [Button::Primary, Button::Middle] {
            pointer.press(button);
            pointer.release(button);
        }

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "both presses reached the window");
        assert_eq!(seen[0].0, Some(MousePress(seen[0].1)), "the primary button");
        assert_eq!(seen[1].0, None, "the middle button");
    }

    /// A split tab of a window with other tabs is not dragged out as a
    /// window, which would take only its active pane, but carried by GTK's
    /// drag and drop, which keeps both panes here until it lands: Escape
    /// leaves the tab whole. Its unsplit neighbour, and a split window's
    /// only tab, which moves with its window, are dragged out as windows.
    ///
    /// parity: TAB-035, VIEW-059
    #[gtk::test]
    fn a_split_tab_with_other_tabs_is_not_dragged_out_as_a_window() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .split_tab(Some(&fixture.uri_of("Documents")))
            .expect("split");
        test.wait_for_listing("the right pane");
        let split = test.window.imp().session.borrow().active_id().expect("a tab");
        assert!(
            test.window.drags_out_as_window(split),
            "a split window's only tab"
        );

        test.window.add_tab(&fixture.uri()).expect("a folder");
        test.wait_for_listing("the second tab");
        let plain = test.window.imp().session.borrow().active_id().expect("a tab");

        assert!(!test.window.drags_out_as_window(split), "a split tab with others");
        assert!(test.window.drags_out_as_window(plain), "an unsplit tab");
    }

    /// Escape takes a torn-out tab back where it was even when a window is
    /// busy by then, as when a file operation started or a dialog opened
    /// during the drag: the tab waits in its window, held, so it neither
    /// closes nor moves, and goes back once neither window is busy.
    ///
    /// parity: TAB-035
    #[gtk::test]
    fn escape_takes_the_tab_back_once_its_windows_are_no_longer_busy() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let moved = test.window.imp().session.borrow().active_id().expect("a tab");
        let later = test.window.tabs_after(moved);
        let dragged = test.window.window_for_tab(moved).expect("a window for the tab");
        let _opened = OpenedWindows::only(&[&test.window]);
        test.window.release_moved_tab(moved);
        let tab = dragged.imp().session.borrow().active_id().expect("the tab");
        assert!(test.window.begin_test_write(), "the window it left is busy");

        dragged.take_tab_back(&test.window, tab, later);
        wait_for(Duration::from_millis(300));
        let waited = (test.window.tab_count(), dragged.window_drag_holds(tab));
        dragged.close_tab(tab);
        let kept = dragged.tab_count();
        test.window.end_test_write();
        wait_until("the tab back where it was", || test.window.tab_count() == 2);

        assert_eq!(waited, (1, true), "the tab waits in its window, held");
        assert_eq!(kept, 1, "a waiting tab does not close");
        assert_eq!(
            test.window.current_uri(),
            Some(fixture.uri_of("Documents")),
            "back in front where it was"
        );
    }

    /// Escape after the tab that followed the torn-out tab closed puts it
    /// back before the next tab still open there, where it was, not at the
    /// end of the row.
    ///
    /// parity: TAB-035
    #[gtk::test]
    fn escape_after_the_next_tab_closed_puts_the_tab_back_before_the_one_after() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        for folder in ["Documents", "Music", "Pictures"] {
            test.window.add_tab(&fixture.uri_of(folder)).expect("a folder");
            test.wait_for_listing("another tab");
        }
        let ids: Vec<TabId> = test
            .window
            .imp()
            .session
            .borrow()
            .tabs()
            .iter()
            .map(|tab| tab.id)
            .collect();
        let [_, moved, next, _] = ids[..] else {
            panic!("four tabs are open");
        };
        let later = test.window.tabs_after(moved);
        let dragged = test.window.window_for_tab(moved).expect("a window for the tab");
        let _opened = OpenedWindows::only(&[&test.window]);
        test.window.release_moved_tab(moved);
        let tab = dragged.imp().session.borrow().active_id().expect("the tab");
        test.window.close_tab(next);

        put_tab_back(&dragged, Some(&test.window), (tab, Some(later)), true, false);

        let order: Vec<String> = test
            .window
            .imp()
            .session
            .borrow()
            .tabs()
            .iter()
            .map(|tab| tab.uri().to_owned())
            .collect();
        assert_eq!(
            order,
            [
                fixture.uri(),
                fixture.uri_of("Documents"),
                fixture.uri_of("Pictures")
            ]
        );
    }

    /// Escape after the window the tab left went away, with the dragged
    /// window hidden over another row, shows that window again: the tab has
    /// nowhere to go back to, and stays in a window that can be seen.
    ///
    /// parity: TAB-035
    #[gtk::test]
    fn escape_shows_a_hidden_dragged_window_whose_origin_went_away() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let moved = test.window.imp().session.borrow().active_id().expect("a tab");
        let later = test.window.tabs_after(moved);
        let dragged = test.window.window_for_tab(moved).expect("a window for the tab");
        let _opened = OpenedWindows::only(&[&test.window]);
        test.window.release_moved_tab(moved);
        let tab = dragged.imp().session.borrow().active_id().expect("the tab");
        dragged.set_visible(false);

        put_tab_back(&dragged, None, (tab, Some(later)), true, true);

        assert!(dragged.is_visible(), "the dragged window shows again");
        assert_eq!(dragged.tab_count(), 1, "with its tab");
    }

    /// A window drag whose tab moved into another row while a tab opened in
    /// its hidden window, as an external request may open one there, ends
    /// with that window shown again, so the tab it keeps can be seen.
    ///
    /// parity: TAB-033
    #[gtk::test]
    fn a_hidden_dragged_window_that_keeps_a_tab_after_a_merge_shows_again() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let merged = test.window.imp().session.borrow().active_id().expect("a tab");
        test.window.release_moved_tab(merged);
        test.window.set_visible(false);

        show_dragged_again(&test.window, merged, true, true);

        assert!(test.window.is_visible(), "the dragged window shows again");
    }

    /// Escape with the dragged window hidden over a row, after a tab opened
    /// in it during the drag, takes the dragged tab back and shows that
    /// window again with the tab it keeps, so no tab is left unseen.
    ///
    /// parity: TAB-035
    #[gtk::test]
    fn escape_shows_a_hidden_dragged_window_that_keeps_a_tab_opened_during_the_drag() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let moved = test.window.imp().session.borrow().active_id().expect("a tab");
        let later = test.window.tabs_after(moved);
        let dragged = test.window.window_for_tab(moved).expect("a window for the tab");
        let _opened = OpenedWindows::only(&[&test.window]);
        test.window.release_moved_tab(moved);
        let tab = dragged.imp().session.borrow().active_id().expect("the tab");
        dragged.add_tab(&fixture.uri()).expect("a folder");
        dragged.set_visible(false);

        put_tab_back(&dragged, Some(&test.window), (tab, Some(later)), true, true);

        assert_eq!(test.window.tab_count(), 2, "the tab went back");
        assert!(dragged.is_visible(), "the dragged window shows again");
        assert_eq!(dragged.tab_count(), 1, "with the tab opened in it");
    }

    /// Escape before the window the tab left let it go, after a tab opened
    /// in the hidden dragged window, keeps that window and its tab, shown:
    /// only the copy of the dragged tab goes.
    ///
    /// parity: TAB-035
    #[gtk::test]
    fn escape_before_the_origin_let_go_keeps_a_tab_opened_in_the_dragged_window() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let moved = test.window.imp().session.borrow().active_id().expect("a tab");
        let later = test.window.tabs_after(moved);
        let dragged = test.window.window_for_tab(moved).expect("a window for the tab");
        let _opened = OpenedWindows::only(&[&test.window]);
        let tab = dragged.imp().session.borrow().active_id().expect("the tab");
        dragged.add_tab(&fixture.uri()).expect("a folder");
        dragged.set_visible(false);

        put_tab_back(&dragged, Some(&test.window), (tab, Some(later)), false, true);

        assert_eq!(test.window.tab_count(), 2, "the window it left kept the tab");
        assert!(dragged.is_visible(), "the dragged window shows again");
        assert_eq!(dragged.tab_count(), 1, "with the tab opened in it");
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
