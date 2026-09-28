// SPDX-License-Identifier: AGPL-3.0-only
//! How the tab strip shares its width between tabs.
//!
//! Ports the flex rules of `.tabs` and `.tab` in `desktop/ui/style.css`:
//! every tab is 215 pixels wide (180 or 150 in a narrow window), tabs
//! shrink evenly toward 100 pixels when the title bar runs out of room, and
//! below that the strip scrolls. A `GtkBox` cannot shrink its children
//! below their natural width without also dropping to their minimum, so
//! the tab strip uses this layout.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::widget_tree::{laid_out_children, widest_minimum_width};

/// A tab's width when there is room (`.tab{width:215px}`).
pub(super) const TAB_WIDTH: i32 = 215;
/// The narrowest a tab gets before the strip scrolls: the Windows 11 tab
/// minimum (ui-spec.md §4.1), where the web's `min-width:80px` left too
/// little of a title to read.
pub(super) const MIN_TAB_WIDTH: i32 = 100;
/// The space between tabs (`.tabs{gap:2px}`).
pub(super) const TAB_GAP: i32 = 2;

/// The widths a tab may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabWidths {
    /// The width when there is room.
    pub widest: i32,
    /// The narrowest a tab's contents let it get; a large text size can
    /// make this more than [`MIN_TAB_WIDTH`].
    pub narrowest: i32,
}

impl TabWidths {
    /// The narrowest a tab gets: [`MIN_TAB_WIDTH`], or more when its
    /// contents need it.
    fn narrowest_allowed(self) -> i32 {
        self.narrowest.max(MIN_TAB_WIDTH)
    }

    /// The widest a tab gets: its width when there is room, never less
    /// than [`Self::narrowest_allowed`].
    fn widest_allowed(self) -> i32 {
        self.widest.max(self.narrowest_allowed())
    }
}

/// The width each of `count` tabs gets in `available` pixels.
pub(super) fn tab_width(available: i32, count: i32, widths: TabWidths) -> i32 {
    if count == 0 {
        return widths.widest_allowed();
    }
    let gaps = TAB_GAP * (count - 1);
    let even_share = (available - gaps) / count;
    even_share.clamp(widths.narrowest_allowed(), widths.widest_allowed())
}

/// The strip's width for `count` tabs of `width` pixels.
fn strip_width(count: i32, width: i32) -> i32 {
    if count == 0 {
        return 0;
    }
    count * width + TAB_GAP * (count - 1)
}

mod imp {
    use std::cell::Cell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{
        laid_out_children, strip_width, tab_width, widest_minimum_width, TabWidths, MIN_TAB_WIDTH, TAB_GAP,
        TAB_WIDTH,
    };

    /// Private state of [`super::TabLayout`].
    #[derive(Debug)]
    pub struct TabLayout {
        /// A tab's width when there is room.
        pub(super) widest: Cell<i32>,
    }

    impl Default for TabLayout {
        fn default() -> Self {
            Self {
                widest: Cell::new(TAB_WIDTH),
            }
        }
    }

    impl TabLayout {
        fn widths(&self, tabs: &[gtk::Widget]) -> TabWidths {
            TabWidths {
                widest: self.widest.get(),
                narrowest: widest_minimum_width(tabs).unwrap_or(MIN_TAB_WIDTH),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TabLayout {
        const NAME: &'static str = "OxTabLayout";
        type Type = super::TabLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for TabLayout {}

    impl LayoutManagerImpl for TabLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let tabs = laid_out_children(widget);
            if orientation == gtk::Orientation::Vertical {
                let height = tabs
                    .iter()
                    .map(|tab| tab.measure(orientation, -1).0)
                    .max()
                    .unwrap_or(0);
                return (height, height, -1, -1);
            }
            let count = i32::try_from(tabs.len()).unwrap_or(i32::MAX);
            let widths = self.widths(&tabs);
            let minimum = strip_width(count, widths.narrowest_allowed());
            let natural = strip_width(count, widths.widest_allowed());
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let tabs = laid_out_children(widget);
            let count = i32::try_from(tabs.len()).unwrap_or(i32::MAX);
            let each = tab_width(width, count, self.widths(&tabs));
            let mut x = 0;
            for tab in tabs {
                let placement = gtk::Allocation::new(x, 0, each, height);
                tab.size_allocate(&placement, -1);
                x += each + TAB_GAP;
            }
        }
    }
}

glib::wrapper! {
    /// Lays out the tab strip's tabs side by side at an equal width.
    pub struct TabLayout(ObjectSubclass<imp::TabLayout>)
        @extends gtk::LayoutManager;
}

impl TabLayout {
    /// Makes tabs `width` pixels wide when there is room.
    pub fn set_tab_width(&self, width: i32) {
        if self.imp().widest.replace(width) != width {
            self.layout_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(widest: i32, narrowest: i32) -> TabWidths {
        TabWidths { widest, narrowest }
    }

    #[test]
    fn tabs_are_full_width_while_there_is_room() {
        assert_eq!(tab_width(1000, 3, widths(TAB_WIDTH, MIN_TAB_WIDTH)), TAB_WIDTH);
        assert_eq!(
            tab_width(1000, 3, widths(180, MIN_TAB_WIDTH)),
            180,
            "a narrow window"
        );
    }

    #[test]
    fn tabs_shrink_evenly_then_stop_at_their_minimum() {
        assert_eq!(tab_width(302, 2, widths(TAB_WIDTH, MIN_TAB_WIDTH)), 150);
        assert_eq!(
            tab_width(100, 4, widths(TAB_WIDTH, MIN_TAB_WIDTH)),
            100,
            "WinUI's minimum"
        );
    }

    #[test]
    fn tabs_never_get_narrower_than_their_contents_need() {
        assert_eq!(tab_width(100, 4, widths(TAB_WIDTH, 112)), 112);
        assert_eq!(tab_width(2000, 1, widths(TAB_WIDTH, 240)), 240);
    }
}
