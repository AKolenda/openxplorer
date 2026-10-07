// SPDX-License-Identifier: AGPL-3.0-only
//! Date groups follow the calendar (VIEW-022): "Today", "Yesterday" and
//! Explorer's other periods count from the day the listing was grouped,
//! so when the day changes the window groups its folders again.
//!
//! A timer wakes the window just after the next local midnight. A timer
//! does not run while the computer sleeps, so the window also checks the
//! day when it becomes the active window again and on a refresh (F5). On
//! the same day the check changes nothing.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use super::BrowserWindow;

impl BrowserWindow {
    /// Groups the folders shown again when the day has changed since they
    /// were grouped.
    pub(crate) fn follow_the_day(&self) {
        for pane in self.folder_panes() {
            if pane.model().day_changed() {
                super::view_style::group_pane(pane, pane.model().grouping());
            }
        }
    }

    /// Follows the day at every midnight and whenever the window becomes
    /// active again.
    pub(super) fn install_day_changes(&self) {
        self.connect_is_active_notify(|window| {
            if window.is_active() {
                window.follow_the_day();
            }
        });
        self.wake_after_midnight();
    }

    /// Follows the day just after the next local midnight, then waits for
    /// the one after. The timer ends with the window.
    fn wake_after_midnight(&self) {
        let window = self.downgrade();
        glib::timeout_add_local_once(until_after_midnight(), move || {
            if let Some(window) = window.upgrade() {
                window.follow_the_day();
                window.wake_after_midnight();
            }
        });
    }
}

/// How long until one second past the next local midnight; an hour should
/// the clock be unreadable, so the window checks again later.
fn until_after_midnight() -> Duration {
    let fallback = Duration::from_secs(3600);
    let Ok(now) = glib::DateTime::now_local() else {
        return fallback;
    };
    let Some(next) = next_midnight(&now) else {
        return fallback;
    };
    let micros = next.difference(&now).as_microseconds();
    u64::try_from(micros)
        .map(|micros| Duration::from_micros(micros) + Duration::from_secs(1))
        .unwrap_or(fallback)
}

/// The start of the day after `now`, in `now`'s time zone.
fn next_midnight(now: &glib::DateTime) -> Option<glib::DateTime> {
    let today = glib::DateTime::new(
        &now.timezone(),
        now.year(),
        now.month(),
        now.day_of_month(),
        0,
        0,
        0.0,
    )
    .ok()?;
    today.add_days(1).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The next midnight is the start of tomorrow, also at 23:59:59 and
    /// across a month's end.
    #[test]
    fn the_next_midnight_starts_tomorrow() {
        let zone = glib::TimeZone::utc();
        let late = glib::DateTime::new(&zone, 2026, 1, 31, 23, 59, 59.0).expect("a time");
        let next = next_midnight(&late).expect("the next midnight");
        assert_eq!(
            (next.year(), next.month(), next.day_of_month(), next.hour()),
            (2026, 2, 1, 0)
        );
        let early = glib::DateTime::new(&zone, 2026, 10, 4, 0, 0, 1.0).expect("a time");
        let next = next_midnight(&early).expect("the next midnight");
        assert_eq!((next.month(), next.day_of_month()), (10, 5));
    }
}
