// SPDX-License-Identifier: AGPL-3.0-only
//! The details view's column titles, drawn as the current app draws them.
//!
//! Ports the title part of `renderColumns` in `desktop/ui/app.js` and the
//! `.column-head` rules of `desktop/ui/style.css`: the Size title is
//! right-aligned, and only the sorted column shows an arrow, the app's own
//! 10-pixel chevron (`icon('down')`, turned up while ascending) rather
//! than GTK's filled triangle (ui-spec.md §4.5, §7.2).
//!
//! GTK 4.14 builds column titles itself and has no API for their layout.
//! Each title is a button holding a box with the title's label and GTK's
//! sort indicator, in column order; this module works on that box, and
//! warns once if a GTK release changes it.

use gtk::glib;
use gtk::prelude::*;
use ox_core::LOG_DOMAIN;

use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};
use crate::icons::{self, Icon};

/// The arrow's edge (`.column-head svg{width:10px;height:10px}`).
const CARET_SIZE: i32 = 10;

/// The CSS class of the arrow; `resources/skin/folder-views.css` places
/// it and turns it up for [`SortDirection::Ascending`].
const CARET_CLASS: &str = "sort-caret";

/// Aligns the titles and gives each a hidden sort arrow, the current
/// app's, for [`show_sort_caret`] to show. Returns the arrows in column
/// order, or none, after a warning, if GTK built the titles differently.
pub(crate) fn style_titles(view: &gtk::ColumnView) -> Vec<gtk::Image> {
    let titles = title_boxes(view);
    if titles.len() != SortColumn::ALL.len() {
        glib::g_warning!(LOG_DOMAIN, "The column titles have an unexpected structure");
        return Vec::new();
    }
    if let Some(size_title) = titles.last() {
        // `.column:last-child .column-label{justify-content:flex-end}`,
        // with the arrow kept beside the text.
        size_title.set_halign(gtk::Align::End);
    }
    titles.iter().map(append_caret).collect()
}

/// Each column title, in column order.
pub(crate) fn title_buttons(view: &gtk::ColumnView) -> Vec<gtk::Widget> {
    let header = view.first_child().filter(|child| child.css_name() == "header");
    let first_title = header.and_then(|header| header.first_child());
    std::iter::successors(first_title, WidgetExt::next_sibling).collect()
}

/// The box inside each column title, in column order.
fn title_boxes(view: &gtk::ColumnView) -> Vec<gtk::Box> {
    title_buttons(view)
        .into_iter()
        .filter_map(|title| title.first_child().and_downcast::<gtk::Box>())
        .collect()
}

/// Adds a hidden arrow after the title's label and GTK's indicator,
/// which the skin hides.
fn append_caret(title: &gtk::Box) -> gtk::Image {
    let caret = icons::image(Icon::ChevronDown16, CARET_SIZE);
    caret.add_css_class(CARET_CLASS);
    caret.set_visible(false);
    title.append(&caret);
    caret
}

/// Shows the arrow, among the titles' `carets`, of the column the view
/// sorts by, pointing its way, and hides the others (`aria-sort` in
/// `renderColumns`). An unsorted view (`None`) shows no arrow.
pub(crate) fn show_sort_caret(carets: &[gtk::Image], sorted_by: Option<SortOrder>) {
    for (column, caret) in SortColumn::ALL.into_iter().zip(carets) {
        let direction = sorted_by
            .filter(|order| order.column == column)
            .map(|order| order.direction);
        point_caret(caret, direction);
    }
}

/// Shows `caret` pointing `direction`, or hides it for `None`.
fn point_caret(caret: &gtk::Image, direction: Option<SortDirection>) {
    caret.set_visible(direction.is_some());
    for other in SortDirection::ALL {
        caret.remove_css_class(other.css_class());
    }
    if let Some(direction) = direction {
        caret.add_css_class(direction.css_class());
    }
}

/// The direction each shown title's arrow shows, in column order: `None`
/// where no arrow is visible. A hidden column's title is left out.
#[cfg(test)]
pub(crate) fn shown_carets(view: &gtk::ColumnView) -> Vec<Option<SortDirection>> {
    let shown_titles = title_boxes(view).into_iter().filter(|title| {
        let button = title.parent();
        button.is_some_and(|button| button.is_visible())
    });
    let carets = shown_titles.filter_map(|title| {
        title
            .last_child()
            .filter(|child| child.has_css_class(CARET_CLASS))
    });
    carets.map(|caret| shown_direction(&caret)).collect()
}

/// The direction a visible caret points; `None` for a hidden one.
#[cfg(test)]
fn shown_direction(caret: &gtk::Widget) -> Option<SortDirection> {
    if !caret.is_visible() {
        return None;
    }
    SortDirection::ALL
        .into_iter()
        .find(|direction| caret.has_css_class(direction.css_class()))
}
