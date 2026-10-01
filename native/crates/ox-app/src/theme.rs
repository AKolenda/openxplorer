// SPDX-License-Identifier: AGPL-3.0-only
//! The Explorer skin: stylesheets, light and dark palettes and text size.
//!
//! Ports `applyTheme` in `v2.0.0:desktop/ui/app.js` and `system_dark` /
//! `apply_native_theme` in `v2.0.0:desktop/winspace.py`. One [`Skin`] per display
//! holds the providers ([`providers`]) and what they draw; windows connect
//! to its `appearance-changed` and `text-size-changed` signals, as they
//! connect to `places-changed` on the shared
//! [`AppContext`](crate::app_context::AppContext).

pub(crate) mod accent;
mod appearance_button;
pub(crate) mod contrast;
pub(crate) mod desktop_text;
mod fonts;
mod providers;
mod stylesheets;
pub(crate) mod system;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

pub(crate) use appearance_button::{tooltip, AppearanceExt};
use ox_core::settings::{Appearance, Theme};

use crate::icons;
use crate::text_size::TextSize;
use accent::Accent;
use contrast::Contrast;
use desktop_text::{drawn_text_size, DesktopText};
use providers::Providers;

/// Emitted when the palette or the theme choice changed.
const APPEARANCE_CHANGED: &str = "appearance-changed";

/// Emitted when text is drawn at another size.
const TEXT_SIZE_CHANGED: &str = "text-size-changed";

/// The text the skin draws: its size and, when the desktop's font is in
/// use, that font's family.
#[derive(Debug, Clone, PartialEq)]
struct DrawnText {
    size: TextSize,
    family: Option<String>,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;

    use super::{Accent, Appearance, Contrast, DesktopText, DrawnText, Providers, TextSize, Theme};
    use super::{APPEARANCE_CHANGED, TEXT_SIZE_CHANGED};

    /// Private state of [`super::Skin`].
    #[derive(Debug, Default)]
    pub(crate) struct Skin {
        /// The providers the skin reloads; set once, when it is created.
        pub(super) providers: OnceCell<Providers>,
        /// The appearance the palette draws.
        pub(super) appearance: Cell<Appearance>,
        /// The desktop's accent drawn over the palette.
        pub(super) accent: Cell<Accent>,
        /// Whether the high-contrast rules are loaded.
        pub(super) contrast: Cell<Contrast>,
        /// The text size the user chose.
        pub(super) text_size: Cell<TextSize>,
        /// The text size and desktop font family drawn, once drawn.
        pub(super) drawn_text: RefCell<Option<DrawnText>>,
        /// What the desktop asks of text.
        pub(super) desktop_text: RefCell<DesktopText>,
        /// Whether text uses the desktop's font instead of the Windows one.
        pub(super) uses_desktop_font: Cell<bool>,
        /// The user's theme choice.
        pub(super) theme: Cell<Theme>,
        /// The desktop's colour scheme, which [`Theme::System`] follows.
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
    /// change the [`Theme`] without asking the desktop again.
    pub(crate) struct Skin(ObjectSubclass<imp::Skin>);
}

impl Skin {
    /// Forces GTK's built-in theme and installs the skin on `display`: its
    /// stylesheets and the app's bundled icons.
    pub(crate) fn install(display: &gdk::Display) -> Self {
        icons::register(display);
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

    /// The providers the skin draws with.
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

    /// The display-wide theme choice shared by every window.
    pub(crate) fn theme(&self) -> Theme {
        self.imp().theme.get()
    }

    /// Applies a shared theme choice and synchronizes all window palettes.
    /// Windows hear `appearance-changed` even when the drawn appearance
    /// stays the same, so every window's Appearance menu shows the new
    /// choice.
    pub(crate) fn set_theme(&self, theme: Theme) {
        let imp = self.imp();
        let changed = imp.theme.replace(theme) != theme;
        let appearance = theme.appearance(imp.desktop_appearance.get());
        if appearance != self.appearance() {
            self.draw(appearance);
        } else if changed {
            self.emit_by_name::<()>(APPEARANCE_CHANGED, &[]);
        }
    }

    /// Records the desktop's colour scheme and follows it when the theme
    /// is [`Theme::System`].
    pub(crate) fn set_desktop_appearance(&self, desktop: Appearance) {
        self.imp().desktop_appearance.set(desktop);
        self.draw(self.theme().appearance(desktop));
    }

    /// Switches the palette to `appearance` and tells the windows; does
    /// nothing when it is drawn already.
    fn draw(&self, appearance: Appearance) {
        if self.imp().appearance.replace(appearance) == appearance {
            return;
        }
        self.providers().draw_palette(appearance);
        self.providers().draw_accent(self.accent(), appearance);
        self.emit_by_name::<()>(APPEARANCE_CHANGED, &[]);
    }

    /// The text size the user chose, which Ctrl+plus steps from and
    /// Settings shows.
    pub(crate) fn text_size(&self) -> TextSize {
        self.imp().text_size.get()
    }

    /// The text size drawn: the chosen one at the desktop's text factor,
    /// which the windows lay rows and tiles out for.
    pub(crate) fn drawn_text_size(&self) -> TextSize {
        let drawn = self.imp().drawn_text.borrow();
        drawn
            .as_ref()
            .map_or_else(|| self.text_size(), |drawn| drawn.size)
    }

    /// Chooses the text size `size` and tells the windows when it changed.
    pub(crate) fn set_text_size(&self, size: TextSize) {
        let chosen_changed = self.imp().text_size.replace(size) != size;
        let drawn_changed = self.draw_text();
        if chosen_changed && !drawn_changed {
            self.emit_by_name::<()>(TEXT_SIZE_CHANGED, &[]);
        }
    }

    /// Records what the desktop asks of text and redraws it.
    pub(crate) fn set_desktop_text(&self, desktop: DesktopText) {
        if *self.imp().desktop_text.borrow() == desktop {
            return;
        }
        self.imp().desktop_text.replace(desktop);
        self.draw_text();
    }

    /// Uses the desktop's font, or the Windows font stack, and redraws text.
    pub(crate) fn set_uses_desktop_font(&self, uses_desktop_font: bool) {
        if self.imp().uses_desktop_font.replace(uses_desktop_font) != uses_desktop_font {
            self.draw_text();
        }
    }

    /// Draws text at the chosen size, the desktop's factor and the font in
    /// use; tells the windows and returns true when that changed anything.
    fn draw_text(&self) -> bool {
        let imp = self.imp();
        let uses_desktop_font = imp.uses_desktop_font.get();
        let desktop = imp.desktop_text.borrow().clone();
        let text = DrawnText {
            size: drawn_text_size(imp.text_size.get(), desktop.factor(uses_desktop_font)),
            family: desktop.family.filter(|_| uses_desktop_font),
        };
        if imp.drawn_text.borrow().as_ref() == Some(&text) {
            return false;
        }
        self.providers().draw_text_size(text.size, text.family.as_deref());
        imp.drawn_text.replace(Some(text));
        self.emit_by_name::<()>(TEXT_SIZE_CHANGED, &[]);
        true
    }

    /// The desktop's accent drawn now.
    pub(crate) fn accent(&self) -> Accent {
        self.imp().accent.get()
    }

    /// Draws `accent` over the palette.
    pub(crate) fn set_accent(&self, accent: Accent) {
        if self.imp().accent.replace(accent) == accent {
            return;
        }
        self.providers().draw_accent(accent, self.appearance());
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

/// GNOME's interface schema, which holds the desktop's look and clock.
pub(crate) const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";

/// The host desktop's settings under `schema_id`, when that schema is
/// installed with `key`. `None` inside Flatpak, where the schema holds only
/// the runtime's defaults, not the desktop's.
pub(crate) fn host_desktop_key(schema_id: &str, key: &str) -> Option<gio::Settings> {
    if ox_core::integration::Sandbox::detect().is_flatpak() {
        return None;
    }
    let settings = desktop_settings(schema_id)?;
    let has_key = settings.settings_schema()?.has_key(key);
    has_key.then_some(settings)
}

/// The desktop's settings under `schema_id` (GNOME's interface and
/// accessibility keys), when that schema is installed.
pub(crate) fn desktop_settings(schema_id: &str) -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(schema_id, true)?;
    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
    Some(settings)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::test_support::harness::{self, ThemeGuard};

    /// Counts the `appearance-changed` signals `skin` emits.
    fn count_appearance_changes(skin: &Skin) -> Rc<Cell<u32>> {
        let count = Rc::new(Cell::new(0));
        let counter = Rc::clone(&count);
        skin.connect_appearance_changed(move || counter.set(counter.get() + 1));
        count
    }

    /// A new theme choice is announced even when the palette stays, so
    /// every window's Appearance menu shows it; the same choice again is
    /// not announced.
    #[gtk::test]
    fn a_new_theme_choice_is_announced_even_when_the_palette_stays() {
        let skin = Skin::detached();
        skin.set_desktop_appearance(Appearance::Light);
        let changes = count_appearance_changes(&skin);

        skin.set_theme(Theme::Light);
        assert_eq!(changes.get(), 1, "System to Light on a light desktop");

        skin.set_theme(Theme::Light);
        assert_eq!(changes.get(), 1, "Light chosen again");

        skin.set_theme(Theme::Dark);
        assert_eq!(changes.get(), 2, "Light to Dark redraws once");
        assert_eq!(skin.appearance(), Appearance::Dark);
    }

    /// [`Theme::System`] draws the desktop's colour scheme and follows it
    /// when it changes; Light and Dark ignore it (`applyTheme`).
    ///
    /// parity: LOOK-003
    #[gtk::test]
    fn system_follows_the_desktop() {
        let skin = Skin::detached();
        skin.set_theme(Theme::System);
        skin.set_desktop_appearance(Appearance::Dark);
        assert_eq!(skin.appearance(), Appearance::Dark);
        skin.set_desktop_appearance(Appearance::Light);
        assert_eq!(skin.appearance(), Appearance::Light);

        skin.set_theme(Theme::Light);
        skin.set_desktop_appearance(Appearance::Dark);
        assert_eq!(skin.appearance(), Appearance::Light);

        skin.set_theme(Theme::Dark);
        skin.set_desktop_appearance(Appearance::Light);
        assert_eq!(skin.appearance(), Appearance::Dark);
    }

    /// The accent the desktop asks for replaces the Windows blue in what
    /// the skin draws, in each appearance, and blue brings it back.
    ///
    /// parity: LOOK-024
    #[gtk::test]
    fn the_desktop_accent_replaces_the_windows_blue() {
        #[expect(deprecated, reason = "GTK 4.14 offers no other lookup of a named colour")]
        fn accent_of(widget: &gtk::Label) -> Option<gdk::RGBA> {
            widget.style_context().lookup_color("ox_accent")
        }
        let _theme = ThemeGuard::keep();
        let skin = harness::skin();
        let label = gtk::Label::new(None);
        skin.set_theme(Theme::Light);
        let windows_blue = accent_of(&label);
        assert_eq!(windows_blue, gdk::RGBA::parse("#0067c0").ok());

        skin.set_accent(Accent::Green);
        assert_eq!(accent_of(&label), gdk::RGBA::parse("#2e763b").ok());
        skin.set_theme(Theme::Dark);
        assert_eq!(accent_of(&label), gdk::RGBA::parse("#8dc196").ok());

        skin.set_accent(Accent::Windows);
        assert_eq!(accent_of(&label), gdk::RGBA::parse("#74beff").ok());
    }

    /// GTK's own widgets under the skin, such as its dialogs, follow the
    /// drawn palette through the dark variant of the skin's display; the
    /// desktop's own colour scheme is never written.
    ///
    /// parity: LOOK-006
    #[gtk::test]
    fn the_drawn_palette_switches_the_display_dark_variant() {
        let _theme = ThemeGuard::keep();
        let display = gdk::Display::default().expect("GTK tests run on a private display");
        let display_settings = gtk::Settings::for_display(&display);
        let desktop = desktop_settings(INTERFACE_SCHEMA).filter(|settings| {
            let schema = settings.settings_schema();
            schema.is_some_and(|schema| schema.has_key("color-scheme"))
        });
        let desktop_scheme = || desktop.as_ref().map(|settings| settings.string("color-scheme"));
        let scheme_before = desktop_scheme();

        harness::skin().set_theme(Theme::Dark);
        assert!(display_settings.is_gtk_application_prefer_dark_theme());

        harness::skin().set_theme(Theme::Light);
        assert!(!display_settings.is_gtk_application_prefer_dark_theme());
        assert_eq!(desktop_scheme(), scheme_before, "GNOME's setting is left alone");
    }
}
