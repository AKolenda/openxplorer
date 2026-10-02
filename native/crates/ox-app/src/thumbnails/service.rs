// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's thumbnailer service, `org.freedesktop.thumbnails.Thumbnailer1`
//! (tumbler, or another implementation of the specification), when the
//! session bus has one. It makes thumbnails of every type it supports
//! (videos, documents, fonts, pictures) into the shared cache, so the
//! app needs no thumbnailer of its own for them. A request the view no
//! longer needs is taken back out of the service's queue (`Dequeue`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::ThumbnailFlavor;

const BUS_NAME: &str = "org.freedesktop.thumbnails.Thumbnailer1";
const OBJECT_PATH: &str = "/org/freedesktop/thumbnails/Thumbnailer1";
const INTERFACE: &str = BUS_NAME;

/// The most outcomes kept that no caller waited for.
const MAX_UNCLAIMED: usize = 64;

/// How long the service may take to answer a call, in milliseconds.
const CALL_TIMEOUT_MS: i32 = 10_000;

/// Whether the session has the service, once asked.
#[derive(Debug, Clone)]
enum Availability {
    Unknown,
    Absent,
    Present(Rc<Service>),
}

thread_local! {
    static SERVICE: RefCell<Availability> = const { RefCell::new(Availability::Unknown) };
}

/// The outcome of one queued request, by its handle.
#[derive(Debug, Default)]
struct Outcomes {
    /// Requests that made their thumbnail (`Ready`).
    ready: HashSet<u32>,
    /// Requests whose senders wait for `Finished`.
    waiting: HashMap<u32, async_channel::Sender<bool>>,
    /// Requests that finished before their caller waited for them.
    finished: HashMap<u32, bool>,
}

/// The service, with the types it supports.
#[derive(Debug)]
pub(super) struct Service {
    connection: gio::DBusConnection,
    /// URI scheme and MIME type pairs from `GetSupported`.
    supported: HashSet<(String, String)>,
    outcomes: Rc<RefCell<Outcomes>>,
    _signals: Vec<gio::SignalSubscription>,
}

/// The session's thumbnailer service, or `None` without one. Asked once
/// per run; the service is started on demand if it is activatable.
pub(super) async fn service() -> Option<Rc<Service>> {
    match SERVICE.with_borrow(Clone::clone) {
        Availability::Present(service) => return Some(service),
        Availability::Absent => return None,
        Availability::Unknown => {}
    }
    let found = Service::connect().await.map(Rc::new);
    SERVICE.with_borrow_mut(|availability| {
        // Another request may have asked at the same time.
        if let Availability::Present(service) = availability {
            return Some(Rc::clone(service));
        }
        *availability = found.clone().map_or(Availability::Absent, Availability::Present);
        found
    })
}

impl Service {
    /// Connects to the service and reads what it supports.
    async fn connect() -> Option<Self> {
        let connection = gio::bus_get_future(gio::BusType::Session).await.ok()?;
        let reply = connection
            .call_future(
                Some(BUS_NAME),
                OBJECT_PATH,
                INTERFACE,
                "GetSupported",
                None,
                None,
                gio::DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
            )
            .await
            .ok()?;
        let (schemes, types) = reply.get::<(Vec<String>, Vec<String>)>()?;
        let supported = schemes.into_iter().zip(types).collect();
        let outcomes = Rc::new(RefCell::new(Outcomes::default()));
        let signals = ["Ready", "Error", "Finished"]
            .map(|signal| subscribe(&connection, signal, &outcomes))
            .into();
        Some(Self {
            connection,
            supported,
            outcomes,
            _signals: signals,
        })
    }

    /// Whether the service makes thumbnails of `content_type` at `uri`.
    pub(super) fn supports(&self, uri: &str, content_type: &str) -> bool {
        let scheme = glib::Uri::peek_scheme(uri).map(|scheme| scheme.to_string());
        scheme.is_some_and(|scheme| self.supported.contains(&(scheme, content_type.to_owned())))
    }

    /// Asks for the thumbnail of `uri` in `flavor` and waits until the
    /// service is done; `true` when it made one. Dropping the future
    /// takes the request back out of the queue.
    pub(super) async fn make(&self, uri: &str, content_type: &str, flavor: ThumbnailFlavor) -> bool {
        let arguments = (
            vec![uri.to_owned()],
            vec![content_type.to_owned()],
            flavor.as_str(),
            "foreground",
            0_u32,
        )
            .to_variant();
        let reply = self
            .connection
            .call_future(
                Some(BUS_NAME),
                OBJECT_PATH,
                INTERFACE,
                "Queue",
                Some(&arguments),
                None,
                gio::DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
            )
            .await;
        let Some((handle,)) = reply.ok().and_then(|reply| reply.get::<(u32,)>()) else {
            return false;
        };
        let queued = Queued {
            connection: self.connection.clone(),
            outcomes: Rc::clone(&self.outcomes),
            handle,
            done: false,
        };
        let (sender, receiver) = async_channel::bounded(1);
        {
            let mut outcomes = self.outcomes.borrow_mut();
            if let Some(made) = outcomes.finished.remove(&handle) {
                return queued.finish(made);
            }
            outcomes.waiting.insert(handle, sender);
        }
        let made = receiver.recv().await.unwrap_or(false);
        queued.finish(made)
    }
}

/// A request in the service's queue; dropped before it finished, it is
/// taken out again.
struct Queued {
    connection: gio::DBusConnection,
    outcomes: Rc<RefCell<Outcomes>>,
    handle: u32,
    done: bool,
}

impl Queued {
    fn finish(mut self, made: bool) -> bool {
        self.done = true;
        made
    }
}

impl Drop for Queued {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.outcomes.borrow_mut().waiting.remove(&self.handle);
        self.connection.call(
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            "Dequeue",
            Some(&(self.handle,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            gio::Cancellable::NONE,
            |_| {},
        );
    }
}

/// Follows the service's `signal`, recording each request's outcome.
fn subscribe(
    connection: &gio::DBusConnection,
    signal: &'static str,
    outcomes: &Rc<RefCell<Outcomes>>,
) -> gio::SignalSubscription {
    let outcomes = Rc::downgrade(outcomes);
    connection.subscribe_to_signal(
        Some(BUS_NAME),
        Some(INTERFACE),
        Some(signal),
        Some(OBJECT_PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |message| {
            let handle = message
                .parameters
                .try_child_value(0)
                .and_then(|handle| handle.get::<u32>());
            let (Some(outcomes), Some(handle)) = (outcomes.upgrade(), handle) else {
                return;
            };
            let mut outcomes = outcomes.borrow_mut();
            match signal {
                "Ready" => {
                    outcomes.ready.insert(handle);
                }
                "Finished" => {
                    let made = outcomes.ready.remove(&handle);
                    if let Some(sender) = outcomes.waiting.remove(&handle) {
                        let _ = sender.try_send(made);
                    } else {
                        // Kept for a caller about to wait; those of
                        // requests taken back are never asked for.
                        if outcomes.finished.len() >= MAX_UNCLAIMED {
                            outcomes.finished.clear();
                        }
                        outcomes.finished.insert(handle, made);
                    }
                }
                _ => {}
            }
        },
    )
}
