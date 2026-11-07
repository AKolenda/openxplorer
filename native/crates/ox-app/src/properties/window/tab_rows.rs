// SPDX-License-Identifier: AGPL-3.0-only
//! [`TabRows`]: tabs in as many rows as the window's width needs, as
//! Windows' property sheets lay them out. There are as few rows as the
//! tabs fit in, the tabs spread over them evenly in order; with more than
//! one row, each row is stretched to the full width, and the row holding the tab shown sits
//! next to the page, the others above it in their order.

use gtk::glib;
use gtk::prelude::*;

mod imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::TabRows`].
    #[derive(Debug, Default)]
    pub(crate) struct TabRows {}

    #[glib::object_subclass]
    impl ObjectSubclass for TabRows {
        const NAME: &'static str = "OxTabRows";
        type Type = super::TabRows;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("tabrows");
        }
    }

    impl ObjectImpl for TabRows {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for TabRows {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let tabs = self.obj().shown_tabs();
            if orientation == gtk::Orientation::Horizontal {
                // At least the widest tab; at best all of them on one row.
                let minimum = tabs.iter().map(|tab| tab.width).max().unwrap_or(0);
                let natural = tabs.iter().map(|tab| tab.width).sum::<i32>();
                return (minimum, natural.max(minimum), -1, -1);
            }
            let width = if for_size < 0 { i32::MAX } else { for_size };
            let height = super::rows(&tabs, width)
                .iter()
                .map(|row| row.iter().map(|&at| tabs[at].height).max().unwrap_or(0))
                .sum::<i32>();
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let widget = self.obj();
            let tabs = widget.shown_tabs();
            let mut rows = super::rows(&tabs, width);
            // The row of the tab shown goes next to the page.
            if let Some(shown) = rows
                .iter()
                .position(|row| row.iter().any(|&at| tabs[at].is_shown))
            {
                let row = rows.remove(shown);
                rows.push(row);
            }
            let stretch = rows.len() > 1;
            let mut y = 0;
            for row in &rows {
                let row_height = row.iter().map(|&at| tabs[at].height).max().unwrap_or(0);
                let used: i32 = row.iter().map(|&at| tabs[at].width).sum();
                let count = i32::try_from(row.len()).unwrap_or(1).max(1);
                let spare = if stretch { (width - used).max(0) } else { 0 };
                let mut x = 0;
                for (place, &at) in row.iter().enumerate() {
                    let place = i32::try_from(place).unwrap_or(0);
                    // The spare width is shared out evenly, the last tab
                    // taking what rounding leaves.
                    let extra = spare / count + i32::from(place < spare % count);
                    let tab_width = tabs[at].width + extra;
                    // A tab after another overlaps its right edge by a
                    // pixel, so neighbours share one border line.
                    let overlap = i32::from(place > 0);
                    tabs[at].widget.size_allocate(
                        &gtk::Allocation::new(x - overlap, y, tab_width + overlap, row_height),
                        -1,
                    );
                    x += tab_width;
                }
                y += row_height;
            }
        }
    }
}

glib::wrapper! {
    /// Tabs in rows that fit the width, as Windows' property sheets.
    pub(crate) struct TabRows(ObjectSubclass<imp::TabRows>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for TabRows {
    /// A tab list, for screen readers; the role is fixed at construction.
    fn default() -> Self {
        glib::Object::builder()
            .property("accessible-role", gtk::AccessibleRole::TabList)
            .build()
    }
}

/// One tab as the rows see it.
struct Tab {
    widget: gtk::Widget,
    width: i32,
    height: i32,
    is_shown: bool,
}

impl TabRows {
    /// Adds `tab` after the others.
    pub(crate) fn append(&self, tab: &impl IsA<gtk::Widget>) {
        tab.set_parent(self);
        if let Some(toggle) = tab.dynamic_cast_ref::<gtk::ToggleButton>() {
            // The row of the tab shown moves next to the page.
            toggle.connect_active_notify(glib::clone!(
                #[weak(rename_to = rows)]
                self,
                move |_| rows.queue_allocate()
            ));
        }
        self.queue_resize();
    }

    /// The visible tabs, with their natural sizes.
    fn shown_tabs(&self) -> Vec<Tab> {
        let mut tabs = Vec::new();
        let mut child = self.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if !widget.is_visible() {
                continue;
            }
            let (_, width, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
            let (_, height, _, _) = widget.measure(gtk::Orientation::Vertical, width);
            let is_shown = widget
                .downcast_ref::<gtk::ToggleButton>()
                .is_some_and(ToggleButtonExt::is_active);
            tabs.push(Tab {
                widget,
                width,
                height,
                is_shown,
            });
        }
        tabs
    }
}

/// The tabs of each row, in order, for rows `width` wide. As in
/// Windows' multi-row tabs, there are as few rows as fit the tabs, and
/// the tabs are spread over them as evenly as fits: never one tab alone
/// on a row while another row is full. Of the even spreads, the one with
/// more tabs in the first rows wins, then the one whose widest row is
/// narrowest.
fn rows(tabs: &[Tab], width: i32) -> Vec<Vec<usize>> {
    let widths: Vec<i32> = tabs.iter().map(|tab| tab.width).collect();
    rows_of(&widths, width)
}

/// [`rows`] for tabs `widths` wide.
fn rows_of(widths: &[i32], width: i32) -> Vec<Vec<usize>> {
    if widths.is_empty() {
        return Vec::new();
    }
    let count = greedy_row_count(widths, width);
    if count <= 1 {
        return vec![(0..widths.len()).collect()];
    }
    let mut best: Option<(Spread, Vec<usize>)> = None;
    let mut sizes = Vec::with_capacity(count);
    spreads(widths, width, count, &mut sizes, &mut |sizes| {
        let spread = Spread::of(widths, sizes);
        if best.as_ref().is_none_or(|(kept, _)| spread < *kept) {
            best = Some((spread, sizes.to_vec()));
        }
    });
    let Some((_, sizes)) = best else {
        // Some tab is wider than the row: one tab a row, in order.
        return (0..widths.len()).map(|at| vec![at]).collect();
    };
    let mut start = 0;
    sizes
        .iter()
        .map(|&size| {
            let row = (start..start + size).collect();
            start += size;
            row
        })
        .collect()
}

/// How many rows filling each until the next tab does not fit takes: the
/// fewest rows the tabs fit in, in order.
fn greedy_row_count(widths: &[i32], width: i32) -> usize {
    let mut count = 0;
    let mut used = 0;
    for &tab in widths {
        if count == 0 || used + tab > width {
            count += 1;
            used = tab;
        } else {
            used += tab;
        }
    }
    count
}

/// How even a spread of tabs over rows is; smaller is better.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Spread {
    /// The most tabs in a row less the fewest.
    count_gap: usize,
    /// The rows' sizes, reversed, so more tabs in the first rows wins, as
    /// Windows' C: drive has four tabs in front and three behind.
    front: Vec<std::cmp::Reverse<usize>>,
    /// The widest row's tabs.
    widest: i32,
}

impl Spread {
    fn of(widths: &[i32], sizes: &[usize]) -> Self {
        let mut start = 0;
        let mut widest = 0;
        for &size in sizes {
            widest = widest.max(widths[start..start + size].iter().sum());
            start += size;
        }
        let most = sizes.iter().copied().max().unwrap_or(0);
        let fewest = sizes.iter().copied().min().unwrap_or(0);
        Self {
            count_gap: most - fewest,
            front: sizes.iter().map(|&size| std::cmp::Reverse(size)).collect(),
            widest,
        }
    }
}

/// Calls `found` with each way of cutting `widths`, in order, into `rows`
/// rows of at least one tab, every row fitting `width`.
fn spreads(
    widths: &[i32],
    width: i32,
    rows: usize,
    sizes: &mut Vec<usize>,
    found: &mut impl FnMut(&[usize]),
) {
    if rows == 1 {
        if !widths.is_empty() && widths.iter().sum::<i32>() <= width {
            sizes.push(widths.len());
            found(sizes);
            sizes.pop();
        }
        return;
    }
    let mut used = 0;
    for size in 1..=widths.len().saturating_sub(rows - 1) {
        used += widths[size - 1];
        if used > width {
            break;
        }
        sizes.push(size);
        spreads(&widths[size..], width, rows - 1, sizes, found);
        sizes.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::rows_of;

    /// Five tabs that need two rows take three and two, not four and one.
    #[test]
    fn rows_share_the_tabs_evenly() {
        let widths = [60, 70, 60, 70, 120];
        assert_eq!(rows_of(&widths, 300), vec![vec![0, 1, 2], vec![3, 4]]);
        // Windows' C: drive: General, Tools, Hardware, Sharing in front,
        // Security, Previous Versions, Quota behind.
        let drive = [59, 48, 68, 62, 62, 119, 52];
        assert_eq!(rows_of(&drive, 380), vec![vec![0, 1, 2, 3], vec![4, 5, 6]]);
    }

    /// Tabs that fit take one row; seven tabs over three rows take three,
    /// two and two.
    #[test]
    fn rows_are_as_few_as_fit() {
        assert_eq!(rows_of(&[50, 50, 50], 300), vec![vec![0, 1, 2]]);
        let widths = [80; 7];
        assert_eq!(rows_of(&widths, 250), vec![vec![0, 1, 2], vec![3, 4], vec![5, 6]]);
    }

    /// A tab wider than the row still shows, alone on its row.
    #[test]
    fn a_wide_tab_takes_its_own_row() {
        assert_eq!(rows_of(&[400, 50], 300), vec![vec![0], vec![1]]);
    }
}
