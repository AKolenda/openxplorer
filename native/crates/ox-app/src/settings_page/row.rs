// SPDX-License-Identifier: AGPL-3.0-only
//! One setting: a title and a one-line description on the left, a compact
//! control on the right.
//!
//! Replaces the `.settings-line` rows of `desktop/ui/app.js`
//! (`renderSettingsPage`), whose controls sat under long paragraphs. The
//! static layout is the template `resources/ui/settings-row.ui`. A row the
//! native preview cannot run yet is still shown, with its current wording,
//! and says which `native/ROADMAP.md` milestone brings it ([`Availability`]).

use std::cell::{Cell, OnceCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::search::{RowText, SearchQuery};
use crate::window::{children, Milestone};

/// The class of a row that matches the settings search.
const SEARCH_MATCH_CLASS: &str = "search-match";
/// The class of the row the search jumped to.
const JUMP_TARGET_CLASS: &str = "jump-target";

/// Whether the native preview can do what a row controls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Availability {
    /// It works.
    #[default]
    Ready,
    /// Shown but disabled until the milestone brings it.
    Unported(Milestone),
    /// Saved in the settings file now, which the Python app follows; the
    /// native preview follows it from the milestone on.
    SavedForLater(Milestone),
}

impl Availability {
    /// The line under the description, or `None` for a row that works.
    pub(crate) fn notice(self) -> Option<String> {
        match self {
            Availability::Ready => None,
            Availability::Unported(milestone) => Some(milestone.notice()),
            Availability::SavedForLater(milestone) => Some(format!(
                "Saved for OpenXplorer; the native preview follows it with {}.",
                milestone.description()
            )),
        }
    }
}

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
    use super::{Availability, Cell, OnceCell, RowLayout, RowText};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SettingRow`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/settings-row.ui")]
    pub(crate) struct SettingRow {
        /// The row's name.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// The line under the name.
        #[template_child]
        pub(super) description_label: TemplateChild<gtk::Label>,
        /// Which milestone brings a row the preview cannot run yet.
        #[template_child]
        pub(super) notice_label: TemplateChild<gtk::Label>,
        /// The switch, drop-down or buttons.
        #[template_child]
        pub(super) control_slot: TemplateChild<gtk::Box>,
        /// What the row says, set once by `SettingRow::new`.
        pub(super) text: OnceCell<RowText>,
        /// Whether the preview can do what the row controls.
        pub(super) availability: Cell<Availability>,
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

    impl ObjectImpl for SettingRow {}
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
    /// A row saying `text`, without controls yet.
    pub(crate) fn new(text: RowText) -> Self {
        let row: Self = glib::Object::new();
        let imp = row.imp();
        imp.title_label.set_text(text.title);
        imp.description_label.set_text(text.description);
        imp.description_label.set_visible(!text.description.is_empty());
        imp.text.set(text).expect("a new row has no text yet");
        row
    }

    /// What the row says.
    pub(crate) fn text(&self) -> RowText {
        *self.imp().text.get().expect("SettingRow::new sets the text")
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
        control.upcast_ref::<gtk::Widget>().update_relation(&[relation]);
        imp.control_slot.append(control);
    }

    /// The row's controls, in order.
    pub(crate) fn controls(&self) -> Vec<gtk::Widget> {
        children(&*self.imp().control_slot).collect()
    }

    /// Whether the native preview can do what the row controls.
    pub(crate) fn availability(&self) -> Availability {
        self.imp().availability.get()
    }

    /// Marks what the preview can do with the row: an unported row's
    /// controls are disabled, and both kinds of pending row name their
    /// milestone in a line under the description and in a tooltip.
    pub(crate) fn set_availability(&self, availability: Availability) {
        let imp = self.imp();
        imp.availability.set(availability);
        let notice = availability.notice();
        imp.notice_label.set_text(notice.as_deref().unwrap_or_default());
        imp.notice_label.set_visible(notice.is_some());
        self.set_tooltip_text(notice.as_deref());
        let is_unported = matches!(availability, Availability::Unported(_));
        for control in self.controls() {
            control.set_sensitive(!is_unported);
        }
    }

    /// Leaves the milestone out of the row, for a group whose heading
    /// states it for every row.
    pub(crate) fn hide_notice(&self) {
        self.imp().notice_label.set_visible(false);
    }

    /// The milestone line the row shows, if it shows one.
    #[cfg(test)]
    pub(crate) fn shown_notice(&self) -> Option<String> {
        let label = &self.imp().notice_label;
        label.is_visible().then(|| label.text().to_string())
    }

    /// Shows the row when it matches `query`, marked as a match while a
    /// search is typed, and says whether it shows.
    pub(crate) fn apply_query(&self, query: &SearchQuery) -> bool {
        let matches = self.text().matches(query);
        self.set_visible(matches);
        self.remove_css_class(JUMP_TARGET_CLASS);
        if matches && !query.is_empty() {
            self.add_css_class(SEARCH_MATCH_CLASS);
        } else {
            self.remove_css_class(SEARCH_MATCH_CLASS);
        }
        matches
    }

    /// Outlines the row as the one the search jumped to, and gives its
    /// first working control keyboard focus. False when it has none, as a
    /// disabled row.
    pub(crate) fn jump_here(&self) -> bool {
        self.add_css_class(JUMP_TARGET_CLASS);
        for control in self.controls() {
            if control.is_sensitive() && control.grab_focus() {
                return true;
            }
        }
        false
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
