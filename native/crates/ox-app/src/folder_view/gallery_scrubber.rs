// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery's date scrubber at the right edge (HOME-028).

use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::GalleryScrubber`].
    #[derive(Debug, Default)]
    pub(crate) struct GalleryScrubber {}

    #[glib::object_subclass]
    impl ObjectSubclass for GalleryScrubber {
        const NAME: &'static str = "OxGalleryScrubber";
        type Type = super::GalleryScrubber;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for GalleryScrubber {}
    impl WidgetImpl for GalleryScrubber {}
}

glib::wrapper! {
    /// Gallery's date scrubber.
    pub(crate) struct GalleryScrubber(ObjectSubclass<imp::GalleryScrubber>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl GalleryScrubber {
    /// The years marked, for tests.
    #[cfg(test)]
    pub(crate) fn year_labels(&self) -> Vec<String> {
        let _ = self.imp();
        Vec::new()
    }

    /// The month the scrubber names at `fraction` of its height, for tests.
    #[cfg(test)]
    pub(crate) fn month_at(&self, _fraction: f64) -> Option<String> {
        None
    }

    /// A click at `fraction` of its height, for tests.
    #[cfg(test)]
    pub(crate) fn click_at(&self, _fraction: f64) {}

    /// A drag from `from` to `to`, fractions of its height, for tests.
    #[cfg(test)]
    pub(crate) fn drag(&self, _from: f64, _to: f64) {}

    /// A press on the up arrow (`up`) or the down arrow, for tests.
    #[cfg(test)]
    pub(crate) fn step(&self, _up: bool) {}
}
