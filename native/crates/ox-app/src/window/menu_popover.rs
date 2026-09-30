// SPDX-License-Identifier: AGPL-3.0-only
//! The app's drop-down and context menus: a glyph, a label and a shortcut
//! per item, in the classic or the compact style.
//!
//! Ports `openMenu` in `desktop/ui/app.js` with `.menu.win10` and
//! `.menu.win11` in `desktop/ui/style.css`. GTK's `PopoverMenu` hides the
//! icon of a labelled item, so the items are rows of a `GtkListBox`, which
//! also gives arrow-key movement and Enter activation. Each row runs a
//! window or application action: a disabled action, or an item disabled
//! in this menu, greys its row out, and a checked item shows the check
//! glyph in place of its own, as app.js does. The compact style
//! (CMD-008) puts a strip of icon buttons above the list: Cut, Copy,
//! Paste, Rename and Delete in the context menus.

#[cfg(test)]
mod inspection;
mod items;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};
use crate::integration;

use super::unported;

pub(super) use items::{ItemAvailability, ItemCheck, MenuAction, MenuEntry, MenuItem, MenuStyle};

/// The class of a row that follows a divider.
const AFTER_DIVIDER: &str = "after-divider";

/// A menu row's glyph: 16 pixels, as Windows 11 draws menu icons (ui-spec.md I05;
/// the web app's classic menus drew 15).
const ROW_GLYPH: i32 = 16;

/// The check mark a row shows when the menu opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckMark {
    /// A command, which is never checked.
    NotCheckable,
    /// A choice or toggle that is off.
    Unchecked,
    /// A choice or toggle that is on: the check glyph replaces the item's.
    Checked,
}

impl CheckMark {
    /// The mark of a choice or toggle that is on when `checked`.
    fn checked_if(checked: bool) -> Self {
        if checked {
            CheckMark::Checked
        } else {
            CheckMark::Unchecked
        }
    }

    /// The state screen readers announce, `None` for a command.
    fn accessible_state(self) -> Option<gtk::AccessibleTristate> {
        match self {
            CheckMark::NotCheckable => None,
            CheckMark::Unchecked => Some(gtk::AccessibleTristate::False),
            CheckMark::Checked => Some(gtk::AccessibleTristate::True),
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{MenuEntry, MenuItem, MenuStyle};

    /// Private state of [`super::MenuPopover`].
    #[derive(Debug, Default)]
    pub(crate) struct MenuPopover {
        /// The compact style's icon buttons, above the list.
        pub(super) strip: OnceCell<gtk::Box>,
        /// The rows, built by `constructed`.
        pub(super) list: OnceCell<gtk::ListBox>,
        /// What the rows show, dividers included.
        pub(super) entries: RefCell<Vec<MenuEntry>>,
        /// What the strip's buttons run; the strip shows only in the
        /// compact style.
        pub(super) strip_items: RefCell<Vec<MenuItem>>,
        /// The classic or compact look.
        pub(super) style: Cell<MenuStyle>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MenuPopover {
        const NAME: &'static str = "OxMenuPopover";
        type Type = super::MenuPopover;
        type ParentType = gtk::Popover;
    }

    impl ObjectImpl for MenuPopover {
        fn constructed(&self) {
            self.parent_constructed();
            let popover = self.obj();
            // No arrow, the left edges lined up and 4 pixels below the
            // button, as `openMenu` places `.menu` in app.js.
            popover.set_has_arrow(false);
            popover.set_halign(gtk::Align::Start);
            popover.set_offset(0, 4);
            popover.add_css_class("ox-menu");
            let strip = super::strip_box();
            let list = super::item_list(&popover);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&strip);
            content.append(&list);
            popover.set_child(Some(&content));
            self.strip.set(strip).expect("constructed runs once per object");
            self.list.set(list).expect("constructed runs once per object");
            popover.set_style(MenuStyle::Classic);
            // Check marks follow the actions' state when the menu opens,
            // so the rows are drawn on show. The keyboard starts on the
            // first item that can be chosen once the popover is mapped:
            // before that it cannot take focus.
            popover.connect_show(super::MenuPopover::redraw);
            popover.connect_map(super::MenuPopover::focus_first_item);
        }
    }

    impl WidgetImpl for MenuPopover {}

    impl PopoverImpl for MenuPopover {}
}

glib::wrapper! {
    /// A drop-down menu of [`MenuEntry`] rows.
    pub(crate) struct MenuPopover(ObjectSubclass<imp::MenuPopover>)
        @extends gtk::Popover, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::ShortcutManager;
}

/// The row of icon buttons of the compact style (`.context-strip`).
fn strip_box() -> gtk::Box {
    let strip = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .homogeneous(true)
        .css_classes(["context-strip"])
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    strip.update_property(&[gtk::accessible::Property::Label("File actions")]);
    strip
}

/// The list of rows of `popover`.
fn item_list(popover: &MenuPopover) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .accessible_role(gtk::AccessibleRole::Menu)
        .build();
    // A divider is the header of the row after it, so the keyboard never
    // lands on it. GTK clears the headers of a list without a header
    // function, so rows only carry a class.
    list.set_header_func(|row, _| {
        let divider = row
            .has_css_class(AFTER_DIVIDER)
            .then(|| gtk::Separator::new(gtk::Orientation::Horizontal));
        row.set_header(divider.as_ref());
    });
    list.connect_row_activated(glib::clone!(
        #[weak]
        popover,
        move |_, row| popover.choose_row(row.index())
    ));
    // Up on the first item and Down on the last wrap around, as `openMenu`
    // moves between enabled items.
    list.connect_keynav_failed(|list, direction| {
        let wrapped_to = match direction {
            gtk::DirectionType::Down => first_enabled_row(list),
            gtk::DirectionType::Up => last_enabled_row(list),
            _ => None,
        };
        match wrapped_to {
            Some(row) => {
                row.grab_focus();
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });
    list
}

/// The rows of `list` a user can choose, in order.
fn enabled_rows(list: &gtk::ListBox) -> impl DoubleEndedIterator<Item = gtk::ListBoxRow> {
    let rows: Vec<gtk::ListBoxRow> = super::widget_tree::children(list)
        .filter_map(|child| child.downcast::<gtk::ListBoxRow>().ok())
        .filter(WidgetExt::is_sensitive)
        .collect();
    rows.into_iter()
}

/// The first row of `list` a user can choose.
fn first_enabled_row(list: &gtk::ListBox) -> Option<gtk::ListBoxRow> {
    enabled_rows(list).next()
}

/// The last row of `list` a user can choose.
fn last_enabled_row(list: &gtk::ListBox) -> Option<gtk::ListBoxRow> {
    enabled_rows(list).next_back()
}

impl MenuPopover {
    /// A menu showing `entries`.
    pub(super) fn new(entries: Vec<MenuEntry>) -> Self {
        let popover: Self = glib::Object::new();
        popover.set_entries(entries);
        popover
    }

    /// Replaces the menu's entries.
    pub(super) fn set_entries(&self, entries: Vec<MenuEntry>) {
        self.imp().entries.replace(entries);
        self.redraw();
    }

    /// Shows the menu in `style`, with `strip_items` as the compact
    /// style's icon buttons (the classic style lists them as rows).
    pub(super) fn set_style_and_strip(&self, style: MenuStyle, strip_items: Vec<MenuItem>) {
        self.imp().strip_items.replace(strip_items);
        self.set_style(style);
        self.redraw();
    }

    /// Draws the menu in `style` and names it for screen readers.
    fn set_style(&self, style: MenuStyle) {
        let previous = self.imp().style.replace(style);
        self.remove_css_class(previous.css_class());
        self.add_css_class(style.css_class());
        self.update_property(&[gtk::accessible::Property::Label(style.accessible_name())]);
    }

    fn list(&self) -> &gtk::ListBox {
        self.imp().list.get().expect("constructed builds the list")
    }

    fn strip(&self) -> &gtk::Box {
        self.imp().strip.get().expect("constructed builds the strip")
    }

    /// Moves the keyboard to the first row that can be chosen.
    fn focus_first_item(&self) {
        if let Some(row) = first_enabled_row(self.list()) {
            row.grab_focus();
        }
    }

    /// Rebuilds the strip and the rows, reading each action's state for
    /// its check mark and whether it is enabled.
    fn redraw(&self) {
        self.redraw_strip();
        let list = self.list();
        list.remove_all();
        let mut after_divider = false;
        for entry in self.imp().entries.borrow().iter() {
            let MenuEntry::Item(item) = entry else {
                after_divider = true;
                continue;
            };
            let can_choose = self.can_choose(item);
            let row = item_row(item, self.check_mark(item));
            row.set_sensitive(can_choose);
            explain_availability(self, row.upcast_ref(), item, can_choose);
            if after_divider {
                row.add_css_class(AFTER_DIVIDER);
                after_divider = false;
            }
            list.append(&row);
        }
    }

    /// Fills the strip with a button per strip item, in the compact style
    /// only.
    fn redraw_strip(&self) {
        let strip = self.strip();
        while let Some(child) = strip.first_child() {
            strip.remove(&child);
        }
        let shows_strip = self.imp().style.get() == MenuStyle::Compact;
        let items = self.imp().strip_items.borrow();
        strip.set_visible(shows_strip && !items.is_empty());
        if !shows_strip {
            return;
        }
        for item in items.iter() {
            strip.append(&self.strip_button(item));
        }
    }

    /// An icon button of the strip.
    fn strip_button(&self, item: &MenuItem) -> gtk::Button {
        let can_choose = self.can_choose(item);
        let button = gtk::Button::builder()
            .child(&icons::image(item.glyph, ROW_GLYPH))
            .tooltip_text(item_tooltip(item))
            .sensitive(can_choose)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&item.label)]);
        explain_availability(self, button.upcast_ref(), item, can_choose);
        let item = item.clone();
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = popover)]
            self,
            move |_| popover.choose(&item)
        ));
        button
    }

    /// True when `item` can be chosen now: it is not disabled in this menu
    /// and its action is enabled.
    fn can_choose(&self, item: &MenuItem) -> bool {
        item.availability != ItemAvailability::Disabled && item.action.is_enabled(self.upcast_ref())
    }

    /// Runs the item of the row at `index`.
    fn choose_row(&self, index: i32) {
        let item = {
            let entries = self.imp().entries.borrow();
            let items = entries.iter().filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item),
                MenuEntry::Divider => None,
            });
            let mut items = items;
            usize::try_from(index)
                .ok()
                .and_then(|index| items.nth(index).cloned())
        };
        if let Some(item) = item {
            self.choose(&item);
        }
    }

    /// Closes the menu, then runs `item`'s action, as `closeMenu()` before
    /// `it.fn()` in app.js, so an item may open another menu here.
    fn choose(&self, item: &MenuItem) {
        self.popdown();
        // GTK fails only when no ancestor has the action. Every browser
        // window and the application register them all, so that is a menu
        // outside a window, which has nothing to run.
        let _ = self.activate_action(&item.action.detailed_name(), item.target.as_ref());
    }

    /// The check mark `item` shows now.
    fn check_mark(&self, item: &MenuItem) -> CheckMark {
        match &item.check {
            ItemCheck::Plain => CheckMark::NotCheckable,
            ItemCheck::Fixed(checked) => CheckMark::checked_if(*checked),
            ItemCheck::FollowsAction => {
                let state = item.action.state(self.upcast_ref());
                let expected = item.target.clone().unwrap_or_else(|| true.to_variant());
                CheckMark::checked_if(state.as_ref() == Some(&expected))
            }
        }
    }
}

/// The tooltip of `item`: its label, and for a command that another
/// milestone brings, that milestone.
fn item_tooltip(item: &MenuItem) -> String {
    match item.action {
        MenuAction::Window(action) => unported::tooltip(action, &item.label),
        MenuAction::Application(_) => item.label.clone(),
    }
}

/// Why `item` cannot be chosen, when something says: the reason this
/// menu gave, the milestone that brings the command, or its action's in
/// the window of `menu`.
fn disabled_reason(menu: &MenuPopover, item: &MenuItem) -> Option<String> {
    if item.availability == ItemAvailability::Disabled {
        if let Some(reason) = item.disabled_reason {
            return Some(reason.to_owned());
        }
    }
    let tooltip = item_tooltip(item);
    if tooltip != item.label {
        return None;
    }
    item.action.disabled_reason(menu.upcast_ref()).map(str::to_owned)
}

/// Adds to the tooltip of `control`, which shows `item` in `menu`, why it
/// cannot be chosen, and tells screen readers too.
fn explain_availability(menu: &MenuPopover, control: &gtk::Widget, item: &MenuItem, can_choose: bool) {
    let reason = (!can_choose).then(|| disabled_reason(menu, item)).flatten();
    let Some(reason) = reason else {
        return;
    };
    control.set_tooltip_text(Some(&format!("{}\n{reason}", item.label)));
    control.update_property(&[gtk::accessible::Property::Description(&reason)]);
}

/// A row's glyph (the check mark while checked, as app.js draws it; the
/// application's own icon for an item that opens one), its label and its
/// shortcut.
fn item_content(item: &MenuItem, check: CheckMark) -> gtk::Box {
    let glyph = if check == CheckMark::Checked {
        Icon::Checkmark
    } else {
        item.glyph
    };
    let application_icon = item
        .application_icon
        .as_deref()
        .filter(|_| check != CheckMark::Checked)
        .and_then(|icon| integration::application_image(Some(icon), ROW_GLYPH));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    content.append(&application_icon.unwrap_or_else(|| icons::image(glyph, ROW_GLYPH)));
    let label = gtk::Label::builder()
        .label(&item.label)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    content.append(&label);
    if let Some(shortcut) = item.shortcut {
        let shortcut = gtk::Label::builder()
            .label(shortcut)
            .css_classes(["shortcut"])
            .build();
        content.append(&shortcut);
    }
    content
}

/// The row for `item`, showing `check`.
fn item_row(item: &MenuItem, check: CheckMark) -> gtk::ListBoxRow {
    let role = match check {
        CheckMark::NotCheckable => gtk::AccessibleRole::MenuItem,
        CheckMark::Unchecked | CheckMark::Checked => gtk::AccessibleRole::MenuItemCheckbox,
    };
    let row = gtk::ListBoxRow::builder()
        .child(&item_content(item, check))
        .accessible_role(role)
        .build();
    row.update_property(&[gtk::accessible::Property::Label(&item.label)]);
    // Every item's title is its label (`b.title=it.label` in app.js); a
    // disabled command adds the milestone that brings it.
    row.set_tooltip_text(Some(&item_tooltip(item)));
    if let Some(state) = check.accessible_state() {
        row.update_state(&[gtk::accessible::State::Checked(state)]);
    }
    if check == CheckMark::Checked {
        row.add_css_class("checked");
    }
    row
}
