// SPDX-License-Identifier: AGPL-3.0-only
//! The Explorer skin: stylesheets, light and dark palettes and text size.
//!
//! Ports `applyTheme` in `desktop/ui/app.js` and `system_dark` /
//! `apply_native_theme` in `desktop/winspace.py`. One [`Skin`] per display
//! holds the providers ([`providers`]) and what they draw; windows connect
//! to its `appearance-changed` and `text-size-changed` signals, as they
//! connect to `places-changed` on the shared
//! [`AppContext`](crate::shared::AppContext).

pub(crate) mod contrast;
mod fonts;
mod preference;
mod providers;
mod stylesheets;
pub(crate) mod system;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

pub(crate) use preference::{Appearance, ThemePreference};

use crate::text_size::TextSize;
use contrast::Contrast;
use providers::Providers;

/// Emitted when the palette or the theme choice changed.
const APPEARANCE_CHANGED: &str = "appearance-changed";

/// Emitted when text is drawn at another size.
const TEXT_SIZE_CHANGED: &str = "text-size-changed";

mod imp {
    use std::cell::{Cell, OnceCell};
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;

    use super::{Appearance, Contrast, Providers, TextSize, ThemePreference};
    use super::{APPEARANCE_CHANGED, TEXT_SIZE_CHANGED};

    /// Private state of [`super::Skin`].
    #[derive(Debug, Default)]
    pub(crate) struct Skin {
        /// The providers the skin reloads; set once, when it is created.
        pub(super) providers: OnceCell<Providers>,
        /// The appearance the palette draws.
        pub(super) appearance: Cell<Appearance>,
        /// Whether the high-contrast rules are loaded.
        pub(super) contrast: Cell<Contrast>,
        /// The size text is drawn at.
        pub(super) text_size: Cell<TextSize>,
        /// The user's theme choice.
        pub(super) preference: Cell<ThemePreference>,
        /// The desktop's colour scheme, which [`ThemePreference::System`]
        /// follows.
        pub(super) desktop_appearance: Cell<Appearance>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Skin {
        const NAME: &'static str = "OxSkin";
        type Type = super::Skin;
    }

    impl ObjectImpl for Skin {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder(APPEARANCE_CHANGED).build(),
                    Signal::builder(TEXT_SIZE_CHANGED).build(),
                ]
            })
        }
    }
}

glib::wrapper! {
    /// The skin of one display, shared by every window: what the CSS
    /// providers draw and the choices behind it.
    ///
    /// The skin also remembers the desktop's appearance, so a window can
    /// change the [`ThemePreference`] without asking the desktop again.
    pub(crate) struct Skin(ObjectSubclass<imp::Skin>);
}

impl Skin {
    /// Forces GTK's built-in theme and installs the skin on `display`.
    pub(crate) fn install(display: &gdk::Display) -> Self {
        Self::with_providers(Providers::install(display))
    }

    /// A skin on no display, for tests that watch what windows connect to
    /// it without restyling the shared test display.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self::with_providers(Providers::detached())
    }

    /// A skin that draws with `providers`.
    ///
    /// # Panics
    ///
    /// Never: a new skin has no providers yet.
    fn with_providers(providers: Providers) -> Self {
        let skin: Self = glib::Object::new();
        skin.imp()
            .providers
            .set(providers)
            .expect("a new skin has no providers yet");
        skin
    }

    fn providers(&self) -> &Providers {
        self.imp()
            .providers
            .get()
            .expect("Skin::with_providers is the only constructor and sets the providers")
    }

    /// The appearance currently drawn.
    pub(crate) fn appearance(&self) -> Appearance {
        self.imp().appearance.get()
    }

    /// The display-wide preference shared by every window.
    pub(crate) fn preference(&self) -> ThemePreference {
        self.imp().preference.get()
    }

    /// Applies a shared preference and synchronizes all window palettes.
    /// Windows hear `appearance-changed` even when the drawn appearance
    /// stays the same, so every window's Appearance menu shows the new
    /// choice.
    pub(crate) fn set_preference(&self, preference: ThemePreference) {
        let imp = self.imp();
        let changed = imp.preference.replace(preference) != preference;
        let redrawn = self.draw(preference.resolve(imp.desktop_appearance.get()));
        if changed && !redrawn {
            self.emit_by_name::<()>(APPEARANCE_CHANGED, &[]);
        }
    }

    /// Records the desktop's colour scheme and follows it when the
    /// preference is [`ThemePreference::System`].
    pub(crate) fn set_desktop_appearance(&self, desktop: Appearance) {
        self.imp().desktop_appearance.set(desktop);
        self.draw(self.preference().resolve(desktop));
    }

    /// Switches the palette to `appearance` and tells the windows.
    /// Returns false when it was drawn already.
    fn draw(&self, appearance: Appearance) -> bool {
        if self.imp().appearance.replace(appearance) == appearance {
            return false;
        }
        self.providers().draw_palette(appearance);
        self.emit_by_name::<()>(APPEARANCE_CHANGED, &[]);
        true
    }

    /// The size text is drawn at.
    pub(crate) fn text_size(&self) -> TextSize {
        self.imp().text_size.get()
    }

    /// Draws text at `size` and tells the windows when it changed.
    pub(crate) fn set_text_size(&self, size: TextSize) {
        if self.imp().text_size.replace(size) == size {
            return;
        }
        self.providers().draw_text_size(size);
        self.emit_by_name::<()>(TEXT_SIZE_CHANGED, &[]);
    }

    /// The contrast drawn now, for tests that follow the desktop setting.
    #[cfg(test)]
    pub(crate) fn contrast(&self) -> Contrast {
        self.imp().contrast.get()
    }

    /// Adds the high-contrast rules for [`Contrast::High`] and removes
    /// them for [`Contrast::Normal`].
    pub(crate) fn set_contrast(&self, contrast: Contrast) {
        if self.imp().contrast.replace(contrast) == contrast {
            return;
        }
        self.providers().draw_contrast(contrast);
    }

    /// Calls `callback` whenever the palette or the theme choice changed;
    /// a window disconnects the returned handler when it closes.
    pub(crate) fn connect_appearance_changed(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(APPEARANCE_CHANGED, false, move |_| {
            callback();
            None
        })
    }

    /// Calls `callback` whenever text is drawn at another size; a window
    /// disconnects the returned handler when it closes.
    pub(crate) fn connect_text_size_changed(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(TEXT_SIZE_CHANGED, false, move |_| {
            callback();
            None
        })
    }

    /// True while anything is connected to the skin's signals; a closed
    /// window must leave nothing behind.
    #[cfg(test)]
    pub(crate) fn has_listeners(&self) -> bool {
        self.is_connected(APPEARANCE_CHANGED) || self.is_connected(TEXT_SIZE_CHANGED)
    }

    /// True while a handler is connected to the signal `name`.
    #[cfg(test)]
    fn is_connected(&self, name: &str) -> bool {
        let signal = glib::subclass::SignalId::lookup(name, Self::static_type())
            .expect("the skin registers both of its signals");
        glib::signal::signal_has_handler_pending(self, signal, None, true)
    }
}
