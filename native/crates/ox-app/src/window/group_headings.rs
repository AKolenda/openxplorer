// SPDX-License-Identifier: AGPL-3.0-only
//! Collapsing and expanding a grouped folder's groups from their headings
//! (VIEW-022), in Details, Tiles and Content.
//!
//! Windows 11 heads each group with a chevron that collapses the group
//! under its heading and expands it again. A click on the chevron does
//! that at once; a click anywhere else on the heading's row selects every
//! item of the group (Ctrl adds the group or takes it out, Shift extends
//! the selection to it), and a double-click collapses or expands it, as
//! Windows' list view does (`LVGS_SELECTED`; `LVHT_EX_GROUP_COLLAPSE` apart
//! from `LVHT_EX_GROUP_HEADER`). With the keyboard on a heading, Left
//! collapses its group and Right expands it; Space and Enter select its
//! items. The heading's right-click menu (or Shift+F10 and the Menu key on
//! it) collapses or expands that group, or every group. A collapsed
//! group's items keep their selection, and Select all still selects them;
//! the arrows, Shift ranges and type-to-select pass them by.

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::i18n::gettext_static;

use super::actions::plain_action;
use super::menu_popover::{MenuEntry, MenuItem};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::cells::FileCell;
use crate::folder_view::grid::GroupSlot;
use crate::icons::Icon;

/// The group heading `widget` shows or is inside: the heading cell, the
/// row or tile around it, which holds the keyboard, or another cell of a
/// Details heading row, which carries its line.
pub(super) fn heading_near(widget: &gtk::Widget) -> Option<GroupSlot> {
    let ancestors = || {
        std::iter::successors(Some(widget.clone()), WidgetExt::parent)
            .take_while(|widget| !widget.is::<gtk::ListBase>())
    };
    let row_heading = |cell: FileCell| cell.slot().and_then(|slot| slot.row_heading());
    if let Some(cell) = ancestors().find_map(|widget| widget.downcast::<FileCell>().ok()) {
        return row_heading(cell);
    }
    let item = ancestors()
        .find(|widget| ["row", "child"].contains(&widget.css_name().as_str()))
        .unwrap_or_else(|| widget.clone());
    super::widget_tree::descendants::<FileCell>(&item)
        .into_iter()
        .find_map(row_heading)
}

/// Whether `widget`, picked under the pointer, is inside a widget with
/// the CSS class `class`, within its row.
fn is_inside(widget: &gtk::Widget, class: &str) -> bool {
    std::iter::successors(Some(widget.clone()), WidgetExt::parent)
        .take_while(|widget| !widget.is::<gtk::ListBase>())
        .any(|widget| widget.has_css_class(class))
}

/// Where a pointer at (`x`, `y`) in `view` is on a group heading's row.
#[derive(Debug, Clone)]
pub(super) enum HeadingSpot {
    /// The chevron, which collapses or expands the group at once.
    Chevron(GroupSlot),
    /// The title, which selects the group's items.
    Title(GroupSlot),
    /// The rest of the row, which Windows 11 leaves alone.
    Rest,
}

impl HeadingSpot {
    /// The heading the pointer acts on, when it acts on one.
    pub(super) fn heading(&self) -> Option<&GroupSlot> {
        match self {
            HeadingSpot::Chevron(heading) | HeadingSpot::Title(heading) => Some(heading),
            HeadingSpot::Rest => None,
        }
    }
}

/// Where (`x`, `y`) in `view` is on a heading's row, `None` off one.
pub(super) fn heading_spot(view: &gtk::Widget, x: f64, y: f64) -> Option<HeadingSpot> {
    let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
    let heading = heading_near(&picked)?;
    Some(if is_inside(&picked, "group-chevron") {
        HeadingSpot::Chevron(heading)
    } else if is_inside(&picked, "group-heading-hit") {
        HeadingSpot::Title(heading)
    } else {
        HeadingSpot::Rest
    })
}

/// The heading whose row (`x`, `y`) in `view` is on, whatever part.
pub(super) fn heading_at(view: &gtk::Widget, x: f64, y: f64) -> Option<GroupSlot> {
    let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
    heading_near(&picked)
}

/// What a heading key does to its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadingKey {
    Collapse,
    Expand,
    Select,
}

impl HeadingKey {
    /// The effect of `key` on a heading with the keyboard, if it has one.
    fn of(key: gdk::Key) -> Option<Self> {
        Some(match key {
            gdk::Key::Left | gdk::Key::KP_Left => HeadingKey::Collapse,
            gdk::Key::Right | gdk::Key::KP_Right => HeadingKey::Expand,
            gdk::Key::Return
            | gdk::Key::KP_Enter
            | gdk::Key::ISO_Enter
            | gdk::Key::space
            | gdk::Key::KP_Space => HeadingKey::Select,
            _ => return None,
        })
    }

    /// Whether the group is collapsed after the key, or `None` for a key
    /// that selects the group's items.
    fn collapses(self) -> Option<bool> {
        match self {
            HeadingKey::Collapse => Some(true),
            HeadingKey::Expand => Some(false),
            HeadingKey::Select => None,
        }
    }
}

/// Collapse all groups and Expand all groups, as the heading menu and the
/// folder menu's View list them.
fn all_groups_entries() -> [MenuEntry; 2] {
    [
        MenuItem::new(
            gettext_static("Expand all groups"),
            Icon::ChevronDown16,
            WindowAction::ExpandAllGroups,
        )
        .into(),
        MenuItem::new(
            gettext_static("Collapse all groups"),
            Icon::ChevronRight16,
            WindowAction::CollapseAllGroups,
        )
        .into(),
    ]
}

/// The entries a heading's menu starts with, for a group that is
/// `collapsed`, as Windows 11 lists them above the menu of the group's
/// items: Collapse group (Left) or Expand group (Right), Expand all groups
/// and Collapse all groups.
fn heading_entries(collapsed: bool) -> Vec<MenuEntry> {
    let own = if collapsed {
        MenuItem::new(
            gettext_static("Expand group"),
            Icon::ChevronDown16,
            WindowAction::ExpandGroup,
        )
        .with_shortcut("Right")
    } else {
        MenuItem::new(
            gettext_static("Collapse group"),
            Icon::ChevronRight16,
            WindowAction::CollapseGroup,
        )
        .with_shortcut("Left")
    };
    let mut entries = vec![own.into()];
    entries.extend(all_groups_entries());
    entries.push(MenuEntry::Divider);
    entries
}

impl BrowserWindow {
    /// The group heading with the keyboard in the active pane, if one has
    /// it.
    pub(super) fn focused_heading(&self) -> Option<GroupSlot> {
        let focus = GtkWindowExt::focus(self)?;
        let view = self.folder_pane().input_widget();
        focus.is_ancestor(&view).then(|| heading_near(&focus))?
    }

    /// Left, Right, Enter and Space on a group heading with the keyboard:
    /// collapse or expand its group, or select its items. `None` for
    /// every other key, or with no heading focused.
    pub(super) fn group_heading_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<glib::Propagation> {
        let effect = HeadingKey::of(key)?;
        let held = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::META_MASK;
        if modifiers.intersects(held) {
            return None;
        }
        let heading = self.focused_heading()?;
        self.reset_typeahead();
        match effect.collapses() {
            Some(collapsed) => heading.set_group_collapsed(collapsed),
            None => {
                if let Some((model, index)) = heading.model().zip(heading.group_index()) {
                    model.select_group(index, false, false);
                }
            }
        }
        self.keep_heading_focus(&heading);
        Some(glib::Propagation::Stop)
    }

    /// A press on a group heading's row at (`x`, `y`) in `view`, the
    /// `presses`th of a click, as Windows 11 takes it: on the chevron it
    /// collapses or expands the group at once, without selecting; on the
    /// title the first press selects the group's items (Ctrl adds them or
    /// takes them out, Shift extends the selection to them) and gives the
    /// heading the keyboard, and a double-click collapses or expands the
    /// group. The rest of the row takes no click. Returns false when the
    /// press is not on a heading's row.
    pub(super) fn press_heading(
        &self,
        view: &gtk::Widget,
        x: f64,
        y: f64,
        presses: i32,
        modifiers: gdk::ModifierType,
    ) -> bool {
        let Some(spot) = heading_spot(view, x, y) else {
            return false;
        };
        self.reset_typeahead();
        match spot {
            HeadingSpot::Chevron(heading) if presses == 1 => heading.toggle_group(),
            HeadingSpot::Title(heading) if presses == 1 => {
                if let Some((model, index)) = heading.model().zip(heading.group_index()) {
                    let add = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
                    let extend = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                    model.select_group(index, add, extend);
                }
                self.keep_heading_focus(&heading);
            }
            HeadingSpot::Title(heading) if presses == 2 => heading.toggle_group(),
            _ => {}
        }
        true
    }

    /// Gives the keyboard back to `heading` after its group changed: the
    /// rows under it were added or removed, which may move the focus.
    pub(super) fn keep_heading_focus(&self, heading: &GroupSlot) {
        let (Some(model), Some(index)) = (heading.model(), heading.group_index()) else {
            return;
        };
        let Some(slot) = model.layout().and_then(|layout| layout.heading_slot(index)) else {
            return;
        };
        let pane = self.folder_pane();
        let flags = gtk::ListScrollFlags::FOCUS;
        if pane.details().grouped().as_ref() == Some(&model) {
            pane.details().column_view().scroll_to(slot, None, flags, None);
        } else if pane.icon_view().grouped().as_ref() == Some(&model) {
            pane.icon_view().grid().scroll_to(slot, flags, None);
        }
    }

    /// The folder menu's View: the command bar's, then Collapse all groups
    /// and Expand all groups while the view heads the folder's groups, as
    /// Windows 11's folder menu has them.
    pub(super) fn background_view_menu(&self) -> Vec<MenuEntry> {
        let mut entries = super::command_bar::view_menu();
        if self.folder_pane().grouped().is_some() {
            entries.push(MenuEntry::Divider);
            entries.extend(all_groups_entries());
        }
        entries
    }

    /// The entries a menu opened on `heading` starts with, which its
    /// commands then act on.
    pub(super) fn heading_menu_for(&self, heading: &GroupSlot) -> Vec<MenuEntry> {
        self.imp().menu_heading.replace(Some(heading.clone()));
        heading_entries(heading.collapsed())
    }

    /// The heading the menu's commands act on: the one it opened on, else
    /// the one with the keyboard.
    fn menu_heading(&self) -> Option<GroupSlot> {
        self.imp()
            .menu_heading
            .borrow()
            .clone()
            .filter(|heading| heading.model().is_some())
            .or_else(|| self.focused_heading())
    }

    /// Collapses the menu's group, or expands it with `collapsed` false.
    fn set_menu_group_collapsed(&self, collapsed: bool) {
        if let Some(heading) = self.menu_heading() {
            heading.set_group_collapsed(collapsed);
        }
    }

    /// Collapses every group of the active pane's grouped view, or
    /// expands every one with `collapsed` false.
    fn set_all_groups_collapsed(&self, collapsed: bool) {
        let grouped = self
            .menu_heading()
            .and_then(|heading| heading.model())
            .or_else(|| self.folder_pane().grouped());
        if let Some(grouped) = grouped {
            grouped.set_all_collapsed(collapsed);
        }
    }

    /// Rings the whole row of the heading that has the keyboard, in Tiles
    /// and Content, whose rows are several slots (Details rings its row by
    /// itself), and takes the ring off the heading that had it. Followed
    /// from the window's focus, later than GTK moves it: restyling the
    /// slots while GTK binds or moves them upsets its lists.
    fn follow_heading_focus(&self) {
        self.connect_focus_widget_notify(|window| {
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                window,
                move || {
                    let now = window.focused_heading();
                    let before = window.imp().focused_row.replace(now.clone());
                    if before == now {
                        return;
                    }
                    if let Some(before) = before {
                        before.set_row_focused(false);
                    }
                    if let Some(now) = now {
                        now.set_row_focused(true);
                    }
                }
            ));
        });
    }

    /// Adds the heading menu's commands.
    pub(super) fn install_group_heading_actions(&self) {
        self.follow_heading_focus();
        self.add_action_entries([
            plain_action(WindowAction::CollapseGroup, |window| {
                window.set_menu_group_collapsed(true);
            }),
            plain_action(WindowAction::ExpandGroup, |window| {
                window.set_menu_group_collapsed(false);
            }),
            plain_action(WindowAction::CollapseAllGroups, |window| {
                window.set_all_groups_collapsed(true);
            }),
            plain_action(WindowAction::ExpandAllGroups, |window| {
                window.set_all_groups_collapsed(false);
            }),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Left collapses, Right expands, Enter and Space select the group's
    /// items, as on Windows 11's group headings.
    ///
    /// parity: VIEW-022
    #[test]
    fn heading_keys_collapse_expand_and_select() {
        assert_eq!(HeadingKey::of(gdk::Key::Left), Some(HeadingKey::Collapse));
        assert_eq!(HeadingKey::of(gdk::Key::Right), Some(HeadingKey::Expand));
        assert_eq!(HeadingKey::of(gdk::Key::Return), Some(HeadingKey::Select));
        assert_eq!(HeadingKey::of(gdk::Key::space), Some(HeadingKey::Select));
        assert_eq!(HeadingKey::of(gdk::Key::Down), None);
        assert_eq!(HeadingKey::Collapse.collapses(), Some(true));
        assert_eq!(HeadingKey::Expand.collapses(), Some(false));
        assert_eq!(HeadingKey::Select.collapses(), None);
    }

    /// A heading's menu starts with its group's command, its shortcut
    /// shown, then Expand all groups and Collapse all groups, above the
    /// menu of the group's items, as Windows 11's.
    ///
    /// parity: VIEW-022
    #[test]
    fn a_heading_menu_starts_with_the_group_commands() {
        use crate::window::menu_popover::MenuAction;
        let actions = |collapsed| {
            heading_entries(collapsed)
                .into_iter()
                .map(|entry| match entry {
                    MenuEntry::Item(item) => Some((item.action, item.shortcut)),
                    MenuEntry::Divider => None,
                })
                .collect::<Vec<_>>()
        };
        let window = |action| MenuAction::Window(action);
        assert_eq!(
            actions(false),
            [
                Some((window(WindowAction::CollapseGroup), Some("Left"))),
                Some((window(WindowAction::ExpandAllGroups), None)),
                Some((window(WindowAction::CollapseAllGroups), None)),
                None,
            ]
        );
        assert_eq!(
            actions(true).first().copied().flatten(),
            Some((window(WindowAction::ExpandGroup), Some("Right")))
        );
    }
}
