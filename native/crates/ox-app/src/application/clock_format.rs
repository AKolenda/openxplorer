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
use ox_core::integration::Sandbox;

const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
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
        let settings = interface_settings();
        if let Some(settings) = &settings {
            apply(settings);
            settings.connect_changed(Some(CLOCK_FORMAT_KEY), |settings, _| apply(settings));
        }
        Self { _settings: settings }
    }
}

/// GNOME's interface settings on the host, when the schema has the key.
fn interface_settings() -> Option<gio::Settings> {
    if Sandbox::detect().is_flatpak() {
        return None;
    }
    let settings = crate::theme::desktop_settings(INTERFACE_SCHEMA)?;
    let has_key = settings.settings_schema()?.has_key(CLOCK_FORMAT_KEY);
    has_key.then_some(settings)
}

/// Shows times on the clock `settings` ask for.
fn apply(settings: &gio::Settings) {
    let value = settings
        .user_value(CLOCK_FORMAT_KEY)
        .and_then(|value| value.get::<String>());
    set_clock_format(ClockFormat::from_gnome(value.as_deref()));
}
