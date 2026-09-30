// SPDX-License-Identifier: AGPL-3.0-only
//! Keyboard and pointer input of the folder views and the address entry:
//! a new window's first keyboard focus, the keys and clicks that feed or
//! end type-to-select, middle-click to open a folder in a tab, and
//! activation. The typed prefix itself lives in [`super::type_to_select`],
//! the context menu in [`super::context_menu`].
//!
//! Ports `onKey` and the type-select glue in `desktop/ui/app.js`
//! (`desktop/tests/ui_type_select.py` is its specification): Escape first
//! clears the prefix and only then the selection; arrows, clicks,
//! shortcuts and leaving the view start a new prefix.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::activation::{activation_for, Activation};
use super::file_drop::DropZone;
use super::folder_pane::PanePage;
use super::gestures;
use super::type_to_select::monotonic_now;
use super::BrowserWindow;

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
        let details = self.folder_pane().details().column_view().clone();
        let grid = self.folder_pane().icon_view().grid().clone();
        self.folder_input(details.upcast_ref());
        self.folder_input(grid.upcast_ref());
        self.address_bar().connect_cancelled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.finish_address()
        ));
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
    }

    /// Enter and double-click open the one selected item. With several
    /// selected, Enter opens nothing, as app.js does, so an item outside
    /// the selection is never opened.
    pub(super) fn connect_view_activation(&self) {
        let activate = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |position: u32| {
                // DND-007: the press or release of a drag never opens an
                // item.
                if window.are_item_clicks_paused() {
                    return;
                }
                let selected = window.folder_pane().model().selected_positions();
                let is_the_selection = selected.is_empty() || selected == [position];
                if is_the_selection {
                    window.activate_item(position);
                }
            }
        );
        let on_row = activate.clone();
        self.folder_pane()
            .details()
            .column_view()
            .connect_activate(move |_, position| on_row(position));
        self.folder_pane()
            .icon_view()
            .grid()
            .connect_activate(move |_, position| activate(position));
    }

    /// Gives `view` type-to-select, the window's key handling, prefix
    /// resets on clicks, middle-click to open a folder, the context menu,
    /// and file drag and drop.
    fn folder_input(&self, view: &gtk::Widget) {
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
        view.add_controller(self.prefix_reset_on_click());
        view.add_controller(self.folder_middle_click(view));
        self.attach_context_menu(view);
        self.attach_file_drag(view);
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
        let shortcut =
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK;
        if modifiers.intersects(shortcut) {
            // Ctrl+F, Ctrl+C and friends end a typed prefix.
            self.reset_typeahead();
            return glib::Propagation::Proceed;
        }
        if let Some(handled) = self.prefix_editing_key(input, key) {
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
    /// Escape clears the prefix, and only without one the selection.
    /// `None` for every other key.
    fn prefix_editing_key(&self, input: &gtk::IMMulticontext, key: gdk::Key) -> Option<glib::Propagation> {
        let now = monotonic_now();
        let prefix_active = self.imp().typeahead.borrow().is_active(now);
        match key {
            gdk::Key::Escape if prefix_active => {
                input.reset();
                self.reset_typeahead();
            }
            gdk::Key::Escape => self.folder_pane().model().select_none(),
            gdk::Key::BackSpace if prefix_active => self.erase_typed_character(now),
            // Space toggles the native selection unless a prefix is typed.
            gdk::Key::space if !prefix_active => return Some(glib::Propagation::Proceed),
            _ => return None,
        }
        Some(glib::Propagation::Stop)
    }

    /// A pointer press in a view starts a new prefix; the click itself
    /// goes on to the view.
    fn prefix_reset_on_click(&self) -> gtk::GestureClick {
        let click = gtk::GestureClick::new();
        click.set_button(gestures::EVERY_BUTTON);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.reset_typeahead()
        ));
        click
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
        let pane = self.folder_pane();
        let position = pane.owners().position_at(view, x, y)?;
        let item = pane.model().item(position)?;
        match activation_for(item.entry()) {
            Activation::Folder(uri) => Some(uri),
            Activation::File | Activation::Archive | Activation::Refused(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
