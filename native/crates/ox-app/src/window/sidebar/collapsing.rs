// SPDX-License-Identifier: AGPL-3.0-only
//! Collapsing This PC and Network, as Windows Explorer's navigation pane
//! does (SIDE-033): a click on a section's chevron, or Left and Right on
//! its head row, hides the rows inside it or shows them again. A collapsed
//! section stays collapsed while the rows are rebuilt; it is kept per
//! window, not saved. While the place that holds the current location is
//! inside a collapsed section, the section's head is highlighted instead.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::entries::{RowLevel, Section, SidebarEntry};

/// How long a drag must stay over a collapsed section's head before it
/// expands (DND-027).
const DRAG_EXPAND_DELAY: std::time::Duration = std::time::Duration::from_millis(750);
use super::{row, Sidebar};

impl Sidebar {
    /// Whether `section` is collapsed.
    pub(super) fn is_collapsed(&self, section: Section) -> bool {
        self.imp().collapsed.borrow().contains(&section)
    }

    /// Collapses the section whose key is `key` (This PC or Network), or
    /// expands it again, as its chevron does.
    pub(in crate::window) fn toggle_section(&self, key: &str) {
        let section = self
            .imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.level == RowLevel::Group && section_key(entry.section) == Some(key))
            .map(|entry| entry.section);
        if let Some(section) = section {
            self.collapse_section(section, !self.is_collapsed(section));
        }
    }

    /// Hides the rows inside `section` when `collapsed`, or shows them,
    /// turns its head's chevron to match and highlights the place for the
    /// location again, which may now be the head.
    fn collapse_section(&self, section: Section, collapsed: bool) {
        {
            let mut sections = self.imp().collapsed.borrow_mut();
            sections.retain(|shown| *shown != section);
            if collapsed {
                sections.push(section);
            }
        }
        {
            let entries = self.imp().entries.borrow();
            let in_section = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.section == section);
            for (index, entry) in in_section {
                let Some(row) = i32::try_from(index)
                    .ok()
                    .and_then(|index| self.list().row_at_index(index))
                else {
                    continue;
                };
                match entry.level {
                    RowLevel::Place => {}
                    RowLevel::Group => row::show_expanded(&row, section, !collapsed),
                    RowLevel::Child => row.set_visible(!collapsed),
                }
            }
        }
        self.highlight_location();
    }

    /// A drag is at `y` of the list, or left it with `None`: held over the
    /// head of a collapsed section for [`DRAG_EXPAND_DELAY`], the section
    /// expands, as Windows Explorer's navigation pane expands a collapsed
    /// node under a drag. Nothing in the sidebar opens in the folder view.
    pub(in crate::window) fn expand_after_drag_hover(&self, y: Option<f64>) {
        let section = y.and_then(|y| self.collapsed_head_at(y));
        let waiting = self
            .imp()
            .drag_expand
            .borrow()
            .as_ref()
            .map(|(section, _)| *section);
        if waiting == section {
            return;
        }
        if let Some((_, timer)) = self.imp().drag_expand.take() {
            timer.remove();
        }
        let Some(section) = section else {
            return;
        };
        let timer = glib::timeout_add_local_once(
            DRAG_EXPAND_DELAY,
            glib::clone!(
                #[weak(rename_to = sidebar)]
                self,
                move || {
                    sidebar.imp().drag_expand.replace(None);
                    sidebar.collapse_section(section, false);
                }
            ),
        );
        self.imp().drag_expand.replace(Some((section, timer)));
    }

    /// The collapsed section whose head row is at `y` of the list.
    fn collapsed_head_at(&self, y: f64) -> Option<Section> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let row = self.list().row_at_y(y as i32)?;
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        let entry = entries.get(index)?;
        (entry.level == RowLevel::Group && self.is_collapsed(entry.section)).then_some(entry.section)
    }

    /// The labels of the rows shown, without those a collapsed section
    /// hides, for tests.
    #[cfg(test)]
    pub(in crate::window) fn labels_shown(&self) -> Vec<String> {
        let entries = self.imp().entries.borrow();
        entries
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                i32::try_from(*index)
                    .ok()
                    .and_then(|index| self.list().row_at_index(index))
                    .is_some_and(|row| row.is_visible())
            })
            .map(|(_, entry)| entry.label.clone())
            .collect()
    }

    /// Left collapses This PC or Network while its head row has keyboard
    /// focus, and Right expands it, as in Explorer's navigation pane and
    /// the folder tree; the chevron itself takes no focus. As in Windows
    /// 11's tree, Right on an expanded head moves to its first row, and
    /// Left on a row inside This PC or Network moves to the head.
    pub(super) fn collapse_sections_with_arrow_keys(&self, list: &gtk::ListBox) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if !modifiers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let expand = match key {
                    gdk::Key::Right | gdk::Key::KP_Right => true,
                    gdk::Key::Left | gdk::Key::KP_Left => false,
                    _ => return glib::Propagation::Proceed,
                };
                if sidebar.expand_focused_section(expand) || sidebar.step_through_tree(expand) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        list.add_controller(keys);
    }

    /// Expands, or collapses, the section whose head row has keyboard
    /// focus; false when another row has it or the section already is.
    fn expand_focused_section(&self, expand: bool) -> bool {
        let focused = self.list().focus_child().and_downcast::<gtk::ListBoxRow>();
        let Some(section) = focused.and_then(|row| self.head_section(&row)) else {
            return false;
        };
        if self.is_collapsed(section) != expand {
            return false;
        }
        self.collapse_section(section, !expand);
        true
    }

    /// Moves keyboard focus as Windows' tree does once a section is as
    /// the key asks: `into` (Right) from an expanded head to its first
    /// row, else (Left) from a row inside a section to its head; false
    /// where there is nowhere to go.
    fn step_through_tree(&self, into: bool) -> bool {
        let Some(focused) = self.list().focus_child().and_downcast::<gtk::ListBoxRow>() else {
            return false;
        };
        let Ok(index) = usize::try_from(focused.index()) else {
            return false;
        };
        let target = {
            let entries = self.imp().entries.borrow();
            let Some(entry) = entries.get(index) else {
                return false;
            };
            match (into, entry.level) {
                (true, RowLevel::Group) if !self.is_collapsed(entry.section) => entries
                    .get(index + 1)
                    .filter(|next| next.section == entry.section && next.level == RowLevel::Child)
                    .map(|_| index + 1),
                (false, RowLevel::Child) => entries[..index]
                    .iter()
                    .rposition(|head| head.level == RowLevel::Group && head.section == entry.section),
                _ => None,
            }
        };
        let row = target
            .and_then(|target| i32::try_from(target).ok())
            .and_then(|target| self.list().row_at_index(target));
        row.is_some_and(|row| row.grab_focus())
    }

    /// The section `row` is the head of, `None` for any other row.
    fn head_section(&self, row: &gtk::ListBoxRow) -> Option<Section> {
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        let entry = entries.get(index)?;
        (entry.level == RowLevel::Group).then_some(entry.section)
    }

    /// The row that shows the place at `index` of `entries` highlighted:
    /// the head of its section while that is collapsed, else its own.
    pub(super) fn shown_holder(&self, entries: &[SidebarEntry], index: usize) -> usize {
        shown_row(entries, &self.imp().collapsed.borrow(), index)
    }

    /// Whether the section whose key is `key` is collapsed, for tests.
    #[cfg(test)]
    pub(in crate::window) fn section_is_collapsed(&self, key: &str) -> bool {
        let collapsed = self.imp().collapsed.borrow();
        collapsed.iter().any(|section| section_key(*section) == Some(key))
    }
}

/// The key `section` is saved and named by (`thisPc`, `network`).
fn section_key(section: Section) -> Option<&'static str> {
    section.hiding().map(|(key, _)| key)
}

/// The row of `entries` that shows the entry at `index`: the head of its
/// section while `collapsed` lists the section, else the entry's own.
fn shown_row(entries: &[SidebarEntry], collapsed: &[Section], index: usize) -> usize {
    let Some(entry) = entries.get(index) else {
        return index;
    };
    if entry.level != RowLevel::Child || !collapsed.contains(&entry.section) {
        return index;
    }
    entries
        .iter()
        .position(|head| head.level == RowLevel::Group && head.section == entry.section)
        .unwrap_or(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::{Art, Icon};
    use crate::window::sidebar::entries::RowTarget;

    fn entry(section: Section, level: RowLevel, label: &str) -> SidebarEntry {
        SidebarEntry {
            section,
            level,
            label: label.to_owned(),
            icon: Art::Glyph(Icon::Folder),
            target: RowTarget::Location(format!("file:///{label}")),
            tooltip: label.to_owned(),
            pinned: false,
            menu: None,
            eject: None,
        }
    }

    /// A place inside a collapsed section shows as its section's head;
    /// once the section is expanded, or for a place outside it, as itself.
    ///
    /// parity: SIDE-033
    #[test]
    fn a_place_in_a_collapsed_section_shows_as_its_head() {
        let entries = [
            entry(Section::Home, RowLevel::Place, "Home"),
            entry(Section::ThisPc, RowLevel::Group, "This PC"),
            entry(Section::ThisPc, RowLevel::Child, "Local Disk"),
            entry(Section::Network, RowLevel::Group, "Network"),
            entry(Section::Network, RowLevel::Child, "studio-nas"),
        ];
        let collapsed = [Section::ThisPc];
        assert_eq!(
            shown_row(&entries, &collapsed, 2),
            1,
            "Local Disk shows as This PC"
        );
        assert_eq!(shown_row(&entries, &collapsed, 1), 1);
        assert_eq!(shown_row(&entries, &collapsed, 4), 4, "Network is expanded");
        assert_eq!(shown_row(&entries, &collapsed, 0), 0);
        assert_eq!(shown_row(&entries, &[], 2), 2, "nothing collapsed");
    }
}
