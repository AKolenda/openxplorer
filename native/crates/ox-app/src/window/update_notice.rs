// SPDX-License-Identifier: AGPL-3.0-only
//! The update notice: a card at the top right of the window that pops up
//! when the daily check at launch finds a newer release (UPD-001).
//!
//! It names the release and the running version, and offers "Install
//! update…" (or "Details…" for a build that updates through its package
//! manager), which opens Software updates, and Later, which dismisses the
//! release's notice for good. The close button dismisses it too. The
//! status bar's "Update available" stays until the update is installed,
//! so dismissing loses nothing.
//!
//! [`UpdateNotice`] is a widget subclass the window's overlay holds; the
//! window decides when it shows ([`super::BrowserWindow`]'s updates).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};
use crate::update::AvailableUpdate;

/// The glyph beside the title.
const NOTICE_GLYPH: i32 = 20;

/// The glyph of the close button.
const CLOSE_GLYPH: i32 = 16;

/// The widest the text gets before it wraps, in characters.
const MAX_WIDTH_CHARS: i32 = 34;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::UpdateNotice`].
    #[derive(Debug, Default)]
    pub(crate) struct UpdateNotice {
        /// The card.
        pub(super) surface: OnceCell<gtk::Box>,
        /// What the notice says about the release.
        pub(super) text: OnceCell<gtk::Label>,
        /// "Install update…" or "Details…".
        pub(super) open_button: OnceCell<gtk::Button>,
        /// Later.
        pub(super) later_button: OnceCell<gtk::Button>,
        /// The close button.
        pub(super) close_button: OnceCell<gtk::Button>,
        /// The release shown, while the notice shows.
        pub(super) shown: RefCell<Option<super::AvailableUpdate>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UpdateNotice {
        const NAME: &'static str = "OxUpdateNotice";
        type Type = super::UpdateNotice;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }
    }

    impl ObjectImpl for UpdateNotice {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(surface) = self.surface.get() {
                surface.unparent();
            }
        }
    }

    impl WidgetImpl for UpdateNotice {}
}

glib::wrapper! {
    /// The update notice, hidden until the window shows a release in it.
    pub(crate) struct UpdateNotice(ObjectSubclass<imp::UpdateNotice>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl UpdateNotice {
    /// The card: the glyph, the title and text, the two buttons, and the
    /// close button at the top right.
    fn build(&self) {
        self.set_halign(gtk::Align::End);
        self.set_valign(gtk::Align::Start);
        self.set_visible(false);
        let title_text = ox_core::i18n::gettext("Update available");
        self.update_property(&[gtk::accessible::Property::Label(&title_text)]);

        let glyph = icons::image(Icon::ArrowDownload, NOTICE_GLYPH);
        glyph.set_valign(gtk::Align::Start);
        glyph.add_css_class("update-notice-glyph");

        let title = gtk::Label::builder()
            .label(&title_text)
            .xalign(0.0)
            .css_classes(["update-notice-title"])
            .build();
        let text = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(MAX_WIDTH_CHARS)
            .accessible_role(gtk::AccessibleRole::Status)
            .css_classes(["update-notice-text"])
            .build();
        let open_button = gtk::Button::builder().css_classes(["accent"]).build();
        let later_button = gtk::Button::builder()
            .label(ox_core::i18n::gettext("Later"))
            .css_classes(["bordered"])
            .build();
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.add_css_class("update-notice-buttons");
        buttons.append(&open_button);
        buttons.append(&later_button);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_hexpand(true);
        column.append(&title);
        column.append(&text);
        column.append(&buttons);

        let close_button = gtk::Button::builder()
            .child(&icons::image(Icon::Dismiss, CLOSE_GLYPH))
            .valign(gtk::Align::Start)
            .tooltip_text(ox_core::i18n::gettext("Dismiss"))
            .css_classes(["update-notice-close"])
            .build();
        close_button.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "Dismiss",
        ))]);

        let surface = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .css_classes(["update-notice"])
            .build();
        surface.append(&glyph);
        surface.append(&column);
        surface.append(&close_button);
        surface.set_parent(self);

        let imp = self.imp();
        imp.surface.set(surface).expect("constructed runs once");
        imp.text.set(text).expect("constructed runs once");
        imp.open_button.set(open_button).expect("constructed runs once");
        imp.later_button.set(later_button).expect("constructed runs once");
        imp.close_button.set(close_button).expect("constructed runs once");
    }

    /// Calls `open` when the notice's "Install update…" or "Details…" is
    /// clicked, and `dismiss` on Later and the close button; each gets the
    /// release shown.
    pub(crate) fn connect_choices(
        &self,
        open: impl Fn(AvailableUpdate) + 'static,
        dismiss: impl Fn(AvailableUpdate) + Clone + 'static,
    ) {
        let imp = self.imp();
        let shown = |notice: &Self| *notice.imp().shown.borrow();
        let open_button = imp.open_button.get().expect("built at construction");
        open_button.connect_clicked(glib::clone!(
            #[weak(rename_to = notice)]
            self,
            move |_| {
                if let Some(update) = shown(&notice) {
                    open(update);
                }
            }
        ));
        for button in [imp.later_button.get(), imp.close_button.get()]
            .into_iter()
            .flatten()
        {
            let dismiss = dismiss.clone();
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = notice)]
                self,
                move |_| {
                    if let Some(update) = shown(&notice) {
                        dismiss(update);
                    }
                }
            ));
        }
    }

    /// Pops the notice up for `update`, or hides it for `None`.
    pub(crate) fn show_update(&self, update: Option<AvailableUpdate>) {
        let imp = self.imp();
        if *imp.shown.borrow() == update {
            return;
        }
        imp.shown.replace(update);
        let Some(update) = update else {
            self.set_visible(false);
            return;
        };
        if let Some(text) = imp.text.get() {
            text.set_text(&update.notice_text());
        }
        if let Some(button) = imp.open_button.get() {
            button.set_label(update.notice_button());
        }
        self.set_visible(true);
    }

    /// The release the notice shows, for tests.
    #[cfg(test)]
    pub(crate) fn shown_update(&self) -> Option<AvailableUpdate> {
        *self.imp().shown.borrow()
    }

    /// The notice's text, for tests.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.imp()
            .text
            .get()
            .map(|text| text.text().to_string())
            .unwrap_or_default()
    }

    /// Clicks Later, for tests.
    #[cfg(test)]
    pub(crate) fn click_later(&self) {
        if let Some(button) = self.imp().later_button.get() {
            button.emit_clicked();
        }
    }
}
