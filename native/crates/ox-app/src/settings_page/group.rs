// SPDX-License-Identifier: AGPL-3.0-only
//! A titled group of settings in a thin frame, its rows divided by thin
//! lines.
//!
//! Replaces the heavy `.settings-section` cards of `desktop/ui/app.js`
//! with the flat groups of the settings mockup (SET-019). The static
//! layout is the template `resources/ui/settings-group.ui`. When the
//! native preview lacks a whole group, its heading names the milestone
//! once instead of every row repeating it.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::row::{Availability, PageWidth, SettingRow};
use super::search::SearchQuery;
use crate::window::children;

mod imp {
    use super::{Availability, Cell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SettingsGroup`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/settings-group.ui")]
    pub(crate) struct SettingsGroup {
        /// The group's name.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// Buttons at the right of the title, such as "Refresh status".
        #[template_child]
        pub(super) actions: TemplateChild<gtk::Box>,
        /// The milestone that brings every row of the group.
        #[template_child]
        pub(super) notice_label: TemplateChild<gtk::Label>,
        /// The rows, in a thin frame.
        #[template_child]
        pub(super) rows: TemplateChild<gtk::Box>,
        /// What the heading says the preview can do with every row.
        pub(super) shared_availability: Cell<Availability>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsGroup {
        const NAME: &'static str = "OxSettingsGroup";
        type Type = super::SettingsGroup;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(group: &glib::subclass::InitializingObject<Self>) {
            group.init_template();
        }
    }

    impl ObjectImpl for SettingsGroup {}
    impl WidgetImpl for SettingsGroup {}
    impl BoxImpl for SettingsGroup {}
}

glib::wrapper! {
    /// Setting rows under one heading.
    pub(crate) struct SettingsGroup(ObjectSubclass<imp::SettingsGroup>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl SettingsGroup {
    /// An empty group headed `title`; an empty title leaves the heading
    /// out, for a group whose one row names itself.
    pub(crate) fn new(title: &str) -> Self {
        let group: Self = glib::Object::new();
        let title_label = &group.imp().title_label;
        title_label.set_text(title);
        title_label.set_visible(!title.is_empty());
        group
    }

    /// Adds `row` at the end. A row pending the milestone the heading
    /// already names does not repeat it.
    pub(crate) fn add_row(&self, row: &SettingRow) {
        let shared = self.imp().shared_availability.get();
        if shared != Availability::Ready && row.availability() == shared {
            row.hide_notice();
        }
        self.imp().rows.append(row);
    }

    /// Adds a row that is not a setting, such as a folder the search
    /// index can take; the settings search passes it by.
    pub(crate) fn add_plain_row(&self, row: &impl IsA<gtk::Widget>) {
        self.imp().rows.append(row);
    }

    /// Removes every row.
    pub(crate) fn remove_rows(&self) {
        let rows = &self.imp().rows;
        while let Some(row) = rows.first_child() {
            rows.remove(&row);
        }
    }

    /// Names in the heading what the preview can do with every row, which
    /// the rows added afterwards then leave out.
    pub(crate) fn set_shared_availability(&self, availability: Availability) {
        let imp = self.imp();
        imp.shared_availability.set(availability);
        let notice = availability.notice();
        imp.notice_label.set_text(notice.as_deref().unwrap_or_default());
        imp.notice_label.set_visible(notice.is_some());
    }

    /// The milestone line the heading shows, if it shows one.
    #[cfg(test)]
    pub(crate) fn shown_notice(&self) -> Option<String> {
        let label = &self.imp().notice_label;
        label.is_visible().then(|| label.text().to_string())
    }

    /// Puts `action` at the right of the heading.
    pub(crate) fn add_heading_action(&self, action: &impl IsA<gtk::Widget>) {
        self.imp().actions.append(action);
    }

    /// The group's setting rows, in order.
    pub(crate) fn rows(&self) -> Vec<SettingRow> {
        let rows = children(&*self.imp().rows).filter_map(|child| child.downcast::<SettingRow>().ok());
        rows.collect()
    }

    /// Shows the rows that match `query`, and the group while any does;
    /// returns how many match.
    pub(crate) fn apply_query(&self, query: &SearchQuery) -> usize {
        let matching = self.rows().iter().filter(|row| row.apply_query(query)).count();
        self.set_visible(matching > 0);
        matching
    }

    /// Lays every row out for a page `width` wide.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        for row in self.rows() {
            row.fit_to_width(width);
        }
    }
}
