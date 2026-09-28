// SPDX-License-Identifier: AGPL-3.0-only
//! A section of Settings: the right side for one category, or for a
//! sub-page one of its rows opens. A title, the line under it, then status
//! cards, groups of rows and notes.
//!
//! Replaces the stacked `.settings-section` cards of `renderSettingsPage`
//! in `desktop/ui/app.js`: only the chosen category shows (SET-019). The
//! page is as wide as [`WIDEST_PAGE`] where there is room, as the mockup's
//! `.page`, and narrower where there is not, so rows stay easy to scan in
//! a wide window.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::group::SettingsGroup;
use super::parts;
use super::row::{PageWidth, SettingRow};
use super::search::SearchQuery;
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

    /// Adds a group of rows at the end.
    pub(crate) fn append_group(&self, group: &SettingsGroup) {
        self.body().append(group);
    }

    /// Adds a status card or a note at the end. The settings search hides
    /// them, so only matching rows show.
    pub(crate) fn append_extra(&self, extra: &impl IsA<gtk::Widget>) {
        self.body().append(extra);
    }

    /// The page's groups, in order.
    pub(crate) fn groups(&self) -> Vec<SettingsGroup> {
        let groups = children(self.body()).filter_map(|child| child.downcast::<SettingsGroup>().ok());
        groups.collect()
    }

    /// Every row of the page, in order.
    pub(crate) fn rows(&self) -> Vec<SettingRow> {
        self.groups().iter().flat_map(SettingsGroup::rows).collect()
    }

    /// Shows only the rows that match `query`, and the status cards and
    /// notes only while nothing is typed; returns how many rows match.
    pub(crate) fn apply_query(&self, query: &SearchQuery) -> usize {
        let mut matching = 0;
        for child in children(self.body()) {
            match child.downcast::<SettingsGroup>() {
                Ok(group) => matching += group.apply_query(query),
                Err(extra) => extra.set_visible(query.is_empty()),
            }
        }
        matching
    }

    /// Lays every row and status card out for a page `width` wide.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        for child in children(self.body()) {
            match child.downcast::<SettingsGroup>() {
                Ok(group) => group.fit_to_width(width),
                Err(extra) => parts::fit_status_card(&extra, width),
            }
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
