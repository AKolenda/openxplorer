// SPDX-License-Identifier: AGPL-3.0-only
//! Holds a tile's lines (Tiles view) and, each time GTK lays them out,
//! tells its cell, which hides the size while the name takes two lines:
//! Windows 11's tiles show three lines at most, a long name's two and the
//! type (VIEW-071). A `gtk::Box` cannot do this itself: GTK lays a box out
//! through its layout manager, not its own allocation.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub(crate) struct TileLines {
        pub(super) child: RefCell<Option<gtk::Widget>>,
        /// Called after each allocation, once the name is laid out.
        pub(super) laid_out: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TileLines {
        const NAME: &'static str = "OxTileLines";
        type Type = super::TileLines;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for TileLines {
        fn dispose(&self) {
            self.laid_out.take();
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for TileLines {
        // A wrapped name is as tall as the width it gets.
        fn request_mode(&self) -> gtk::SizeRequestMode {
            self.child
                .borrow()
                .as_ref()
                .map_or(gtk::SizeRequestMode::ConstantSize, WidgetExt::request_mode)
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.child
                .borrow()
                .as_ref()
                .map_or((0, 0, -1, -1), |child| child.measure(orientation, for_size))
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                child.allocate(width, height, baseline, None);
            }
            if let Some(laid_out) = self.laid_out.borrow().as_ref() {
                laid_out();
            }
        }
    }
}

glib::wrapper! {
    /// A tile's lines, which report each layout.
    pub(crate) struct TileLines(ObjectSubclass<imp::TileLines>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TileLines {
    /// Holds `lines` and calls `laid_out` after each layout.
    pub(crate) fn new(lines: &impl IsA<gtk::Widget>, laid_out: impl Fn() + 'static) -> Self {
        let frame: Self = glib::Object::new();
        lines.set_parent(&frame);
        frame.imp().child.replace(Some(lines.clone().upcast()));
        frame.imp().laid_out.replace(Some(Box::new(laid_out)));
        frame
    }
}
