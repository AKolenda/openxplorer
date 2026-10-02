// SPDX-License-Identifier: AGPL-3.0-only
//! Keyboard and pointer input of the folder views and the address entry:
//! a new window's first keyboard focus, the keys and clicks that feed or
//! end type-to-select, a click on blank space, middle-click to open a
//! folder in a tab, and activation. The typed prefix itself lives in
//! [`super::type_to_select`], the context menu in [`super::context_menu`].
//!
//! Ports `onKey` and the type-select glue in `v2.0.0:desktop/ui/app.js`
//! (`v2.0.0:desktop/tests/ui_type_select.py` is its specification): Escape first
//! clears the prefix and only then the selection; arrows, clicks,
//! shortcuts and leaving the view start a new prefix.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::activation::{activation_for, Activation};
use super::file_drop::DropZone;
use super::folder_pane::PanePage;
use super::gestures;
use super::session::Direction;
use super::split_view::is_plain_tab;
use super::type_to_select::monotonic_now;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What screen readers call a folder view (`#main`'s `aria-label`).
const FOLDER_VIEW_LABEL: &str = crate::i18n::message_id("Folder contents — type a filename prefix to select");

/// Whether a press on blank space with `modifiers` held keeps the
/// selection: Ctrl and Shift do, for the rubber band that may follow, which
/// toggles its items with Ctrl and adds them with Shift (SEL-012).
fn press_keeps_selection(modifiers: gdk::ModifierType) -> bool {
    modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK)
}

/// How a folder opened with `modifiers` held opens (TAB-026): Ctrl in a new
/// tab behind, Ctrl+Shift in a new tab in front, Shift in a new window;
/// `None` opens it in place.
fn open_folder_action(modifiers: gdk::ModifierType) -> Option<WindowAction> {
    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    match (ctrl, shift) {
        (true, true) => Some(WindowAction::OpenTab),
        (true, false) => Some(WindowAction::OpenTabBackground),
        (false, true) => Some(WindowAction::OpenWindow),
        (false, false) => None,
    }
}

/// Keys that only modify another key; pressing one keeps the prefix, so
/// capitals and `AltGr` characters can be typed.
fn is_modifier_key(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Caps_Lock
            | gdk::Key::Shift_Lock
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::ISO_Level5_Shift
            | gdk::Key::Mode_switch
    )
}

impl BrowserWindow {
    /// Adds keyboard and pointer handling to both folder views, and makes
    /// Escape in the address entry return to the breadcrumbs.
    pub(super) fn install_input(&self) {
        for pane in self.folder_panes() {
            let details = pane.details().column_view().clone();
            let grid = pane.icon_view().grid().clone();
            self.folder_input(details.upcast_ref());
            self.folder_input(grid.upcast_ref());
        }
        self.install_selection_keys();
        self.install_slow_click_rename();
        self.address_bar().connect_cancelled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.finish_address()
        ));
        self.install_focus_regions();
    }

    /// Focuses the file list once GTK has finished showing the window,
    /// which ends by focusing the first focusable widget (see
    /// [`Self::focus_new_file_list`]).
    pub(super) fn focus_file_list_once_shown(&self) {
        self.imp().file_list_awaits_focus.set(true);
        self.connect_map(|window| {
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                window,
                move || window.focus_new_file_list()
            ));
        });
    }

    /// Gives a new window's file list keyboard focus once the window is
    /// shown and its first location is listed, as `#main` has focus when
    /// app.js starts, and again after Settings hides (see
    /// `settings_tab.rs`). A landing page or an empty folder has no list to
    /// focus, so nothing keeps focus: GTK would otherwise leave it on the
    /// first focusable widget, and a focused crumb draws the address bar's
    /// editing line.
    pub(super) fn focus_new_file_list(&self) {
        if !(self.is_mapped() && self.is_listed()) {
            return;
        }
        // Only when asked for: later listings leave focus where it is.
        let awaits_focus = self.imp().file_list_awaits_focus.replace(false);
        if !awaits_focus {
            return;
        }
        if self.folder_pane().page() == Some(PanePage::Listing) {
            self.folder_pane().focus_view();
        } else {
            GtkWindowExt::set_focus(self, None::<&gtk::Widget>);
        }
    }

    /// Opens what was typed into the address bar when Enter is pressed.
    pub(super) fn connect_address_entry(&self) {
        self.address_bar().connect_submitted(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |address| window.submit_address(address)
        ));
        self.address_bar().connect_typed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |typed| window.complete_address(typed)
        ));
    }

    /// Enter and double-click open the activated item. When it is one of
    /// several selected items, all of them open, as Dolphin's
    /// `itemsActivated` does (OPEN-003); an item outside a selection of
    /// several is never opened.
    pub(super) fn connect_view_activation(&self) {
        for pane in self.folder_panes() {
            pane.details().column_view().connect_activate(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, position| window.activate_from_view(position)
            ));
            pane.icon_view().grid().connect_activate(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, position| window.activate_from_view(position)
            ));
        }
    }

    /// Opens the item at `position` of the active pane, which Enter or a
    /// double-click activated. A folder opened with Ctrl goes to a new tab
    /// behind, with Ctrl+Shift to a new tab in front and with Shift to a
    /// new window, whatever is selected, as in Dolphin (TAB-026).
    pub(super) fn activate_from_view(&self, position: u32) {
        // DND-007: the press or release of a drag never opens an item.
        if self.are_item_clicks_paused() {
            return;
        }
        let modifiers = self
            .imp()
            .view_modifiers
            .get()
            .unwrap_or(gdk::ModifierType::empty());
        let opened_elsewhere = open_folder_action(modifiers).zip(self.folder_in_view(position));
        if let Some((action, uri)) = opened_elsewhere {
            action.activate_from(self, Some(&uri.to_variant()));
            return;
        }
        let selected = self.folder_pane().model().selected_positions();
        if selected.is_empty() || selected == [position] {
            self.activate_item(position);
        } else if selected.contains(&position) {
            self.open_selection();
        }
    }

    /// Gives `view` type-to-select, the window's key handling, prefix
    /// resets on clicks, deselection by a click on blank space, rename by
    /// a slow second click on a name,
    /// middle-click to open a folder, the context menu, and file drag and
    /// drop.
    fn folder_input(&self, view: &gtk::Widget) {
        view.update_property(&[gtk::accessible::Property::Label(ox_core::i18n::gettext_static(
            FOLDER_VIEW_LABEL,
        ))]);
        let input = self.typing_input(view);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |controller, key, _, modifiers| window.folder_key(controller, &input, key, modifiers)
        ));
        view.add_controller(keys);
        view.add_controller(self.blank_space_press(view));
        self.attach_slow_click_rename(view);
        // After the press, which clears the selection the band starts from.
        self.attach_rubber_band(view);
        view.add_controller(self.folder_middle_click(view));
        self.attach_context_menu(view);
        self.attach_file_drag(view);
        self.attach_details_hover(view);
        self.attach_file_drop_zone(view, DropZone::FolderView);
    }

    /// The input method that turns key presses in `view` into text for
    /// type-to-select. Like GTK's own text widgets, it knows `view` only
    /// while `view` is realized: GTK's Wayland input method would otherwise
    /// ask a destroyed view for its position.
    fn typing_input(&self, view: &gtk::Widget) -> gtk::IMMulticontext {
        let input = gtk::IMMulticontext::new();
        view.connect_realize(glib::clone!(
            #[strong]
            input,
            move |view| input.set_client_widget(Some(view))
        ));
        view.connect_unrealize(glib::clone!(
            #[strong]
            input,
            move |_| {
                input.focus_out();
                input.set_client_widget(None::<&gtk::Widget>);
            }
        ));
        input.connect_commit(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, text| window.type_text(text)
        ));
        view.add_controller(self.typing_focus(&input));
        input
    }

    /// Tells `input` when the view gains and loses keyboard focus; losing
    /// it also ends a typed prefix.
    fn typing_focus(&self, input: &gtk::IMMulticontext) -> gtk::EventControllerFocus {
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[strong]
            input,
            move |_| input.focus_in()
        ));
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            input,
            move |_| {
                input.focus_out();
                window.reset_typeahead();
            }
        ));
        focus
    }

    /// Handles a key in a folder view before the view does.
    fn folder_key(
        &self,
        controller: &gtk::EventControllerKey,
        input: &gtk::IMMulticontext,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> glib::Propagation {
        // Keys typed while an item is renamed in place belong to its field.
        if self.focus_is_in_text_field() {
            return glib::Propagation::Proceed;
        }
        self.imp().view_modifiers.set(Some(modifiers));
        if is_plain_tab(key, modifiers) && self.tab_to_other_pane() {
            return glib::Propagation::Stop;
        }
        let shortcut =
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK;
        if modifiers.intersects(shortcut) {
            // Ctrl+F, Ctrl+C and friends end a typed prefix.
            self.reset_typeahead();
            return glib::Propagation::Proceed;
        }
        if let Some(handled) = self.prefix_editing_key(controller, input, key) {
            return handled;
        }
        if let Some(handled) = self.grid_row_key(key, modifiers) {
            return handled;
        }
        if let Some(handled) = self.tree_key(key, modifiers) {
            return handled;
        }
        // The input method composes text before it reaches type-to-select.
        let consumed = controller
            .current_event()
            .is_some_and(|event| input.filter_keypress(&event));
        if consumed {
            return glib::Propagation::Stop;
        }
        if !is_modifier_key(key) {
            // Arrows, Home, End, Enter: navigation starts a new prefix.
            self.reset_typeahead();
        }
        glib::Propagation::Proceed
    }

    /// Escape, Backspace and Space, which act on a typed prefix first:
    /// Escape clears the prefix, and only without one the selection;
    /// Backspace erases a typed character, and only without a prefix goes
    /// back, as in Dolphin and Explorer (NAV-004), once the input method
    /// did not take it for text it is composing; Space is prefix text, and
    /// only without one selects the current item. `None` for every other
    /// key.
    fn prefix_editing_key(
        &self,
        controller: &gtk::EventControllerKey,
        input: &gtk::IMMulticontext,
        key: gdk::Key,
    ) -> Option<glib::Propagation> {
        let now = monotonic_now();
        let prefix_active = self.imp().typeahead.borrow().is_active(now);
        match key {
            gdk::Key::Escape if prefix_active => {
                input.reset();
                self.reset_typeahead();
            }
            gdk::Key::Escape if self.close_quick_look() => {}
            // In an Open or Save dialog Escape cancels, as in Windows,
            // even with items selected.
            gdk::Key::Escape if self.is_picking() => self.cancel_picking(),
            gdk::Key::Escape => self.clear_selection(),
            gdk::Key::BackSpace if prefix_active => self.erase_typed_character(now),
            gdk::Key::BackSpace => {
                let composing = controller
                    .current_event()
                    .is_some_and(|event| input.filter_keypress(&event));
                if !composing {
                    self.go_history(Direction::Backward);
                }
            }
            // Space previews the selected file in GNOME's previewer
            // (PROP-012), else selects the current item, unless a prefix
            // is typed.
            gdk::Key::space if !prefix_active && self.toggle_quick_look() => {}
            gdk::Key::space if !prefix_active => self.select_current_item(),
            _ => return None,
        }
        Some(glib::Propagation::Stop)
    }

    /// Space: adds the item with keyboard focus to the selection, as in
    /// Dolphin and Explorer, and never removes it (GTK's own Space toggles
    /// it; Ctrl+Space still does).
    fn select_current_item(&self) {
        let pane = self.folder_pane();
        let current = GtkWindowExt::focus(self).and_then(|focus| pane.owners().position_of(&focus));
        if let Some(position) = current {
            if !pane.model().selection().is_selected(position) {
                pane.model().selection().select_item(position, false);
            }
        }
    }

    /// A primary press on blank space in `view`, below the rows or between
    /// the tiles, clears the selection and keeps keyboard focus on the view,
    /// as in Windows Explorer and Dolphin; with Ctrl or Shift held the
    /// selection stays. A drag from there draws a rubber band
    /// ([`super::rubber_band`]).
    fn blank_space_press(&self, view: &gtk::Widget) -> gtk::GestureClick {
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |press, _, x, y| {
                window.imp().view_modifiers.set(Some(press.current_event_state()));
                if window.are_item_clicks_paused() || !window.is_blank_space(&view, x, y) {
                    return;
                }
                if !press_keeps_selection(press.current_event_state()) {
                    window.folder_pane().model().select_none();
                }
                window.folder_pane().focus_view();
            }
        ));
        press
    }

    /// Whether (`x`, `y`) in `view` is its blank space: inside the list,
    /// not on an item, and not on the column titles.
    pub(super) fn is_blank_space(&self, view: &gtk::Widget, x: f64, y: f64) -> bool {
        if self.folder_pane().owners().position_at(view, x, y).is_some() {
            return false;
        }
        // The details view's rows are an inner list view; its titles are
        // not inside it. The icon grid is the list itself.
        view.pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|picked| &picked == view || picked.ancestor(gtk::ListView::static_type()).is_some())
    }

    /// Middle-click on a folder opens it in a tab without selecting it;
    /// files are never launched this way.
    fn folder_middle_click(&self, view: &gtk::Widget) -> gtk::GestureClick {
        gestures::middle_click(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |gesture, x, y| {
                // A sign-in or another dialog in front takes the clicks.
                if window.shows_dialog() || window.dialog_layer().shown().is_some() {
                    return;
                }
                let Some(uri) = window.folder_at(&view, x, y) else {
                    return;
                };
                window.reset_typeahead();
                let action = gestures::open_action(gesture.current_event_state());
                action.activate_from(&window, Some(&uri.to_variant()));
            }
        ))
    }

    /// The location of the folder at (`x`, `y`) in `view`, if a folder is
    /// there.
    fn folder_at(&self, view: &gtk::Widget, x: f64, y: f64) -> Option<String> {
        let position = self.folder_pane().owners().position_at(view, x, y)?;
        self.folder_in_view(position)
    }

    /// The location of the folder at `position` of the active pane, if a
    /// folder is there.
    fn folder_in_view(&self, position: u32) -> Option<String> {
        let item = self.folder_pane().model().item(position)?;
        match activation_for(item.entry()) {
            Activation::Folder(uri) => Some(uri),
            Activation::File | Activation::Archive | Activation::Refused(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SEL-002
    #[test]
    fn ctrl_or_shift_keeps_the_selection_on_blank_space() {
        assert!(!press_keeps_selection(gdk::ModifierType::empty()));
        assert!(press_keeps_selection(gdk::ModifierType::CONTROL_MASK));
        assert!(press_keeps_selection(gdk::ModifierType::SHIFT_MASK));
    }

    /// parity: SEL-029
    #[test]
    fn modifier_keys_keep_the_typed_prefix() {
        for key in [gdk::Key::Shift_L, gdk::Key::Caps_Lock, gdk::Key::ISO_Level3_Shift] {
            assert!(is_modifier_key(key), "{key:?}");
        }
        for key in [gdk::Key::Down, gdk::Key::Home, gdk::Key::Return] {
            assert!(!is_modifier_key(key), "{key:?}");
        }
    }
}
