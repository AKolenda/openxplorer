// SPDX-License-Identifier: AGPL-3.0-only
//! The check box before the Name title, which selects every item or none
//! (SEL-014), as the one at the start of Explorer's Details header when
//! item check boxes are on.
//!
//! It is checked while every item is selected, mixed while some are, and
//! clear while none is. Clicking it selects every item, or none when
//! every item already is. It shows while the pointer is on the column
//! titles or some item is selected, as the items' own check boxes show
//! while hovered or selected (folder-views.css).
//!
//! The title is a button that sorts the view when clicked, so the check
//! box takes its clicks before the title sees them (`own_clicks`).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::DetailsView;
use crate::folder_view::column_titles;
use crate::folder_view::sorting::SortColumn;

/// How the check box shows a selection of `selected` items out of
/// `total`: (checked, mixed).
fn look_for(selected: u64, total: u32) -> (bool, bool) {
    let all = total > 0 && selected >= u64::from(total);
    (all, selected > 0 && !all)
}

/// The check box's accessible name.
fn check_name() -> &'static str {
    ox_core::i18n::gettext_static("Select all")
}

impl DetailsView {
    /// Puts the check box before the Name title, following `selection`.
    pub(super) fn add_select_all(&self, selection: &gtk::MultiSelection) {
        let Some(title) = column_titles::title_box_of(self.column_view(), SortColumn::Name) else {
            return;
        };
        let check = gtk::CheckButton::new();
        check.add_css_class("item-check");
        check.add_css_class("select-all");
        check.set_focusable(false);
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some(check_name()));
        check.update_property(&[gtk::accessible::Property::Label(check_name())]);
        title.prepend(&check);
        crate::folder_view::cells::own_clicks(&check);
        let handler = check.connect_toggled(glib::clone!(
            #[weak]
            selection,
            move |_| {
                let (all, _) = look_for(selection.selection().size(), selection.n_items());
                if all {
                    selection.unselect_all();
                } else {
                    selection.select_all();
                }
            }
        ));
        let follow = std::rc::Rc::new(glib::clone!(
            #[weak]
            check,
            move |selection: &gtk::MultiSelection| {
                let (all, mixed) = look_for(selection.selection().size(), selection.n_items());
                check.block_signal(&handler);
                check.set_active(all);
                check.set_inconsistent(mixed);
                check.unblock_signal(&handler);
            }
        ));
        follow(selection);
        let on_change = std::rc::Rc::clone(&follow);
        selection.connect_selection_changed(move |selection, _, _| on_change(selection));
        selection.connect_items_changed(move |selection, _, _, _| follow(selection));
        self.imp()
            .select_all
            .set(check)
            .expect("DetailsView::new adds the check box once");
    }

    /// Shows the check box while item check boxes are `shown`.
    pub(crate) fn show_select_all(&self, shown: bool) {
        if let Some(check) = self.imp().select_all.get() {
            check.set_visible(shown);
        }
    }

    /// The check box, for tests.
    #[cfg(test)]
    pub(crate) fn select_all_check(&self) -> Option<gtk::CheckButton> {
        self.imp().select_all.get().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::look_for;

    #[test]
    fn the_box_is_checked_for_all_mixed_for_some_and_clear_for_none() {
        assert_eq!(look_for(0, 0), (false, false), "an empty folder");
        assert_eq!(look_for(0, 5), (false, false));
        assert_eq!(look_for(2, 5), (false, true));
        assert_eq!(look_for(5, 5), (true, false));
    }
}
