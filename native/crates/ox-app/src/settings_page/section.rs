// SPDX-License-Identifier: AGPL-3.0-only
//! A section of Settings: the right side for one category, or for a
//! sub-page one of its rows opens. A title, the line under it, then status
//! cards, groups of rows and notes, each a [`Part`] of the section.
//!
//! Replaces the stacked `.settings-section` cards of `renderSettingsPage`
//! in `v2.0.0:desktop/ui/app.js`: only the chosen category shows (SET-019). The
//! page is as wide as [`WIDEST_PAGE`] where there is room, as the mockup's
//! `.page`, and narrower where there is not, so rows stay easy to scan in
//! a wide window.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::group::SettingsGroup;
use super::row::{PageWidth, SettingRow};
use super::search::SearchQuery;
use super::status_card::StatusCard;
use crate::icons::{self, Icon};
use crate::window::children;

/// The widest a page grows, in pixels (`.page{width:820px}` in the
/// settings mockup).
const WIDEST_PAGE: i32 = 820;

/// The back arrow of a sub-page.
const BACK_GLYPH: i32 = 18;

/// Whether a section is a category's page or a sub-page a row opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageKind {
    /// A category chosen in the list.
    Category,
    /// A page with a back arrow to its category.
    Subpage,
}

mod imp {
    use std::cell::OnceCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SettingsSection`].
    #[derive(Debug, Default)]
    pub(crate) struct SettingsSection {
        /// The title, the lead and the body, top to bottom: the page's one
        /// child.
        pub(super) column: OnceCell<gtk::Box>,
        /// The status cards, groups and notes.
        pub(super) body: OnceCell<gtk::Box>,
        /// The arrow back to the category, on a sub-page.
        pub(super) back_button: OnceCell<gtk::Button>,
    }

    impl SettingsSection {
        /// The page's one child.
        pub(super) fn column(&self) -> &gtk::Box {
            self.column.get().expect("constructed builds the column")
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsSection {
        const NAME: &'static str = "OxSettingsSection";
        type Type = super::SettingsSection;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SettingsSection {
        fn constructed(&self) {
            self.parent_constructed();
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.set_parent(&*self.obj());
            self.column.set(column).expect("constructed runs once per object");
            self.obj().set_halign(gtk::Align::Start);
        }

        fn dispose(&self) {
            if let Some(column) = self.column.get() {
                column.unparent();
            }
        }
    }

    // The page lays its column out itself rather than through a layout
    // manager: GTK asks a layout manager for sizes instead of `measure`,
    // which widens the page here.
    impl WidgetImpl for SettingsSection {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            self.column().request_mode()
        }

        /// Asks for the widest page as its natural width: the page starts
        /// at the left and takes that much where the window has room, and
        /// less where it has not, but never stretches its rows further.
        /// The width never depends on a height: asked for one, the wrapped
        /// labels would answer with their text on a single line.
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                let (minimum, ..) = self.column().measure(orientation, -1);
                return (minimum, super::WIDEST_PAGE.max(minimum), -1, -1);
            }
            self.column().measure(orientation, for_size)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.column().allocate(width, height, baseline, None);
        }
    }
}

glib::wrapper! {
    /// One section of Settings: a category's page or a sub-page, of the
    /// kind [`PageKind`] says.
    pub(crate) struct SettingsSection(ObjectSubclass<imp::SettingsSection>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SettingsSection {
    /// An empty page titled `title`, with `lead` under the title.
    pub(crate) fn new(title: &str, lead: &str, kind: PageKind) -> Self {
        let page: Self = glib::Object::new();
        page.build_header(title, lead, kind);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("page-body");
        page.imp().column().append(&body);
        page.imp().body.set(body).expect("a new page has no body yet");
        page
    }

    /// The title, with the back arrow before it on a sub-page, and the lead.
    fn build_header(&self, title: &str, lead: &str, kind: PageKind) {
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title_row.add_css_class("page-title-row");
        if kind == PageKind::Subpage {
            let back = back_button();
            title_row.append(&back);
            self.imp()
                .back_button
                .set(back)
                .expect("a new page has no back arrow yet");
        }
        let title_label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .wrap(true)
            .accessible_role(gtk::AccessibleRole::Heading)
            .css_classes(["page-title"])
            .build();
        title_row.append(&title_label);
        let lead_label = gtk::Label::builder()
            .label(lead)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["page-lead"])
            .build();
        let column = self.imp().column();
        column.append(&title_row);
        column.append(&lead_label);
    }

    fn body(&self) -> &gtk::Box {
        self.imp()
            .body
            .get()
            .expect("SettingsSection::new builds the body")
    }

    /// The arrow back to the category, on a sub-page.
    pub(crate) fn back_button(&self) -> Option<&gtk::Button> {
        self.imp().back_button.get()
    }

    /// Adds a status card at the end.
    pub(crate) fn append_card(&self, card: &StatusCard) {
        self.body().append(card);
    }

    /// Adds a group of rows at the end.
    pub(crate) fn append_group(&self, group: &SettingsGroup) {
        self.body().append(group);
    }

    /// Adds a note or a paragraph at the end. The settings search hides
    /// it, so only matching settings show.
    pub(crate) fn append_text(&self, text: &impl IsA<gtk::Widget>) {
        self.body().append(text);
    }

    /// The section's parts, top to bottom.
    fn parts(&self) -> Vec<Part> {
        children(self.body()).map(Part::of).collect()
    }

    /// The section's groups, in order.
    #[cfg(test)]
    pub(crate) fn groups(&self) -> Vec<SettingsGroup> {
        let parts = self.parts().into_iter();
        let groups = parts.filter_map(|part| match part {
            Part::Group(group) => Some(group),
            Part::Card(_) | Part::Text(_) => None,
        });
        groups.collect()
    }

    /// Every row of the section, in order.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<SettingRow> {
        self.groups().iter().flat_map(SettingsGroup::rows).collect()
    }

    /// Shows only the rows and status cards that match `query`, and the
    /// notes only while nothing is typed; returns how many settings match.
    pub(crate) fn apply_query(&self, query: &SearchQuery) -> usize {
        let mut matching = 0;
        for part in self.parts() {
            matching += match part {
                Part::Group(group) => group.apply_query(query),
                Part::Card(card) => usize::from(card.apply_query(query)),
                Part::Text(text) => {
                    text.set_visible(query.is_empty());
                    0
                }
            };
        }
        matching
    }

    /// The first setting the search shows, top to bottom: a status card or
    /// a row.
    pub(crate) fn first_match(&self) -> Option<SearchTarget> {
        self.parts().into_iter().find_map(|part| match part {
            Part::Card(card) => card.is_visible().then_some(SearchTarget::Card(card)),
            Part::Group(group) => {
                let row = group.rows().into_iter().find(WidgetExt::is_visible)?;
                Some(SearchTarget::Row(row))
            }
            Part::Text(_) => None,
        })
    }

    /// Lays every row and status card out for a page `width` wide.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        for part in self.parts() {
            match part {
                Part::Group(group) => group.fit_to_width(width),
                Part::Card(card) => card.fit_to_width(width),
                Part::Text(_) => {}
            }
        }
    }
}

/// One part of a section's body, as [`SettingsSection::append_card`],
/// [`SettingsSection::append_group`] and [`SettingsSection::append_text`]
/// added it.
#[derive(Debug)]
enum Part {
    /// A status card, which the settings search finds.
    Card(StatusCard),
    /// A group of rows, which the settings search filters.
    Group(SettingsGroup),
    /// A note or a paragraph, hidden while searching.
    Text(gtk::Widget),
}

impl Part {
    /// The part `child` of the body is.
    fn of(child: gtk::Widget) -> Self {
        let child = match child.downcast::<StatusCard>() {
            Ok(card) => return Part::Card(card),
            Err(child) => child,
        };
        match child.downcast::<SettingsGroup>() {
            Ok(group) => Part::Group(group),
            Err(text) => Part::Text(text),
        }
    }
}

/// A setting the settings search found: a status card or a row.
#[derive(Debug, Clone)]
pub(crate) enum SearchTarget {
    /// A status card, found by its text and its buttons.
    Card(StatusCard),
    /// A row of settings.
    Row(SettingRow),
}

impl SearchTarget {
    /// Outlines the setting and gives its first working control keyboard
    /// focus; false when it has none.
    pub(crate) fn jump_here(&self) -> bool {
        match self {
            SearchTarget::Card(card) => card.jump_here(),
            SearchTarget::Row(row) => row.jump_here(),
        }
    }

    /// The setting's widget, for scrolling to it.
    pub(crate) fn widget(&self) -> &gtk::Widget {
        match self {
            SearchTarget::Card(card) => card.upcast_ref(),
            SearchTarget::Row(row) => row.upcast_ref(),
        }
    }
}

/// The flat arrow before a sub-page's title.
fn back_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::image(Icon::ArrowLeft, BACK_GLYPH))
        .tooltip_text("Back")
        .valign(gtk::Align::Center)
        .css_classes(["page-back"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Back")]);
    button
}
