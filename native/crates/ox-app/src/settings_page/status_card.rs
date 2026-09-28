// SPDX-License-Identifier: AGPL-3.0-only
//! A status card: the tinted summary at the top of a category, with its
//! main actions at the right.
//!
//! Replaces a Python section's summary and main buttons in
//! `renderSettingsPage` (`desktop/ui/app.js`), such as the "Default file
//! explorer" card and its main button, in the look of the settings
//! mockup's `.hero`. The static layout is the template
//! `resources/ui/status-card.ui`. The settings search finds a card by its
//! title, its text and its buttons' labels, so "make default" and "refresh
//! all" find the buttons that say so.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::row::PageWidth;
use super::search::{jump_to, shown_text, SearchQuery};
use crate::icons::{self, Icon};
use crate::window::children;

/// The glyph in a status card's round badge.
const STATUS_GLYPH: i32 = 22;

/// What a status card says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusText<'a> {
    /// The glyph in the round badge.
    pub glyph: Icon,
    /// The one-line state, such as "Instant search".
    pub title: &'a str,
    /// The line under it.
    pub text: &'a str,
    /// The milestone that brings the card's actions, when the native
    /// preview lacks them.
    pub notice: Option<&'a str>,
}

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::StatusCard`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/status-card.ui")]
    pub(crate) struct StatusCard {
        /// The glyph in the round badge.
        #[template_child]
        pub(super) glyph: TemplateChild<gtk::Image>,
        /// The one-line state.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// The line under it.
        #[template_child]
        pub(super) text_label: TemplateChild<gtk::Label>,
        /// The milestone that brings the card's actions.
        #[template_child]
        pub(super) notice_label: TemplateChild<gtk::Label>,
        /// The main actions.
        #[template_child]
        pub(super) actions: TemplateChild<gtk::Box>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StatusCard {
        const NAME: &'static str = "OxStatusCard";
        type Type = super::StatusCard;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(card: &glib::subclass::InitializingObject<Self>) {
            card.init_template();
        }
    }

    impl ObjectImpl for StatusCard {}
    impl WidgetImpl for StatusCard {}
    impl BoxImpl for StatusCard {}
}

glib::wrapper! {
    /// The summary of a category, at the top of its page.
    pub(crate) struct StatusCard(ObjectSubclass<imp::StatusCard>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl StatusCard {
    /// A card saying `status`, with `actions` at its right.
    pub(crate) fn new(status: StatusText<'_>, actions: &[gtk::Widget]) -> Self {
        let card: Self = glib::Object::new();
        let imp = card.imp();
        icons::set_icon(&imp.glyph, status.glyph, STATUS_GLYPH);
        imp.title_label.set_text(status.title);
        imp.text_label.set_text(status.text);
        if let Some(notice) = status.notice {
            imp.notice_label.set_text(notice);
            imp.notice_label.set_visible(true);
        }
        for action in actions {
            imp.actions.append(action);
        }
        card
    }

    /// Says `title` as the card's state, such as whether `OpenXplorer` is
    /// the default file explorer once that has been read.
    pub(crate) fn set_title(&self, title: &str) {
        self.imp().title_label.set_text(title);
    }

    /// Says `text` under the card's state, such as how many names are
    /// indexed.
    pub(crate) fn set_text(&self, text: &str) {
        self.imp().text_label.set_text(text);
    }

    /// The card's one-line state.
    #[cfg(test)]
    pub(crate) fn title(&self) -> String {
        self.imp().title_label.text().into()
    }

    /// Shows the card while nothing is typed or when its title, text or
    /// buttons match `query`, and says whether it matches a search.
    pub(crate) fn apply_query(&self, query: &SearchQuery) -> bool {
        let imp = self.imp();
        let title = imp.title_label.text();
        let text = imp.text_label.text();
        let actions = shown_text(&*imp.actions);
        let finding = query.find_in(&[&title, &text, &actions]);
        finding.show_on(self);
        !query.is_empty() && finding.is_shown()
    }

    /// Outlines the card as the setting the search jumped to, and gives its
    /// first working button keyboard focus. False when none works.
    pub(crate) fn jump_here(&self) -> bool {
        let actions: Vec<gtk::Widget> = children(&*self.imp().actions).collect();
        jump_to(self, &actions)
    }

    /// Lays the card out for a page `width` wide: its parts side by side,
    /// or stacked in a narrow window.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        let orientation = match width {
            PageWidth::Roomy => gtk::Orientation::Horizontal,
            PageWidth::Narrow => gtk::Orientation::Vertical,
        };
        self.set_orientation(orientation);
    }
}
