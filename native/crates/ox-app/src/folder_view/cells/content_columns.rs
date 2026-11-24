// SPDX-License-Identifier: AGPL-3.0-only
//! Holds a Content row's columns beside its icon and starts each where
//! Windows 11 File Explorer starts it (VIEW-071).
//!
//! Windows' Content row has up to three columns: the name, a picture's
//! "Type:" and "Dimensions:", and "Date modified:" and "Size:". Measured on
//! Windows 11 (Compact view, 100%) in a 1745-pixel row and in a 775-pixel
//! one (a window snapped to half of a 1920-pixel screen), the columns'
//! text starts, from the row's left edge:
//!
//! | Row | Type: | Date modified: |
//! |---|---|---|
//! | 775 | 368 | 498 |
//! | 1745 | 374 | 674 |
//!
//! Neither column is anchored to the right, and neither is a fixed share
//! of the row: the Type column stays almost in place, and the date column
//! starts 357 pixels in plus 18% of the row. Both are taken as linear in
//! the row's width through these two measures. A name is cut with "..."
//! 15 pixels before the next column, and the date column never runs past
//! the row's end.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Pixels between a column's edge and the ink of its first letter.
const INK_OFFSET: i32 = 1;

/// The room left between a cut name's box and the next column, so the
/// "..." ends about 16 pixels before it, as in Windows.
const NAME_GAP: i32 = 8;

/// Where text starting `at_775` pixels into a 775-pixel row and `at_1745`
/// into a 1745-pixel one starts in a `row`-pixel row, on the line through
/// both measures.
fn column_start(row: i32, at_775: f64, at_1745: f64) -> i32 {
    let slope = (at_1745 - at_775) / (1745.0 - 775.0);
    let start = at_775 + slope * f64::from(row - 775);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a row is a few thousand pixels at most"
    )]
    let start = start.round() as i32;
    start
}

mod imp {
    use std::cell::RefCell;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{glib, graphene};

    #[derive(Default)]
    pub(crate) struct ContentColumns {
        /// The name's lines, a picture's type and dimensions, and the date
        /// and size, in that order.
        pub(super) columns: RefCell<Option<[gtk::Widget; 3]>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContentColumns {
        const NAME: &'static str = "OxContentColumns";
        type Type = super::ContentColumns;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ContentColumns {
        fn dispose(&self) {
            for column in self.columns.take().into_iter().flatten() {
                column.unparent();
            }
        }
    }

    impl WidgetImpl for ContentColumns {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(columns) = self.columns.borrow().clone() else {
                return (0, 0, -1, -1);
            };
            let sizes = columns
                .iter()
                .filter(|column| column.is_visible())
                .map(|column| column.measure(orientation, -1));
            let (minimum, natural) = if orientation == gtk::Orientation::Horizontal {
                sizes.fold((0, 0), |(min, nat), size| (min + size.0, nat + size.1))
            } else {
                sizes.fold((0, 0), |(min, nat), size| (min.max(size.0), nat.max(size.1)))
            };
            let _ = for_size;
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some([lines, pictures, properties]) = self.columns.borrow().clone() else {
                return;
            };
            let (start, row) = self.row_span().unwrap_or((0, width));
            let natural = |column: &gtk::Widget| column.measure(gtk::Orientation::Horizontal, -1).1;
            let place = |column: &gtk::Widget, x: i32, column_width: i32| {
                let at = gsk_offset(x);
                column.allocate(column_width.max(0), height, -1, Some(at));
            };
            // The date column keeps its whole width inside the row.
            let date_width = natural(&properties);
            let date_x = (super::column_start(row, 498.0, 674.0) - start - super::INK_OFFSET)
                .min(width - date_width)
                .max(0);
            place(&properties, date_x, date_width);
            let mut name_end = date_x;
            if pictures.is_visible() {
                let type_x =
                    (super::column_start(row, 368.0, 374.0) - start - super::INK_OFFSET).min(name_end);
                place(&pictures, type_x, natural(&pictures).min(name_end - type_x));
                name_end = type_x;
            }
            let least = lines.measure(gtk::Orientation::Horizontal, -1).0;
            place(&lines, 0, (name_end - super::NAME_GAP).max(least));
        }
    }

    /// A move `x` pixels to the right.
    fn gsk_offset(x: i32) -> gtk::gsk::Transform {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a row is a few thousand pixels at most"
        )]
        let x = x as f32;
        gtk::gsk::Transform::new().translate(&graphene::Point::new(x, 0.0))
    }

    impl ContentColumns {
        /// Where the columns start in their row, and the row's width: the
        /// grid's child, whose edges are the row's.
        fn row_span(&self) -> Option<(i32, i32)> {
            let columns = self.obj();
            let row = std::iter::successors(columns.parent(), WidgetExt::parent).find(|widget| {
                widget
                    .parent()
                    .is_some_and(|parent| parent.css_name() == "gridview")
            })?;
            let start = columns.compute_point(&row, &graphene::Point::zero())?;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a row is a few thousand pixels at most"
            )]
            let start = start.x().round() as i32;
            Some((start, row.width()))
        }
    }
}

glib::wrapper! {
    /// A Content row's name, picture and date columns.
    pub(crate) struct ContentColumns(ObjectSubclass<imp::ContentColumns>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ContentColumns {
    /// Holds `lines` (the name), `pictures` (a picture's type and
    /// dimensions) and `properties` (the date and size).
    pub(crate) fn new(
        lines: &impl IsA<gtk::Widget>,
        pictures: &impl IsA<gtk::Widget>,
        properties: &impl IsA<gtk::Widget>,
    ) -> Self {
        let columns: Self = glib::Object::new();
        let children = [
            lines.clone().upcast(),
            pictures.clone().upcast(),
            properties.clone().upcast(),
        ];
        for child in &children {
            child.set_parent(&columns);
        }
        columns.imp().columns.replace(Some(children));
        columns
    }
}
