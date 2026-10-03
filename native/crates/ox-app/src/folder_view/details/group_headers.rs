// SPDX-License-Identifier: AGPL-3.0-only
//! The title above each group of a grouped listing (VIEW-022).
//!
//! Windows Explorer heads each group with its name, its item count and a
//! line to the right edge ("Today (3) ———"); Dolphin draws the same. GTK
//! 4.12 gives a column view a header per section of its model, which the
//! folder model makes one per group.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::DetailsView;
use crate::folder_view::item::FileItem;
use crate::folder_view::model::FolderModel;

/// Names the group an item is in, `None` while the items are not grouped.
pub(crate) type GroupTitle = Rc<dyn Fn(&FileItem) -> Option<String>>;

/// "Today (3)": a group's title and how many items it holds.
fn header_text(title: &str, count: u32) -> String {
    ox_core::i18n::format_message(
        "{title} ({count})",
        &[("title", title), ("count", &count.to_string())],
    )
}

/// Headers showing `title` of their group's first item and its count, with
/// a line to the edge.
fn header_factory(title: GroupTitle) -> gtk::SignalListItemFactory {
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
    factory.connect_bind(move |_, object| {
        let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let item = header.item().and_downcast::<FileItem>();
        let label = header
            .child()
            .and_then(|row| row.first_child())
            .and_downcast::<gtk::Label>();
        if let (Some(item), Some(label)) = (item, label) {
            let text = title(&item).unwrap_or_default();
            label.set_text(&header_text(&text, header.n_items()));
        }
    });
    factory
}

impl DetailsView {
    /// Heads each group of the listing with its title, or shows no headers
    /// with `None`.
    pub(crate) fn show_group_headers(&self, title: Option<GroupTitle>) {
        let factory = title.map(header_factory);
        self.column_view().set_header_factory(factory.as_ref());
    }

    /// Whether the groups are headed.
    pub(crate) fn shows_group_headers(&self) -> bool {
        self.column_view().header_factory().is_some()
    }

    /// Notes that the window restores a scroll position now, which a list
    /// shown from its top must not override.
    pub(crate) fn note_scroll_restore(&self) {
        let restores = &self.imp().scroll_restores;
        restores.set(restores.get().wrapping_add(1));
    }

    /// Shows a grouped list filled from empty (a folder's first items)
    /// from its top. GTK keeps the first row at the top edge, which leaves
    /// the first group's header above it, scrolled out of sight. A scroll
    /// position the window restores (Back, a tab switch) still wins, even
    /// one restored before this runs.
    pub(super) fn start_grouped_lists_at_the_top(&self, model: &FolderModel) {
        model.selection().connect_items_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |list, position, removed, added| {
                let filled = position == 0 && removed == 0 && added > 0 && list.n_items() == added;
                if !filled || !view.shows_group_headers() {
                    return;
                }
                let adjustment = view.vadjustment();
                let restores = view.imp().scroll_restores.get();
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    view,
                    move || {
                        let restored = view.imp().scroll_restores.get() != restores;
                        if !restored && adjustment.value() > 0.0 {
                            adjustment.set_value(0.0);
                        }
                    }
                ));
            }
        ));
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
