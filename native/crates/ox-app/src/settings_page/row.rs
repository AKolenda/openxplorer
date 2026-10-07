// SPDX-License-Identifier: AGPL-3.0-only
//! One setting: a one-line title on the left, with an ⓘ whose bubble has
//! the details, and a compact control on the right.
//!
//! Replaces the `.settings-line` rows of `v2.0.0:desktop/ui/app.js`
//! (`renderSettingsPage`), whose controls sat under long paragraphs. As in
//! the settings mockup, each row is one short line; what it used to say
//! under its name is in the ⓘ's bubble ([`InfoBubble`]). A row whose line
//! states something read later, such as the app that opens ZIP files,
//! still shows that status under its name. The static layout is the
//! template `resources/ui/settings-row.ui`.

use std::cell::{Cell, OnceCell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::i18n::gettext;

use super::info_bubble::InfoBubble;
use super::search::{jump_to, shown_text, RowText, SearchQuery};
use crate::window::children;

/// What a screen reader calls a control put on a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlName {
    /// The row's title, for a switch or a drop-down that has no text.
    RowTitle,
    /// Its own label, such as a button's "Reset"; the row describes it.
    OwnLabel,
}

/// How much room the settings page has for its rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PageWidth {
    /// Room for the controls beside the text.
    #[default]
    Roomy,
    /// A narrow window: every row puts its controls under the text.
    Narrow,
}

/// Where a row puts its controls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RowLayout {
    /// To the right of the text.
    #[default]
    ControlsBeside,
    /// Under the text: for wide controls, and every row of a narrow window.
    ControlsBelow,
}

mod imp {
    use super::{Cell, InfoBubble, OnceCell, RefCell, RowLayout, RowText};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SettingRow`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/settings-row.ui")]
    pub(crate) struct SettingRow {
        /// The name and the ⓘ after it.
        #[template_child]
        pub(super) title_line: TemplateChild<gtk::Grid>,
        /// The row's name.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// A live status under the name, for the rows that have one.
        #[template_child]
        pub(super) description_label: TemplateChild<gtk::Label>,
        /// The ⓘ with the row's details, when it has any.
        pub(super) info: OnceCell<InfoBubble>,
        /// Details added after the row's own, which the search finds too.
        pub(super) more_words: RefCell<String>,
        /// The switch, drop-down or buttons.
        #[template_child]
        pub(super) control_slot: TemplateChild<gtk::Box>,
        /// What the row says, set once by `SettingRow::new`.
        pub(super) text: OnceCell<RowText>,
        /// Where the controls go while the window has room.
        pub(super) roomy_layout: Cell<RowLayout>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingRow {
        const NAME: &'static str = "OxSettingRow";
        type Type = super::SettingRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(row: &glib::subclass::InitializingObject<Self>) {
            row.init_template();
        }
    }

    impl ObjectImpl for SettingRow {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "settings-row.ui");
        }
    }
    impl WidgetImpl for SettingRow {}
    impl BoxImpl for SettingRow {}
}

glib::wrapper! {
    /// One setting of a [`SettingsGroup`](super::group::SettingsGroup).
    pub(crate) struct SettingRow(ObjectSubclass<imp::SettingRow>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl SettingRow {
    /// A row saying `text`, without controls yet: its title on one line,
    /// and its description in the ⓘ's bubble.
    pub(crate) fn new(text: RowText) -> Self {
        let row: Self = glib::Object::new();
        let imp = row.imp();
        imp.title_label.set_text(&gettext(text.title));
        imp.description_label.set_visible(false);
        imp.text.set(text).expect("a new row has no text yet");
        if !text.description.is_empty() {
            row.add_details(&gettext(text.description));
        }
        row
    }

    /// Adds `details` to the ⓘ's bubble, opening one if the row has none
    /// yet, such as a note that used to follow the row's group. The
    /// settings search finds the row by them too.
    pub(crate) fn add_details(&self, details: &str) {
        let imp = self.imp();
        if let Some(info) = imp.info.get() {
            info.add_paragraph(details);
            let mut more = imp.more_words.borrow_mut();
            more.push(' ');
            more.push_str(details);
            return;
        }
        let title = imp.title_label.text();
        let info = InfoBubble::new(&title, details);
        imp.title_line.attach(info.widget(), 1, 0, 1, 1);
        let _ = imp.info.set(info);
    }

    /// The ⓘ with the row's details, when it has any.
    #[cfg(test)]
    pub(crate) fn info(&self) -> Option<&InfoBubble> {
        self.imp().info.get()
    }

    /// What the row says.
    pub(crate) fn text(&self) -> RowText {
        *self.imp().text.get().expect("SettingRow::new sets the text")
    }

    /// Shows `description` under the title, for a line that states
    /// something read later, such as the app that opens ZIP files. The
    /// row's own details stay in its ⓘ, and the search still finds the
    /// row by its own words.
    pub(crate) fn set_description(&self, description: &str) {
        let label = &self.imp().description_label;
        label.set_text(description);
        label.set_visible(!description.is_empty());
    }

    /// The line the row shows under its title.
    #[cfg(test)]
    pub(crate) fn shown_description(&self) -> String {
        self.imp().description_label.text().into()
    }

    /// Puts `control` after the row's other controls, named for screen
    /// readers as `name` says.
    pub(crate) fn add_control(&self, control: &impl IsA<gtk::Widget>, name: ControlName) {
        let imp = self.imp();
        let title: &gtk::Accessible = imp.title_label.upcast_ref();
        let description: &gtk::Accessible = imp.description_label.upcast_ref();
        let relation = match name {
            ControlName::RowTitle => gtk::accessible::Relation::LabelledBy(&[title]),
            ControlName::OwnLabel => gtk::accessible::Relation::DescribedBy(&[title, description]),
        };
        let control = control.upcast_ref::<gtk::Widget>();
        control.update_relation(&[relation]);
        // The details the ⓘ shows are read with the control, so a screen
        // reader user needs no bubble.
        if let (ControlName::RowTitle, Some(info)) = (name, imp.info.get()) {
            control.update_property(&[gtk::accessible::Property::Description(&info.details())]);
        }
        imp.control_slot.append(control);
    }

    /// The row's controls, in order.
    pub(crate) fn controls(&self) -> Vec<gtk::Widget> {
        children(&*self.imp().control_slot).collect()
    }

    /// Shows the row when it matches `query` by its own words, the labels
    /// of its controls or `heading`, what its group's heading shows; marks
    /// it as a match while a search is typed, and says whether it shows.
    pub(crate) fn apply_query(&self, query: &SearchQuery, heading: &str) -> bool {
        let words = format!("{} {}", self.text().words(), self.imp().more_words.borrow());
        let controls = shown_text(&*self.imp().control_slot);
        let finding = query.find_in(&[&words, &controls, heading]);
        finding.show_on(self);
        finding.is_shown()
    }

    /// Outlines the row as the one the search jumped to, and gives its
    /// first working control keyboard focus. False when it has none, as a
    /// disabled row.
    pub(crate) fn jump_here(&self) -> bool {
        jump_to(self, &self.controls())
    }

    /// Where the controls go while the window has room; wide controls such
    /// as the theme cards go below.
    pub(crate) fn set_roomy_layout(&self, layout: RowLayout) {
        self.imp().roomy_layout.set(layout);
        self.apply_layout(layout);
    }

    /// Puts the controls below the text in a narrow window, and back where
    /// they belong in a roomy one. A switch is small enough to stay beside.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        let only_switches = self.controls().iter().all(ObjectExt::is::<gtk::Switch>);
        let layout = match width {
            PageWidth::Narrow if !only_switches => RowLayout::ControlsBelow,
            PageWidth::Roomy | PageWidth::Narrow => self.imp().roomy_layout.get(),
        };
        self.apply_layout(layout);
    }

    fn apply_layout(&self, layout: RowLayout) {
        let slot = &self.imp().control_slot;
        match layout {
            RowLayout::ControlsBeside => {
                self.set_orientation(gtk::Orientation::Horizontal);
                slot.set_halign(gtk::Align::End);
            }
            RowLayout::ControlsBelow => {
                // Filling the row lets a field take its width; buttons keep
                // their own and start at the left.
                self.set_orientation(gtk::Orientation::Vertical);
                slot.set_halign(gtk::Align::Fill);
            }
        }
    }
}
