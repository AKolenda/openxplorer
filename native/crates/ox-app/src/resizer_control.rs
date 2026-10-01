// SPDX-License-Identifier: AGPL-3.0-only
//! A resizer as keyboard and screen readers meet it.
//!
//! The Python app's `#sidebar-resizer` and `.column-resizer` handles are
//! `role=separator` elements named "Resize …" with `aria-valuemin`,
//! `aria-valuemax` and `aria-valuenow`. GTK's own drag handles (the
//! `GtkPaned` handle, the edges of the column titles) have roles that
//! cannot be changed, so a [`ResizerControl`] of no width stands beside
//! them: a vertical separator that implements `GtkAccessibleRange`, so
//! screen readers read the width as its value and may set it, and that
//! hands a width set that way to its owner.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Called with a width a screen reader asks for.
    pub(super) type ValueRequested = Box<dyn Fn(f64)>;

    /// Private state of [`super::ResizerControl`].
    #[derive(Default)]
    pub(crate) struct ResizerControl {
        /// Resizes what the control stands for.
        pub(super) value_requested: RefCell<Option<ValueRequested>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResizerControl {
        const NAME: &'static str = "OxResizerControl";
        type Type = super::ResizerControl;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::AccessibleRange,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("resizer");
            klass.set_accessible_role(gtk::AccessibleRole::Separator);
        }
    }

    impl ObjectImpl for ResizerControl {}
    impl WidgetImpl for ResizerControl {}
    impl AccessibleImpl for ResizerControl {}

    impl AccessibleRangeImpl for ResizerControl {
        fn set_current_value(&self, value: f64) -> bool {
            let requested = self.value_requested.borrow();
            let Some(requested) = requested.as_ref() else {
                return false;
            };
            requested(value);
            true
        }
    }
}

glib::wrapper! {
    /// A separator of no width, named after what it resizes, that carries
    /// the width and its limits as its value.
    pub(crate) struct ResizerControl(ObjectSubclass<imp::ResizerControl>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleRange, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ResizerControl {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ResizerControl {
    /// A resizer named `label`, such as "Resize sidebar".
    pub(crate) fn new(label: &str) -> Self {
        let control = Self::default();
        control.set_label(label);
        control
    }

    /// Names the resizer.
    pub(crate) fn set_label(&self, label: &str) {
        self.update_property(&[
            gtk::accessible::Property::Label(label),
            gtk::accessible::Property::Orientation(gtk::Orientation::Vertical),
        ]);
    }

    /// Announces `now` pixels within `min..=max`.
    pub(crate) fn set_values(&self, min: i32, max: i32, now: i32) {
        self.update_property(&[
            gtk::accessible::Property::ValueMin(f64::from(min)),
            gtk::accessible::Property::ValueMax(f64::from(max)),
            gtk::accessible::Property::ValueNow(f64::from(now)),
        ]);
    }

    /// Calls `resize` with the width a screen reader sets.
    pub(crate) fn connect_value_requested(&self, resize: impl Fn(f64) + 'static) {
        self.imp().value_requested.replace(Some(Box::new(resize)));
    }

    /// Asks for `value` as a screen reader would, for tests.
    #[cfg(test)]
    pub(crate) fn request_value(&self, value: f64) -> bool {
        AccessibleRangeImpl::set_current_value(self.imp(), value)
    }
}
