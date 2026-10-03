// SPDX-License-Identifier: AGPL-3.0-only
//! The app's drop-down and context menus: a glyph, a label and a shortcut
//! per item, in the classic or the compact style.
//!
//! Ports `openMenu` in `v2.0.0:desktop/ui/app.js` with `.menu.win10` and
//! `.menu.win11` in `v2.0.0:desktop/ui/style.css`. GTK's `PopoverMenu` hides the
//! icon of a labelled item, so the items are rows of a `GtkListBox`, which
//! also gives arrow-key movement and Enter activation. Each row runs a
//! window or application action: a disabled action, or an item disabled
//! in this menu, greys its row out, and a checked item shows the check
//! glyph in place of its own, as app.js does. The compact style
//! (CMD-008) puts a strip of icon buttons above the list: Cut, Copy,
//! Paste, Rename and Delete in the context menus.
//!
//! An item with a submenu (a chevron at its right) opens it beside the
//! menu, as Windows 11's menus do: hovering it opens the submenu after a
//! moment, clicking it or Right opens it at once, and Left or Escape
//! closes only the submenu. Choosing an item in a submenu closes every
//! menu of the chain, then runs it.

#[cfg(test)]
mod inspection;
mod items;

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, Icon};
use crate::integration;

pub(super) use items::{ItemAvailability, ItemCheck, MenuAction, MenuEntry, MenuItem, MenuStyle};

/// The class of a row that follows a divider.
const AFTER_DIVIDER: &str = "after-divider";

/// How long the pointer rests on a row before its submenu opens, or on
/// another row before an open submenu closes; Windows waits about as long.
const SUBMENU_DELAY: Duration = Duration::from_millis(250);

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
        /// The submenu open beside a row, and that row's index.
        pub(super) submenu: RefCell<Option<(super::MenuPopover, i32)>>,
        /// The row the pointer rests on, and the timer that opens or
        /// closes a submenu once it has rested there.
        pub(super) hover: RefCell<(Option<i32>, Option<glib::SourceId>)>,
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
            // A service catalogue can make a menu taller than the monitor.
            // Keep every command reachable by scrolling and keyboard focus.
            let rows = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .propagate_natural_height(true)
                .max_content_height(560)
                .child(&list)
                .build();
            content.append(&rows);
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
            popover.connect_closed(|popover| {
                popover.cancel_hover();
                popover.close_submenu();
            });
        }

        fn dispose(&self) {
            self.obj().cancel_hover();
            self.obj().close_submenu();
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
    strip.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "File actions",
    ))]);
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
    follow_hover(popover, &list);
    open_submenus_by_keyboard(popover, &list);
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

/// Opens a row's submenu once the pointer rests on it, and closes an open
/// submenu once it rests on another row.
fn follow_hover(popover: &MenuPopover, list: &gtk::ListBox) {
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(glib::clone!(
        #[weak]
        popover,
        move |_, _, y| {
            #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
            let index = popover.list().row_at_y(y as i32).map(|row| row.index());
            popover.rest_on(index);
        }
    ));
    motion.connect_leave(glib::clone!(
        #[weak]
        popover,
        move |_| popover.rest_on(None)
    ));
    list.add_controller(motion);
}

/// Right opens the focused row's submenu and moves into it; Left in a
/// submenu closes it and goes back to its row.
fn open_submenus_by_keyboard(popover: &MenuPopover, list: &gtk::ListBox) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        popover,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| match key {
            gdk::Key::Right | gdk::Key::KP_Right => {
                let focused = popover.list().focus_child().and_downcast::<gtk::ListBoxRow>();
                let opens = focused.and_then(|row| {
                    let item = popover.item(row.index())?;
                    (!item.submenu.is_empty() && popover.can_choose(&item)).then_some(row.index())
                });
                match opens {
                    Some(index) => {
                        popover.open_submenu(index, true);
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            }
            gdk::Key::Left | gdk::Key::KP_Left => match popover.parent_menu() {
                Some(parent) => {
                    parent.close_submenu();
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
            _ => glib::Propagation::Proceed,
        }
    ));
    list.add_controller(keys);
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

    /// Calls `on_middle_click` with the index of a row middle-clicked and
    /// the modifiers held, after closing the menu, so a history menu can
    /// open its entry in a new tab (NAV-007).
    pub(super) fn connect_row_middle_click(
        &self,
        on_middle_click: impl Fn(usize, gdk::ModifierType) + 'static,
    ) {
        self.list()
            .add_controller(super::gestures::middle_click(glib::clone!(
                #[weak(rename_to = popover)]
                self,
                move |gesture, _, y| {
                    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
                    let row = popover.list().row_at_y(y as i32);
                    let Some(index) = row.and_then(|row| usize::try_from(row.index()).ok()) else {
                        return;
                    };
                    popover.popdown();
                    on_middle_click(index, gesture.current_event_state());
                }
            )));
    }

    /// The rows' list, which a drag's drop target watches.
    pub(super) fn row_list(&self) -> gtk::ListBox {
        self.list().clone()
    }

    /// The item of the row at `y` in the rows' list.
    pub(super) fn item_at(&self, y: f64) -> Option<MenuItem> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let row = self.list().row_at_y(y as i32)?;
        self.item(row.index())
    }

    /// Marks the row of the item whose target is `target` with
    /// `css_class`, and no other row.
    pub(super) fn mark_row(&self, target: Option<&glib::Variant>, css_class: &str) {
        for (index, item) in self.items().into_iter().enumerate() {
            let Some(row) = i32::try_from(index)
                .ok()
                .and_then(|index| self.list().row_at_index(index))
            else {
                continue;
            };
            if target.is_some() && item.target.as_ref() == target {
                row.add_css_class(css_class);
            } else {
                row.remove_css_class(css_class);
            }
        }
    }

    /// The items, one per row, without the dividers.
    fn items(&self) -> Vec<MenuItem> {
        let entries = self.imp().entries.borrow();
        let items = entries.iter().filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item.clone()),
            MenuEntry::Divider => None,
        });
        items.collect()
    }

    /// The item of the row at `index`.
    fn item(&self, index: i32) -> Option<MenuItem> {
        let index = usize::try_from(index).ok()?;
        self.items().into_iter().nth(index)
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
        // A submenu hangs from a row, which is about to go.
        self.close_submenu();
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

    /// Runs the item of the row at `index`, or opens its submenu.
    fn choose_row(&self, index: i32) {
        let Some(item) = self.item(index) else {
            return;
        };
        if item.submenu.is_empty() {
            self.choose(&item);
        } else {
            self.open_submenu(index, true);
        }
    }

    /// Closes the menu, then runs `item`'s action, as `closeMenu()` before
    /// `it.fn()` in app.js, so an item may open another menu here. An item
    /// with a submenu opens it instead. In a submenu, every menu of the
    /// chain closes.
    fn choose(&self, item: &MenuItem) {
        if !item.submenu.is_empty() {
            if let Some(index) = self.items().iter().position(|shown| shown == item) {
                self.open_submenu(i32::try_from(index).unwrap_or(i32::MAX), true);
            }
            return;
        }
        // The first menu of the chain runs the action: closing it lets go
        // of its submenus, which then have no window to find it in.
        let mut first = self.clone();
        while let Some(parent) = first.parent_menu() {
            first = parent;
        }
        self.popdown();
        first.popdown();
        // GTK fails only when no ancestor has the action. Every browser
        // window and the application register them all, so that is a menu
        // outside a window, which has nothing to run.
        let _ = first.activate_action(&item.action.detailed_name(), item.target.as_ref());
    }

    /// The menu this one is a submenu of, if it is one.
    fn parent_menu(&self) -> Option<MenuPopover> {
        self.parent()?.ancestor(MenuPopover::static_type()).and_downcast()
    }

    /// The submenu open beside a row, for tests.
    #[cfg(test)]
    pub(crate) fn open_submenu_menu(&self) -> Option<MenuPopover> {
        self.imp().submenu.borrow().as_ref().map(|(menu, _)| menu.clone())
    }

    /// Opens the submenu of the row at `index` beside it, closing another
    /// one; with `focused`, the keyboard moves into it.
    fn open_submenu(&self, index: i32, focused: bool) {
        let open = self.imp().submenu.borrow().as_ref().map(|(_, row)| *row);
        if open == Some(index) {
            if focused {
                if let Some((menu, _)) = self.imp().submenu.borrow().as_ref() {
                    menu.focus_first_item();
                }
            }
            return;
        }
        self.close_submenu();
        let (Some(item), Some(row)) = (self.item(index), self.list().row_at_index(index)) else {
            return;
        };
        if item.submenu.is_empty() || !self.can_choose(&item) {
            return;
        }
        let menu = MenuPopover::new(item.submenu.clone());
        // Beside the row, its first item level with the row, as Windows
        // opens a submenu. It hangs from this menu, not from the row: the
        // pointer moving over the submenu must not count as hovering this
        // menu's rows, which events bubbling up through the row would.
        menu.set_position(gtk::PositionType::Right);
        menu.set_halign(gtk::Align::Fill);
        menu.set_offset(2, -4);
        menu.set_parent(self);
        if let Some(bounds) = row.compute_bounds(self) {
            #[expect(clippy::cast_possible_truncation, reason = "menu rows are small")]
            let area = gdk::Rectangle::new(
                bounds.x() as i32,
                bounds.y() as i32,
                bounds.width() as i32,
                bounds.height() as i32,
            );
            menu.set_pointing_to(Some(&area));
        }
        // Escape, or a click elsewhere, closes the submenu by itself:
        // it is let go of once GTK is done closing it.
        menu.connect_closed(glib::clone!(
            #[weak(rename_to = popover)]
            self,
            move |closed| {
                let open = popover
                    .imp()
                    .submenu
                    .borrow()
                    .as_ref()
                    .map(|(menu, _)| menu.clone());
                if open.as_ref() == Some(closed) {
                    let closed = closed.clone();
                    glib::idle_add_local_once(glib::clone!(
                        #[weak]
                        popover,
                        move || {
                            let still_open = popover
                                .imp()
                                .submenu
                                .borrow()
                                .as_ref()
                                .map(|(menu, _)| menu.clone());
                            if still_open.as_ref() == Some(&closed) {
                                popover.close_submenu();
                            }
                        }
                    ));
                }
            }
        ));
        self.imp().submenu.replace(Some((menu.clone(), index)));
        menu.popup();
        if !focused {
            // The pointer opened it: the keyboard stays on the row.
            row.grab_focus();
        }
    }

    /// Closes the submenu open beside a row, if one is.
    fn close_submenu(&self) {
        let Some((menu, index)) = self.imp().submenu.take() else {
            return;
        };
        menu.popdown();
        menu.unparent();
        if let Some(row) = self.list().row_at_index(index) {
            if self.is_visible() {
                row.grab_focus();
            }
        }
    }

    /// The pointer rests on the row at `index`, or on none: after
    /// [`SUBMENU_DELAY`], that row's submenu opens, or the open one
    /// closes when the pointer rests on another row.
    fn rest_on(&self, index: Option<i32>) {
        if self.imp().hover.borrow().0 == index {
            return;
        }
        self.cancel_hover();
        self.imp().hover.borrow_mut().0 = index;
        let open = self.imp().submenu.borrow().as_ref().map(|(_, row)| *row);
        let opens = index.filter(|index| {
            self.item(*index)
                .is_some_and(|item| !item.submenu.is_empty() && self.can_choose(&item))
        });
        let wanted = match (opens, index) {
            (Some(row), _) if open != Some(row) => Some(true),
            (None, Some(_)) if open.is_some() => Some(false),
            _ => None,
        };
        let Some(opening) = wanted else {
            return;
        };
        let timer = glib::timeout_add_local_once(
            SUBMENU_DELAY,
            glib::clone!(
                #[weak(rename_to = popover)]
                self,
                move || {
                    popover.imp().hover.borrow_mut().1 = None;
                    match (opening, opens) {
                        (true, Some(row)) => popover.open_submenu(row, false),
                        _ => popover.close_submenu(),
                    }
                }
            ),
        );
        self.imp().hover.borrow_mut().1 = Some(timer);
    }

    /// Stops a submenu from opening or closing on its timer.
    fn cancel_hover(&self) {
        let mut hover = self.imp().hover.borrow_mut();
        hover.0 = None;
        if let Some(timer) = hover.1.take() {
            timer.remove();
        }
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

/// The tooltip of `item`: its label, which a narrow menu may cut short.
fn item_tooltip(item: &MenuItem) -> String {
    item.label.clone()
}

/// Why `item` cannot be chosen, when something says: the reason this
/// menu gave, or its action's in the window of `menu`.
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
    if item.emphasised {
        let bold = gtk::pango::AttrList::new();
        bold.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
        label.set_attributes(Some(&bold));
    }
    content.append(&label);
    if !item.submenu.is_empty() {
        content.append(&icons::image(Icon::ChevronRight16, ROW_GLYPH));
    }
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
    // Every item's title is its label (`b.title=it.label` in app.js).
    row.set_tooltip_text(Some(&item.label));
    if let Some(state) = check.accessible_state() {
        row.update_state(&[gtk::accessible::State::Checked(state)]);
    }
    if check == CheckMark::Checked {
        row.add_css_class("checked");
    }
    row
}

/// Names an icon-only menu button for screen readers. GTK 4.14 gives
/// keyboard focus to the menu button's inner toggle, which does not take
/// the menu button's name, so both carry it.
pub(super) fn name_menu_button(button: &gtk::MenuButton, name: &str) {
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    if let Some(toggle) = button.first_child() {
        toggle.update_property(&[gtk::accessible::Property::Label(name)]);
    }
}
