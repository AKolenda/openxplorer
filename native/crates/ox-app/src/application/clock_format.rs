// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's 12-hour or 24-hour clock, which Properties timestamps
//! follow (GNOME's `org.gnome.desktop.interface clock-format`, Settings >
//! Date & Time > Time Format).
//!
//! Only a value the user set counts: the schema's default is `24h` in
//! every locale, so an unset key leaves the locale's own clock. Inside
//! Flatpak the schema holds only the runtime's defaults, so the locale's
//! clock stays there too.

use gtk::gio;
use gtk::prelude::*;
use ox_core::format::{set_clock_format, ClockFormat};

use crate::theme::{host_desktop_key, INTERFACE_SCHEMA};

const CLOCK_FORMAT_KEY: &str = "clock-format";

/// Follows GNOME's clock format while it lives.
#[derive(Debug)]
pub(super) struct ClockSetting {
    /// GNOME's interface settings, kept so their change handler lives.
    _settings: Option<gio::Settings>,
}

impl ClockSetting {
    /// Applies the desktop's clock format now and on every change.
    pub(super) fn follow() -> Self {
        let settings = host_desktop_key(INTERFACE_SCHEMA, CLOCK_FORMAT_KEY);
        if let Some(settings) = &settings {
            apply(settings);
            settings.connect_changed(Some(CLOCK_FORMAT_KEY), |settings, _| apply(settings));
        }
        Self { _settings: settings }
    }
}

/// Shows times on the clock `settings` ask for.
fn apply(settings: &gio::Settings) {
    let value = settings
        .user_value(CLOCK_FORMAT_KEY)
        .and_then(|value| value.get::<String>());
    set_clock_format(ClockFormat::from_gnome(value.as_deref()));
}

#[cfg(test)]
mod tests {
    use ox_core::format::clock_format;

    use super::*;
    use crate::test_support::desktop_setting::DesktopSetting;
    use crate::test_support::harness::wait_until;

    /// Puts the locale's clock back when a test ends, also when it fails.
    struct LocaleClock;

    impl Drop for LocaleClock {
        fn drop(&mut self) {
            set_clock_format(ClockFormat::Locale);
        }
    }

    /// The clock follows a format the user sets at once, and an unset key
    /// leaves the locale's clock, although the schema's default is `24h`.
    ///
    /// parity: LOOK-026
    #[gtk::test]
    fn the_desktop_clock_format_is_followed_at_once() {
        let Some(settings) = host_desktop_key(INTERFACE_SCHEMA, CLOCK_FORMAT_KEY) else {
            return;
        };
        let Some(clock) = DesktopSetting::in_memory(settings.clone(), CLOCK_FORMAT_KEY) else {
            return;
        };
        let _locale_clock = LocaleClock;
        let _setting = ClockSetting::follow();
        assert_eq!(clock_format(), ClockFormat::Locale);

        clock.set_string("12h");
        wait_until("the 12-hour clock", || clock_format() == ClockFormat::TwelveHour);
        clock.set_string("24h");
        wait_until("the 24-hour clock", || {
            clock_format() == ClockFormat::TwentyFourHour
        });
        settings.reset(CLOCK_FORMAT_KEY);
        wait_until("the locale's clock", || clock_format() == ClockFormat::Locale);
    }
}
