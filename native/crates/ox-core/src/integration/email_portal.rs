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
//! - **Paths the email app reads as written.** GTK's portal backend puts
//!   each path into a `mailto:` address unescaped, so a file whose path
//!   has a character that address would read otherwise is refused (see
//!   [`mailto_safe`]).
//! - **Only where it works.** The portal exports the interface only when
//!   one of the desktop's portal backends implements it, so reading its
//!   `version` property tells whether the desktop can share by email; it
//!   must be at least [`ATTACHMENTS_VERSION`], which brought attachments.
//!
//! The portal answers with a `Response` signal on a request object, as
//! the Background portal does ([`super::request_autostart`]); the
//! subscription starts before the call, so an answer that comes at once
//! is not missed.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_channel::mpsc;
use futures_util::StreamExt;
use gio::prelude::*;
use rustix::fs::OFlags;

use super::background_portal::DESKTOP_PORTAL_PATH;
use crate::private_storage::KernelOpenFlags;

/// The portal interface that opens a new email.
pub const EMAIL_INTERFACE: &str = "org.freedesktop.portal.Email";

/// The first version of the Email portal that takes `attachment_fds`
/// (xdg-desktop-portal 0.9); version 1 has no attachments, so it would
/// open an email without the files.
pub const ATTACHMENTS_VERSION: u32 = 2;

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
    /// The path of the file, named by its name, is not [`mailto_safe`].
    #[error("{}", crate::i18n::format_message("“{name}” cannot be attached to an email because its path has &, #, ?, % or a control character, which the email app could misread. Rename or move it, then share it again.", &[("name", .0)]))]
    UnsafePath(String),
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
/// worker thread. Only a regular file is attached: the check runs on the
/// opened file, which `O_NONBLOCK` opens without waiting for a pipe's
/// writer.
///
/// # Errors
///
/// [`EmailError::Unreadable`] when a file cannot be opened or is not a
/// regular file, and [`EmailError::UnsafePath`] when its path is not
/// [`mailto_safe`].
pub fn open_attachments(paths: &[PathBuf]) -> Result<Vec<File>, EmailError> {
    paths.iter().map(|path| open_attachment(path)).collect()
}

/// Opens the regular file at `path` read-only.
fn open_attachment(path: &Path) -> Result<File, EmailError> {
    let file = OpenOptions::new()
        .read(true)
        .kernel_flags(OFlags::NONBLOCK)
        .open(path)
        .map_err(EmailError::Unreadable)?;
    let is_file = file.metadata().map_err(EmailError::Unreadable)?.is_file();
    if !is_file {
        return Err(EmailError::Unreadable(std::io::ErrorKind::InvalidInput.into()));
    }
    // The path the portal names: it reads the descriptor's link in /proc,
    // where symbolic links are already followed.
    let resolved =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(EmailError::Unreadable)?;
    if !mailto_safe(&resolved) {
        let name = path.file_name().unwrap_or(path.as_os_str());
        return Err(EmailError::UnsafePath(name.to_string_lossy().into_owned()));
    }
    Ok(file)
}

/// Whether `path` reads the same inside a `mailto:` address. GTK's portal
/// backend (`xdg-desktop-portal-gtk`'s `src/email.c`, unchanged through
/// 1.15.3) appends each path after `&attachment=` without escaping it, so
/// `&` would start another field, such as `bcc=` adding a recipient, `#`
/// or `?` would cut the path short, `%` would be decoded and a control
/// character has no place in an address.
fn mailto_safe(path: &Path) -> bool {
    !path
        .as_os_str()
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'&' | b'#' | b'?' | b'%') || byte.is_ascii_control())
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

    /// A folder, or a pipe put where a Recent file was, is refused, and
    /// opening the pipe does not wait for a writer.
    #[test]
    fn only_regular_files_are_attached_and_a_pipe_does_not_block() {
        let folder = tempfile::tempdir().expect("a folder");
        let pipe = folder.path().join("queue");
        let made = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .expect("mkfifo runs");
        assert!(made.success(), "a pipe");

        let folder_path = folder.path().to_owned();
        assert!(matches!(
            open_attachments(&[folder_path]),
            Err(EmailError::Unreadable(_))
        ));
        assert!(matches!(
            open_attachments(&[pipe]),
            Err(EmailError::Unreadable(_))
        ));
    }

    /// The name, a folder or the target of a link with a character the
    /// `mailto:` address would misread refuses the file by its name; a
    /// link whose own name has one but whose target has none is fine, as
    /// the portal names the target.
    #[test]
    fn a_path_the_email_address_would_misread_is_refused_by_name() {
        let folder = tempfile::tempdir().expect("a folder");
        let refused_as = |path: &Path| match open_attachments(&[path.to_owned()]) {
            Err(EmailError::UnsafePath(name)) => Some(name),
            _ => None,
        };
        for name in [
            "report&bcc=someone@example.invalid",
            "minutes#2.txt",
            "why?.txt",
            "100%.txt",
            "two\nlines.txt",
        ] {
            let path = folder.path().join(name);
            std::fs::write(&path, "plan").expect("a file");
            assert_eq!(refused_as(&path).as_deref(), Some(name));
        }
        let team = folder.path().join("R&D");
        std::fs::create_dir(&team).expect("a folder");
        std::fs::write(team.join("plan.txt"), "plan").expect("a file");
        assert_eq!(refused_as(&team.join("plan.txt")).as_deref(), Some("plan.txt"));

        let link = folder.path().join("minutes.txt");
        std::os::unix::fs::symlink(folder.path().join("minutes#2.txt"), &link).expect("a link");
        assert_eq!(refused_as(&link).as_deref(), Some("minutes.txt"));
        let plain = folder.path().join("plan.txt");
        std::fs::write(&plain, "plan").expect("a file");
        let odd_link = folder.path().join("plan&more.txt");
        std::os::unix::fs::symlink(&plain, &odd_link).expect("a link");
        assert!(open_attachments(&[plain, odd_link]).is_ok());
    }
}
