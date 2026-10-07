// SPDX-License-Identifier: AGPL-3.0-only
//! The title above each group of a grouped listing (VIEW-022).
//!
//! Windows Explorer heads each group with its name, its item count and a
//! line to the right edge ("Today (3) ———"); Dolphin draws the same. GTK
//! 4.12 gives a column view a header per section of its model, which the
//! folder model makes one per group.

use std::collections::HashMap;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::DetailsView;
use crate::folder_view::item::FileItem;
use crate::folder_view::model::FolderModel;

/// Names the group an item is in, `None` while the items are not grouped.
pub(crate) type GroupTitle = Rc<dyn Fn(&FileItem) -> Option<String>>;

/// The group titles a view's headers show, kept for recounts; `None`
/// while the groups are not headed.
#[derive(Default)]
pub(crate) struct HeaderTitle(Option<GroupTitle>);

impl std::fmt::Debug for HeaderTitle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("HeaderTitle")
            .field(&self.0.as_ref().map(|_| "titles"))
            .finish()
    }
}

/// "Today (3)": a group's title and how many items it holds.
fn header_text(title: &str, count: u32) -> String {
    ox_core::i18n::format_message(
        "{title} ({count})",
        &[("title", title), ("count", &count.to_string())],
    )
}

/// Headers showing `title` of their group's first item and its count, with
/// a line to the edge. The count comes from the model, not from GTK's
/// `GtkListHeader:n-items`: GTK 4.14 keeps a header bound while its
/// section grows or shrinks and does not always move its end, so
/// "Today (2)" stayed after a third file came. The view recounts the
/// headers on screen whenever the list changes ([`DetailsView::follow_group_counts`]).
fn header_factory(view: &DetailsView, title: GroupTitle) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let label = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["group-title"])
            .build();
        let line = gtk::Separator::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        let row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["group-header"])
            .build();
        row.append(&label);
        row.append(&line);
        header.set_child(Some(&row));
    });
    let shown_title = Rc::clone(&title);
    factory.connect_bind(glib::clone!(
        #[weak]
        view,
        move |_, object| {
            let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
                return;
            };
            view.imp().headers.borrow_mut().push(header.downgrade());
            let counts = view.group_counts(&shown_title);
            show_header_text(header, &shown_title, &counts);
        }
    ));
    factory.connect_unbind(glib::clone!(
        #[weak]
        view,
        move |_, object| {
            let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
                return;
            };
            view.imp()
                .headers
                .borrow_mut()
                .retain(|shown| shown.upgrade().is_some_and(|shown| shown != *header));
        }
    ));
    // The title closure is kept for recounts.
    view.imp().header_title.replace(HeaderTitle(Some(title)));
    factory
}

/// Writes "Today (3)" into `header`: its first item's group and that
/// group's count in `counts`.
fn show_header_text(header: &gtk::ListHeader, title: &GroupTitle, counts: &HashMap<String, u32>) {
    let item = header.item().and_downcast::<FileItem>();
    let label = header
        .child()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Label>();
    if let (Some(item), Some(label)) = (item, label) {
        let text = title(&item).unwrap_or_default();
        let count = counts.get(&text).copied().unwrap_or_else(|| header.n_items());
        label.set_text(&header_text(&text, count));
    }
}

impl DetailsView {
    /// Heads each group of the listing with its title, or shows no headers
    /// with `None`.
    pub(crate) fn show_group_headers(&self, title: Option<GroupTitle>) {
        if title.is_none() {
            self.imp().header_title.replace(HeaderTitle(None));
        }
        let factory = title.map(|title| header_factory(self, title));
        self.column_view().set_header_factory(factory.as_ref());
    }

    /// How many items each group of the list holds, by title, read from
    /// the model's sections.
    fn group_counts(&self, title: &GroupTitle) -> HashMap<String, u32> {
        let mut counts = HashMap::new();
        let Some(model) = self.column_view().model() else {
            return counts;
        };
        let Some(sections) = model.dynamic_cast_ref::<gtk::SectionModel>() else {
            return counts;
        };
        let total = model.n_items();
        let mut position = 0;
        while position < total {
            let (start, end) = sections.section(position);
            let item = model.item(start).and_downcast::<FileItem>();
            if let Some(name) = item.as_ref().and_then(|item| title(item)) {
                *counts.entry(name).or_insert(0) += end.saturating_sub(start);
            }
            position = end.max(position + 1);
        }
        counts
    }

    /// Recounts the group headers on screen whenever the list changes,
    /// once per change on the main loop: files added or removed, a refresh
    /// (F5) or another sort.
    pub(super) fn follow_group_counts(&self, model: &FolderModel) {
        model.selection().connect_items_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, _, _, _| {
                if view.imp().recount_pending.replace(true) {
                    return;
                }
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    view,
                    move || {
                        view.imp().recount_pending.set(false);
                        view.recount_group_headers();
                    }
                ));
            }
        ));
    }

    /// Writes every header on screen again with its group's count now.
    fn recount_group_headers(&self) {
        let Some(title) = self.imp().header_title.borrow().0.clone() else {
            return;
        };
        let counts = self.group_counts(&title);
        let headers: Vec<gtk::ListHeader> = self
            .imp()
            .headers
            .borrow()
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .collect();
        for header in headers {
            show_header_text(&header, &title, &counts);
        }
    }

    /// Runs `change`, which shows, hides or moves columns, with the group
    /// headers taken off and put back after it. A header holds a cell of
    /// every column: changing the columns under it left GTK finalizing
    /// headers whose cells were still in them, and GTK 4.22 then crashed
    /// in `gtk_widget_unparent` the next time a column was shown or hidden
    /// (Downloads grouped, the Recycle Bin, then Back).
    pub(super) fn change_columns_without_headers(&self, change: impl FnOnce()) {
        let view = self.column_view();
        let headers = view.header_factory();
        if headers.is_some() {
            view.set_header_factory(None::<&gtk::ListItemFactory>);
        }
        change();
        if let Some(headers) = headers {
            view.set_header_factory(Some(&headers));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_counts_its_items() {
        assert_eq!(header_text("Today", 3), "Today (3)");
    }
}
