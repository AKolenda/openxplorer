// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's text preferences: its interface font and its text
//! scaling.
//!
//! GNOME's Settings > Accessibility > Large Text (`text-scaling-factor`)
//! reaches GTK as the font resolution `gtk-xft-dpi`, and the interface
//! font as `gtk-font-name`. The skin sizes text in pixels, which GTK does
//! not scale by the resolution, so the skin reads the factor itself and
//! draws text one or more levels larger: the app's text size times the
//! desktop's factor, rounded to the nearest level, so rows, menus and
//! tiles grow with the text as they do for Ctrl+plus.
//!
//! The Windows font stack stays the default, for the Windows look. With
//! Settings > Appearance > "Use the desktop font" on, text uses the
//! desktop's font family, and its size scales the text as the factor does
//! (the Windows 13-pixel text against the desktop font's size).

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, pango};

use crate::text_size::TextSize;

/// The resolution GTK reports at a text scaling factor of 1, in 1/1024
/// dots per inch.
const UNSCALED_DPI: f64 = 96.0 * 1024.0;

/// The window's text size in pixels at 100 %, which the desktop font's
/// size is measured against (`window.ox` in `theme/fonts.rs`).
const WINDOWS_TEXT_PIXELS: f64 = 13.0;

/// What the desktop asks of text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DesktopText {
    /// The interface font's family, such as "Noto Sans".
    pub(crate) family: Option<String>,
    /// The interface font's size in pixels at 96 dots per inch.
    pub(crate) pixels: Option<f64>,
    /// The accessibility text scaling factor: 1, or 1.25 for Large Text.
    pub(crate) scaling: f64,
}

impl Default for DesktopText {
    fn default() -> Self {
        Self {
            family: None,
            pixels: None,
            scaling: 1.0,
        }
    }
}

impl DesktopText {
    /// What `settings` (a display's GTK settings) ask of text.
    pub(crate) fn of_settings(settings: &gtk::Settings) -> Self {
        let font = settings
            .gtk_font_name()
            .map(|name| pango::FontDescription::from_string(&name));
        let family = font
            .as_ref()
            .and_then(pango::FontDescription::family)
            .map(String::from)
            .filter(|family| !family.is_empty());
        let pixels = font
            .as_ref()
            .filter(|font| !font.is_size_absolute() && font.size() > 0)
            .map(|font| f64::from(font.size()) / f64::from(pango::SCALE) * 96.0 / 72.0);
        let dpi = settings.gtk_xft_dpi();
        let scaling = if dpi > 0 {
            f64::from(dpi) / UNSCALED_DPI
        } else {
            1.0
        };
        Self {
            family,
            pixels,
            scaling,
        }
    }

    /// The factor text is drawn at beyond the app's text size: the
    /// accessibility scaling, and with the desktop font, its size against
    /// the Windows text.
    pub(crate) fn factor(&self, uses_desktop_font: bool) -> f64 {
        let font = match self.pixels {
            Some(pixels) if uses_desktop_font => pixels / WINDOWS_TEXT_PIXELS,
            _ => 1.0,
        };
        self.scaling * font
    }
}

/// Keeps a skin's text in step with the desktop's font and text scaling
/// while it lives.
#[derive(Debug)]
pub(crate) struct DesktopTextWatch {
    settings: gtk::Settings,
    handlers: Vec<glib::SignalHandlerId>,
}

impl DesktopTextWatch {
    /// Calls `on_change` with what `settings` ask of text now and after
    /// every change of the font or the text scaling.
    pub(crate) fn new(settings: &gtk::Settings, on_change: impl Fn(DesktopText) + 'static) -> Self {
        on_change(DesktopText::of_settings(settings));
        let on_change = Rc::new(on_change);
        let handlers = ["gtk-font-name", "gtk-xft-dpi"]
            .into_iter()
            .map(|property| {
                let on_change = Rc::clone(&on_change);
                settings.connect_notify_local(Some(property), move |settings, _| {
                    on_change(DesktopText::of_settings(settings));
                })
            })
            .collect();
        Self {
            settings: settings.clone(),
            handlers,
        }
    }
}

impl Drop for DesktopTextWatch {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.settings.disconnect(handler);
        }
    }
}

/// The text size drawn for the `chosen` size at `factor`: as many levels
/// above (or below) the chosen one as the level nearest to 100% times the
/// factor is above 100%. Moving by whole levels keeps every Ctrl+plus and
/// Ctrl+minus a visible step until the drawn size reaches an end.
pub(crate) fn drawn_text_size(chosen: TextSize, factor: f64) -> TextSize {
    if (factor - 1.0).abs() < f64::EPSILON {
        return chosen;
    }
    let wanted = f64::from(TextSize::DEFAULT.percent()) * factor;
    let distance = |size: &TextSize| (f64::from(size.percent()) - wanted).abs();
    let scaled_default = TextSize::all()
        .min_by(|first, second| distance(first).total_cmp(&distance(second)))
        .unwrap_or(TextSize::DEFAULT);
    chosen.moved_by(scaled_default.levels_above(TextSize::DEFAULT))
}

/// The rule that sets the window's, menus' and tooltips' font to
/// `family`, the desktop's, over the Windows stack of `base.css`.
pub(crate) fn font_family_rule(family: &str) -> String {
    let quoted = family.replace(['\\', '"'], "");
    format!("window.ox, window.ox popover, tooltip {{ font-family: \"{quoted}\", sans-serif; }}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_size::Step;

    /// Large Text draws the app's text a level larger, and the desktop's
    /// font is used, at its size, only when the user asks for it.
    ///
    /// parity: LOOK-025
    #[gtk::test]
    fn the_desktops_font_and_large_text_reach_the_text_size() {
        let settings = gtk::Settings::default().expect("GTK tests run on a private display");
        let (font_before, dpi_before) = (settings.gtk_font_name(), settings.gtk_xft_dpi());
        settings.set_gtk_font_name(Some("Noto Sans 11"));
        settings.set_gtk_xft_dpi(120 * 1024);
        let desktop = DesktopText::of_settings(&settings);
        settings.set_gtk_font_name(font_before.as_deref());
        settings.set_gtk_xft_dpi(dpi_before);

        assert_eq!(desktop.family.as_deref(), Some("Noto Sans"));
        assert!((desktop.scaling - 1.25).abs() < 1e-9, "{}", desktop.scaling);
        let default = TextSize::DEFAULT;
        assert_eq!(drawn_text_size(default, desktop.factor(false)).percent(), 125);
        // 11 points are 14.67 pixels: 1.25 × 1.13 of the Windows text.
        assert_eq!(drawn_text_size(default, desktop.factor(true)).percent(), 150);
        assert_eq!(drawn_text_size(TextSize::from_percent(200), 1.25).percent(), 200);
        assert_eq!(drawn_text_size(TextSize::from_percent(90), 1.0).percent(), 90);
        // Every Ctrl+plus draws larger text until the largest size.
        let mut chosen = TextSize::from_percent(80);
        while drawn_text_size(chosen, 1.25).percent() < 200 {
            let larger = Step::Increase.apply(chosen);
            assert!(
                drawn_text_size(larger, 1.25).percent() > drawn_text_size(chosen, 1.25).percent(),
                "from {}%",
                chosen.percent()
            );
            chosen = larger;
        }

        let skin = crate::theme::Skin::detached();
        skin.set_desktop_text(desktop);
        assert_eq!(skin.text_size(), default, "the chosen size stays");
        assert_eq!(skin.drawn_text_size().percent(), 125, "Large Text");
        skin.set_uses_desktop_font(true);
        assert_eq!(skin.drawn_text_size().percent(), 150, "the desktop font's size");
        assert_eq!(
            font_family_rule("Noto Sans"),
            "window.ox, window.ox popover, tooltip { font-family: \"Noto Sans\", sans-serif; }\n"
        );
    }
}
