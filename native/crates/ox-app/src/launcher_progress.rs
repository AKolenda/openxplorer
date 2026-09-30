// SPDX-License-Identifier: AGPL-3.0-only
//! Transfer progress on the dock or taskbar icon (INT-027).
//!
//! Docks and taskbars that follow Unity's launcher API (Dash to Dock,
//! Dash to Panel, Zorin's taskbar, Plank, KDE's task manager) draw a
//! progress bar on an app's icon when the app emits
//! `com.canonical.Unity.LauncherEntry.Update` on the session bus. GNOME
//! Shell's own dash ignores the signal, so nothing changes there. A
//! running operation's panel keeps one [`LauncherProgress`] and reports
//! whole percents only, so a fast copy does not flood the bus.

use std::cell::Cell;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::config::APP_ID;

/// The interface and member docks listen to.
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const MEMBER: &str = "Update";
/// The object the signal comes from; docks match on the app URI instead.
const OBJECT_PATH: &str = "/io/winspace/Development/LauncherEntry";

/// The progress an operation shows on the app's icon.
#[derive(Debug)]
pub(crate) struct LauncherProgress {
    /// The session connection the signal goes out on.
    connection: gio::DBusConnection,
    /// The last whole percent sent.
    percent: Cell<Option<u32>>,
}

impl LauncherProgress {
    /// Progress for the application of the window `widget` is in, when it
    /// is on the session bus.
    pub(crate) fn for_widget(widget: &impl IsA<gtk::Widget>) -> Option<Self> {
        let window = widget.root()?.downcast::<gtk::Window>().ok()?;
        let connection = window.application()?.dbus_connection()?;
        Some(Self {
            connection,
            percent: Cell::new(None),
        })
    }

    /// Shows `fraction` (0–1) on the icon, when its whole percent changed.
    pub(crate) fn show(&self, fraction: f64) {
        let percent = whole_percent(fraction);
        if self.percent.replace(Some(percent)) != Some(percent) {
            self.emit(&update_parameters(Some(fraction)));
        }
    }
}

impl Drop for LauncherProgress {
    /// Hides the bar when the operation ends.
    fn drop(&mut self) {
        self.emit(&update_parameters(None));
    }
}

impl LauncherProgress {
    fn emit(&self, parameters: &glib::Variant) {
        // A dock that is not listening is fine; nothing depends on it.
        let _ = self
            .connection
            .emit_signal(None, OBJECT_PATH, INTERFACE, MEMBER, Some(parameters));
    }
}

/// `fraction` as a whole percent, 0–100.
fn whole_percent(fraction: f64) -> u32 {
    // Clamped to 0–100 first, so the cast cannot truncate or wrap.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0–100"
    )]
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).round() as u32;
    percent
}

/// The `Update` signal's `(sa{sv})`: the app's desktop entry URI and, for
/// `Some(fraction)`, the progress shown; `None` hides it.
fn update_parameters(fraction: Option<f64>) -> glib::Variant {
    let properties = glib::VariantDict::new(None);
    properties.insert("progress-visible", fraction.is_some());
    if let Some(fraction) = fraction {
        properties.insert("progress", fraction.clamp(0.0, 1.0));
    }
    let uri = format!("application://{APP_ID}.desktop").to_variant();
    glib::Variant::tuple_from_iter([uri, properties.end()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signal names the app's desktop entry and carries the progress,
    /// or hides it.
    ///
    /// parity: INT-027
    #[test]
    fn the_update_names_the_app_and_its_progress() {
        let shown = update_parameters(Some(0.25));
        assert_eq!(shown.type_().as_str(), "(sa{sv})");
        let uri = shown.child_value(0).get::<String>();
        assert_eq!(uri, Some(format!("application://{APP_ID}.desktop")));
        let properties = glib::VariantDict::new(Some(&shown.child_value(1)));
        assert_eq!(
            properties.lookup::<bool>("progress-visible").ok().flatten(),
            Some(true)
        );
        assert_eq!(properties.lookup::<f64>("progress").ok().flatten(), Some(0.25));
        let hidden = update_parameters(None);
        let properties = glib::VariantDict::new(Some(&hidden.child_value(1)));
        assert_eq!(
            properties.lookup::<bool>("progress-visible").ok().flatten(),
            Some(false)
        );
        assert_eq!(whole_percent(0.254), 25);
        assert_eq!(whole_percent(3.0), 100);
    }
}
