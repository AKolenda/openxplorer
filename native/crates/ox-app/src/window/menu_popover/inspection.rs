// SPDX-License-Identifier: AGPL-3.0-only
//! What the tests read of a [`MenuPopover`]: its rows, their labels and
//! check marks, the compact style's strip and the style.

use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{MenuPopover, MenuStyle};

impl MenuPopover {
    /// Rests the pointer on the row labelled `label`, or on no row with
    /// `None`, as hovering does, for tests.
    pub(crate) fn hover_row(&self, label: Option<&str>) {
        let index = label.map(|label| self.row(label).index());
        self.rest_on(index);
    }

    /// Presses `key` in the rows' list, as the keyboard does, for tests.
    pub(crate) fn press_in_list(&self, key: gtk::gdk::Key) -> bool {
        let keys = self
            .list()
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .expect("the list takes keys");
        let no_keycode = 0_u32;
        let none = gtk::gdk::ModifierType::empty();
        keys.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &none])
    }

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

    /// Middle-clicks the row labelled `label`, for tests.
    pub(crate) fn middle_click_row(&self, label: &str) {
        let row = self.row(label);
        let list = self.list();
        let point = row.compute_point(list, &gtk::graphene::Point::new(1.0, 1.0));
        let y = point.map_or(0.0, |point| f64::from(point.y()));
        let gesture = list
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
            .find(|gesture| gesture.button() == gtk::gdk::BUTTON_MIDDLE)
            .expect("the menu listens to the middle button");
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &1.0_f64, &y]);
        gesture.emit_by_name::<()>("released", &[&1_i32, &1.0_f64, &y]);
    }

    /// The labels of the rows showing a check mark, for tests.
    pub(crate) fn checked_labels(&self) -> Vec<String> {
        let checked = self.rows().into_iter().filter(|row| row.has_css_class("checked"));
        checked.filter_map(|row| row_label(&row)).collect()
    }

    /// The labels of the strip's buttons while it shows (the first line of
    /// their tooltips; a second says why one is disabled), for tests.
    pub(crate) fn strip_labels(&self) -> Vec<String> {
        if !self.strip().is_visible() {
            return Vec::new();
        }
        crate::window::widget_tree::children(self.strip())
            .filter_map(|child| child.tooltip_text())
            .filter_map(|tooltip| tooltip.lines().next().map(str::to_owned))
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
