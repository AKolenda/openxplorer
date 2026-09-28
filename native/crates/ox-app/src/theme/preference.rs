// SPDX-License-Identifier: AGPL-3.0-only
//! The theme the user chooses and the appearance it resolves to.
//!
//! Ports the theme handling of `applyTheme` in `desktop/ui/app.js`: the
//! saved `preferences.theme` is `system`, `light` or `dark`, and `system`
//! follows the desktop's colour scheme.

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

impl ThemePreference {
    /// Parses `system`, `light` or `dark`; anything else means `system`,
    /// as in `applyTheme`.
    pub(crate) fn parse(value: &str) -> Self {
        Self::from_key(value).unwrap_or_default()
    }

    /// The preference for an action-state key, or `None` for another value.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "system" => Some(ThemePreference::System),
            "light" => Some(ThemePreference::Light),
            "dark" => Some(ThemePreference::Dark),
            _ => None,
        }
    }

    /// The settings and action-state value.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            ThemePreference::System => "system",
            ThemePreference::Light => "light",
            ThemePreference::Dark => "dark",
        }
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

    /// parity: LOOK-003
    #[test]
    fn preferences_parse_like_apply_theme() {
        assert_eq!(ThemePreference::parse("dark"), ThemePreference::Dark);
        assert_eq!(ThemePreference::parse("light"), ThemePreference::Light);
        assert_eq!(ThemePreference::parse("sepia"), ThemePreference::System);
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
    fn tooltips_match_the_web_interface() {
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
