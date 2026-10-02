// SPDX-License-Identifier: AGPL-3.0-only
//! The Open and Save dialog backend on the application's bus name, ready
//! before the name is (INT-032).
//!
//! The desktop portal starts the app through D-Bus activation of its
//! name, often while the portal itself starts, and calls the backend as
//! soon as the name has an owner. `GApplication` owns the name during
//! registration, before `startup` builds the application state, and GTK
//! accepts windows only once registration has finished. So:
//!
//! - the object is exported in `dbus_register`, which runs before the name
//!   is requested, so no call finds it missing;
//! - a call that arrives before the windows can be made waits in
//!   [`ChooserRoute`], holding the application so it does not quit, and
//!   is shown from the main loop once `startup` has attached the window
//!   opener;
//! - a launch for D-Bus activation (`--gapplication-service`) stays up for
//!   [`SERVICE_LINGER`] without a window, so the portal's first call, which
//!   may come seconds after it started the app, finds it running
//!   ([`linger_as_service`]), and waits for the portal before reading the
//!   desktop's appearance from it rather than starting a second portal.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{ChooserCall, ChooserNotShown, FileChooserBus};

/// How long a D-Bus-activated instance waits for its first request
/// without a window.
pub(super) const SERVICE_LINGER: Duration = Duration::from_secs(60);

/// The option the D-Bus service file starts the app with.
const SERVICE_OPTION: &str = "--gapplication-service";

/// What shows a call in a window.
type Target = Box<dyn Fn(ChooserCall) -> Result<(), ChooserNotShown>>;

/// Where calls go: the window opener once the application has started,
/// a queue before that.
#[derive(Default)]
pub(super) struct ChooserRoute {
    target: RefCell<Option<Target>>,
    pending: RefCell<Vec<ChooserCall>>,
    /// The application, held while calls wait.
    application: glib::WeakRef<gio::Application>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

impl fmt::Debug for ChooserRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChooserRoute")
            .field("is_attached", &self.target.borrow().is_some())
            .field("pending", &self.pending.borrow().len())
            .finish_non_exhaustive()
    }
}

impl ChooserRoute {
    /// Shows `call`, or keeps it, holding the application, until the
    /// windows can be made.
    fn deliver(&self, call: ChooserCall) -> Result<(), ChooserNotShown> {
        if let Some(target) = self.target.borrow().as_ref() {
            if self.pending.borrow().is_empty() {
                return target(call);
            }
        }
        if self.hold.borrow().is_none() {
            let hold = self.application.upgrade().map(|application| application.hold());
            self.hold.replace(hold);
        }
        self.pending.borrow_mut().push(call);
        Ok(())
    }

    /// Sends calls to `target` from now on. Calls that waited are shown
    /// from the main loop, after `startup` has returned and GTK accepts
    /// windows; one the target cannot show is dropped, which answers
    /// "other".
    pub(super) fn attach(
        self: &Rc<Self>,
        target: impl Fn(ChooserCall) -> Result<(), ChooserNotShown> + 'static,
    ) {
        self.target.replace(Some(Box::new(target)));
        let route = Rc::clone(self);
        glib::idle_add_local_once(move || route.flush());
    }

    /// Shows the calls that waited, then lets the application go: their
    /// windows hold it now.
    fn flush(&self) {
        let waiting: Vec<ChooserCall> = self.pending.take();
        if let Some(target) = self.target.borrow().as_ref() {
            for call in waiting {
                let _ = target(call);
            }
        }
        self.hold.take();
    }
}

/// The backend on `connection` for `application`, delivering through
/// `route`; `None`, with a warning, when it cannot be exported, so the app
/// still starts.
pub(super) fn export(
    application: &gio::Application,
    connection: &gio::DBusConnection,
    route: &Rc<ChooserRoute>,
) -> Option<FileChooserBus> {
    route.application.set(Some(application));
    let delivering = Rc::clone(route);
    let mut bus = FileChooserBus::new(connection.clone(), move |call| delivering.deliver(call));
    match bus.export() {
        Ok(()) => Some(bus),
        Err(error) => {
            glib::g_warning!(ox_core::LOG_DOMAIN, "{error}");
            None
        }
    }
}

/// Keeps a D-Bus-activated instance (`arguments` holds
/// `--gapplication-service`) running for [`SERVICE_LINGER`] without a
/// window, rather than quitting before the request that started it, and
/// makes it wait for the desktop portal instead of starting it.
pub(super) fn linger_as_service(application: &impl IsA<gio::Application>, arguments: &[String]) {
    if arguments.iter().any(|argument| argument == SERVICE_OPTION) {
        let milliseconds = u32::try_from(SERVICE_LINGER.as_millis()).unwrap_or(u32::MAX);
        application.set_inactivity_timeout(milliseconds);
        // The portal may be what started this instance, and still be
        // starting: wait for it rather than start a second one.
        crate::theme::system::wait_for_portal_before_reading();
    }
}
