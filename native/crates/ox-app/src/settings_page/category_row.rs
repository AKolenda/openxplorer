// SPDX-License-Identifier: AGPL-3.0-only
//! One category in the Settings list: the accent bar of the chosen row and
//! the category's coloured glyph and name. A search changes nothing the
//! row shows; it only keeps how many of the category's settings match.
//!
//! Ports the section links of `renderSettingsPage` in `v2.0.0:desktop/ui/app.js`
//! (`.settings-nav-link`). The static layout is the template
//! `resources/ui/category-row.ui`. Each row knows its [`Category`], so the
//! list never maps rows to categories by their position.

use std::cell::{Cell, OnceCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::pages::Category;
use crate::icons;

/// A category's glyph in the list.
const CATEGORY_GLYPH: i32 = 18;

mod imp {
    use super::{Category, Cell, OnceCell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::CategoryRow`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/category-row.ui")]
    pub(crate) struct CategoryRow {
        /// The category's coloured glyph.
        #[template_child]
        pub(super) glyph: TemplateChild<gtk::Image>,
        /// The category's name.
        #[template_child]
        pub(super) title: TemplateChild<gtk::Label>,
        /// The category the row stands for, set once by `CategoryRow::new`.
        pub(super) category: OnceCell<Category>,
        /// How many of its settings match the search typed now.
        pub(super) matches: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CategoryRow {
        const NAME: &'static str = "OxCategoryRow";
        type Type = super::CategoryRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(row: &glib::subclass::InitializingObject<Self>) {
            row.init_template();
        }
    }

    impl ObjectImpl for CategoryRow {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "category-row.ui");
        }
    }
    impl WidgetImpl for CategoryRow {}
    impl ListBoxRowImpl for CategoryRow {}
}

glib::wrapper! {
    /// A category in the list on the left of Settings.
    pub(crate) struct CategoryRow(ObjectSubclass<imp::CategoryRow>)
        @extends gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl CategoryRow {
    /// The list row of `category`, named for screen readers by its title.
    pub(crate) fn new(category: Category) -> Self {
        let row: Self = glib::Object::new();
        let imp = row.imp();
        icons::set_icon(&imp.glyph, category.icon(), CATEGORY_GLYPH);
        imp.glyph.add_css_class(category.css_class());
        imp.title.set_text(&ox_core::i18n::gettext(category.title()));
        row.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            category.title(),
        ))]);
        imp.category.set(category).expect("a new row has no category yet");
        row
    }

    /// The category the row stands for.
    pub(crate) fn category(&self) -> Category {
        *self
            .imp()
            .category
            .get()
            .expect("CategoryRow::new sets the category")
    }

    /// How many of the category's settings match the search typed now.
    pub(crate) fn matches(&self) -> usize {
        self.imp().matches.get()
    }

    /// Keeps `matches`, the number of the category's settings that match
    /// the search typed now.
    pub(crate) fn set_matches(&self, matches: usize) {
        self.imp().matches.set(matches);
    }
}
