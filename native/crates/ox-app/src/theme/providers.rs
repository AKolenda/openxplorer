// SPDX-License-Identifier: AGPL-3.0-only
//! The skin's CSS providers and their place in GTK's style cascade.
//!
//! Ports the stylesheet half of `apply_native_theme` in
//! `desktop/winspace.py`. GTK's built-in theme is forced underneath the
//! skin so the desktop theme (Zorin's) cannot leak into it; only this
//! application's GTK settings change, never GNOME's.

use gtk::gdk;

use super::contrast::Contrast;
use super::fonts::css_for_text_size;
use super::stylesheets;
use super::Appearance;
use crate::text_size::TextSize;

/// A provider's place in the skin's cascade. Each layer sits one step
/// above the one before it, above the application priority, so it wins
/// over the layers before it.
#[derive(Debug, Clone, Copy)]
enum Layer {
    /// The rules of every region of the window.
    Rules,
    /// Font sizes and heights generated for the text size.
    TextSize,
    /// The colour tokens the rules use.
    Palette,
    /// The high-contrast rules.
    HighContrast,
}

impl Layer {
    /// The GTK style priority of this layer.
    fn priority(self) -> u32 {
        let steps_above_application = match self {
            Layer::Rules => 0,
            Layer::TextSize => 1,
            Layer::Palette => 2,
            Layer::HighContrast => 3,
        };
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + steps_above_application
    }
}

/// The providers the skin reloads after installing them, one per
/// [`Layer`] whose stylesheet changes. The rules never change, so their
/// provider is not kept.
#[derive(Debug)]
pub(super) struct Providers {
    /// The colour tokens of the drawn [`Appearance`].
    palette: gtk::CssProvider,
    /// Font sizes and heights for the text size.
    text_size: gtk::CssProvider,
    /// The high-contrast rules, empty at normal contrast.
    high_contrast: gtk::CssProvider,
}

impl Providers {
    /// Forces GTK's built-in theme on `display` and adds the skin over it:
    /// the light palette at the default text size and normal contrast.
    pub(super) fn install(display: &gdk::Display) -> Self {
        force_builtin_theme(&gtk::Settings::for_display(display));
        let providers = Self::starting();
        add_to_display(display, &provider_with(stylesheets::RULES), Layer::Rules);
        add_to_display(display, &providers.text_size, Layer::TextSize);
        add_to_display(display, &providers.palette, Layer::Palette);
        add_to_display(display, &providers.high_contrast, Layer::HighContrast);
        providers
    }

    /// Providers on no display, for a skin that draws nothing.
    #[cfg(test)]
    pub(super) fn detached() -> Self {
        Self::starting()
    }

    /// The providers loaded with what a skin starts with: the light
    /// palette, the default text size and no high-contrast rules.
    fn starting() -> Self {
        Self {
            palette: provider_with(stylesheets::palette(Appearance::Light)),
            text_size: provider_with(&css_for_text_size(TextSize::DEFAULT)),
            high_contrast: provider_with(""),
        }
    }

    /// Loads the palette of `appearance`.
    pub(super) fn draw_palette(&self, appearance: Appearance) {
        self.palette.load_from_string(stylesheets::palette(appearance));
        // GTK's built-in theme draws whatever the skin leaves unstyled, so
        // it switches variant too. This is the application's own setting;
        // GNOME's is never changed.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(appearance == Appearance::Dark);
        }
    }

    /// Loads the font sizes and heights of `size`.
    pub(super) fn draw_text_size(&self, size: TextSize) {
        self.text_size.load_from_string(&css_for_text_size(size));
    }

    /// Loads the high-contrast rules for [`Contrast::High`] and empties
    /// their provider for [`Contrast::Normal`].
    pub(super) fn draw_contrast(&self, contrast: Contrast) {
        let rules = match contrast {
            Contrast::Normal => "",
            Contrast::High => stylesheets::HIGH_CONTRAST_RULES,
        };
        self.high_contrast.load_from_string(rules);
    }
}

/// Uses GTK's own theme under the skin, in its light variant until the
/// skin draws dark.
fn force_builtin_theme(settings: &gtk::Settings) {
    settings.set_gtk_theme_name(Some("Default"));
    settings.set_gtk_application_prefer_dark_theme(false);
}

/// A provider loaded with `css`.
fn provider_with(css: &str) -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    provider
}

/// Adds `provider` to `display` at `layer`.
fn add_to_display(display: &gdk::Display, provider: &gtk::CssProvider, layer: Layer) {
    gtk::style_context_add_provider_for_display(display, provider, layer.priority());
}
