// SPDX-License-Identifier: AGPL-3.0-only
//! How the tab strip shares its width between tabs.
//!
//! Ports the flex rules of `.tabs` and `.tab` in `desktop/ui/style.css`:
//! every tab is 215 pixels wide, tabs shrink evenly toward 80 pixels when
//! the title bar runs out of room, and below that the strip scrolls. A
//! `GtkBox` cannot shrink its children below their natural width without
//! also dropping to their minimum, so the tab strip uses this layout.

use gtk::glib;
use gtk::prelude::*;

/// A tab's width when there is room (`.tab{width:215px}`).
pub(super) const TAB_WIDTH: i32 = 215;
/// The narrowest a tab gets before the strip scrolls (`min-width:80px`).
pub(super) const MIN_TAB_WIDTH: i32 = 80;
/// The space between tabs (`.tabs{gap:2px}`).
pub(super) const TAB_GAP: i32 = 2;

/// The width each of `count` tabs gets in `available` pixels, never less
/// than `narrowest`: [`MIN_TAB_WIDTH`], or more when a tab's contents need
/// it at a large text size.
pub(super) fn tab_width(available: i32, count: i32, narrowest: i32) -> i32 {
    let narrowest = narrowest.max(MIN_TAB_WIDTH);
    let widest = TAB_WIDTH.max(narrowest);
    if count == 0 {
        return widest;
    }
    let gaps = TAB_GAP * (count - 1);
    ((available - gaps) / count).clamp(narrowest, widest)
}

/// The widest minimum width among `tabs`.
fn narrowest_tab(tabs: &[gtk::Widget]) -> i32 {
    let minimum_of = |tab: &gtk::Widget| tab.measure(gtk::Orientation::Horizontal, -1).0;
    tabs.iter().map(minimum_of).max().unwrap_or(MIN_TAB_WIDTH)
}

/// The strip's width for `count` tabs of `width` pixels.
fn strip_width(count: i32, width: i32) -> i32 {
    if count == 0 {
        return 0;
    }
    count * width + TAB_GAP * (count - 1)
}

mod imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{strip_width, tab_width, TAB_GAP};

    /// The tab layout has no state; see [`super::TabLayout`].
    #[derive(Debug, Default)]
    pub struct TabLayout;

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
            let tabs = super::visible_children(widget);
            if orientation == gtk::Orientation::Vertical {
                let height = tabs
                    .iter()
                    .map(|tab| tab.measure(orientation, -1).0)
                    .max()
                    .unwrap_or(0);
                return (height, height, -1, -1);
            }
            let count = i32::try_from(tabs.len()).unwrap_or(i32::MAX);
            let narrowest = super::narrowest_tab(&tabs);
            let minimum = strip_width(count, tab_width(0, count, narrowest));
            let natural = strip_width(count, tab_width(i32::MAX, 1, narrowest));
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let tabs = super::visible_children(widget);
            let count = i32::try_from(tabs.len()).unwrap_or(i32::MAX);
            let each = tab_width(width, count, super::narrowest_tab(&tabs));
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
    /// A layout for a tab strip.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

fn visible_children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        if current.should_layout() {
            children.push(current.clone());
        }
        child = current.next_sibling();
    }
    children
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_are_full_width_while_there_is_room() {
        assert_eq!(tab_width(1000, 3, MIN_TAB_WIDTH), TAB_WIDTH);
    }

    #[test]
    fn tabs_shrink_evenly_then_stop_at_their_minimum() {
        assert_eq!(tab_width(302, 2, MIN_TAB_WIDTH), 150);
        assert_eq!(tab_width(100, 4, MIN_TAB_WIDTH), MIN_TAB_WIDTH);
    }

    #[test]
    fn tabs_never_get_narrower_than_their_contents_need() {
        assert_eq!(tab_width(100, 4, 96), 96);
        assert_eq!(tab_width(2000, 1, 240), 240);
    }
}
