// SPDX-License-Identifier: AGPL-3.0-only
//! Keyboard and pointer input of the folder views and the address entry:
//! type-to-select, middle-click to open a folder in a tab, and activation.
//! The context menu has a module of its own ([`super::context_menu`]).
//!
//! Ports `setupKeys` and the type-select glue in `desktop/ui/app.js`
//! (`desktop/tests/ui_type_select.py` is its specification): typed
//! characters jump to the next name with that prefix; Escape first clears
//! the prefix and only then the selection; arrows, clicks, shortcuts and
//! leaving the view start a new prefix.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::typeahead::{self, TypeSelect};

use super::activation::{activation_for, Activation};
use super::gestures;
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

/// The status-bar hint for a type-to-select result.
pub(super) fn typeahead_hint(result: &TypeSelect, matched_name: Option<&str>) -> String {
    match (result.text.is_empty(), matched_name) {
        (true, _) => String::new(),
        (false, Some(name)) => format!("Jump to: {} — {name}", result.text),
        (false, None) => format!("No name starts with “{}”", result.text),
    }
}

impl BrowserWindow {
    /// Adds keyboard and pointer handling to both folder views and the
    /// address entry.
    pub(super) fn install_input(&self) {
        let details = self.content().details.clone();
        let grid = self.content().grid.clone();
        self.folder_input(details.upcast_ref());
        self.folder_input(grid.upcast_ref());
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key != gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                window.finish_address();
                glib::Propagation::Stop
            }
        ));
        self.chrome().address.entry.add_controller(escape);
    }

    /// Opens what was typed into the address bar when Enter is pressed.
    pub(super) fn connect_address_entry(&self) {
        self.chrome().address.entry.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |entry| window.submit_address(entry.text().as_str())
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
                let selected = window.content().model.selected_positions();
                let is_the_selection = matches!(selected.as_slice(), []) || selected == [position];
                if is_the_selection {
                    window.activate_item(position);
                }
            }
        );
        let on_row = activate.clone();
        self.content()
            .details
            .connect_activate(move |_, position| on_row(position));
        self.content()
            .grid
            .connect_activate(move |_, position| activate(position));
    }

    fn folder_input(&self, view: &gtk::Widget) {
        let input = gtk::IMMulticontext::new();
        input.set_client_widget(Some(view));
        input.connect_commit(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, text| window.type_text(text)
        ));
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
        view.add_controller(focus);
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
    }

    /// Handles a key in a folder view before the view does.
    fn folder_key(
        &self,
        controller: &gtk::EventControllerKey,
        input: &gtk::IMMulticontext,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> glib::Propagation {
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
        let now = glib::monotonic_time() / 1000;
        let prefix_active = self.imp().type_ahead.borrow().controller.active(now);
        match key {
            gdk::Key::Escape if prefix_active => {
                input.reset();
                self.reset_typeahead();
            }
            gdk::Key::Escape => self.content().model.select_none(),
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
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.reset_typeahead()
        ));
        click
    }

    fn erase_typed_character(&self, now: i64) {
        let model = &self.content().model;
        let count = model.n_items() as usize;
        let current = model.first_selected().map(|index| index as usize);
        let result = self.imp().type_ahead.borrow_mut().controller.backspace(
            count,
            |index| model.name_at(u32::try_from(index).unwrap_or(u32::MAX)),
            current,
            now,
        );
        if let Some(result) = result {
            self.apply_typeahead(&result);
        }
    }

    /// Adds text the input method committed to the typed prefix and
    /// selects the next matching name.
    pub(super) fn type_text(&self, text: &str) {
        let model = &self.content().model;
        for character in text.chars() {
            let now = glib::monotonic_time() / 1000;
            let current = model.first_selected().map(|index| index as usize);
            let count = model.n_items() as usize;
            let typed = character.to_string();
            let result = self.imp().type_ahead.borrow_mut().controller.push(
                &typed,
                count,
                |index| model.name_at(u32::try_from(index).unwrap_or(u32::MAX)),
                current,
                now,
            );
            if let Some(result) = result {
                self.apply_typeahead(&result);
            }
        }
    }

    fn apply_typeahead(&self, result: &TypeSelect) {
        let position = result.index.and_then(|index| u32::try_from(index).ok());
        if let Some(position) = position {
            self.content().model.select_only(position);
            self.content().reveal(position);
        }
        let matched_name = position.map(|position| self.content().model.name_at(position));
        let hint = typeahead_hint(result, matched_name.as_deref());
        let label = &self.chrome().hint;
        label.set_text(&hint);
        if position.is_some() {
            label.remove_css_class("miss");
        } else {
            label.add_css_class("miss");
        }
        self.restart_typeahead_timer();
    }

    fn restart_typeahead_timer(&self) {
        let timeout = Duration::from_millis(typeahead::TIMEOUT_MS.unsigned_abs());
        let timer = glib::timeout_add_local_once(
            timeout,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().type_ahead.borrow_mut().timer = None;
                    window.reset_typeahead();
                }
            ),
        );
        let previous = self.imp().type_ahead.borrow_mut().timer.replace(timer);
        if let Some(previous) = previous {
            previous.remove();
        }
    }

    /// Forgets the typed prefix and clears its hint.
    pub(super) fn reset_typeahead(&self) {
        let timer = {
            let mut type_ahead = self.imp().type_ahead.borrow_mut();
            type_ahead.controller.reset();
            type_ahead.timer.take()
        };
        if let Some(timer) = timer {
            timer.remove();
        }
        self.chrome().hint.set_text("");
    }

    /// Middle-click on a folder opens it in a tab without selecting it;
    /// files are never launched this way.
    fn folder_middle_click(&self, view: &gtk::Widget) -> gtk::GestureClick {
        let view = view.downgrade();
        gestures::middle_click(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |gesture, x, y| {
                let Some(view) = view.upgrade() else { return };
                let position = window.content().owners.position_at(&view, x, y);
                let item = position.and_then(|position| window.content().model.item(position));
                let Some(Activation::Folder(uri)) = item.map(|item| activation_for(item.entry())) else {
                    return;
                };
                window.reset_typeahead();
                let action = gestures::open_action(gesture.current_event_state());
                let _ = WidgetExt::activate_action(&window, action, Some(&uri.to_variant()));
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str, index: Option<usize>) -> TypeSelect {
        TypeSelect {
            text: text.into(),
            index,
            cycling: false,
        }
    }

    /// Ported from `desktop/tests/ui_type_select.py` (the "Jump to" hint).
    #[test]
    fn the_hint_names_the_item_it_jumped_to() {
        let hint = typeahead_hint(&result("SC", Some(3)), Some("scripts"));
        assert_eq!(hint, "Jump to: SC — scripts");
        let miss = typeahead_hint(&result("zz", None), None);
        assert_eq!(miss, "No name starts with “zz”");
        assert_eq!(typeahead_hint(&result("", None), None), "");
    }

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
