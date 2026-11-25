// SPDX-License-Identifier: AGPL-3.0-only
//! How GNOME's keys and the portal's settings map to an appearance.

use super::*;
use crate::test_support::desktop_setting::DesktopSetting;
use crate::test_support::harness::wait_until;

/// GNOME's colour scheme decides first when it has a preference, over
/// the portal's and any theme name.
///
/// parity: LOOK-004
#[test]
fn color_scheme_decides_first() {
    let gnome = |key| appearance_from_gnome_scheme(Some(key));
    let dark_theme = Some(appearance_of_theme_name("ZorinBlue-Dark"));
    assert_eq!(
        decide(&[gnome("prefer-dark"), Some(Appearance::Light)], &[None]),
        Appearance::Dark
    );
    assert_eq!(
        decide(&[gnome("prefer-light"), None], &[dark_theme]),
        Appearance::Light
    );
}

/// GNOME's `default` scheme leaves it to the portal's colour scheme,
/// which KDE Plasma, Cinnamon and COSMIC set while GNOME's schema keeps
/// its defaults, and then to the theme names.
///
/// parity: LOOK-004
#[test]
fn default_scheme_falls_back_to_the_portal_then_the_theme_names() {
    let default = appearance_from_gnome_scheme(Some("default"));
    assert_eq!(default, None);
    let adwaita = Some(appearance_of_theme_name("Adwaita"));
    assert_eq!(
        decide(&[default, appearance_from_portal(1)], &[adwaita]),
        Appearance::Dark,
        "the portal prefers dark"
    );
    assert_eq!(
        decide(&[default, appearance_from_portal(0)], &[adwaita]),
        Appearance::Light,
        "no preference and a light theme"
    );
}

/// Without a colour scheme, any theme name with "dark" in it asks for
/// dark: GNOME's may still be Adwaita on Xfce, MATE and Cinnamon, while
/// GTK's own name is the desktop's (`Greybird-dark`).
///
/// parity: LOOK-004
#[test]
fn any_dark_theme_name_asks_for_dark() {
    let theme = |name| Some(appearance_of_theme_name(name));
    assert_eq!(
        decide(&[None, None], &[theme("Adwaita"), None, theme("Greybird-dark")]),
        Appearance::Dark
    );
    assert_eq!(
        decide(&[None, None], &[theme("ZorinBlue-Light"), theme("Mint-Y")]),
        Appearance::Light
    );
    assert_eq!(decide(&[None, None], &[None]), Appearance::Light);
}

/// GTK's `HighContrastInverse` is light text on black, so it asks for the
/// dark palette as a "dark" name does; `HighContrast` asks for the light one.
///
/// parity: LOOK-004, ACC-011
#[test]
fn the_inverse_high_contrast_theme_is_dark() {
    assert_eq!(
        appearance_from_gnome_keys(Some("default"), Some("HighContrastInverse")),
        Some(Appearance::Dark)
    );
    assert_eq!(
        appearance_from_gnome_keys(Some("default"), Some("HighContrast")),
        Some(Appearance::Light)
    );
}

/// parity: LOOK-004
#[test]
fn portal_values_one_and_two_prefer_dark_and_light() {
    assert_eq!(appearance_from_portal(1), Some(Appearance::Dark));
    assert_eq!(appearance_from_portal(2), Some(Appearance::Light));
    assert_eq!(appearance_from_portal(0), None);
}

/// Without GNOME's schema, a portal with no preference (0) leaves the
/// GTK preference read at startup in charge, not the palette the skin
/// has drawn on the display since.
///
/// parity: LOOK-004
#[gtk::test]
fn a_portal_without_preference_falls_back_to_the_startup_gtk_preference() {
    let display = gdk::Display::default().expect("GTK tests run on a private display");
    let drawn_on_display = gtk_preference(&display);
    let startup_preference = match drawn_on_display {
        Appearance::Light => Appearance::Dark,
        Appearance::Dark => Appearance::Light,
    };
    let scheme = SystemScheme::without_gnome_schema(startup_preference);
    assert_eq!(
        scheme.appearance(),
        startup_preference,
        "before the portal answers"
    );

    scheme.set_portal_value(2);
    assert_eq!(scheme.appearance(), Appearance::Light, "the portal prefers light");

    scheme.set_portal_value(0);
    assert_eq!(
        scheme.appearance(),
        startup_preference,
        "the portal has no preference"
    );
}

/// A change of GNOME's colour scheme reaches the app at once, both
/// ways, while the app writes nothing back. Skipped unless `GSettings`
/// keeps its values in memory, so the user's dconf stays untouched.
///
/// parity: LOOK-004
#[gtk::test]
fn a_changed_gnome_color_scheme_is_followed_at_once() {
    let Some(settings) = super::super::desktop_settings(INTERFACE_SCHEMA) else {
        return;
    };
    let has_key = settings
        .settings_schema()
        .is_some_and(|schema| schema.has_key(COLOR_SCHEME_KEY));
    if !has_key {
        return;
    }
    let Some(color_scheme) = DesktopSetting::in_memory(settings, COLOR_SCHEME_KEY) else {
        return;
    };
    let heard = Rc::new(Cell::new(None));
    let scheme = SystemScheme::new(&GtkDesktop::preferring(Appearance::Light), {
        let heard = Rc::clone(&heard);
        move |appearance| heard.set(Some(appearance))
    });
    for (value, expected) in [
        ("prefer-dark", Appearance::Dark),
        ("prefer-light", Appearance::Light),
    ] {
        color_scheme.set_string(value);
        wait_until(value, || heard.get() == Some(expected));
        assert_eq!(scheme.appearance(), expected);
    }
}

/// One `SettingChanged` signal and the setting expected from it.
struct PortalSignalCase {
    namespace: &'static str,
    key: &'static str,
    value: glib::Variant,
    expected: Option<PortalSetting>,
}

/// parity: LOOK-004
#[test]
fn portal_changes_accept_only_the_color_scheme_and_gtk_theme() {
    let cases = [
        PortalSignalCase {
            namespace: APPEARANCE_NAMESPACE,
            key: "color-scheme",
            value: 1u32.to_variant(),
            expected: Some(PortalSetting::ColorScheme(1)),
        },
        PortalSignalCase {
            namespace: APPEARANCE_NAMESPACE,
            key: "color-scheme",
            value: 0u32.to_variant(),
            expected: Some(PortalSetting::ColorScheme(0)),
        },
        PortalSignalCase {
            namespace: INTERFACE_SCHEMA,
            key: "gtk-theme",
            value: "ZorinBlue-Dark".to_variant(),
            expected: Some(PortalSetting::GtkTheme("ZorinBlue-Dark".to_owned())),
        },
        PortalSignalCase {
            namespace: INTERFACE_SCHEMA,
            key: "color-scheme",
            value: "prefer-dark".to_variant(),
            expected: None,
        },
        PortalSignalCase {
            namespace: "another.namespace",
            key: "color-scheme",
            value: 1u32.to_variant(),
            expected: None,
        },
        PortalSignalCase {
            namespace: APPEARANCE_NAMESPACE,
            key: "accent-color",
            value: 1u32.to_variant(),
            expected: None,
        },
        PortalSignalCase {
            namespace: APPEARANCE_NAMESPACE,
            key: "color-scheme",
            value: "dark".to_variant(),
            expected: None,
        },
    ];
    for case in cases {
        let parameters = (case.namespace, case.key, case.value).to_variant();
        assert_eq!(
            portal_change(&parameters),
            case.expected,
            "{}.{}",
            case.namespace,
            case.key
        );
    }
}
