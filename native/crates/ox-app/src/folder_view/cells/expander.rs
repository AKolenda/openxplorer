// SPDX-License-Identifier: AGPL-3.0-only
//! The arrow before a folder's name in the details view, which expands the
//! folder in place (VIEW-035), and the indent of what it lists.
//!
//! Dolphin draws a chevron before every expandable folder and indents the
//! expanded contents one step per level; the rows of files keep the arrow's
//! room, so names stay aligned. The arrow shows only while folders may
//! expand, so a search or the icon views look as before.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{as_list_item, FileCell};
use crate::folder_view::tree::FolderTree;
use crate::icons::{self, Icon};

/// How far each level of an expanded folder is indented, in pixels.
const INDENT: i32 = 16;

/// The arrow's glyph edge (the bundled 16-pixel chevrons).
const ARROW_GLYPH: i32 = 16;

/// The arrow of a folder that is `expanded`, and what it does.
fn arrow_look(expanded: bool) -> (Icon, &'static str) {
    if expanded {
        (Icon::ChevronDown16, "Collapse")
    } else {
        (Icon::ChevronRight16, "Expand")
    }
}

impl FileCell {
    /// Puts the arrow before the icon; clicking it expands or collapses the
    /// row in `tree` that `list_item` shows.
    fn add_expander(&self, list_item: &gtk::ListItem, tree: &FolderTree) {
        let arrow = &self.imp().expander;
        arrow.add_css_class("folder-expander");
        arrow.set_focusable(false);
        arrow.set_valign(gtk::Align::Center);
        self.prepend(arrow);
        arrow.connect_clicked(glib::clone!(
            #[weak]
            list_item,
            #[strong]
            tree,
            move |_| {
                if let Some(row) = tree.row(list_item.position()) {
                    tree.set_expanded(&row, !row.is_expanded());
                }
            }
        ));
    }

    /// Shows the arrow and indent of `row`, following it as it expands and
    /// collapses; without folders that expand, no arrow.
    fn show_tree_row(&self, row: Option<&gtk::TreeListRow>, tree: &FolderTree) {
        let imp = self.imp();
        if let Some((shown, handler)) = imp.tree_row.take() {
            shown.disconnect(handler);
        }
        let arrow = &imp.expander;
        arrow.set_visible(tree.is_expandable());
        let Some(row) = row.filter(|_| tree.is_expandable()) else {
            return;
        };
        let depth = i32::try_from(row.depth()).unwrap_or(0);
        arrow.set_margin_start(depth * INDENT);
        let expandable = row.is_expandable();
        // Files keep the arrow's room, so names stay aligned.
        arrow.set_opacity(if expandable { 1.0 } else { 0.0 });
        arrow.set_can_target(expandable);
        self.show_arrow(row.is_expanded());
        let handler = row.connect_expanded_notify(glib::clone!(
            #[weak(rename_to = cell)]
            self,
            move |row| cell.show_arrow(row.is_expanded())
        ));
        imp.tree_row.replace(Some((row.clone(), handler)));
    }

    /// Points the arrow down for an expanded folder, right otherwise.
    fn show_arrow(&self, expanded: bool) {
        let arrow = &self.imp().expander;
        let (glyph, name) = arrow_look(expanded);
        arrow.set_child(Some(&icons::image(glyph, ARROW_GLYPH)));
        arrow.set_tooltip_text(Some(name));
        arrow.update_property(&[gtk::accessible::Property::Label(name)]);
    }

    /// The arrow, for tests.
    #[cfg(test)]
    pub(crate) fn expander(&self) -> gtk::Button {
        self.imp().expander.clone()
    }
}

/// Gives the details rows `factory` builds the arrow and indent of their
/// row in `tree`; [`super::connect_file_cells`] must have connected the
/// factory first.
pub(crate) fn connect_expanders(factory: &gtk::SignalListItemFactory, tree: &FolderTree) {
    let setup_tree = tree.clone();
    factory.connect_setup(move |_, object| {
        let list_item = as_list_item(object);
        if let Some(cell) = list_item.child().and_downcast::<FileCell>() {
            cell.add_expander(list_item, &setup_tree);
        }
    });
    let bind_tree = tree.clone();
    factory.connect_bind(move |_, object| {
        let list_item = as_list_item(object);
        if let Some(cell) = list_item.child().and_downcast::<FileCell>() {
            let row = bind_tree.row(list_item.position());
            cell.show_tree_row(row.as_ref(), &bind_tree);
        }
    });
    let unbind_tree = tree.clone();
    factory.connect_unbind(move |_, object| {
        let list_item = as_list_item(object);
        if let Some(cell) = list_item.child().and_downcast::<FileCell>() {
            cell.show_tree_row(None, &unbind_tree);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrow_points_the_way_the_folder_goes() {
        assert_eq!(arrow_look(false).1, "Expand");
        assert_eq!(arrow_look(true).1, "Collapse");
    }
}
