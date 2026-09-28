// SPDX-License-Identifier: AGPL-3.0-only
//! The theme the user chooses and the appearance it resolves to.
//!
//! Ports the theme handling of `applyTheme` in `desktop/ui/app.js`: the
//! saved `preferences.theme` is `system`, `light` or `dark`, and `system`
//! follows the desktop's colour scheme. The saved value is ox-core's
//! [`Theme`]; this module adds what the window draws for it.

use ox_core::settings::Theme;

use crate::icons::Glyph;

/// The appearance actually drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum Appearance {
    /// Light surfaces with dark text; the skin draws it until told
    /// otherwise.
    #[default]
    Light,
    /// Dark surfaces with light text.
    Dark,
}

impl Appearance {
    /// Label of the theme button ("Light" or "Dark").
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// Glyph of the theme button.
    pub(crate) const fn glyph(self) -> Glyph {
        match self {
            Appearance::Light => Glyph::Sun,
            Appearance::Dark => Glyph::Moon,
        }
    }
}

/// The user's choice in settings (`preferences.theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ThemePreference {
    /// Follow the desktop colour scheme; the choice when none is saved.
    #[default]
    System,
    /// Use the light palette regardless of the desktop preference.
    Light,
    /// Use the dark palette regardless of the desktop preference.
    Dark,
}

impl From<Theme> for ThemePreference {
    /// The choice saved in settings as `theme`.
    fn from(theme: Theme) -> Self {
        match theme {
            Theme::System => ThemePreference::System,
            Theme::Light => ThemePreference::Light,
            Theme::Dark => ThemePreference::Dark,
        }
    }
}

impl From<ThemePreference> for Theme {
    /// The value settings save for the choice.
    fn from(preference: ThemePreference) -> Self {
        match preference {
            ThemePreference::System => Theme::System,
            ThemePreference::Light => Theme::Light,
            ThemePreference::Dark => Theme::Dark,
        }
    }
}

impl ThemePreference {
    /// The preference for an action-state key (the settings value), or
    /// `None` for another value.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Theme::from_key(key).map(Self::from)
    }

    /// The action-state value, which is also the settings value.
    pub(crate) fn key(self) -> &'static str {
        Theme::from(self).as_str()
    }

    /// The appearance for this choice while the desktop draws `desktop`.
    pub(crate) const fn resolve(self, desktop: Appearance) -> Appearance {
        match self {
            ThemePreference::System => desktop,
            ThemePreference::Light => Appearance::Light,
            ThemePreference::Dark => Appearance::Dark,
        }
    }

    /// Tooltip of the theme button, as in `applyTheme`.
    pub(crate) fn tooltip(self, appearance: Appearance) -> String {
        let shown = match self {
            ThemePreference::System => {
                let current = appearance.label().to_lowercase();
                format!("System ({current})")
            }
            chosen => chosen.key().to_owned(),
        };
        format!("Appearance: {shown}. Click to change.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The saved theme a window starts from: settings drop an unknown
    /// value, which leaves the default, System (`applyTheme`).
    fn saved_choice(saved: &str) -> ThemePreference {
        ThemePreference::from(Theme::from_key(saved).unwrap_or_default())
    }

    /// parity: LOOK-003
    #[test]
    fn saved_themes_parse_and_anything_else_means_system() {
        assert_eq!(saved_choice("dark"), ThemePreference::Dark);
        assert_eq!(saved_choice("light"), ThemePreference::Light);
        assert_eq!(saved_choice("sepia"), ThemePreference::System);
    }

    #[test]
    fn every_choice_round_trips_through_settings_and_action_keys() {
        for theme in Theme::ALL {
            let preference = ThemePreference::from(theme);
            assert_eq!(Theme::from(preference), theme);
            assert_eq!(ThemePreference::from_key(preference.key()), Some(preference));
        }
        assert_eq!(ThemePreference::from_key("sepia"), None);
    }

    /// parity: LOOK-003
    #[test]
    fn system_follows_the_desktop() {
        assert_eq!(
            ThemePreference::System.resolve(Appearance::Dark),
            Appearance::Dark
        );
        assert_eq!(
            ThemePreference::System.resolve(Appearance::Light),
            Appearance::Light
        );
        assert_eq!(
            ThemePreference::Light.resolve(Appearance::Dark),
            Appearance::Light
        );
        assert_eq!(ThemePreference::Dark.resolve(Appearance::Light), Appearance::Dark);
    }

    #[test]
    fn the_tooltip_names_the_choice_and_the_system_appearance() {
        assert_eq!(
            ThemePreference::System.tooltip(Appearance::Dark),
            "Appearance: System (dark). Click to change."
        );
        assert_eq!(
            ThemePreference::Light.tooltip(Appearance::Light),
            "Appearance: light. Click to change."
        );
    }
}
