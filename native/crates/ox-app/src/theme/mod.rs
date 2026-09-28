// SPDX-License-Identifier: AGPL-3.0-only
//! The Explorer skin: stylesheets, light and dark palettes and text size.
//!
//! Ports `applyTheme` in `desktop/ui/app.js` and `system_dark` /
//! `apply_native_theme` in `desktop/winspace.py`. GTK's built-in theme is
//! forced underneath the skin so the desktop theme (Zorin's) cannot leak
//! into it; only this application's GTK settings change, never GNOME's.

pub(crate) mod contrast;
mod fonts;
mod preference;
mod stylesheets;
pub(crate) mod system;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;

pub(crate) use fonts::css_for_text_size;
pub use preference::{Appearance, ThemePreference};

use crate::text_size;
use contrast::Contrast;

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

/// A window's callback for [`SkinChange`]s. Shared, so [`Skin::notify`]
/// can call it after releasing the list.
struct SkinListener {
    id: ListenerId,
    callback: Rc<dyn Fn(SkinChange)>,
}

/// The CSS providers of one display, shared by every window.
///
/// The skin also remembers the desktop's appearance, so a window can
/// change the [`ThemePreference`] without asking the desktop again.
pub struct Skin {
    /// The colour tokens of the drawn [`Appearance`].
    palette_provider: gtk::CssProvider,
    /// Font sizes and heights for the text size.
    text_size_provider: gtk::CssProvider,
    /// The high-contrast rules, empty at normal contrast.
    contrast_provider: gtk::CssProvider,
    /// The appearance the palette draws.
    appearance: Cell<Appearance>,
    /// Whether the high-contrast rules are loaded.
    contrast: Cell<Contrast>,
    /// The text size in percent, always one of the levels.
    text_size: Cell<u32>,
    /// The user's theme choice.
    preference: Cell<ThemePreference>,
    /// The desktop's colour scheme, which [`ThemePreference::System`]
    /// follows.
    desktop_appearance: Cell<Appearance>,
    /// The number the next [`ListenerId`] gets.
    next_listener: Cell<usize>,
    /// The windows' callbacks, in registration order.
    listeners: RefCell<Vec<SkinListener>>,
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
    /// Forces GTK's built-in theme and installs the skin on `display`, one
    /// provider per [`Layer`].
    pub(crate) fn install(display: &gdk::Display) -> Self {
        force_builtin_theme(&gtk::Settings::for_display(display));
        add_provider(display, stylesheets::RULES, Layer::Rules);
        let default_text_size = css_for_text_size(text_size::DEFAULT);
        let text_size_provider = add_provider(display, &default_text_size, Layer::TextSize);
        let light_palette = stylesheets::palette(Appearance::Light);
        let palette_provider = add_provider(display, light_palette, Layer::Palette);
        let contrast_provider = add_provider(display, "", Layer::HighContrast);
        Self {
            palette_provider,
            text_size_provider,
            contrast_provider,
            appearance: Cell::new(Appearance::Light),
            contrast: Cell::new(Contrast::Normal),
            text_size: Cell::new(text_size::DEFAULT),
            preference: Cell::new(ThemePreference::System),
            desktop_appearance: Cell::new(Appearance::Light),
            next_listener: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
        }
    }

    /// The appearance currently drawn.
    pub(crate) fn appearance(&self) -> Appearance {
        self.appearance.get()
    }

    /// The display-wide preference shared by every window.
    pub(crate) fn preference(&self) -> ThemePreference {
        self.preference.get()
    }

    /// Applies a shared preference and synchronizes all window palettes.
    /// Listeners hear about it even when the drawn appearance stays the
    /// same, so every window's Appearance menu shows the new choice.
    pub(crate) fn set_preference(&self, preference: ThemePreference) {
        let changed = self.preference.replace(preference) != preference;
        let redrawn = self.draw(preference.resolve(self.desktop_appearance.get()));
        if changed && !redrawn {
            self.notify_appearance();
        }
    }

    /// Records the desktop's colour scheme and follows it when the
    /// preference is [`ThemePreference::System`].
    pub(crate) fn set_desktop_appearance(&self, desktop: Appearance) {
        self.desktop_appearance.set(desktop);
        self.draw(self.preference().resolve(desktop));
    }

    /// Switches the palette to `appearance` and tells the listeners.
    /// Returns false when it was drawn already.
    fn draw(&self, appearance: Appearance) -> bool {
        if self.appearance.replace(appearance) == appearance {
            return false;
        }
        self.palette_provider
            .load_from_string(stylesheets::palette(appearance));
        // GTK's built-in theme draws whatever the skin leaves unstyled, so
        // it switches variant too. This is the application's own setting;
        // GNOME's is never changed.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(appearance == Appearance::Dark);
        }
        self.notify_appearance();
        true
    }

    /// The text size in percent.
    pub(crate) fn text_size(&self) -> u32 {
        self.text_size.get()
    }

    /// Applies a text size in percent, or the default for a size that is
    /// not one of the levels, and tells the listeners when it changed.
    pub(crate) fn set_text_size(&self, percent: u32) {
        let percent = text_size::normalize(percent);
        if self.text_size.replace(percent) == percent {
            return;
        }
        self.text_size_provider
            .load_from_string(&css_for_text_size(percent));
        self.notify(SkinChange::TextSize(percent));
    }

    /// The contrast drawn now, for tests that follow the desktop setting.
    #[cfg(test)]
    pub(crate) fn contrast(&self) -> Contrast {
        self.contrast.get()
    }

    /// Adds the high-contrast rules for [`Contrast::High`] and removes
    /// them for [`Contrast::Normal`].
    pub(crate) fn set_contrast(&self, contrast: Contrast) {
        if self.contrast.replace(contrast) == contrast {
            return;
        }
        let rules = match contrast {
            Contrast::Normal => "",
            Contrast::High => stylesheets::HIGH_CONTRAST_RULES,
        };
        self.contrast_provider.load_from_string(rules);
    }

    /// Registers a window's callback for palette and text-size changes;
    /// disconnect it when the window closes.
    ///
    /// # Panics
    ///
    /// Only after `usize::MAX` registrations in one process.
    pub(crate) fn connect_changed(&self, callback: impl Fn(SkinChange) + 'static) -> ListenerId {
        let number = self.next_listener.get();
        let next = number
            .checked_add(1)
            .expect("appearance listener IDs cannot be exhausted");
        self.next_listener.set(next);
        let id = ListenerId(number);
        self.listeners.borrow_mut().push(SkinListener {
            id,
            callback: Rc::new(callback),
        });
        id
    }

    /// Removes a callback returned by [`Self::connect_changed`].
    pub(crate) fn disconnect_changed(&self, id: ListenerId) {
        self.listeners.borrow_mut().retain(|listener| listener.id != id);
    }

    /// Number of registered callbacks; closing a window must lower it.
    pub(crate) fn listener_count(&self) -> usize {
        self.listeners.borrow().len()
    }

    /// Tells the listeners which appearance is drawn now.
    fn notify_appearance(&self) {
        self.notify(SkinChange::Appearance(self.appearance()));
    }

    /// Calls every listener with `change`. They are copied out first, so a
    /// listener may connect or disconnect others.
    fn notify(&self, change: SkinChange) {
        let callbacks: Vec<_> = self
            .listeners
            .borrow()
            .iter()
            .map(|listener| Rc::clone(&listener.callback))
            .collect();
        for callback in callbacks {
            callback(change);
        }
    }
}

/// Uses GTK's own theme under the skin, in its light variant until the
/// skin draws dark.
fn force_builtin_theme(settings: &gtk::Settings) {
    settings.set_gtk_theme_name(Some("Default"));
    settings.set_gtk_application_prefer_dark_theme(false);
}

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

/// Adds a provider with `css` to `display` at `layer`, and returns it so
/// it can be reloaded.
fn add_provider(display: &gdk::Display, css: &str, layer: Layer) -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    gtk::style_context_add_provider_for_display(display, &provider, layer.priority());
    provider
}
