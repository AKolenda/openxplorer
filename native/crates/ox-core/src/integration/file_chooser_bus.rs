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
//! Likewise a handle's `Close` is accepted only from the portal connection
//! that made the request, so another program that guesses the handle path
//! cannot close the user's dialog.
//!
//! **The bus is answered from a thread of its own.** `GDBus` answers an
//! object's calls, its `Properties.GetAll` included, on the main context
//! that registered it. The desktop portal starts its backends while it
//! starts, and loads each backend's properties then; a GTK application's
//! startup meanwhile waits for the portal (`GtkApplication` asks for its
//! inhibit interface with a blocking call on desktops without a GNOME or
//! Xfce session manager). Registered on the main thread, each would wait
//! for the other until their D-Bus timeouts, delaying every portal at
//! login. So the object is registered on a dispatch thread with its own
//! main loop, which answers property requests at once; a method call
//! crosses to the main thread as plain data ([`IncomingCall`]) and its
//! reply comes back the same way, so nothing that is not thread-safe
//! leaves its thread.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use futures_channel::{mpsc as call_queue, oneshot};
use futures_util::StreamExt;
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

/// The D-Bus error for a method a handle object does not have.
const UNKNOWN_METHOD_ERROR: &str = "org.freedesktop.DBus.Error.UnknownMethod";

/// The D-Bus error for a call the app could not show.
const FAILED_ERROR: &str = "org.freedesktop.DBus.Error.Failed";

/// The name of the dispatch thread.
const DISPATCH_THREAD: &str = "ox-file-chooser";

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

/// A method call's outcome on its way back to the dispatch thread: the
/// reply's body, or a D-Bus error's name and message.
type Outcome = Result<glib::Variant, (&'static str, String)>;

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
    /// The unique bus name of the portal connection that made the request,
    /// the only one whose `Close` is accepted.
    requester: String,
    /// Takes the reply back to the dispatch thread, until it is sent.
    responder: RefCell<Option<oneshot::Sender<Outcome>>>,
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
        self.state.responder.borrow().is_none()
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
        let Some(responder) = self.responder.take() else {
            return;
        };
        let (response, results) = self.request.reply(answer);
        let body = glib::Variant::tuple_from_iter([response.to_variant(), results]);
        // The dispatch thread may be gone while the app quits; the caller
        // then sees the connection close, which answers it too.
        let _ = responder.send(Ok(body));
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

/// A method call as it crosses from the dispatch thread: only data and
/// the channel its outcome goes back on.
struct IncomingCall {
    sender: Option<String>,
    method: String,
    parameters: glib::Variant,
    responder: oneshot::Sender<Outcome>,
}

/// The dispatch thread: its main loop, which unexporting stops.
struct Dispatch {
    registration: gio::RegistrationId,
    main_loop: glib::MainLoop,
    thread: Option<JoinHandle<()>>,
}

/// The backend on one bus connection.
pub struct FileChooserBus {
    connection: gio::DBusConnection,
    handler: Rc<CallHandler>,
    dispatch: Option<Dispatch>,
}

impl fmt::Debug for FileChooserBus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileChooserBus")
            .field("connection", &self.connection)
            .field("is_exported", &self.dispatch.is_some())
            .finish_non_exhaustive()
    }
}

impl FileChooserBus {
    /// A backend on `connection` that is not exported yet; `handler`
    /// receives every call that passed the checks, on the main context
    /// that is the thread default when [`Self::export`] is called.
    pub fn new(
        connection: gio::DBusConnection,
        handler: impl Fn(ChooserCall) -> Result<(), ChooserNotShown> + 'static,
    ) -> Self {
        Self {
            connection,
            handler: Rc::new(handler),
            dispatch: None,
        }
    }

    /// Whether the object is exported.
    pub fn is_exported(&self) -> bool {
        self.dispatch.is_some()
    }

    /// Exports the interface at [`PORTAL_BACKEND_PATH`] from the dispatch
    /// thread, which this starts, and serves the calls on the calling
    /// thread's default main context. Exporting again does nothing.
    ///
    /// # Errors
    ///
    /// [`ChooserRegistrationFailed`] when another object holds the path on
    /// this connection, or the thread cannot start.
    pub fn export(&mut self) -> Result<(), ChooserRegistrationFailed> {
        if self.dispatch.is_some() {
            return Ok(());
        }
        let (calls, mut incoming) = call_queue::unbounded::<IncomingCall>();
        let dispatch = start_dispatch(&self.connection, calls)?;
        let handler = Rc::clone(&self.handler);
        let connection = self.connection.clone();
        glib::spawn_future_local(async move {
            while let Some(call) = incoming.next().await {
                glib::spawn_future_local(answer(connection.clone(), call, Rc::clone(&handler)));
            }
        });
        self.dispatch = Some(dispatch);
        Ok(())
    }

    /// Removes the object and stops the dispatch thread. Calls already
    /// shown keep their windows; their replies are dropped with the thread.
    pub fn unexport(&mut self) {
        let Some(mut dispatch) = self.dispatch.take() else {
            return;
        };
        // The ID came from this connection and was not unregistered yet,
        // so this cannot fail; GDBus allows it from any thread.
        let _ = self.connection.unregister_object(dispatch.registration);
        dispatch.main_loop.quit();
        if let Some(thread) = dispatch.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FileChooserBus {
    fn drop(&mut self) {
        self.unexport();
    }
}

/// Starts the dispatch thread, registers the object there and waits for
/// the outcome. The thread forwards each method call to `calls` and
/// returns its outcome to the caller when it comes back.
fn start_dispatch(
    connection: &gio::DBusConnection,
    calls: call_queue::UnboundedSender<IncomingCall>,
) -> Result<Dispatch, ChooserRegistrationFailed> {
    let (ready, registered) = mpsc::channel::<Result<(gio::RegistrationId, glib::MainLoop), glib::Error>>();
    let connection = connection.clone();
    let thread = std::thread::Builder::new()
        .name(DISPATCH_THREAD.to_owned())
        .spawn(move || {
            let context = glib::MainContext::new();
            let main_loop = glib::MainLoop::new(Some(&context), false);
            let _ = context.with_thread_default(|| {
                let registration = register_forwarding(&connection, calls);
                let is_registered = registration.is_ok();
                let _ = ready.send(registration.map(|id| (id, main_loop.clone())));
                if is_registered {
                    main_loop.run();
                }
            });
        })
        .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string()))?;
    match registered.recv() {
        Ok(Ok((registration, main_loop))) => Ok(Dispatch {
            registration,
            main_loop,
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error.into())
        }
        Err(_) => Err(glib::Error::new(gio::IOErrorEnum::Failed, "the dispatch thread stopped").into()),
    }
}

/// Registers the interface on the dispatch thread, forwarding calls.
///
/// # Panics
///
/// Never: the interface description is a constant that declares the
/// interface.
fn register_forwarding(
    connection: &gio::DBusConnection,
    calls: call_queue::UnboundedSender<IncomingCall>,
) -> Result<gio::RegistrationId, glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(INTERFACE_XML)?;
    let interface = node
        .lookup_interface(FILE_CHOOSER_INTERFACE)
        .expect("INTERFACE_XML declares the FileChooser interface");
    connection
        .register_object(PORTAL_BACKEND_PATH, &interface)
        .method_call(move |_, sender, _, _, method, parameters, invocation| {
            let (responder, outcome) = oneshot::channel::<Outcome>();
            let call = IncomingCall {
                sender: sender.map(str::to_owned),
                method: method.to_owned(),
                parameters,
                responder,
            };
            if calls.unbounded_send(call).is_err() {
                invocation.return_dbus_error(FAILED_ERROR, &ChooserNotShown.to_string());
                return;
            }
            glib::spawn_future_local(async move {
                match outcome.await {
                    Ok(Ok(body)) => invocation.return_value(Some(&body)),
                    Ok(Err((name, message))) => invocation.return_dbus_error(name, &message),
                    Err(_) => invocation.return_dbus_error(FAILED_ERROR, &ChooserNotShown.to_string()),
                }
            });
        })
        .build()
}

/// Checks the sender and the arguments of `call`, exports its handle
/// object and hands it to `handler`; refuses with a D-Bus error otherwise.
async fn answer(connection: gio::DBusConnection, call: IncomingCall, handler: Rc<CallHandler>) {
    let IncomingCall {
        sender,
        method,
        parameters,
        responder,
    } = call;
    let refuse = |responder: oneshot::Sender<Outcome>, name: &'static str, message: String| {
        let _ = responder.send(Err((name, message)));
    };
    let requester = match sender {
        Some(sender) if comes_from_portal(&connection, Some(&sender)).await => sender,
        _ => {
            refuse(
                responder,
                ACCESS_DENIED_ERROR,
                "Only the desktop portal may open OpenXplorer's file dialogs.".to_owned(),
            );
            return;
        }
    };
    // GDBus has already checked the argument types against INTERFACE_XML.
    let Some((handle, app_id, parent_window, title, options)) = parameters.get::<(
        glib::variant::ObjectPath,
        String,
        String,
        String,
        glib::VariantDict,
    )>() else {
        refuse(responder, INVALID_ARGS_ERROR, "Unexpected arguments.".to_owned());
        return;
    };
    let request = match ChooserRequest::from_call(&method, &title, &options) {
        Ok(request) => request,
        Err(refusal) => {
            refuse(responder, INVALID_ARGS_ERROR, refusal.to_string());
            return;
        }
    };
    let state = Rc::new(ReplyState {
        request: request.clone(),
        connection: connection.clone(),
        requester,
        responder: RefCell::new(Some(responder)),
        handle: RefCell::new(None),
        on_close: RefCell::new(None),
    });
    match export_handle(&connection, handle.as_str(), &state) {
        Ok(registration) => {
            state.handle.replace(Some(registration));
        }
        Err(error) => {
            if let Some(responder) = state.responder.take() {
                refuse(responder, FAILED_ERROR, error.to_string());
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
        if let Some(responder) = state.responder.take() {
            refuse(responder, FAILED_ERROR, failure.to_string());
        }
        if let Some(handle) = state.handle.take() {
            let _ = connection.unregister_object(handle);
        }
    }
}

/// Whether `sender` owns `org.freedesktop.portal.Desktop`, asked of the
/// bus daemon without starting any service.
async fn comes_from_portal(connection: &gio::DBusConnection, sender: Option<&str>) -> bool {
    let Some(sender) = sender else {
        return false;
    };
    let reply = connection
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

/// Exports the call's `Request` object at `handle`; its `Close` ends the
/// call when it comes from the portal connection that made the request,
/// and is refused with an access error from anyone else. The object holds
/// the state weakly, so it never keeps a finished call alive.
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
        .method_call(move |_, sender, _, _, method, _, invocation| {
            if method != "Close" {
                invocation.return_dbus_error(UNKNOWN_METHOD_ERROR, "The request has no such method.");
                return;
            }
            let Some(state) = state.upgrade() else {
                invocation.return_value(None);
                return;
            };
            if sender != Some(state.requester.as_str()) {
                invocation.return_dbus_error(
                    ACCESS_DENIED_ERROR,
                    "Only the desktop portal that opened this dialog may close it.",
                );
                return;
            }
            invocation.return_value(None);
            state.close();
        })
        .build()
}
