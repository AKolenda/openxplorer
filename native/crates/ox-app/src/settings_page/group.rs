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
use super::search::{shown_text, SearchQuery};
use crate::window::children;

mod imp {
    use super::{Availability, Cell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SettingsGroup`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/settings-group.ui")]
    pub(crate) struct SettingsGroup {
        /// The title, its buttons and the milestone line.
        #[template_child]
        pub(super) heading: TemplateChild<gtk::Box>,
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
        group.show_heading_when_it_has_content();
        group
    }

    /// An empty group headed `title` whose every row waits for what
    /// `availability` names: the heading names the milestone once, and
    /// rows pending the same milestone leave it out.
    pub(crate) fn pending(title: &str, availability: Availability) -> Self {
        let group = Self::new(title);
        let imp = group.imp();
        imp.shared_availability.set(availability);
        let notice = availability.notice();
        imp.notice_label.set_text(notice.as_deref().unwrap_or_default());
        imp.notice_label.set_visible(notice.is_some());
        group.show_heading_when_it_has_content();
        group
    }

    /// Shows the heading while it has a title, a button or a milestone
    /// line; an empty heading would still take its margin.
    fn show_heading_when_it_has_content(&self) {
        let imp = self.imp();
        let has_title = !imp.title_label.text().is_empty();
        let has_actions = imp.actions.first_child().is_some();
        let has_notice = imp.notice_label.is_visible();
        imp.heading.set_visible(has_title || has_actions || has_notice);
    }

    /// Adds `row` at the end. A row pending the milestone the heading
    /// already names does not repeat it, whether its availability was set
    /// before or after.
    pub(crate) fn add_row(&self, row: &SettingRow) {
        row.set_heading_availability(self.imp().shared_availability.get());
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

    /// The milestone line the heading shows, if it shows one.
    #[cfg(test)]
    pub(crate) fn shown_notice(&self) -> Option<String> {
        let label = &self.imp().notice_label;
        label.is_visible().then(|| label.text().to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_page::row::ControlName;
    use crate::settings_page::row::Milestone;
    use crate::settings_page::search::RowText;

    const RESTORE: RowText = RowText {
        title: "Restore previous",
        description: "Restores the recorded file handlers.",
        keywords: "undo",
    };

    const PENDING: Availability = Availability::Unported(Milestone::Distribution);

    /// A row of [`RESTORE`] with one button.
    fn row_with_a_button() -> (SettingRow, gtk::Button) {
        let row = SettingRow::new(RESTORE);
        let button = gtk::Button::with_label(RESTORE.title);
        row.add_control(&button, ControlName::OwnLabel);
        (row, button)
    }

    /// The heading names the milestone once, whether a row's availability
    /// was set before or after the row joined the group.
    ///
    /// parity: SET-019
    #[gtk::test]
    fn a_pending_group_names_its_milestone_once_whatever_the_order() {
        let group = SettingsGroup::pending("Advanced", PENDING);
        let (set_before, _) = row_with_a_button();
        set_before.set_availability(PENDING);
        group.add_row(&set_before);
        let (set_after, _) = row_with_a_button();
        group.add_row(&set_after);
        set_after.set_availability(PENDING);

        assert_eq!(group.shown_notice(), PENDING.notice());
        assert_eq!(set_before.shown_notice(), None);
        assert_eq!(set_after.shown_notice(), None);
    }

    /// parity: SET-019
    #[gtk::test]
    fn a_row_pending_a_milestone_its_heading_does_not_name_names_its_own() {
        let group = SettingsGroup::new("Advanced");
        let (row, _) = row_with_a_button();
        group.add_row(&row);

        row.set_availability(PENDING);

        assert_eq!(row.shown_notice(), PENDING.notice());
        assert_eq!(group.shown_notice(), None);
    }

    /// parity: SET-019
    #[gtk::test]
    fn a_control_added_after_the_row_is_marked_unported_is_disabled_too() {
        let (row, added_before) = row_with_a_button();
        row.set_availability(PENDING);
        let added_after = gtk::Button::with_label("Test");

        row.add_control(&added_after, ControlName::OwnLabel);

        assert!(!added_before.is_sensitive());
        assert!(!added_after.is_sensitive());
    }
}
