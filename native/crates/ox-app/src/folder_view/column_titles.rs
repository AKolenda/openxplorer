// SPDX-License-Identifier: AGPL-3.0-only
//! The details view's column titles, drawn as Windows 11 File Explorer
//! draws them.
//!
//! Every title is left-aligned, Size's too, while the sizes under it stay
//! right-aligned. Only the sorted column shows an arrow: a chevron right
//! after its title, up while ascending and down while descending, rather
//! than GTK's filled triangle. Sorted any other way than
//! [`SortOrder::DEFAULT`], the title turns the accent with a line under it
//! (ui-spec.md §4.5, §7.2). `resources/skin/folder-views.css` draws both
//! from the classes this module gives the sorted title.
//!
//! GTK 4.14 builds column titles itself and has no API for their layout.
//! Each title is a button holding a box with the title's label and GTK's
//! sort indicator, in column order; this module works on those buttons,
//! and warns once if a GTK release changes them.

use gtk::glib;
use gtk::prelude::*;
use ox_core::LOG_DOMAIN;

use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};

/// The class of Name's title, which keeps Name's minimum width wherever
/// Choose Details moves it.
const NAME_TITLE_CLASS: &str = "name-title";

/// The class of the sorted title while the view sorts any other way than
/// [`SortOrder::DEFAULT`], which draws it in the accent.
const CUSTOM_SORT_CLASS: &str = "custom-sort";

/// Each column's title, found by its column. They are read once, while
/// the titles stand in the order the columns were added in
/// ([`SortColumn::ALL`]): GTK keeps a column's title as the column moves,
/// but after a column moves to the right the header's children no longer
/// follow the columns' order (GTK 4.14), so a title is never found by its
/// place.
#[derive(Debug, Default)]
pub(crate) struct ColumnTitles(Vec<(SortColumn, gtk::Widget)>);

impl ColumnTitles {
    /// The titles of `view`, whose columns must still be in the order of
    /// [`SortColumn::ALL`], with Name's marked by its own class; none,
    /// after a warning, if GTK built the titles differently.
    pub(crate) fn new(view: &gtk::ColumnView) -> Self {
        let titles = title_buttons(view);
        let boxes = titles
            .iter()
            .filter(|title| title.first_child().and_downcast::<gtk::Box>().is_some())
            .count();
        if titles.len() != SortColumn::ALL.len() || boxes != titles.len() {
            glib::g_warning!(LOG_DOMAIN, "The column titles have an unexpected structure");
            return Self::default();
        }
        let titles: Vec<_> = SortColumn::ALL.into_iter().zip(titles).collect();
        for (column, title) in &titles {
            if *column == SortColumn::Name {
                title.add_css_class(NAME_TITLE_CLASS);
            }
        }
        Self(titles)
    }

    /// The title of `column`.
    pub(crate) fn title(&self, column: SortColumn) -> Option<&gtk::Widget> {
        self.0
            .iter()
            .find(|(titled, _)| *titled == column)
            .map(|(_, title)| title)
    }

    /// The box inside the title of `column`, which holds its label.
    pub(crate) fn title_box(&self, column: SortColumn) -> Option<gtk::Box> {
        self.title(column)?.first_child().and_downcast::<gtk::Box>()
    }

    /// Marks the title of the column the view sorts by with the way it
    /// sorts, which shows its chevron, and as a custom sort unless that is
    /// [`SortOrder::DEFAULT`], and clears the others (`aria-sort` in
    /// `renderColumns`). An unsorted view (`None`) shows no chevron. The
    /// sorted title's label ends in an ellipsis when it is too long for
    /// its column, so the chevron after it keeps its room, as in File
    /// extension's 90-pixel default; the others overflow as before.
    pub(crate) fn show_sort(&self, sorted_by: Option<SortOrder>) {
        for (column, title) in &self.0 {
            let order = sorted_by.filter(|order| order.column == *column);
            point_caret(title, order.map(|order| order.direction));
            if let Some(label) = self.title_box(*column).and_then(|title_box| label_in(&title_box)) {
                label.set_ellipsize(if order.is_some() {
                    gtk::pango::EllipsizeMode::End
                } else {
                    gtk::pango::EllipsizeMode::None
                });
            }
            if order.is_some_and(|order| order != SortOrder::DEFAULT) {
                title.add_css_class(CUSTOM_SORT_CLASS);
            } else {
                title.remove_css_class(CUSTOM_SORT_CLASS);
            }
        }
    }
}

/// The label in a title's box, after Name's Select all box.
fn label_in(title_box: &gtk::Box) -> Option<gtk::Label> {
    std::iter::successors(title_box.first_child(), WidgetExt::next_sibling)
        .find_map(|child| child.downcast::<gtk::Label>().ok())
}

/// Each column title, in the header's order.
pub(crate) fn title_buttons(view: &gtk::ColumnView) -> Vec<gtk::Widget> {
    let header = view.first_child().filter(|child| child.css_name() == "header");
    let first_title = header.and_then(|header| header.first_child());
    std::iter::successors(first_title, WidgetExt::next_sibling).collect()
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

/// Whether `title` is drawn as a sort other than the default.
#[cfg(test)]
pub(crate) fn is_custom_sort(title: &gtk::Widget) -> bool {
    title.has_css_class(CUSTOM_SORT_CLASS)
}

/// The direction `title`'s chevron shows, or `None` without one.
#[cfg(test)]
pub(crate) fn caret_of(title: &gtk::Widget) -> Option<SortDirection> {
    SortDirection::ALL
        .into_iter()
        .find(|direction| title.has_css_class(direction.css_class()))
}
