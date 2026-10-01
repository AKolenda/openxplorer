// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's accent colour.
//!
//! Windows 11 draws selection, focus, primary buttons and progress in the
//! system accent; GNOME 47 and later let the user pick one in Settings >
//! Appearance (`org.gnome.desktop.interface accent-color`). The skin
//! follows that key live. Blue, GNOME's default, keeps the Windows accent
//! of the palettes (`#0067c0` light, `#74beff` dark). Every other accent
//! has a shade per appearance, chosen as Windows chooses its own: dark
//! enough in light for white text on it, light enough in dark for black
//! text on it (at least 4.5:1, which a test checks). Inside Flatpak the
//! schema holds only the runtime's defaults, so the Windows blue stays.

use gtk::gio;
use gtk::prelude::*;
use ox_core::integration::Sandbox;
use ox_core::settings::Appearance;

const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";

/// GNOME's key behind Settings > Appearance > Accent Color.
pub(crate) const ACCENT_COLOR_KEY: &str = "accent-color";

/// An accent the skin draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Accent {
    /// The Windows blue of the palettes, for GNOME's blue or no choice.
    #[default]
    Windows,
    /// One of GNOME's other accents.
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
    Slate,
}

impl Accent {
    /// Every accent other than the Windows blue.
    #[cfg(test)]
    pub(crate) const OTHERS: [Accent; 8] = [
        Accent::Teal,
        Accent::Green,
        Accent::Yellow,
        Accent::Orange,
        Accent::Red,
        Accent::Pink,
        Accent::Purple,
        Accent::Slate,
    ];

    /// The accent GNOME's `accent-color` value names; blue and anything
    /// unknown keep the Windows blue.
    pub(crate) fn from_gnome(name: &str) -> Self {
        match name {
            "teal" => Accent::Teal,
            "green" => Accent::Green,
            "yellow" => Accent::Yellow,
            "orange" => Accent::Orange,
            "red" => Accent::Red,
            "pink" => Accent::Pink,
            "purple" => Accent::Purple,
            "slate" => Accent::Slate,
            _ => Accent::Windows,
        }
    }

    /// The accent's shade in `appearance`, `None` for the Windows blue,
    /// which the palettes draw. GNOME's accent darkened for light and
    /// lightened for dark, as Windows shades its accent.
    pub(crate) const fn shade(self, appearance: Appearance) -> Option<&'static str> {
        let (light, dark) = match self {
            Accent::Windows => return None,
            Accent::Teal => ("#1a7080", "#7ebfca"),
            Accent::Green => ("#2e763b", "#8dc196"),
            Accent::Yellow => ("#8c5f00", "#daae52"),
            Accent::Orange => ("#b44500", "#f5a06b"),
            Accent::Red => ("#c62739", "#f49ea8"),
            Accent::Pink => ("#a24a74", "#e6a0c2"),
            Accent::Purple => ("#9141ac", "#cca8d9"),
            Accent::Slate => ("#596978", "#a9b5c0"),
        };
        Some(match appearance {
            Appearance::Light => light,
            Appearance::Dark => dark,
        })
    }
}

/// The tokens that `accent` redefines over the palette of `appearance`:
/// the accent itself, which buttons, focus rings, progress and the sidebar
/// bar use, and the selection's fill and edges tinted with it. Empty for
/// the Windows blue.
pub(crate) fn stylesheet(accent: Accent, appearance: Appearance) -> String {
    let Some(shade) = accent.shade(appearance) else {
        return String::new();
    };
    let selected_tint = match appearance {
        Appearance::Light => ".1",
        Appearance::Dark => ".22",
    };
    format!(
        "@define-color ox_accent {shade};\n\
         @define-color ox_selected mix(@ox_bg, @ox_accent, {selected_tint});\n\
         @define-color ox_selected_edge alpha(@ox_accent, .16);\n\
         @define-color ox_tile_selected_edge alpha(@ox_accent, .38);\n"
    )
}

/// Follows GNOME's accent key while it lives.
#[derive(Debug)]
pub(crate) struct AccentSetting {
    /// GNOME's interface settings, on the host with the key installed.
    settings: Option<gio::Settings>,
}

impl AccentSetting {
    /// Starts following the key; `on_change` hears every later change.
    pub(crate) fn watch(on_change: impl Fn(Accent) + 'static) -> Self {
        let settings = interface_settings();
        if let Some(settings) = &settings {
            settings.connect_changed(Some(ACCENT_COLOR_KEY), move |settings, _| {
                on_change(accent_of(settings));
            });
        }
        Self { settings }
    }

    /// The accent the desktop asks for now.
    pub(crate) fn accent(&self) -> Accent {
        self.settings.as_ref().map_or(Accent::Windows, accent_of)
    }
}

/// GNOME's interface settings on the host, when the schema has the key.
pub(crate) fn interface_settings() -> Option<gio::Settings> {
    if Sandbox::detect().is_flatpak() {
        return None;
    }
    let settings = super::desktop_settings(INTERFACE_SCHEMA)?;
    let has_key = settings.settings_schema()?.has_key(ACCENT_COLOR_KEY);
    has_key.then_some(settings)
}

/// The accent `settings` ask for.
fn accent_of(settings: &gio::Settings) -> Accent {
    Accent::from_gnome(&settings.string(ACCENT_COLOR_KEY))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relative luminance of `#rrggbb` (WCAG 2).
    fn luminance(hex: &str) -> f64 {
        let channel = |start: usize| {
            let value = f64::from(u8::from_str_radix(&hex[start..start + 2], 16).expect("hex"));
            let value = value / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    /// The WCAG contrast ratio of two `#rrggbb` colours.
    fn contrast(first: &str, second: &str) -> f64 {
        let (first, second) = (luminance(first), luminance(second));
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    /// GNOME's names choose the accent, blue keeps the Windows accent, and
    /// every other accent is readable under the text the palettes put on
    /// it: white in light, black in dark.
    ///
    /// parity: LOOK-024
    #[test]
    fn gnome_accents_have_a_readable_shade_in_each_appearance() {
        assert_eq!(Accent::from_gnome("blue"), Accent::Windows);
        assert_eq!(Accent::from_gnome("green"), Accent::Green);
        assert_eq!(Accent::from_gnome("magenta"), Accent::Windows);
        assert_eq!(stylesheet(Accent::Windows, Appearance::Light), "");
        for accent in Accent::OTHERS {
            let light = accent.shade(Appearance::Light).expect("a light shade");
            let dark = accent.shade(Appearance::Dark).expect("a dark shade");
            assert!(contrast(light, "#ffffff") >= 4.5, "{accent:?} in light: {light}");
            assert!(contrast(dark, "#000000") >= 4.5, "{accent:?} in dark: {dark}");
            assert!(
                contrast(dark, "#202020") >= 4.5,
                "{accent:?} on the dark background"
            );
        }
        let green = stylesheet(Accent::Green, Appearance::Dark);
        assert!(green.starts_with("@define-color ox_accent #8dc196;\n"), "{green}");
        assert!(
            green.contains("ox_selected mix(@ox_bg, @ox_accent, .22)"),
            "{green}"
        );
    }
}
