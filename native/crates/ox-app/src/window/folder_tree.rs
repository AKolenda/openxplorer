// SPDX-License-Identifier: AGPL-3.0-only
//! The folder tree (SIDE-028): Dolphin's Folders panel (F7), shown in the
//! navigation pane below the places, where Explorer's navigation pane
//! has its tree of folders.
//!
//! View > Folder tree or F7 shows it. It starts at the home folder while
//! the folder shown is inside it ("Limit to home folder", on by default),
//! else at the top of the folder's file system or server. It follows the
//! active tab: the folders on the way open, and the folder shown is
//! selected and scrolled to ("Scroll to the folder shown"). A click
//! opens a folder, its chevron (or Right and Left) opens and closes its
//! subfolders, which are read only then ([`model`]), and a middle-click or
//! Ctrl+click opens it in a tab. Its context menu ([`menu`]) acts on the
//! folder clicked, not on the folder shown.

mod actions;
mod follow;
mod menu;
mod model;

use std::cell::{Cell, OnceCell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::settings::FolderTreeOptions;

use crate::icons::{Art, ArtImage};

use super::gestures;
use super::menu_popover::MenuPopover;
use super::window_action::WindowAction;

/// The size of a folder's icon in the tree.
const FOLDER_ICON_SIZE: i32 = 16;

/// A shown row and the tree row it draws.
#[derive(Debug)]
struct BoundRow {
    item: glib::WeakRef<gtk::ListItem>,
    row: gtk::TreeListRow,
    expanded: glib::SignalHandlerId,
}

mod imp {
    use super::{Cell, FolderTreeOptions, MenuPopover, OnceCell, RefCell};
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};

    /// Private state of [`super::FolderTree`].
    #[derive(Debug, Default)]
    pub(crate) struct FolderTree {
        /// The rows, built by `constructed`.
        pub(super) view: OnceCell<gtk::ListView>,
        /// The selection the rows show, over the current tree.
        pub(super) selection: OnceCell<gtk::SingleSelection>,
        /// The tree's options, as saved.
        pub(super) options: Cell<FolderTreeOptions>,
        /// The folder at the top of the tree.
        pub(super) root: RefCell<Option<gio::File>>,
        /// The folder the active tab shows, if it is a folder.
        pub(super) shown: RefCell<Option<gio::File>>,
        /// Counts the walks to the folder shown; a walk stops once a
        /// newer one starts.
        pub(super) walk: Cell<u64>,
        /// The rows' context menu.
        pub(super) menu: OnceCell<MenuPopover>,
        /// The tree row each shown row draws, with the handler that reads
        /// its subfolders when it is expanded.
        pub(super) bound: RefCell<Vec<super::BoundRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderTree {
        const NAME: &'static str = "OxFolderTree";
        type Type = super::FolderTree;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for FolderTree {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(menu) = self.menu.get() {
                menu.unparent();
            }
        }
    }

    impl WidgetImpl for FolderTree {}
    impl BoxImpl for FolderTree {}
}

glib::wrapper! {
    /// The tree of folders in the navigation pane.
    pub(crate) struct FolderTree(ObjectSubclass<imp::FolderTree>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for FolderTree {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl FolderTree {
    /// Builds the list of rows into the tree.
    fn build(&self) {
        self.set_orientation(gtk::Orientation::Vertical);
        self.add_css_class("folder-tree");
        self.set_visible(false);
        let selection = gtk::SingleSelection::builder()
            .autoselect(false)
            .can_unselect(true)
            .build();
        let view = gtk::ListView::builder()
            .model(&selection)
            .factory(&self.row_factory())
            .build();
        view.update_property(&[gtk::accessible::Property::Label("Folder tree")]);
        view.connect_activate(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            move |_, position| tree.open_row(position, gdk::ModifierType::empty())
        ));
        self.open_and_close_with_arrow_keys(&view);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&view)
            .build();
        self.append(&scroller);
        let menu = MenuPopover::new(Vec::new());
        menu.set_parent(self);
        let imp = self.imp();
        imp.menu.set(menu).expect("constructed runs once");
        imp.selection.set(selection).expect("constructed runs once");
        imp.view.set(view).expect("constructed runs once");
    }

    /// The tree's rows: a chevron, the folder's icon and its name.
    fn row_factory(&self) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let content = gtk::Box::builder().spacing(8).build();
                content.append(&ArtImage::new(Art::Folder, FOLDER_ICON_SIZE));
                content.append(
                    &gtk::Label::builder()
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build(),
                );
                tree.handle_clicks(&content, item);
                let expander = gtk::TreeExpander::builder().child(&content).build();
                item.set_child(Some(&expander));
            }
        ));
        factory.connect_bind(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            move |_, item| {
                if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                    tree.bind_row(item);
                }
            }
        ));
        factory.connect_unbind(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            move |_, item| {
                if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                    tree.unbind_row(item);
                }
            }
        ));
        factory
    }

    /// Shows the tree row of `item` in its widgets, and reads the row's
    /// subfolders when the user expands it.
    fn bind_row(&self, item: &gtk::ListItem) {
        let row = item.item().and_downcast::<gtk::TreeListRow>();
        let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else {
            return;
        };
        let name = row.as_ref().map(model::row_name).unwrap_or_default();
        let label = expander
            .child()
            .and_then(|content| content.last_child())
            .and_downcast::<gtk::Label>();
        if let Some(label) = label {
            label.set_text(&name);
        }
        let path = row
            .as_ref()
            .and_then(model::row_file)
            .map(|file| file.parse_name());
        expander.set_tooltip_text(path.as_deref());
        item.set_accessible_label(&name);
        expander.set_list_row(row.as_ref());
        let Some(row) = row else {
            return;
        };
        let expanded = row.connect_expanded_notify(|row| {
            if row.is_expanded() {
                model::load_children(row);
            }
        });
        self.imp().bound.borrow_mut().push(BoundRow {
            item: item.downgrade(),
            row,
            expanded,
        });
    }

    /// Forgets the tree row `item` showed, and the rows of items that are
    /// gone.
    fn unbind_row(&self, item: &gtk::ListItem) {
        if let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() {
            expander.set_list_row(None);
        }
        let mut bound = self.imp().bound.borrow_mut();
        let (gone, kept): (Vec<BoundRow>, Vec<BoundRow>) = bound
            .drain(..)
            .partition(|shown| shown.item.upgrade().is_none_or(|shown| shown == *item));
        *bound = kept;
        for shown in gone {
            shown.row.disconnect(shown.expanded);
        }
    }

    /// A click on a row's folder opens it, with Ctrl in a tab; a
    /// middle-click opens it in a tab; a right-click opens its menu.
    fn handle_clicks(&self, content: &gtk::Box, item: &gtk::ListItem) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.connect_released(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            #[weak]
            item,
            move |gesture, _, x, y| {
                let position = item.position();
                match gesture.current_button() {
                    gdk::BUTTON_PRIMARY => tree.open_row(position, gestures::held_modifiers(gesture)),
                    gdk::BUTTON_SECONDARY => {
                        if let Some(widget) = gesture.widget() {
                            tree.show_menu(position, &widget, x, y);
                        }
                    }
                    _ => {}
                }
            }
        ));
        content.add_controller(click);
        let middle = gestures::middle_click(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            #[weak]
            item,
            move |gesture, _, _| {
                let modifiers = gesture.current_event_state() | gdk::ModifierType::CONTROL_MASK;
                tree.open_row(item.position(), modifiers);
            }
        ));
        content.add_controller(middle);
    }

    /// Right opens the focused folder's subfolders, or moves to the first
    /// one; Left closes them, or moves to the folder above, as in
    /// Explorer's and Dolphin's trees.
    fn open_and_close_with_arrow_keys(&self, view: &gtk::ListView) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = tree)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if !modifiers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let moved = match key {
                    gdk::Key::Right | gdk::Key::KP_Right => tree.step_in(),
                    gdk::Key::Left | gdk::Key::KP_Left => tree.step_out(),
                    _ => return glib::Propagation::Proceed,
                };
                if moved {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        view.add_controller(keys);
    }

    /// Right on the selected row; false where there is none.
    fn step_in(&self) -> bool {
        let Some((position, row)) = self.selected_row() else {
            return false;
        };
        if row.is_expanded() {
            self.focus_row(position + 1);
        } else {
            row.set_expanded(true);
            model::load_children(&row);
        }
        true
    }

    /// Left on the selected row; false where there is none.
    fn step_out(&self) -> bool {
        let Some((_, row)) = self.selected_row() else {
            return false;
        };
        if row.is_expanded() {
            row.set_expanded(false);
        } else if let Some(parent) = row.parent() {
            self.focus_row(parent.position());
        }
        true
    }

    /// The selected row and its position.
    fn selected_row(&self) -> Option<(u32, gtk::TreeListRow)> {
        let selection = self.imp().selection.get()?;
        let position = selection.selected();
        let row = selection.selected_item().and_downcast::<gtk::TreeListRow>()?;
        Some((position, row))
    }

    /// Selects, focuses and scrolls to the row at `position`.
    fn focus_row(&self, position: u32) {
        let Some(view) = self.imp().view.get() else {
            return;
        };
        if position < view.model().map_or(0, |model| model.n_items()) {
            view.scroll_to(
                position,
                gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
                None,
            );
        }
    }

    /// The row at `position` of the tree.
    fn row(&self, position: u32) -> Option<gtk::TreeListRow> {
        self.tree_model()?.row(position)
    }

    /// The tree's model, once a folder was shown.
    fn tree_model(&self) -> Option<gtk::TreeListModel> {
        self.imp().selection.get()?.model().and_downcast()
    }

    /// The location of the folder at `position`.
    pub(super) fn uri_at(&self, position: u32) -> Option<String> {
        let row = self.row(position)?;
        Some(model::row_file(&row)?.uri().to_string())
    }

    /// Opens the folder at `position`: in this tab, unless `modifiers`
    /// hold Ctrl, which opens it in a tab (in front with Shift too).
    fn open_row(&self, position: u32, modifiers: gdk::ModifierType) {
        let Some(uri) = self.uri_at(position) else {
            return;
        };
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
            gestures::open_action(modifiers).activate_from(self, Some(&uri.to_variant()));
            return;
        }
        let file = gio::File::for_uri(&uri);
        // A double-click activates the row after its first click opened
        // the folder; opening it again would add nothing.
        if self
            .imp()
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|shown| shown.equal(&file))
        {
            return;
        }
        WindowAction::GoTo.activate_from(self, Some(&uri.to_variant()));
    }

    /// Opens the menu of the folder at `position` at (`x`, `y`) of
    /// `widget`.
    fn show_menu(&self, position: u32, widget: &gtk::Widget, x: f64, y: f64) {
        let Some(entries) = self.menu_entries(position) else {
            return;
        };
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = gtk::graphene::Point::new(x as f32, y as f32);
        let Some(point) = widget.compute_point(self, &point) else {
            return;
        };
        let menu = self.imp().menu.get().expect("constructed builds the menu");
        menu.set_entries(entries);
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let target = gdk::Rectangle::new(point.x() as i32, point.y() as i32, 1, 1);
        menu.set_pointing_to(Some(&target));
        menu.popup();
    }

    /// Activates the row at `position`, as Enter does, for tests.
    #[cfg(test)]
    pub(super) fn activate_row(&self, position: u32) {
        let view = self.imp().view.get().expect("constructed builds the view");
        view.emit_by_name::<()>("activate", &[&position]);
    }

    /// The location of the selected folder, for tests.
    #[cfg(test)]
    pub(super) fn selected_uri(&self) -> Option<String> {
        let (position, _) = self.selected_row()?;
        self.uri_at(position)
    }

    /// The names of the rows, indented two spaces a level, for tests.
    #[cfg(test)]
    pub(super) fn outline(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut position = 0;
        while let Some(row) = self.row(position) {
            let indent = "  ".repeat(usize::try_from(row.depth()).unwrap_or_default());
            lines.push(format!("{indent}{}", model::row_name(&row)));
            position += 1;
        }
        lines
    }
}
