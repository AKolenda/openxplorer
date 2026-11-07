// SPDX-License-Identifier: AGPL-3.0-only
//! Collapsing and expanding a grouped folder's groups from their headings
//! (VIEW-022), in Details, Tiles and Content.
//!
//! Windows 11 heads each group with a chevron that collapses the group
//! under its heading and expands it again. With the keyboard on a heading,
//! Left collapses its group and Right expands it; Enter and Space toggle
//! it. The heading's right-click menu (or Shift+F10 and the Menu key on
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
    if let Some(cell) = ancestors().find_map(|widget| widget.downcast::<FileCell>().ok()) {
        return cell.heading();
    }
    let item = ancestors()
        .find(|widget| ["row", "child"].contains(&widget.css_name().as_str()))
        .unwrap_or_else(|| widget.clone());
    super::widget_tree::descendants::<FileCell>(&item)
        .into_iter()
        .find_map(|cell| cell.heading())
}

/// The heading a click landed on at (`x`, `y`) in `view`.
pub(super) fn heading_at(view: &gtk::Widget, x: f64, y: f64) -> Option<GroupSlot> {
    let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
    heading_near(&picked)
}

/// What a heading key does to its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadingKey {
    Collapse,
    Expand,
    Toggle,
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
            | gdk::Key::KP_Space => HeadingKey::Toggle,
            _ => return None,
        })
    }

    /// Whether the group is collapsed after the key, from `collapsed`.
    fn collapses(self, collapsed: bool) -> bool {
        match self {
            HeadingKey::Collapse => true,
            HeadingKey::Expand => false,
            HeadingKey::Toggle => !collapsed,
        }
    }
}

/// The heading menu's entries for a group that is `collapsed`: that
/// group's command, then both commands for every group.
fn heading_menu(collapsed: bool) -> Vec<MenuEntry> {
    let own = if collapsed {
        MenuItem::new(
            gettext_static("Expand group"),
            Icon::ChevronDown16,
            WindowAction::ExpandGroup,
        )
    } else {
        MenuItem::new(
            gettext_static("Collapse group"),
            Icon::ChevronRight16,
            WindowAction::CollapseGroup,
        )
    };
    vec![
        own.into(),
        MenuEntry::Divider,
        MenuItem::new(
            gettext_static("Collapse all groups"),
            Icon::ChevronRight16,
            WindowAction::CollapseAllGroups,
        )
        .into(),
        MenuItem::new(
            gettext_static("Expand all groups"),
            Icon::ChevronDown16,
            WindowAction::ExpandAllGroups,
        )
        .into(),
    ]
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
    /// collapse, expand or toggle its group. `None` for every other key,
    /// or with no heading focused.
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
        heading.set_group_collapsed(effect.collapses(heading.collapsed()));
        self.keep_heading_focus(&heading);
        Some(glib::Propagation::Stop)
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

    /// The heading menu's entries for `heading`, which its commands then
    /// act on.
    pub(super) fn heading_menu_for(&self, heading: &GroupSlot) -> Vec<MenuEntry> {
        self.imp().menu_heading.replace(Some(heading.clone()));
        heading_menu(heading.collapsed())
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

    /// Adds the heading menu's commands.
    pub(super) fn install_group_heading_actions(&self) {
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

    /// Left collapses, Right expands, Enter and Space toggle, as Windows
    /// 11's group headings do.
    ///
    /// parity: VIEW-022
    #[test]
    fn heading_keys_collapse_expand_and_toggle() {
        assert_eq!(HeadingKey::of(gdk::Key::Left), Some(HeadingKey::Collapse));
        assert_eq!(HeadingKey::of(gdk::Key::Right), Some(HeadingKey::Expand));
        assert_eq!(HeadingKey::of(gdk::Key::Return), Some(HeadingKey::Toggle));
        assert_eq!(HeadingKey::of(gdk::Key::space), Some(HeadingKey::Toggle));
        assert_eq!(HeadingKey::of(gdk::Key::Down), None);
        assert!(HeadingKey::Collapse.collapses(true), "Left keeps it collapsed");
        assert!(!HeadingKey::Expand.collapses(false));
        assert!(HeadingKey::Toggle.collapses(false));
        assert!(!HeadingKey::Toggle.collapses(true));
    }
}
