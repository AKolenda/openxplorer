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

const ACCESSIBILITY_SCHEMA: &str = "org.gnome.desktop.a11y.interface";
const HIGH_CONTRAST_KEY: &str = "high-contrast";

/// How much contrast the desktop asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Contrast {
    /// The skin as designed.
    #[default]
    Normal,
    /// Stronger outlines for the selected sidebar row and keyboard focus.
    High,
}

/// Follows GNOME's high-contrast key while it lives.
#[derive(Debug)]
pub(crate) struct ContrastSetting {
    /// GNOME's accessibility settings, when the schema is installed.
    settings: Option<gio::Settings>,
}

impl ContrastSetting {
    /// Starts following the key; `on_change` hears every later change.
    pub(crate) fn watch(on_change: impl Fn(Contrast) + 'static) -> Self {
        let settings = accessibility_settings();
        if let Some(settings) = &settings {
            settings.connect_changed(Some(HIGH_CONTRAST_KEY), move |settings, _| {
                on_change(contrast_of(settings));
            });
        }
        Self { settings }
    }

    /// The contrast the desktop asks for now.
    pub(crate) fn contrast(&self) -> Contrast {
        self.settings.as_ref().map_or(Contrast::Normal, contrast_of)
    }
}

/// GNOME's accessibility settings, when the schema and its key exist.
fn accessibility_settings() -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(ACCESSIBILITY_SCHEMA, true)?;
    if !schema.has_key(HIGH_CONTRAST_KEY) {
        return None;
    }
    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
    Some(settings)
}

/// The contrast that `settings` ask for.
fn contrast_of(settings: &gio::Settings) -> Contrast {
    if settings.boolean(HIGH_CONTRAST_KEY) {
        Contrast::High
    } else {
        Contrast::Normal
    }
}
