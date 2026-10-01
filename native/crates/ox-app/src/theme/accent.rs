// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's accent colour.
//!
//! Windows 11 draws selection, focus, primary buttons and progress in the
//! system accent. GNOME 47 and later let the user pick one in Settings >
//! Appearance (`org.gnome.desktop.interface accent-color`); Zorin OS picks
//! it with the variant of its theme (`gtk-theme` `ZorinGreen-Light` and so
//! on), and inside Flatpak the Settings portal reports both. [`setting`]
//! follows those sources live. Blue, GNOME's default, keeps the Windows
//! accent of the palettes (`#0067c0` light, `#74beff` dark). Every other
//! accent has a shade per appearance, chosen as Windows chooses its own:
//! dark enough in light for white text on it, light enough in dark for
//! black text on it (at least 4.5:1, which a test checks).

mod setting;

use ox_core::settings::Appearance;

pub(crate) use setting::AccentSetting;

/// GNOME's accents as libadwaita draws them, which the portal reports.
const GNOME_ACCENTS: [(Accent, u32); 9] = [
    (Accent::Windows, 0x35_84_e4),
    (Accent::Teal, 0x21_90_a4),
    (Accent::Green, 0x3a_94_4a),
    (Accent::Yellow, 0xc8_88_00),
    (Accent::Orange, 0xed_5b_00),
    (Accent::Red, 0xe6_2d_42),
    (Accent::Pink, 0xd5_61_99),
    (Accent::Purple, 0x91_41_ac),
    (Accent::Slate, 0x6f_83_96),
];

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

    /// The accent of a Zorin OS theme variant (`ZorinGreen-Dark`), or
    /// `None` for any other theme.
    pub(crate) fn from_zorin_theme(name: &str) -> Option<Self> {
        let variant = name.strip_prefix("Zorin")?;
        let colour = variant.split('-').next().unwrap_or(variant);
        Some(match colour {
            "Blue" => Accent::Windows,
            "Green" => Accent::Green,
            "Orange" => Accent::Orange,
            "Red" => Accent::Red,
            "Purple" => Accent::Purple,
            "Grey" | "Gray" => Accent::Slate,
            _ => return None,
        })
    }

    /// The GNOME accent nearest to the colour the Settings portal reports
    /// (`org.freedesktop.appearance accent-color`, red, green and blue
    /// from 0 to 1), or `None` when a channel is out of range, which
    /// means the desktop has no accent.
    pub(crate) fn nearest_to(red: f64, green: f64, blue: f64) -> Option<Self> {
        let channels = [red, green, blue];
        if !channels.iter().all(|channel| (0.0..=1.0).contains(channel)) {
            return None;
        }
        let distance = |(_, hex): &(Accent, u32)| {
            let reference = [hex >> 16, (hex >> 8) & 0xff, hex & 0xff];
            channels
                .iter()
                .zip(reference)
                .map(|(channel, reference)| (channel * 255.0 - f64::from(reference)).powi(2))
                .sum::<f64>()
        };
        GNOME_ACCENTS
            .iter()
            .min_by(|first, second| distance(first).total_cmp(&distance(second)))
            .map(|(accent, _)| *accent)
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
        assert_eq!(Accent::from_zorin_theme("ZorinGreen-Dark"), Some(Accent::Green));
        assert_eq!(Accent::from_zorin_theme("ZorinGrey-Light"), Some(Accent::Slate));
        assert_eq!(Accent::from_zorin_theme("ZorinBlue-Light"), Some(Accent::Windows));
        assert_eq!(Accent::from_zorin_theme("Adwaita-dark"), None);
        assert_eq!(Accent::nearest_to(0.93, 0.36, 0.0), Some(Accent::Orange));
        assert_eq!(Accent::nearest_to(0.21, 0.52, 0.89), Some(Accent::Windows));
        assert_eq!(Accent::nearest_to(-1.0, -1.0, -1.0), None);
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
