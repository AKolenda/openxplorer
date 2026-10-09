// SPDX-License-Identifier: AGPL-3.0-only
//! A titled group of settings in a thin frame, its rows divided by thin
//! lines. A folded group, for settings rarely changed, shows only its
//! title with an arrow until it is opened; a search that finds one of its
//! rows opens it while the search lasts.
//!
//! Replaces the heavy `.settings-section` cards of `v2.0.0:desktop/ui/app.js`
//! with the flat groups of the settings mockup (SET-019). The static
//! layout is the template `resources/ui/settings-group.ui`.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::row::{PageWidth, SettingRow};
use super::search::{shown_text, SearchQuery};
use crate::icons::{self, Icon};
use crate::window::children;

/// The arrow of a folded group's title.
const FOLD_GLYPH: i32 = 12;

mod imp {
    use std::cell::{Cell, OnceCell};

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
        /// The title of a folded group, which opens and closes it.
        pub(super) fold: OnceCell<(gtk::Button, gtk::Image)>,
        /// Whether a folded group is open, as the user left it.
        pub(super) is_open: Cell<bool>,
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

    impl ObjectImpl for SettingsGroup {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "settings-group.ui");
        }
    }
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

    /// A group headed `title` that starts folded: only the title shows,
    /// with an arrow, and clicking it, or Enter or Space on it, opens or
    /// closes the group.
    pub(crate) fn new_folded(title: &str) -> Self {
        let group = Self::new("");
        let imp = group.imp();
        let arrow = icons::image(Icon::ChevronRight16, FOLD_GLYPH);
        // One line: a label that wraps beside the arrow would measure
        // taller for a wide row than for no width at all.
        let label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["fold-title"])
            .build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&label);
        content.append(&arrow);
        let toggle = gtk::Button::builder()
            .child(&content)
            .css_classes(["group-fold"])
            .build();
        toggle.update_property(&[gtk::accessible::Property::Label(title)]);
        toggle.connect_clicked(glib::clone!(
            #[weak]
            group,
            move |_| group.set_open(!group.imp().is_open.get())
        ));
        group.add_css_class("folded-group");
        group.prepend(&toggle);
        imp.fold
            .set((toggle, arrow))
            .expect("a new group has no fold yet");
        group.show_rows(false);
        group
    }

    /// Whether the group folds.
    #[cfg(test)]
    pub(crate) fn is_folded(&self) -> bool {
        self.imp().fold.get().is_some()
    }

    /// Opens or closes a folded group, as clicking its title does.
    pub(crate) fn set_open(&self, open: bool) {
        self.imp().is_open.set(open);
        self.show_rows(open);
    }

    /// Whether a folded group's rows show now.
    #[cfg(test)]
    pub(crate) fn shows_rows(&self) -> bool {
        self.imp().rows.is_visible()
    }

    /// Shows or hides a folded group's rows, turning its arrow and telling
    /// screen readers.
    fn show_rows(&self, shown: bool) {
        let imp = self.imp();
        let Some((toggle, arrow)) = imp.fold.get() else {
            return;
        };
        imp.rows.set_visible(shown);
        // Open, the title and its rows are one frame.
        if shown {
            self.add_css_class("open");
        } else {
            self.remove_css_class("open");
        }
        let glyph = if shown {
            Icon::ChevronDown16
        } else {
            Icon::ChevronRight16
        };
        icons::set_icon(arrow, glyph, FOLD_GLYPH);
        toggle.update_state(&[gtk::accessible::State::Expanded(Some(shown))]);
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
        // A search opens a folded group that has matches; ending it puts
        // the group back as the user left it.
        let opened_by_search = !query.is_empty() && matching > 0;
        self.show_rows(opened_by_search || self.imp().is_open.get());
        matching
    }

    /// What the heading shows: the title and the labels of its buttons,
    /// such as "What opens where Refresh status", or a folded group's title.
    fn heading_text(&self) -> String {
        let imp = self.imp();
        let actions = shown_text(&*imp.actions);
        let fold = imp
            .fold
            .get()
            .map(|(toggle, _)| shown_text(toggle))
            .unwrap_or_default();
        format!("{} {actions} {fold}", imp.title_label.text())
    }

    /// Lays every row out for a page `width` wide.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        for row in self.rows() {
            row.fit_to_width(width);
        }
    }
}
