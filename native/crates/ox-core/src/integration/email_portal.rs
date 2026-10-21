// SPDX-License-Identifier: AGPL-3.0-only
//! Sharing files by email through the desktop's Email portal, which the
//! command bar's Share button does.
//!
//! New in the native app: the Python app had no Share command. Windows 11's
//! Share hands the selected files to an app the user picks; the Linux
//! desktops have no portal for a share menu, but every desktop portal
//! that implements `org.freedesktop.portal.Email` (KDE Plasma's, and
//! GTK's on GNOME) opens the user's own email app with files attached.
//!
//! The rules:
//!
//! - **The user sends.** `ComposeEmail` only opens a new message in the
//!   email app; nothing is sent until the user sends it there.
//! - **Descriptors, never contents.** Each file goes as a read-only file
//!   descriptor (`attachment_fds`). This module opens the files and never
//!   reads them: the portal turns each descriptor into the path it names
//!   for the email app.
//! - **Only where it works.** The portal exports the interface only when
//!   one of the desktop's portal backends implements it, so reading its
//!   `version` property tells whether the desktop can share by email.
//!
//! The portal answers with a `Response` signal on a request object, as
//! the Background portal does ([`super::request_autostart`]); the
//! subscription starts before the call, so an answer that comes at once
//! is not missed.

use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_channel::mpsc;
use futures_util::StreamExt;
use gio::prelude::*;

use super::background_portal::DESKTOP_PORTAL_PATH;

/// The portal interface that opens a new email.
pub const EMAIL_INTERFACE: &str = "org.freedesktop.portal.Email";

/// The most files one email takes: a D-Bus message carries at most 16
/// file descriptors on `dbus-daemon`'s default limits.
pub const MAX_ATTACHMENTS: usize = 16;

/// The interface that reads the portal's `version`.
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

/// The interface of the request object the answer comes from.
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";

/// The `Response` code of a request that failed; 0 is success and 1
/// means the user cancelled.
const RESPONSE_FAILED: u32 = 2;

/// How long the portal may take to accept the call.
const CALL_TIMEOUT_MS: i32 = 25_000;

/// How long the answer may take: the portal answers once the email app
/// started, or failed to.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// Numbers the requests of this process, so each answer is matched to
/// its own request.
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(0);

/// Why the files were not shared. `Display` is the message the window
/// shows.
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    /// A file could not be opened to attach it.
    #[error("{}", crate::i18n::gettext("The files could not be attached to an email."))]
    Unreadable(#[source] std::io::Error),
    /// The portal could not be called; the D-Bus error is the source,
    /// kept out of the message.
    #[error("{}", crate::i18n::gettext("The email app could not be opened."))]
    Unavailable(#[source] glib::Error),
    /// The portal answered that it could not open the email app, as it
    /// does when no email app is set up.
    #[error("{}", crate::i18n::gettext("The email app could not be opened."))]
    Failed,
    /// The portal did not answer in time.
    #[error("{}", crate::i18n::gettext("The email app could not be opened."))]
    Unanswered,
}

/// The version of the Email portal that `portal` serves on `connection`;
/// `None` when it serves none, as when no portal backend of the desktop
/// implements it.
pub async fn email_portal_version(connection: &gio::DBusConnection, portal: &str) -> Option<u32> {
    let reply = connection
        .call_future(
            Some(portal),
            DESKTOP_PORTAL_PATH,
            PROPERTIES_INTERFACE,
            "Get",
            Some(&(EMAIL_INTERFACE, "version").to_variant()),
            Some(&<(glib::Variant,)>::static_variant_type()),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
        )
        .await
        .ok()?;
    reply.child_value(0).as_variant()?.get::<u32>()
}

/// Opens each of `paths` read-only, to attach it; blocks, so it runs on a
/// worker thread.
///
/// # Errors
///
/// [`EmailError::Unreadable`] when a file cannot be opened.
pub fn open_attachments(paths: &[PathBuf]) -> Result<Vec<File>, EmailError> {
    paths
        .iter()
        .map(|path| File::open(path).map_err(EmailError::Unreadable))
        .collect()
}

/// Asks `portal` on `connection` to open a new email in the desktop's
/// email app with `attachments` attached. The user cancelling counts as
/// done.
///
/// # Errors
///
/// [`EmailError::Unavailable`] when the portal cannot be called,
/// [`EmailError::Failed`] when it could not open the email app and
/// [`EmailError::Unanswered`] when it does not answer.
pub async fn compose_email(
    connection: &gio::DBusConnection,
    portal: &str,
    attachments: &[File],
) -> Result<(), EmailError> {
    let token = format!("openxplorer_email{}", NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let fd_list = gio::UnixFDList::new();
    let mut handles = Vec::with_capacity(attachments.len());
    for file in attachments {
        handles.push(fd_list.append(file).map_err(EmailError::Unavailable)?);
    }
    let (sender, mut responses) = mpsc::unbounded();
    // Every Response of the portal, so the answer is caught even when the
    // portal names another request object than the one expected.
    let _subscription = connection.subscribe_to_signal(
        Some(portal),
        Some(REQUEST_INTERFACE),
        Some("Response"),
        None,
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let _ = sender.unbounded_send((signal.object_path.to_owned(), signal.parameters.clone()));
        },
    );
    let (reply, _) = connection
        .call_with_unix_fd_list_future(
            Some(portal),
            DESKTOP_PORTAL_PATH,
            EMAIL_INTERFACE,
            "ComposeEmail",
            Some(&(String::new(), options(&token, &handles)).to_variant()),
            Some(&<(glib::variant::ObjectPath,)>::static_variant_type()),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            Some(&fd_list),
        )
        .await
        .map_err(EmailError::Unavailable)?;
    let handle = reply.child_value(0).str().unwrap_or_default().to_owned();
    let answer = async {
        while let Some((path, parameters)) = responses.next().await {
            if path == handle {
                return answered(&parameters);
            }
        }
        // The subscription ended with the connection.
        Err(EmailError::Unanswered)
    };
    glib::future_with_timeout(ANSWER_TIMEOUT, answer)
        .await
        .unwrap_or(Err(EmailError::Unanswered))
}

/// The options of `ComposeEmail` (`a{sv}`): the request's token and the
/// attachments, as indexes into the call's file descriptor list.
fn options(token: &str, handles: &[i32]) -> glib::VariantDict {
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", token);
    let handles: Vec<glib::variant::Handle> = handles.iter().copied().map(glib::variant::Handle).collect();
    options.insert_value("attachment_fds", &handles.to_variant());
    options
}

/// What the `Response` parameters `(ua{sv})` say: success or the user
/// cancelling, else a failure.
fn answered(parameters: &glib::Variant) -> Result<(), EmailError> {
    match parameters.try_child_value(0).and_then(|code| code.get::<u32>()) {
        Some(code) if code < RESPONSE_FAILED => Ok(()),
        _ => Err(EmailError::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(code: u32) -> glib::Variant {
        (code, glib::VariantDict::new(None)).to_variant()
    }

    #[test]
    fn the_call_has_the_portal_signature() {
        let arguments = (String::new(), options("openxplorer_email1", &[0, 1])).to_variant();

        assert_eq!(arguments.type_().as_str(), "(sa{sv})");
        let options = glib::VariantDict::new(Some(&arguments.child_value(1)));
        let fds = options
            .lookup_value("attachment_fds", None)
            .expect("the attachments");
        assert_eq!(fds.type_().as_str(), "ah");
        assert_eq!(fds.n_children(), 2);
    }

    #[test]
    fn the_options_carry_only_the_token_and_the_attachments() {
        let options = options("openxplorer_email7", &[0, 1, 2]);

        assert_eq!(
            options.lookup::<String>("handle_token").ok().flatten().as_deref(),
            Some("openxplorer_email7")
        );
        assert_eq!(options.end().n_children(), 2, "no address, subject or body");
    }

    #[test]
    fn success_and_cancelling_are_done_and_a_failure_is_not() {
        assert!(answered(&response(0)).is_ok());
        assert!(answered(&response(1)).is_ok(), "the user cancelled");
        assert!(matches!(answered(&response(2)), Err(EmailError::Failed)));
        assert!(matches!(answered(&().to_variant()), Err(EmailError::Failed)));
    }

    #[test]
    fn a_missing_file_cannot_be_attached() {
        let folder = tempfile::tempdir().expect("a folder");
        let present = folder.path().join("plan.txt");
        std::fs::write(&present, "plan").expect("a file");

        assert_eq!(
            open_attachments(std::slice::from_ref(&present))
                .map(|files| files.len())
                .ok(),
            Some(1)
        );
        let missing = open_attachments(&[present, folder.path().join("gone.txt")]);
        assert!(matches!(missing, Err(EmailError::Unreadable(_))));
    }
}
