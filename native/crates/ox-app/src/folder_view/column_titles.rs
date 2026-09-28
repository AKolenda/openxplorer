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

use crate::folder_view::details;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::icons::{self, Glyph};

/// The arrow's edge (`.column-head svg{width:10px;height:10px}`).
const CARET_SIZE: i32 = 10;

/// The CSS class of the arrow; `resources/skin/folder-views.css` places
/// it and turns it up for [`SortDirection::Ascending`].
const CARET_CLASS: &str = "sort-caret";

/// Aligns the titles and gives them the current app's sort arrow, which
/// follows the view's sorter from now on.
pub(crate) fn style_titles(view: &gtk::ColumnView) {
    let titles = title_boxes(view);
    if titles.len() != SortColumn::ALL.len() {
        glib::g_warning!("openxplorer", "The column titles have an unexpected structure");
        return;
    }
    if let Some(size_title) = titles.last() {
        // `.column:last-child .column-label{justify-content:flex-end}`,
        // with the arrow kept beside the text.
        size_title.set_halign(gtk::Align::End);
    }
    let carets: Vec<gtk::Image> = titles.iter().map(append_caret).collect();
    show_sort_caret(view, &carets);
    let Some(sorter) = view.sorter() else {
        return;
    };
    sorter.connect_changed(glib::clone!(
        #[weak]
        view,
        move |_, _| show_sort_caret(&view, &carets)
    ));
}

/// The box inside each column title, in column order.
fn title_boxes(view: &gtk::ColumnView) -> Vec<gtk::Box> {
    let header = view.first_child().filter(|child| child.css_name() == "header");
    let first_title = header.and_then(|header| header.first_child());
    std::iter::successors(first_title, WidgetExt::next_sibling)
        .filter_map(|title| title.first_child().and_downcast::<gtk::Box>())
        .collect()
}

/// Adds a hidden arrow after the title's label and GTK's indicator,
/// which the skin hides.
fn append_caret(title: &gtk::Box) -> gtk::Image {
    let caret = icons::glyph(Glyph::Down, CARET_SIZE);
    caret.add_css_class(CARET_CLASS);
    caret.set_visible(false);
    title.append(&caret);
    caret
}

/// Shows the arrow of the column the view sorts by, pointing its way,
/// and hides the others (`aria-sort` in `renderColumns`).
fn show_sort_caret(view: &gtk::ColumnView, carets: &[gtk::Image]) {
    let primary_sort = details::primary_sort(view);
    for (column, caret) in SortColumn::ALL.into_iter().zip(carets) {
        let direction = primary_sort
            .filter(|(sorted_column, _)| *sorted_column == column)
            .map(|(_, direction)| direction);
        point_caret(caret, direction);
    }
}

/// Shows `caret` pointing `direction`, or hides it for `None`.
fn point_caret(caret: &gtk::Image, direction: Option<SortDirection>) {
    caret.set_visible(direction.is_some());
    for other in SortDirection::ALL {
        caret.remove_css_class(other.key());
    }
    if let Some(direction) = direction {
        caret.add_css_class(direction.key());
    }
}

/// The direction each title's arrow shows, in column order: `None` where
/// no arrow is visible.
#[cfg(test)]
pub(crate) fn shown_carets(view: &gtk::ColumnView) -> Vec<Option<SortDirection>> {
    let carets = title_boxes(view).into_iter().filter_map(|title| {
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
        .find(|direction| caret.has_css_class(direction.key()))
}
