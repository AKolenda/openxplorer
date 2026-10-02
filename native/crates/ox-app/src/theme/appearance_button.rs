// SPDX-License-Identifier: AGPL-3.0-only
//! What the Appearance button shows for the theme and the drawn appearance.
//!
//! Ports the theme button of `applyTheme` in `v2.0.0:desktop/ui/app.js`. The saved
//! choice is ox-core's [`Theme`] and the palette it resolves to is ox-core's
//! [`Appearance`] ([`Theme::appearance`]); this module adds only the label,
//! icon and tooltip the window shows for them.

use ox_core::settings::{Appearance, Theme};

use crate::icons::Icon;

/// The Appearance button's label and icon for a drawn [`Appearance`].
pub(crate) trait AppearanceExt {
    /// Label of the theme button ("Light" or "Dark").
    fn label(self) -> &'static str;

    /// Glyph of the theme button: a sun or a moon.
    fn icon(self) -> Icon;
}

impl AppearanceExt for Appearance {
    fn label(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Appearance::Light => Icon::WeatherSunny,
            Appearance::Dark => Icon::WeatherMoon,
        }
    }
}

/// Tooltip of the theme button for the chosen `theme` while `appearance`
/// is drawn, as in `applyTheme`.
pub(crate) fn tooltip(theme: Theme, appearance: Appearance) -> String {
    let shown = match theme {
        Theme::System => {
            let current = appearance.label().to_lowercase();
            ox_core::i18n::format_message("System ({current})", &[("current", &(current).to_string())])
        }
        chosen => chosen.as_str().to_owned(),
    };
    ox_core::i18n::format_message(
        "Appearance: {shown}. Click to change.",
        &[("shown", &(shown).to_string())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: LOOK-003
    #[test]
    fn the_tooltip_names_the_choice_and_the_system_appearance() {
        assert_eq!(
            tooltip(Theme::System, Appearance::Dark),
            "Appearance: System (dark). Click to change."
        );
        assert_eq!(
            tooltip(Theme::Light, Appearance::Light),
            "Appearance: light. Click to change."
        );
    }

    #[test]
    fn the_button_shows_a_sun_for_light_and_a_moon_for_dark() {
        assert_eq!(Appearance::Light.label(), "Light");
        assert_eq!(Appearance::Light.icon(), Icon::WeatherSunny);
        assert_eq!(Appearance::Dark.label(), "Dark");
        assert_eq!(Appearance::Dark.icon(), Icon::WeatherMoon);
    }
}
