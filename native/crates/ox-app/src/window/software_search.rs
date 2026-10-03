// SPDX-License-Identifier: AGPL-3.0-only
//! "Find an app in Software": when no application opens a file type, the
//! failure dialog offers a search for one (OPEN-010).
//!
//! GNOME Software is asked over D-Bus to search for the content type, as
//! `gnome-software --search` does, through its `search` action; no command
//! line is built. The button shows only while Software is installed.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::OpenError;

/// The failure dialog's button.
pub(super) const FIND_IN_SOFTWARE: &str = crate::i18n::message_id("Find an app in Software");

/// GNOME Software's desktop ID, which is also its bus name.
const SOFTWARE_ID: &str = "org.gnome.Software";

/// How long Software may take to take the request.
const CALL_TIMEOUT_MS: i32 = 10_000;

/// The content type to search for when `reason` says no application opens
/// the file of type `content_type`.
pub(super) fn unhandled_type(reason: &str, content_type: Option<&str>) -> Option<String> {
    let no_application = reason == OpenError::NoApplication.to_string();
    no_application.then(|| content_type.map(str::to_owned)).flatten()
}

/// Whether GNOME Software is installed.
pub(super) fn is_available() -> bool {
    crate::integration::installed_application(&format!("{SOFTWARE_ID}.desktop")).is_some()
}

/// Asks GNOME Software to search for applications that open
/// `content_type`, starting it when it does not run.
///
/// # Errors
///
/// D-Bus's reason when Software could not be reached.
pub(super) async fn search_software(content_type: &str) -> Result<(), glib::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPEN-010
    #[test]
    fn only_a_type_without_an_application_is_searched_for() {
        let none = OpenError::NoApplication.to_string();
        assert_eq!(
            unhandled_type(&none, Some("application/x-blender")),
            Some("application/x-blender".to_owned())
        );
        assert_eq!(unhandled_type(&none, None), None);
        let other = OpenError::NeedsLocalPath.to_string();
        assert_eq!(unhandled_type(&other, Some("application/pdf")), None);
    }
}
