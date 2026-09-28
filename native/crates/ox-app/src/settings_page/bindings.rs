// SPDX-License-Identifier: AGPL-3.0-only
//! Controls bound to a preference: they show its current value, follow
//! changes made elsewhere and save the user's changes.
//!
//! Ports the `onchange` handlers of the Python settings page
//! (`fire('preferences', {...})` in `desktop/ui/app.js`). A control shows
//! the preference as last read or saved, and follows changes made in
//! another window or by the Python app. Only a change the user makes is
//! saved: while the page shows the current values, the handlers stand by,
//! so showing a value never writes the settings file.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{Preferences, PreferencesUpdate};

use super::choice_list::ChoiceButton;
use super::parts;
use super::{SettingsPage, SharedHandler, MESSAGE};

/// One option of a drop-down: the value it stands for and its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Choice<T: 'static> {
    /// The value saved when it is chosen.
    pub value: T,
    /// What the drop-down shows.
    pub label: &'static str,
}

/// How a control reads one preference and saves a new value.
#[derive(Debug, Clone, Copy)]
pub(super) struct PreferenceBinding<T> {
    /// The preference's current value.
    pub read: fn(&Preferences) -> T,
    /// The settings change that saves `value`.
    pub write: fn(T) -> PreferencesUpdate,
}

impl SettingsPage {
    /// Shows the rows the preferences and the skin as they are now, and
    /// again whenever another window or the Python app changes them.
    pub(super) fn follow_shared_state(&self) {
        let context = self.context();
        let show = glib::clone!(
            #[weak(rename_to = page)]
            self,
            move || page.show_current_preferences()
        );
        let places = context.connect_places_changed(show.clone());
        let text_size = context.skin().connect_text_size_changed(show);
        let mut handlers = self.imp().handlers.borrow_mut();
        handlers.push(SharedHandler {
            object: context.clone().upcast(),
            id: places,
        });
        handlers.push(SharedHandler {
            object: context.skin().clone().upcast(),
            id: text_size,
        });
    }

    /// Shows `message` in the window's message line, such as the outcome
    /// of a change to the default apps.
    pub(super) fn report(&self, message: &str) {
        self.emit_by_name::<()>(MESSAGE, &[&message]);
    }

    /// Runs `hook` each time Settings opens.
    pub(super) fn when_opened(&self, hook: impl Fn() + 'static) {
        self.imp().opened_hooks.borrow_mut().push(Box::new(hook));
    }

    /// Settings opened: reads what may have changed while it was closed.
    pub(super) fn refresh(&self) {
        self.show_current_preferences();
        for hook in self.imp().opened_hooks.borrow().iter() {
            hook();
        }
    }

    /// Saves `update` to the shared settings file. A failure leaves the
    /// change in this window and says so in the window's message line, as
    /// the Python app's text-size toast does.
    pub(super) fn save_preferences(&self, update: PreferencesUpdate) {
        let page = self.downgrade();
        self.context().update_preferences(update, move |result| {
            let Err(error) = result else {
                return;
            };
            let Some(page) = page.upgrade() else {
                return;
            };
            let message = format!("Changed for this window, but could not be saved: {error}");
            page.emit_by_name::<()>(MESSAGE, &[&message]);
        });
    }

    /// Calls `follower` with the current preferences now and whenever they
    /// change, so its control shows them.
    pub(super) fn follow_preferences(&self, follower: impl Fn(&Preferences) + 'static) {
        let preferences = self.context().settings_data().preferences;
        self.while_showing_preferences(|| follower(&preferences));
        self.imp().followers.borrow_mut().push(Box::new(follower));
    }

    /// Shows the current preferences in every bound control.
    pub(super) fn show_current_preferences(&self) {
        let preferences = self.context().settings_data().preferences;
        self.while_showing_preferences(|| {
            for follower in self.imp().followers.borrow().iter() {
                follower(&preferences);
            }
        });
    }

    /// Runs `show`, during which control changes are not the user's.
    pub(super) fn while_showing_preferences(&self, show: impl FnOnce()) {
        let showing = &self.imp().showing_preferences;
        let was_showing = showing.replace(true);
        show();
        showing.set(was_showing);
    }

    /// Whether a control changed because the user changed it, not because
    /// the page showed a value.
    pub(super) fn is_user_change(&self) -> bool {
        !self.imp().showing_preferences.get()
    }

    /// A switch showing the preference `binding` reads, which saves the
    /// user's changes.
    pub(super) fn preference_switch(&self, binding: PreferenceBinding<bool>) -> gtk::Switch {
        let switch = parts::switch();
        self.follow_preferences(glib::clone!(
            #[weak]
            switch,
            move |preferences| switch.set_active((binding.read)(preferences))
        ));
        switch.connect_active_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |switch| {
                if page.is_user_change() {
                    page.save_preferences((binding.write)(switch.is_active()));
                }
            }
        ));
        switch
    }

    /// A drop-down of `choices` showing the preference `binding` reads,
    /// which saves the option the user chooses.
    pub(super) fn preference_choice<T: Copy + PartialEq + 'static>(
        &self,
        choices: &'static [Choice<T>],
        binding: PreferenceBinding<T>,
    ) -> gtk::MenuButton {
        let labels: Vec<String> = choices.iter().map(|choice| choice.label.to_owned()).collect();
        let drop_down = ChoiceButton::new(&labels);
        let list = drop_down.choices.clone();
        self.follow_preferences(glib::clone!(
            #[weak]
            list,
            move |preferences| {
                let current = (binding.read)(preferences);
                let position = choices.iter().position(|choice| choice.value == current);
                list.set_selected(position_u32(position.unwrap_or_default()));
            }
        ));
        list.connect_selected_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |list| {
                if !page.is_user_change() {
                    return;
                }
                if let Some(chosen) = choices.get(list.selected() as usize) {
                    page.save_preferences((binding.write)(chosen.value));
                }
            }
        ));
        drop_down.button
    }
}

/// `position` in a list of a few options, as GTK counts them.
pub(super) fn position_u32(position: usize) -> u32 {
    u32::try_from(position).expect("a drop-down has a few options")
}
