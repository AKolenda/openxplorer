// SPDX-License-Identifier: AGPL-3.0-only
//! Starting the app at login from inside Flatpak, through the desktop's
//! Background portal.
//!
//! New in the native app: the Python app ran only on the host, where Show
//! in folder writes its own autostart entry ([`super::reveal`]). A Flatpak
//! cannot write the host's autostart folder, so it asks
//! `org.freedesktop.portal.Background.RequestBackground`, which writes the
//! entry for the sandboxed app, or removes it, and may ask the user
//! first. The portal answers with a `Response` signal on a request
//! object; the subscription starts before the call, so an answer that
//! comes at once is not missed.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_channel::mpsc;
use futures_util::StreamExt;
use gio::prelude::*;

/// The bus name of the XDG desktop portal, which serves every portal
/// interface (Background, Settings, ...).
pub const DESKTOP_PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";

/// The object every desktop portal interface is exported at.
pub const DESKTOP_PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// The portal interface that asks to run in the background.
const BACKGROUND_INTERFACE: &str = "org.freedesktop.portal.Background";

/// The interface of the request object the answer comes from.
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";

/// The `Response` code of a request the portal granted; 1 means the user
/// cancelled, 2 any other refusal.
const RESPONSE_SUCCESS: u32 = 0;

/// How long the portal may take to accept the call.
const CALL_TIMEOUT_MS: i32 = 25_000;

/// How long the answer may take, which can wait for the user; a request
/// the portal never answers must not leave Settings waiting.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// Numbers the requests of this process, so each answer is matched to
/// its own request.
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(0);

/// Whether the app starts at login, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutostartRequest {
    /// Start the app at login (true) or stop doing so (false).
    pub autostart: bool,
    /// The command the autostart entry runs inside the sandbox, the
    /// program first.
    pub commandline: Vec<String>,
    /// Why the app asks, which the portal may show the user.
    pub reason: String,
}

/// Why the portal did not do what was asked. `Display` is the message
/// the window shows.
#[derive(Debug, thiserror::Error)]
pub enum BackgroundError {
    /// The desktop has no Background portal, or the call failed; the
    /// D-Bus error is the source, kept out of the message.
    #[error("The desktop cannot start OpenXplorer at login.")]
    Unavailable(#[source] glib::Error),
    /// The user or the desktop refused.
    #[error("The desktop did not allow OpenXplorer to start at login.")]
    Refused,
    /// The portal did not answer in time.
    #[error("The desktop did not answer whether OpenXplorer may start at login.")]
    Unanswered,
}

/// Asks `portal` on `connection` to start the app at login, or to stop
/// doing so, as `request` says.
///
/// # Errors
///
/// [`BackgroundError::Unavailable`] when the portal cannot be called,
/// [`BackgroundError::Refused`] when it answers that the app may not start
/// at login, and [`BackgroundError::Unanswered`] when it does not answer.
pub async fn request_autostart(
    connection: &gio::DBusConnection,
    portal: &str,
    request: &AutostartRequest,
) -> Result<(), BackgroundError> {
    let token = format!("openxplorer{}", NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
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
    let reply = connection
        .call_future(
            Some(portal),
            DESKTOP_PORTAL_PATH,
            BACKGROUND_INTERFACE,
            "RequestBackground",
            Some(&(String::new(), options(request, &token)).to_variant()),
            Some(&<(glib::variant::ObjectPath,)>::static_variant_type()),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
        )
        .await
        .map_err(BackgroundError::Unavailable)?;
    let handle = reply.child_value(0).str().unwrap_or_default().to_owned();
    let answer = async {
        while let Some((path, parameters)) = responses.next().await {
            if path == handle {
                return granted(request, &parameters);
            }
        }
        // The subscription ended with the connection.
        Err(BackgroundError::Refused)
    };
    glib::future_with_timeout(ANSWER_TIMEOUT, answer)
        .await
        .unwrap_or(Err(BackgroundError::Unanswered))
}

/// The options of `RequestBackground` (`a{sv}`): the request's token,
/// the reason, and whether and how to start at login.
fn options(request: &AutostartRequest, token: &str) -> glib::VariantDict {
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", token);
    options.insert("reason", request.reason.as_str());
    options.insert("autostart", request.autostart);
    options.insert("commandline", request.commandline.clone());
    options.insert("dbus-activatable", false);
    options
}

/// Whether the `Response` parameters `(ua{sv})` grant `request`: the
/// request succeeded and, when asking to start at login, the answer's
/// `autostart` is true.
fn granted(request: &AutostartRequest, parameters: &glib::Variant) -> Result<(), BackgroundError> {
    let Some((response, results)) = parameters.get::<(u32, glib::VariantDict)>() else {
        return Err(BackgroundError::Refused);
    };
    if response != RESPONSE_SUCCESS {
        return Err(BackgroundError::Refused);
    }
    let autostart = results.lookup::<bool>("autostart").ok().flatten();
    if request.autostart && autostart != Some(true) {
        return Err(BackgroundError::Refused);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(autostart: bool) -> AutostartRequest {
        AutostartRequest {
            autostart,
            commandline: vec![
                "/app/bin/openxplorer".to_owned(),
                "--filemanager-service".to_owned(),
            ],
            reason: "Answer Show in folder after login.".to_owned(),
        }
    }

    fn response(code: u32, autostart: Option<bool>) -> glib::Variant {
        let results = glib::VariantDict::new(None);
        results.insert("background", true);
        if let Some(autostart) = autostart {
            results.insert("autostart", autostart);
        }
        (code, results).to_variant()
    }

    #[test]
    fn the_call_has_the_portal_signature() {
        let arguments = (String::new(), options(&request(true), "openxplorer1")).to_variant();

        assert_eq!(arguments.type_().as_str(), "(sa{sv})");
        assert_eq!(response(0, Some(true)).type_().as_str(), "(ua{sv})");
    }

    #[test]
    fn the_options_carry_the_token_the_command_and_the_choice() {
        let options = options(&request(true), "openxplorer7");

        assert_eq!(
            options.lookup::<String>("handle_token").ok().flatten().as_deref(),
            Some("openxplorer7")
        );
        assert_eq!(options.lookup::<bool>("autostart").ok().flatten(), Some(true));
        assert_eq!(
            options.lookup::<Vec<String>>("commandline").ok().flatten(),
            Some(request(true).commandline)
        );
        assert_eq!(
            options.lookup::<bool>("dbus-activatable").ok().flatten(),
            Some(false)
        );
    }

    #[test]
    fn starting_at_login_needs_success_and_autostart() {
        assert!(granted(&request(true), &response(0, Some(true))).is_ok());
        assert!(granted(&request(true), &response(0, Some(false))).is_err());
        assert!(granted(&request(true), &response(0, None)).is_err());
        assert!(granted(&request(true), &response(1, Some(true))).is_err());
        assert!(granted(&request(true), &response(2, Some(true))).is_err());
    }

    #[test]
    fn stopping_needs_only_success() {
        assert!(granted(&request(false), &response(0, Some(false))).is_ok());
        assert!(granted(&request(false), &response(0, None)).is_ok());
        assert!(granted(&request(false), &response(1, None)).is_err());
    }
}
