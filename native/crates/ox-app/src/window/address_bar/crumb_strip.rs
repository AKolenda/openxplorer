// SPDX-License-Identifier: AGPL-3.0-only
//! The row of crumbs, laid out as Windows 11 File Explorer's address bar
//! lays out its own: a chevron first, which lists the top-level places,
//! then the crumbs and the dividers between them. When the crumbs do not
//! fit, the leading ones collapse, from the first on, into a « button in
//! the chevron's place, whose menu lists them; the current folder's crumb
//! always shows in full. Each allocation decides again, so widening the
//! window brings the crumbs back. The chevron or the « takes the keyboard
//! as the crumbs do, so the hidden crumbs stay a Tab or a Left away.
//!
//! [`CrumbStrip`] never asks for more width than it gets: its minimum
//! width is nothing, so a deep path never widens the window, and a last
//! crumb wider than the bar is cut off at the bar's end.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

/// The gap between two items of the row, as the crumbs' box had.
const SPACING: i32 = 2;

/// The CSS class of the chevron that lists the top-level places.
pub(super) const ROOT_CLASS: &str = "crumb-root";

/// The CSS class of the « button that lists the hidden crumbs.
pub(super) const OVERFLOW_CLASS: &str = "crumb-overflow";

/// The CSS class of a divider between crumbs.
pub(super) const DIVIDER_CLASS: &str = "crumb-divider";

/// The CSS class of a crumb button.
pub(super) const CRUMB_CLASS: &str = "crumb";

mod imp {
    use std::cell::OnceCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::CrumbStrip`]: the two buttons it always
    /// has, before the crumbs.
    #[derive(Debug, Default)]
    pub(crate) struct CrumbStrip {
        /// The chevron after the location icon.
        pub(super) root: OnceCell<gtk::Button>,
        /// The « button shown in its place while crumbs are hidden.
        pub(super) overflow: OnceCell<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CrumbStrip {
        const NAME: &'static str = "OxCrumbStrip";
        type Type = super::CrumbStrip;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for CrumbStrip {
        fn constructed(&self) {
            self.parent_constructed();
            let strip = self.obj();
            strip.set_overflow(gtk::Overflow::Hidden);
            let root = super::strip_button(super::ROOT_CLASS);
            let overflow = super::strip_button(super::OVERFLOW_CLASS);
            overflow.set_child_visible(false);
            overflow.set_can_focus(false);
            root.set_parent(&*strip);
            overflow.set_parent(&*strip);
            self.root.set(root).expect("constructed runs once");
            self.overflow.set(overflow).expect("constructed runs once");
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for CrumbStrip {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let strip = self.obj();
            if orientation == gtk::Orientation::Horizontal {
                // Everything in full, the chevron included and the «
                // left out; but never a minimum, so the bar can shrink.
                let widths: Vec<i32> = strip.items().iter().map(super::natural_width).collect();
                let natural = super::row_width(super::natural_width(strip.root_button()), &widths);
                return (0, natural, -1, -1);
            }
            let mut tallest = (0, 0);
            let mut child = strip.first_child();
            while let Some(widget) = child {
                let (minimum, natural, _, _) = widget.measure(gtk::Orientation::Vertical, -1);
                tallest = (tallest.0.max(minimum), tallest.1.max(natural));
                child = widget.next_sibling();
            }
            (tallest.0, tallest.1, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.obj().lay_out(width, height);
        }
    }
}

glib::wrapper! {
    /// The crumbs of the address bar, with the chevron or « before them.
    pub(crate) struct CrumbStrip(ObjectSubclass<imp::CrumbStrip>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// A chevron button of the row, its glyph drawn by the skin
/// (`resources/skin/navigation.css`) so it turns round right to left.
/// Like a crumb it takes the keyboard, and Down opens its menu as Enter
/// and Space do, as Down on a crumb opens that crumb's.
fn strip_button(class: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .css_classes([class])
        .focus_on_click(false)
        .build();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|keys, key, _, modifiers| {
        let down = matches!(key, gdk::Key::Down | gdk::Key::KP_Down);
        if !down || !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        if let Some(button) = keys.widget().and_downcast::<gtk::Button>() {
            button.emit_clicked();
        }
        glib::Propagation::Stop
    });
    button.add_controller(keys);
    button
}

/// The width `widget` asks for.
fn natural_width(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.measure(gtk::Orientation::Horizontal, -1).1
}

/// The width of `lead` and then `widths`, each [`SPACING`] apart.
fn row_width(lead: i32, widths: &[i32]) -> i32 {
    widths.iter().fold(lead, |total, width| total + SPACING + width)
}

/// One crumb and the divider before it, as indices into the row's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    divider: Option<usize>,
    crumb: usize,
}

/// The crumbs of `items` (whether each is a crumb, in order) with the
/// divider before each. Dividers come only between crumbs.
fn segments(is_crumb: &[bool]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut divider = None;
    for (index, &crumb) in is_crumb.iter().enumerate() {
        if crumb {
            segments.push(Segment {
                divider,
                crumb: index,
            });
            divider = None;
        } else {
            divider = Some(index);
        }
    }
    segments
}

/// How many leading crumbs to hide so the rest fit in `available`: the
/// chevron (`root`) leads while none is hidden, the « (`overflow`) once
/// any is, and the first crumb shown loses the divider before it. The
/// last crumb always shows, even when it alone is too wide.
fn hidden_count(segments: &[Segment], widths: &[i32], root: i32, overflow: i32, available: i32) -> usize {
    let shown_width = |first: usize| {
        let mut shown = Vec::new();
        for (index, segment) in segments.iter().enumerate().skip(first) {
            if let (Some(divider), false) = (segment.divider, index == first) {
                shown.push(widths[divider]);
            }
            shown.push(widths[segment.crumb]);
        }
        let lead = if first == 0 { root } else { overflow };
        row_width(lead, &shown)
    };
    let last = segments.len().saturating_sub(1);
    (0..last)
        .find(|&first| shown_width(first) <= available)
        .unwrap_or(last)
}

impl CrumbStrip {
    /// The chevron that lists the top-level places.
    pub(super) fn root_button(&self) -> &gtk::Button {
        self.imp().root.get().expect("built with the strip")
    }

    /// The « button that lists the hidden crumbs.
    pub(super) fn overflow_button(&self) -> &gtk::Button {
        self.imp().overflow.get().expect("built with the strip")
    }

    /// Adds a crumb or a divider at the end.
    pub(super) fn append(&self, widget: &impl IsA<gtk::Widget>) {
        widget.set_parent(self);
    }

    /// Removes every crumb and divider.
    pub(super) fn clear(&self) {
        for item in self.items() {
            item.unparent();
        }
    }

    /// The crumbs and dividers, first to last.
    pub(super) fn items(&self) -> Vec<gtk::Widget> {
        let overflow = self.overflow_button().upcast_ref::<gtk::Widget>();
        let mut items = Vec::new();
        let mut child = overflow.next_sibling();
        while let Some(widget) = child {
            child = widget.next_sibling();
            items.push(widget);
        }
        items
    }

    /// The buttons the keyboard stops at, first to last: the chevron or
    /// the «, whichever shows, then the crumbs that show.
    pub(super) fn keyboard_stops(&self) -> Vec<gtk::Button> {
        let mut stops = Vec::new();
        let mut child = self.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is_child_visible() {
                stops.extend(widget.downcast::<gtk::Button>().ok());
            }
        }
        stops
    }

    /// The crumbs collapsed into the « button, first to last.
    pub(super) fn hidden_crumbs(&self) -> Vec<gtk::Button> {
        self.items()
            .into_iter()
            .filter(|item| item.has_css_class(CRUMB_CLASS) && !item.is_child_visible())
            .filter_map(|item| item.downcast::<gtk::Button>().ok())
            .collect()
    }

    /// Hides the leading crumbs that do not fit in `width`, then places
    /// the rest from the start (the right, right to left), each as high
    /// as the row.
    fn lay_out(&self, width: i32, height: i32) {
        let items = self.items();
        let widths: Vec<i32> = items.iter().map(natural_width).collect();
        let is_crumb: Vec<bool> = items.iter().map(|item| item.has_css_class(CRUMB_CLASS)).collect();
        let segments = segments(&is_crumb);
        let root = self.root_button();
        let overflow = self.overflow_button();
        let hidden = hidden_count(
            &segments,
            &widths,
            natural_width(root),
            natural_width(overflow),
            width,
        );
        let mut shown = vec![true; items.len()];
        for (index, segment) in segments.iter().enumerate().take(hidden + 1) {
            if let Some(divider) = segment.divider {
                shown[divider] = false;
            }
            if index < hidden {
                shown[segment.crumb] = false;
            }
        }
        let lead: [(gtk::Widget, i32, bool); 2] = [
            (
                root.clone().upcast(),
                natural_width(root),
                hidden == 0 && !segments.is_empty(),
            ),
            (overflow.clone().upcast(), natural_width(overflow), hidden > 0),
        ];
        let rest = items.into_iter().zip(widths).zip(shown);
        let row = lead
            .into_iter()
            .chain(rest.map(|((item, item_width), item_shown)| (item, item_width, item_shown)));
        // The chevron and the « take each other's place, and the focus.
        let lead_focused = root.has_focus() || overflow.has_focus();
        let shown_lead = if hidden > 0 {
            overflow.clone()
        } else {
            root.clone()
        };
        let rtl = self.direction() == gtk::TextDirection::Rtl;
        let mut x = 0;
        for (widget, child_width, visible) in row {
            widget.set_child_visible(visible);
            // A hidden crumb or chevron is no stop for Tab or the arrow keys.
            if widget.is::<gtk::Button>() {
                widget.set_can_focus(visible);
            }
            if !visible {
                continue;
            }
            let start = if rtl { width - x - child_width } else { x };
            widget.size_allocate(&gtk::Allocation::new(start, 0, child_width, height), -1);
            x += child_width + SPACING;
        }
        if lead_focused && shown_lead.is_child_visible() && !shown_lead.has_focus() {
            shown_lead.grab_focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of `count` crumbs `crumb` wide with `divider`-wide dividers
    /// between them: its segments and item widths.
    fn row(count: usize, crumb: i32, divider: i32) -> (Vec<Segment>, Vec<i32>) {
        let mut is_crumb = Vec::new();
        let mut widths = Vec::new();
        for index in 0..count {
            if index > 0 {
                is_crumb.push(false);
                widths.push(divider);
            }
            is_crumb.push(true);
            widths.push(crumb);
        }
        (segments(&is_crumb), widths)
    }

    #[test]
    fn crumbs_that_fit_all_show_after_the_chevron() {
        let (segments, widths) = row(3, 50, 20);
        // 20 + 3 × 50 + 2 × 20 + 5 gaps of 2.
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 220), 0);
    }

    #[test]
    fn leading_crumbs_collapse_until_the_rest_fit_and_the_last_always_shows() {
        let (segments, widths) = row(4, 50, 20);
        // In full: 20 + 4 × 50 + 3 × 20 + 7 gaps of 2.
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 294), 0);
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 293), 1);
        // Without the first crumb: « 50 > 50 > 50, no divider after «.
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 220), 1);
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 219), 2);
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 146), 2);
        assert_eq!(hidden_count(&segments, &widths, 20, 20, 145), 3);
        assert_eq!(
            hidden_count(&segments, &widths, 20, 20, 10),
            3,
            "the last crumb stays"
        );
        assert_eq!(hidden_count(&[], &[], 20, 20, 0), 0);
    }

    #[test]
    fn a_crumb_without_a_divider_before_it_is_its_own_segment() {
        // "/" then "usr", with no divider after the root, then "> share".
        let segments = segments(&[true, true, false, true]);
        assert_eq!(
            segments,
            [
                Segment {
                    divider: None,
                    crumb: 0
                },
                Segment {
                    divider: None,
                    crumb: 1
                },
                Segment {
                    divider: Some(2),
                    crumb: 3
                },
            ]
        );
    }
}
