// SPDX-License-Identifier: AGPL-3.0-only
//! Walking and emptying a widget's children.
//!
//! GTK 4 links a widget's children as a chain (`first_child`, then
//! `next_sibling`), and `GtkBox` has no call that removes them all. The
//! chrome redraws its crumbs, tabs, caption buttons and landing pages by
//! replacing their children, and its layouts measure the children they
//! place, so these helpers keep that walk in one place. The web app does
//! the same with `innerHTML = ''` and `children` in `desktop/ui/app.js`.

use gtk::prelude::*;

/// The children of `widget`, first to last.
pub(super) fn children(widget: &impl IsA<gtk::Widget>) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), WidgetExt::next_sibling)
}

/// The children of `widget` that a layout should place: the visible ones.
pub(super) fn laid_out_children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    children(widget).filter(WidgetExt::should_layout).collect()
}

/// Removes every child of `container`.
pub(super) fn remove_children(container: &impl IsA<gtk::Box>) {
    let container = container.upcast_ref::<gtk::Box>();
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
