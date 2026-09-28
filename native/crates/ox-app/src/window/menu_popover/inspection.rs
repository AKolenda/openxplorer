// SPDX-License-Identifier: AGPL-3.0-only
//! What the tests read of a [`MenuPopover`]: its rows, their labels and
//! check marks, the compact style's strip and the style.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{MenuPopover, MenuStyle};

impl MenuPopover {
    /// The labels of the rows, a divider as `-`, for tests.
    pub(crate) fn row_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        for row in self.rows() {
            if row.header().is_some() {
                labels.push("-".to_owned());
            }
            labels.extend(row_label(&row));
        }
        labels
    }

    /// The rows, for tests.
    pub(crate) fn rows(&self) -> Vec<gtk::ListBoxRow> {
        crate::window::widget_tree::children(self.list())
            .filter_map(|child| child.downcast::<gtk::ListBoxRow>().ok())
            .collect()
    }

    /// The row labelled `label`, for tests.
    pub(crate) fn row(&self, label: &str) -> gtk::ListBoxRow {
        self.rows()
            .into_iter()
            .find(|row| row_label(row).as_deref() == Some(label))
            .unwrap_or_else(|| panic!("the menu has a {label} row"))
    }

    /// The labels of the rows showing a check mark, for tests.
    pub(crate) fn checked_labels(&self) -> Vec<String> {
        let checked = self.rows().into_iter().filter(|row| row.has_css_class("checked"));
        checked.filter_map(|row| row_label(&row)).collect()
    }

    /// The accessible names of the strip's buttons while it shows, for
    /// tests.
    pub(crate) fn strip_labels(&self) -> Vec<String> {
        if !self.strip().is_visible() {
            return Vec::new();
        }
        crate::window::widget_tree::children(self.strip())
            .filter_map(|child| child.tooltip_text())
            .map(String::from)
            .collect()
    }

    /// The classic or compact look, for tests.
    pub(in crate::window) fn style(&self) -> MenuStyle {
        self.imp().style.get()
    }
}

/// The label of `row`, for tests.
fn row_label(row: &gtk::ListBoxRow) -> Option<String> {
    let content = row.child()?;
    let glyph = content.first_child()?;
    let label = glyph.next_sibling().and_downcast::<gtk::Label>()?;
    Some(label.text().to_string())
}
