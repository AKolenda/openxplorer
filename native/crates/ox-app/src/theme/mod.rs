// SPDX-License-Identifier: AGPL-3.0-only
//! The Explorer skin: stylesheets, light and dark palettes and text size.
//!
//! Ports `applyTheme` in `desktop/ui/app.js` and `system_dark` /
//! `apply_native_theme` in `desktop/winspace.py`. GTK's built-in theme is
//! forced underneath the skin so the desktop theme (Zorin's) cannot leak
//! into it; only this application's GTK settings change, never GNOME's.

pub mod contrast;
mod fonts;
mod stylesheets;
pub mod system;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;

pub use fonts::css_for_text_size;

use contrast::Contrast;

use crate::icons::Glyph;

/// The appearance actually drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    /// Light surfaces with dark text.
    Light,
    /// Dark surfaces with light text.
    Dark,
}

impl Appearance {
    /// Label of the theme button ("Light" or "Dark").
    pub const fn label(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// Glyph of the theme button.
    pub const fn glyph(self) -> Glyph {
        match self {
            Appearance::Light => Glyph::Sun,
            Appearance::Dark => Glyph::Moon,
        }
    }
}

/// The user's choice in settings (`preferences.theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    /// Follow the desktop colour scheme.
    System,
    /// Use the light palette regardless of the desktop preference.
    Light,
    /// Use the dark palette regardless of the desktop preference.
    Dark,
}

impl ThemePreference {
    /// Parses `system`, `light` or `dark`; anything else means `system`,
    /// as in `applyTheme`.
    pub fn parse(value: &str) -> Self {
        Self::from_key(value).unwrap_or(ThemePreference::System)
    }

    /// The preference for an action-state key, or `None` for another value.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "system" => Some(ThemePreference::System),
            "light" => Some(ThemePreference::Light),
            "dark" => Some(ThemePreference::Dark),
            _ => None,
        }
    }

    /// The settings and action-state value.
    pub const fn key(self) -> &'static str {
        match self {
            ThemePreference::System => "system",
            ThemePreference::Light => "light",
            ThemePreference::Dark => "dark",
        }
    }

    /// The appearance for this choice given the desktop's scheme.
    pub fn resolve(self, system_dark: bool) -> Appearance {
        let dark = match self {
            ThemePreference::System => system_dark,
            ThemePreference::Light => false,
            ThemePreference::Dark => true,
        };
        if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        }
    }

    /// Tooltip of the theme button, as in `applyTheme`.
    pub fn tooltip(self, appearance: Appearance) -> String {
        let shown = match self {
            ThemePreference::System => {
                let current = appearance.label().to_lowercase();
                format!("System ({current})")
            }
            other => other.key().to_string(),
        };
        format!("Appearance: {shown}. Click to change.")
    }
}

/// What changed in a [`Skin`], as its listeners hear it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinChange {
    /// The palette or the theme preference changed; this appearance is
    /// drawn now.
    Appearance(Appearance),
    /// Text is drawn at this size now, in percent.
    TextSize(u32),
}

/// Identifies a callback registered with [`Skin::connect_changed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenerId(usize);

/// The CSS providers of one display, shared by every window.
///
/// The skin also remembers whether the desktop prefers dark, so a window
/// can change the [`ThemePreference`] without asking the desktop again.
pub struct Skin {
    palette: gtk::CssProvider,
    text: gtk::CssProvider,
    contrast_rules: gtk::CssProvider,
    appearance: Cell<Appearance>,
    contrast: Cell<Contrast>,
    text_size: Cell<u32>,
    preference: Cell<ThemePreference>,
    system_dark: Cell<bool>,
    next_listener: Cell<usize>,
    listeners: RefCell<Vec<SkinListener>>,
}

struct SkinListener {
    id: ListenerId,
    callback: Rc<dyn Fn(SkinChange)>,
}

impl std::fmt::Debug for Skin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Skin")
            .field("appearance", &self.appearance.get())
            .field("preference", &self.preference.get())
            .field("text_size", &self.text_size.get())
            .field("listeners", &self.listener_count())
            .finish_non_exhaustive()
    }
}

impl Skin {
    /// Forces GTK's built-in theme and installs the skin on `display`.
    pub fn install(display: &gdk::Display) -> Self {
        force_builtin_theme(&gtk::Settings::for_display(display));
        let base = gtk::CssProvider::new();
        base.load_from_string(stylesheets::RULES);
        let palette = gtk::CssProvider::new();
        palette.load_from_string(stylesheets::palette(Appearance::Light));
        let text = gtk::CssProvider::new();
        text.load_from_string(&css_for_text_size(crate::text_size::DEFAULT));
        let contrast_rules = gtk::CssProvider::new();
        let priority = gtk::STYLE_PROVIDER_PRIORITY_APPLICATION;
        gtk::style_context_add_provider_for_display(display, &base, priority);
        gtk::style_context_add_provider_for_display(display, &text, priority + 1);
        gtk::style_context_add_provider_for_display(display, &palette, priority + 2);
        gtk::style_context_add_provider_for_display(display, &contrast_rules, priority + 3);
        Self {
            palette,
            text,
            contrast_rules,
            appearance: Cell::new(Appearance::Light),
            contrast: Cell::new(Contrast::Normal),
            text_size: Cell::new(crate::text_size::DEFAULT),
            preference: Cell::new(ThemePreference::System),
            system_dark: Cell::new(false),
            next_listener: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
        }
    }

    /// The appearance currently drawn.
    pub fn appearance(&self) -> Appearance {
        self.appearance.get()
    }

    /// Switches the palette. Returns false when nothing changed.
    pub fn set_appearance(&self, appearance: Appearance) -> bool {
        if self.appearance.replace(appearance) == appearance {
            return false;
        }
        self.palette.load_from_string(stylesheets::palette(appearance));
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(appearance == Appearance::Dark);
        }
        self.notify_appearance();
        true
    }

    /// The display-wide preference shared by every window.
    pub fn preference(&self) -> ThemePreference {
        self.preference.get()
    }

    /// Applies a shared preference and synchronizes all window palettes.
    /// Listeners hear about it even when the drawn appearance stays the
    /// same, so every window's Appearance menu shows the new choice.
    pub fn set_preference(&self, preference: ThemePreference) {
        let changed = self.preference.replace(preference) != preference;
        let redrawn = self.set_appearance(preference.resolve(self.system_dark.get()));
        if changed && !redrawn {
            self.notify_appearance();
        }
    }

    /// Records the desktop's colour scheme and follows it when the
    /// preference is [`ThemePreference::System`].
    pub fn set_system_dark(&self, dark: bool) {
        self.system_dark.set(dark);
        self.set_appearance(self.preference().resolve(dark));
    }

    /// Registers a window's callback for palette and text-size changes;
    /// disconnect it when the window closes.
    ///
    /// # Panics
    ///
    /// Only after `usize::MAX` registrations in one process.
    pub fn connect_changed(&self, listener: impl Fn(SkinChange) + 'static) -> ListenerId {
        let id = self.next_listener.get();
        let next = id
            .checked_add(1)
            .expect("appearance listener IDs cannot be exhausted");
        self.next_listener.set(next);
        self.listeners.borrow_mut().push(SkinListener {
            id: ListenerId(id),
            callback: Rc::new(listener),
        });
        ListenerId(id)
    }

    /// Removes a callback returned by [`Self::connect_changed`].
    pub fn disconnect_changed(&self, id: ListenerId) {
        self.listeners.borrow_mut().retain(|listener| listener.id != id);
    }

    /// Number of registered callbacks; closing a window must lower it.
    pub fn listener_count(&self) -> usize {
        self.listeners.borrow().len()
    }

    fn notify_appearance(&self) {
        self.notify(SkinChange::Appearance(self.appearance()));
    }

    /// Calls every listener with `change`. They are copied out first, so a
    /// listener may connect or disconnect others.
    fn notify(&self, change: SkinChange) {
        let listeners: Vec<_> = self
            .listeners
            .borrow()
            .iter()
            .map(|listener| Rc::clone(&listener.callback))
            .collect();
        for listener in listeners {
            listener(change);
        }
    }

    /// The text size in percent.
    pub fn text_size(&self) -> u32 {
        self.text_size.get()
    }

    /// The contrast drawn now.
    pub fn contrast(&self) -> Contrast {
        self.contrast.get()
    }

    /// Adds the high-contrast rules for [`Contrast::High`] and removes
    /// them for [`Contrast::Normal`].
    pub fn set_contrast(&self, contrast: Contrast) {
        if self.contrast.replace(contrast) == contrast {
            return;
        }
        let rules = match contrast {
            Contrast::Normal => "",
            Contrast::High => stylesheets::HIGH_CONTRAST_RULES,
        };
        self.contrast_rules.load_from_string(rules);
    }

    /// Applies a text size (percent) and tells the listeners. Returns false
    /// when nothing changed.
    pub fn set_text_size(&self, percent: u32) -> bool {
        let percent = crate::text_size::normalize(percent);
        if self.text_size.replace(percent) == percent {
            return false;
        }
        self.text.load_from_string(&css_for_text_size(percent));
        self.notify(SkinChange::TextSize(percent));
        true
    }
}

/// Uses GTK's own theme under the skin, in the matching variant.
fn force_builtin_theme(settings: &gtk::Settings) {
    settings.set_gtk_theme_name(Some("Default"));
    settings.set_gtk_application_prefer_dark_theme(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_parse_like_apply_theme() {
        assert_eq!(ThemePreference::parse("dark"), ThemePreference::Dark);
        assert_eq!(ThemePreference::parse("light"), ThemePreference::Light);
        assert_eq!(ThemePreference::parse("sepia"), ThemePreference::System);
    }

    #[test]
    fn system_follows_the_desktop() {
        assert_eq!(ThemePreference::System.resolve(true), Appearance::Dark);
        assert_eq!(ThemePreference::System.resolve(false), Appearance::Light);
        assert_eq!(ThemePreference::Light.resolve(true), Appearance::Light);
        assert_eq!(ThemePreference::Dark.resolve(false), Appearance::Dark);
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
