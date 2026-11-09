// SPDX-License-Identifier: AGPL-3.0-only
//! What the tests read of a [`MenuPopover`]: its rows, their labels and
//! check marks, why a row is disabled, the compact style's strip and the
//! style.

use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{MenuPopover, MenuStyle};

impl MenuPopover {
    /// Clicks the row labelled `label`, which opens its submenu beside
    /// the menu, and returns that submenu, for tests.
    pub(crate) fn open_submenu_of(&self, label: &str) -> MenuPopover {
        self.row(label).emit_activate();
        let submenu = self
            .open_submenu_menu()
            .unwrap_or_else(|| panic!("{label} opens a submenu"));
        crate::test_support::harness::wait_until("the submenu", || submenu.is_mapped());
        submenu
    }

    /// Rests the pointer on the row labelled `label`, or on no row with
    /// `None`, as moving the pointer there does, for tests.
    pub(crate) fn hover_row(&self, label: Option<&str>) {
        let index = label.map(|label| usize::try_from(self.row(label).index()).expect("a row"));
        self.rest_on(index);
    }

    /// The pointer reaches this submenu from the menu it opens from, for
    /// tests.
    pub(crate) fn pointer_enters(&self) {
        if let Some(parent) = self.parent_menu() {
            parent.keep_submenu_open();
        }
    }

    /// Whether this menu is a submenu, and the menu it opens from, for
    /// tests.
    pub(crate) fn opens_from(&self) -> Option<MenuPopover> {
        self.parent_menu()
    }

    /// The labels of the menu's items, drawn or not, for tests.
    pub(crate) fn items_labels(&self) -> Vec<String> {
        self.items().into_iter().map(|item| item.label).collect()
    }

    /// The labels of the items shown in bold, for tests.
    pub(crate) fn emphasised_labels(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .filter(|item| item.emphasised)
            .map(|item| item.label)
            .collect()
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

    /// Why the row labelled `label` is disabled, which its accessible
    /// description tells screen readers, for tests. GTK cannot read an
    /// accessible description back, so this asks the menu for the reason
    /// it gave the row, after checking that the row has a description.
    pub(crate) fn disabled_reason_of(&self, label: &str) -> Option<String> {
        let row = self.row(label);
        if row.is_sensitive()
            || !gtk::test_accessible_has_property(&row, gtk::AccessibleProperty::Description)
        {
            return None;
        }
        let item = self.items().into_iter().find(|item| item.label == label)?;
        super::disabled_reason(self, &item)
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

    /// Clicks the row labelled `label` with the primary button, through
    /// the list's own click handling, for tests.
    pub(crate) fn click_row(&self, label: &str) {
        let row = self.row(label);
        let list = self.list();
        // Rows just put in place are laid out on the next frame.
        crate::test_support::harness::wait_until("the row laid out", || {
            let top = row.compute_point(list, &gtk::graphene::Point::new(0.0, 1.0));
            #[expect(clippy::cast_possible_truncation, reason = "menu rows are small")]
            let at = top.and_then(|top| list.row_at_y(top.y() as i32));
            row.height() > 0 && at.as_ref() == Some(&row)
        });
        let point = row.compute_point(list, &gtk::graphene::Point::new(4.0, 4.0));
        let (x, y) = point.map_or((0.0, 0.0), |point| (f64::from(point.x()), f64::from(point.y())));
        let gesture = list
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
            .find(|gesture| gesture.button() == gtk::gdk::BUTTON_PRIMARY)
            .expect("the list takes clicks");
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
        gesture.emit_by_name::<()>("released", &[&1_i32, &x, &y]);
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

    /// The text drawn under each strip button's icon, for tests.
    pub(crate) fn strip_texts(&self) -> Vec<String> {
        crate::window::widget_tree::children(self.strip())
            .filter_map(|button| button.downcast::<gtk::Button>().ok())
            .filter_map(|button| button.child())
            .filter_map(|face| face.last_child().and_downcast::<gtk::Label>())
            .map(|label| label.text().to_string())
            .collect()
    }

    /// The strip's buttons while it shows, each with whether it can be
    /// pressed, for tests.
    pub(crate) fn strip_states(&self) -> Vec<(String, bool)> {
        if !self.strip().is_visible() {
            return Vec::new();
        }
        crate::window::widget_tree::children(self.strip())
            .filter_map(|child| {
                let tooltip = child.tooltip_text()?;
                let label = tooltip.lines().next()?.to_owned();
                Some((label, child.is_sensitive()))
            })
            .collect()
    }

    /// The classic or compact look, for tests.
    pub(in crate::window) fn style(&self) -> MenuStyle {
        self.imp().style.get()
    }
}

/// The label of `row`, for tests: the first label after its mark and
/// glyph.
pub(crate) fn row_label(row: &gtk::ListBoxRow) -> Option<String> {
    let content = row.child()?;
    let label = crate::window::widget_tree::children(&content)
        .find_map(|child| child.downcast::<gtk::Label>().ok())?;
    Some(label.text().to_string())
}
