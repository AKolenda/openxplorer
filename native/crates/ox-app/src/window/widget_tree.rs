// SPDX-License-Identifier: AGPL-3.0-only
//! Walking, measuring and emptying a widget's children.
//!
//! GTK 4 links a widget's children as a chain (`first_child`, then
//! `next_sibling`), and `GtkBox` has no call that removes them all. The
//! window's frame redraws its crumbs, tabs, caption buttons and landing
//! pages by replacing their children, and its layouts measure the children
//! they place, so these helpers keep that walk in one place. The web app
//! does the same with `innerHTML = ''` and `children` in
//! `v2.0.0:desktop/ui/app.js`.

use gtk::prelude::*;

/// The children of `widget`, first to last.
pub(crate) fn children(widget: &impl IsA<gtk::Widget>) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), WidgetExt::next_sibling)
}

/// Every descendant of `widget` of type `T`, in tree order.
pub(crate) fn descendants<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut found = Vec::new();
    for child in children(widget) {
        let below = descendants::<T>(&child);
        if let Ok(matching) = child.downcast::<T>() {
            found.push(matching);
        }
        found.extend(below);
    }
    found
}

/// The children of `widget` that a layout should place: the visible ones.
pub(super) fn laid_out_children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    children(widget).filter(WidgetExt::should_layout).collect()
}

/// The widest of the minimum widths of `widgets`, or `None` for none: the
/// width a layout cannot squeeze any of them below.
pub(super) fn widest_minimum_width(widgets: &[gtk::Widget]) -> Option<i32> {
    let minimum_width = |widget: &gtk::Widget| widget.measure(gtk::Orientation::Horizontal, -1).0;
    widgets.iter().map(minimum_width).max()
}

/// Removes every child of `container`.
pub(super) fn remove_children(container: &impl IsA<gtk::Box>) {
    let container = container.upcast_ref::<gtk::Box>();
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
