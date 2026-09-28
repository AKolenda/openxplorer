// SPDX-License-Identifier: AGPL-3.0-only
//! The search box at the right of the navigation row.
//!
//! Ports `.search-wrap` in `desktop/ui/index.html` and `style.css`: a
//! bordered field reading `Search <folder>` with a thin magnifier at its
//! right end, as in Windows 11. A `GtkSearchEntry` does the typing, so its
//! delayed `search-changed`, Escape handling and clear button stay; only
//! its own magnifier, which GTK always draws first, is hidden in favour of
//! the trailing one, and its clear button shows the bundled close glyph
//! instead of the desktop theme's `edit-clear-symbolic`.
//!
//! [`SearchBox`] is a `GtkBox` subclass laid out by the template
//! `resources/ui/search-box.ui`. The window hears the typed text through
//! [`SearchBox::connect_query_changed`], never through the entry itself.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};

/// The trailing magnifier's glyph.
const MAGNIFIER_GLYPH: i32 = 15;
/// The clear button's glyph: 16 pixels, GTK's own size for an entry's
/// icons (the initial `-gtk-icon-size`), so the field keeps its layout.
const CLEAR_GLYPH: i32 = 16;

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SearchBox`]: the template's widgets.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/search-box.ui")]
    pub(crate) struct SearchBox {
        /// The field the user types in.
        #[template_child]
        pub(super) entry: TemplateChild<gtk::SearchEntry>,
        /// The magnifier at the right end.
        #[template_child]
        pub(super) magnifier: TemplateChild<gtk::Image>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchBox {
        const NAME: &'static str = "OxSearchBox";
        type Type = super::SearchBox;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(search_box: &glib::subclass::InitializingObject<Self>) {
            search_box.init_template();
        }
    }

    impl ObjectImpl for SearchBox {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().finish_template();
        }
    }

    impl WidgetImpl for SearchBox {}
    impl BoxImpl for SearchBox {}
}

glib::wrapper! {
    /// The bordered search box in the navigation row.
    pub(crate) struct SearchBox(ObjectSubclass<imp::SearchBox>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl SearchBox {
    /// Swaps the entry's leading magnifier for the trailing one, and its
    /// clear icon for the bundled one.
    fn finish_template(&self) {
        let imp = self.imp();
        hide_leading_magnifier(&imp.entry);
        show_bundled_clear_icon(&imp.entry);
        icons::set_icon(&imp.magnifier, Icon::Search, MAGNIFIER_GLYPH);
    }

    /// Calls `on_query_changed` with the text once typing pauses, and at
    /// once when the text is cleared.
    pub(super) fn connect_query_changed(&self, on_query_changed: impl Fn(&str) + 'static) {
        self.imp()
            .entry
            .connect_search_changed(move |entry| on_query_changed(entry.text().as_str()));
    }

    /// Moves keyboard focus into the box (Ctrl+F).
    pub(super) fn focus(&self) {
        self.imp().entry.grab_focus();
    }

    /// Names the folder the box searches: "Search Documents".
    pub(super) fn set_folder_title(&self, title: &str) {
        let placeholder = format!("Search {title}");
        self.imp().entry.set_placeholder_text(Some(&placeholder));
    }

    /// Enables the box in folders and disables it on landing pages.
    pub(super) fn set_enabled(&self, enabled: bool) {
        self.set_sensitive(enabled);
    }

    /// Empties the box, which ends the filter.
    pub(super) fn clear(&self) {
        self.imp().entry.set_text("");
    }

    /// The field the user types in, for tests.
    #[cfg(test)]
    pub(super) fn entry(&self) -> gtk::SearchEntry {
        self.imp().entry.get()
    }
}

/// Hides the magnifier `GtkSearchEntry` puts before the text; it is the
/// entry's first child image in GTK 4.
fn hide_leading_magnifier(entry: &gtk::SearchEntry) {
    let leading = entry.first_child().and_downcast::<gtk::Image>();
    if let Some(image) = leading {
        image.set_visible(false);
    }
}

/// Shows the bundled close glyph in the button that empties the entry: its
/// last child image in GTK 4, which GTK names once and then only shows
/// while there is text.
fn show_bundled_clear_icon(entry: &gtk::SearchEntry) {
    let clear = entry.last_child().and_downcast::<gtk::Image>();
    if let Some(image) = clear {
        icons::set_icon(&image, Icon::Dismiss16, CLEAR_GLYPH);
    }
}
