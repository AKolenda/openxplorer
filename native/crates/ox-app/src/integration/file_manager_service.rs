// SPDX-License-Identifier: AGPL-3.0-only
//! The `org.freedesktop.FileManager1` service while Show in folder is
//! enabled, and the application hold that keeps it answering without a
//! window.
//!
//! Ports `enable_reveal`, `disable_reveal`, the `startup` registration
//! and `handle_reveal` of `v2.0.0:desktop/winspace.py` (INT-013, INT-017). The
//! service itself, its argument checks and its error replies are
//! ox-core's [`FileManagerBus`]; requests reach the application through
//! the handler it attached, which shows them in a window.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{
    BusStatus, FileManagerBus, FileManagerRequest, RequestNotOpened, RevealRegistration,
};

use super::changes::IntegrationError;
use super::DesktopIntegration;

/// What the application does with a checked request and the caller's
/// startup ID: shows it in a window (`handle_reveal`).
pub(crate) type RequestHandler = dyn Fn(FileManagerRequest, String) -> Result<(), RequestNotOpened>;

impl DesktopIntegration {
    /// Attaches the running application: requests go to `handler`, and
    /// when Show in folder is enabled the service starts, as the Python
    /// app's `startup` does (`if self.reveal.enabled()`).
    ///
    /// # Panics
    ///
    /// When called twice: one application owns the integration.
    pub(crate) fn attach(
        &self,
        app: &impl IsA<gio::Application>,
        handler: impl Fn(FileManagerRequest, String) -> Result<(), RequestNotOpened> + 'static,
    ) {
        let imp = self.imp();
        imp.application.set(Some(app.upcast_ref()));
        let handler: Rc<RequestHandler> = Rc::new(handler);
        assert!(
            imp.request_handler.set(handler).is_ok(),
            "one application attaches the desktop integration"
        );
        self.start_service_if_enabled();
    }

    /// Starts the service if Show in folder is enabled, for startup and
    /// `--filemanager-service`. The application is held while the session
    /// files are read, so a service launch without a window does not quit
    /// before it knows whether to stay.
    pub(crate) fn start_service_if_enabled(&self) {
        let Some(app) = self.imp().application.upgrade() else {
            return;
        };
        let reading = app.hold();
        let reveal = self.services().reveal.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = integration)]
            self,
            async move {
                if reveal.run_in_background(RevealRegistration::is_enabled).await {
                    if let Err(error) = integration.start_file_manager_service() {
                        glib::g_warning!(ox_core::LOG_DOMAIN, "{error}");
                    }
                }
                drop(reading);
            }
        ));
    }

    /// Shows `request` as if it came over the bus: `--select`.
    ///
    /// # Errors
    ///
    /// [`RequestNotOpened`] when no application is attached or it could
    /// not show the request.
    pub(crate) fn show_request(&self, request: FileManagerRequest) -> Result<(), RequestNotOpened> {
        let handler = self.imp().request_handler.get().ok_or(RequestNotOpened)?;
        handler(request, String::new())
    }

    /// Owns `org.freedesktop.FileManager1` and holds the application, so
    /// it answers Show in folder with no window open. Starting again does
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::NoApplication`] before the application is
    /// attached or registered on the session bus, and
    /// [`IntegrationError::Registration`] when the object cannot be
    /// exported.
    pub(crate) fn start_file_manager_service(&self) -> Result<(), IntegrationError> {
        let imp = self.imp();
        if imp.file_manager.borrow().is_some() {
            return Ok(());
        }
        let app = imp.application.upgrade().ok_or(IntegrationError::NoApplication)?;
        let connection = app.dbus_connection().ok_or(IntegrationError::NoApplication)?;
        let handler = Rc::clone(imp.request_handler.get().ok_or(IntegrationError::NoApplication)?);
        let changed = glib::clone!(
            #[weak(rename_to = integration)]
            self,
            move || integration.notify_changed()
        );
        let mut bus = FileManagerBus::new(
            connection,
            move |request, startup_id| handler(request, startup_id),
            changed,
        );
        bus.enable()?;
        imp.file_manager.replace(Some(Rc::new(bus)));
        imp.service_hold.replace(Some(app.hold()));
        Ok(())
    }

    /// Gives up `org.freedesktop.FileManager1` and the application hold
    /// (`disable_reveal`, and quitting).
    pub(crate) fn stop_file_manager_service(&self) {
        let imp = self.imp();
        // Dropping the service releases the name and the object, at once
        // or when a status read that still holds it finishes.
        let stopped = imp.file_manager.take();
        imp.service_hold.take();
        if stopped.is_some() {
            self.notify_changed();
        }
    }

    /// Whether this app owns `org.freedesktop.FileManager1` now.
    pub(crate) fn owns_file_manager(&self) -> bool {
        self.imp()
            .file_manager
            .borrow()
            .as_ref()
            .is_some_and(|bus| bus.is_owned())
    }

    /// Who owns `org.freedesktop.FileManager1`, asked of the bus daemon
    /// without starting any service. Without its own service the app asks
    /// through an unregistered one, as the Python app's endpoint did
    /// before it was enabled.
    pub(super) async fn file_manager_status(&self) -> BusStatus {
        let running = self.imp().file_manager.borrow().clone();
        if let Some(bus) = running {
            return bus.status().await;
        }
        let Some(connection) = self.session_connection() else {
            return BusStatus::default();
        };
        let observer = FileManagerBus::new(connection, |_, _| Err(RequestNotOpened), || {});
        observer.status().await
    }

    /// The application's session-bus connection, once it is registered.
    pub(super) fn session_connection(&self) -> Option<gio::DBusConnection> {
        self.imp().application.upgrade()?.dbus_connection()
    }
}
