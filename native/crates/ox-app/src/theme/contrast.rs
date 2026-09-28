// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's high-contrast setting.
//!
//! Ports the `@media (prefers-contrast: more)` rules of
//! `desktop/ui/style.css`. On GNOME the user asks for more contrast with
//! `org.gnome.desktop.a11y.interface high-contrast` (Settings >
//! Accessibility), so the skin follows that key (ui-spec.md §4.13, §8).
//! Without the schema the contrast stays normal.

use gtk::gio;
use gtk::prelude::*;

const A11Y_SCHEMA: &str = "org.gnome.desktop.a11y.interface";
const HIGH_CONTRAST_KEY: &str = "high-contrast";

/// How much contrast the desktop asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contrast {
    /// The skin as designed.
    Normal,
    /// Stronger outlines for the selected sidebar row and keyboard focus.
    High,
}

/// Follows GNOME's high-contrast key while it lives.
#[derive(Debug)]
pub struct ContrastSetting {
    settings: Option<gio::Settings>,
}

impl ContrastSetting {
    /// Starts following the key; `on_change` hears every later change.
    pub fn watch(on_change: impl Fn(Contrast) + 'static) -> Self {
        let settings = accessibility_settings();
        if let Some(settings) = &settings {
            settings.connect_changed(Some(HIGH_CONTRAST_KEY), move |settings, _| {
                on_change(contrast_of(settings));
            });
        }
        Self { settings }
    }

    /// The contrast the desktop asks for now.
    pub fn contrast(&self) -> Contrast {
        self.settings.as_ref().map_or(Contrast::Normal, contrast_of)
    }
}

/// GNOME's accessibility settings, when the schema and its key exist.
fn accessibility_settings() -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(A11Y_SCHEMA, true)?;
    schema
        .has_key(HIGH_CONTRAST_KEY)
        .then(|| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None))
}

fn contrast_of(settings: &gio::Settings) -> Contrast {
    if settings.boolean(HIGH_CONTRAST_KEY) {
        Contrast::High
    } else {
        Contrast::Normal
    }
}
