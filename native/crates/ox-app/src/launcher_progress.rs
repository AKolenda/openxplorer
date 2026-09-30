// SPDX-License-Identifier: AGPL-3.0-only
//! Transfer progress on the dock or taskbar icon (INT-027).
//!
//! Docks and taskbars that follow Unity's launcher API (Dash to Dock,
//! Dash to Panel, Zorin's taskbar, Plank, KDE's task manager) draw a
//! progress bar on an app's icon when the app emits
//! `com.canonical.Unity.LauncherEntry.Update` on the session bus. GNOME
//! Shell's own dash ignores the signal, so nothing changes there. A
//! running operation's panel keeps one [`LauncherProgress`]. The icon is
//! the application's, so while operations run in several windows it shows
//! their mean progress and hides only when the last one ends. Only whole
//! percents are sent, so a fast copy does not flood the bus.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::config::APP_ID;

/// The interface and member docks listen to.
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const MEMBER: &str = "Update";
/// The object the signal comes from; docks match on the app URI instead.
const OBJECT_PATH: &str = "/io/winspace/Development/LauncherEntry";

/// What the icon is told to show.
#[derive(Debug, Clone, Copy, PartialEq)]
enum IconUpdate {
    /// The bar at this fraction (0–1).
    Show(f64),
    /// No bar.
    Hide,
}

/// The progress of every running operation, which the one icon shows
/// together.
#[derive(Debug, Default)]
struct Operations {
    /// Each running operation's fraction (0–1), by its number.
    fractions: BTreeMap<u64, f64>,
    /// The number the next operation gets.
    next: u64,
    /// The whole percent shown last; `None` while the bar is hidden.
    shown: Option<u32>,
}

impl Operations {
    /// Adds an operation at 0 and returns its number.
    fn add(&mut self) -> u64 {
        let number = self.next;
        self.next += 1;
        self.fractions.insert(number, 0.0);
        number
    }

    /// Sets operation `number` to `fraction`, or removes it for `None`,
    /// and returns the update to send: the mean for a new whole percent,
    /// [`IconUpdate::Hide`] after the last operation, and `None` when the
    /// icon already shows it.
    fn set(&mut self, number: u64, fraction: Option<f64>) -> Option<IconUpdate> {
        match fraction {
            Some(fraction) => self.fractions.insert(number, fraction.clamp(0.0, 1.0)),
            None => self.fractions.remove(&number),
        };
        if self.fractions.is_empty() {
            return self.shown.take().map(|_| IconUpdate::Hide);
        }
        #[allow(clippy::cast_precision_loss, reason = "a handful of operations")]
        let mean = self.fractions.values().sum::<f64>() / self.fractions.len() as f64;
        let percent = whole_percent(mean);
        (self.shown.replace(percent) != Some(percent)).then_some(IconUpdate::Show(mean))
    }
}

thread_local! {
    /// The operations of this application, which runs on the main thread.
    static OPERATIONS: RefCell<Operations> = RefCell::default();
}

/// One operation's share of the progress on the app's icon.
#[derive(Debug)]
pub(crate) struct LauncherProgress {
    /// The session connection the signal goes out on.
    connection: gio::DBusConnection,
    /// This operation's number in [`OPERATIONS`].
    number: u64,
}

impl LauncherProgress {
    /// Progress for the application of the window `widget` is in, when it
    /// is on the session bus.
    pub(crate) fn for_widget(widget: &impl IsA<gtk::Widget>) -> Option<Self> {
        let window = widget.root()?.downcast::<gtk::Window>().ok()?;
        let connection = window.application()?.dbus_connection()?;
        let number = OPERATIONS.with_borrow_mut(Operations::add);
        Some(Self { connection, number })
    }

    /// Sets this operation's progress to `fraction` (0–1).
    pub(crate) fn show(&self, fraction: f64) {
        self.update(Some(fraction));
    }

    fn update(&self, fraction: Option<f64>) {
        let update = OPERATIONS.with_borrow_mut(|operations| operations.set(self.number, fraction));
        let Some(update) = update else {
            return;
        };
        let shown = match update {
            IconUpdate::Show(fraction) => Some(fraction),
            IconUpdate::Hide => None,
        };
        // A dock that is not listening is fine; nothing depends on it.
        let _ = self.connection.emit_signal(
            None,
            OBJECT_PATH,
            INTERFACE,
            MEMBER,
            Some(&update_parameters(shown)),
        );
    }
}

impl Drop for LauncherProgress {
    /// Leaves the icon to the other operations, or hides the bar after
    /// the last one.
    fn drop(&mut self) {
        self.update(None);
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

    /// Operations in two windows share the icon: it shows their mean, and
    /// the first to end leaves the bar to the other.
    ///
    /// parity: INT-027
    #[test]
    fn the_bar_hides_only_after_the_last_operation() {
        let mut operations = Operations::default();
        let first = operations.add();
        let second = operations.add();
        assert_eq!(operations.set(first, Some(0.5)), Some(IconUpdate::Show(0.25)));
        assert_eq!(operations.set(first, Some(0.502)), None);
        assert_eq!(operations.set(first, None), Some(IconUpdate::Show(0.0)));
        assert_eq!(operations.set(second, Some(1.0)), Some(IconUpdate::Show(1.0)));
        assert_eq!(operations.set(second, None), Some(IconUpdate::Hide));
    }
}
