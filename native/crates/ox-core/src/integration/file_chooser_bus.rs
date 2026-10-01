// SPDX-License-Identifier: AGPL-3.0-only
//! The `org.freedesktop.impl.portal.FileChooser` backend, which shows
//! other applications' Open and Save dialogs in the app.
//!
//! New in the native app (INT-032). The object is exported at
//! [`PORTAL_BACKEND_PATH`] on the application's own bus name, which the
//! packaged `.portal` file names; the desktop portal sends calls there only
//! after the user picked `OpenXplorer` for file dialogs
//! ([`FileDialogRegistration`](super::FileDialogRegistration)). Each call
//! is answered later, when the window reports the user's choice through
//! its [`ChooserReply`]; meanwhile the call's
//! `org.freedesktop.impl.portal.Request` object at the caller's handle
//! lets the portal close the dialog.
//!
//! Safety rule "only the portal may ask": a call is served only when its
//! sender owns `org.freedesktop.portal.Desktop`, so another program on the
//! session bus cannot pop up dialogs or read back what the user chose.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use gio::prelude::*;

use super::background_portal::DESKTOP_PORTAL_NAME;
use super::file_chooser_request::{ChooserAnswer, ChooserRequest, FILE_CHOOSER_INTERFACE};

/// Where every portal backend exports its interfaces.
pub const PORTAL_BACKEND_PATH: &str = "/org/freedesktop/portal/desktop";

/// The interface of a call's handle object.
const REQUEST_INTERFACE: &str = "org.freedesktop.impl.portal.Request";

/// The backend interface, as `org.freedesktop.impl.portal.FileChooser.xml`
/// declares it.
const INTERFACE_XML: &str = r#"<node><interface name="org.freedesktop.impl.portal.FileChooser">
<method name="OpenFile"><arg type="o" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="u" direction="out"/><arg type="a{sv}" direction="out"/></method>
<method name="SaveFile"><arg type="o" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="u" direction="out"/><arg type="a{sv}" direction="out"/></method>
<method name="SaveFiles"><arg type="o" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="u" direction="out"/><arg type="a{sv}" direction="out"/></method>
</interface></node>"#;

/// The handle object's interface.
const REQUEST_XML: &str = r#"<node><interface name="org.freedesktop.impl.portal.Request">
<method name="Close"/>
</interface></node>"#;

/// How long asking the bus daemon who the portal is may take, in
/// milliseconds.
const OWNER_TIMEOUT_MS: i32 = 2000;

/// The longest application ID or parent window kept, in characters.
const MAX_ID_CHARS: usize = 512;

/// The D-Bus error for a sender that is not the portal.
const ACCESS_DENIED_ERROR: &str = "org.freedesktop.DBus.Error.AccessDenied";

/// The D-Bus error for a refused call.
const INVALID_ARGS_ERROR: &str = "org.freedesktop.DBus.Error.InvalidArgs";

/// The D-Bus error for a call the app could not show.
const FAILED_ERROR: &str = "org.freedesktop.DBus.Error.Failed";

/// The backend could not be exported.
#[derive(Debug, thiserror::Error)]
#[error("The file dialog service could not be registered: {0}")]
pub struct ChooserRegistrationFailed(#[from] glib::Error);

/// The app could not show a dialog, for example while quitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("OpenXplorer could not show the file dialog.")]
pub struct ChooserNotShown;

/// One call the window should show.
#[derive(Debug)]
pub struct ChooserCall {
    /// The checked request.
    pub request: ChooserRequest,
    /// The calling application's ID as the portal reports it; empty for an
    /// unsandboxed caller.
    pub app_id: String,
    /// The caller's window, `x11:<id>` or `wayland:<handle>`, possibly
    /// empty.
    pub parent_window: String,
    /// Where the window sends the answer.
    pub reply: ChooserReply,
}

/// What the app does with a call: shows it, or refuses at once.
type CallHandler = dyn Fn(ChooserCall) -> Result<(), ChooserNotShown>;

/// The answer channel of one call. Clones share it; the first
/// [`ChooserReply::send`] answers and later ones do nothing. If every
/// clone is dropped unanswered, the call ends with "other", so the
/// calling application never waits forever.
#[derive(Clone)]
pub struct ChooserReply {
    state: Rc<ReplyState>,
}

impl fmt::Debug for ChooserReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChooserReply")
            .field("is_answered", &self.is_answered())
            .finish_non_exhaustive()
    }
}

/// The state behind a [`ChooserReply`].
struct ReplyState {
    request: ChooserRequest,
    connection: gio::DBusConnection,
    invocation: RefCell<Option<gio::DBusMethodInvocation>>,
    handle: RefCell<Option<gio::RegistrationId>>,
    on_close: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl ChooserReply {
    /// Answers the call with `answer`, once.
    pub fn send(&self, answer: &ChooserAnswer) {
        self.state.answer(answer);
    }

    /// Whether the call has been answered.
    pub fn is_answered(&self) -> bool {
        self.state.invocation.borrow().is_none()
    }

    /// Runs `on_close` when the portal closes the dialog (its `Request`'s
    /// `Close`). The call has then been answered with "other".
    pub fn connect_closed(&self, on_close: impl FnOnce() + 'static) {
        self.state.on_close.replace(Some(Box::new(on_close)));
    }
}

impl ReplyState {
    /// Sends the reply and removes the handle object, if not done yet.
    fn answer(&self, answer: &ChooserAnswer) {
        let Some(invocation) = self.invocation.take() else {
            return;
        };
        let (response, results) = self.request.reply(answer);
        invocation.return_value(Some(&glib::Variant::tuple_from_iter([
            response.to_variant(),
            results,
        ])));
        if let Some(handle) = self.handle.take() {
            // The ID came from this connection and was not unregistered
            // yet, so this cannot fail.
            let _ = self.connection.unregister_object(handle);
        }
    }

    /// The portal's `Close`: ends the call, then lets the window go.
    fn close(&self) {
        self.answer(&ChooserAnswer::Ended);
        let on_close = self.on_close.take();
        if let Some(on_close) = on_close {
            on_close();
        }
    }
}

impl Drop for ReplyState {
    /// A dropped, unanswered call ends with "other".
    fn drop(&mut self) {
        self.answer(&ChooserAnswer::Ended);
    }
}

/// The backend on one bus connection.
pub struct FileChooserBus {
    connection: gio::DBusConnection,
    handler: Rc<CallHandler>,
    registration: Option<gio::RegistrationId>,
}

impl fmt::Debug for FileChooserBus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileChooserBus")
            .field("connection", &self.connection)
            .field("is_exported", &self.registration.is_some())
            .finish_non_exhaustive()
    }
}

impl FileChooserBus {
    /// A backend on `connection` that is not exported yet; `handler`
    /// receives every call that passed the checks.
    pub fn new(
        connection: gio::DBusConnection,
        handler: impl Fn(ChooserCall) -> Result<(), ChooserNotShown> + 'static,
    ) -> Self {
        Self {
            connection,
            handler: Rc::new(handler),
            registration: None,
        }
    }

    /// Whether the object is exported.
    pub fn is_exported(&self) -> bool {
        self.registration.is_some()
    }

    /// Exports the interface at [`PORTAL_BACKEND_PATH`]. Exporting again
    /// does nothing.
    ///
    /// # Errors
    ///
    /// [`ChooserRegistrationFailed`] when another object holds the path on
    /// this connection.
    ///
    /// # Panics
    ///
    /// Never: the interface description is a constant that declares the
    /// interface.
    pub fn export(&mut self) -> Result<(), ChooserRegistrationFailed> {
        if self.registration.is_some() {
            return Ok(());
        }
        let node = gio::DBusNodeInfo::for_xml(INTERFACE_XML)?;
        let interface = node
            .lookup_interface(FILE_CHOOSER_INTERFACE)
            .expect("INTERFACE_XML declares the FileChooser interface");
        let handler = Rc::clone(&self.handler);
        let registration = self
            .connection
            .register_object(PORTAL_BACKEND_PATH, &interface)
            .method_call(move |connection, sender, _, _, method, parameters, invocation| {
                let call = IncomingCall {
                    connection,
                    sender: sender.map(str::to_owned),
                    method: method.to_owned(),
                    parameters,
                    invocation,
                };
                glib::spawn_future_local(call.answer(Rc::clone(&handler)));
            })
            .build()?;
        self.registration = Some(registration);
        Ok(())
    }

    /// Removes the object. Calls already shown keep their replies.
    pub fn unexport(&mut self) {
        if let Some(registration) = self.registration.take() {
            // The ID came from this connection and was not unregistered
            // yet, so this cannot fail.
            let _ = self.connection.unregister_object(registration);
        }
    }
}

impl Drop for FileChooserBus {
    fn drop(&mut self) {
        self.unexport();
    }
}

/// One method call on its way to the handler.
struct IncomingCall {
    connection: gio::DBusConnection,
    sender: Option<String>,
    method: String,
    parameters: glib::Variant,
    invocation: gio::DBusMethodInvocation,
}

impl IncomingCall {
    /// Checks the sender and the arguments, exports the handle object and
    /// hands the call to `handler`; refuses with a D-Bus error otherwise.
    async fn answer(self, handler: Rc<CallHandler>) {
        if !self.comes_from_portal().await {
            self.invocation.return_dbus_error(
                ACCESS_DENIED_ERROR,
                "Only the desktop portal may open OpenXplorer's file dialogs.",
            );
            return;
        }
        // GDBus has already checked the argument types against INTERFACE_XML.
        let Some((handle, app_id, parent_window, title, options)) = self.parameters.get::<(
            glib::variant::ObjectPath,
            String,
            String,
            String,
            glib::VariantDict,
        )>() else {
            self.invocation
                .return_dbus_error(INVALID_ARGS_ERROR, "Unexpected arguments.");
            return;
        };
        let request = match ChooserRequest::from_call(&self.method, &title, &options) {
            Ok(request) => request,
            Err(refusal) => {
                self.invocation
                    .return_dbus_error(INVALID_ARGS_ERROR, &refusal.to_string());
                return;
            }
        };
        let state = Rc::new(ReplyState {
            request: request.clone(),
            connection: self.connection.clone(),
            invocation: RefCell::new(Some(self.invocation)),
            handle: RefCell::new(None),
            on_close: RefCell::new(None),
        });
        match export_handle(&self.connection, handle.as_str(), &state) {
            Ok(registration) => {
                state.handle.replace(Some(registration));
            }
            Err(error) => {
                if let Some(invocation) = state.invocation.take() {
                    invocation.return_dbus_error(FAILED_ERROR, &error.to_string());
                }
                return;
            }
        }
        let call = ChooserCall {
            request,
            app_id: app_id.chars().take(MAX_ID_CHARS).collect(),
            parent_window: parent_window.chars().take(MAX_ID_CHARS).collect(),
            reply: ChooserReply {
                state: Rc::clone(&state),
            },
        };
        if let Err(failure) = handler(call) {
            if let Some(invocation) = state.invocation.take() {
                invocation.return_dbus_error(FAILED_ERROR, &failure.to_string());
            }
            if let Some(handle) = state.handle.take() {
                let _ = self.connection.unregister_object(handle);
            }
        }
    }

    /// Whether the sender owns `org.freedesktop.portal.Desktop`, asked of
    /// the bus daemon without starting any service.
    async fn comes_from_portal(&self) -> bool {
        let Some(sender) = self.sender.as_deref() else {
            return false;
        };
        let reply = self
            .connection
            .call_future(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                Some(&(DESKTOP_PORTAL_NAME,).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                OWNER_TIMEOUT_MS,
            )
            .await;
        reply
            .ok()
            .and_then(|reply| reply.get::<(String,)>())
            .is_some_and(|(owner,)| owner == sender)
    }
}

/// Exports the call's `Request` object at `handle`; its `Close` ends the
/// call. The object holds the state weakly, so it never keeps a finished
/// call alive.
fn export_handle(
    connection: &gio::DBusConnection,
    handle: &str,
    state: &Rc<ReplyState>,
) -> Result<gio::RegistrationId, glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(REQUEST_XML)?;
    let interface = node
        .lookup_interface(REQUEST_INTERFACE)
        .expect("REQUEST_XML declares the Request interface");
    let state = Rc::downgrade(state);
    connection
        .register_object(handle, &interface)
        .method_call(move |_, _, _, _, _, _, invocation| {
            invocation.return_value(None);
            if let Some(state) = state.upgrade() {
                state.close();
            }
        })
        .build()
}
