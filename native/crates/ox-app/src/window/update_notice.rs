// SPDX-License-Identifier: AGPL-3.0-only
//! The update notice: a card at the top right of the window that pops up
//! when the daily check at launch finds a newer release (UPD-001).
//!
//! It names the release and the running version, and offers "Install
//! update…" (or "Details…" for a build that updates through its package
//! manager), which opens Software updates, and Later, which dismisses the
//! release's notice for good. The close button dismisses it too. The
//! command bar's Update (unless Settings › About turns it off) and the
//! status bar's "Update available" stay until the update is installed, so
//! dismissing loses nothing.
//!
//! [`UpdateNotice`] is a widget subclass the window's overlay holds; the
//! window decides when it shows ([`super::BrowserWindow`]'s updates).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::notice_card;
use crate::icons::Icon;
use crate::update::AvailableUpdate;

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
    /// The card: the glyph, the title and text, "Install update…" or
    /// "Details…" and Later, and the close button at the top right.
    fn build(&self) {
        let card = notice_card::build(
            self,
            Icon::ArrowDownload,
            &ox_core::i18n::gettext("Update available"),
        );
        card.secondary.set_label(&ox_core::i18n::gettext("Later"));
        let imp = self.imp();
        imp.surface.set(card.surface).expect("constructed runs once");
        imp.text.set(card.text).expect("constructed runs once");
        imp.open_button.set(card.primary).expect("constructed runs once");
        imp.later_button
            .set(card.secondary)
            .expect("constructed runs once");
        imp.close_button.set(card.close).expect("constructed runs once");
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
