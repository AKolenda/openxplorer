// SPDX-License-Identifier: AGPL-3.0-only
//! Set as background through the desktop's Wallpaper portal, as Windows
//! 11 File Explorer's command bar offers it for a selected picture.
//!
//! `org.freedesktop.portal.Wallpaper.SetWallpaperFile` takes the picture
//! as a read-only file descriptor; the desktop shows its own preview and
//! asks the user, so nothing changes until the user agrees there. The
//! portal exports the interface only when one of the desktop's backends
//! implements it, so reading its `version` tells whether the desktop can
//! set a background. The answer is matched by the request's own object
//! path, from any sender, as Share's is ([`super::compose_email`]).

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_channel::mpsc;
use futures_util::StreamExt;
use gio::prelude::*;

use super::background_portal::DESKTOP_PORTAL_PATH;
use super::email_portal::request_path;

/// The Wallpaper portal's interface.
pub const WALLPAPER_INTERFACE: &str = "org.freedesktop.portal.Wallpaper";

const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
/// `Response` codes 0 (done) and 1 (cancelled by the user) are no failure.
const RESPONSE_FAILED: u32 = 2;
const CALL_TIMEOUT_MS: i32 = 25_000;
/// How long the user may take in the desktop's own preview.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);

static NEXT_TOKEN: AtomicU32 = AtomicU32::new(0);

/// Why the background was not set. `Display` is the message the window
/// shows.
#[derive(Debug, thiserror::Error)]
pub enum WallpaperError {
    /// The picture could not be opened.
    #[error("{}", crate::i18n::gettext("The picture could not be opened."))]
    Unreadable(#[source] std::io::Error),
    /// The portal could not be called.
    #[error("{}", crate::i18n::gettext("The desktop background could not be set."))]
    Unavailable(#[source] glib::Error),
    /// The portal answered that it could not set it.
    #[error("{}", crate::i18n::gettext("The desktop background could not be set."))]
    Failed,
}

/// The version of the Wallpaper portal `portal` serves; `None` when the
/// desktop has no backend for it.
pub async fn wallpaper_portal_version(connection: &gio::DBusConnection, portal: &str) -> Option<u32> {
    let reply = connection
        .call_future(
            Some(portal),
            DESKTOP_PORTAL_PATH,
            PROPERTIES_INTERFACE,
            "Get",
            Some(&(WALLPAPER_INTERFACE, "version").to_variant()),
            Some(&<(glib::Variant,)>::static_variant_type()),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
        )
        .await
        .ok()?;
    reply.child_value(0).as_variant()?.get::<u32>()
}

/// Opens the picture at `path` read-only, never following a link put in
/// its place and never waiting on a pipe; blocks, so it runs on a worker
/// thread.
///
/// # Errors
///
/// [`WallpaperError::Unreadable`] when it cannot be opened or is not a
/// regular file.
pub fn open_picture(path: &Path) -> Result<File, WallpaperError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc_nonblock())
        .open(path)
        .map_err(WallpaperError::Unreadable)?;
    let metadata = file.metadata().map_err(WallpaperError::Unreadable)?;
    if !metadata.is_file() {
        return Err(WallpaperError::Unreadable(
            std::io::ErrorKind::InvalidInput.into(),
        ));
    }
    Ok(file)
}

/// `O_NONBLOCK`, so a FIFO put in the picture's place is not waited on.
fn libc_nonblock() -> i32 {
    i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or(0)
}

/// Asks the desktop to make `picture` its background (`set-on`
/// "background"), showing its own preview first.
///
/// # Errors
///
/// [`WallpaperError::Unavailable`] when the portal cannot be called and
/// [`WallpaperError::Failed`] when it answers that it could not. The user
/// cancelling in the preview is no error.
pub async fn set_wallpaper_file(
    connection: &gio::DBusConnection,
    portal: &str,
    picture: &File,
) -> Result<(), WallpaperError> {
    let token = format!(
        "openxplorer_wallpaper{}",
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    );
    let fd_list = gio::UnixFDList::new();
    let handle = fd_list.append(picture).map_err(WallpaperError::Unavailable)?;
    let (sender, mut responses) = mpsc::unbounded();
    let expected = request_path(connection.unique_name().as_deref().unwrap_or_default(), &token);
    let _subscription = connection.subscribe_to_signal(
        None,
        Some(REQUEST_INTERFACE),
        Some("Response"),
        None,
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let _ = sender.unbounded_send((signal.object_path.to_owned(), signal.parameters.clone()));
        },
    );
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", token.as_str());
    options.insert("show-preview", true);
    options.insert("set-on", "background");
    let (reply, _) = connection
        .call_with_unix_fd_list_future(
            Some(portal),
            DESKTOP_PORTAL_PATH,
            WALLPAPER_INTERFACE,
            "SetWallpaperFile",
            Some(&(String::new(), glib::variant::Handle(handle), options).to_variant()),
            Some(&<(glib::variant::ObjectPath,)>::static_variant_type()),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            Some(&fd_list),
        )
        .await
        .map_err(WallpaperError::Unavailable)?;
    let handle = reply.child_value(0).str().unwrap_or_default().to_owned();
    let answer = async {
        while let Some((path, parameters)) = responses.next().await {
            if path == handle || path == expected {
                let code = parameters.try_child_value(0).and_then(|code| code.get::<u32>());
                return match code {
                    Some(code) if code < RESPONSE_FAILED => Ok(()),
                    _ => Err(WallpaperError::Failed),
                };
            }
        }
        Err(WallpaperError::Failed)
    };
    glib::future_with_timeout(ANSWER_TIMEOUT, answer)
        .await
        .unwrap_or(Err(WallpaperError::Failed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_has_the_portal_signature() {
        let options = glib::VariantDict::new(None);
        let parameters = (String::new(), glib::variant::Handle(0), options).to_variant();
        assert_eq!(parameters.type_().as_str(), "(sha{sv})");
    }
}
