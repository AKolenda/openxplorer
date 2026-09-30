// SPDX-License-Identifier: AGPL-3.0-only
//! A titled group of settings in a thin frame, its rows divided by thin
//! lines.
//!
//! Replaces the heavy `.settings-section` cards of `desktop/ui/app.js`
//! with the flat groups of the settings mockup (SET-019). The static
//! layout is the template `resources/ui/settings-group.ui`.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::row::{PageWidth, SettingRow};
use super::search::{shown_text, SearchQuery};
use crate::window::children;

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SettingsGroup`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/settings-group.ui")]
    pub(crate) struct SettingsGroup {
        /// The title and its buttons.
        #[template_child]
        pub(super) heading: TemplateChild<gtk::Box>,
        /// The group's name.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// Buttons at the right of the title, such as "Refresh status".
        #[template_child]
        pub(super) actions: TemplateChild<gtk::Box>,
        /// The rows, in a thin frame.
        #[template_child]
        pub(super) rows: TemplateChild<gtk::Box>,
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
        group.show_heading_when_it_has_content();
        group
    }

    /// Shows the heading while it has a title or a button; an empty
    /// heading would still take its margin.
    fn show_heading_when_it_has_content(&self) {
        let imp = self.imp();
        let has_title = !imp.title_label.text().is_empty();
        let has_actions = imp.actions.first_child().is_some();
        imp.heading.set_visible(has_title || has_actions);
    }

    /// Adds `row` at the end.
    pub(crate) fn add_row(&self, row: &SettingRow) {
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

    /// Puts `action` at the right of the heading.
    pub(crate) fn add_heading_action(&self, action: &impl IsA<gtk::Widget>) {
        self.imp().actions.append(action);
        self.show_heading_when_it_has_content();
    }

    /// The group's setting rows, in order.
    pub(crate) fn rows(&self) -> Vec<SettingRow> {
        let rows = children(&*self.imp().rows).filter_map(|child| child.downcast::<SettingRow>().ok());
        rows.collect()
    }

    /// Shows the rows that match `query`, by their own text or by the
    /// group's heading, and the group while any does; returns how many
    /// match.
    pub(crate) fn apply_query(&self, query: &SearchQuery) -> usize {
        let heading = self.heading_text();
        let rows = self.rows();
        let matching = rows.iter().filter(|row| row.apply_query(query, &heading));
        let matching = matching.count();
        self.set_visible(matching > 0);
        matching
    }

    /// What the heading shows: the title and the labels of its buttons,
    /// such as "What opens where Refresh status".
    fn heading_text(&self) -> String {
        let imp = self.imp();
        let actions = shown_text(&*imp.actions);
        format!("{} {actions}", imp.title_label.text())
    }

    /// Lays every row out for a page `width` wide.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        for row in self.rows() {
            row.fit_to_width(width);
        }
    }
}
