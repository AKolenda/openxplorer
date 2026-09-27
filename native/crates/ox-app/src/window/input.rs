// SPDX-License-Identifier: AGPL-3.0-only
//! Folder-only keyboard input and native context menus.

use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::typeahead::TypeSelect;

use super::BrowserWindow;

impl BrowserWindow {
    pub(super) fn install_input(self: &Rc<Self>) {
        self.folder_input(&self.content.details);
        self.folder_input(&self.content.grid);
        let escape = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(browser) = weak.upgrade() {
                    browser.finish_address();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.chrome.entry.add_controller(escape);
    }

    fn folder_input(self: &Rc<Self>, view: &impl IsA<gtk::Widget>) {
        let input = gtk::IMMulticontext::new();
        input.set_client_widget(Some(view));
        let weak = Rc::downgrade(self);
        input.connect_commit(move |_, text| {
            if let Some(browser) = weak.upgrade() {
                browser.type_text(text);
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let context = input.clone();
        focus.connect_enter(move |_| context.focus_in());
        let context = input.clone();
        focus.connect_leave(move |_| context.focus_out());
        view.add_controller(focus);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |controller, key, _, modifiers| {
            let Some(browser) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            let now = glib::monotonic_time() / 1000;
            if key == gdk::Key::Escape {
                input.reset();
                browser.reset_typeahead();
                browser.content.model.select_none();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::BackSpace {
                let count = browser.content.model.n_items() as usize;
                let current = browser.content.model.first_selected().map(|index| index as usize);
                let result = browser.typeahead.borrow_mut().backspace(
                    count,
                    |index| browser.content.model.name_at(index as u32),
                    current,
                    now,
                );
                if let Some(result) = result {
                    browser.apply_typeahead(result);
                    return glib::Propagation::Stop;
                }
            }
            // Space toggles native selection until a filename prefix is active.
            if key == gdk::Key::space && !browser.typeahead.borrow().active(now) {
                return glib::Propagation::Proceed;
            }
            // Let GTK's input method compose text before it reaches type-ahead.
            // Navigation keys and unhandled keys continue to the native view.
            if controller
                .current_event()
                .is_some_and(|event| input.filter_keypress(&event))
            {
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(keys);
        self.context_menu(view);
    }

    fn type_text(self: &Rc<Self>, text: &str) {
        for character in text.chars() {
            let now = glib::monotonic_time() / 1000;
            let current = self.content.model.first_selected().map(|index| index as usize);
            let count = self.content.model.n_items() as usize;
            let result = self.typeahead.borrow_mut().push(
                &character.to_string(),
                count,
                |index| self.content.model.name_at(index as u32),
                current,
                now,
            );
            if let Some(result) = result {
                self.apply_typeahead(result);
            }
        }
    }

    fn apply_typeahead(self: &Rc<Self>, result: TypeSelect) {
        if let Some(index) = result.index {
            self.content.model.select_only(index as u32);
            self.content.reveal(index as u32);
        }
        self.chrome.hint.set_text(&if result.text.is_empty() {
            String::new()
        } else if result.index.is_some() {
            format!("Jump to: {}", result.text)
        } else {
            format!("No name starts with “{}”", result.text)
        });
        if let Some(timer) = self.typeahead_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(
            Duration::from_millis(crate::typeahead::TIMEOUT_MS as u64),
            move || {
                if let Some(browser) = weak.upgrade() {
                    browser.typeahead_timer.borrow_mut().take();
                    browser.typeahead.borrow_mut().reset();
                    browser.chrome.hint.set_text("");
                }
            },
        );
        self.typeahead_timer.borrow_mut().replace(timer);
    }

    pub(super) fn reset_typeahead(&self) {
        self.typeahead.borrow_mut().reset();
        self.chrome.hint.set_text("");
        if let Some(timer) = self.typeahead_timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn context_menu(self: &Rc<Self>, view: &impl IsA<gtk::Widget>) {
        let menu = gio::Menu::new();
        menu.append(Some("Open"), Some("win.open"));
        menu.append(Some("Refresh"), Some("win.refresh"));
        let selection = gio::Menu::new();
        selection.append(Some("Select all"), Some("win.select-all"));
        selection.append(Some("Select none"), Some("win.select-none"));
        selection.append(Some("Invert selection"), Some("win.invert-selection"));
        menu.append_section(None, &selection);
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.add_css_class("ox-menu");
        popover.set_parent(view);
        let weak_popover = popover.downgrade();
        view.connect_destroy(move |_| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.unparent();
            }
        });
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        let weak = Rc::downgrade(self);
        let weak_view = view.as_ref().downgrade();
        gesture.connect_pressed(move |gesture, _, x, y| {
            let (Some(browser), Some(view)) = (weak.upgrade(), weak_view.upgrade()) else {
                return;
            };
            if let Some(position) = browser.content.owners.position_at(&view, x, y) {
                if !browser.content.model.selection().is_selected(position) {
                    browser.content.model.select_only(position);
                }
            } else {
                browser.content.model.select_none();
            }
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        view.add_controller(gesture);
    }
}
