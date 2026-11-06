// SPDX-License-Identifier: AGPL-3.0-only
//! [`FixedWidth`]: holds the Properties window's content at Windows 11's
//! width, whatever an item's name, location or other values say.
//!
//! A window that cannot be resized takes its content's natural width, and
//! a label's natural width is its whole text, even when it wraps or ends
//! in "…". So a long name or path would widen the window, as Windows'
//! Properties never are. The container asks for the child's minimum width
//! and at most the window's width, so wrapping values wrap and ending
//! values end with "…" at the window's width, on every tab.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::FixedWidth`].
    #[derive(Debug, Default)]
    pub(crate) struct FixedWidth {
        /// The content.
        pub(super) child: RefCell<Option<gtk::Widget>>,
        /// The widest the content asks to be.
        pub(super) width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FixedWidth {
        const NAME: &'static str = "OxFixedWidth";
        type Type = super::FixedWidth;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for FixedWidth {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for FixedWidth {
        /// Heights are measured for a width, so wrapped values get the
        /// lines they need.
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else {
                return (0, 0, -1, -1);
            };
            if orientation == gtk::Orientation::Horizontal {
                let (minimum, _, _, _) = child.measure(orientation, -1);
                return (minimum, minimum.max(self.width.get()), -1, -1);
            }
            let width = if for_size < 0 {
                let (minimum, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                minimum.max(self.width.get())
            } else {
                for_size
            };
            child.measure(orientation, width)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                child.allocate(width, height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    /// Holds its child at a fixed width.
    pub(crate) struct FixedWidth(ObjectSubclass<imp::FixedWidth>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FixedWidth {
    /// Holds `child`, asking for at most `width` pixels across, or its
    /// minimum when that is wider.
    pub(crate) fn new(child: &impl IsA<gtk::Widget>, width: i32) -> Self {
        let holder: Self = glib::Object::new();
        holder.imp().width.set(width);
        child.set_parent(&holder);
        holder.imp().child.replace(Some(child.clone().upcast()));
        holder
    }
}
