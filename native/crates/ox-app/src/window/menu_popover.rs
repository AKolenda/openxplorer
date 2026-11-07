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
//! check mark in a column at its left, before its own glyph, as Windows
//! 11 draws it. The compact style (CMD-008) puts a strip of icon buttons
//! above the list: Cut, Copy, Paste, Rename and Delete in the context
//! menus.
//!
//! An item with a submenu (a chevron at the far right of its row) opens it
//! beside the menu, as Windows 11's menus do: resting the pointer on the
//! row for [`SUBMENU_DELAY`] opens it, as does a click, Right or Enter at
//! once; resting on another row for as long closes it, so moving across
//! the rows towards an open submenu does not. Its first item is level
//! with the row, and GTK flips it to the left near the screen's edge.
//! The keyboard moving to another row takes over from the pointer: the
//! submenu it was about to open does not, and one open beside the row it
//! left closes. Left or Escape closes only the open submenu, the innermost one. Choosing an item
//! in a submenu closes every menu, then runs it.
//!
//! A submenu is a popup of the menu it opens from, without a pointer grab
//! of its own (`autohide` off): the menu that opened from the window keeps
//! the only grab, so the compositor sees one grabbing popup, as it does
//! for a menu without submenus, however many submenus open and close.
//! Each submenu was once a second grabbing popup nested on the first, and
//! opening Group by in the Sort menu of a Save dialog froze the KDE Plasma
//! Wayland session. Keys reach a submenu through the menu holding the
//! grab, as GTK sends them to the window's focus, which moves into the
//! submenu. A submenu closes before the menu it opens from.

mod height;
#[cfg(test)]
mod inspection;
#[cfg(test)]
pub(crate) use inspection::{row_label, row_tooltip};
mod items;
mod motion;

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, Icon};
use crate::integration;

pub(super) use items::{ItemAvailability, ItemCheck, MenuAction, MenuEntry, MenuItem, MenuStyle};
#[cfg(test)]
pub(crate) use motion::Slide;
pub(crate) use motion::OPENING_MOTION;

/// The class of a row that follows a divider.
const AFTER_DIVIDER: &str = "after-divider";

/// How long the pointer rests on a row before its submenu opens, or on
/// another row before an open submenu closes: Windows' default menu show
/// delay (`SPI_GETMENUSHOWDELAY`, the `MenuShowDelay` setting), 400 ms.
pub(super) const SUBMENU_DELAY: Duration = Duration::from_millis(400);

/// The class of a row whose submenu is open, which stays highlighted.
const SUBMENU_OPEN: &str = "submenu-open";

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
    /// A choice or toggle that is on: the check mark shows in its column.
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
    use std::time::Instant;

    use gtk::glib::SourceId;

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
        /// The submenu open beside a row, and that row's item index.
        pub(super) submenu: RefCell<Option<(super::MenuPopover, usize)>>,
        /// While the open submenu waits for its row to be laid out before
        /// it shows, whether the keyboard opened it.
        pub(super) submenu_waits: Cell<Option<bool>>,
        /// The row the pointer rests on, and the timer that opens or closes
        /// a submenu once it has rested there for `SUBMENU_DELAY`.
        pub(super) hover: RefCell<(Option<usize>, Option<SourceId>)>,
        /// Whether this menu is a submenu, opened beside a row of another.
        pub(super) is_submenu: Cell<bool>,
        /// Whether the menu opens beside its button because GTK cut it
        /// short below it (height.rs).
        pub(super) beside_button: Cell<bool>,
        /// Which of the two names its opening motion took last
        /// (motion.rs).
        pub(super) opening_turn: Cell<bool>,
        /// When the menu last began to open.
        pub(super) opening_since: Cell<Option<Instant>>,
        /// Whether GTK has placed the popup since the menu began to open.
        pub(super) placed: Cell<bool>,
        /// Which way the menu slides as it opens.
        pub(super) slide: Cell<Option<super::motion::Slide>>,
        /// The timer that lets the contents count as opaque again once
        /// the menu has faded in.
        pub(super) opaque_again: RefCell<Option<SourceId>>,
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
            // Keep every command reachable by scrolling and keyboard focus;
            // a menu that fits on its monitor shows in full (height.rs).
            let rows = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .propagate_natural_height(true)
                .max_content_height(super::height::UNKNOWN_MONITOR_LIMIT)
                .child(&list)
                .build();
            content.append(&rows);
            // Once shown, the menu fits the monitor its own popup is on.
            popover.connect_realize(super::MenuPopover::follow_monitor_changes);
            // It slides in from where GTK places it (motion.rs).
            popover.connect_realize(super::MenuPopover::follow_placement);
            popover.connect_map(super::MenuPopover::refit_if_shown);
            popover.set_child(Some(&content));
            self.strip.set(strip).expect("constructed runs once per object");
            self.list.set(list).expect("constructed runs once per object");
            popover.set_style(MenuStyle::Classic);
            // Check marks follow the actions' state when the menu opens,
            // so the rows are drawn on show. The keyboard starts on the
            // first item that can be chosen once the popover is mapped:
            // before that it cannot take focus.
            // A submenu gets the keyboard from the row that opened it,
            // and only when the keyboard opened it.
            popover.connect_show(super::MenuPopover::redraw);
            popover.connect_map(|popover| {
                if !popover.imp().is_submenu.get() {
                    popover.focus_first_item();
                }
            });
            popover.connect_closed(super::MenuPopover::forget_hover);
        }

        fn dispose(&self) {
            self.obj().forget_hover();
            self.obj().close_submenu();
        }
    }

    impl WidgetImpl for MenuPopover {
        fn show(&self) {
            // Before GTK places the menu, so it is placed at its full size.
            self.obj().open_below_button_again();
            self.obj().fit_to_monitor();
            // It fades and slides in, as Windows 11's menus open.
            self.obj().start_opening_motion();
            #[expect(
                deprecated,
                reason = "gtk-rs names the show vfunc after gtk_widget_show; the vfunc stays"
            )]
            self.parent_show();
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            // A submenu opened before the menu laid its rows out, as Right
            // can just after the menu opens, shows beside its row once it
            // has; one already shown follows its row.
            self.obj().point_submenu_at_its_row();
        }

        fn unmap(&self) {
            // A submenu leaves the screen before the menu it opens from,
            // so no popup outlives its parent.
            self.obj().forget_hover();
            self.obj().close_submenu();
            self.parent_unmap();
        }
    }

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
    follow_the_keyboard(popover, &list);
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
/// submenu once it rests on another row. A submenu has no grab of its own,
/// so each menu hears the pointer over its own rows.
fn follow_hover(popover: &MenuPopover, list: &gtk::ListBox) {
    let rest_at = glib::clone!(
        #[weak]
        popover,
        move |y: f64| {
            #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
            let row = popover.list().row_at_y(y as i32);
            popover.rest_on(row.and_then(|row| usize::try_from(row.index()).ok()));
        }
    );
    let motion = gtk::EventControllerMotion::new();
    let on_enter = rest_at.clone();
    motion.connect_enter(glib::clone!(
        #[weak]
        popover,
        move |_, _, y| {
            // The pointer reached this submenu: the menu it opens from
            // keeps it open, whatever rows the pointer crossed on its way.
            if let Some(parent) = popover.parent_menu() {
                parent.keep_submenu_open();
            }
            on_enter(y);
        }
    ));
    motion.connect_motion(move |_, _, y| rest_at(y));
    motion.connect_leave(glib::clone!(
        #[weak]
        popover,
        move |_| popover.rest_on(None)
    ));
    list.add_controller(motion);
}

/// The keyboard moving between rows, with the arrow keys, Home, End or
/// Page Up and Down, takes over from the pointer, as in Windows: a submenu
/// waiting for the pointer to rest no longer opens or closes, and one open
/// beside the row the keyboard left closes. Runs after the list has moved
/// the focus, wrapping around included.
fn follow_the_keyboard(popover: &MenuPopover, list: &gtk::ListBox) {
    list.connect_local(
        "move-cursor",
        true,
        glib::clone!(
            #[weak]
            popover,
            #[upgrade_or]
            None,
            move |_| {
                popover.keyboard_moved();
                None
            }
        ),
    );
}

/// Right, Enter and Space open the focused row's submenu with the keyboard
/// on its first item; Left or Escape closes the submenu open beside the
/// menu, or else the submenu the keyboard is in, and goes back to its row.
fn open_submenus_by_keyboard(popover: &MenuPopover, list: &gtk::ListBox) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        popover,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| match key {
            gdk::Key::Right
            | gdk::Key::KP_Right
            | gdk::Key::Return
            | gdk::Key::KP_Enter
            | gdk::Key::ISO_Enter
            | gdk::Key::space
            | gdk::Key::KP_Space => {
                let focused = popover.list().focus_child().and_downcast::<gtk::ListBoxRow>();
                let opens = focused
                    .and_then(|row| usize::try_from(row.index()).ok())
                    .filter(|index| popover.opens_submenu(*index));
                match opens {
                    Some(index) => {
                        popover.open_submenu(index, true);
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            }
            gdk::Key::Left | gdk::Key::KP_Left | gdk::Key::Escape => {
                // A submenu the pointer opened leaves the keyboard here:
                // the key closes the innermost one open from this menu,
                // not this menu. Otherwise it closes this menu if it is a
                // submenu, and a menu that opened from the window is GTK's
                // to close on Escape.
                let closes = if popover.open_submenu_menu().is_some() {
                    let mut deepest = popover.clone();
                    while let Some(inner) = deepest
                        .open_submenu_menu()
                        .filter(|inner| inner.open_submenu_menu().is_some())
                    {
                        deepest = inner;
                    }
                    Some(deepest)
                } else {
                    popover.parent_menu()
                };
                match closes {
                    Some(menu) => {
                        menu.close_submenu_to_its_row();
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            }
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

    /// A submenu showing `entries` in `style`, to open beside a row of
    /// another menu: to its right, its first item level with the row, or
    /// to its left where there is no room on the right. It takes no
    /// pointer grab: the menu it opens from holds the only one.
    fn new_submenu(entries: Vec<MenuEntry>, style: MenuStyle) -> Self {
        let menu: Self = glib::Object::new();
        menu.imp().is_submenu.set(true);
        menu.set_autohide(false);
        menu.set_position(gtk::PositionType::Right);
        menu.set_valign(gtk::Align::Start);
        // Up by the menu's border and padding, and a compact row's margin,
        // so the first item's top is level with the row's.
        let inset = match style {
            MenuStyle::Classic => 4,
            MenuStyle::Compact => 5,
        };
        menu.set_offset(0, -inset);
        menu.add_css_class("submenu");
        menu.set_style(style);
        menu.set_entries(entries);
        menu
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
    pub(in crate::window) fn items(&self) -> Vec<MenuItem> {
        items_of(&self.imp().entries.borrow()).cloned().collect()
    }

    /// The item of the row at `index`.
    fn item(&self, index: i32) -> Option<MenuItem> {
        self.items().into_iter().nth(usize::try_from(index).ok()?)
    }

    fn list(&self) -> &gtk::ListBox {
        self.imp().list.get().expect("constructed builds the list")
    }

    fn strip(&self) -> &gtk::Box {
        self.imp().strip.get().expect("constructed builds the strip")
    }

    /// Moves the keyboard to the first item that can be chosen.
    fn focus_first_item(&self) {
        if let Some(row) = first_enabled_row(self.list()) {
            row.grab_focus();
        }
    }

    /// Rebuilds the strip and the rows, reading each action's state for
    /// its check mark and whether it is enabled.
    fn redraw(&self) {
        // A submenu opens from a row, which is about to go.
        self.close_submenu();
        self.redraw_strip();
        let list = self.list();
        list.remove_all();
        let mut after_divider = false;
        let entries = self.imp().entries.borrow().clone();
        // A menu with a choice or toggle keeps a column for the marks, as
        // Windows 11 does, so every glyph lines up.
        let marks = if items_of(&entries).any(|item| item.check != ItemCheck::Plain) {
            MarkColumn::Shown
        } else {
            MarkColumn::None
        };
        // A row without a glyph keeps the glyph's column empty where other
        // rows have one, as Windows' classic menu lines up text-only rows
        // with those that show an icon.
        let glyphs = if items_of(&entries).any(shows_a_glyph) {
            GlyphColumn::Shown
        } else {
            GlyphColumn::None
        };
        for entry in &entries {
            let MenuEntry::Item(item) = entry else {
                after_divider = true;
                continue;
            };
            let can_choose = self.can_choose(item);
            let row = item_row(item, self.check_mark(item), marks, glyphs);
            row.set_sensitive(can_choose);
            explain_availability(self, row.upcast_ref(), item, can_choose);
            if after_divider {
                row.add_css_class(AFTER_DIVIDER);
                after_divider = false;
            }
            list.append(&row);
        }
        self.refit_if_shown();
    }

    /// Fills the strip with a button per strip item, in the compact style
    /// only.
    fn redraw_strip(&self) {
        let strip = self.strip();
        while let Some(child) = strip.first_child() {
            strip.remove(&child);
        }
        // A submenu shows its own items only.
        let shows_strip = self.imp().style.get() == MenuStyle::Compact && !self.imp().is_submenu.get();
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
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        if let Some(item) = self.items().into_iter().nth(index) {
            self.choose(&item);
        }
    }

    /// Closes the menu, then runs `item`'s action, as `closeMenu()` before
    /// `it.fn()` in app.js, so an item may open another menu here. An item
    /// with a submenu opens it instead. In a submenu, every menu closes.
    fn choose(&self, item: &MenuItem) {
        if !item.submenu.is_empty() {
            if let Some(index) = self.items().iter().position(|shown| shown == item) {
                self.open_submenu(index, false);
            }
            return;
        }
        // The first menu runs the action: it hangs from the window, and its
        // submenus close with it.
        let first = self.root_menu();
        first.popdown();
        // GTK fails only when no ancestor has the action. Every browser
        // window and the application register them all, so that is a menu
        // outside a window, which has nothing to run.
        let _ = first.activate_action(&item.action.detailed_name(), item.target.as_ref());
    }

    /// The menu this one is a submenu of, if it is one.
    fn parent_menu(&self) -> Option<MenuPopover> {
        if !self.imp().is_submenu.get() {
            return None;
        }
        self.parent().and_downcast()
    }

    /// The menu that opened from the window, of which this is a submenu or
    /// which this is.
    fn root_menu(&self) -> MenuPopover {
        let mut first = self.clone();
        while let Some(parent) = first.parent_menu() {
            first = parent;
        }
        first
    }

    /// Whether the item of the row at `index` opens a submenu now.
    fn opens_submenu(&self, index: usize) -> bool {
        self.items()
            .into_iter()
            .nth(index)
            .is_some_and(|item| !item.submenu.is_empty() && self.can_choose(&item))
    }

    /// The submenu open beside a row, for tests and the snapshot hook.
    pub(crate) fn open_submenu_menu(&self) -> Option<MenuPopover> {
        self.imp().submenu.borrow().as_ref().map(|(menu, _)| menu.clone())
    }

    /// The row at `index`.
    fn row_at(&self, index: usize) -> Option<gtk::ListBoxRow> {
        self.list().row_at_index(i32::try_from(index).ok()?)
    }

    /// Opens the submenu of the item at `index` beside its row, closing
    /// another one; with `keyboard`, the keyboard moves to its first item,
    /// as Right and Enter do, and otherwise stays where it is, as when the
    /// pointer opens it.
    pub(super) fn open_submenu(&self, index: usize, keyboard: bool) {
        let open = self
            .open_submenu_menu()
            .filter(|_| self.open_submenu_index() == Some(index));
        if let Some(open) = open {
            if let Some(waiting_keyboard) = self.imp().submenu_waits.get() {
                self.imp().submenu_waits.set(Some(waiting_keyboard || keyboard));
            } else if keyboard {
                open.focus_first_item();
            }
            return;
        }
        if !self.opens_submenu(index) {
            return;
        }
        self.cancel_hover();
        self.close_submenu();
        let (Some(item), Some(row)) = (self.items().into_iter().nth(index), self.row_at(index)) else {
            return;
        };
        let menu = MenuPopover::new_submenu(item.submenu.clone(), self.imp().style.get());
        menu.set_parent(self);
        // Escape, or GTK closing it otherwise, lets go of it once GTK is
        // done closing it.
        menu.connect_closed(glib::clone!(
            #[weak(rename_to = popover)]
            self,
            move |closed| {
                if popover.open_submenu_menu().as_ref() != Some(closed) {
                    return;
                }
                let closed = closed.clone();
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    popover,
                    move || {
                        if popover.open_submenu_menu().as_ref() == Some(&closed) {
                            popover.close_submenu();
                        }
                    }
                ));
            }
        ));
        row.add_css_class(SUBMENU_OPEN);
        row.update_state(&[gtk::accessible::State::Expanded(Some(true))]);
        self.imp().submenu.replace(Some((menu.clone(), index)));
        match row_area(self, &row) {
            Some(area) => show_submenu(&menu, &area, keyboard),
            // Not before the row has a place: GTK 4.14 then finds no
            // monitor to place the submenu on (a critical in
            // gdk_x11_surface_layout_popup). The menu shows it once it has
            // laid the row out.
            None => self.imp().submenu_waits.set(Some(keyboard)),
        }
    }

    /// Opens the submenu of the item labelled `label` beside its row, with
    /// the keyboard on its first item, as Right on its row does; false
    /// when no such item has a submenu that can open.
    pub(in crate::window) fn open_submenu_labelled(&self, label: &str) -> bool {
        let Some(index) = self.items().iter().position(|item| item.label == label) else {
            return false;
        };
        self.open_submenu(index, true);
        self.open_submenu_index() == Some(index)
    }

    /// Closes the open submenu and gives the keyboard to its row, as Left
    /// and Escape do.
    fn close_submenu_to_its_row(&self) {
        let row = self.open_submenu_index().and_then(|index| self.row_at(index));
        self.close_submenu();
        if let Some(row) = row {
            row.grab_focus();
        }
    }

    /// Points the open submenu at its row where the menu has laid it out,
    /// if it points elsewhere, or shows one that waited for its row, after
    /// the menu's allocation.
    fn point_submenu_at_its_row(&self) {
        let Some((menu, index)) = self.imp().submenu.borrow().clone() else {
            return;
        };
        let Some(area) = self.row_at(index).and_then(|row| row_area(self, &row)) else {
            return;
        };
        if let Some(keyboard) = self.imp().submenu_waits.take() {
            glib::idle_add_local_once(glib::clone!(
                #[weak(rename_to = popover)]
                self,
                #[weak]
                menu,
                move || {
                    let area = popover.row_at(index).and_then(|row| row_area(&popover, &row));
                    if let (true, Some(area)) = (popover.open_submenu_menu().as_ref() == Some(&menu), area) {
                        show_submenu(&menu, &area, keyboard);
                    }
                }
            ));
            return;
        }
        let (points, pointing) = menu.pointing_to();
        if !points || pointing != area {
            menu.set_pointing_to(Some(&area));
        }
    }

    /// The item index of the row whose submenu is open.
    fn open_submenu_index(&self) -> Option<usize> {
        self.imp().submenu.borrow().as_ref().map(|(_, index)| *index)
    }

    /// Closes the submenu open beside a row, and any submenu of its own,
    /// if one is. The keyboard, if it was in the submenu, goes back to the
    /// row.
    pub(super) fn close_submenu(&self) {
        let Some((menu, index)) = self.imp().submenu.take() else {
            return;
        };
        self.imp().submenu_waits.set(None);
        let root = self.root();
        let focus = root.as_ref().and_then(gtk::prelude::RootExt::focus);
        let had_keyboard = focus.is_some_and(|focus| focus.is_ancestor(&menu));
        // A submenu of the submenu closes first.
        menu.close_submenu();
        menu.forget_hover();
        menu.popdown();
        menu.unparent();
        if let Some(row) = self.row_at(index) {
            row.remove_css_class(SUBMENU_OPEN);
            row.update_state(&[gtk::accessible::State::Expanded(Some(false))]);
            if had_keyboard && self.is_visible() {
                row.grab_focus();
            }
        }
    }

    /// The pointer rests on the row at `index`, or on none: after
    /// [`SUBMENU_DELAY`], that row's submenu opens, or the open one closes
    /// when the pointer rests on another row. Leaving the rows keeps an
    /// open submenu open, as Windows does.
    fn rest_on(&self, index: Option<usize>) {
        if self.imp().hover.borrow().0 == index {
            return;
        }
        self.cancel_hover();
        self.imp().hover.borrow_mut().0 = index;
        let Some(index) = index else {
            return;
        };
        // The row the pointer is on is the one the keyboard goes on from,
        // as on Windows: Right then opens its submenu, Enter chooses it.
        // The pointer put it there, so it shows the hover's gray, not the
        // keyboard's focus: a key pressed just before, such as the Ctrl+S
        // that opened a Save dialog or a Tab to its name, would otherwise
        // draw it in the keyboard's blue. The next key shows focus again.
        if let Some(row) = self
            .row_at(index)
            .filter(|row| row.is_sensitive() && self.is_mapped())
        {
            row.grab_focus();
            if let Some(window) = self.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(false);
            }
        }
        let open = self.open_submenu_index();
        if open == Some(index) {
            return;
        }
        let opens = self.opens_submenu(index);
        if !opens && open.is_none() {
            return;
        }
        let timer = glib::timeout_add_local_once(
            SUBMENU_DELAY,
            glib::clone!(
                #[weak(rename_to = popover)]
                self,
                move || {
                    popover.imp().hover.borrow_mut().1 = None;
                    // Only for the row the pointer still rests on.
                    if popover.imp().hover.borrow().0 != Some(index) {
                        return;
                    }
                    if opens {
                        popover.open_submenu(index, false);
                    } else {
                        popover.close_submenu();
                    }
                }
            ),
        );
        self.imp().hover.borrow_mut().1 = Some(timer);
    }

    /// The pointer reached the open submenu: it stays open, and its row
    /// counts as the row the pointer rests on.
    fn keep_submenu_open(&self) {
        self.cancel_hover();
        self.imp().hover.borrow_mut().0 = self.open_submenu_index();
    }

    /// The keyboard moved between rows: a submenu the pointer's timer was
    /// to open or close stays as it is, unless it is open beside a row
    /// other than the focused one, which closes it. The row the pointer rests on stays
    /// known, so only the pointer coming onto another row starts a timer
    /// again.
    fn keyboard_moved(&self) {
        self.cancel_hover();
        let focused = self
            .list()
            .focus_child()
            .and_downcast::<gtk::ListBoxRow>()
            .and_then(|row| usize::try_from(row.index()).ok());
        if self
            .open_submenu_index()
            .is_some_and(|open| Some(open) != focused)
        {
            self.close_submenu();
        }
    }

    /// Stops a submenu from opening or closing on its timer. The row the
    /// pointer rests on stays known, so the pointer moving on along it
    /// does not count as coming onto it again.
    fn cancel_hover(&self) {
        if let Some(timer) = self.imp().hover.borrow_mut().1.take() {
            timer.remove();
        }
    }

    /// Stops the timer and forgets the row the pointer rests on, as when
    /// the menu closes.
    fn forget_hover(&self) {
        self.cancel_hover();
        self.imp().hover.borrow_mut().0 = None;
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

/// Closes every menu of `window` when a mouse button is pressed anywhere
/// in it but on a menu. A menu normally closes itself on such a press, but
/// on KDE Plasma a press in the app's own window is the app's to handle,
/// and the Sort menu once stayed on screen until a window of another app
/// was clicked. The press goes on to whatever it is on.
pub(super) fn close_menus_on_press(window: &impl IsA<gtk::Widget>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(0);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    gesture.connect_pressed(|gesture, _, _, _| {
        // Only watching: the press is not taken from the widgets under it.
        gesture.set_state(gtk::EventSequenceState::Denied);
        let pressed = gesture.current_event().and_then(|event| event.surface());
        if let Some(window) = gesture.widget() {
            close_menus_in(&window, pressed.as_ref());
        }
    });
    window.as_ref().add_controller(gesture);
}

/// Closes the menus of `root` for a press on `pressed`: all of them,
/// unless the press is on one of them. A menu that is closed but whose
/// surface is still on screen is taken off it.
pub(super) fn close_menus_in(root: &gtk::Widget, pressed: Option<&gdk::Surface>) {
    let mut menus = Vec::new();
    collect_menus(root, &mut menus);
    let on_a_menu = pressed.is_some_and(|pressed| {
        menus
            .iter()
            .any(|menu| menu.is_visible() && gtk::prelude::NativeExt::surface(menu).as_ref() == Some(pressed))
    });
    if on_a_menu {
        return;
    }
    for menu in &menus {
        if menu.is_visible() {
            menu.popdown();
        }
    }
    for menu in &menus {
        let left_on_screen =
            gtk::prelude::NativeExt::surface(menu).is_some_and(|surface| surface.is_mapped());
        if !menu.is_visible() && left_on_screen {
            WidgetExt::unrealize(menu);
        }
    }
}

/// Adds the menus among `widget`'s descendants to `menus`.
fn collect_menus(widget: &gtk::Widget, menus: &mut Vec<MenuPopover>) {
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(menu) = current.downcast_ref::<MenuPopover>() {
            menus.push(menu.clone());
        }
        collect_menus(&current, menus);
        child = current.next_sibling();
    }
}

/// Shows `submenu` beside `row_area`, its row's place in the menu it opens
/// from, with the keyboard on its first item if `keyboard` opened it.
fn show_submenu(submenu: &MenuPopover, row_area: &gdk::Rectangle, keyboard: bool) {
    submenu.set_pointing_to(Some(row_area));
    submenu.popup();
    if keyboard {
        submenu.focus_first_item();
    }
}

/// Where `row` is in `menu`, once the menu has laid it out.
fn row_area(menu: &MenuPopover, row: &gtk::ListBoxRow) -> Option<gdk::Rectangle> {
    if row.height() == 0 {
        return None;
    }
    let bounds = row.compute_bounds(menu)?;
    #[expect(clippy::cast_possible_truncation, reason = "menu rows are small")]
    let area = gdk::Rectangle::new(
        bounds.x() as i32,
        bounds.y() as i32,
        bounds.width() as i32,
        bounds.height() as i32,
    );
    Some(area)
}

/// The items among `entries`, without the dividers.
fn items_of(entries: &[MenuEntry]) -> impl Iterator<Item = &MenuItem> {
    entries.iter().filter_map(|entry| match entry {
        MenuEntry::Item(item) => Some(item),
        MenuEntry::Divider => None,
    })
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

/// Whether a menu's rows keep a column for check marks and dots before
/// their glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkColumn {
    /// The menu has no choice or toggle.
    None,
    /// The menu has one, so every row keeps the column.
    Shown,
}

/// Whether a menu's rows keep a column for glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlyphColumn {
    /// No row has a glyph.
    None,
    /// A row has one, so every row keeps the column.
    Shown,
}

/// Whether `item`'s row draws a glyph: its own, or its application's icon.
fn shows_a_glyph(item: &MenuItem) -> bool {
    item.shows_glyph || item.application_icon.is_some()
}

/// The mark column of a row: a dot for a chosen choice (or an item that
/// asks for one, such as the open Details pane), a check mark for a toggle
/// that is on, as Windows 11 draws them, else an empty slot.
fn mark_slot(item: &MenuItem, check: CheckMark) -> gtk::Widget {
    if check != CheckMark::Checked {
        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        slot.set_size_request(ROW_GLYPH, ROW_GLYPH);
        slot.add_css_class("menu-mark-slot");
        return slot.upcast();
    }
    let mark = if item.dot_mark {
        Icon::CircleSmallFilled
    } else {
        Icon::Checkmark
    };
    let image = icons::image(mark, ROW_GLYPH);
    image.add_css_class("menu-mark");
    image.upcast()
}

/// A row's mark column when the menu has one, its glyph (the
/// application's own icon for an item that opens one), its label and its
/// shortcut.
fn item_content(item: &MenuItem, check: CheckMark, marks: MarkColumn, glyphs: GlyphColumn) -> gtk::Box {
    let application_icon = item
        .application_icon
        .as_deref()
        .and_then(|icon| integration::application_image(Some(icon), ROW_GLYPH));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    if marks == MarkColumn::Shown {
        content.append(&mark_slot(item, check));
    }
    let glyph = application_icon.or_else(|| {
        item.shows_glyph.then(|| {
            let image = icons::image(item.glyph, ROW_GLYPH);
            if item.glyph_look.mirrored {
                image.add_css_class("mirrored");
            }
            if item.glyph_look.accent {
                image.add_css_class("accent-glyph");
            }
            image
        })
    });
    if let Some(glyph) = glyph {
        content.append(&glyph);
    } else if marks == MarkColumn::None || glyphs == GlyphColumn::Shown {
        // A row without a glyph or a mark column, such as Windows
        // Explorer's text-only right-drag menu's (DND-027), keeps the
        // glyph's column empty, so its text sits where other menus' does,
        // as does a text-only row of a menu whose other rows have glyphs.
        let column = gtk::Box::builder()
            .width_request(ROW_GLYPH)
            .height_request(ROW_GLYPH)
            .build();
        content.append(&column);
    }
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
    if let Some(shortcut) = item.shortcut {
        let shortcut = gtk::Label::builder()
            .label(shortcut)
            .css_classes(["shortcut"])
            .build();
        content.append(&shortcut);
    }
    if !item.submenu.is_empty() {
        let chevron = icons::image(Icon::ChevronRight16, ROW_GLYPH);
        chevron.add_css_class("submenu-arrow");
        content.append(&chevron);
    }
    content
}

/// The whole label of `row` while the menu cuts it short, else `None`.
fn cut_short_label(row: &gtk::ListBoxRow) -> Option<String> {
    let content = row.child()?;
    let label = crate::window::widget_tree::children(&content)
        .find_map(|child| child.downcast::<gtk::Label>().ok())?;
    label.layout().is_ellipsized().then(|| label.text().to_string())
}

/// The row for `item`, showing `check` in the menu's `marks` column, and
/// its glyph, or an empty slot, in the `glyphs` column.
fn item_row(item: &MenuItem, check: CheckMark, marks: MarkColumn, glyphs: GlyphColumn) -> gtk::ListBoxRow {
    let role = match check {
        CheckMark::NotCheckable => gtk::AccessibleRole::MenuItem,
        CheckMark::Unchecked | CheckMark::Checked => gtk::AccessibleRole::MenuItemCheckbox,
    };
    let row = gtk::ListBoxRow::builder()
        .child(&item_content(item, check, marks, glyphs))
        .accessible_role(role)
        .build();
    row.update_property(&[gtk::accessible::Property::Label(&item.label)]);
    // No tooltip that would only repeat the label (`b.title=it.label` in
    // app.js), as Windows 11's menus show none: only a label the menu cuts
    // short shows whole, which adds what the row cannot show. A disabled
    // row's own tooltip, saying why ([`explain_availability`]), comes
    // first.
    row.set_has_tooltip(true);
    row.connect_query_tooltip(|row, _, _, _, tooltip| {
        if row.tooltip_text().is_some() {
            return false;
        }
        let Some(label) = cut_short_label(row) else {
            return false;
        };
        tooltip.set_text(Some(&label));
        true
    });
    if let Some(state) = check.accessible_state() {
        row.update_state(&[gtk::accessible::State::Checked(state)]);
    }
    if check == CheckMark::Checked {
        row.add_css_class("checked");
    }
    if !item.submenu.is_empty() {
        // Screen readers announce a submenu and whether it is open.
        row.update_property(&[gtk::accessible::Property::HasPopup(true)]);
        row.update_state(&[gtk::accessible::State::Expanded(Some(false))]);
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
