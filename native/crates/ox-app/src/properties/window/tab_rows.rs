// SPDX-License-Identifier: AGPL-3.0-only
//! [`TabRows`]: tabs in as many rows as the window's width needs, as
//! Windows' property sheets lay them out. Tabs fill a row in order until
//! the next one does not fit; with more than one row, each row is
//! stretched to the full width, and the row holding the tab shown sits
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

/// The tabs of each row, in order, for rows `width` wide: a row takes
/// tabs until the next one does not fit, and always at least one.
fn rows(tabs: &[Tab], width: i32) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut used = 0;
    for (at, tab) in tabs.iter().enumerate() {
        match rows.last_mut() {
            Some(row) if used + tab.width <= width => {
                row.push(at);
                used += tab.width;
            }
            _ => {
                rows.push(vec![at]);
                used = tab.width;
            }
        }
    }
    rows
}
