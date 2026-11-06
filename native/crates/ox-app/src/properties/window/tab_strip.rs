// SPDX-License-Identifier: AGPL-3.0-only
//! The row of tabs of a Properties window, as Windows' property sheets
//! draw it: one row while the tabs fit the window's width, more rows when
//! they do not (a drive's General, Hardware, Sharing, Security, Previous
//! Versions and Customize), never a wider window. The notebook keeps the
//! pages and its own tab labels, with its tab row hidden; [`super::tab_rows`]
//! lays the tabs out.
//!
//! The row is one stop of the Tab key, on the tab shown; Left and Right,
//! Home and End move between tabs, and Ctrl+Tab, Ctrl+Shift+Tab,
//! Ctrl+Page Down and Ctrl+Page Up anywhere in the window show the next or
//! previous tab, as in Windows. Screen readers hear a tab list of tabs,
//! the one shown selected, and each page is labelled by its tab.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::PropertiesWindow;

impl PropertiesWindow {
    /// Sets up the row of tabs, empty and hidden until a second tab.
    pub(super) fn build_tab_strip(&self) {
        let strip = &self.imp().tab_strip;
        strip.add_css_class("properties-tabs");
        strip.set_visible(false);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| window.tab_strip_key(key, modifiers)
        ));
        strip.add_controller(keys);
        self.imp().notebook.connect_switch_page(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, page| window.show_selected_tab(page)
        ));
    }

    /// Adds the tab `label` for `page`.
    pub(super) fn add_tab_button(&self, label: &str, page: &gtk::Widget) {
        let imp = self.imp();
        let button = gtk::ToggleButton::builder()
            .label(label)
            .css_classes(["properties-tab"])
            .focus_on_click(true)
            .build();
        button.set_accessible_role(gtk::AccessibleRole::Tab);
        if let Some(first) = imp.tab_buttons.borrow().first() {
            button.set_group(Some(first));
        }
        let index = u32::try_from(imp.tab_buttons.borrow().len()).unwrap_or(u32::MAX);
        button.connect_toggled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |button| {
                if button.is_active() {
                    window.imp().notebook.set_current_page(Some(index));
                }
            }
        ));
        page.update_relation(&[gtk::accessible::Relation::LabelledBy(&[button.upcast_ref()])]);
        imp.tab_strip.append(&button);
        imp.tab_buttons.borrow_mut().push(button);
        self.update_tab_strip();
        self.show_selected_tab(imp.notebook.current_page().unwrap_or(0));
    }

    /// Shows the row when there is more than one tab, or when asked to
    /// show a single one.
    pub(super) fn update_tab_strip(&self) {
        let imp = self.imp();
        let count = imp.tab_buttons.borrow().len();
        imp.tab_strip
            .set_visible(count > 1 || (count == 1 && imp.shows_single_tab.get()));
    }

    /// Marks the tab of page `index` as shown: checked, selected for
    /// screen readers, and the only one the Tab key reaches.
    fn show_selected_tab(&self, index: u32) {
        let buttons = self.imp().tab_buttons.borrow().clone();
        for (at, button) in buttons.iter().enumerate() {
            let is_shown = u32::try_from(at).is_ok_and(|at| at == index);
            if is_shown && !button.is_active() {
                button.set_active(true);
            }
            button.set_focusable(is_shown);
            button.update_state(&[gtk::accessible::State::Selected(Some(is_shown))]);
        }
    }

    /// Puts the keyboard on the tab shown; false when the row is hidden.
    pub(super) fn focus_selected_tab(&self) -> bool {
        let imp = self.imp();
        if !imp.tab_strip.is_visible() {
            return false;
        }
        let shown = imp
            .notebook
            .current_page()
            .and_then(|page| usize::try_from(page).ok());
        let button = shown.and_then(|at| imp.tab_buttons.borrow().get(at).cloned());
        button.is_some_and(|button| button.grab_focus())
    }

    /// Shows the tab `step` places after the one shown (before it when
    /// negative), going round at either end; with `focus`, the keyboard
    /// goes to it.
    fn step_tab(&self, step: i32, focus: bool) {
        let imp = self.imp();
        let count = i32::try_from(imp.tab_buttons.borrow().len()).unwrap_or(0);
        if count < 2 {
            return;
        }
        let shown = imp
            .notebook
            .current_page()
            .and_then(|page| i32::try_from(page).ok())
            .unwrap_or(0);
        let next = (shown + step).rem_euclid(count);
        self.show_tab_at(next, focus);
    }

    /// Shows the tab at `index`; with `focus`, the keyboard goes to it.
    fn show_tab_at(&self, index: i32, focus: bool) {
        let imp = self.imp();
        let Ok(index) = u32::try_from(index) else {
            return;
        };
        imp.notebook.set_current_page(Some(index));
        if focus {
            self.focus_selected_tab();
        }
    }

    /// Left, Right, Home and End on the row of tabs.
    fn tab_strip_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        if !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        let count = i32::try_from(self.imp().tab_buttons.borrow().len()).unwrap_or(0);
        match key {
            gdk::Key::Left | gdk::Key::KP_Left => self.step_tab(-1, true),
            gdk::Key::Right | gdk::Key::KP_Right => self.step_tab(1, true),
            gdk::Key::Home | gdk::Key::KP_Home => self.show_tab_at(0, true),
            gdk::Key::End | gdk::Key::KP_End => self.show_tab_at(count - 1, true),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// Ctrl+Tab and Ctrl+Page Down show the next tab, Ctrl+Shift+Tab and
    /// Ctrl+Page Up the previous one, wherever the keyboard is.
    pub(super) fn tab_keys(&self) -> gtk::ShortcutController {
        let controller = gtk::ShortcutController::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let control = gdk::ModifierType::CONTROL_MASK;
        let shift = gdk::ModifierType::SHIFT_MASK;
        for (key, modifiers, step) in [
            (gdk::Key::Tab, control, 1),
            (gdk::Key::Page_Down, control, 1),
            (gdk::Key::ISO_Left_Tab, control | shift, -1),
            (gdk::Key::Tab, control | shift, -1),
            (gdk::Key::Page_Up, control, -1),
        ] {
            let window = self.downgrade();
            let action = gtk::CallbackAction::new(move |_, _| {
                let Some(window) = window.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let had_tab_focus =
                    gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focus: gtk::Widget| {
                        focus.is::<gtk::ToggleButton>() && focus.has_css_class("properties-tab")
                    });
                window.step_tab(step, had_tab_focus);
                glib::Propagation::Stop
            });
            controller.add_shortcut(gtk::Shortcut::new(
                Some(gtk::KeyvalTrigger::new(key, modifiers)),
                Some(action),
            ));
        }
        controller
    }
}
