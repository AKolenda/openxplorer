// SPDX-License-Identifier: AGPL-3.0-only
//! Holds a group heading (its chevron, title and line) in a cell, and in
//! Details lays it out across the whole row, from the row's left edge to
//! its right, over the other columns, as Windows 11 draws it whatever the
//! Name column's width (VIEW-022). It asks only for the heading's least
//! room, so the heading never widens Name; the other columns' cells in
//! the row draw nothing (`skin/folder-views.css`). In an icon grid
//! (Gallery's months, HOME-019) it lays the heading out from its cell to
//! the grid's right edge, as a tile is too narrow for a month's name.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{glib, graphene, gsk};

    #[derive(Default)]
    pub(crate) struct HeadingSpan {
        pub(super) child: RefCell<Option<gtk::Widget>>,
        /// Whether the heading spans its row (Details).
        pub(super) spans_row: Cell<bool>,
        /// Whether the heading runs on to the grid's right edge (icon
        /// tiles).
        pub(super) spans_grid: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HeadingSpan {
        const NAME: &'static str = "OxHeadingSpan";
        type Type = super::HeadingSpan;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for HeadingSpan {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for HeadingSpan {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else {
                return (0, 0, -1, -1);
            };
            let (minimum, natural, _, _) = child.measure(orientation, for_size);
            let spans = self.spans_row.get() || self.spans_grid.get();
            if spans && orientation == gtk::Orientation::Horizontal {
                // The row gives the heading its room, not the column.
                return (minimum, minimum, -1, -1);
            }
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.child.borrow().clone() else {
                return;
            };
            if let Some((start, row_width)) = self.row_span() {
                let at = gsk::Transform::new().translate(&graphene::Point::new(-start, 0.0));
                child.allocate(row_width.max(width), height, -1, Some(at));
            } else if let Some(room) = self.grid_room() {
                child.allocate(room.max(width), height, baseline, None);
            } else {
                child.allocate(width, height, baseline, None);
            }
        }
    }

    impl HeadingSpan {
        /// The room from the span's left edge to its grid's right edge,
        /// while the heading runs on there.
        fn grid_room(&self) -> Option<i32> {
            if !self.spans_grid.get() {
                return None;
            }
            let span = self.obj();
            let grid = std::iter::successors(span.parent(), WidgetExt::parent)
                .find(|widget| widget.css_name() == "gridview")?;
            let start = span.compute_point(&grid, &graphene::Point::zero())?;
            // Stops short of the grid's right padding, where the tiles end.
            let end = grid.width() - 24;
            #[allow(clippy::cast_possible_truncation)]
            Some(end - start.x() as i32)
        }

        /// Where the span starts in its row, and the row's width, while
        /// the heading spans its row.
        fn row_span(&self) -> Option<(f32, i32)> {
            if !self.spans_row.get() {
                return None;
            }
            let span = self.obj();
            let row = std::iter::successors(span.parent(), WidgetExt::parent)
                .find(|widget| widget.css_name() == "row")?;
            let start = span.compute_point(&row, &graphene::Point::zero())?;
            Some((start.x(), row.width()))
        }
    }
}

glib::wrapper! {
    /// A cell's group heading, which in Details spans its row.
    pub(crate) struct HeadingSpan(ObjectSubclass<imp::HeadingSpan>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl HeadingSpan {
    /// Holds `heading`, across its row when `spans_row`.
    pub(crate) fn new(heading: &impl IsA<gtk::Widget>, spans_row: bool) -> Self {
        let span: Self = glib::Object::new();
        span.imp().spans_row.set(spans_row);
        span.set_overflow(gtk::Overflow::Visible);
        heading.set_parent(&span);
        span.imp().child.replace(Some(heading.clone().upcast()));
        span.set_hexpand(true);
        span
    }

    /// Has the heading run on from its cell to the grid's right edge, as
    /// an icon tile's heading does.
    pub(crate) fn span_grid(&self) {
        self.imp().spans_grid.set(true);
        self.queue_resize();
    }

    /// Lays the heading out across its row again: another column's width
    /// can change the row's while the cell's own stays.
    pub(crate) fn respan(&self) {
        if self.imp().spans_row.get() {
            self.queue_allocate();
        }
    }
}
