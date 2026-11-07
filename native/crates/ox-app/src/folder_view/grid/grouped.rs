// SPDX-License-Identifier: AGPL-3.0-only
//! The grid model of grouped Tiles and Content (VIEW-022, VIEW-071): the
//! folder's selection model with a heading row before each group and
//! blanks after it, laid out by [`SlotLayout`].
//!
//! The selection stays the folder's own: [`GroupedSelection`] passes what
//! the grid selects on to it, slot by slot, and tells the grid what it
//! selects, so Details, the status bar and the window's commands see one
//! selection. Headings, their lines and blanks are [`GroupSlot`]s, which
//! cannot be selected.

use std::collections::HashMap;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::details::{header_text, GroupTitle};
use crate::folder_view::group_slots::{group_runs, ItemsChange, Slot, SlotLayout};
use crate::folder_view::item::FileItem;

/// What a [`GroupSlot`] draws.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SlotKind {
    /// A group's title and count, and the start of its line.
    #[default]
    Heading,
    /// The rest of a heading's line, across the row.
    Rule,
    /// Nothing.
    Blank,
}

#[expect(
    unreachable_pub,
    reason = "the glib::Properties derive always makes the text property's accessors pub"
)]
mod slot_imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::SlotKind;

    /// Private state of [`super::GroupSlot`].
    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::GroupSlot)]
    pub(crate) struct GroupSlot {
        pub(super) kind: Cell<SlotKind>,
        /// A heading's "Today (3)"; empty for the others.
        #[property(get, set)]
        pub(super) text: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GroupSlot {
        const NAME: &'static str = "OxGroupSlot";
        type Type = super::GroupSlot;
    }

    #[glib::derived_properties]
    impl ObjectImpl for GroupSlot {}
}

glib::wrapper! {
    /// A slot of the grid that shows no item: a heading, its line or a
    /// blank.
    pub(crate) struct GroupSlot(ObjectSubclass<slot_imp::GroupSlot>);
}

impl GroupSlot {
    fn new(kind: SlotKind) -> Self {
        let slot: Self = glib::Object::new();
        slot.imp().kind.set(kind);
        slot
    }

    /// What the slot draws.
    pub(crate) fn kind(&self) -> SlotKind {
        self.imp().kind.get()
    }
}

/// The slot objects of one group. GTK warns of an object listed twice,
/// so every slot has its own; they are kept while the group's title is,
/// so a change of the listing rebinds only the slots it moved.
#[derive(Debug)]
struct GroupObjects {
    heading: GroupSlot,
    rules: Vec<GroupSlot>,
    blanks: Vec<GroupSlot>,
}

impl GroupObjects {
    fn new() -> Self {
        Self {
            heading: GroupSlot::new(SlotKind::Heading),
            rules: Vec::new(),
            blanks: Vec::new(),
        }
    }

    /// Keeps `count` objects of `kind` in `slots`.
    fn fit(slots: &mut Vec<GroupSlot>, count: usize, kind: SlotKind) {
        slots.truncate(count);
        while slots.len() < count {
            slots.push(GroupSlot::new(kind));
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};

    use super::GroupObjects;
    use crate::folder_view::details::GroupTitle;
    use crate::folder_view::group_slots::SlotLayout;

    /// Private state of [`super::GroupedSelection`].
    #[derive(Default)]
    pub(crate) struct GroupedSelection {
        /// The folder's selection model.
        pub(super) items: OnceCell<gtk::MultiSelection>,
        /// Names each item's group.
        pub(super) title: OnceCell<GroupTitle>,
        pub(super) layout: RefCell<Option<SlotLayout>>,
        pub(super) objects: RefCell<Vec<GroupObjects>>,
        /// The item a plain or Ctrl click or move last selected, which
        /// Shift extends from.
        pub(super) anchor: Cell<Option<u32>>,
        pub(super) handlers: RefCell<Vec<glib::SignalHandlerId>>,
    }

    impl std::fmt::Debug for GroupedSelection {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("GroupedSelection")
                .field("layout", &self.layout)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GroupedSelection {
        const NAME: &'static str = "OxGroupedSelection";
        type Type = super::GroupedSelection;
        type Interfaces = (gio::ListModel, gtk::SelectionModel);
    }

    impl ObjectImpl for GroupedSelection {
        fn dispose(&self) {
            if let Some(items) = self.items.get() {
                for handler in self.handlers.take() {
                    items.disconnect(handler);
                }
            }
        }
    }

    impl ListModelImpl for GroupedSelection {
        fn item_type(&self) -> glib::Type {
            <glib::Object as glib::prelude::StaticType>::static_type()
        }

        fn n_items(&self) -> u32 {
            self.layout.borrow().as_ref().map_or(0, SlotLayout::len)
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.obj().slot_object(position)
        }
    }

    impl SelectionModelImpl for GroupedSelection {
        fn selection_in_range(&self, position: u32, n_items: u32) -> gtk::Bitset {
            let selected = gtk::Bitset::new_empty();
            let obj = self.obj();
            for slot in position..position.saturating_add(n_items).min(self.n_items()) {
                if obj
                    .item_at(slot)
                    .is_some_and(|item| obj.items().is_selected(item))
                {
                    selected.add(slot);
                }
            }
            selected
        }

        fn is_selected(&self, position: u32) -> bool {
            let obj = self.obj();
            obj.item_at(position)
                .is_some_and(|item| obj.items().is_selected(item))
        }

        fn select_all(&self) -> bool {
            self.obj().items().select_all()
        }

        fn unselect_all(&self) -> bool {
            self.obj().items().unselect_all()
        }

        fn select_item(&self, position: u32, unselect_rest: bool) -> bool {
            let obj = self.obj();
            let Some(item) = obj.item_at(position) else {
                return false;
            };
            self.anchor.set(Some(item));
            obj.items().select_item(item, unselect_rest)
        }

        fn unselect_item(&self, position: u32) -> bool {
            let obj = self.obj();
            let Some(item) = obj.item_at(position) else {
                return false;
            };
            self.anchor.set(Some(item));
            obj.items().unselect_item(item)
        }

        fn select_range(&self, position: u32, n_items: u32, unselect_rest: bool) -> bool {
            let obj = self.obj();
            match obj.items_in(position, n_items) {
                Some((first, count)) => obj.items().select_range(first, count, unselect_rest),
                None if unselect_rest => obj.items().unselect_all(),
                None => false,
            }
        }

        fn unselect_range(&self, position: u32, n_items: u32) -> bool {
            let obj = self.obj();
            obj.items_in(position, n_items)
                .is_some_and(|(first, count)| obj.items().unselect_range(first, count))
        }

        fn set_selection(&self, selected: &gtk::Bitset, mask: &gtk::Bitset) -> bool {
            let obj = self.obj();
            let items_selected = gtk::Bitset::new_empty();
            let items_mask = gtk::Bitset::new_empty();
            let end = mask.maximum().min(self.n_items().saturating_sub(1));
            for slot in (mask.minimum()..=end).filter(|slot| mask.contains(*slot)) {
                let Some(item) = obj.item_at(slot) else {
                    continue;
                };
                items_mask.add(item);
                if selected.contains(slot) {
                    items_selected.add(item);
                }
            }
            obj.items().set_selection(&items_selected, &items_mask)
        }
    }
}

glib::wrapper! {
    /// The folder's selection model as grouped Tiles and Content list it:
    /// a heading row before each group, and each group's last row filled.
    pub(crate) struct GroupedSelection(ObjectSubclass<imp::GroupedSelection>)
        @implements gio::ListModel, gtk::SelectionModel;
}

impl GroupedSelection {
    /// `items` grouped by `title`, in `columns` columns.
    pub(crate) fn new(items: &gtk::MultiSelection, title: GroupTitle, columns: u32) -> Self {
        let model: Self = glib::Object::new();
        let imp = model.imp();
        imp.items
            .set(items.clone())
            .expect("a new model has no items yet");
        if imp.title.set(title).is_err() {
            unreachable!("a new model has no titles yet");
        }
        model.relayout(columns, None, false);
        let on_items = items.connect_items_changed(glib::clone!(
            #[weak]
            model,
            move |_, position, removed, added| {
                let change = ItemsChange {
                    position,
                    removed,
                    added,
                };
                let imp = model.imp();
                imp.anchor
                    .set(imp.anchor.get().and_then(|anchor| change.moved(anchor)));
                model.relayout(model.columns(), Some(change), true);
            }
        ));
        let on_selection = items.connect_selection_changed(glib::clone!(
            #[weak]
            model,
            move |_, position, count| model.follow_selection(position, count)
        ));
        imp.handlers.replace(vec![on_items, on_selection]);
        model
    }

    /// The folder's selection model.
    pub(crate) fn items(&self) -> &gtk::MultiSelection {
        self.imp()
            .items
            .get()
            .expect("GroupedSelection::new sets the items")
    }

    /// The columns the slots are laid out in.
    pub(crate) fn columns(&self) -> u32 {
        self.imp().layout.borrow().as_ref().map_or(1, SlotLayout::columns)
    }

    /// Lays the slots out in `columns` columns.
    pub(crate) fn set_columns(&self, columns: u32) {
        if columns.max(1) != self.columns() {
            self.relayout(columns, None, true);
        }
    }

    /// Lays the groups out again, as their titles may have changed (the
    /// day moved on, or the folder was grouped anew).
    pub(crate) fn regroup(&self) {
        self.relayout(self.columns(), None, true);
    }

    /// The listing position the grid's `slot` shows, if it shows an item.
    pub(crate) fn item_at(&self, slot: u32) -> Option<u32> {
        self.imp().layout.borrow().as_ref()?.item_at(slot)
    }

    /// The grid slot showing the listing's item at `position`.
    pub(crate) fn slot_of(&self, position: u32) -> Option<u32> {
        self.imp().layout.borrow().as_ref()?.slot_of(position)
    }

    /// The layout of the slots, for the window's keys.
    pub(crate) fn layout(&self) -> Option<SlotLayout> {
        self.imp().layout.borrow().clone()
    }

    /// The item Shift extends a selection from.
    pub(crate) fn anchor(&self) -> Option<u32> {
        self.imp().anchor.get()
    }

    /// The first item at or after `slot` and how many items lie in the
    /// `count` slots from it; `None` when they hold none.
    fn items_in(&self, slot: u32, count: u32) -> Option<(u32, u32)> {
        let layout = self.imp().layout.borrow();
        let layout = layout.as_ref()?;
        let end = slot.saturating_add(count).min(layout.len());
        let first = (slot..end).find_map(|slot| layout.item_at(slot))?;
        let last = (slot..end).rev().find_map(|slot| layout.item_at(slot))?;
        Some((first, last - first + 1))
    }

    /// The object the grid shows in `slot`.
    fn slot_object(&self, slot: u32) -> Option<glib::Object> {
        let imp = self.imp();
        let layout = imp.layout.borrow();
        let layout = layout.as_ref()?;
        let objects = imp.objects.borrow();
        let (group, offset) = layout.place(slot)?;
        let group_objects = objects.get(group)?;
        let object = match layout.slot(slot)? {
            Slot::Item(position) => return self.items().item(position),
            Slot::Heading(_) => &group_objects.heading,
            Slot::Rule => group_objects.rules.get(usize::try_from(offset).ok()? - 1)?,
            Slot::Blank => {
                let past = offset - layout.columns() - layout.groups()[group].len();
                group_objects.blanks.get(usize::try_from(past).ok()?)?
            }
        };
        Some(object.clone().upcast())
    }

    /// Groups the listing again in `columns` columns, after `change`, and
    /// tells the grid which slots changed when `notify`.
    fn relayout(&self, columns: u32, change: Option<ItemsChange>, notify: bool) {
        let imp = self.imp();
        let items = self.items();
        let title = imp.title.get().expect("GroupedSelection::new sets the titles");
        let titles: Vec<String> = (0..items.n_items())
            .map(|position| {
                items
                    .item(position)
                    .and_downcast::<FileItem>()
                    .and_then(|item| title(&item))
                    .unwrap_or_default()
            })
            .collect();
        let layout = SlotLayout::new(group_runs(titles.iter().map(String::as_str)), columns);
        self.keep_objects(&layout);
        let old = imp.layout.replace(Some(layout));
        if !notify {
            return;
        }
        let changed = {
            let new = imp.layout.borrow();
            let new = new.as_ref().expect("just laid out");
            match &old {
                Some(old) => old.changes_to(new, change),
                None => Some((0, 0, new.len())),
            }
        };
        if let Some((position, removed, added)) = changed {
            self.items_changed(position, removed, added);
        }
    }

    /// Gives every group of `layout` its slot objects, keeping those of a
    /// group with the same title, and writes each heading's title and
    /// count.
    fn keep_objects(&self, layout: &SlotLayout) {
        let mut kept: HashMap<String, GroupObjects> = HashMap::new();
        let old_titles: Vec<String> = self
            .imp()
            .layout
            .borrow()
            .as_ref()
            .map(|old| old.groups().iter().map(|group| group.title.clone()).collect())
            .unwrap_or_default();
        for (title, objects) in old_titles.into_iter().zip(self.imp().objects.take()) {
            kept.entry(title).or_insert(objects);
        }
        let columns = layout.columns();
        let objects: Vec<GroupObjects> = layout
            .groups()
            .iter()
            .map(|group| {
                let mut objects = kept.remove(&group.title).unwrap_or_else(GroupObjects::new);
                let rules = usize::try_from(columns - 1).unwrap_or_default();
                let blanks = usize::try_from((columns - group.len() % columns) % columns).unwrap_or_default();
                GroupObjects::fit(&mut objects.rules, rules, SlotKind::Rule);
                GroupObjects::fit(&mut objects.blanks, blanks, SlotKind::Blank);
                let text = header_text(&group.title, group.len());
                if objects.heading.text() != text {
                    objects.heading.set_text(text);
                }
                objects
            })
            .collect();
        self.imp().objects.replace(objects);
    }

    /// Tells the grid the folder's selection changed for `count` items
    /// from `position`.
    fn follow_selection(&self, position: u32, count: u32) {
        let first = self.slot_of(position);
        let last = count
            .checked_sub(1)
            .and_then(|last| self.slot_of(position + last));
        match (first, last) {
            (Some(first), Some(last)) if last >= first => self.selection_changed(first, last - first + 1),
            _ => {
                let len = self.n_items();
                if len > 0 {
                    self.selection_changed(0, len);
                }
            }
        }
    }
}
