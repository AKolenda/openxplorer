// SPDX-License-Identifier: AGPL-3.0-only
//! The list under the editable address, which app.js lacked: the
//! addresses typed before (NAV-043) and completions of what is being typed
//! (NAV-030), as Explorer's address bar and Dolphin's location combo box
//! offer them.
//!
//! Every typed address applied with Enter goes to the top of a most-recent-first
//! history of the window, moving up if it was there, and the oldest drop
//! off past [`HISTORY_LENGTH`]. F4 or the chevron edits the address with
//! the history listed below; Alt+Down lists it while editing. The window
//! hears typed text through [`AddressBar::connect_typed`] and answers with
//! [`AddressBar::show_completions`]. Up and Down move through the list and
//! put the row into the entry, Enter goes there, a click on a row goes
//! there at once, and Escape puts back the typed text and closes the list
//! before it cancels editing.
//!
//! The keys follow Windows' autocomplete list (the shell's `IAutoComplete`
//! drop-down, as `ReactOS`'s `CAutoComplete::OnListUpDown` reimplements it):
//! Down with no row selected selects the first and Up the last; Down on
//! the last row and Up on the first leave the list with no row selected
//! and the typed text back in the entry, so the next key wraps round.
//! The selected row has Windows 11's list selection look, a light fill
//! and a short accent bar at its left edge; the pointer's hover draws the
//! fill alone (`resources/skin/menus.css`).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::{AddressBar, AddressMode};

/// The most typed addresses the history keeps.
pub(crate) const HISTORY_LENGTH: usize = 20;

/// The most rows the list shows.
const LIST_LENGTH: usize = 12;

/// Puts `address` at the top of `history`, once, keeping
/// [`HISTORY_LENGTH`] entries. An address with a user name or password is
/// never kept (NAV-035 refuses it anyway).
pub(super) fn remember(history: &mut Vec<String>, address: &str) {
    let address = address.trim();
    let authority = address
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest));
    if address.is_empty() || authority.is_some_and(|authority| authority.contains('@')) {
        return;
    }
    history.retain(|kept| kept != address);
    history.insert(0, address.to_owned());
    history.truncate(HISTORY_LENGTH);
}

impl AddressBar {
    /// Builds the list and connects the entry's keys, Enter and focus.
    pub(super) fn add_suggestions(&self) {
        let imp = self.imp();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .css_classes(["address-suggestions"])
            .build();
        list.connect_row_activated(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_, row| {
                if let Some(text) = row_text(row) {
                    bar.submit_text(&text);
                }
            }
        ));
        let popover = gtk::Popover::builder()
            .autohide(false)
            .has_arrow(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .child(&list)
            .css_classes(["ox-menu"])
            .build();
        popover.set_parent(self);
        imp.suggestion_list.set(list).expect("add_suggestions runs once");
        imp.suggestion_popover
            .set(popover)
            .expect("add_suggestions runs once");
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| bar.suggestion_key(key, modifiers)
        ));
        imp.entry.add_controller(keys);
        imp.entry.connect_changed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |entry| {
                if !bar.imp().quiet_change.get() {
                    bar.imp().text_typed.replace(entry.text().into());
                }
            }
        ));
        imp.entry.connect_activate(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| bar.hide_suggestions()
        ));
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| bar.hide_suggestions()
        ));
        imp.entry.add_controller(focus);
    }

    /// Up, Down and Escape while the list shows; Alt+Down lists the
    /// history.
    fn suggestion_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        let down = matches!(key, gdk::Key::Down | gdk::Key::KP_Down);
        if alt && down {
            self.show_history();
            return glib::Propagation::Stop;
        }
        if !self.suggestions_shown() || !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Down | gdk::Key::KP_Down => self.move_suggestion(1),
            gdk::Key::Up | gdk::Key::KP_Up => self.move_suggestion(-1),
            gdk::Key::Escape => {
                let typed = self.imp().text_typed.borrow().clone();
                self.set_text_quietly(&typed);
                self.hide_suggestions();
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// Selects the row `step` rows on and puts it into the entry. Past
    /// either end no row is selected and the typed text comes back, as
    /// Windows' autocomplete list does.
    fn move_suggestion(&self, step: i32) {
        let Some(list) = self.imp().suggestion_list.get() else {
            return;
        };
        let count = i32::try_from(row_count(list)).unwrap_or(i32::MAX);
        let current = list.selected_row().map(|row| row.index());
        if let Some(row) = next_index(current, step, count).and_then(|next| list.row_at_index(next)) {
            list.select_row(Some(&row));
            if let Some(text) = row_text(&row) {
                self.set_text_quietly(&text);
            }
        } else {
            list.unselect_all();
            let typed = self.imp().text_typed.borrow().clone();
            self.set_text_quietly(&typed);
        }
    }

    /// Puts `text` into the entry, cursor at the end, without asking for
    /// completions.
    fn set_text_quietly(&self, text: &str) {
        let imp = self.imp();
        imp.quiet_change.set(true);
        imp.entry.set_text(text);
        imp.entry.set_position(-1);
        imp.quiet_change.set(false);
    }

    /// Calls `on_typed` with the entry's text whenever the user changes it.
    pub(in crate::window) fn connect_typed(&self, on_typed: impl Fn(&str) + 'static) {
        self.imp().entry.connect_changed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |entry| {
                let typing = entry.focus_child().is_some() && bar.mode() == AddressMode::Entry;
                if typing && !bar.imp().quiet_change.get() {
                    on_typed(entry.text().as_str());
                }
            }
        ));
    }

    /// Adds `address`, just applied, to the typed history.
    pub(in crate::window) fn remember_typed(&self, address: &str) {
        remember(&mut self.imp().typed_history.borrow_mut(), address);
    }

    /// The entry's text.
    pub(in crate::window) fn typed_text(&self) -> String {
        self.imp().entry.text().into()
    }

    /// Whether the entry shows and has keyboard focus.
    pub(in crate::window) fn is_typing(&self) -> bool {
        self.mode() == AddressMode::Entry && self.imp().entry.focus_child().is_some()
    }

    /// Lists `completions` of `typed`, unless the entry has moved on.
    pub(in crate::window) fn show_completions(&self, typed: &str, completions: &[String]) {
        if self.imp().entry.text() != typed {
            return;
        }
        self.show_suggestions(completions);
    }

    /// Forgets the typed history (Folder Options' Clear File Explorer
    /// history).
    pub(in crate::window) fn forget_typed_history(&self) {
        self.imp().typed_history.borrow_mut().clear();
    }

    /// The typed history, newest first, for tests.
    #[cfg(test)]
    pub(in crate::window) fn typed_history(&self) -> Vec<String> {
        self.imp().typed_history.borrow().clone()
    }

    /// Edits the address with the typed history listed below it (F4 and
    /// the chevron).
    pub(in crate::window) fn show_history(&self) {
        let history = self.imp().typed_history.borrow().clone();
        self.show_suggestions(&history);
    }

    /// Lists `rows`, or closes the list when there are none.
    fn show_suggestions(&self, rows: &[String]) {
        let imp = self.imp();
        let (Some(list), Some(popover)) = (imp.suggestion_list.get(), imp.suggestion_popover.get()) else {
            return;
        };
        list.remove_all();
        for text in rows.iter().take(LIST_LENGTH) {
            list.append(&row_contents(text));
        }
        if rows.is_empty() || self.mode() != AddressMode::Entry {
            popover.popdown();
            return;
        }
        popover.set_size_request(imp.stack.width(), -1);
        popover.popup();
    }

    /// Closes the list.
    pub(in crate::window) fn hide_suggestions(&self) {
        if let Some(popover) = self.imp().suggestion_popover.get() {
            popover.popdown();
        }
    }

    /// Whether the list shows.
    pub(in crate::window) fn suggestions_shown(&self) -> bool {
        self.imp()
            .suggestion_popover
            .get()
            .is_some_and(gtk::Popover::is_visible)
    }

    /// The rows listed, for tests.
    #[cfg(test)]
    pub(in crate::window) fn suggestion_rows(&self) -> Vec<String> {
        let Some(list) = self.imp().suggestion_list.get() else {
            return Vec::new();
        };
        let rows = (0..).map_while(|index| list.row_at_index(index));
        rows.filter_map(|row| row_text(&row)).collect()
    }
}

#[cfg(test)]
impl AddressBar {
    /// The row at `index` of the list, for tests.
    pub(in crate::window) fn suggestion_row(&self, index: i32) -> gtk::ListBoxRow {
        let list = self.imp().suggestion_list.get().expect("the list is built");
        list.row_at_index(index).expect("a row there")
    }
}

/// The number of rows in `list`.
fn row_count(list: &gtk::ListBox) -> usize {
    (0..).map_while(|index| list.row_at_index(index)).count()
}

/// The row after `current` (none selected) `step` rows on in a list of
/// `count`, or none past either end, as Windows' autocomplete list moves.
fn next_index(current: Option<i32>, step: i32, count: i32) -> Option<i32> {
    if count <= 0 {
        return None;
    }
    let next = match current {
        None if step > 0 => 0,
        None => count - 1,
        Some(index) => index + step,
    };
    (0..count).contains(&next).then_some(next)
}

/// A row's contents: the accent bar the selected row shows at its left
/// edge, then the address.
fn row_contents(text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let pill = gtk::Box::builder()
        .css_classes(["pill"])
        .valign(gtk::Align::Center)
        .build();
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    row.append(&pill);
    row.append(&label);
    row
}

/// The address a row shows.
fn row_text(row: &gtk::ListBoxRow) -> Option<String> {
    let contents = row.child()?;
    let label = contents.last_child().and_downcast::<gtk::Label>()?;
    Some(label.text().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Down from no row selects the first and Up the last; past either
    /// end the selection leaves the list, so the next key wraps round.
    #[test]
    fn up_and_down_wrap_through_the_typed_text() {
        assert_eq!(next_index(None, 1, 3), Some(0));
        assert_eq!(next_index(Some(0), 1, 3), Some(1));
        assert_eq!(next_index(Some(2), 1, 3), None);
        assert_eq!(next_index(None, -1, 3), Some(2));
        assert_eq!(next_index(Some(1), -1, 3), Some(0));
        assert_eq!(next_index(Some(0), -1, 3), None);
        assert_eq!(next_index(None, 1, 0), None);
    }

    /// parity: NAV-043
    #[test]
    fn typed_addresses_go_to_the_top_once_and_the_oldest_drop_off() {
        let mut history = Vec::new();
        for number in 0..25 {
            remember(&mut history, &format!("/srv/folder {number}"));
        }
        remember(&mut history, " /srv/folder 10 ");
        remember(&mut history, "smb://demo:secret@nas/share");
        remember(&mut history, "");

        assert_eq!(history.len(), HISTORY_LENGTH);
        assert_eq!(history[0], "/srv/folder 10");
        assert_eq!(history[1], "/srv/folder 24");
        assert_eq!(
            history
                .iter()
                .filter(|kept| kept.as_str() == "/srv/folder 10")
                .count(),
            1
        );
        assert!(!history.iter().any(|kept| kept.contains('@')));
        assert!(
            !history.contains(&"/srv/folder 4".to_owned()),
            "the oldest dropped off"
        );
    }
}
