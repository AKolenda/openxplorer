// SPDX-License-Identifier: AGPL-3.0-only
//! The landing pages' card grids.
//!
//! Ports `.quick-grid` and `.drive-grid` in `desktop/ui/style.css`:
//! `grid-template-columns: repeat(auto-fit, minmax(<narrowest>, 1fr))`. As
//! many columns as fit at the narrowest width share the row; with fewer
//! cards than columns, the cards share the whole width. `GtkFlowBox` keeps
//! a fixed number of columns and does not stretch them, so the grids use
//! this layout.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::widget_tree::{laid_out_children, widest_minimum_width};

/// A grid's sizes, from its stylesheet rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GridSpacing {
    /// The narrowest a column gets (`minmax(<this>, 1fr)`).
    pub narrowest_column: i32,
    /// Pixels between columns.
    pub column_gap: i32,
    /// Pixels between rows.
    pub row_gap: i32,
}

/// Quick access cards (`.quick-grid`).
pub(super) const QUICK_GRID: GridSpacing = GridSpacing {
    narrowest_column: 170,
    column_gap: 12,
    row_gap: 8,
};

/// Drive and network cards (`.drive-grid`).
pub(super) const DRIVE_GRID: GridSpacing = GridSpacing {
    narrowest_column: 245,
    column_gap: 13,
    row_gap: 13,
};

/// How many columns `count` cards take in `width` pixels: as many as fit
/// at the narrowest width, at least one, never more than there are cards.
fn column_count(spacing: GridSpacing, width: i32, count: i32) -> i32 {
    let track = spacing.narrowest_column + spacing.column_gap;
    let fitting = (width + spacing.column_gap) / track;
    fitting.clamp(1, count.max(1))
}

/// The width of each of `columns` columns in `width` pixels.
fn column_width(spacing: GridSpacing, width: i32, columns: i32) -> i32 {
    let gaps = spacing.column_gap * (columns - 1);
    ((width - gaps) / columns).max(0)
}

mod imp {
    use std::cell::Cell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{
        column_count, column_width, laid_out_children, widest_minimum_width, GridSpacing, QUICK_GRID,
    };

    /// Private state of [`super::CardGridLayout`].
    #[derive(Debug)]
    pub(crate) struct CardGridLayout {
        /// The grid's narrowest column and gaps.
        pub(super) spacing: Cell<GridSpacing>,
    }

    impl Default for CardGridLayout {
        fn default() -> Self {
            Self {
                spacing: Cell::new(QUICK_GRID),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardGridLayout {
        const NAME: &'static str = "OxCardGridLayout";
        type Type = super::CardGridLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for CardGridLayout {}

    impl LayoutManagerImpl for CardGridLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let spacing = self.spacing.get();
            let cards = laid_out_children(widget);
            if orientation == gtk::Orientation::Horizontal {
                // A card is never squeezed below its own minimum.
                let widest_card = widest_minimum_width(&cards).unwrap_or(0);
                let narrowest = spacing.narrowest_column.min(widest_card);
                return (narrowest, narrowest, -1, -1);
            }
            let height = super::rows_height(spacing, &cards, for_size);
            (height, height, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, _height: i32, _baseline: i32) {
            let spacing = self.spacing.get();
            let cards = laid_out_children(widget);
            let count = i32::try_from(cards.len()).unwrap_or(i32::MAX);
            let columns = column_count(spacing, width, count);
            let each = column_width(spacing, width, columns);
            let mut y = 0;
            for row in cards.chunks(usize::try_from(columns).unwrap_or(1)) {
                let row_height = super::tallest(row, each);
                let mut x = 0;
                for card in row {
                    card.size_allocate(&gtk::Allocation::new(x, y, each, row_height), -1);
                    x += each + spacing.column_gap;
                }
                y += row_height + spacing.row_gap;
            }
        }
    }
}

glib::wrapper! {
    /// Lays out cards in stretching columns, as a CSS `auto-fit` grid.
    pub(crate) struct CardGridLayout(ObjectSubclass<imp::CardGridLayout>)
        @extends gtk::LayoutManager;
}

impl CardGridLayout {
    /// A layout with `spacing`.
    pub(super) fn new(spacing: GridSpacing) -> Self {
        let layout: Self = glib::Object::new();
        layout.imp().spacing.set(spacing);
        layout
    }
}

/// An empty card grid with `spacing`.
pub(super) fn card_grid(spacing: GridSpacing) -> gtk::Box {
    gtk::Box::builder()
        .layout_manager(&CardGridLayout::new(spacing))
        .css_classes(["card-grid"])
        .build()
}

/// The height of the tallest of `cards` at `width`.
fn tallest(cards: &[gtk::Widget], width: i32) -> i32 {
    let height_of = |card: &gtk::Widget| card.measure(gtk::Orientation::Vertical, width).1;
    cards.iter().map(height_of).max().unwrap_or(0)
}

/// The height of `cards` laid out in `width` pixels (any width when -1).
fn rows_height(spacing: GridSpacing, cards: &[gtk::Widget], width: i32) -> i32 {
    if cards.is_empty() {
        return 0;
    }
    let count = i32::try_from(cards.len()).unwrap_or(i32::MAX);
    let columns = if width < 0 {
        count
    } else {
        column_count(spacing, width, count)
    };
    let each = if width < 0 {
        -1
    } else {
        column_width(spacing, width, columns)
    };
    let rows = cards.chunks(usize::try_from(columns).unwrap_or(1));
    let row_count = i32::try_from(rows.len()).unwrap_or(i32::MAX);
    let heights: i32 = rows.map(|row| tallest(row, each)).sum();
    heights + spacing.row_gap * (row_count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid, a width and a card count, and the columns and column width
    /// they get.
    struct GridCase {
        spacing: GridSpacing,
        width: i32,
        cards: i32,
        columns: i32,
        column_width: i32,
    }

    #[test]
    fn columns_fit_at_their_narrowest_and_stretch_to_the_width() {
        // The reference This PC page at 1440 pixels has 906 pixels of page:
        // six Quick access folders in five columns, two drives and three
        // network locations sharing the whole width.
        let cases = [
            GridCase {
                spacing: QUICK_GRID,
                width: 906,
                cards: 6,
                columns: 5,
                column_width: 171,
            },
            GridCase {
                spacing: DRIVE_GRID,
                width: 906,
                cards: 2,
                columns: 2,
                column_width: 446,
            },
            GridCase {
                spacing: DRIVE_GRID,
                width: 906,
                cards: 3,
                columns: 3,
                column_width: 293,
            },
            GridCase {
                spacing: DRIVE_GRID,
                width: 100,
                cards: 3,
                columns: 1,
                column_width: 100,
            },
        ];
        for case in cases {
            let columns = column_count(case.spacing, case.width, case.cards);
            assert_eq!(columns, case.columns, "{} cards in {}", case.cards, case.width);
            assert_eq!(column_width(case.spacing, case.width, columns), case.column_width);
        }
    }
}
