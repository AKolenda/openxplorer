// SPDX-License-Identifier: AGPL-3.0-only
//! The banner of a tab that shows a previous version (PROP-022).
//!
//! Ports `renderSnapshotBanner` in `v2.0.0:desktop/ui/app.js` and
//! `#snapshot-banner` in `v2.0.0:desktop/ui/index.html`: under the command bar, a
//! clock, "Previous version", the date and time read from the snapshot's
//! name (or the name itself) with a tooltip saying where they come from,
//! and `Read-only in OpenXplorer · Restore a copy to edit`. Live folders
//! show no banner. The colours are the caution tokens of
//! `native/docs/ui-spec.md` C35.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::versions::{SnapshotLocation, NO_DATE_EXPLANATION};

use super::version_row::medium_date;
use crate::icons::{self, Icon};

/// The banner's heading.
const HEADING: &str = "Previous version";
/// What the user may do with a previous version.
const SAFETY: &str = "Read-only in OpenXplorer · Restore a copy to edit";
/// The clock's size (`icon('clock', 16)`).
const GLYPH_SIZE: i32 = 16;

/// The date text of `snapshot`: `5 Sep 2026 · 14:30`, or the snapshot's
/// name when it has no date, with the tooltip that explains it.
pub(crate) fn date_line(snapshot: &SnapshotLocation) -> (String, &'static str) {
    let Some(date) = snapshot.date() else {
        return (snapshot.label().to_owned(), NO_DATE_EXPLANATION);
    };
    let time = date.time_text();
    let day = medium_date(&date);
    let text = if time.is_empty() {
        day
    } else {
        format!("{day} · {time}")
    };
    (text, date.explanation())
}

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SnapshotBanner`].
    #[derive(Debug, Default)]
    pub(crate) struct SnapshotBanner {
        /// The date and time, or the snapshot's name.
        pub(super) date: gtk::Label,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SnapshotBanner {
        const NAME: &'static str = "OxSnapshotBanner";
        type Type = super::SnapshotBanner;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for SnapshotBanner {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for SnapshotBanner {}
    impl BoxImpl for SnapshotBanner {}
}

glib::wrapper! {
    /// The banner of a tab inside a snapshot.
    pub(crate) struct SnapshotBanner(ObjectSubclass<imp::SnapshotBanner>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for SnapshotBanner {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SnapshotBanner {
    fn build(&self) {
        self.add_css_class("snapshot-banner");
        self.set_visible(false);
        self.set_accessible_role(gtk::AccessibleRole::Status);
        self.append(&icons::image(Icon::Clock, GLYPH_SIZE));
        let heading = gtk::Label::builder()
            .label(HEADING)
            .css_classes(["snapshot-heading"])
            .build();
        self.append(&heading);
        self.append(&self.imp().date);
        let safety = gtk::Label::builder()
            .label(SAFETY)
            .hexpand(true)
            .xalign(1.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["snapshot-safety"])
            .build();
        self.append(&safety);
    }

    /// Shows the banner for `snapshot`, or hides it for a live folder.
    pub(crate) fn show_snapshot(&self, snapshot: Option<&SnapshotLocation>) {
        let Some(snapshot) = snapshot else {
            self.set_visible(false);
            return;
        };
        let (text, explanation) = date_line(snapshot);
        let date = &self.imp().date;
        date.set_text(&text);
        date.set_tooltip_text(Some(explanation));
        self.set_visible(true);
    }

    /// The date text shown, for tests.
    #[cfg(test)]
    pub(crate) fn date_text(&self) -> String {
        self.imp().date.text().to_string()
    }
}
