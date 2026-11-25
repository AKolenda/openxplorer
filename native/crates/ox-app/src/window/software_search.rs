// SPDX-License-Identifier: AGPL-3.0-only
//! "Find an app in Software": the app chooser offers a search for an app
//! that opens a file's type (OPEN-010), as Windows' chooser links to the
//! Microsoft Store.
//!
//! GNOME Software is asked over D-Bus to search for the content type, as
//! `gnome-software --search` does, through its `search` action; no command
//! line is built. The link shows only while Software is installed.

use gtk::prelude::*;
use gtk::{gio, glib};

/// GNOME Software's desktop ID, which is also its bus name.
const SOFTWARE_ID: &str = "org.gnome.Software";

/// How long Software may take to take the request.
const CALL_TIMEOUT_MS: i32 = 10_000;

/// Whether GNOME Software is installed.
pub(crate) fn is_available() -> bool {
    crate::integration::installed_application(&format!("{SOFTWARE_ID}.desktop")).is_some()
}

/// Asks GNOME Software to search for applications that open
/// `content_type`, starting it when it does not run.
///
/// # Errors
///
/// D-Bus's reason when Software could not be reached.
pub(crate) async fn search_software(content_type: &str) -> Result<(), glib::Error> {
    let connection = gio::bus_get_future(gio::BusType::Session).await?;
    let parameters = (
        "search",
        vec![content_type.to_variant()],
        glib::VariantDict::new(None),
    )
        .to_variant();
    connection
        .call_future(
            Some(SOFTWARE_ID),
            "/org/gnome/Software",
            "org.freedesktop.Application",
            "ActivateAction",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
        )
        .await?;
    Ok(())
}
