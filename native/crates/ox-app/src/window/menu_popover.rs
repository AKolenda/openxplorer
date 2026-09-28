// SPDX-License-Identifier: AGPL-3.0-only
//! The app's drop-down menus: a glyph, a label and a shortcut per item.
//!
//! Ports `openMenu` in `desktop/ui/app.js` with the classic Windows look of
//! `.menu.win10` in `desktop/ui/style.css`. GTK's `PopoverMenu` hides the
//! icon of a labelled item, so the items are rows of a `GtkListBox`, which
//! also gives arrow-key movement and Enter activation. Each row runs a
//! window or application action: a disabled action greys its row out, and
//! a checked item shows the check glyph in place of its own, as app.js
//! does.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::application::AppAction;
use crate::icons::{self, Icon};

use super::unported;
use super::window_action::WindowAction;

/// The class of a row that follows a divider.
const AFTER_DIVIDER: &str = "after-divider";

/// A menu row's glyph: 16 pixels, as Windows 11 draws menu icons (ui-spec.md I05;
/// the web app's classic menus drew 15).
const ROW_GLYPH: i32 = 16;

/// Whether an item shows a check mark.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ItemCheck {
    /// Never checked.
    Plain,
    /// Checked or not, decided when the menu is built.
    Fixed(bool),
    /// Checked while the action's state equals the item's target (a
    /// choice), or is `true` for an item without a target (a toggle).
    FollowsAction,
}

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

/// The action a menu item runs: the window's, or the application's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuAction {
    /// A window action (`win.*`).
    Window(WindowAction),
    /// An application action (`app.*`).
    Application(AppAction),
}

impl From<WindowAction> for MenuAction {
    fn from(action: WindowAction) -> Self {
        MenuAction::Window(action)
    }
}

impl From<AppAction> for MenuAction {
    fn from(action: AppAction) -> Self {
        MenuAction::Application(action)
    }
}

impl MenuAction {
    /// The name a row runs the action by, such as `win.sort`.
    fn detailed_name(self) -> String {
        match self {
            MenuAction::Window(action) => action.detailed_name(),
            MenuAction::Application(action) => action.detailed_name(),
        }
    }

    /// The action's state as `widget`'s window sees it.
    fn state(self, widget: &gtk::Widget) -> Option<glib::Variant> {
        let window = widget.root().and_downcast::<gtk::ApplicationWindow>()?;
        match self {
            MenuAction::Window(action) => window.action_state(action.name()),
            MenuAction::Application(action) => window.application()?.action_state(action.name()),
        }
    }
}

/// One menu item.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MenuItem {
    /// The visible and accessible name.
    pub label: String,
    /// The glyph before the label.
    pub glyph: Icon,
    /// The action it runs.
    pub action: MenuAction,
    /// The action's parameter.
    pub target: Option<glib::Variant>,
    /// The keyboard shortcut shown at the right, such as `Ctrl+N`.
    pub shortcut: Option<&'static str>,
    /// How the item shows that it is chosen.
    pub check: ItemCheck,
}

impl MenuItem {
    /// An item that runs `action` without a parameter.
    pub(super) fn new(label: &str, glyph: Icon, action: impl Into<MenuAction>) -> Self {
        Self {
            label: label.to_owned(),
            glyph,
            action: action.into(),
            target: None,
            shortcut: None,
            check: ItemCheck::Plain,
        }
    }

    /// A choice of the string action `action`, checked while it is chosen.
    pub(super) fn choice(label: &str, glyph: Icon, action: WindowAction, value: &str) -> Self {
        Self {
            target: Some(value.to_variant()),
            check: ItemCheck::FollowsAction,
            ..Self::new(label, glyph, action)
        }
    }

    /// An item for the boolean action `action`, checked while it is on.
    pub(super) fn toggle(label: &str, glyph: Icon, action: WindowAction) -> Self {
        Self {
            check: ItemCheck::FollowsAction,
            ..Self::new(label, glyph, action)
        }
    }

    /// The same item showing `shortcut`.
    pub(super) fn with_shortcut(self, shortcut: &'static str) -> Self {
        Self {
            shortcut: Some(shortcut),
            ..self
        }
    }
}

/// A menu line: an item or a divider.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum MenuEntry {
    /// A clickable item.
    Item(MenuItem),
    /// A thin line between groups.
    Divider,
}

impl From<MenuItem> for MenuEntry {
    fn from(item: MenuItem) -> Self {
        MenuEntry::Item(item)
    }
}

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::MenuEntry;

    /// Private state of [`super::MenuPopover`].
    #[derive(Debug, Default)]
    pub struct MenuPopover {
        /// The rows, built by `constructed`.
        pub(super) list: OnceCell<gtk::ListBox>,
        /// What the rows show, dividers included.
        pub(super) entries: RefCell<Vec<MenuEntry>>,
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
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .activate_on_single_click(true)
                .accessible_role(gtk::AccessibleRole::Menu)
                .build();
            // A divider is the header of the row after it, so the keyboard
            // never lands on it. GTK clears the headers of a list without a
            // header function, so rows only carry a class.
            list.set_header_func(|row, _| {
                let divider = row
                    .has_css_class(super::AFTER_DIVIDER)
                    .then(|| gtk::Separator::new(gtk::Orientation::Horizontal));
                row.set_header(divider.as_ref());
            });
            // A row runs its action itself; the menu then closes, as
            // `closeMenu()` before `it.fn()` in app.js.
            list.connect_row_activated(glib::clone!(
                #[weak(rename_to = popover)]
                self.obj(),
                move |_, _| popover.popdown()
            ));
            popover.set_child(Some(&list));
            self.list.set(list).expect("constructed runs once per object");
            // Check marks follow the actions' state when the menu opens.
            popover.connect_show(super::MenuPopover::redraw);
        }
    }

    impl WidgetImpl for MenuPopover {}

    impl PopoverImpl for MenuPopover {}
}

glib::wrapper! {
    /// A drop-down menu of [`MenuEntry`] rows.
    pub struct MenuPopover(ObjectSubclass<imp::MenuPopover>)
        @extends gtk::Popover, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::ShortcutManager;
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

    fn list(&self) -> &gtk::ListBox {
        self.imp().list.get().expect("constructed builds the list")
    }

    /// Rebuilds the rows, reading each action's state for its check mark.
    fn redraw(&self) {
        let list = self.list();
        list.remove_all();
        let mut after_divider = false;
        for entry in self.imp().entries.borrow().iter() {
            let MenuEntry::Item(item) = entry else {
                after_divider = true;
                continue;
            };
            let row = item_row(item, self.check_mark(item));
            if after_divider {
                row.add_css_class(AFTER_DIVIDER);
                after_divider = false;
            }
            list.append(&row);
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

    /// The labels of the rows, a divider as `-`, for tests.
    #[cfg(test)]
    pub fn row_labels(&self) -> Vec<String> {
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
    #[cfg(test)]
    pub fn rows(&self) -> Vec<gtk::ListBoxRow> {
        super::widget_tree::children(self.list())
            .filter_map(|child| child.downcast::<gtk::ListBoxRow>().ok())
            .collect()
    }

    /// The labels of the rows showing a check mark, for tests.
    #[cfg(test)]
    pub fn checked_labels(&self) -> Vec<String> {
        let checked = self.rows().into_iter().filter(|row| row.has_css_class("checked"));
        checked.filter_map(|row| row_label(&row)).collect()
    }
}

/// A row's glyph (the check mark while checked, as app.js draws it), its
/// label and its shortcut.
fn item_content(item: &MenuItem, check: CheckMark) -> gtk::Box {
    let glyph = if check == CheckMark::Checked {
        Icon::Checkmark
    } else {
        item.glyph
    };
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    content.append(&icons::image(glyph, ROW_GLYPH));
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
        .action_name(item.action.detailed_name())
        .build();
    row.update_property(&[gtk::accessible::Property::Label(&item.label)]);
    row.set_action_target_value(item.target.as_ref());
    // A disabled item says which milestone brings it.
    if let MenuAction::Window(action) = item.action {
        if unported::is_unported(action) {
            row.set_tooltip_text(Some(&unported::tooltip(action, &item.label)));
        }
    }
    if let Some(state) = check.accessible_state() {
        row.update_state(&[gtk::accessible::State::Checked(state)]);
    }
    if check == CheckMark::Checked {
        row.add_css_class("checked");
    }
    row
}

/// The label of `row`, for tests.
#[cfg(test)]
fn row_label(row: &gtk::ListBoxRow) -> Option<String> {
    let content = row.child()?;
    let glyph = content.first_child()?;
    let label = glyph.next_sibling().and_downcast::<gtk::Label>()?;
    Some(label.text().to_string())
}
