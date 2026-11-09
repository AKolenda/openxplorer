// SPDX-License-Identifier: AGPL-3.0-only
//! The details view's column titles, drawn as Windows 11 File Explorer
//! draws them.
//!
//! Every title is left-aligned, Size's too, while the sizes under it stay
//! right-aligned. Only the sorted column shows an arrow: a small chevron
//! centred on the top edge of its title, up while ascending and down while
//! descending, rather than GTK's filled triangle after the label
//! (ui-spec.md §4.5, §7.2). `resources/skin/folder-views.css` draws the
//! chevron as the title's background, from the class this module gives
//! the sorted title.
//!
//! GTK 4.14 builds column titles itself and has no API for their layout.
//! Each title is a button holding a box with the title's label and GTK's
//! sort indicator, in column order; this module works on those buttons,
//! and warns once if a GTK release changes them.

use gtk::glib;
use gtk::prelude::*;
use ox_core::LOG_DOMAIN;

use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};

/// The titles, in column order, for [`show_sort_caret`] to mark, or none,
/// after a warning, if GTK built the titles differently.
pub(crate) fn style_titles(view: &gtk::ColumnView) -> Vec<gtk::Widget> {
    let titles = title_buttons(view);
    if titles.len() != SortColumn::ALL.len() || title_boxes(view).len() != titles.len() {
        glib::g_warning!(LOG_DOMAIN, "The column titles have an unexpected structure");
        return Vec::new();
    }
    titles
}

/// Each column title, in column order.
pub(crate) fn title_buttons(view: &gtk::ColumnView) -> Vec<gtk::Widget> {
    let header = view.first_child().filter(|child| child.css_name() == "header");
    let first_title = header.and_then(|header| header.first_child());
    std::iter::successors(first_title, WidgetExt::next_sibling).collect()
}

/// The box inside the title of `column`, while the titles are still in
/// the order the view was built with.
pub(crate) fn title_box_of(view: &gtk::ColumnView, column: SortColumn) -> Option<gtk::Box> {
    let index = SortColumn::ALL.iter().position(|shown| *shown == column)?;
    title_boxes(view).into_iter().nth(index)
}

/// The box inside each column title, in column order.
fn title_boxes(view: &gtk::ColumnView) -> Vec<gtk::Box> {
    title_buttons(view)
        .into_iter()
        .filter_map(|title| title.first_child().and_downcast::<gtk::Box>())
        .collect()
}

/// Marks the title, among `titles`, of the column the view sorts by
/// with the way it sorts, which shows its chevron, and clears the others
/// (`aria-sort` in `renderColumns`). An unsorted view (`None`) shows no
/// chevron.
pub(crate) fn show_sort_caret(titles: &[gtk::Widget], sorted_by: Option<SortOrder>) {
    for (column, title) in SortColumn::ALL.into_iter().zip(titles) {
        let direction = sorted_by
            .filter(|order| order.column == column)
            .map(|order| order.direction);
        point_caret(title, direction);
    }
}

/// Gives `title` the class of `direction`, or none for `None`.
fn point_caret(title: &gtk::Widget, direction: Option<SortDirection>) {
    for other in SortDirection::ALL {
        if Some(other) != direction {
            title.remove_css_class(other.css_class());
        }
    }
    if let Some(direction) = direction {
        title.add_css_class(direction.css_class());
    }
}

/// The direction each shown title's chevron shows, in column order:
/// `None` where no chevron is drawn. A hidden column's title is left out.
#[cfg(test)]
pub(crate) fn shown_carets(view: &gtk::ColumnView) -> Vec<Option<SortDirection>> {
    let shown_titles = title_buttons(view).into_iter().filter(WidgetExt::is_visible);
    shown_titles
        .map(|title| {
            SortDirection::ALL
                .into_iter()
                .find(|direction| title.has_css_class(direction.css_class()))
        })
        .collect()
}
